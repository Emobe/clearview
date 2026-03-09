mod appbar;
mod gfx;

use std::{cell::RefCell, mem::size_of, sync::Arc, time::Instant};

use windows::{
    core::w,
    Win32::{
        Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DispatchMessageW, GetCursorPos, GetMessageW,
            GetSystemMetrics, LoadCursorW, LWA_ALPHA, PostQuitMessage, RegisterClassExW,
            RegisterWindowMessageW, SetLayeredWindowAttributes, SetTimer,
            SetWindowDisplayAffinity, SetWindowPos, ShowWindow, TranslateMessage,
            WDA_EXCLUDEFROMCAPTURE, HTTRANSPARENT, IDC_ARROW, MSG, SM_CXSCREEN, SM_CYSCREEN,
            SW_HIDE, SW_SHOW, WM_DESTROY, WM_NCHITTEST, WM_TIMER, WNDCLASSEXW,
            WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
            SWP_NOACTIVATE, SWP_NOZORDER,
        },
    },
};

use cv_core::{DisplayMode, Edge, Frame, FrameState, SharedState};
use gfx::WgpuState;

struct WindowData {
    wgpu:           WgpuState,
    frame_state:    FrameState,
    app_state:      SharedState,
    smooth_x:       f32,
    smooth_y:       f32,
    last_tick:      Instant,
    screen_w:       i32,
    screen_h:       i32,
    callback_msg:   u32,
    // Change-detection: what is currently applied to the window.
    cur_enabled:    bool,
    cur_mode:       DisplayMode,
    cur_panel_size: u32,
    appbar_active:  bool,
}

thread_local! {
    static WIN_DATA: RefCell<Option<WindowData>> = const { RefCell::new(None) };
}

