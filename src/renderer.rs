use std::{
    cell::RefCell,
    mem::size_of,
    sync::{Arc, Mutex},
    time::Instant,
};

use windows::{
    core::w,
    Win32::{
        Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM},
        Graphics::Gdi::{
            CreateCompatibleDC, CreateDIBSection, CreateSolidBrush, DeleteDC, DeleteObject,
            FillRect, GetDC, ReleaseDC, SelectObject, StretchBlt, BITMAPINFO, BITMAPINFOHEADER,
            BI_RGB, DIB_RGB_COLORS, SRCCOPY,
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DispatchMessageW, DrawIconEx, GetCursorInfo,
            GetCursorPos, GetMessageW, GetSystemMetrics, LoadCursorW, LWA_ALPHA, PostQuitMessage,
            RegisterClassExW, SetLayeredWindowAttributes, SetTimer, SetWindowDisplayAffinity,
            ShowWindow, TranslateMessage, WDA_EXCLUDEFROMCAPTURE, CURSOR_SHOWING, CURSORINFO,
            DI_NORMAL, HICON, HTTRANSPARENT, IDC_ARROW, MSG, SM_CXCURSOR, SM_CXSCREEN,
            SM_CYCURSOR, SM_CYSCREEN, SW_HIDE, SW_SHOW, WM_DESTROY, WM_NCHITTEST, WM_TIMER,
            WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOPMOST, WS_EX_TRANSPARENT,
            WS_POPUP,
        },
    },
};

use crate::{capture::Frame, state::SharedState};

pub type FrameState = Arc<Mutex<Option<Arc<Frame>>>>;

struct WindowData {
    frame_state: FrameState,
    app_state: SharedState,
    smooth_x: f32,
    smooth_y: f32,
    last_tick: Instant,
    /// Tracks whether the window is currently shown so we only call ShowWindow on changes.
    visible: bool,
}

thread_local! {
    static WIN_DATA: RefCell<Option<WindowData>> = const { RefCell::new(None) };
}

