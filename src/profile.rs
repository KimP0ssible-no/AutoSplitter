//! Profile XML: one game mission's OCR settings, triggers, segments and personal bests.

use std::fmt::Write as _;
use std::path::Path;

pub const CURRENT_VERSION: u32 = 2;

#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    pub name: String,
    pub keywords: Vec<String>,
    pub is_final: bool,
    /// Personal best, cumulative seconds from the start.
    pub pb: Option<f64>,
    /// Seconds into the run before this segment may match ("pte").
    pub pte: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    pub version: u32,
    pub game: String,
    pub mission: String,
    pub target_fps: u32,
    pub cooldown_seconds: f32,
    pub pb_protection_seconds: f32,
    // Kept so old profiles round-trip; nothing reads them.
    pub pixel_enabled: bool,
    pub fade_threshold: i32,
    pub fade_frames_required: i32,
    pub ocr_enabled: bool,
    /// Left, top, right, bottom in physical screen pixels.
    pub region: [i32; 4],
    pub interval_seconds: f32,
    pub change_threshold: f32,
    pub match_threshold: f32,
    pub retry_frames: u32,
    pub language: String,
    pub start_from: Vec<String>,
    pub start_to: Vec<String>,
    pub segments: Vec<Segment>,
    pub livesplit_enabled: bool,
    pub livesplit_host: String,
    pub livesplit_port: u16,
    pub overlay_enabled: bool,
    pub overlay_x: i32,
    pub overlay_y: i32,
    pub overlay_opacity: u8,
    /// Overlay size, 1.0 for the standard size.
    pub overlay_scale: f32,
    /// "Mouse3", "F5" or "VK123", as the legacy app wrote it.
    pub split_key: String,
}

impl Default for Profile {
    fn default() -> Self {
        Profile {
            version: CURRENT_VERSION,
            game: String::new(),
            mission: String::new(),
            target_fps: 5,
            cooldown_seconds: 5.0,
            pb_protection_seconds: 120.0,
            pixel_enabled: true,
            fade_threshold: 12,
            fade_frames_required: 5,
            ocr_enabled: true,
            region: [0, 0, 50, 200],
            interval_seconds: 1.0,
            change_threshold: 0.6,
            match_threshold: 0.92,
            retry_frames: 3,
            language: "en".into(),
            start_from: Vec::new(),
            start_to: Vec::new(),
            segments: Vec::new(),
            livesplit_enabled: false,
            livesplit_host: "localhost".into(),
            livesplit_port: 16834,
            overlay_enabled: false,
            overlay_x: 100,
            overlay_y: 100,
            overlay_opacity: 80,
            overlay_scale: 1.0,
            split_key: "Mouse3".into(),
        }
    }
}

type Node<'a, 'i> = roxmltree::Node<'a, 'i>;

fn child<'a, 'i>(n: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    n.children().find(|c| c.has_tag_name(name))
}

fn texts(n: Option<Node>, name: &str) -> Vec<String> {
    let Some(n) = n else { return Vec::new() };
    n.children().filter(|c| c.has_tag_name(name)).filter_map(|c| c.text()).filter(|t| !t.is_empty()).map(String::from).collect()
}

fn num<T: std::str::FromStr>(n: Option<Node>, attr: &str, default: T) -> T {
    n.and_then(|n| n.attribute(attr)).and_then(|v| v.trim().parse().ok()).unwrap_or(default)
}

// pugixml's as_bool: true when the value starts with 1, t, T, y or Y.
fn flag(n: Option<Node>, attr: &str, default: bool) -> bool {
    n.and_then(|n| n.attribute(attr)).map_or(default, |v| matches!(v.trim_start().chars().next(), Some('1' | 't' | 'T' | 'y' | 'Y')))
}

