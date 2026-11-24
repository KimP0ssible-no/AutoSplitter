// No console window behind the program on Windows.
#![cfg_attr(windows, windows_subsystem = "windows")]

use autosplitter::capture::Capture;
use autosplitter::input::{self, Input};
use autosplitter::livesplit::LiveSplit;
use autosplitter::ocr::{self, Ocr};
use autosplitter::profile::Profile;
use autosplitter::run::{self, Detector, Event, State, fmt_time, one_line};
use autosplitter::ui::{MainWindow, Overlay, Row};
use autosplitter::{paths, picker, update};
use slint::{ComponentHandle, Model, ModelRc, PhysicalPosition, SharedString, VecModel};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const LOG_LINES: usize = 1000;

/// Seconds since the program started; capture, OCR and hotkeys share this clock.
fn now() -> f64 {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    EPOCH.get_or_init(Instant::now).elapsed().as_secs_f64()
}

/// From the window to the backend thread.
enum Cmd {
    Start(Box<Profile>, bool),
    Stop,
    Reset,
    Split(f64),
    Debug(bool),
    Language(String),
}

/// From the backend (and the update check) to the window.
enum Ev {
    Log(String),
    Ocr(String, u32),
    Started(f64, String),
    Split { index: usize, skipped: usize, segment_time: f64, total: f64 },
    Done(f64, Vec<Option<f64>>),
    Reset,
    Stopped,
    Update(String, String),
}

// ---- Backend thread: OCR, capture, the detector and LiveSplit live here only. ----

fn backend(rx: Receiver<Cmd>, tx: Sender<Ev>) {
    let (mut ocr, mut livesplit) = (None, None);
    while let Ok(cmd) = rx.recv() {
        match cmd {
            Cmd::Start(p, debug) => run_profile(&p, debug, &rx, &tx, &mut ocr, &mut livesplit),
            Cmd::Reset => {
                if let Some(ls) = &mut livesplit {
                    let _ = ls.reset();
                }
            }
            Cmd::Language(l) => {
                if let Err(e) = load_ocr(&mut ocr, &l, &tx) {
                    let _ = tx.send(Ev::Log(format!("[ERROR] OCR: {e}")));
                }
            }
            Cmd::Stop | Cmd::Split(_) | Cmd::Debug(_) => {}
        }
    }
}

fn load_ocr<'a>(ocr: &'a mut Option<Ocr>, language: &str, tx: &Sender<Ev>) -> Result<&'a mut Ocr, String> {
    let (tag, _, known) = ocr::language_pack(language);
    if !known {
        let _ = tx.send(Ev::Log(format!("[WARN] Unknown OCR language '{language}', using English")));
    }
    if ocr.as_ref().is_none_or(|o| o.language != tag) {
        *ocr = None;
        let started = Instant::now();
        *ocr = Some(Ocr::new(&paths::models_dir(), tag)?);
        let _ = tx.send(Ev::Log(format!("[INFO] OCR ready ({tag}, {} ms)", started.elapsed().as_millis())));
    }
    ocr.as_mut().ok_or_else(|| "OCR is not loaded".into())
}

