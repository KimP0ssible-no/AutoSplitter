// No console window behind the program on Windows.
#![cfg_attr(windows, windows_subsystem = "windows")]

use autosplitter::capture::Capture;
use autosplitter::ocr::{self, Ocr};
use autosplitter::profile::{Profile, Segment};
use autosplitter::run::{crop_bright, fmt_time, one_line, parse_time};
use autosplitter::ui::EditorWindow;
use autosplitter::dialog::tell;
use autosplitter::{paths, picker};
use slint::{ComponentHandle, Model, ModelRc, SharedString, StandardListViewItem, VecModel};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const LANGUAGES: [&str; 2] = ["en", "zh-CN"];

struct Editor {
    ui: slint::Weak<EditorWindow>,
    profile: Profile,
    saved: Profile,
    path: Option<PathBuf>,
    from: Rc<VecModel<StandardListViewItem>>,
    to: Rc<VecModel<StandardListViewItem>>,
    segments: Rc<VecModel<StandardListViewItem>>,
    /// The segment the fields on the right show, -1 for none.
    selected: i32,
    /// What the preview reads: the region and the OCR language.
    preview: Arc<Mutex<([i32; 4], String)>>,
}

type Shared = Rc<RefCell<Editor>>;

fn items(list: &[String]) -> Vec<StandardListViewItem> {
    list.iter().map(|s| StandardListViewItem::from(s.as_str())).collect()
}

fn texts(model: &VecModel<StandardListViewItem>) -> Vec<String> {
    model.iter().map(|i| i.text.to_string()).collect()
}

/// "Name · PB 5:54.000 · blocked 240 s · ends the run": the last segment ends the run when saved.
fn segment_labels(segments: &[Segment]) -> Vec<StandardListViewItem> {
    let last = segments.len().saturating_sub(1);
    segments
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let mut label = s.name.clone();
            if let Some(pb) = s.pb {
                label += &format!("  \u{b7}  PB {}", fmt_time(pb));
            }
            if s.pte > 0 {
                label += &format!("  \u{b7}  blocked {} s", s.pte);
            }
            if i == last {
                label += "  \u{b7}  ends the run";
            }
            label.as_str().into()
        })
        .collect()
}

fn show(ed: &mut Editor) {
    let Some(ui) = ed.ui.upgrade() else { return };
    let p = &ed.profile;
    ui.set_file(ed.path.as_ref().map(|p| p.display().to_string()).unwrap_or_default().into());
    ui.set_game(p.game.as_str().into());
    ui.set_mission(p.mission.as_str().into());
    ui.set_fps(p.target_fps as i32);
    ui.set_interval(p.interval_seconds.to_string().into());
    ui.set_match_percent((p.match_threshold * 100.0).round() as i32);
    ui.set_retries(p.retry_frames as i32);
    ui.set_cooldown(p.cooldown_seconds.to_string().into());
    ui.set_pb_protection(p.pb_protection_seconds.round() as i32);
    let (tag, _, _) = ocr::language_pack(&p.language);
    ui.set_language(LANGUAGES.iter().position(|l| *l == tag).unwrap_or(0) as i32);
    let [l, t, r, b] = p.region;
    ui.set_left(l.to_string().into());
    ui.set_top(t.to_string().into());
    ui.set_right(r.to_string().into());
    ui.set_bottom(b.to_string().into());
    ed.from.set_vec(items(&p.start_from));
    ed.to.set_vec(items(&p.start_to));
    ed.segments.set_vec(segment_labels(&p.segments));
    ui.set_segment_current(-1);
    ed.selected = -1;
    fill_segment(ed);
    *ed.preview.lock().unwrap_or_else(|e| e.into_inner()) = (p.region, tag.to_string());
}

