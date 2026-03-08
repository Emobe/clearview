# clear-view — Implementation Plan

A ZoomText alternative written in Rust using DXGI Desktop Duplication and Direct3D 11.

---

## Phase 1 Scope

- Full-screen magnification of the primary monitor
- Mouse tracking with smooth follow (lerp)
- Adjustable zoom level
- Bilinear interpolation (Lanczos placeholder in UI)
- Global hotkey toggle (Win+=)
- egui settings panel

---

## Project Structure

```
clear-view/
├── Cargo.toml
├── PLAN.md
├── shaders/
│   └── magnify.hlsl          # bilinear upscale pixel shader
└── src/
    ├── main.rs               # entry point: spawns threads, runs eframe on main thread
    ├── app.rs                # egui settings panel (eframe App impl)
    ├── state.rs              # AppState, shared via Arc<RwLock<>>
    ├── overlay.rs            # Win32 borderless topmost window + message pump
    ├── capture.rs            # DXGI Desktop Duplication loop
    ├── renderer.rs           # D3D11 device, swap chain, shader pipeline, present
    └── hotkey.rs             # RegisterHotKey + WM_HOTKEY message handling
```

---

## Dependencies (`Cargo.toml`)

```toml
[dependencies]
windows = { version = "0.58", features = [
    "Win32_Foundation",
    "Win32_Graphics_Dxgi",
    "Win32_Graphics_Dxgi_Common",
    "Win32_Graphics_Direct3D",
    "Win32_Graphics_Direct3D11",
    "Win32_Graphics_Direct3D_Fxc",   # D3DCompile at runtime
    "Win32_UI_WindowsAndMessaging",
    "Win32_UI_HiDpi",
    "Win32_Graphics_Gdi",
    "Win32_System_LibraryLoader",
] }
eframe = "0.29"        # settings window, runs on main thread
egui = "0.29"
parking_lot = "0.12"   # faster RwLock than std
crossbeam-channel = "0.5"
```

---

## Shared State (`src/state.rs`)

```rust
pub struct AppState {
    pub enabled: bool,
    pub zoom: f32,                   // 1.0–10.0, default 2.0
    pub smooth_speed: f32,           // lerp factor 0.0–1.0, default 0.15
    pub interpolation: Interpolation,
    pub viewport_center: [f32; 2],   // current smoothed position (normalized 0..1)
}

pub enum Interpolation {
    Bilinear,
    Lanczos,  // placeholder, not yet implemented
}
```

Shared as `Arc<RwLock<AppState>>`, cloned into every thread.

---

## Threading Model

```
main thread
  └─ eframe (egui settings window)
       reads/writes AppState via Arc<RwLock>

background thread A: capture loop
  └─ AcquireNextFrame()
  └─ sends D3D11 texture handle to render thread via crossbeam channel

background thread B: render loop
  └─ reads AppState (zoom, smooth_speed, interpolation, viewport_center)
  └─ reads cursor pos via GetCursorPos()
  └─ lerps viewport_center toward cursor each frame
  └─ uploads constants to GPU
  └─ draws fullscreen quad → Present()

background thread C: hotkey pump
  └─ RegisterHotKey (Win+= by default)
  └─ WM_HOTKEY → toggles AppState.enabled
  └─ shows/hides overlay window accordingly
```

---

## Overlay Window (`src/overlay.rs`)

| Property | Value |
|---|---|
| Style | `WS_POPUP \| WS_VISIBLE` |
| Extended style | `WS_EX_TOPMOST \| WS_EX_TRANSPARENT \| WS_EX_NOACTIVATE` |
| Self-capture exclusion | `SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE)` |
| Position | Full primary monitor rect |
| Mouse passthrough | `WS_EX_TRANSPARENT` — clicks fall through to apps below |
| DPI awareness | `SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)` |

`WDA_EXCLUDEFROMCAPTURE` is the key flag that prevents the overlay window from appearing
in its own DXGI Desktop Duplication capture, avoiding an infinite mirror loop.

---

## Capture Pipeline (`src/capture.rs`)