fn run_profile(p: &Profile, debug: bool, rx: &Receiver<Cmd>, tx: &Sender<Ev>, ocr: &mut Option<Ocr>, ls: &mut Option<LiveSplit>) {
    let log = |s: String| drop(tx.send(Ev::Log(s)));
    let stopped = |s: String| {
        log(s);
        let _ = tx.send(Ev::Stopped);
    };
    let ocr = match load_ocr(ocr, &p.language, tx) {
        Ok(o) => o,
        Err(e) => return stopped(format!("[ERROR] OCR: {e}")),
    };
    *ls = None;
    if p.livesplit_enabled {
        match LiveSplit::connect(&p.livesplit_host, p.livesplit_port) {
            Ok(c) => {
                log(format!("[INFO] LiveSplit connected: {}:{}", p.livesplit_host, p.livesplit_port));
                *ls = Some(c);
            }
            Err(e) => log(format!("[WARN] LiveSplit connection failed ({e}), continuing without LiveSplit")),
        }
    }
    let mut capture = match Capture::new(p.region) {
        Ok(c) => c,
        Err(e) => return stopped(format!("[ERROR] Screen capture: {e}")),
    };
    let mut det = Detector::new(p);
    det.debug = debug;
    log(format!("[INFO] Started: {} ({} segments)", p.display_name(), p.segments.len()));
    let frame_time = Duration::from_secs_f64(1.0 / f64::from(p.target_fps.max(1)));
    let mut capture_error = false;
    loop {
        let tick = Instant::now();
        let mut events = Vec::new();
        while let Ok(cmd) = rx.try_recv() {
            match cmd {
                Cmd::Stop => {
                    if let Some(c) = ls {
                        let _ = c.reset();
                    }
                    return stopped("[INFO] Stopped".into());
                }
                Cmd::Reset => {
                    det.reset();
                    if let Some(c) = ls {
                        let _ = c.reset();
                    }
                    let _ = tx.send(Ev::Reset);
                    log("[INFO] Run reset".into());
                }
                Cmd::Split(ts) => events.extend(det.manual_split(ts)),
                Cmd::Debug(d) => det.debug = d,
                Cmd::Start(..) | Cmd::Language(_) => {}
            }
        }
        match capture.grab(0) {
            Ok(Some(f)) => {
                capture_error = false;
                let crop = run::crop_bright(&f.bgra, f.width, f.height, f.stride, [0, 0, f.width as i32, f.height as i32]);
                events.extend(det.frame(crop, now(), &mut |img| ocr.read(img).map_err(|e| log(format!("[ERROR] OCR: {e}"))).ok()));
            }
            Ok(None) => {}
            Err(e) if !capture_error => {
                capture_error = true;
                log(format!("[ERROR] Screen capture: {e}"));
            }
            Err(_) => {}
        }
        for e in events {
            forward(e, &det, tx, ls);
        }
        if let Some(rest) = frame_time.checked_sub(tick.elapsed()) {
            std::thread::sleep(rest);
        }
    }
}

fn forward(e: Event, det: &Detector, tx: &Sender<Ev>, ls: &mut Option<LiveSplit>) {
    let sent = match e {
        Event::Ocr { text, ms } => tx.send(Ev::Ocr(text, ms)),
        Event::Log(s) => tx.send(Ev::Log(s)),
        Event::Started { text } => {
            let livesplit = ls.as_mut().map(|c| c.start());
            if let Some(Err(e)) = livesplit {
                let _ = tx.send(Ev::Log(format!("[WARN] LiveSplit: {e}")));
            }
            let t = now();
            tx.send(Ev::Started(t - det.elapsed(t), text))
        }
        Event::Split { index, skipped, segment_time, total, manual: _ } => {
            if let Some(Err(e)) = ls.as_mut().map(|c| c.split(skipped, total)) {
                let _ = tx.send(Ev::Log(format!("[WARN] LiveSplit: {e}")));
            }
            let _ = tx.send(Ev::Split { index, skipped, segment_time, total });
            if det.state() == State::Done {
                let _ = tx.send(Ev::Done(total, det.times.clone()));
            }
            Ok(())
        }
    };
    drop(sent);
}

// ---- Window ----

struct App {
    ui: slint::Weak<MainWindow>,
    overlay: Overlay,
    cmd: Sender<Cmd>,
    profile: Profile,
    path: Option<PathBuf>,
    files: Vec<PathBuf>,
    rows: Rc<VecModel<Row>>,
    log: Rc<VecModel<SharedString>>,
    /// Run start on the [`now`] clock, while the clock runs.
    start: Option<f64>,
    running: bool,
    binding: bool,
}

type Shared = Rc<RefCell<App>>;

fn log(app: &Shared, line: &str) {
    let a = app.borrow();
    for l in line.lines() {
        a.log.push(format!("{} {l}", chrono::Local::now().format("%H:%M:%S")).into());
    }
    while a.log.row_count() > LOG_LINES {
        a.log.remove(0);
    }
}

