mod app;
mod capture;
mod hotkey;
mod renderer;
mod state;

use std::sync::{Arc, Mutex};

use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};

fn main() -> eframe::Result {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    let shared = state::new_shared();
    let frame_state: renderer::FrameState = Arc::new(Mutex::new(None));

    // Capture thread: DXGI → CPU Vec<u8> → frame_state
    {
        let frame_state = frame_state.clone();
        std::thread::spawn(move || {
            let mut capturer = match capture::Capturer::new() {
                Ok(c) => c,
                Err(e) => { eprintln!("[capture] init failed: {e}"); return; }
            };
            loop {
                match capturer.next_frame(100) {
                    Ok(Some(frame)) => {
                        *frame_state.lock().unwrap() = Some(Arc::new(frame));
                    }
                    Ok(None) => {}
                    Err(e) => {
                        eprintln!("[capture] {e} — reconnecting");
                        if capturer.reconnect().is_err() {
                            break;
                        }
                    }
                }
            }
        });
    }

    // Renderer thread: GDI fullscreen window, reads frame_state + app state
    {
        let frame_state = frame_state.clone();
        let app_state = shared.clone();
        std::thread::spawn(move || {
            renderer::run_overlay(frame_state, app_state);
        });
    }

    // Hotkey thread: Win+= toggles enabled, shows/hides overlay
    {
        let state = shared.clone();
        std::thread::spawn(move || {
            hotkey::hotkey_loop(state);
        });
    }

    // egui settings panel on main thread
    let state_for_egui = shared.clone();
    eframe::run_native(
        "clear-view settings",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([320.0, 220.0])
                .with_resizable(false),
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(app::ClearViewApp::new(cc, state_for_egui)))),
    )
}
