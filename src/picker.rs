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
        let s = p.window().scale_factor();
        let (x0, x1) = (p.get_x0().min(p.get_x1()) * s, p.get_x0().max(p.get_x1()) * s);
        let (y0, y1) = (p.get_y0().min(p.get_y1()) * s, p.get_y0().max(p.get_y1()) * s);
        close();
        // A click without a drag is not a region.
        if x1 - x0 >= 4.0 && y1 - y0 >= 4.0 {
            done([m[0] + x0 as i32, m[1] + y0 as i32, m[0] + x1.ceil() as i32, m[1] + y1.ceil() as i32]);
        }
    });
    picker.on_cancel(close);
    picker.show().map_err(|e| e.to_string())?;
    OPEN.with(|o| *o.borrow_mut() = Some(picker));
    Ok(())
}

// Hidden now, dropped after the callback that closed it has returned.
fn close() {
    OPEN.with(|o| o.borrow().as_ref().map(|p| p.hide()));
    slint::Timer::single_shot(std::time::Duration::ZERO, || drop(OPEN.with(|o| o.borrow_mut().take())));
}