fn ask(title: &str, text: &str) -> bool {
    rfd::MessageDialog::new().set_title(title).set_description(text).set_buttons(rfd::MessageButtons::YesNo).show() == rfd::MessageDialogResult::Yes
}

fn tell(title: &str, text: &str) {
    rfd::MessageDialog::new().set_title(title).set_description(text).set_level(rfd::MessageLevel::Error).show();
}

fn set_clock(app: &App, text: &str, gold: bool) {
    let Some(ui) = app.ui.upgrade() else { return };
    ui.set_timer(text.into());
    ui.set_timer_state(i32::from(gold));
    app.overlay.set_timer(text.into());
    app.overlay.set_timer_state(i32::from(gold));
}

fn set_active(app: &App, row: i32) {
    if let Some(ui) = app.ui.upgrade() {
        ui.set_active_row(row);
    }
    app.overlay.set_active_row(row);
}

/// Segment names and PBs, no run times.
fn show_segments(app: &App) {
    let rows: Vec<Row> = app
        .profile
        .segments
        .iter()
        .map(|s| Row { name: s.name.as_str().into(), pb: s.pb.map_or("-".into(), fmt_time).into(), ..Row::default() })
        .collect();
    app.rows.set_vec(rows);
    let pb_total = app.profile.segments.last().and_then(|s| s.pb).map_or("-".into(), fmt_time);
    app.overlay.set_heading(app.profile.display_name().into());
    app.overlay.set_pb_total(pb_total.into());
    if let Some(ui) = app.ui.upgrade() {
        ui.set_heading(app.profile.display_name().into());
    }
    set_active(app, -1);
    set_clock(app, "0:00.000", false);
}

/// Puts the profile's settings into the window.
fn show_profile(app: &App) {
    let Some(ui) = app.ui.upgrade() else { return };
    let p = &app.profile;
    ui.set_fps(p.target_fps as i32);
    ui.set_retries(p.retry_frames as i32);
    ui.set_interval(p.interval_seconds.to_string().into());
    ui.set_match_percent((p.match_threshold * 100.0).round() as i32);
    ui.set_pb_protection(p.pb_protection_seconds.round() as i32);
    ui.set_livesplit(p.livesplit_enabled);
    ui.set_livesplit_host(p.livesplit_host.as_str().into());
    ui.set_livesplit_port(p.livesplit_port.to_string().into());
    ui.set_overlay(p.overlay_enabled);
    ui.set_overlay_opacity(f32::from(p.overlay_opacity));
    ui.set_split_key(p.split_key.as_str().into());
    ui.set_region(format!("{},{},{},{}", p.region[0], p.region[1], p.region[2], p.region[3]).into());
    show_segments(app);
    app.overlay.set_bg_opacity(f32::from(p.overlay_opacity));
    app.overlay.window().set_position(PhysicalPosition::new(p.overlay_x, p.overlay_y));
    show_overlay(app);
}

/// The window's settings into the profile (not saved).
fn read_settings(app: &mut App) {
    let Some(ui) = app.ui.upgrade() else { return };
    let p = &mut app.profile;
    p.target_fps = ui.get_fps().max(1) as u32;
    p.retry_frames = ui.get_retries().max(1) as u32;
    if let Ok(v) = ui.get_interval().trim().parse::<f32>()
        && v > 0.0
    {
        p.interval_seconds = v;
    }
    p.match_threshold = ui.get_match_percent() as f32 / 100.0;
    p.pb_protection_seconds = ui.get_pb_protection() as f32;
    p.livesplit_enabled = ui.get_livesplit();
    p.livesplit_host = ui.get_livesplit_host().trim().to_string();
    p.livesplit_port = ui.get_livesplit_port().trim().parse().unwrap_or(p.livesplit_port);
    p.overlay_enabled = ui.get_overlay();
    p.overlay_opacity = ui.get_overlay_opacity().round() as u8;
    if app.overlay.window().is_visible() {
        let pos = app.overlay.window().position();
        (p.overlay_x, p.overlay_y) = (pos.x, pos.y);
    }
}

fn show_overlay(app: &App) {
    let Some(ui) = app.ui.upgrade() else { return };
    app.overlay.set_bg_opacity(ui.get_overlay_opacity());
    let _ = if ui.get_overlay() { app.overlay.show() } else { app.overlay.hide() };
}

