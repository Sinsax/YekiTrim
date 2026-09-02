//! 编译期内嵌的 WGSL 着色器。

pub const FULLSCREEN_TEXTURED_VERT: &str = r#"
struct VsIn {
    @builtin(vertex_index) vertex_index: u32,
};

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VsOut {
    var out: VsOut;
    var pos = array<vec2<f32>, 4>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(1.0, 1.0),
    );
    var uv = array<vec2<f32>, 4>(
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
    );
    out.position = vec4<f32>(pos[vertex_index], 0.0, 1.0);
    out.uv = uv[vertex_index];
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(1.0, 0.0, 1.0, 1.0);
}
"#;

pub const SOURCE_VERT: &str = r#"
struct VsIn {
    @builtin(vertex_index) vertex_index: u32,
};

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

struct Uniforms {
    transform: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VsOut {
    var out: VsOut;
    var pos = array<vec2<f32>, 4>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(1.0, 1.0),
    );
    var uv = array<vec2<f32>, 4>(
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
    );
    out.position = u.transform * vec4<f32>(pos[vertex_index], 0.0, 1.0);
    out.uv = uv[vertex_index];
    return out;
}

"#;

pub const SOURCE_FRAG: &str = r#"
@group(0) @binding(1) var src_tex: texture_2d<f32>;
@group(0) @binding(2) var src_sampler: sampler;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(src_tex, src_sampler, in.uv);
}
"#;

pub const EFFECT_SHADER: &str = r#"
struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VsOut {
    var out: VsOut;
    var pos = array<vec2<f32>, 4>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(1.0, 1.0),
    );
    var uv = array<vec2<f32>, 4>(
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
    );
    out.position = vec4<f32>(pos[vertex_index], 0.0, 1.0);
    out.uv = uv[vertex_index];
    return out;
}

struct Uniforms {
    flash_color: vec4<f32>,
    flash_alpha_pad: vec4<f32>,
    overlay_color: vec4<f32>,
    overlay_alpha_pad: vec4<f32>,
    hue_color: vec4<f32>,
    hue_alpha_pad: vec4<f32>,
    misc_pad: vec4<f32>,
    blur_color_shift: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var src_tex: texture_2d<f32>;
@group(0) @binding(2) var src_sampler: sampler;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    var color = textureSample(src_tex, src_sampler, in.uv);

    // flash: 直接混合到画面
    if (u.flash_alpha_pad.x > 0.0) {
        color = mix(color, u.flash_color, u.flash_alpha_pad.x);
    }

    // overlay: 直接混合
    if (u.overlay_alpha_pad.x > 0.0) {
        color = mix(color, u.overlay_color, u.overlay_alpha_pad.x);
    }

    // hue_shift flash
    if (u.hue_alpha_pad.x > 0.0) {
        color = mix(color, u.hue_color, u.hue_alpha_pad.x);
    }

    // intro black
    if (u.misc_pad.x > 0.0) {
        color = mix(color, vec4<f32>(0.0, 0.0, 0.0, 1.0), u.misc_pad.x);
    }

    // vignette: 径向暗角
    if (u.misc_pad.y > 0.0) {
        let uv = in.uv;
        let d = distance(uv, vec2<f32>(0.5, 0.5)) * 2.0;
        let t = clamp((d - 0.55) / 0.45, 0.0, 1.0);
        let dark = vec4<f32>(0.0, 0.0, 0.0, 1.0);
        color = mix(color, dark, t * 0.5 * u.misc_pad.y);
    }

    // noise: 简单的确定性噪点（用 uv 生成伪随机）
    if (u.misc_pad.z > 0.0) {
        let n = fract(sin(dot(in.uv, vec2<f32>(12.9898, 78.233))) * 43758.5453);
        let noise_color = vec4<f32>(n, n, n, 1.0);
        color = mix(color, noise_color, u.misc_pad.z * 0.5);
    }

    // blur: 简单 3x3 box blur（采样周围像素）
    if (u.blur_color_shift.x > 0.0) {
        let off = u.blur_color_shift.x * 0.002;
        var blurred = vec4<f32>(0.0);
        let offsets = array<vec2<f32>, 9>(
            vec2<f32>(-1.0, -1.0), vec2<f32>(0.0, -1.0), vec2<f32>(1.0, -1.0),
            vec2<f32>(-1.0, 0.0), vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0),
            vec2<f32>(-1.0, 1.0), vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 1.0)
        );
        for (var i = 0; i < 9; i = i + 1) {
            blurred += textureSample(src_tex, src_sampler, in.uv + offsets[i] * off);
        }
        color = blurred / 9.0;
    }

    // color_shift: 简化 sepia 效果
    if (u.blur_color_shift.y > 0.0) {
        let r = color.r;
        let g = color.g;
        let b = color.b;
        color.r = min(1.0, r * 0.393 + g * 0.769 + b * 0.189);
        color.g = min(1.0, r * 0.349 + g * 0.686 + b * 0.168);
        color.b = min(1.0, r * 0.272 + g * 0.534 + b * 0.131);
    }

    return color;
}
"#;

pub const SPRITE_SHADER: &str = r#"
struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

struct Uniforms {
    dst_rect: vec4<f32>,
    src_rect: vec4<f32>,
    opacity: f32,
    rotation: f32,
    _pad: vec3<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var tex: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VsOut {
    var out: VsOut;
    var pos = array<vec2<f32>, 4>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 1.0),
    );
    var uv = array<vec2<f32>, 4>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 1.0),
    );
    let x = u.dst_rect.x;
    let y = u.dst_rect.y;
    let w = u.dst_rect.z;
    let h = u.dst_rect.w;
    let c = vec2<f32>(x + w * 0.5, y + h * 0.5);
    var p = pos[vertex_index];
    p = p * vec2<f32>(w, h) + vec2<f32>(x, y);
    // rotate around center
    let dx = p.x - c.x;
    let dy = p.y - c.y;
    let cos = cos(u.rotation);
    let sin = sin(u.rotation);
    p = vec2<f32>(c.x + dx * cos - dy * sin, c.y + dx * sin + dy * cos);
    // convert to NDC
    out.position = vec4<f32>(p.x / 960.0 - 1.0, 1.0 - p.y / 540.0, 0.0, 1.0);
    out.uv = uv[vertex_index] * vec2<f32>(u.src_rect.z - u.src_rect.x, u.src_rect.w - u.src_rect.y) + u.src_rect.xy;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let c = textureSample(tex, samp, in.uv);
    return vec4<f32>(c.rgb, c.a * u.opacity);
}
"#;
