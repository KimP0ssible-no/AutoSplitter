//! From OCR text to start and split events: the state machine, the frame ring buffer and PB rules.

use crate::matcher::{fuzzy_contains, fuzzy_ratio};
use crate::profile::Profile;
use image::RgbImage;
use std::collections::VecDeque;
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Waiting,
    Running,
    Done,
}

/// What a matched text triggers.
enum Hit {
    Start,
    Split(usize),
}

#[derive(Debug, PartialEq)]
pub enum Event {
    /// Newest OCR text of one OCR pass, for the preview panel.
    Ocr { text: String, ms: u32 },
    Started { text: String },
    Split { index: usize, skipped: usize, segment_time: f64, total: f64, manual: bool },
    Log(String),
}

pub struct Detector {
    p: Profile,
    retries: usize,
    /// Frames kept: one OCR interval of captures, 5 to 30.
    ring: usize,
    state: State,
    armed: bool,
    current: Option<usize>,
    frames: VecDeque<(RgbImage, f64)>,
    last_ocr: Option<f64>,
    previous_text: String,
    start: f64,
    last_split: f64,
    last_manual: Option<f64>,
    /// Cumulative time of each segment in this run, None where it was skipped.
    pub times: Vec<Option<f64>>,
    pub debug: bool,
}

impl Detector {
    pub fn new(profile: &Profile) -> Detector {
        let mut d = Detector {
            p: profile.clone(),
            retries: (profile.retry_frames as usize).min(profile.target_fps.saturating_sub(1) as usize).max(1),
            ring: ((profile.target_fps as f32 * profile.interval_seconds).ceil() as usize).clamp(5, 30),
            state: State::Waiting,
            armed: false,
            current: None,
            frames: VecDeque::new(),
            last_ocr: None,
            previous_text: String::new(),
            start: 0.0,
            last_split: 0.0,
            last_manual: None,
            times: Vec::new(),
            debug: false,
        };
        d.reset();
        d
    }

    pub fn reset(&mut self) {
        self.state = State::Waiting;
        // No <From> keywords: the first <To> match starts the run.
        self.armed = self.p.start_from.is_empty();
        self.current = None;
        self.frames.clear();
        self.last_ocr = None;
        self.previous_text.clear();
        self.last_manual = None;
        self.times = vec![None; self.p.segments.len()];
    }

    pub fn state(&self) -> State {
        self.state
    }

    /// Takes one captured crop (None when it was too dark to read) and runs OCR when the interval is due.
    /// OCR tries the newest buffered crops first. On a match it goes back through older crops while they
    /// still show the matched text, and the earliest of them gives the time: within one capture of when
    /// the text appeared.
    pub fn frame(&mut self, crop: Option<RgbImage>, ts: f64, ocr: &mut dyn FnMut(&RgbImage) -> Option<String>) -> Vec<Event> {
        let mut events = Vec::new();
        if !self.p.ocr_enabled {
            return events;
        }
        if let Some(c) = crop {
            if self.frames.len() == self.ring {
                self.frames.pop_front();
            }
            self.frames.push_back((c, ts));
        }
        if self.frames.is_empty() || self.last_ocr.is_some_and(|t| ts - t < f64::from(self.p.interval_seconds)) {
            return events;
        }
        self.last_ocr = Some(ts);
        let (mut newest, mut newest_ms, mut matched) = (String::new(), 0, false);
        for age in 0..self.retries.min(self.frames.len()) {
            let (image, frame_ts) = &self.frames[self.frames.len() - 1 - age];
            let frame_ts = *frame_ts;
            let started = Instant::now();
            let Some(text) = ocr(image).filter(|t| !t.is_empty()) else { continue };
            let ms = started.elapsed().as_millis() as u32;
            if newest.is_empty() {
                (newest, newest_ms) = (text.clone(), ms);
            }
            if !self.changed(&text) {
                continue;
            }
            let Some(hit) = self.hit(&text, frame_ts) else { continue };
            let (mut first, mut first_ts) = (age, frame_ts);
            for older in age + 1..self.frames.len() {
                let (image, t) = &self.frames[self.frames.len() - 1 - older];
                let t = *t;
                match ocr(image) {
                    Some(o) if self.shows(&hit, &o, t) => (first, first_ts) = (older, t),
                    _ => break,
                }
            }
            if self.debug && first > 0 {
                events.push(Event::Log(format!("[DEBUG] The text first showed {first} capture(s) back")));
            }
            match hit {
                Hit::Start => {
                    self.state = State::Running;
                    (self.start, self.last_split) = (first_ts, first_ts);
                    events.push(Event::Started { text: text.clone() });
                }
                Hit::Split(index) => self.split(index, index - self.current.map_or(0, |c| c + 1), first_ts, false, &mut events),
            }
            self.previous_text.clear();
            (newest, newest_ms, matched) = (text, ms, true);
            break;
        }
        if !matched && !newest.is_empty() && self.changed(&newest) {
            self.previous_text = newest.clone();
        }
        if self.debug && !newest.is_empty() {
            let (w, h) = self.frames.back().map_or((0, 0), |(i, _)| i.dimensions());
            events.push(Event::Log(format!("[DEBUG] OCR ({newest_ms} ms, {w}x{h}): \"{}\"", one_line(&newest))));
        }
        events.push(Event::Ocr { text: newest, ms: newest_ms });
        events
    }

