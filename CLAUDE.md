# CLAUDE.md

This file provides guidance to Claude Code when working with code in this repository.

## What this project is

`clear-view` is a Windows screen magnifier and accessibility tool (ZoomText alternative) written in Rust. It uses DXGI Desktop Duplication for screen capture, wgpu for GPU rendering, and egui for the settings panel.

## Build & run
```bash
cargo build
cargo run
cargo build --release
```

When running cargo build, suppress warnings with RUSTFLAGS="-Awarnings" cargo build unless specifically debugging a warning. Only show errors.

Windows-only. Will not compile on other platforms.

## Crate structure

- `cv-core` — shared types: Frame, AppState, SharedState, DisplayMode
- `cv-capture` — DXGI Desktop Duplication → staging texture → CPU Vec<u8> readback
- `cv-render` — wgpu pipeline, AppBar window management, rendering
- `app` — main.rs, egui settings panel, hotkey thread

## Architecture

Four concurrent threads sharing `Arc<RwLock<AppState>>`:

- **Main thread** — runs `eframe` (egui settings panel) with `with_always_on_top()`
- **Capture thread** — DXGI Desktop Duplication → D3D11 staging texture → CPU BGRA8 readback, stored in `Arc<Mutex<Option<Arc<Frame>>>>`
- **Render thread** — wgpu pipeline; uploads frame as texture each tick, blits to display window via shader, lerps cursor for smooth follow
- **Hotkey thread** — `RegisterHotKey` (Win+=) toggles `AppState.enabled`; renderer shows/hides and registers/releases AppBar accordingly

## Key design decisions

- **Self-capture prevention**: `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)` prevents the overlay appearing in its own DXGI capture.
- **Overlay flags**: `WS_POPUP | WS_EX_TOPMOST | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_LAYERED` — clicks pass through, window stays on top.
- **Overlay starts hidden**: Created without `WS_VISIBLE`. Show/hide via `ShowWindow` on enabled state change.
- **Smooth follow**: Frame-rate-independent lerp: `alpha = 1.0 - (1.0 - smooth_speed).powf(dt * 60.0)`
- **Interpolation**: Bilinear active. Lanczos is a greyed-out placeholder in egui until phase 4.
- **Docked panel**: Uses SHAppBarMessage (ABM_NEW, ABM_SETPOS, ABM_ACTIVATE, ABM_REMOVE). Supports all four edges (ABE_TOP/BOTTOM/LEFT/RIGHT). Desktop work area shifts to accommodate. AppBar released on toggle off or app exit.
- **Display modes**: Fullscreen or docked (top/bottom/left/right). Mode switching is immediate on egui selection. Win+= toggles on/off in all modes and restores work area when hidden.
- **wgpu pipeline**: DXGI capture → BGRA8 Vec<u8> → upload to wgpu texture each frame → fullscreen or panel quad blit with sampler.
- **windows crate**: Use version 0.62. `D3D11CreateDevice` software param is `HMODULE::default()` not `None`.
- **Rust 2024 edition**: Requires explicit `unsafe {}` blocks inside `unsafe fn` bodies.

## Dead ends — do not retry these approaches

- **D3D11 shared device context**: Immediate context is single-threaded. Frames never rendered. Use CPU readback instead.
- **ClipCursor**: Never use. Blocks user from part of the screen with no way to interact with content in that region.
- **Mixing screen-space and frame-space coordinates**: Always ensure cursor coords and frame coords are in the same space before any layout math.
- **GDI StretchBlt for rendering**: Was used in phase 1, replaced with wgpu in phase 2 for shader support.

## DXGI error handling

| Error | Action |
|---|---|
| `DXGI_ERROR_WAIT_TIMEOUT` | Retry `AcquireNextFrame` |
| `DXGI_ERROR_ACCESS_LOST` | Re-create `IDXGIOutputDuplication` |
| `DXGI_ERROR_DEVICE_REMOVED` | Re-create D3D11 device and all resources |

## Windows crate

Use the `windows` crate (not `winapi` or `windows-sys`). Add new features to the `windows` dependency in `Cargo.toml`. `RegisterHotKey`, `MOD_WIN`, `VK_OEM_PLUS` are in `Win32::UI::Input::KeyboardAndMouse` not `WindowsAndMessaging`.

## Planned phases

1. ✅ Full-screen magnification (GDI, single monitor)
2. ⬜ wgpu rendering + AppBar docked panel (top/bottom/left/right, egui mode selector)
3. ⬜ Colour filters (inverted, greyscale, greyscale+inverted) — shader-level
4. ⬜ Lanczos upscaling shader
5. ⬜ Cursor enhancement (larger cursor, colour override, highlight halo)
6. ⬜ Multi-monitor
7. ⬜ TTS / screen reader (UIA for Chrome/Edge, IAccessible2 for Firefox, SAPI for speech — browser compatibility is a known complexity)