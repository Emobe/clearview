use windows::{
    core::*,
    Win32::{
        Foundation::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::*,
    },
};

/// HWND wrapper that is safe to send across threads.
/// Windows guarantees ShowWindow / DestroyWindow are safe to call from any thread.
#[derive(Clone, Copy)]
pub struct SendHwnd(pub HWND);
unsafe impl Send for SendHwnd {}
unsafe impl Sync for SendHwnd {}

/// Creates a borderless, topmost, click-through fullscreen window on the primary
/// monitor and excludes it from DXGI Desktop Duplication capture so it never
/// appears in its own magnified output.
pub fn create_overlay() -> Result<SendHwnd> {
    unsafe {
        let hinstance: HINSTANCE = GetModuleHandleW(None)?.into();
        let class_name = w!("clear_view_overlay");

        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(overlay_wndproc),
            hInstance: hinstance,
            lpszClassName: class_name,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            ..Default::default()
        };

        RegisterClassExW(&wc);

        // Primary monitor dimensions
        let width = GetSystemMetrics(SM_CXSCREEN);
        let height = GetSystemMetrics(SM_CYSCREEN);

        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_LAYERED,
            class_name,
            w!("clear-view"),
            WS_POPUP,
            0,
            0,
            width,
            height,
            None,
            None,
            hinstance,
            None,
        )?;

        // Make layered window fully opaque
        SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA)?;

        // Exclude this window from DXGI Desktop Duplication capture.
        // Without this the overlay appears in its own captured frame, causing
        // an infinite mirror. Available since Windows 10 2004.
        SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE)?;

        Ok(SendHwnd(hwnd))
    }
}

#[allow(dead_code)]
pub fn show_overlay(hwnd: HWND) {
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
    }
}

#[allow(dead_code)]
pub fn hide_overlay(hwnd: HWND) {
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
}

extern "system" fn overlay_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match msg {
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}