    // While waiting, unchanged text is not checked again; while running, every pass is checked.
    fn changed(&self, text: &str) -> bool {
        self.state == State::Running || fuzzy_ratio(text, &self.previous_text) < self.p.change_threshold
    }

    fn any(&self, text: &str, keys: &[String]) -> bool {
        keys.iter().any(|k| fuzzy_contains(text, k, self.p.match_threshold))
    }

    /// What `text`, captured at `ts`, triggers. A <From> keyword only arms the start, which has no time.
    fn hit(&mut self, text: &str, ts: f64) -> Option<Hit> {
        match self.state {
            State::Waiting if !self.armed => {
                // The <To> check waits for a later pass than the one that matched <From>.
                self.armed = self.any(text, &self.p.start_from);
                None
            }
            State::Waiting => self.any(text, &self.p.start_to).then_some(Hit::Start),
            State::Running if !self.last_manual.is_some_and(|m| ts - m < f64::from(self.p.cooldown_seconds)) => {
                let next = self.current.map_or(0, |c| c + 1);
                (next..self.p.segments.len()).find(|&i| self.shows(&Hit::Split(i), text, ts)).map(Hit::Split)
            }
            _ => None,
        }
    }

    /// True when `text`, captured at `ts`, shows `hit`: its keywords, and for a split a time after the
    /// last split that is past the segment's block time (pte).
    fn shows(&self, hit: &Hit, text: &str, ts: f64) -> bool {
        match *hit {
            Hit::Start => self.any(text, &self.p.start_to),
            Hit::Split(i) => {
                let s = &self.p.segments[i];
                ts > self.last_split && ts - self.start >= f64::from(s.pte) && self.any(text, &s.keywords)
            }
        }
    }

    fn split(&mut self, index: usize, skipped: usize, ts: f64, manual: bool, events: &mut Vec<Event>) {
        let total = ts - self.start;
        events.push(Event::Split { index, skipped, segment_time: ts - self.last_split, total, manual });
        if skipped > 0 {
            events.push(Event::Log(format!("WARNING: Skipped {skipped} segment(s)")));
        }
        self.last_split = ts;
        self.current = Some(index);
        self.times[index] = Some(total);
        if self.p.segments[index].is_final {
            self.state = State::Done;
        }
    }

    /// Moves to the next segment. Before the start trigger it starts the run instead, as
    /// LiveSplit's start/split key does. Either blocks OCR splits for the cooldown.
    pub fn manual_split(&mut self, ts: f64) -> Vec<Event> {
        let mut events = Vec::new();
        if self.state == State::Waiting {
            self.state = State::Running;
            (self.start, self.last_split, self.last_manual) = (ts, ts, Some(ts));
            events.push(Event::Started { text: "split key".into() });
            return events;
        }
        let next = self.current.map_or(0, |c| c + 1);
        if self.state != State::Running || next >= self.p.segments.len() || self.last_manual.is_some_and(|m| ts - m < f64::from(self.p.cooldown_seconds)) {
            return events;
        }
        self.last_manual = Some(ts);
        self.previous_text.clear();
        self.split(next, 0, ts, true, &mut events);
        events
    }

    /// Run time so far, from the frame that started the run.
    pub fn elapsed(&self, ts: f64) -> f64 {
        if self.state == State::Waiting { 0.0 } else { ts - self.start }
    }
}

/// Crops the OCR region from a BGRA frame. Returns None for a dark crop
/// (fewer than 20 pixels brighter than 150), because OCR on it only finds noise.
pub fn crop_bright(bgra: &[u8], width: usize, height: usize, stride: usize, region: [i32; 4]) -> Option<RgbImage> {
    let clamp = |v: i32, max: usize| v.clamp(0, max as i32) as usize;
    let (l, t, r, b) = (clamp(region[0], width), clamp(region[1], height), clamp(region[2], width), clamp(region[3], height));
    if r <= l || b <= t {
        return None;
    }
    let mut bright = 0;
    let img = RgbImage::from_fn((r - l) as u32, (b - t) as u32, |x, y| {
        let i = (t + y as usize) * stride + (l + x as usize) * 4;
        let (bl, g, rd) = (bgra[i] as u32, bgra[i + 1] as u32, bgra[i + 2] as u32);
        if (299 * rd + 587 * g + 114 * bl) / 1000 > 150 {
            bright += 1;
        }
        image::Rgb([rd as u8, g as u8, bl as u8])
    });
    (bright >= 20).then_some(img)
}