fn save(app: &Shared) {
    let (path, xml_ok) = {
        let a = app.borrow();
        let Some(path) = a.path.clone() else { return };
        let ok = a.profile.save(&path);
        (path, ok)
    };
    match xml_ok {
        Ok(()) => log(app, &format!("[INFO] Saved {}", path.display())),
        Err(e) => log(app, &format!("[ERROR] Cannot save {}: {e}", path.display())),
    }
}

/// Re-reads the profile folder and selects `select` (or the first profile).
fn refresh(app: &Shared, select: Option<PathBuf>) {
    let dir = paths::profiles_dir();
    let files = paths::list_profiles(&dir);
    let names: Vec<SharedString> = files
        .iter()
        .map(|f| match Profile::load(f) {
            Ok(p) => p.display_name().into(),
            Err(_) => format!("{} (parse error)", f.file_name().unwrap_or_default().to_string_lossy()).into(),
        })
        .collect();
    let index = select.and_then(|s| files.iter().position(|f| *f == s)).unwrap_or(0);
    let Some(ui) = app.borrow().ui.upgrade() else { return };
    ui.set_profiles(ModelRc::new(VecModel::from(names)));
    app.borrow_mut().files = files;
    ui.set_profile_index(index as i32);
    load(app, index);
}

fn load(app: &Shared, index: usize) {
    let Some(path) = app.borrow().files.get(index).cloned() else {
        log(app, &format!("[WARN] No profiles in {}", paths::profiles_dir().display()));
        return;
    };
    match Profile::load(&path) {
        Ok(p) => {
            let language = p.language.clone();
            {
                let mut a = app.borrow_mut();
                a.profile = p;
                a.path = Some(path.clone());
                show_profile(&a);
                let _ = a.cmd.send(Cmd::Language(language));
            }
            log(app, &format!("[INFO] Profile: {}", path.display()));
        }
        Err(e) => log(app, &format!("[ERROR] {}: {e}", path.display())),
    }
}

fn start(app: &Shared) {
    let mut a = app.borrow_mut();
    if a.running || a.path.is_none() {
        return;
    }
    read_settings(&mut a);
    show_segments(&a);
    a.running = true;
    a.start = None;
    let debug = a.ui.upgrade().is_some_and(|ui| {
        ui.set_running(true);
        ui.set_run_state(1);
        ui.get_debug()
    });
    let _ = a.cmd.send(Cmd::Start(Box::new(a.profile.clone()), debug));
}

fn stop(app: &Shared) {
    let _ = app.borrow().cmd.send(Cmd::Stop);
}

fn reset(app: &Shared) {
    let mut a = app.borrow_mut();
    let _ = a.cmd.send(Cmd::Reset);
    a.start = None;
    show_segments(&a);
}

fn handle(app: &Shared, ev: Ev) {
    match ev {
        Ev::Log(s) => log(app, &s),
        Ev::Ocr(text, ms) => {
            if let Some(ui) = app.borrow().ui.upgrade() {
                ui.set_ocr_text(one_line(&text).into());
                ui.set_ocr_ms(format!("{ms} ms").into());
            }
        }
        Ev::Started(at, text) => {
            {
                let mut a = app.borrow_mut();
                a.start = Some(at);
                set_active(&a, 0);
                if let Some(ui) = a.ui.upgrade() {
                    ui.set_run_state(2);
                }
            }
            log(app, &format!("Timer started: {}", one_line(&text)));
        }
        Ev::Split { index, skipped, segment_time, total } => {
            let name = {
                let a = app.borrow();
                let Some(seg) = a.profile.segments.get(index) else { return };
                let (delta, sign) = match seg.pb {
                    Some(pb) => {
                        let d = total - pb;
                        (if d >= 0.0 { format!("+{d:.1}") } else { format!("{d:.1}") }, if d > 0.001 { 1 } else if d < -0.001 { -1 } else { 0 })
                    }
                    None => ("-".into(), 0),
                };
                if let Some(mut row) = a.rows.row_data(index) {
                    row.split = fmt_time(segment_time).into();
                    row.total = fmt_time(total).into();
                    row.delta = delta.into();
                    row.sign = sign;
                    a.rows.set_row_data(index, row);
                }
                set_active(&a, index as i32 + 1);
                seg.name.clone()
            };
            if skipped > 0 {
                log(app, &format!("[WARN] Skipped {skipped} segment(s)"));
            }
            log(app, &format!("Split: {name} [{}]", fmt_time(segment_time)));
        }
        Ev::Done(total, times) => run_complete(app, total, times),
        Ev::Reset => {
            let mut a = app.borrow_mut();
            a.start = None;
            show_segments(&a);
            if let Some(ui) = a.ui.upgrade() {
                ui.set_run_state(1);
            }
        }
        Ev::Stopped => {
            let mut a = app.borrow_mut();
            a.running = false;
            a.start = None;
            if let Some(ui) = a.ui.upgrade() {
                ui.set_running(false);
                ui.set_run_state(0);
            }
        }
        Ev::Update(tag, notes) => offer_update(app, &tag, &notes),
    }
}