/// The form into the profile.
fn read(ed: &mut Editor) {
    let Some(ui) = ed.ui.upgrade() else { return };
    let p = &mut ed.profile;
    p.game = ui.get_game().trim().to_string();
    p.mission = ui.get_mission().trim().to_string();
    p.target_fps = ui.get_fps().max(1) as u32;
    p.retry_frames = ui.get_retries().max(1) as u32;
    p.match_threshold = ui.get_match_percent() as f32 / 100.0;
    p.pb_protection_seconds = ui.get_pb_protection() as f32;
    let positive = |s: SharedString, old: f32| s.trim().parse::<f32>().ok().filter(|v| *v >= 0.0).unwrap_or(old);
    p.interval_seconds = positive(ui.get_interval(), p.interval_seconds).max(0.1);
    p.cooldown_seconds = positive(ui.get_cooldown(), p.cooldown_seconds);
    p.language = LANGUAGES[ui.get_language().clamp(0, 1) as usize].to_string();
    p.region = region(&ui).unwrap_or(p.region);
    p.start_from = texts(&ed.from);
    p.start_to = texts(&ed.to);
    // The last segment ends the run.
    let last = p.segments.len().saturating_sub(1);
    for (i, s) in p.segments.iter_mut().enumerate() {
        s.is_final = i == last;
    }
}

fn region(ui: &EditorWindow) -> Option<[i32; 4]> {
    let v = [ui.get_left(), ui.get_top(), ui.get_right(), ui.get_bottom()].map(|s| s.trim().parse::<i32>().ok());
    let [l, t, r, b] = v;
    let r = [l?, t?, r?, b?];
    (r[2] > r[0] && r[3] > r[1]).then_some(r)
}

/// Keeps the edits of the segment shown so far, then shows segment `i`.
fn select_segment(ed: &mut Editor, i: i32) {
    apply_segment(ed);
    ed.selected = i;
    fill_segment(ed);
}

fn fill_segment(ed: &Editor) {
    let Some(ui) = ed.ui.upgrade() else { return };
    let seg = usize::try_from(ed.selected).ok().and_then(|i| ed.profile.segments.get(i));
    ui.set_segment_name(seg.map_or("".into(), |s| s.name.as_str().into()));
    ui.set_segment_pte(seg.map_or("0".into(), |s| s.pte.to_string().into()));
    ui.set_segment_pb(seg.and_then(|s| s.pb).map_or("".into(), |pb| fmt_time(pb).into()));
    ui.set_segment_keywords(seg.map_or("".into(), |s| s.keywords.join("\n").into()));
}

fn apply_segment(ed: &mut Editor) {
    let Some(ui) = ed.ui.upgrade() else { return };
    let Some(i) = usize::try_from(ed.selected).ok().filter(|i| *i < ed.profile.segments.len()) else { return };
    let s = &mut ed.profile.segments[i];
    s.name = ui.get_segment_name().trim().to_string();
    s.pte = ui.get_segment_pte().trim().parse().unwrap_or(0);
    // Empty clears the PB; text that is not a time keeps the old one.
    let pb = ui.get_segment_pb();
    s.pb = if pb.trim().is_empty() { None } else { parse_time(&pb).or(s.pb) };
    s.keywords = ui.get_segment_keywords().lines().map(str::trim).filter(|k| !k.is_empty()).map(String::from).collect();
    ed.segments.set_row_data(i, segment_labels(&ed.profile.segments).swap_remove(i));
}

