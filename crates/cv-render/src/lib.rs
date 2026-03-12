mod appbar;
mod gfx;

use std::{
    cell::RefCell,
    mem::size_of,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::Instant,
};

use windows::{
    Win32::{
        Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DispatchMessageW, GetCursorPos, GetMessageW,
            HTTRANSPARENT, IDC_ARROW, LWA_ALPHA, LoadCursorW, MSG, PostQuitMessage,
            RegisterClassExW, RegisterWindowMessageW, SW_HIDE, SW_SHOW, SWP_NOACTIVATE,
            SWP_NOZORDER, SetLayeredWindowAttributes, SetTimer, SetWindowDisplayAffinity,
            SetWindowPos, ShowCursor, ShowWindow, TranslateMessage, WDA_EXCLUDEFROMCAPTURE,
            WM_DESTROY, WM_NCHITTEST, WM_TIMER, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
            WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
        },
    },
    core::w,
};

use cv_core::{
    ColorFilter, DisplayMode, Edge, Frame, FrameState, Interpolation, OutputInfo, SharedState,
};
use gfx::WgpuState;

struct WindowData {
    wgpu: WgpuState,
    frame_state: FrameState,
    app_state: SharedState,
    smooth_x: f32,
    smooth_y: f32,
    last_tick: Instant,
    /// All monitors enumerated at startup (virtual-screen coords).
    outputs: Vec<OutputInfo>,
    /// Index into `outputs` for the currently-active monitor.
    active_output_idx: u32,
    /// Top-left of the active monitor in virtual screen coordinates.
    monitor_left: i32,
    monitor_top: i32,
    /// Width/height of the active monitor.
    screen_w: i32,
    screen_h: i32,
    /// Shared with the capture thread — signals which output to duplicate.
    desired_output: Arc<AtomicU32>,
    callback_msg: u32,
    // Change-detection: what is currently applied to the window.
    cur_enabled: bool,
    cur_mode: DisplayMode,
    cur_panel_size: u32,
    appbar_active: bool,
    // GPU write caching — skip redundant uploads/uniform writes.
    last_frame: Option<Arc<Frame>>,
    last_crop: [f32; 4],
    last_color_mode: u32,
    last_interp_mode: u32,
    last_cursor_x: u32,
    last_cursor_y: u32,
    // Software cursor — hardware cursor state.
    cursor_hidden: bool,
}

thread_local! {
    static WIN_DATA: RefCell<Option<WindowData>> = const { RefCell::new(None) };
}

