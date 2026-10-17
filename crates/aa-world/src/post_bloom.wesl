// The blur chain of the bloom (see `post_bloom.rs`): `downsample` halves a mip with the 13 tap filter of "Next
// Generation Post Processing in Call of Duty" (smooth, no flicker), `upsample` adds a 3x3 tent filtered lower mip onto
// the one above it (blend: add). The chain stands in for the engine's three Gaussian blurred bloom levels
// (`HDRFlaresPS` sums them without weights).

@group(0) @binding(0) var input_texture: texture_2d<f32>;
@group(0) @binding(1) var s: sampler;

@fragment
fn downsample(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let a = textureSample(input_texture, s, uv, vec2<i32>(-2, 2)).rgb;
    let b = textureSample(input_texture, s, uv, vec2<i32>(0, 2)).rgb;
    let c = textureSample(input_texture, s, uv, vec2<i32>(2, 2)).rgb;
    let d = textureSample(input_texture, s, uv, vec2<i32>(-2, 0)).rgb;
    let e = textureSample(input_texture, s, uv).rgb;
    let f = textureSample(input_texture, s, uv, vec2<i32>(2, 0)).rgb;
    let g = textureSample(input_texture, s, uv, vec2<i32>(-2, -2)).rgb;
    let h = textureSample(input_texture, s, uv, vec2<i32>(0, -2)).rgb;
    let i = textureSample(input_texture, s, uv, vec2<i32>(2, -2)).rgb;
    let j = textureSample(input_texture, s, uv, vec2<i32>(-1, 1)).rgb;
    let k = textureSample(input_texture, s, uv, vec2<i32>(1, 1)).rgb;
    let l = textureSample(input_texture, s, uv, vec2<i32>(-1, -1)).rgb;
    let m = textureSample(input_texture, s, uv, vec2<i32>(1, -1)).rgb;
    var sample = (a + c + g + i) * 0.03125;
    sample += (b + d + f + h) * 0.0625;
    sample += (e + j + k + l + m) * 0.125;
    return vec4<f32>(sample, 1.0);
}

@fragment
fn upsample(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let frag_size = 1.0 / vec2<f32>(textureDimensions(input_texture));
    let x = frag_size.x;
    let y = frag_size.y;

    let a = textureSample(input_texture, s, vec2<f32>(uv.x - x, uv.y + y)).rgb;
    let b = textureSample(input_texture, s, vec2<f32>(uv.x, uv.y + y)).rgb;
    let c = textureSample(input_texture, s, vec2<f32>(uv.x + x, uv.y + y)).rgb;
    let d = textureSample(input_texture, s, vec2<f32>(uv.x - x, uv.y)).rgb;
    let e = textureSample(input_texture, s, vec2<f32>(uv.x, uv.y)).rgb;
    let f = textureSample(input_texture, s, vec2<f32>(uv.x + x, uv.y)).rgb;
    let g = textureSample(input_texture, s, vec2<f32>(uv.x - x, uv.y - y)).rgb;
    let h = textureSample(input_texture, s, vec2<f32>(uv.x, uv.y - y)).rgb;
    let i = textureSample(input_texture, s, vec2<f32>(uv.x + x, uv.y - y)).rgb;

    var sample = e * 0.25;
    sample += (b + d + f + h) * 0.125;
    sample += (a + c + g + i) * 0.0625;
    return vec4<f32>(sample, 1.0);
}
