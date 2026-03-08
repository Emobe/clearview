mod app;
mod capture;
mod hotkey;
mod overlay;
mod renderer;
mod state;

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};

fn main() -> eframe::Result {
    // DPI awareness must be set before any window is created.
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    let shared = state::new_shared();
    let running = Arc::new(AtomicBool::new(true));

    // Overlay window (created before eframe takes over the main thread)
    let overlay = overlay::create_overlay().expect("failed to create overlay window");

    // D3D11 device shared between capture and render threads
    let (device, ctx) = renderer::create_d3d11_device().expect("failed to create D3D11 device");

    // Channel: capture → render (capacity 1 so render always gets the freshest frame)
    let (tx, rx) = crossbeam_channel::bounded::<
        windows::Win32::Graphics::Direct3D11::ID3D11Texture2D,
    >(1);

    // Capture thread
    {
        let device = device.clone();
        let running = running.clone();
        std::thread::spawn(move || {
            capture::capture_loop(device, tx, running);
        });
    }

    // Render thread — HWND is not Send, so we pass the raw isize and reconstruct inside.
    {
        let hwnd_raw = overlay.0.0 as isize;
        let device = device.clone();
        let state = shared.clone();
        let running = running.clone();
        std::thread::spawn(move || {
            let hwnd = windows::Win32::Foundation::HWND(hwnd_raw as *mut _);
            renderer::render_loop(hwnd, device, ctx, rx, state, running);
        });
    }

    // Hotkey thread
    {
        let state = shared.clone();
        let running = running.clone();
        std::thread::spawn(move || {
            hotkey::hotkey_loop(overlay, state, running);
        });
    }

    // egui settings window — blocks until the user closes it
    let state_for_egui = shared.clone();
    let result = eframe::run_native(
        "clear-view settings",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([320.0, 200.0])
                .with_resizable(false),
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(app::ClearViewApp::new(cc, state_for_egui)))),
    );

    running.store(false, Ordering::Relaxed);

    result
}
