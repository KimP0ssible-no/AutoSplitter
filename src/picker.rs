//! Choosing the OCR region: drag a rectangle on a full-screen picture of one monitor.

use crate::ui::RegionPicker;
use slint::{ComponentHandle, Image, PhysicalPosition, PhysicalSize, Rgba8Pixel, SharedPixelBuffer};
use std::cell::RefCell;
use std::rc::Rc;

thread_local! {
    // The open picker; a Slint window closes when its last handle goes.
    static OPEN: RefCell<Option<RegionPicker>> = const { RefCell::new(None) };
}

/// Opens the picker on the monitor that holds `point` (desktop pixels), else the primary
/// monitor, and calls `done` with the chosen region (left, top, right, bottom in desktop
/// pixels). Tab moves the picker to the next monitor.
pub fn pick_region(point: (i32, i32), done: impl Fn([i32; 4]) + 'static) -> Result<(), String> {
    let m = crate::capture::monitor_at(point).ok_or("no monitor found")?;
    open(m, Rc::new(done))
}

fn open(m: [i32; 4], done: Rc<dyn Fn([i32; 4])>) -> Result<(), String> {
    let frame = crate::capture::screenshot(m)?;
    let mut pixels = SharedPixelBuffer::<Rgba8Pixel>::new(frame.width as u32, frame.height as u32);
    for (y, row) in pixels.make_mut_slice().chunks_mut(frame.width).enumerate() {
        for (x, p) in row.iter_mut().enumerate() {
            let i = y * frame.stride + x * 4;
            *p = Rgba8Pixel::new(frame.bgra[i + 2], frame.bgra[i + 1], frame.bgra[i], 255);
        }
    }
    let picker = RegionPicker::new().map_err(|e| e.to_string())?;
    picker.set_shot(Image::from_rgba8(pixels));
    picker.window().set_position(PhysicalPosition::new(m[0], m[1]));
    picker.window().set_size(PhysicalSize::new((m[2] - m[0]) as u32, (m[3] - m[1]) as u32));
    let again = done.clone();
    picker.on_next_monitor(move || {
        let next = crate::capture::next_monitor(m);
        if next != m {
            OPEN.with(|o| o.borrow().as_ref().map(|p| p.hide()));
            let done = again.clone();
            // After this callback returns: the old picker goes, the next one opens.
            slint::Timer::single_shot(std::time::Duration::ZERO, move || {
                drop(OPEN.with(|o| o.borrow_mut().take()));
                drop(open(next, done));
            });
        }
    });
    let weak = picker.as_weak();
    picker.on_done(move || {
        let Some(p) = weak.upgrade() else { return };
        let region = to_desktop(m, (p.get_view_width(), p.get_view_height()), [p.get_x0(), p.get_y0(), p.get_x1(), p.get_y1()]);
        close();
        // A click without a drag is not a region.
        if let Some(r) = region.filter(|r| r[2] - r[0] >= 4 && r[3] - r[1] >= 4) {
            done(r);
        }
    });
    picker.on_cancel(close);
    picker.show().map_err(|e| e.to_string())?;
    OPEN.with(|o| *o.borrow_mut() = Some(picker));
    Ok(())
}

/// A drag on the picture into desktop pixels of monitor `m`. The picture shows exactly
/// that monitor, so a share of the picture is that share of the monitor, whatever size
/// or DPI scale Windows gave the picker window.
fn to_desktop(m: [i32; 4], (vw, vh): (f32, f32), [x0, y0, x1, y1]: [f32; 4]) -> Option<[i32; 4]> {
    if vw <= 0.0 || vh <= 0.0 {
        return None;
    }
    let (w, h) = ((m[2] - m[0]) as f32, (m[3] - m[1]) as f32);
    let x = |v: f32| m[0] + (v.clamp(0.0, vw) / vw * w).round() as i32;
    let y = |v: f32| m[1] + (v.clamp(0.0, vh) / vh * h).round() as i32;
    Some([x(x0.min(x1)), y(y0.min(y1)), x(x0.max(x1)), y(y0.max(y1))])
}

// Hidden now, dropped after the callback that closed it has returned.
fn close() {
    OPEN.with(|o| o.borrow().as_ref().map(|p| p.hide()));
    slint::Timer::single_shot(std::time::Duration::ZERO, || drop(OPEN.with(|o| o.borrow_mut().take())));
}

#[cfg(test)]
mod tests {
    use super::to_desktop;

    #[test]
    fn drag_to_desktop_pixels() {
        // A 2560x1440 monitor right of the primary, shown at 150 % (1706.67 x 960 logical).
        let m = [1920, 0, 4480, 1440];
        let view = (2560.0 / 1.5, 1440.0 / 1.5);
        assert_eq!(to_desktop(m, view, [10.0, 150.0, 580.0, 310.0]), Some([1935, 225, 2790, 465]));
        // Dragged right to left, and past the edge.
        assert_eq!(to_desktop(m, view, [5000.0, 0.0, 0.0, 960.0]), Some([1920, 0, 4480, 1440]));
        assert_eq!(to_desktop(m, (0.0, 0.0), [0.0; 4]), None);
    }
}
