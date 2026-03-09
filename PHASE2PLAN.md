# Phase 2 Implementation Plan

Branch: `phase-2`

## Goal

Replace GDI rendering with wgpu. Add a docked AppBar panel mode with all four edges.
No behaviour changes to phase-1 fullscreen mode beyond the renderer swap.

---

## New state (cv-core)

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Edge { Top, Bottom, Left, Right }

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DisplayMode { Fullscreen, Docked(Edge) }

// Added to AppState:
pub display_mode: DisplayMode,  // default Fullscreen
pub panel_size: u32,            // default 300, range 50–800 px
```

---

## Cargo changes

### Workspace Cargo.toml
Add to `[workspace.dependencies]`:
```toml
wgpu             = "22"
raw-window-handle = "0.6"
```
Add to windows features: `"Win32_UI_Shell"`

### cv-render/Cargo.toml
Add: `wgpu`, `raw-window-handle`

---

## cv-render crate structure

```
crates/cv-render/src/
├── lib.rs       — run_overlay, wnd_proc, WM_TIMER state machine
├── gfx.rs       — WgpuState struct and all wgpu operations
├── appbar.rs    — SHAppBarMessage helpers
└── shader.wgsl  — WGSL magnify shader
```

---

## gfx.rs — WgpuState

### Initialisation (called once at startup, blocking via `pollster::block_on`)

```
Instance::new(InstanceDescriptor { backends: Backends::DX12, .. })
→ create_surface_unsafe(SurfaceTargetUnsafe::RawHandle { RawDisplayHandle::Windows, RawWindowHandle::Win32 })
→ request_adapter (HighPerformance, compatible_surface)
→ request_device
→ surface.configure (format: Bgra8Unorm, PresentMode::Fifo)
→ create shader, bind group layout, pipeline layout, render pipeline
→ create frame texture (Bgra8Unorm, tex_w × tex_h = screen_w × screen_h)
→ create bilinear sampler
→ create uniform buffer (16 bytes: Crop { src_x, src_y, src_w, src_h })
→ create bind group
```

### wgpu 22 API notes (confirmed from docs)
- `Instance::new` takes `InstanceDescriptor` by **value** (no `&`)
- `SurfaceTargetUnsafe::RawHandle { raw_display_handle, raw_window_handle }`
- `Win32WindowHandle::new(NonZeroIsize)` — hinstance is `Option<NonZeroIsize>`, set after construction
- `entry_point` is `&str` (not `Option<&str>`)
- `compilation_options: PipelineCompilationOptions::default()` required on both VertexState and FragmentState
- `RenderPipelineDescriptor` has `cache: None` field
- `DeviceDescriptor` has `memory_hints: MemoryHints::default()` field
- Texture upload: `ImageCopyTexture` + `ImageDataLayout` (not TexelCopy* — those don't exist in 22)
- `request_adapter` returns `Option<Adapter>` (not Result)
- `SurfaceConfiguration` has `desired_maximum_frame_latency: 2`

### WGSL shader

Fullscreen quad (6 vertices, no vertex buffer). Uniform buffer carries normalised
crop rect. Fragment samples the frame texture at remapped UVs:

```wgsl
let uv = in.uv * vec2(crop.src_w, crop.src_h) + vec2(crop.src_x, crop.src_y);
return textureSample(frame_tex, frame_samp, uv);
```

UV orientation: (0,0) = top-left, (1,1) = bottom-right (matches DXGI top-down capture).

### Crop calculation (per-tick, written to uniform buffer)

```
win_w, win_h  = current window pixel dimensions (varies by mode)
src_w = win_w / zoom        (frame pixels shown horizontally)
src_h = win_h / zoom        (frame pixels shown vertically)
src_x = clamp(cx - src_w/2, 0, tex_w - src_w)
src_y = clamp(cy - src_h/2, 0, tex_h - src_h)
crop  = [src_x/tex_w, src_y/tex_h, src_w/tex_w, src_h/tex_h]
```

`cx, cy` = smoothed cursor position (same lerp formula as phase 1).

### WgpuState public methods
- `new(hwnd, win_w, win_h, tex_w, tex_h) -> Self`
- `resize(&mut self, w, h)` — reconfigures surface
- `upload_frame(&self, data, w, h)` — `queue.write_texture` (skips if dims mismatch)
- `write_crop(&self, [f32; 4])` — `queue.write_buffer`
- `render(&self) -> bool` — render pass + present, returns false on surface error

---

## appbar.rs — AppBar helpers

ABM/ABE constants defined locally (u32 literals) — don't rely on windows crate exporting them.

```rust
const ABM_NEW: u32 = 0;   const ABE_LEFT: u32 = 0;
const ABM_REMOVE: u32 = 1; const ABE_TOP: u32 = 1;
const ABM_QUERYPOS: u32 = 2; const ABE_RIGHT: u32 = 2;
const ABM_SETPOS: u32 = 3;  const ABE_BOTTOM: u32 = 3;