/// Creates the fullscreen magnifier overlay and runs its message loop.
/// Blocks until the window is destroyed.
pub fn run_overlay(frame_state: FrameState, app_state: SharedState) {
    let screen_w = unsafe { GetSystemMetrics(SM_CXSCREEN) };
    let screen_h = unsafe { GetSystemMetrics(SM_CYSCREEN) };

    WIN_DATA.with(|d| {
        *d.borrow_mut() = Some(WindowData {
            frame_state,
            app_state,
            smooth_x: screen_w as f32 / 2.0,
            smooth_y: screen_h as f32 / 2.0,
            last_tick: Instant::now(),
            visible: false,
        });
    });

    unsafe {
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
            WS_POPUP, // hidden until enabled
            0,
            0,
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
        SetTimer(Some(hwnd), 1, 16, None);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_DESTROY => unsafe {
            PostQuitMessage(0);
            LRESULT(0)
        },
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_TIMER => unsafe {
            let enabled = WIN_DATA
                .with(|d| {
                    let b = d.borrow();
                    Some(b.as_ref()?.app_state.read().enabled)
                })
                .unwrap_or(false);

            // Show/hide window when enabled state changes.
            let was_visible = WIN_DATA
                .with(|d| d.borrow().as_ref().map(|w| w.visible))
                .unwrap_or(false);

            if enabled != was_visible {
                WIN_DATA.with(|d| {
                    if let Some(w) = d.borrow_mut().as_mut() {
                        w.visible = enabled;
                    }
                });
                let _ = ShowWindow(hwnd, if enabled { SW_SHOW } else { SW_HIDE });
            }

            if enabled {
                let hdc = GetDC(Some(hwnd));
                draw(hdc);
                ReleaseDC(Some(hwnd), hdc);
            }
            LRESULT(0)
        },
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn draw(hdc: windows::Win32::Graphics::Gdi::HDC) {
    unsafe {
        let screen_w = GetSystemMetrics(SM_CXSCREEN);
        let screen_h = GetSystemMetrics(SM_CYSCREEN);

        let (zoom, smooth_speed) = WIN_DATA
            .with(|d| {
                let b = d.borrow();
                let data = b.as_ref()?;
                let s = data.app_state.read();
                Some((s.zoom, s.smooth_speed))
            })
            .unwrap_or((2.0, 0.15));

        let mut cursor = windows::Win32::Foundation::POINT::default();
        let _ = GetCursorPos(&mut cursor);

        // Smooth follow — frame-rate-independent lerp toward actual cursor
        let smoothed = WIN_DATA.with(|d| {
            let mut b = d.borrow_mut();
            let data = b.as_mut()?;
            let now = Instant::now();
            let dt = now.duration_since(data.last_tick).as_secs_f32();
            data.last_tick = now;
            let alpha = 1.0_f32 - (1.0 - smooth_speed).powf(dt * 60.0);
            data.smooth_x += (cursor.x as f32 - data.smooth_x) * alpha;
            data.smooth_y += (cursor.y as f32 - data.smooth_y) * alpha;
            Some((data.smooth_x, data.smooth_y))
        });

        let (cx, cy) = match smoothed {
            Some(p) => p,
            None => return,
        };

        let maybe_frame: Option<Arc<Frame>> = WIN_DATA.with(|d| {
            let b = d.borrow();
            let data = b.as_ref()?;
            let lock = data.frame_state.lock().ok()?;
            lock.clone()
        });

        let Some(frame) = maybe_frame else {
            // No frame yet — fill black
            let brush = CreateSolidBrush(COLORREF(0x00000000));
            let rect = windows::Win32::Foundation::RECT {
                left: 0,
                top: 0,
                right: screen_w,
                bottom: screen_h,
            };
            let _ = FillRect(hdc, &rect, brush);
            let _ = DeleteObject(brush.into());
            return;
        };

        let fw = frame.width as i32;
        let fh = frame.height as i32;

        // Crop region: (screen / zoom) pixels centred on smoothed cursor
        let src_w = (screen_w as f32 / zoom) as i32;
        let src_h = (screen_h as f32 / zoom) as i32;
        let src_x = (cx as i32 - src_w / 2).clamp(0, (fw - src_w).max(0));
        let src_y = (cy as i32 - src_h / 2).clamp(0, (fh - src_h).max(0));

        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: fw,
                biHeight: -fh, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let mem_dc = CreateCompatibleDC(Some(hdc));
        let mut bits: *mut core::ffi::c_void = core::ptr::null_mut();

        let Ok(hbmp) = CreateDIBSection(Some(mem_dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0)
        else {
            let _ = DeleteDC(mem_dc);
            return;
        };

        if !bits.is_null() {
            std::ptr::copy_nonoverlapping(
                frame.data.as_ptr(),
                bits as *mut u8,
                frame.data.len(),
            );
        }

        let old = SelectObject(mem_dc, hbmp.into());

        let _ = StretchBlt(
            hdc,
            0, 0, screen_w, screen_h,
            Some(mem_dc),
            src_x, src_y, src_w, src_h,
            SRCCOPY,
        );

        // Draw magnified cursor overlay
        let mut ci = CURSORINFO { cbSize: size_of::<CURSORINFO>() as u32, ..Default::default() };
        if GetCursorInfo(&mut ci).is_ok() && ci.flags == CURSOR_SHOWING {
            let zoom_i = zoom as i32;
            let icon_w = GetSystemMetrics(SM_CXCURSOR) * zoom_i;
            let icon_h = GetSystemMetrics(SM_CYCURSOR) * zoom_i;
            let draw_x = (cursor.x - src_x) * zoom_i;
            let draw_y = (cursor.y - src_y) * zoom_i;
            DrawIconEx(hdc, draw_x, draw_y, HICON(ci.hCursor.0), icon_w, icon_h, 0, None, DI_NORMAL).ok();
        }

        SelectObject(mem_dc, old);
        let _ = DeleteObject(hbmp.into());
        let _ = DeleteDC(mem_dc);
    }
}