/// True when a finished run must offer to save its times as the new PB.
/// An improvement of `protection` seconds or more is treated as a glitched run.
pub fn offer_pb(previous_total: Option<f64>, total: f64, protection: f32) -> bool {
    previous_total.is_none_or(|p| total < p && p - total < f64::from(protection))
}

/// Writes the run's times as PBs. A skipped segment keeps its old PB.
pub fn apply_pb(profile: &mut Profile, times: &[Option<f64>]) {
    for (s, t) in profile.segments.iter_mut().zip(times) {
        if t.is_some() {
            s.pb = *t;
        }
    }
}

/// "M:SS.mmm", as the legacy timer showed it.
pub fn fmt_time(seconds: f64) -> String {
    let ms = (seconds.max(0.0) * 1000.0) as u64;
    format!("{}:{:02}.{:03}", ms / 60_000, ms / 1000 % 60, ms % 1000)
}

/// A time typed as "M:SS.mmm", "H:MM:SS.mmm" or plain seconds; None if it is none of these.
pub fn parse_time(s: &str) -> Option<f64> {
    s.trim().split(':').try_fold(0.0, |acc: f64, part| part.trim().parse::<f64>().ok().filter(|v| *v >= 0.0).map(|v| acc * 60.0 + v))
}

/// OCR text on one line, for the log and the preview.
pub fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::Segment;

    fn seg(name: &str, pte: u32, is_final: bool) -> Segment {
        Segment { name: name.into(), keywords: vec![name.into()], is_final, pb: None, pte }
    }

    fn profile() -> Profile {
        Profile {
            start_from: vec!["normal".into()],
            start_to: vec!["enter the estate".into()],
            segments: vec![seg("courtyard", 0, false), seg("tanker", 0, false), seg("lovebirds", 240, true)],
            ..Profile::default()
        }
    }

    /// One capture showing `text`; OCR reads each buffered capture's own text.
    fn feed(d: &mut Detector, shown: &mut Vec<String>, text: &str, ts: f64) -> Vec<Event> {
        shown.push(text.into());
        let img = RgbImage::from_pixel(1, 1, image::Rgb([(shown.len() - 1) as u8, 0, 0]));
        d.frame(Some(img), ts, &mut |i| Some(shown[i.get_pixel(0, 0)[0] as usize].clone())).into_iter().filter(|e| !matches!(e, Event::Ocr { .. })).collect()
    }

    #[test]
    fn two_phase_start_skips_and_pte() {
        let mut d = Detector::new(&profile());
        let mut shown = Vec::new();
        assert!(feed(&mut d, &mut shown, "enter the estate", 0.0).is_empty(), "To before From must not start");
        assert!(feed(&mut d, &mut shown, "Normal", 1.0).is_empty());
        assert_eq!(feed(&mut d, &mut shown, "Enter the Estate", 2.0), vec![Event::Started { text: "Enter the Estate".into() }]);
        // pte 240 keeps the final segment closed early in the run.
        assert!(feed(&mut d, &mut shown, "lovebirds", 3.0).is_empty());
        let e = feed(&mut d, &mut shown, "tanker", 12.0);
        assert_eq!(e[0], Event::Split { index: 1, skipped: 1, segment_time: 10.0, total: 10.0, manual: false });
        assert_eq!(d.times, vec![None, Some(10.0), None]);
        feed(&mut d, &mut shown, "lovebirds", 300.0);
        assert_eq!(d.state(), State::Done);
        assert_eq!(d.times[2], Some(298.0));

        let mut p = profile();
        p.segments[0].pb = Some(5.0);
        apply_pb(&mut p, &d.times);
        assert_eq!(p.segments.iter().map(|s| s.pb).collect::<Vec<_>>(), vec![Some(5.0), Some(10.0), Some(298.0)]);
    }

    #[test]
    fn single_phase_interval_and_manual_cooldown() {
        let mut p = profile();
        p.start_from.clear();
        let mut d = Detector::new(&p);
        let mut shown = Vec::new();
        assert!(matches!(feed(&mut d, &mut shown, "enter the estate", 0.0)[0], Event::Started { .. }));
        // Inside the 1 s OCR interval nothing is read.
        assert!(feed(&mut d, &mut shown, "courtyard", 0.5).is_empty());
        assert!(matches!(d.manual_split(2.0)[0], Event::Split { index: 0, manual: true, .. }));
        // Before the start trigger, the split key starts the run.
        let mut w = Detector::new(&profile());
        assert_eq!(w.manual_split(1.0), vec![Event::Started { text: "split key".into() }]);
        assert_eq!((w.state(), w.elapsed(3.0)), (State::Running, 2.0));
        assert!(w.manual_split(2.0).is_empty(), "cooldown");
        assert!(d.manual_split(3.0).is_empty(), "cooldown");
        assert!(feed(&mut d, &mut shown, "tanker", 4.0).is_empty(), "cooldown blocks OCR splits too");
        assert!(matches!(feed(&mut d, &mut shown, "tanker", 7.5)[0], Event::Split { index: 1, .. }));
    }

    #[test]
    fn retries_older_frames_and_uses_their_time() {
        let mut p = profile();
        p.start_from.clear();
        p.target_fps = 5;
        let mut d = Detector::new(&p);
        let img = |v| RgbImage::from_pixel(1, 1, image::Rgb([v, 0, 0]));
        // The older buffered frame shows the trigger text, the newest does not.
        d.frame(Some(img(1)), 0.0, &mut |_| None);
        let e = d.frame(Some(img(2)), 1.0, &mut |i| Some(if i.get_pixel(0, 0)[0] == 1 { "enter the estate" } else { "loading" }.into()));
        assert!(e.contains(&Event::Started { text: "enter the estate".into() }));
        assert_eq!(d.elapsed(1.5), 1.5);
    }

    #[test]
    fn times_from_the_first_frame_that_showed_the_text() {
        let mut p = profile();
        p.start_from.clear();
        let mut d = Detector::new(&p);
        let img = |v| RgbImage::from_pixel(1, 1, image::Rgb([v, 0, 0]));
        // 5 captures a second; the start text shows from 0.4 on, OCR reads at 0.0 and 1.0.
        let read = |i: &RgbImage| Some(if i.get_pixel(0, 0)[0] >= 2 { "enter the estate" } else { "loading" }.to_string());
        for (v, ts) in [(0, 0.0), (1, 0.2), (2, 0.4), (2, 0.6), (2, 0.8)] {
            d.frame(Some(img(v)), ts, &mut |i| read(i));
        }
        assert!(d.frame(Some(img(2)), 1.0, &mut |i| read(i)).contains(&Event::Started { text: "enter the estate".into() }));
        assert_eq!(d.elapsed(1.5), 1.1, "timed from 0.4, not from the OCR pass at 1.0");

        // A split goes back too, but never to before the start.
        let read = |i: &RgbImage| Some(if i.get_pixel(0, 0)[0] == 3 { "courtyard" } else { "enter the estate" }.to_string());
        for (v, ts) in [(2, 1.2), (3, 1.4), (3, 1.6), (3, 1.8)] {
            d.frame(Some(img(v)), ts, &mut |i| read(i));
        }
        let e = d.frame(Some(img(3)), 2.0, &mut |i| read(i));
        assert!(matches!(e[0], Event::Split { index: 0, .. }));
        assert!((d.times[0].unwrap() - 1.0).abs() < 1e-9, "split at 1.4, run started at 0.4");
    }

    #[test]
    fn helpers() {
        assert!(offer_pb(None, 500.0, 120.0));
        assert!(offer_pb(Some(500.0), 450.0, 120.0));
        assert!(!offer_pb(Some(500.0), 300.0, 120.0), "suspiciously large improvement");
        assert!(!offer_pb(Some(500.0), 510.0, 120.0));
        assert_eq!(fmt_time(354.0), "5:54.000");
        assert_eq!(fmt_time(44.8), "0:44.800");
        assert_eq!(parse_time("1:13.456"), Some(73.456));
        assert_eq!(parse_time("73.456"), Some(73.456));
        assert_eq!(parse_time("1:02:03.5"), Some(3723.5));
        assert_eq!(parse_time(fmt_time(354.0).as_str()), Some(354.0));
        assert_eq!(parse_time("abc"), None);
        assert_eq!(parse_time("-5"), None);
        let mut bgra = vec![0u8; 10 * 10 * 4];
        assert!(crop_bright(&bgra, 10, 10, 40, [0, 0, 10, 10]).is_none());
        bgra.fill(255);
        let c = crop_bright(&bgra, 10, 10, 40, [2, 2, 8, 20]).unwrap();
        assert_eq!(c.dimensions(), (6, 8));
    }
}
