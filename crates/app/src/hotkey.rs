use cv_core::SharedState;
use windows::Win32::UI::{
    Input::KeyboardAndMouse::{MOD_NOREPEAT, MOD_WIN, RegisterHotKey, VK_OEM_PLUS},
    WindowsAndMessaging::*,
};

const HOTKEY_ID: i32 = 1;

/// Registers Win+= as a global toggle and pumps WM_HOTKEY messages.
pub fn hotkey_loop(state: SharedState) {
    unsafe {
        if RegisterHotKey(None, HOTKEY_ID, MOD_WIN | MOD_NOREPEAT, VK_OEM_PLUS.0 as u32).is_err() {
            eprintln!("[hotkey] RegisterHotKey failed — Win+= unavailable");
        }

        let mut msg = MSG::default();
        loop {
            if PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_HOTKEY && msg.wParam.0 as i32 == HOTKEY_ID {
                    let mut s = state.write();
                    s.enabled = !s.enabled;
                }
            } else {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
    }
}
