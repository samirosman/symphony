// Symphony: backdrop blur. A full-screen triangle samples the source; with
// a zero direction it just copies (the downscale), otherwise it applies a
// 13-tap Gaussian along `dir` (one texel step per tap).

struct Params {
    dir: vec2<f32>,
    pad: vec2<f32>,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var<uniform> p: Params;

struct VertexOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VertexOut {
    var o: VertexOut;
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    o.pos = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    o.uv = vec2<f32>(x, y);
    return o;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    if (p.dir.x == 0.0 && p.dir.y == 0.0) {
        return textureSample(src, samp, in.uv);
    }
    var w = array<f32, 7>(0.137023, 0.129618, 0.109719, 0.083109, 0.056332, 0.034167, 0.018544);
    var c = textureSample(src, samp, in.uv) * w[0];
    for (var k = 1; k < 7; k = k + 1) {
        let o = p.dir * f32(k);
        c = c + (textureSample(src, samp, in.uv + o) + textureSample(src, samp, in.uv - o)) * w[k];
    }
    return c;
}
