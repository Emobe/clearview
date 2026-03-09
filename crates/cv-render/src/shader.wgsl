// Crop parameters — all values normalised to [0, 1] in texture space.
struct Crop {
    src_x: f32,
    src_y: f32,
    src_w: f32,
    src_h: f32,
}

@group(0) @binding(0) var<uniform> crop:       Crop;
@group(0) @binding(1) var          frame_tex:  texture_2d<f32>;
@group(0) @binding(2) var          frame_samp: sampler;

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0)       uv:  vec2<f32>,
}

// Fullscreen quad — 6 vertices, no vertex buffer.
// DXGI data is top-down; wgpu texture UV (0,0) = top-left. No flip needed.
@vertex
fn vs(@builtin(vertex_index) vi: u32) -> VOut {
    // NDC positions
    var xy = array<vec2<f32>, 6>(
        vec2(-1.0, -1.0), vec2( 1.0, -1.0), vec2(-1.0,  1.0),
        vec2(-1.0,  1.0), vec2( 1.0, -1.0), vec2( 1.0,  1.0),
    );
    // Texture UVs: NDC y flips → v = 1 - (ndc_y*0.5 + 0.5)
    var uv = array<vec2<f32>, 6>(
        vec2(0.0, 1.0), vec2(1.0, 1.0), vec2(0.0, 0.0),
        vec2(0.0, 0.0), vec2(1.0, 1.0), vec2(1.0, 0.0),
    );
    var out: VOut;
    out.pos = vec4(xy[vi], 0.0, 1.0);
    out.uv  = uv[vi];
    return out;
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    // Remap panel UV into the crop sub-region of the frame texture.
    let uv = in.uv * vec2(crop.src_w, crop.src_h) + vec2(crop.src_x, crop.src_y);
    return textureSample(frame_tex, frame_samp, uv);
}
