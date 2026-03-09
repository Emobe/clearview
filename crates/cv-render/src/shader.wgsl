// Uniforms — 32 bytes (crop 16 bytes + color_mode 4 bytes + interp_mode 4 bytes + 8 bytes padding).
// color_mode:  0=None, 1=Inverted, 2=Greyscale, 3=GreyscaleInverted
// interp_mode: 0=Bilinear, 1=Bicubic (Catmull-Rom)
struct Uniforms {
    src_x:       f32,
    src_y:       f32,
    src_w:       f32,
    src_h:       f32,
    color_mode:  u32,
    interp_mode: u32,
    _pad2:       u32,
    _pad3:       u32,
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

// Catmull-Rom cubic basis weights for fractional offset t in [0, 1].
// Returns weights for pixels at offsets -1, 0, +1, +2 from the floor.
fn cubic_weights(t: f32) -> vec4<f32> {
    let t2 = t * t;
    let t3 = t2 * t;
    return vec4(
        -0.5*t3 + 1.0*t2 - 0.5*t,
         1.5*t3 - 2.5*t2 + 1.0,
        -1.5*t3 + 2.0*t2 + 0.5*t,
         0.5*t3 - 0.5*t2,
    );
}

// Bicubic (Catmull-Rom) sample of frame_tex at UV coord uv.
// Uses textureLoad for exact texel reads — no sampler-level filtering.
fn sample_bicubic(uv: vec2<f32>) -> vec4<f32> {
    let dims  = vec2<f32>(textureDimensions(frame_tex, 0));
    let pixel = uv * dims - 0.5;
    let ip    = vec2<i32>(floor(pixel));
    let frac  = fract(pixel);
    let wx    = cubic_weights(frac.x);
    let wy    = cubic_weights(frac.y);
    let imax  = vec2<i32>(dims) - vec2(1);

    var col = vec4(0.0);
    for (var j = 0i; j < 4i; j = j + 1) {
        var row_col = vec4(0.0);
        for (var i = 0i; i < 4i; i = i + 1) {
            let coord = clamp(ip + vec2(i - 1, j - 1), vec2(0), imax);
            row_col += wx[i] * textureLoad(frame_tex, coord, 0);
        }
        col += wy[j] * row_col;
    }
    return clamp(col, vec4(0.0), vec4(1.0));
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    // Remap panel UV into the crop sub-region of the frame texture.
    let uv = in.uv * vec2(u.src_w, u.src_h) + vec2(u.src_x, u.src_y);

    var col: vec4<f32>;
    if u.interp_mode == 1u {
        col = sample_bicubic(uv);
    } else {
        col = textureSample(frame_tex, frame_samp, uv);
    }

    if u.color_mode == 1u {
        // Inverted
        return vec4(1.0 - col.r, 1.0 - col.g, 1.0 - col.b, col.a);
    } else if u.color_mode == 2u {
        // Greyscale (standard luminance weights)
        let lum = dot(col.rgb, vec3(0.299, 0.587, 0.114));
        return vec4(lum, lum, lum, col.a);
    } else if u.color_mode == 3u {
        // Greyscale + Inverted
        let lum = dot(col.rgb, vec3(0.299, 0.587, 0.114));
        return vec4(1.0 - lum, 1.0 - lum, 1.0 - lum, col.a);
    }

    return col;
}