fn run_complete(app: &Shared, total: f64, times: Vec<Option<f64>>) {
    let (previous, protection) = {
        let mut a = app.borrow_mut();
        a.start = None;
        set_clock(&a, &fmt_time(total), true);
        if let Some(ui) = a.ui.upgrade() {
            ui.set_run_state(3);
        }
        (a.profile.segments.last().and_then(|s| s.pb), a.profile.pb_protection_seconds)
    };
    log(app, &format!("Run complete: {}", fmt_time(total)));
    if !run::offer_pb(previous, total, protection) {
        if previous.is_some_and(|p| total < p) {
            log(app, &format!("[INFO] PB improvement of {} is over the PB protection, not saved", fmt_time(previous.unwrap_or(total) - total)));
        }
        return;
    }
    let text = format!("Run completed in {}\nPrevious PB: {}\n\nSave new Personal Best times?", fmt_time(total), previous.map_or("none".into(), fmt_time));
    if ask("New Personal Best!", &text) {
        {
            let mut a = app.borrow_mut();
            run::apply_pb(&mut a.profile, &times);
        }
        save(app);
        let a = app.borrow();
        show_segments(&a);
        set_clock(&a, &fmt_time(total), true);
    }
}

fn offer_update(app: &Shared, tag: &str, notes: &str) {
    let text = format!("AutoSplitter {tag} is available (you have {VERSION}).\n\nWhat's new:\n{notes}\n\nUpdate now?");
    if !ask("Update available", &text) {
        return;
    }
    let kind = update::InstallKind::of(&paths::exe_dir());
    if kind == update::InstallKind::Manual {
        let _ = update::shell_open(std::path::Path::new(&format!("{}/tag/{tag}", update::RELEASES)), "");
        return;
    }
    log(app, &format!("[INFO] Downloading {tag}..."));
    let (tag, weak) = (tag.to_string(), app.borrow().ui.clone());
    std::thread::spawn(move || {
        let result = update::install(kind, &tag, &paths::exe_dir());
        let _ = weak.upgrade_in_event_loop(move |_| match result {
            Ok(()) => {
                let _ = slint::quit_event_loop();
            }
            Err(e) => tell("Update failed", &e),
        });
    });
}

fn input(app: &Shared, key: Input, ts: f64) {
    if app.borrow().ui.upgrade().is_some_and(|ui| ui.get_debug()) {
        log(app, &format!("[DEBUG] Input: {}", input::key_name(key)));
    }
    if app.borrow().binding {
        {
            let mut a = app.borrow_mut();
            a.binding = false;
            if key != input::ESCAPE {
                a.profile.split_key = input::key_name(key);
            }
            if let Some(ui) = a.ui.upgrade() {
                ui.set_binding(false);
                ui.set_split_key(a.profile.split_key.as_str().into());
            }
        }
        log(app, "[INFO] Split key set; Save Settings keeps it");
        return;
    }
    let split = input::parse_key(&app.borrow().profile.split_key);
    match key {
        input::F9 => start(app),
        input::F10 => reset(app),
        input::F11 => stop(app),
        k if k == split && app.borrow().running => {
            let _ = app.borrow().cmd.send(Cmd::Split(ts));
        }
        _ => {}
    }
}