/// Saves to the profile's file, or to one the user picks; false if nothing was saved.
async fn save(ed: Shared, choose: bool) -> bool {
    {
        let mut e = ed.borrow_mut();
        apply_segment(&mut e);
        read(&mut e);
    }
    let problem = {
        let e = ed.borrow();
        if e.profile.segments.is_empty() {
            Some("Add at least one segment.")
        } else if e.profile.start_to.is_empty() {
            Some("Add at least one To keyword: it starts the run.")
        } else {
            None
        }
    };
    if let Some(p) = problem {
        tell("Save", p);
        return false;
    }
    let current = ed.borrow().path.clone();
    let path = match current {
        Some(p) if !choose => p,
        current => {
            let name = current.as_ref().and_then(|p| p.file_name()).map_or("new_profile.xml".into(), |n| n.to_string_lossy().into_owned());
            let dialog = rfd::AsyncFileDialog::new().set_directory(paths::profiles_dir()).add_filter("Profile", &["xml"]).set_file_name(name);
            match dialog.save_file().await {
                Some(p) => p.path().to_path_buf(),
                None => return false,
            }
        }
    };
    let result = ed.borrow().profile.save(&path);
    match result {
        Ok(()) => {
            let mut e = ed.borrow_mut();
            e.saved = e.profile.clone();
            e.path = Some(path);
            if let Some(ui) = e.ui.upgrade() {
                ui.set_file(e.path.as_ref().map(|p| p.display().to_string()).unwrap_or_default().into());
            }
            true
        }
        Err(err) => {
            tell("Save", &format!("{}: {err}", path.display()));
            false
        }
    }
}

/// True when the form differs from the saved profile.
fn changed(ed: &Shared) -> bool {
    let mut e = ed.borrow_mut();
    apply_segment(&mut e);
    read(&mut e);
    e.profile != e.saved
}

/// False when the user cancels; asks first if there are unsaved changes.
async fn may_discard(ed: Shared) -> bool {
    if !changed(&ed) {
        return true;
    }
    let answer = rfd::AsyncMessageDialog::new()
        .set_title("Unsaved changes")
        .set_description("Save the changes to this profile?")
        .set_buttons(rfd::MessageButtons::YesNoCancel)
        .show()
        .await;
    match answer {
        rfd::MessageDialogResult::Yes => save(ed, false).await,
        rfd::MessageDialogResult::No => true,
        _ => false,
    }
}

fn open(ed: &Shared, path: PathBuf) {
    match Profile::load(&path) {
        Ok(p) => {
            let mut e = ed.borrow_mut();
            e.saved = p.clone();
            e.profile = p;
            e.path = Some(path);
            show(&mut e);
        }
        Err(err) => {
            tell("Open", &format!("{}: {err}", path.display()));
        }
    }
}