impl Profile {
    pub fn parse(xml: &str) -> Result<Profile, String> {
        let doc = roxmltree::Document::parse(xml).map_err(|e| e.to_string())?;
        let root = doc.root_element();
        if !root.has_tag_name("SplitProfile") {
            return Err("the root element is not <SplitProfile>".into());
        }
        let d = Profile::default();
        let text = |name| child(root, name).and_then(|n| n.text()).unwrap_or("").to_string();
        let capture = child(root, "Capture");
        let detection = child(root, "Detection");
        let pixel = detection.and_then(|n| child(n, "Pixel"));
        let ocr = detection.and_then(|n| child(n, "OCR"));
        let start = child(root, "StartTrigger");
        let ls = child(root, "LiveSplit");
        let overlay = child(root, "Overlay");
        let mut region = d.region;
        if let Some(r) = ocr.and_then(|n| n.attribute("region")) {
            let parts: Vec<i32> = r.split(',').filter_map(|p| p.trim().parse().ok()).collect();
            if let Ok(r) = parts.try_into() {
                region = r;
            }
        }
        let segments = child(root, "Segments")
            .map(|s| {
                s.children()
                    .filter(|c| c.has_tag_name("Segment"))
                    .map(|c| Segment {
                        name: c.attribute("name").unwrap_or("Unnamed").to_string(),
                        keywords: texts(Some(c), "Keyword"),
                        is_final: flag(Some(c), "final", false),
                        pb: c.attribute("pb").and_then(|v| v.trim().parse().ok()).filter(|&v: &f64| v >= 0.0),
                        pte: num(Some(c), "pte", 0),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let split_key = child(root, "Hotkeys").and_then(|n| n.attribute("manualSplitKey")).filter(|k| !k.is_empty()).unwrap_or(&d.split_key).to_string();
        Ok(Profile {
            version: num(Some(root), "profileVersion", 0),
            game: text("Game"),
            mission: text("Mission"),
            target_fps: num(capture, "targetFps", d.target_fps).max(1),
            cooldown_seconds: num(detection, "cooldownSeconds", d.cooldown_seconds),
            pb_protection_seconds: num(detection, "pbProtectionSeconds", d.pb_protection_seconds),
            pixel_enabled: flag(pixel, "enabled", d.pixel_enabled),
            fade_threshold: num(pixel, "fadeThreshold", d.fade_threshold),
            fade_frames_required: num(pixel, "fadeFramesRequired", d.fade_frames_required),
            ocr_enabled: flag(ocr, "enabled", d.ocr_enabled),
            region,
            interval_seconds: num(ocr, "intervalSeconds", d.interval_seconds),
            change_threshold: num(ocr, "changeThreshold", d.change_threshold),
            match_threshold: num(ocr, "fuzzyMatchThreshold", d.match_threshold),
            retry_frames: num(ocr, "ocrRetryFrames", d.retry_frames),
            language: ocr.and_then(|n| n.attribute("language")).unwrap_or(&d.language).to_string(),
            start_from: texts(start, "From"),
            start_to: texts(start, "To"),
            segments,
            livesplit_enabled: flag(ls, "enabled", d.livesplit_enabled),
            livesplit_host: ls.and_then(|n| n.attribute("host")).unwrap_or(&d.livesplit_host).to_string(),
            livesplit_port: num(ls, "port", d.livesplit_port),
            overlay_enabled: flag(overlay, "enabled", d.overlay_enabled),
            overlay_x: num(overlay, "x", d.overlay_x),
            overlay_y: num(overlay, "y", d.overlay_y),
            overlay_opacity: num(overlay, "opacity", d.overlay_opacity).min(100),
            overlay_scale: num(overlay, "scale", d.overlay_scale).clamp(0.6, 2.5),
            split_key,
        })
    }

    /// "Game — Mission", as the profile list shows it.
    pub fn display_name(&self) -> String {
        format!("{} \u{2014} {}", self.game, self.mission)
    }

    pub fn to_xml(&self) -> String {
        let e = escape;
        let mut x = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        let [l, t, r, b] = self.region;
        let _ = write!(
            x,
            "<SplitProfile profileVersion=\"{}\">\n  <Game>{}</Game>\n  <Mission>{}</Mission>\n  <Capture targetFps=\"{}\" />\n  \
             <Detection cooldownSeconds=\"{}\" pbProtectionSeconds=\"{}\">\n    \
             <Pixel enabled=\"{}\" fadeThreshold=\"{}\" fadeFramesRequired=\"{}\" />\n    \
             <OCR enabled=\"{}\" region=\"{l},{t},{r},{b}\" intervalSeconds=\"{}\" changeThreshold=\"{}\" fuzzyMatchThreshold=\"{}\" ocrRetryFrames=\"{}\" language=\"{}\" />\n  \
             </Detection>\n  <StartTrigger>\n",
            self.version.max(CURRENT_VERSION),
            e(&self.game),
            e(&self.mission),
            self.target_fps,
            self.cooldown_seconds,
            self.pb_protection_seconds,
            self.pixel_enabled,
            self.fade_threshold,
            self.fade_frames_required,
            self.ocr_enabled,
            self.interval_seconds,
            self.change_threshold,
            self.match_threshold,
            self.retry_frames,
            e(&self.language),
        );
        for k in &self.start_from {
            let _ = writeln!(x, "    <From>{}</From>", e(k));
        }
        for k in &self.start_to {
            let _ = writeln!(x, "    <To>{}</To>", e(k));
        }
        x.push_str("  </StartTrigger>\n  <Segments>\n");
        for s in &self.segments {
            let _ = write!(x, "    <Segment name=\"{}\"", e(&s.name));
            if s.is_final {
                x.push_str(" final=\"true\"");
            }
            if let Some(pb) = s.pb {
                let _ = write!(x, " pb=\"{pb:.3}\"");
            }
            if s.pte > 0 {
                let _ = write!(x, " pte=\"{}\"", s.pte);
            }
            x.push_str(">\n");
            for k in &s.keywords {
                let _ = writeln!(x, "      <Keyword>{}</Keyword>", e(k));
            }
            x.push_str("    </Segment>\n");
        }
        let _ = write!(
            x,
            "  </Segments>\n  <LiveSplit enabled=\"{}\" host=\"{}\" port=\"{}\" />\n  <Overlay enabled=\"{}\" x=\"{}\" y=\"{}\" opacity=\"{}\" scale=\"{}\" />\n  \
             <Hotkeys manualSplitKey=\"{}\" />\n</SplitProfile>\n",
            self.livesplit_enabled,
            e(&self.livesplit_host),
            self.livesplit_port,
            self.overlay_enabled,
            self.overlay_x,
            self.overlay_y,
            self.overlay_opacity,
            self.overlay_scale,
            e(&self.split_key),
        );
        x
    }

    /// Reads a profile. A profile from an older schema is rewritten in the current one,
    /// which keeps every value it had and adds the newer settings with their defaults.
    pub fn load(path: &Path) -> Result<Profile, String> {
        let xml = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let mut p = Profile::parse(&xml)?;
        if p.version < CURRENT_VERSION {
            p.version = CURRENT_VERSION;
            // shortcut: the rewrite drops XML comments and unknown attributes, upgrade to an in-place merge if users keep notes in profiles.
            p.save(path).map_err(|e| e.to_string())?;
        }
        Ok(p)
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let tmp = path.with_extension("xml.tmp");
        std::fs::write(&tmp, self.to_xml())?;
        std::fs::rename(tmp, path)
    }

    /// The template the New button writes.
    pub fn new_template(game: &str, mission: &str) -> Profile {
        Profile {
            game: game.into(),
            mission: mission.into(),
            start_from: vec!["keyword_here".into()],
            start_to: vec!["keyword_here".into()],
            segments: vec![Segment { name: "New Split".into(), keywords: vec!["keyword_here".into()], is_final: true, pb: None, pte: 0 }],
            ..Profile::default()
        }
    }
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_profiles_round_trip() {
        for name in ["example_paradise_lost.xml", "example_lincoln_memorial.xml"] {
            let xml = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("profiles").join(name)).unwrap();
            let p = Profile::parse(&xml).unwrap();
            assert!(!p.segments.is_empty() && p.segments.last().unwrap().is_final, "{name}");
            assert_eq!(Profile::parse(&p.to_xml()).unwrap(), Profile { version: CURRENT_VERSION, ..p });
        }
    }

    #[test]
    fn legacy_values_parse() {
        let p = Profile::parse(
            r#"<SplitProfile><Game>G &amp; H</Game><Mission>M</Mission>
            <Detection><OCR region="441, 60,996,140" engine="windows" fuzzyMatchThreshold="0.920000017" enabled="yes"/></Detection>
            <StartTrigger><From></From><To>go</To></StartTrigger>
            <Segments><Segment name="A" pb="44.8" pte="240"><Keyword>a</Keyword></Segment><Segment final="1"/></Segments></SplitProfile>"#,
        )
        .unwrap();
        assert_eq!((p.version, p.game.as_str(), p.region), (0, "G & H", [441, 60, 996, 140]));
        assert_eq!((p.match_threshold, p.split_key.as_str()), (0.92, "Mouse3"));
        assert!(p.start_from.is_empty() && p.start_to == ["go"]);
        assert_eq!((p.segments[0].pb, p.segments[0].pte, p.segments[1].name.as_str()), (Some(44.8), 240, "Unnamed"));
        assert!(p.segments[1].is_final && p.ocr_enabled);
        assert!(p.to_xml().contains("fuzzyMatchThreshold=\"0.92\"") && p.to_xml().contains("<Game>G &amp; H</Game>"));
        assert!(Profile::parse("<Other/>").is_err());
    }
}
