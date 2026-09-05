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

// blur / color_shift 作用于视频底图本身（对齐 Canvas：ctx.filter 在 drawImage 时生效）
struct FragUniforms {
    blur: f32,
    sepia: f32,
    saturate: f32,
    _pad: f32,
};

@group(0) @binding(3) var<uniform> fu: FragUniforms;

fn gaussian_weight(d: f32, sigma: f32) -> f32 {
    return exp(-d * d / (2.0 * sigma * sigma));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    var color = textureSample(src_tex, src_sampler, in.uv);

    // blur: 5x5 高斯核（对齐 CSS blur(8px) 的观感，sigma = blur/2 px）
    if (fu.blur > 0.0) {
        let sigma = max(fu.blur, 1.0) * 0.5;
        let step = sigma / 1.5;
        var sum = vec4<f32>(0.0);
        var wsum = 0.0;
        for (var dy = -2.0; dy <= 2.0; dy = dy + 1.0) {
            for (var dx = -2.0; dx <= 2.0; dx = dx + 1.0) {
                let w = gaussian_weight(dx, 1.0) * gaussian_weight(dy, 1.0);
                sum = sum + textureSample(src_tex, src_sampler, in.uv + vec2<f32>(dx, dy) * step / vec2<f32>(1920.0, 1080.0)) * w;
                wsum = wsum + w;
            }
        }
        color = sum / wsum;
    }

    // color_shift: sepia/saturate 随事件时间衰减（对齐 JS sepia(0.6→0) saturate(1.4→1)）
    if (fu.sepia > 0.0 || fu.saturate != 1.0) {
        let r = color.r;
        let g = color.g;
        let b = color.b;
        let sepia_r = min(1.0, r * 0.393 + g * 0.769 + b * 0.189);
        let sepia_g = min(1.0, r * 0.349 + g * 0.686 + b * 0.168);
        let sepia_b = min(1.0, r * 0.272 + g * 0.534 + b * 0.131);
        let sepia_color = vec3<f32>(mix(r, sepia_r, fu.sepia), mix(g, sepia_g, fu.sepia), mix(b, sepia_b, fu.sepia));
        let luma = dot(color.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
        let saturated = mix(vec3<f32>(luma), sepia_color, fu.saturate);
        color = vec4<f32>(saturated, color.a);
    }

    return color;
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
    hue_color: vec4<f32>,
    hue_alpha_pad: vec4<f32>,
    // x=intro black, y=vignette, z=noise(白色叠加 alpha), w=overlay 渐变 stop 数
    misc: vec4<f32>,
    // x=overlay 透明度, y=shape(0 linear / 1 radial)
    overlay_meta: vec4<f32>,
    grad0: vec4<f32>,
    grad1: vec4<f32>,
    grad2: vec4<f32>,
    grad3: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var src_tex: texture_2d<f32>;
@group(0) @binding(2) var src_sampler: sampler;

fn grad_stop(i: i32) -> vec4<f32> {
    if (i == 0) { return u.grad0; }
    if (i == 1) { return u.grad1; }
    if (i == 2) { return u.grad2; }
    return u.grad3;
}

fn sample_gradient(t: f32, count: i32) -> vec4<f32> {
    let n = max(count - 1, 1);
    let pos = clamp(t, 0.0, 1.0) * f32(n);
    let i = i32(floor(pos));
    let f = pos - f32(i);
    let a = grad_stop(i);
    let b = grad_stop(min(i + 1, count - 1));
    return mix(a, b, f);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    var color = textureSample(src_tex, src_sampler, in.uv);

    // noise: 白色叠加（对齐 Canvas fillStyle=white + globalAlpha）
    if (u.misc.z > 0.0) {
        color = mix(color, vec4<f32>(1.0), u.misc.z);
    }

    // flash: 直接混合到画面
    if (u.flash_alpha_pad.x > 0.0) {
        color = mix(color, u.flash_color, u.flash_alpha_pad.x);
    }

    // hue_shift flash
    if (u.hue_alpha_pad.x > 0.0) {
        color = mix(color, u.hue_color, u.hue_alpha_pad.x);
    }

    // overlay: 纯色（单 stop）或 linear/radial 渐变（多 stop，几何对齐 Canvas）
    if (u.overlay_meta.x > 0.0 && u.misc.w > 0) {
        var t = 0.0;
        if (u.overlay_meta.y < 0.5) {
            // linear: (0,0) → (1920,1080)，等比 UV 对角
            t = (in.uv.x + in.uv.y) * 0.5;
        } else {
            // radial: 圆心 (960,540) 半径 960
            let d = distance(in.uv, vec2<f32>(0.5, 0.5)) * 2.0;
            t = clamp(d, 0.0, 1.0);
        }
        let g = sample_gradient(t, i32(u.misc.w));
        color = mix(color, vec4<f32>(g.rgb, 1.0), u.overlay_meta.x);
    }

    // intro black
    if (u.misc.x > 0.0) {
        color = mix(color, vec4<f32>(0.0, 0.0, 0.0, 1.0), u.misc.x);
    }

    // vignette: 径向暗角
    if (u.misc.y > 0.0) {
        let uv = in.uv;
        let d = distance(uv, vec2<f32>(0.5, 0.5)) * 2.0;
        let t = clamp((d - 0.55) / 0.45, 0.0, 1.0);
        let dark = vec4<f32>(0.0, 0.0, 0.0, 1.0);
        color = mix(color, dark, t * 0.5 * u.misc.y);
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
    // x=opacity, y=rotation(rad), z=输出宽, w=输出高
    m: vec4<f32>,
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
    let cos = cos(u.m.y);
    let sin = sin(u.m.y);
    p = vec2<f32>(c.x + dx * cos - dy * sin, c.y + dx * sin + dy * cos);
    // convert to NDC（分辨率经 meta.zw 传入，不再硬编码 960/540）
    out.position = vec4<f32>(p.x / (u.m.z * 0.5) - 1.0, 1.0 - p.y / (u.m.w * 0.5), 0.0, 1.0);
    out.uv = uv[vertex_index] * vec2<f32>(u.src_rect.z - u.src_rect.x, u.src_rect.w - u.src_rect.y) + u.src_rect.xy;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let c = textureSample(tex, samp, in.uv);
    return vec4<f32>(c.rgb, c.a * u.m.x);
}
"#;
