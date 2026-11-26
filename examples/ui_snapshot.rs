//! Render a window off-screen to a PNG, to check the look without opening a window:
//! `cargo run --example ui_snapshot -- out.png main|overlay|editor [overlay scale]`.

use autosplitter::profile::Profile;
use autosplitter::run::fmt_time;
use autosplitter::ui::{EditorWindow, MainWindow, Overlay, Row};
use slint::platform::software_renderer::{MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::{ComponentHandle, ModelRc, PhysicalSize, SharedString, StandardListViewItem, VecModel};
use std::rc::Rc;

struct Offscreen(Rc<MinimalSoftwareWindow>);

impl Platform for Offscreen {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

fn model<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(items))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (out, scene) = (args[0].clone(), args.get(1).cloned().unwrap_or("main".into()));
    let scale: f32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1.0);
    let (width, height) = match scene.as_str() {
        "overlay" => ((620.0 * scale) as u32, (390.0 * scale) as u32),
        "editor" => (940, 900),
        _ => (860, 900),
    };
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Offscreen(window.clone()))).unwrap();
    window.set_size(PhysicalSize::new(width, height));

    let p = Profile::parse(include_str!("../profiles/example_paradise_lost.xml")).unwrap();
    let rows: Vec<Row> = p
        .segments
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let done = i < 3;
            let total = s.pb.unwrap_or(0.0) + if i == 1 { 2.4 } else { -1.2 };
            Row {
                name: s.name.as_str().into(),
                split: if done { fmt_time(30.0 + i as f64).into() } else { "".into() },
                total: if done { fmt_time(total).into() } else { "".into() },
                pb: s.pb.map_or("-".into(), fmt_time).into(),
                delta: if done { (if i == 1 { "+2.4" } else { "-1.2" }).into() } else { "".into() },
                sign: if !done { 0 } else if i == 1 { 1 } else { -1 },
            }
        })
        .collect();

    // Kept alive until the frame is drawn.
    let _component: Box<dyn std::any::Any> = match scene.as_str() {
        "overlay" => {
            let o = Overlay::new().unwrap();
            o.set_scale(scale);
            o.set_heading(p.display_name().into());
            o.set_rows(model(rows));
            o.set_active_row(3);
            o.set_timer("2:41.380".into());
            o.set_pb_total("5:54.000".into());
            o.show().unwrap();
            Box::new(o)
        }
        "editor" => {
            let e = EditorWindow::new().unwrap();
            e.set_version("2.0.0".into());
            e.set_file("C:\\Users\\me\\AppData\\Roaming\\AutoSplitter\\profiles\\example_paradise_lost.xml".into());
            e.set_game(p.game.as_str().into());
            e.set_mission(p.mission.as_str().into());
            e.set_left("441".into());
            e.set_top("60".into());
            e.set_right("996".into());
            e.set_bottom("140".into());
            e.set_ocr_text("Enter the Estate Find a way into the Courtyard".into());
            e.set_ocr_ms("38 ms".into());
            let list = |v: &[String]| model(v.iter().map(|s| StandardListViewItem::from(s.as_str())).collect());
            e.set_from_items(list(&p.start_from));
            e.set_to_items(list(&p.start_to));
            let last = p.segments.len() - 1;
            let labels: Vec<String> = p
                .segments
                .iter()
                .enumerate()
                .map(|(i, s)| format!("{}{}{}", s.name, if s.pte > 0 { format!("  \u{b7}  blocked {} s", s.pte) } else { String::new() }, if i == last { "  \u{b7}  ends the run" } else { "" }))
                .collect();
            e.set_segment_items(list(&labels));
            e.set_segment_current(1);
            e.set_segment_name(p.segments[1].name.as_str().into());
            e.set_segment_pb(fmt_time(p.segments[1].pb.unwrap_or(0.0)).into());
            e.set_segment_keywords(p.segments[1].keywords.join("\n").into());
            e.show().unwrap();
            Box::new(e)
        }
        _ => {
            let ui = MainWindow::new().unwrap();
            ui.set_version("2.0.0".into());
            ui.set_heading(p.display_name().into());
            ui.set_run_state(2);
            ui.set_timer("2:41.380".into());
            ui.set_rows(model(rows));
            ui.set_active_row(3);
            ui.set_running(true);
            ui.set_profiles(model(vec![SharedString::from(p.display_name())]));
            ui.set_region("441,60,996,140".into());
            ui.set_ocr_text("Enter the Estate Find a way into the Courtyard".into());
            ui.set_ocr_ms("38 ms".into());
            ui.set_log(model(
                ["12:00:01 [INFO] AutoSplitter 2.0.0", "12:00:02 [INFO] OCR ready (en, 412 ms)", "12:00:09 Timer started: Enter the Estate", "12:00:41 Split: Courtyard to door [0:30.000]"]
                    .map(SharedString::from)
                    .to_vec(),
            ));
            ui.show().unwrap();
            Box::new(ui)
        }
    };

    let mut pixels = vec![PremultipliedRgbaColor::default(); width as usize * height as usize];
    window.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, width as usize);
    });
    let img = image::RgbaImage::from_fn(width, height, |x, y| {
        let p = pixels[(y * width + x) as usize];
        image::Rgba([p.red, p.green, p.blue, p.alpha])
    });
    img.save(out).unwrap();
}
