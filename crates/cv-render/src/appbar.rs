use std::mem::size_of;

use windows::Win32::{
    Foundation::{HWND, LPARAM, RECT},
    UI::Shell::{APPBARDATA, SHAppBarMessage},
};

use cv_core::Edge;

// SHAppBarMessage message codes
const ABM_NEW: u32 = 0;
const ABM_REMOVE: u32 = 1;
const ABM_QUERYPOS: u32 = 2;
const ABM_SETPOS: u32 = 3;
const ABM_WINDOWPOSCHANGED: u32 = 9;

// Edge constants
const ABE_LEFT: u32 = 0;
const ABE_TOP: u32 = 1;
const ABE_RIGHT: u32 = 2;
const ABE_BOTTOM: u32 = 3;

// ABN notification codes (sent in wParam of the callback message)
pub const ABN_POSCHANGED: usize = 1;

fn edge_to_abe(edge: Edge) -> u32 {
    match edge {
        Edge::Left   => ABE_LEFT,
        Edge::Top    => ABE_TOP,
        Edge::Right  => ABE_RIGHT,
        Edge::Bottom => ABE_BOTTOM,
    }
}

/// Ideal panel rect before QUERYPOS adjustment.
pub fn panel_rect(edge: Edge, thickness: i32, sw: i32, sh: i32) -> RECT {
    match edge {
        Edge::Top    => RECT { left: 0,          top: 0,              right: sw,         bottom: thickness        },
        Edge::Bottom => RECT { left: 0,          top: sh - thickness, right: sw,         bottom: sh               },
        Edge::Left   => RECT { left: 0,          top: 0,              right: thickness,  bottom: sh               },
        Edge::Right  => RECT { left: sw - thickness, top: 0,          right: sw,         bottom: sh               },
    }
}

fn make_abd(hwnd: HWND, callback_msg: u32, edge: Edge, rc: RECT) -> APPBARDATA {
    APPBARDATA {
        cbSize:          size_of::<APPBARDATA>() as u32,
        hWnd:            hwnd,
        uCallbackMessage: callback_msg,
        uEdge:           edge_to_abe(edge),
        rc,
        lParam:          LPARAM(0),
    }
}

/// Register as a new appbar and claim space on `edge`.
/// Returns the final RECT after Windows adjusts for other appbars.
pub fn register(hwnd: HWND, edge: Edge, thickness: i32, sw: i32, sh: i32, callback_msg: u32) -> RECT {
    unsafe {
        let mut abd = make_abd(hwnd, callback_msg, edge, panel_rect(edge, thickness, sw, sh));
        SHAppBarMessage(ABM_NEW, &mut abd);

        abd.rc    = panel_rect(edge, thickness, sw, sh);
        abd.uEdge = edge_to_abe(edge);
        SHAppBarMessage(ABM_QUERYPOS, &mut abd);
        SHAppBarMessage(ABM_SETPOS,   &mut abd);
        abd.rc
    }
}

/// Reposition on the same or a new edge (e.g. panel_size or edge changed).
/// Does NOT call ABM_NEW — caller is responsible for the current registration state.
pub fn reposition(hwnd: HWND, edge: Edge, thickness: i32, sw: i32, sh: i32) -> RECT {
    unsafe {
        let mut abd = make_abd(hwnd, 0, edge, panel_rect(edge, thickness, sw, sh));
        abd.uEdge = edge_to_abe(edge);
        SHAppBarMessage(ABM_QUERYPOS, &mut abd);
        SHAppBarMessage(ABM_SETPOS,   &mut abd);
        abd.rc
    }
}

/// Notify the shell that the window has been repositioned via SetWindowPos.
pub fn notify_moved(hwnd: HWND) {
    unsafe {
        let mut abd = APPBARDATA {
            cbSize: size_of::<APPBARDATA>() as u32,
            hWnd:   hwnd,
            ..Default::default()
        };
        SHAppBarMessage(ABM_WINDOWPOSCHANGED, &mut abd);
    }
}

/// Unregister the appbar and release the reserved work area.
pub fn unregister(hwnd: HWND) {
    unsafe {
        let mut abd = APPBARDATA {
            cbSize: size_of::<APPBARDATA>() as u32,
            hWnd:   hwnd,
            ..Default::default()
        };
        SHAppBarMessage(ABM_REMOVE, &mut abd);
    }
}