/// Reads the region once a second for the preview, on its own thread.
fn preview(what: Arc<Mutex<([i32; 4], String)>>, tx: Sender<(String, u32)>) {
    let (mut ocr, mut capture): (Option<Ocr>, Option<(Capture, [i32; 4])>) = (None, None);
    loop {
        let tick = Instant::now();
        let (region, language) = what.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if ocr.as_ref().is_none_or(|o| o.language != language) {
            ocr = None;
            match Ocr::new(&paths::models_dir(), &language) {
                Ok(o) => ocr = Some(o),
                Err(e) => drop(tx.send((format!("OCR: {e}"), 0))),
            }
        }
        if capture.as_ref().is_none_or(|(_, r)| *r != region) {
            capture = Capture::new(region).ok().map(|c| (c, region));
        }
        if let (Some(o), Some((c, _))) = (ocr.as_mut(), capture.as_mut())
            && let Ok(Some(f)) = c.grab(100)
        {
            let started = Instant::now();
            let text = crop_bright(&f.bgra, f.width, f.height, f.stride, [0, 0, f.width as i32, f.height as i32])
                .map(|img| o.read(&img).unwrap_or_else(|e| format!("OCR: {e}")))
                .unwrap_or_default();
            if tx.send((text, started.elapsed().as_millis() as u32)).is_err() {
                return;
            }
        }
        std::thread::sleep(Duration::from_secs(1).saturating_sub(tick.elapsed()));
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    paths::ensure_profiles(&paths::profiles_dir())?;
    let ui = EditorWindow::new()?;
    ui.set_version(env!("CARGO_PKG_VERSION").into());
    let ed: Shared = Rc::new(RefCell::new(Editor {
        ui: ui.as_weak(),
        profile: Profile::new_template("", ""),
        saved: Profile::new_template("", ""),
        path: None,
        from: Rc::new(VecModel::default()),
        to: Rc::new(VecModel::default()),
        segments: Rc::new(VecModel::default()),
        selected: -1,
        preview: Arc::new(Mutex::new(([0, 0, 50, 200], "en".into()))),
    }));
    {
        let mut e = ed.borrow_mut();
        ui.set_from_items(ModelRc::from(e.from.clone()));
        ui.set_to_items(ModelRc::from(e.to.clone()));
        ui.set_segment_items(ModelRc::from(e.segments.clone()));
        show(&mut e);
    }
    if let Some(path) = std::env::args_os().nth(1) {
        open(&ed, PathBuf::from(path));
    }

    let (tx, rx) = mpsc::channel();
    let what = ed.borrow().preview.clone();
    std::thread::spawn(move || preview(what, tx));
    let timer = slint::Timer::default();
    let weak = ui.as_weak();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(200), move || {
        if let (Some(ui), Some((text, ms))) = (weak.upgrade(), rx.try_iter().last()) {
            ui.set_ocr_text(one_line(&text).into());
            ui.set_ocr_ms(format!("{ms} ms").into());
        }
    });

    let e = ed.clone();
    ui.on_new_file(move || {
        let e = e.clone();
        let _ = slint::spawn_local(async move {
            if may_discard(e.clone()).await {
                let mut ed = e.borrow_mut();
                ed.profile = Profile::new_template("", "");
                ed.saved = ed.profile.clone();
                ed.path = None;
                show(&mut ed);
            }
        });
    });
    let e = ed.clone();
    ui.on_open_file(move || {
        let e = e.clone();
        let _ = slint::spawn_local(async move {
            if may_discard(e.clone()).await
                && let Some(p) = rfd::AsyncFileDialog::new().set_directory(paths::profiles_dir()).add_filter("Profile", &["xml"]).pick_file().await
            {
                open(&e, p.path().to_path_buf());
            }
        });
    });
    let e = ed.clone();
    ui.on_save(move || drop(slint::spawn_local(save(e.clone(), false))));
    let e = ed.clone();
    ui.on_save_as(move || drop(slint::spawn_local(save(e.clone(), true))));
    let e = ed.clone();
    ui.on_region_edited(move || {
        let ed = e.borrow();
        if let Some(r) = ed.ui.upgrade().and_then(|ui| region(&ui)) {
            ed.preview.lock().unwrap_or_else(|e| e.into_inner()).0 = r;
        }
    });
    let e = ed.clone();
    ui.on_language_changed(move || {
        let ed = e.borrow();
        if let Some(ui) = ed.ui.upgrade() {
            ed.preview.lock().unwrap_or_else(|e| e.into_inner()).1 = LANGUAGES[ui.get_language().clamp(0, 1) as usize].into();
        }
    });
    let e = ed.clone();
    ui.on_set_region(move || {
        // The game's monitor: where the region typed in the form is.
        let r = e.borrow().preview.lock().unwrap_or_else(|e| e.into_inner()).0;
        let ed = e.clone();
        let picked = picker::pick_region((r[0], r[1]), move |r| {
            let e = ed.borrow();
            if let Some(ui) = e.ui.upgrade() {
                ui.set_left(r[0].to_string().into());
                ui.set_top(r[1].to_string().into());
                ui.set_right(r[2].to_string().into());
                ui.set_bottom(r[3].to_string().into());
            }
            e.preview.lock().unwrap_or_else(|e| e.into_inner()).0 = r;
        });
        if let Err(err) = picked {
            tell("Set Region", &err);
        }
    });

    // Keyword lists: Add takes the entry field, Copy OCR fills it.
    macro_rules! list {
        ($add:ident, $remove:ident, $ocr:ident, $model:ident, $entry:ident, $set_entry:ident, $current:ident) => {
            let e = ed.clone();
            ui.$add(move || {
                let ed = e.borrow();
                let Some(ui) = ed.ui.upgrade() else { return };
                let text = ui.$entry().trim().to_string();
                if !text.is_empty() {
                    ed.$model.push(text.as_str().into());
                    ui.$set_entry("".into());
                }
            });
            let e = ed.clone();
            ui.$remove(move || {
                let ed = e.borrow();
                if let Some(i) = ed.ui.upgrade().and_then(|ui| usize::try_from(ui.$current()).ok()).filter(|i| *i < ed.$model.row_count()) {
                    ed.$model.remove(i);
                }
            });
            let w = ui.as_weak();
            ui.$ocr(move || {
                if let Some(ui) = w.upgrade() {
                    ui.$set_entry(ui.get_ocr_text());
                }
            });
        };
    }
    list!(on_add_from, on_remove_from, on_ocr_to_from, from, get_from_entry, set_from_entry, get_from_current);
    list!(on_add_to, on_remove_to, on_ocr_to_to, to, get_to_entry, set_to_entry, get_to_current);

    let e = ed.clone();
    ui.on_select_segment(move |i| select_segment(&mut e.borrow_mut(), i));
    let e = ed.clone();
    ui.on_apply_segment(move || apply_segment(&mut e.borrow_mut()));
    let e = ed.clone();
    ui.on_add_segment(move || {
        let mut ed = e.borrow_mut();
        apply_segment(&mut ed);
        let s = Segment { name: "New Split".into(), keywords: vec!["keyword_here".into()], is_final: false, pb: None, pte: 0 };
        ed.profile.segments.push(s);
        ed.segments.set_vec(segment_labels(&ed.profile.segments));
        let i = ed.profile.segments.len() as i32 - 1;
        if let Some(ui) = ed.ui.upgrade() {
            ui.set_segment_current(i);
        }
        ed.selected = i;
        fill_segment(&ed);
    });
    let e = ed.clone();
    ui.on_remove_segment(move || {
        let mut ed = e.borrow_mut();
        let Some(ui) = ed.ui.upgrade() else { return };
        let Some(i) = usize::try_from(ui.get_segment_current()).ok().filter(|i| *i < ed.profile.segments.len()) else { return };
        ed.profile.segments.remove(i);
        ed.segments.set_vec(segment_labels(&ed.profile.segments));
        ui.set_segment_current(-1);
        ed.selected = -1;
        fill_segment(&ed);
    });
    let e = ed.clone();
    ui.on_move_segment(move |by| {
        let mut ed = e.borrow_mut();
        apply_segment(&mut ed);
        let Some(ui) = ed.ui.upgrade() else { return };
        let i = ed.selected;
        let j = i + by;
        let n = ed.profile.segments.len() as i32;
        if i < 0 || j < 0 || i >= n || j >= n {
            return;
        }
        ed.profile.segments.swap(i as usize, j as usize);
        ed.segments.set_vec(segment_labels(&ed.profile.segments));
        ui.set_segment_current(j);
        ed.selected = j;
    });
    let w = ui.as_weak();
    ui.on_ocr_to_segment(move || {
        if let Some(ui) = w.upgrade() {
            let mut k = ui.get_segment_keywords().to_string();
            if !k.is_empty() && !k.ends_with('\n') {
                k.push('\n');
            }
            k.push_str(ui.get_ocr_text().as_str());
            ui.set_segment_keywords(k.into());
        }
    });
    let e = ed.clone();
    ui.window().on_close_requested(move || {
        if !changed(&e) {
            return slint::CloseRequestResponse::HideWindow;
        }
        // Open until the question is answered; then the editor quits, or stays on Cancel.
        let e = e.clone();
        let _ = slint::spawn_local(async move {
            if may_discard(e).await {
                let _ = slint::quit_event_loop();
            }
        });
        slint::CloseRequestResponse::KeepWindowShown
    });

    ui.run()?;
    Ok(())
}
