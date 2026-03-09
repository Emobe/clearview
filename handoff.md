# Phase 2 Handoff

## Branch
`phase-2` (branched from `master` after phase-1 merge)

## What phase 2 adds
- **Replace GDI rendering with wgpu** (wgpu 22, raw-window-handle 0.6 — same versions cv3 uses)
- **Docked panel mode** via `SHAppBarMessage` — reserves screen edge space, shifts desktop work area
- **Four edges** supported: Top, Bottom, Left, Right
- **egui changes**: display mode selector (Fullscreen / Top / Bottom / Left / Right), panel size slider (thickness in pixels)
- Lanczos stays deferred (greyed-out placeholder)

## Decisions already made
| Question | Answer |
|---|---|
| wgpu approach | BGRA8 Vec<u8> → wgpu texture upload → fullscreen quad blit each frame |
| Docked panel IS the magnifier window | Yes — egui settings remain a separate floating window |
| What does docked panel magnify | Same cursor-following region as fullscreen, rendered into the panel strip |
| Panel size slider controls | Thickness in pixels (other dimension fills full screen width/height) |
| Mode switch | Immediate on egui selection — no Apply button |
| Win+= in docked mode | Still toggles on/off (hides panel, restores work area) |
| Multi-monitor | Single monitor only (same as phase 1) |

## New state fields needed (cv-core)
```rust
pub enum Edge { Top, Bottom, Left, Right }

pub enum DisplayMode { Fullscreen, Docked(Edge) }

pub struct AppState {
    // existing fields...
    pub display_mode: DisplayMode,  // default: Fullscreen
    pub panel_size: u32,            // thickness px, e.g. 50–800, default 300
}
```

## wgpu surface from HWND
Use `wgpu::SurfaceTargetUnsafe::RawHandle` with `raw-window-handle 0.6`:
```rust
let surface = unsafe {
    instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
        raw_display_handle: RawDisplayHandle::Windows(WindowsDisplayHandle::new()),
        raw_window_handle: RawWindowHandle::Win32(Win32WindowHandle::new(
            NonZeroIsize::new(hwnd.0 as isize).unwrap()
        )),
    })
}?;
```

## AppBar call sequence
When switching to docked:
1. `ABM_NEW` — register window as appbar, provide callback message
2. Set `rc` to the desired edge + thickness rect
3. `ABM_QUERYPOS` — let Windows adjust rect to avoid other appbars
4. `ABM_SETPOS` — commit the position (this shifts the work area)
5. `SetWindowPos` to the returned rect

When switching away from docked (or on shutdown):
1. `ABM_REMOVE` — unregisters, restores work area
2. Resize/reposition window as needed

Handle `ABN_POSCHANGED` in the callback (taskbar moved) → re-run QUERYPOS/SETPOS.

## Cargo deps to add
- `wgpu = "22"` (workspace)
- `raw-window-handle = "0.6"` (workspace)
- `Win32_UI_Shell` feature on `windows` crate (for `SHAppBarMessage`, `APPBARDATA`, `ABM_*`, `ABE_*`)

## Reference
`C:\Users\Anthony\cv3` — cv3's renderer/window.rs has the GDI docked-window pattern we already adapted for phase 1. cv3's renderer crate has wgpu + raw-window-handle in its Cargo.toml but the wgpu code is commented-out stubs — not usable as reference for the wgpu implementation itself.
