use windows::Win32::UI::{
    Input::KeyboardAndMouse::{MOD_NOREPEAT, MOD_WIN, RegisterHotKey, UnregisterHotKey, VK_OEM_PLUS},
    WindowsAndMessaging::*,
};

use crate::state::SharedState;

const HOTKEY_ID: i32 = 1;

/// Registers Win+= as a global toggle hotkey and pumps messages on the calling
/// thread until `running` is cleared. When the hotkey fires it toggles
/// `AppState::enabled` and shows or hides the overlay window.
pub fn hotkey_loop(
    overlay: crate::overlay::SendHwnd,
    state: SharedState,
    running: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    unsafe {
        if RegisterHotKey(
            None,
            HOTKEY_ID,
            MOD_WIN | MOD_NOREPEAT,
            VK_OEM_PLUS.0 as u32,
        )
        .is_err()
        {
            eprintln!("[hotkey] RegisterHotKey failed — toggle hotkey unavailable");
        }

        let mut msg = MSG::default();
        while running.load(std::sync::atomic::Ordering::Relaxed) {
            if PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_HOTKEY && msg.wParam.0 as i32 == HOTKEY_ID {
                    let enabled = {
                        let mut s = state.write();
                        s.enabled = !s.enabled;
                        s.enabled
                    };

                    if enabled {
                        let _ = ShowWindow(overlay.0, SW_SHOW);
                    } else {
                        let _ = ShowWindow(overlay.0, SW_HIDE);
                    }
                }
            } else {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }

        let _ = UnregisterHotKey(None, HOTKEY_ID);
    }
}
