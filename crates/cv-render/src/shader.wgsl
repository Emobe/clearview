// Uniforms — 32 bytes (crop 16 bytes + filter 4 bytes + 12 bytes padding).
// filter: 0=None, 1=Inverted, 2=Greyscale, 3=GreyscaleInverted
struct Uniforms {
    src_x:  f32,
    src_y:  f32,
    src_w:  f32,
    src_h:  f32,
    filter: u32,
    _pad1:  u32,
    _pad2:  u32,
    _pad3:  u32,
}

@group(0) @binding(0) var<uniform> u:          Uniforms;
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
    let uv  = in.uv * vec2(u.src_w, u.src_h) + vec2(u.src_x, u.src_y);
    let col = textureSample(frame_tex, frame_samp, uv);

    if u.filter == 1u {
        // Inverted
        return vec4(1.0 - col.r, 1.0 - col.g, 1.0 - col.b, col.a);
    } else if u.filter == 2u {
        // Greyscale (standard luminance weights)
        let lum = dot(col.rgb, vec3(0.299, 0.587, 0.114));
        return vec4(lum, lum, lum, col.a);
    } else if u.filter == 3u {
        // Greyscale + Inverted
        let lum = dot(col.rgb, vec3(0.299, 0.587, 0.114));
        return vec4(1.0 - lum, 1.0 - lum, 1.0 - lum, col.a);
    }

    return col;
}