const ABN_POSCHANGED: usize = 1;
```

### Key functions

```rust
fn panel_rect(edge: Edge, thickness: i32, sw: i32, sh: i32) -> RECT
fn register(hwnd, edge, thickness, sw, sh, callback_msg) -> RECT   // ABM_NEW + QUERYPOS + SETPOS
fn reposition(hwnd, edge, thickness, sw, sh) -> RECT               // QUERYPOS + SETPOS only
fn unregister(hwnd)                                                 // ABM_REMOVE
```

`register` always calls `ABM_NEW` first, then `QUERYPOS`/`SETPOS` so Windows can
adjust the rect around other appbars (e.g. the taskbar).

---

## lib.rs — Window & state machine

### WindowData (thread_local)

```rust
struct WindowData {
    wgpu:           WgpuState,
    frame_state:    FrameState,
    app_state:      SharedState,
    smooth_x:       f32,
    smooth_y:       f32,
    last_tick:      Instant,
    screen_w:       i32,
    screen_h:       i32,
    hwnd:           HWND,
    callback_msg:   u32,
    // change-detection
    cur_enabled:    bool,
    cur_mode:       DisplayMode,
    cur_panel_size: u32,
    appbar_active:  bool,
}
```

### Window creation

- Style: `WS_POPUP` (no `WS_VISIBLE`) + `WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE`
- Initial size: **fullscreen** (`screen_w × screen_h`) regardless of starting mode.
  First WM_TIMER tick handles any needed resize.
- `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`
- `SetLayeredWindowAttributes(LWA_ALPHA, 255)`
- Callback message: `RegisterWindowMessageW("ClearViewAppBar")`
- `SetTimer(hwnd, 1, 16, None)`

### WM_TIMER logic (called ~60 Hz)

```
1. Read snapshot from WIN_DATA (immutable borrow):
   desired: enabled, mode, panel_size, zoom, smooth_speed, latest frame Arc

2. Detect changes vs cur_enabled / cur_mode / cur_panel_size

3. Handle transitions (outside WIN_DATA borrow — Win32 calls here):
   See transition table below.

4. Update WIN_DATA (mutable borrow):
   - Write cur_enabled / cur_mode / cur_panel_size / appbar_active
   - Call wgpu.resize(new_w, new_h) if window was resized

5. If enabled (mutable borrow, same scope as 4):
   - Lerp smooth cursor (dt-based)
   - Compute crop rect
   - upload_frame if new frame available
   - write_crop
   - render() — if returns false, call resize() to recover
```

### Transition table

| From | To | Win32 actions |
|---|---|---|
| disabled | enabled + Fullscreen | `SetWindowPos` fullscreen, `ShowWindow(SW_SHOW)` |
| disabled | enabled + Docked(e) | `appbar::register`, `SetWindowPos` to returned rect, `ShowWindow(SW_SHOW)` |
| enabled Fullscreen | Docked(e) | `appbar::register`, `SetWindowPos` to rect |
| enabled Docked(A) | Docked(B) | `appbar::unregister`, `appbar::register(B)`, `SetWindowPos` |
| enabled Docked(e) | Fullscreen | `appbar::unregister`, `SetWindowPos` fullscreen |
| enabled any | disabled | if docked: `appbar::unregister`; `ShowWindow(SW_HIDE)` |
| panel_size changed (enabled + docked) | — | `appbar::reposition`, `SetWindowPos` to returned rect |

**Key rule**: when disabling, window stays at panel size (if docked) — do NOT resize to
fullscreen on hide. Only resize when mode actually changes. This matches ZoomText behaviour.

### wnd_proc

```
WM_DESTROY      → PostQuitMessage(0)
WM_NCHITTEST    → HTTRANSPARENT
WM_TIMER        → (state machine above)
callback_msg    → if wParam == ABN_POSCHANGED: appbar::reposition + SetWindowPos + wgpu.resize
_               → DefWindowProcW
```

Callback message ID is read from WIN_DATA to determine which numeric message = appbar callback.

---

## app/src/app.rs — egui changes

Add below the existing enable toggle:

```
[Display mode]
○ Fullscreen  ○ Top  ○ Bottom  ○ Left  ○ Right

[Panel size (px)]   (slider 50–800, only visible when not Fullscreen)
```

egui window height increased from 220 → 280 to fit new controls.

---

## Window dimensions by mode

| Mode | win_w | win_h |
|---|---|---|
| Fullscreen | screen_w | screen_h |
| Docked Top/Bottom | screen_w | panel_size |
| Docked Left/Right | panel_size | screen_h |

Note: actual rect comes from ABM_SETPOS return value (Windows may adjust), not
raw calculation. Use the returned RECT for both SetWindowPos and wgpu.resize.

---

## What is NOT in phase 2

- Cursor overlay in magnified view (deferred to phase 5)
- Lanczos shader (deferred to phase 4)
- Multi-monitor (deferred to phase 6)

---

## Open questions before implementation

1. **egui window height**: Adding mode selector + panel size slider means the settings
   panel needs more vertical space. Current: 320×220. Proposed: 320×280.
   Is that fine, or do you want a different size / scrollable panel?

2. **Minimum wgpu surface size**: wgpu will error if the surface is configured with
   width or height of 0. If `panel_size` is set to a very small value and the QUERYPOS
   call shrinks it further (unlikely but possible), we need a floor. I'll clamp to
   a minimum of 16px on both dimensions. Fine?
