# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this project is

`clear-view` is a Windows screen magnifier (ZoomText alternative) written in Rust. It uses DXGI Desktop Duplication to capture the screen and Direct3D 11 to upscale and present the magnified output via a fullscreen borderless overlay window. An egui settings panel runs alongside it.

See `PLAN.md` for the full architecture and implementation plan.

## Build & run

```bash
cargo build
cargo run
cargo build --release
```

The project is Windows-only. It will not compile on other platforms due to direct use of Win32/DXGI/D3D11 APIs.

## Architecture

The app runs four concurrent threads sharing `Arc<RwLock<AppState>>`:

- **Main thread** — runs `eframe` (egui settings window)
- **Capture thread** (`src/capture.rs`) — DXGI Desktop Duplication loop; acquires frames and sends `ID3D11Texture2D` handles to the render thread via a `crossbeam` channel
- **Render thread** (`src/renderer.rs`) — receives captured textures, lerps viewport center toward cursor each frame, updates a D3D11 constant buffer, draws a fullscreen quad through `shaders/magnify.hlsl`, and calls `Present`
- **Hotkey thread** (`src/hotkey.rs`) — Win32 message pump for `WM_HOTKEY` (Win+=); toggles `AppState.enabled` and shows/hides the overlay

The D3D11 device is created once and shared between the capture and render threads.

## Key design decisions

- **Self-capture prevention**: The overlay window uses `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)` so it never appears in the DXGI-captured frame, preventing an infinite mirror loop.
- **Overlay window flags**: `WS_POPUP | WS_EX_TOPMOST | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE` — clicks pass through to apps below.
- **Smooth follow**: Frame-rate-independent lerp: `alpha = 1.0 - (1.0 - smooth_speed).powf(delta_secs * 60.0)`.
- **Swap chain**: `DXGI_SWAP_EFFECT_FLIP_DISCARD` with 2 buffers; `Present(1, 0)` for vsync.
- **Shader**: `shaders/magnify.hlsl` — maps fullscreen UVs into the zoomed sub-region around the cursor using `D3D11_FILTER_MIN_MAG_MIP_LINEAR` sampling.
- **Interpolation toggle**: Bilinear is active; Lanczos is a greyed-out placeholder in egui for a future phase.

## DXGI error handling

| Error | Action |
|---|---|
| `DXGI_ERROR_WAIT_TIMEOUT` | Retry `AcquireNextFrame` |
| `DXGI_ERROR_ACCESS_LOST` | Re-create `IDXGIOutputDuplication` |
| `DXGI_ERROR_DEVICE_REMOVED` | Re-create D3D11 device and all resources |

## Windows crate features

All Windows API access goes through the `windows` crate (not `winapi` or `windows-sys`). Required feature flags are documented in `PLAN.md` and `Cargo.toml`. Add new features to the `windows` dependency rather than pulling in separate FFI crates.