fn open_editor(app: &Shared, path: &std::path::Path) {
    let dir = paths::exe_dir();
    let editor = ["ProfileEditor.exe", "profile-editor.exe", "profile-editor"].iter().map(|n| dir.join(n)).find(|p| p.is_file());
    match editor.map(|e| std::process::Command::new(e).arg(path).spawn()) {
        Some(Ok(_)) => {}
        Some(Err(e)) => log(app, &format!("[ERROR] Profile Editor: {e}")),
        None => log(app, "[ERROR] ProfileEditor.exe is not beside AutoSplitter.exe"),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let exe_dir = paths::exe_dir();
    update::clean_up(&exe_dir);
    let profiles = paths::profiles_dir();
    paths::ensure_profiles(&profiles)?;

    // The overlay: see-through, above the game, no taskbar button, no focus.
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .with_winit_window_attributes_hook(|attrs| {
            if attrs.title != "AutoSplitter Overlay" {
                return attrs;
            }
            use slint::winit_030::winit::window::WindowLevel;
            let attrs = attrs.with_transparent(true).with_decorations(false).with_window_level(WindowLevel::AlwaysOnTop).with_active(false);
            #[cfg(windows)]
            let attrs = {
                use slint::winit_030::winit::platform::windows::WindowAttributesExtWindows;
                attrs.with_skip_taskbar(true)
            };
            attrs
        })
        .select()?;

    let ui = MainWindow::new()?;
    let overlay = Overlay::new()?;
    ui.set_version(VERSION.into());
    let (cmd_tx, cmd_rx) = mpsc::channel();
    let (ev_tx, ev_rx) = mpsc::channel();
    let backend_tx = ev_tx.clone();
    std::thread::spawn(move || backend(cmd_rx, backend_tx));

    let rows = Rc::new(VecModel::<Row>::default());
    let log_lines = Rc::new(VecModel::<SharedString>::default());
    ui.set_rows(rows.clone().into());
    overlay.set_rows(rows.clone().into());
    ui.set_log(log_lines.clone().into());
    let app: Shared = Rc::new(RefCell::new(App {
        ui: ui.as_weak(),
        overlay: overlay.clone_strong(),
        cmd: cmd_tx,
        profile: Profile::default(),
        path: None,
        files: Vec::new(),
        rows,
        log: log_lines,
        start: None,
        running: false,
        binding: false,
    }));
    log(&app, &format!("[INFO] AutoSplitter {VERSION}, profiles in {}", profiles.display()));

    let (key_tx, key_rx) = mpsc::channel::<(Input, f64)>();
    let key_tx = Mutex::new(key_tx);
    // The time is taken in the hook, so a manual split is not late by the 50 ms poll below.
    match input::start(move |k| drop(key_tx.lock().map(|t| t.send((k, now()))))) {
        Ok(()) => log(&app, "[INFO] Global keys ready: F9 start, F10 reset, F11 stop"),
        Err(e) => log(&app, &format!("[ERROR] Global keys: {e}")),
    }

    {
        let check_tx = ev_tx.clone();
        std::thread::spawn(move || {
            // No internet or no release: nothing to say.
            if let Ok((tag, notes)) = update::latest()
                && update::is_newer(&tag, VERSION)
            {
                let _ = check_tx.send(Ev::Update(tag, notes));
            }
        });
    }

    let timer = slint::Timer::default();
    {
        let app = app.clone();
        timer.start(slint::TimerMode::Repeated, Duration::from_millis(50), move || {
            while let Ok(ev) = ev_rx.try_recv() {
                handle(&app, ev);
            }
            while let Ok((k, ts)) = key_rx.try_recv() {
                input(&app, k, ts);
            }
            let a = app.borrow();
            if let Some(start) = a.start {
                set_clock(&a, &fmt_time(now() - start), false);
            }
        });
    }

    let a = app.clone();
    ui.on_start(move || start(&a));
    let a = app.clone();
    ui.on_reset(move || reset(&a));
    let a = app.clone();
    ui.on_stop(move || stop(&a));
    let a = app.clone();
    ui.on_debug_changed(move || {
        let a = a.borrow();
        let _ = a.cmd.send(Cmd::Debug(a.ui.upgrade().is_some_and(|ui| ui.get_debug())));
    });
    let a = app.clone();
    ui.on_overlay_changed(move || show_overlay(&a.borrow()));
    let o = overlay.as_weak();
    overlay.on_drag(move || {
        use slint::winit_030::WinitWindowAccessor;
        if let Some(o) = o.upgrade() {
            o.window().with_winit_window(|w| w.drag_window());
        }
    });
    let a = app.clone();
    ui.on_select_profile(move |i| load(&a, i.max(0) as usize));
    let a = app.clone();
    ui.on_reload(move || {
        let path = a.borrow().path.clone();
        refresh(&a, path);
    });
    let a = app.clone();
    ui.on_save_settings(move || {
        read_settings(&mut a.borrow_mut());
        save(&a);
    });
    let a = app.clone();
    ui.on_bind_key(move || {
        let mut app = a.borrow_mut();
        app.binding = !app.binding;
        if let Some(ui) = app.ui.upgrade() {
            ui.set_binding(app.binding);
        }
    });
    let a = app.clone();
    ui.on_set_region(move || {
        let Some(ui) = a.borrow().ui.upgrade() else { return };
        let pos = ui.window().position();
        let app = a.clone();
        let picked = picker::pick_region((pos.x + 50, pos.y + 50), move |r| {
            app.borrow_mut().profile.region = r;
            if let Some(ui) = app.borrow().ui.upgrade() {
                ui.set_region(format!("{},{},{},{}", r[0], r[1], r[2], r[3]).into());
            }
            save(&app);
        });
        if let Err(e) = picked {
            log(&a, &format!("[ERROR] Set Region: {e}"));
        }
    });
    let a = app.clone();
    ui.on_edit(move || {
        let path = a.borrow().path.clone();
        if let Some(p) = path {
            open_editor(&a, &p);
        }
    });
    let a = app.clone();
    ui.on_browse(move || {
        let Some(file) = rfd::FileDialog::new().add_filter("Profile", &["xml"]).pick_file() else { return };
        let dir = paths::profiles_dir();
        let target = dir.join(file.file_name().unwrap_or_default());
        if file.parent() != Some(dir.as_path()) {
            if target.exists() {
                return tell("Browse", &format!("{} already exists in the profile folder.", target.display()));
            }
            if let Err(e) = std::fs::copy(&file, &target) {
                return tell("Browse", &e.to_string());
            }
        }
        refresh(&a, Some(target));
    });
    let a = app.clone();
    ui.on_new_profile(move || {
        let Some(file) = rfd::FileDialog::new().set_directory(paths::profiles_dir()).set_file_name("new_profile.xml").add_filter("Profile", &["xml"]).save_file() else { return };
        let mission = file.file_stem().unwrap_or_default().to_string_lossy().into_owned();
        if let Err(e) = Profile::new_template("Game", &mission).save(&file) {
            return tell("New profile", &e.to_string());
        }
        refresh(&a, Some(file.clone()));
        open_editor(&a, &file);
    });
    let a = app.clone();
    ui.on_delete_profile(move || {
        let Some(path) = a.borrow().path.clone() else { return };
        if ask("Delete profile", &format!("Delete {}?\nIts personal bests are deleted too.", path.display())) {
            match std::fs::remove_file(&path) {
                Ok(()) => refresh(&a, None),
                Err(e) => tell("Delete profile", &e.to_string()),
            }
        }
    });
    ui.window().on_close_requested(|| {
        let _ = slint::quit_event_loop();
        slint::CloseRequestResponse::HideWindow
    });

    refresh(&app, None);
    ui.show()?;
    slint::run_event_loop_until_quit()?;
    let _ = app.borrow().cmd.send(Cmd::Stop);
    Ok(())
}