/// Creates the overlay window and blocks on its message loop.
pub fn run_overlay(frame_state: FrameState, app_state: SharedState) {
    let (screen_w, screen_h) = unsafe {
        (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN))
    };

    let hwnd = unsafe {
        let hinstance: HINSTANCE = GetModuleHandleW(None).unwrap().into();
        let class_name = w!("clear_view_overlay");

        let wc = WNDCLASSEXW {
            cbSize:        size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc:   Some(wnd_proc),
            hInstance:     hinstance,
            lpszClassName: class_name,
            hCursor:       LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassExW(&wc);

        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_NOACTIVATE | WS_EX_TOPMOST | WS_EX_TRANSPARENT,
            class_name,
            w!("clear-view"),
            WS_POPUP, // starts hidden
            0, 0, screen_w, screen_h,
            None, None, Some(hinstance), None,
        )
        .expect("CreateWindowExW failed");

        SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE).ok();
        SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA).ok();
        hwnd
    };

    let callback_msg = unsafe { RegisterWindowMessageW(w!("ClearViewAppBar")) };

    // wgpu init (blocking) — window starts fullscreen-sized.
    let wgpu = WgpuState::new(hwnd, screen_w as u32, screen_h as u32,
                               screen_w as u32, screen_h as u32);

    WIN_DATA.with(|d| {
        *d.borrow_mut() = Some(WindowData {
            wgpu,
            frame_state,
            app_state,
            smooth_x:       screen_w as f32 / 2.0,
            smooth_y:       screen_h as f32 / 2.0,
            last_tick:      Instant::now(),
            screen_w, screen_h,
            callback_msg,
            cur_enabled:    false,
            cur_mode:       DisplayMode::Fullscreen,
            cur_panel_size: 300,
            appbar_active:  false,
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
    // Check if this is the appbar callback message.
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
        },
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn handle_appbar_callback(hwnd: HWND, wparam: WPARAM) {
    if wparam.0 != appbar::ABN_POSCHANGED { return; }

    let params = WIN_DATA.with(|d| {
        let b = d.borrow();
        let w = b.as_ref()?;
        if !w.appbar_active { return None; }
        match w.cur_mode {
            DisplayMode::Docked(e) => Some((e, w.cur_panel_size as i32, w.screen_w, w.screen_h)),
            _ => None,
        }
    });
    let Some((edge, thickness, sw, sh)) = params else { return };

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
        enabled:      bool,
        mode:         DisplayMode,
        panel_size:   u32,
        zoom:         f32,
        smooth_speed: f32,
        frame:        Option<Arc<Frame>>,
        cur_enabled:    bool,
        cur_mode:       DisplayMode,
        cur_panel_size: u32,
        appbar_active:  bool,
        screen_w:     i32,
        screen_h:     i32,
        callback_msg: u32,
    }

    let snap = WIN_DATA.with(|d| {
        let b = d.borrow();
        let w = b.as_ref()?;
        let s = w.app_state.read();
        let frame = w.frame_state.lock().ok()?.clone();
        Some(Snap {
            enabled:      s.enabled,
            mode:         s.display_mode,
            panel_size:   s.panel_size,
            zoom:         s.zoom,
            smooth_speed: s.smooth_speed,
            frame,
            cur_enabled:    w.cur_enabled,
            cur_mode:       w.cur_mode,
            cur_panel_size: w.cur_panel_size,
            appbar_active:  w.appbar_active,
            screen_w:   w.screen_w,
            screen_h:   w.screen_h,
            callback_msg: w.callback_msg,
        })
    });
    let Some(snap) = snap else { return };

    let sw = snap.screen_w;
    let sh = snap.screen_h;

    // ── State transitions ────────────────────────────────────────────────────
    let enabled_changed    = snap.enabled    != snap.cur_enabled;
    let mode_changed       = snap.mode       != snap.cur_mode;
    let panel_size_changed = snap.panel_size != snap.cur_panel_size;

    let need_transition = enabled_changed
        || (snap.enabled && mode_changed)
        || (snap.enabled && panel_size_changed);

    let mut new_appbar_active = snap.appbar_active;
    let mut new_rect: Option<RECT> = None;

    if need_transition {
        if enabled_changed && !snap.enabled {
            // ── Disabling ─────────────────────────────────────────────────
            if snap.appbar_active {
                appbar::unregister(hwnd);
                new_appbar_active = false;
            }
            unsafe { let _ = ShowWindow(hwnd, SW_HIDE); }

        } else if enabled_changed && snap.enabled {
            // ── Enabling ──────────────────────────────────────────────────
            if snap.appbar_active {
                appbar::unregister(hwnd);
                new_appbar_active = false;
            }
            match snap.mode {
                DisplayMode::Fullscreen => {
                    let rect = RECT { left: 0, top: 0, right: sw, bottom: sh };
                    move_window(hwnd, rect);
                    new_rect = Some(rect);
                }
                DisplayMode::Docked(e) => {
                    let rect = appbar::register(hwnd, e, snap.panel_size as i32, sw, sh, snap.callback_msg);
                    move_window(hwnd, rect);
                    new_appbar_active = true;
                    new_rect = Some(rect);
                }
            }
            unsafe { let _ = ShowWindow(hwnd, SW_SHOW); }

        } else if snap.enabled && mode_changed {
            // ── Mode change while enabled ─────────────────────────────────
            if snap.appbar_active {
                appbar::unregister(hwnd);
                new_appbar_active = false;
            }
            match snap.mode {
                DisplayMode::Fullscreen => {
                    let rect = RECT { left: 0, top: 0, right: sw, bottom: sh };
                    move_window(hwnd, rect);
                    new_rect = Some(rect);
                }
                DisplayMode::Docked(e) => {
                    let rect = appbar::register(hwnd, e, snap.panel_size as i32, sw, sh, snap.callback_msg);
                    move_window(hwnd, rect);
                    new_appbar_active = true;
                    new_rect = Some(rect);
                }
            }

        } else if snap.enabled && panel_size_changed {
            // ── Panel size change while docked ────────────────────────────
            if let DisplayMode::Docked(e) = snap.mode {
                let rect = appbar::reposition(hwnd, e, snap.panel_size as i32, sw, sh);
                move_window(hwnd, rect);
                new_rect = Some(rect);
            }
        }

        // ── Write back tracking state + resize wgpu surface ──────────────
        WIN_DATA.with(|d| {
            let mut b = d.borrow_mut();
            let w = b.as_mut().unwrap();
            w.cur_enabled    = snap.enabled;
            w.cur_mode       = snap.mode;
            w.cur_panel_size = snap.panel_size;
            w.appbar_active  = new_appbar_active;
            if let Some(rect) = new_rect {
                let (rw, rh) = rect_dims(rect);
                w.wgpu.resize(rw, rh);
            }
        });
    }

    // ── Render ───────────────────────────────────────────────────────────────
    if !snap.enabled { return; }

    let mut cursor = windows::Win32::Foundation::POINT::default();
    unsafe { let _ = GetCursorPos(&mut cursor); }

    WIN_DATA.with(|d| {
        let mut b = d.borrow_mut();
        let w = b.as_mut().unwrap();

        // Frame-rate-independent lerp toward actual cursor.
        let now   = Instant::now();
        let dt    = now.duration_since(w.last_tick).as_secs_f32();
        w.last_tick = now;
        let alpha = 1.0_f32 - (1.0 - snap.smooth_speed).powf(dt * 60.0);
        w.smooth_x += (cursor.x as f32 - w.smooth_x) * alpha;
        w.smooth_y += (cursor.y as f32 - w.smooth_y) * alpha;
        let (cx, cy) = (w.smooth_x, w.smooth_y);

        let (win_w, win_h) = window_dims(snap.mode, snap.panel_size, sw, sh);

        let fw = w.wgpu.tex_w as f32;
        let fh = w.wgpu.tex_h as f32;
        let src_w = (win_w as f32 / snap.zoom).min(fw);
        let src_h = (win_h as f32 / snap.zoom).min(fh);
        let src_x = (cx - src_w * 0.5).clamp(0.0, fw - src_w);
        let src_y = (cy - src_h * 0.5).clamp(0.0, fh - src_h);
        let crop  = [src_x / fw, src_y / fh, src_w / fw, src_h / fh];

        if let Some(frame) = &snap.frame {
            w.wgpu.upload_frame(&frame.data, frame.width, frame.height);
        }
        w.wgpu.write_crop(crop);

        if !w.wgpu.render() {
            // Surface lost/outdated — reconfigure to recover.
            let (rw, rh) = window_dims(snap.mode, snap.panel_size, sw, sh);
            w.wgpu.resize(rw, rh);
        }
    });
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn window_dims(mode: DisplayMode, panel_size: u32, sw: i32, sh: i32) -> (u32, u32) {
    match mode {
        DisplayMode::Fullscreen                               => (sw as u32, sh as u32),
        DisplayMode::Docked(Edge::Top | Edge::Bottom)        => (sw as u32, panel_size),
        DisplayMode::Docked(Edge::Left | Edge::Right)        => (panel_size, sh as u32),
    }
}

fn rect_dims(r: RECT) -> (u32, u32) {
    ((r.right - r.left).max(16) as u32, (r.bottom - r.top).max(16) as u32)
}

fn move_window(hwnd: HWND, r: RECT) {
    unsafe {
        SetWindowPos(
            hwnd, None,
            r.left, r.top,
            r.right  - r.left,
            r.bottom - r.top,
            SWP_NOZORDER | SWP_NOACTIVATE,
        )
        .ok();
    }
    appbar::notify_moved(hwnd);
}
