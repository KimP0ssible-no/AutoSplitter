//! Global keys and mouse buttons, seen while a game has focus. F9, F10 and F11 are
//! registered hot keys; the split key and key binding use low-level hooks, which see a
//! key whatever modifiers are held, and pass it on to the game.

/// A key press (Windows virtual-key code) or mouse button 3 to 5.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    Key(u32),
    Mouse(u8),
}

pub const F9: Input = Input::Key(0x78);
pub const F10: Input = Input::Key(0x79);
pub const F11: Input = Input::Key(0x7A);
pub const ESCAPE: Input = Input::Key(0x1B);

/// The name profiles store: "Mouse3", "F5" or "VK123".
pub fn key_name(input: Input) -> String {
    match input {
        Input::Mouse(b) => format!("Mouse{b}"),
        Input::Key(vk @ 0x70..=0x7B) => format!("F{}", vk - 0x6F),
        Input::Key(vk) => format!("VK{vk}"),
    }
}

/// Reads a stored name; anything unknown is the middle mouse button, as before.
pub fn parse_key(name: &str) -> Input {
    let num = |s: &str| s.parse::<u32>().ok();
    if let Some(b) = name.strip_prefix("Mouse").and_then(num).filter(|b| (1..=5).contains(b)) {
        return Input::Mouse(b as u8);
    }
    if let Some(n) = name.strip_prefix('F').and_then(num).filter(|n| (1..=12).contains(n)) {
        return Input::Key(0x6F + n);
    }
    name.strip_prefix("VK").and_then(num).filter(|&v| v > 0).map_or(Input::Mouse(3), Input::Key)
}

#[cfg(windows)]
mod hooks {
    use super::Input;
    use std::sync::{Mutex, OnceLock};
    use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::Input::KeyboardAndMouse::{MOD_NOREPEAT, RegisterHotKey};
    use windows::Win32::UI::WindowsAndMessaging::*;

    // Hot key ids 1 to 3 are F9 to F11.
    const HOT_KEYS: [(i32, u32); 3] = [(1, 0x78), (2, 0x79), (3, 0x7A)];

    type Handler = Box<dyn Fn(Input) + Send + Sync>;
    static HANDLER: OnceLock<Handler> = OnceLock::new();
    // Held keys, so auto-repeat does not fire again.
    static DOWN: Mutex<Vec<u32>> = Mutex::new(Vec::new());

    unsafe extern "system" fn keyboard(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
        if code == HC_ACTION as i32 {
            // SAFETY: for WH_KEYBOARD_LL with HC_ACTION, lParam points to a KBDLLHOOKSTRUCT.
            let vk = unsafe { (*(l.0 as *const KBDLLHOOKSTRUCT)).vkCode };
            // F9 to F11 arrive as hot keys.
            if HOT_KEYS.iter().any(|&(_, k)| k == vk) {
                // SAFETY: passes the event on unchanged, as every hook must.
                return unsafe { CallNextHookEx(None, code, w, l) };
            }
            let mut down = DOWN.lock().unwrap_or_else(|e| e.into_inner());
            match w.0 as u32 {
                WM_KEYDOWN | WM_SYSKEYDOWN if !down.contains(&vk) => {
                    down.push(vk);
                    drop(down);
                    if let Some(h) = HANDLER.get() {
                        h(Input::Key(vk));
                    }
                }
                WM_KEYUP | WM_SYSKEYUP => down.retain(|&k| k != vk),
                _ => {}
            }
        }
        // SAFETY: passes the event on unchanged, as every hook must.
        unsafe { CallNextHookEx(None, code, w, l) }
    }

    unsafe extern "system" fn mouse(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
        if code == HC_ACTION as i32 {
            let button = match w.0 as u32 {
                WM_MBUTTONDOWN => Some(3),
                WM_XBUTTONDOWN => {
                    // SAFETY: for WH_MOUSE_LL with HC_ACTION, lParam points to an MSLLHOOKSTRUCT.
                    let data = unsafe { (*(l.0 as *const MSLLHOOKSTRUCT)).mouseData };
                    Some(if (data >> 16) as u16 == XBUTTON2 { 5 } else { 4 })
                }
                _ => None,
            };
            if let (Some(b), Some(h)) = (button, HANDLER.get()) {
                h(Input::Mouse(b));
            }
        }
        // SAFETY: passes the event on unchanged, as every hook must.
        unsafe { CallNextHookEx(None, code, w, l) }
    }

    /// Starts listening; the error names each part that could not be set up.
    pub fn start(handler: impl Fn(Input) + Send + Sync + 'static) -> Result<(), String> {
        HANDLER.set(Box::new(handler)).map_err(|_| "global input already started".to_string())?;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            // SAFETY: the hook procedures live for the whole program, and this thread
            // runs the message loop the hooks and hot keys are delivered through.
            unsafe {
                let module = GetModuleHandleW(None).ok().map(Into::into);
                let mut problems = Vec::new();
                if let Err(e) = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard), module, 0) {
                    problems.push(format!("keyboard hook: {}", e.message()));
                }
                if let Err(e) = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse), module, 0) {
                    problems.push(format!("mouse hook: {}", e.message()));
                }
                for (id, vk) in HOT_KEYS {
                    if let Err(e) = RegisterHotKey(None, id, MOD_NOREPEAT, vk) {
                        problems.push(format!("F{}: {}", vk - 0x6F, e.message()));
                    }
                }
                let _ = tx.send(if problems.is_empty() { Ok(()) } else { Err(problems.join("; ")) });
                let mut msg = MSG::default();
                while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    if msg.message == WM_HOTKEY
                        && let (Some(&(_, vk)), Some(h)) = (HOT_KEYS.iter().find(|(id, _)| *id as usize == msg.wParam.0), HANDLER.get())
                    {
                        h(Input::Key(vk));
                    }
                }
            }
        });
        rx.recv().unwrap_or_else(|_| Err("the input thread stopped".into()))
    }
}

#[cfg(windows)]
pub use hooks::start;

/// Global input needs Windows; elsewhere nothing is delivered.
#[cfg(not(windows))]
pub fn start(_handler: impl Fn(Input) + Send + Sync + 'static) -> Result<(), String> {
    Err("global keys need Windows".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        for name in ["Mouse3", "Mouse5", "F1", "F12", "VK65"] {
            assert_eq!(key_name(parse_key(name)), name);
        }
        assert_eq!(parse_key("F9"), F9);
        assert_eq!(parse_key(""), Input::Mouse(3));
        assert_eq!(parse_key("Banana"), Input::Mouse(3));
    }
}