/// Creates the overlay window and blocks on its message loop.
pub fn run_overlay(
    frame_state: FrameState,
    app_state: SharedState,
    outputs: Vec<OutputInfo>,
    desired_output: Arc<AtomicU32>,
) {
    // Use primary monitor (first in list) as the starting monitor.
    let primary = outputs.first().cloned().unwrap_or(OutputInfo {
        idx: 0,
        left: 0,
        top: 0,
        width: 1920,
        height: 1080,
    });
    let screen_w = primary.width as i32;
    let screen_h = primary.height as i32;

    let hwnd = unsafe {
        let hinstance: HINSTANCE = GetModuleHandleW(None).unwrap().into();
        let class_name = w!("clear_view_overlay");

        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinstance,
            lpszClassName: class_name,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassExW(&wc);

        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_NOACTIVATE | WS_EX_TOPMOST | WS_EX_TRANSPARENT,
            class_name,
            w!("clear-view"),
            WS_POPUP, // starts hidden
            primary.left,
            primary.top,
            screen_w,
            screen_h,
            None,
            None,
            Some(hinstance),
            None,
        )
        .expect("CreateWindowExW failed");

        SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE).ok();
        SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA).ok();
        hwnd
    };

    let callback_msg = unsafe { RegisterWindowMessageW(w!("ClearViewAppBar")) };

    // wgpu init (blocking) — window starts primary-monitor-sized.
    let wgpu = WgpuState::new(
        hwnd,
        screen_w as u32,
        screen_h as u32,
        primary.width,
        primary.height,
    );

    WIN_DATA.with(|d| {
        *d.borrow_mut() = Some(WindowData {
            wgpu,
            frame_state,
            app_state,
            smooth_x: screen_w as f32 / 2.0 + primary.left as f32,
            smooth_y: screen_h as f32 / 2.0 + primary.top as f32,
            last_tick: Instant::now(),
            active_output_idx: primary.idx,
            monitor_left: primary.left,
            monitor_top: primary.top,
            screen_w,
            screen_h,
            outputs,
            desired_output,
            callback_msg,
            cur_enabled: false,
            cur_mode: DisplayMode::Fullscreen,
            cur_panel_size: 300,
            appbar_active: false,
            last_frame: None,
            last_crop: [f32::NAN; 4],   // NAN != NAN → forces first write
            last_color_mode: u32::MAX,  // forces first write
            last_interp_mode: u32::MAX, // forces first write
            last_cursor_x: u32::MAX,    // forces first write
            last_cursor_y: u32::MAX,    // forces first write
            cursor_hidden: false,
        });
    });

    unsafe { SetTimer(Some(hwnd), 1, 16, None) };

    let mut msg = MSG::default();
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    // Clean up appbar on exit.
    WIN_DATA.with(|d| {
        if let Some(w) = d.borrow().as_ref() {
            if w.appbar_active {
                appbar::unregister(hwnd);
            }
        }
    });
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let callback_msg = WIN_DATA
        .with(|d| d.borrow().as_ref().map(|w| w.callback_msg))
        .unwrap_or(0);

    if msg != 0 && msg == callback_msg {
        handle_appbar_callback(hwnd, wparam);
        return LRESULT(0);
    }

    match msg {
        WM_DESTROY => unsafe {
            PostQuitMessage(0);
            LRESULT(0)
        },
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_TIMER => {
            on_timer(hwnd);
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn handle_appbar_callback(hwnd: HWND, wparam: WPARAM) {
    if wparam.0 != appbar::ABN_POSCHANGED {
        return;
    }

    let params = WIN_DATA.with(|d| {
        let b = d.borrow();
        let w = b.as_ref()?;
        if !w.appbar_active {
            return None;
        }
        match w.cur_mode {
            DisplayMode::Docked(e) => Some((e, w.cur_panel_size as i32, w.screen_w, w.screen_h)),
            _ => None,
        }
    });
    let Some((edge, thickness, sw, sh)) = params else {
        return;
    };

    // Borrow is dropped — safe to call SetWindowPos.
    let rect = appbar::reposition(hwnd, edge, thickness, sw, sh);
    move_window(hwnd, rect);
    WIN_DATA.with(|d| {
        if let Some(w) = d.borrow_mut().as_mut() {
            let (rw, rh) = rect_dims(rect);
            w.wgpu.resize(rw, rh);
        }
    });
}

fn on_timer(hwnd: HWND) {
    // ── Snapshot desired state ───────────────────────────────────────────────
    struct Snap {
        enabled: bool,
        mode: DisplayMode,
        panel_size: u32,
        zoom: f32,
        smooth_speed: f32,
        color_filter: ColorFilter,
        interpolation: Interpolation,
        frame: Option<Arc<Frame>>,
        cur_enabled: bool,
        cur_mode: DisplayMode,
        cur_panel_size: u32,
        appbar_active: bool,
        screen_w: i32,
        screen_h: i32,
        monitor_left: i32,
        monitor_top: i32,
        callback_msg: u32,
    }

    let snap = WIN_DATA.with(|d| {
        let b = d.borrow();
        let w = b.as_ref()?;
        let s = w.app_state.read();
        let frame = w.frame_state.lock().ok()?.clone();
        Some(Snap {
            enabled: s.enabled,
            mode: s.display_mode,
            panel_size: s.panel_size,
            zoom: s.zoom,
            smooth_speed: s.smooth_speed,
            color_filter: s.color_filter,
            interpolation: s.interpolation,
            frame,
            cur_enabled: w.cur_enabled,
            cur_mode: w.cur_mode,
            cur_panel_size: w.cur_panel_size,
            appbar_active: w.appbar_active,
            screen_w: w.screen_w,
            screen_h: w.screen_h,
            monitor_left: w.monitor_left,
            monitor_top: w.monitor_top,
            callback_msg: w.callback_msg,
        })
    });
    let Some(snap) = snap else { return };

    let sw = snap.screen_w;
    let sh = snap.screen_h;

    // ── State transitions ────────────────────────────────────────────────────
    let enabled_changed = snap.enabled != snap.cur_enabled;
    let mode_changed = snap.mode != snap.cur_mode;
    let panel_size_changed = snap.panel_size != snap.cur_panel_size;

    let need_transition =
        enabled_changed || (snap.enabled && mode_changed) || (snap.enabled && panel_size_changed);

    let mut new_appbar_active = snap.appbar_active;
    let mut new_rect: Option<RECT> = None;
    // cursor hide/show: +1 = hide, -1 = show, 0 = no change
    let mut cursor_delta: i32 = 0;

    if need_transition {
        if enabled_changed && !snap.enabled {
            // ── Disabling ─────────────────────────────────────────────────
            if snap.appbar_active {
                appbar::unregister(hwnd);
                new_appbar_active = false;
            }
            unsafe {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
            // Decide whether to restore hardware cursor (need current state).
            let hidden = WIN_DATA.with(|d| {
                d.borrow()
                    .as_ref()
                    .map(|w| w.cursor_hidden)
                    .unwrap_or(false)
            });
            if hidden {
                cursor_delta = -1;
            }
        } else if enabled_changed && snap.enabled {
            // ── Enabling ──────────────────────────────────────────────────
            if snap.appbar_active {
                appbar::unregister(hwnd);
                new_appbar_active = false;
            }
            match snap.mode {
                DisplayMode::Fullscreen => {
                    let ml = snap.monitor_left;
                    let mt = snap.monitor_top;
                    let rect = RECT {
                        left: ml,
                        top: mt,
                        right: ml + sw,
                        bottom: mt + sh,
                    };
                    new_rect = Some(rect);
                }
                DisplayMode::Docked(e) => {
                    let rect = appbar::register(
                        hwnd,
                        e,
                        snap.panel_size as i32,
                        sw,
                        sh,
                        snap.callback_msg,
                    );
                    new_rect = Some(rect);
                    new_appbar_active = true;
                }
            }
            if snap.mode == DisplayMode::Fullscreen {
                let hidden = WIN_DATA.with(|d| {
                    d.borrow()
                        .as_ref()
                        .map(|w| w.cursor_hidden)
                        .unwrap_or(false)
                });
                if !hidden {
                    cursor_delta = 1;
                }
            }
        } else if snap.enabled && mode_changed {
            // ── Mode change while enabled ─────────────────────────────────
            if snap.appbar_active {
                appbar::unregister(hwnd);
                new_appbar_active = false;
            }
            match snap.mode {
                DisplayMode::Fullscreen => {
                    let ml = snap.monitor_left;
                    let mt = snap.monitor_top;
                    let rect = RECT {
                        left: ml,
                        top: mt,
                        right: ml + sw,
                        bottom: mt + sh,
                    };
                    new_rect = Some(rect);
                    let hidden = WIN_DATA.with(|d| {
                        d.borrow()
                            .as_ref()
                            .map(|w| w.cursor_hidden)
                            .unwrap_or(false)
                    });
                    if !hidden {
                        cursor_delta = 1;
                    }
                }
                DisplayMode::Docked(e) => {
                    let rect = appbar::register(
                        hwnd,
                        e,
                        snap.panel_size as i32,
                        sw,
                        sh,
                        snap.callback_msg,
                    );
                    new_rect = Some(rect);
                    new_appbar_active = true;
                    let hidden = WIN_DATA.with(|d| {
                        d.borrow()
                            .as_ref()
                            .map(|w| w.cursor_hidden)
                            .unwrap_or(false)
                    });
                    if hidden {
                        cursor_delta = -1;
                    }
                }
            }
        } else if snap.enabled && panel_size_changed {
            // ── Panel size change while docked ────────────────────────────
            if let DisplayMode::Docked(e) = snap.mode {
                let rect = appbar::reposition(hwnd, e, snap.panel_size as i32, sw, sh);
                new_rect = Some(rect);
            }
        }

        // ── move_window outside any WIN_DATA borrow ───────────────────────
        if let Some(rect) = new_rect {
            move_window(hwnd, rect);
        }

        // ── ShowWindow (only on enable/disable) ───────────────────────────
        if enabled_changed && snap.enabled {
            unsafe {
                let _ = ShowWindow(hwnd, SW_SHOW);
            }
        }

        // ── ShowCursor after all window calls are done ────────────────────
        // ShowCursor uses an internal display counter, not a boolean.
        // Loop until the counter crosses the threshold so a single call
        // doesn't fail when the counter is already above/below target.
        if cursor_delta > 0 {
            unsafe { while ShowCursor(false) >= 0 {} }
        } else if cursor_delta < 0 {
            unsafe { while ShowCursor(true) < 0 {} }
        }

        // ── Write back tracking state + resize wgpu surface ──────────────
        WIN_DATA.with(|d| {
            let mut b = d.borrow_mut();
            let w = b.as_mut().unwrap();
            w.cur_enabled = snap.enabled;
            w.cur_mode = snap.mode;
            w.cur_panel_size = snap.panel_size;
            w.appbar_active = new_appbar_active;
            if cursor_delta == 1 {
                w.cursor_hidden = true;
            }
            if cursor_delta == -1 {
                w.cursor_hidden = false;
            }
            if let Some(rect) = new_rect {
                let (rw, rh) = rect_dims(rect);
                w.wgpu.resize(rw, rh);
            }
        });
    }

    // ── Render ───────────────────────────────────────────────────────────────
    if !snap.enabled {
        return;
    }

    let mut cursor = windows::Win32::Foundation::POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut cursor);
    }

    // ── Monitor follow: collect switch rect before taking the main borrow ───
    // We compute the move rect here (outside WIN_DATA borrow) so that
    // move_window → SetWindowPos → wnd_proc re-entry cannot deadlock.
    let monitor_move_rect: Option<RECT> = WIN_DATA.with(|d| {
        let mut b = d.borrow_mut();
        let w = b.as_mut().unwrap();

        let target = w.outputs.iter().find(|o| {
            cursor.x >= o.left
                && cursor.x < o.left + o.width as i32
                && cursor.y >= o.top
                && cursor.y < o.top + o.height as i32
        });
        let Some(target) = target else { return None };
        if target.idx == w.active_output_idx {
            return None;
        }

        let new_idx = target.idx;
        let nl = target.left;
        let nt = target.top;
        let nw = target.width;
        let nh = target.height;

        w.desired_output.store(new_idx, Ordering::Relaxed);
        w.active_output_idx = new_idx;
        w.monitor_left = nl;
        w.monitor_top = nt;
        w.screen_w = nw as i32;
        w.screen_h = nh as i32;

        w.wgpu.recreate_frame_texture(nw, nh);
        w.last_frame = None; // force re-upload on next frame

        if snap.mode == DisplayMode::Fullscreen {
            let rect = RECT {
                left: nl,
                top: nt,
                right: nl + nw as i32,
                bottom: nt + nh as i32,
            };
            w.wgpu.resize(nw, nh);
            Some(rect)
        } else {
            None
        }
    });

    // SetWindowPos is called here — borrow fully released, no re-entrancy risk.
    if let Some(rect) = monitor_move_rect {
        move_window(hwnd, rect);
    }

    // ── Main render: lerp, crop, upload, uniforms, present ──────────────────
    WIN_DATA.with(|d| {
        let mut b = d.borrow_mut();
        let w = b.as_mut().unwrap();

        // Frame-rate-independent lerp toward actual cursor.
        let now = Instant::now();
        let dt = now.duration_since(w.last_tick).as_secs_f32();
        w.last_tick = now;
        let alpha = 1.0_f32 - (1.0 - snap.smooth_speed).powf(dt * 60.0);
        w.smooth_x += (cursor.x as f32 - w.smooth_x) * alpha;
        w.smooth_y += (cursor.y as f32 - w.smooth_y) * alpha;

        // Convert smoothed cursor to monitor-local coordinates (DXGI frame origin = 0,0).
        let cx = w.smooth_x - w.monitor_left as f32;
        let cy = w.smooth_y - w.monitor_top as f32;

        let cur_sw = w.screen_w;
        let cur_sh = w.screen_h;
        let (win_w, win_h) = window_dims(snap.mode, snap.panel_size, cur_sw, cur_sh);

        let fw = w.wgpu.tex_w as f32;
        let fh = w.wgpu.tex_h as f32;
        let src_w = (win_w as f32 / snap.zoom).min(fw);
        let src_h = (win_h as f32 / snap.zoom).min(fh);
        let src_x = (cx - src_w * 0.5).clamp(0.0, fw - src_w);
        let src_y = (cy - src_h * 0.5).clamp(0.0, fh - src_h);
        let crop = [src_x / fw, src_y / fh, src_w / fw, src_h / fh];

        // Upload frame only when the capture thread has produced a new Arc<Frame>.
        if let Some(frame) = &snap.frame {
            let new_frame = w
                .last_frame
                .as_ref()
                .map_or(true, |last| !Arc::ptr_eq(last, frame));
            if new_frame {
                w.wgpu.upload_frame(&frame.data, frame.width, frame.height);
                w.last_frame = Some(Arc::clone(frame));
            }
        }

        // Cursor position in output window pixel space, accounting for zoom/crop.
        let cursor_x = ((cx - src_x) / src_w * win_w as f32).clamp(0.0, win_w as f32 - 1.0) as u32;
        let cursor_y = ((cy - src_y) / src_h * win_h as f32).clamp(0.0, win_h as f32 - 1.0) as u32;

        // Write uniforms only when crop, settings, or cursor have changed.
        let color_mode = snap.color_filter.as_u32();
        let interp_mode = snap.interpolation.as_u32();
        if crop != w.last_crop
            || color_mode != w.last_color_mode
            || interp_mode != w.last_interp_mode
            || cursor_x != w.last_cursor_x
            || cursor_y != w.last_cursor_y
        {
            w.wgpu
                .write_uniforms(crop, color_mode, interp_mode, cursor_x, cursor_y);
            w.last_crop = crop;
            w.last_color_mode = color_mode;
            w.last_interp_mode = interp_mode;
            w.last_cursor_x = cursor_x;
            w.last_cursor_y = cursor_y;
        }

        if !w.wgpu.render() {
            // Surface lost/outdated — reconfigure to recover.
            let (rw, rh) = window_dims(snap.mode, snap.panel_size, cur_sw, cur_sh);
            w.wgpu.resize(rw, rh);
        }
    });
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn window_dims(mode: DisplayMode, panel_size: u32, sw: i32, sh: i32) -> (u32, u32) {
    match mode {
        DisplayMode::Fullscreen => (sw as u32, sh as u32),
        DisplayMode::Docked(Edge::Top | Edge::Bottom) => (sw as u32, panel_size),
        DisplayMode::Docked(Edge::Left | Edge::Right) => (panel_size, sh as u32),
    }
}

fn rect_dims(r: RECT) -> (u32, u32) {
    (
        (r.right - r.left).max(16) as u32,
        (r.bottom - r.top).max(16) as u32,
    )
}

fn move_window(hwnd: HWND, r: RECT) {
    unsafe {
        SetWindowPos(
            hwnd,
            None,
            r.left,
            r.top,
            r.right - r.left,
            r.bottom - r.top,
            SWP_NOZORDER | SWP_NOACTIVATE,
        )
        .ok();
    }
    appbar::notify_moved(hwnd);
}