1. `CreateDXGIFactory1` → enumerate adapter 0 → output 0
2. `D3D11CreateDevice` on that adapter (shared device instance)
3. `output.DuplicateOutput(device)` → `IDXGIOutputDuplication`
4. Frame loop:
   - `AcquireNextFrame(timeout_ms: 1000)` — handle `DXGI_ERROR_WAIT_TIMEOUT` gracefully (just retry)
   - Extract `ID3D11Texture2D` from frame resource
   - Send to render thread via `crossbeam_channel::Sender`
   - `ReleaseFrame()`
   - On `DXGI_ERROR_DEVICE_REMOVED` / `DXGI_ERROR_ACCESS_LOST`: re-initialize the duplicator

---

## Render Pipeline (`src/renderer.rs`)

```
D3D11 device (shared with capture thread)
  ├─ Swap chain on overlay HWND  (DXGI_SWAP_EFFECT_FLIP_DISCARD, 2 buffers)
  ├─ Fullscreen quad vertex buffer (4 verts, triangle strip, UV 0..1)
  ├─ Vertex shader: passthrough (position + UV)
  ├─ Pixel shader: magnify.hlsl
  │    inputs: SRV of latest captured texture, sampler state
  │    constant buffer: { center: float2, zoom: float, _pad: float }
  ├─ Linear sampler state (D3D11_FILTER_MIN_MAG_MIP_LINEAR)
  └─ Per frame:
       1. Receive latest captured texture from channel (non-blocking try_recv)
       2. Update constant buffer (zoom, smoothed viewport_center)
       3. Bind SRV of captured texture to t0
       4. Draw(4, 0) — triangle strip fullscreen quad
       5. Present(1, 0)  ← vsync on
```

---

## Pixel Shader (`shaders/magnify.hlsl`)

```hlsl
cbuffer Constants : register(b0) {
    float2 center;  // normalized 0..1, smoothed mouse position
    float  zoom;
    float  _pad;
};

Texture2D    screen_tex : register(t0);
SamplerState linear_smp : register(s0);

float4 main(float2 uv : TEXCOORD) : SV_Target {
    // Map fullscreen UV into the zoomed sub-region centred on cursor
    float2 sample_uv = (uv - 0.5) / zoom + center;
    // Clamp to edge to avoid wrapping artifacts at screen borders
    sample_uv = saturate(sample_uv);
    return screen_tex.Sample(linear_smp, sample_uv);
}
```

---

## Smooth Mouse Follow

Frame-rate-independent lerp using delta time so behaviour is consistent at any FPS:

```rust
let alpha = 1.0 - (1.0 - smooth_speed).powf(delta_secs * 60.0);
center[0] += (mouse_norm[0] - center[0]) * alpha;
center[1] += (mouse_norm[1] - center[1]) * alpha;
```

`mouse_norm` is the raw cursor position divided by the screen dimensions (0..1 range).

---

## Hotkey (`src/hotkey.rs`)

- `RegisterHotKey(None, id: 1, MOD_WIN | MOD_NOREPEAT, VK_OEM_PLUS)` — Win+=
- Dedicated message pump thread calls `GetMessage` and handles `WM_HOTKEY`
- On hotkey: flip `AppState.enabled`, show or hide overlay window via `ShowWindow`

---

## egui Settings Panel (`src/app.rs`)

| Control | Type | Purpose |
|---|---|---|
| Enable / Disable | Checkbox | Mirrors hotkey toggle |
| Zoom | Slider 1.0–10.0 | Magnification factor |
| Follow speed | Slider 0.01–1.0 | Lerp smoothing amount |
| Interpolation | ComboBox | Bilinear (active) / Lanczos (greyed out) |
| Hotkey | Static label | Shows current binding (Win+=) |

The egui window is a normal non-topmost window — it sits below the overlay naturally.

---

## Error Handling Strategy

| Error | Response |
|---|---|
| `DXGI_ERROR_WAIT_TIMEOUT` | Retry `AcquireNextFrame` immediately |
| `DXGI_ERROR_ACCESS_LOST` | Re-create `IDXGIOutputDuplication` |
| `DXGI_ERROR_DEVICE_REMOVED` | Re-create D3D11 device + all resources |
| Monitor resolution change | Recreate swap chain buffers on next Present failure |

---

## Future Phases

- **Lens mode** — magnify only a floating rectangular region, not full screen
- **Lanczos upscaling** — HLSL compute shader implementation
- **Color filters** — greyscale, invert, high contrast modes
- **Multi-monitor** — enumerate all outputs, per-monitor overlays
- **Configurable hotkey** — key picker in egui panel
- **Persistent settings** — save/load via `serde` + TOML config file
