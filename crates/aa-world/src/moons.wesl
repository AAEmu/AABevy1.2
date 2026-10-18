// Original x2moon.cfx and renderer 381B8550 / 381B89F0 geometry and constants.
import bevy_pbr::render::mesh_view_bindings::view;

struct MoonParams {
    direction: vec4<f32>, right: vec4<f32>, up: vec4<f32>,
    color: vec4<f32>, inner: vec4<f32>, outer: vec4<f32>,
}
@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> moon: MoonParams;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var moon_map: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var moon_sampler: sampler;

struct Input { @location(0) corner: vec3<f32>, }
struct Output {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) direction: vec3<f32>,
}

@vertex
fn vertex(input: Input) -> Output {
    var out: Output;
    let pos = moon.direction.xyz + moon.direction.w * (moon.right.xyz * input.corner.x + moon.up.xyz * input.corner.y);
    let clip = view.clip_from_world * vec4<f32>(pos, 0.0);
    out.clip = vec4<f32>(clip.xy, 0.0, clip.w);
    out.uv = 0.5 + vec2<f32>(input.corner.x, -input.corner.y) * moon.right.w;
    out.direction = pos;
    return out;
}

@fragment
fn fragment(input: Output) -> @location(0) vec4<f32> {
    let albedo = textureSample(moon_map, moon_sampler, input.uv);
    let m = 1.0 - dot(normalize(input.direction), moon.direction.xyz);
    var color = moon.color.rgb * albedo.rgb;
    color += moon.inner.rgb / (1.05 + m * moon.inner.w);
    color += moon.outer.rgb / (1.05 + m * moon.outer.w);
    let ratio = saturate(1.0 - length(input.uv - 0.5) / moon.right.w);
    let alpha = max(albedo.a, ratio * ratio);
    // Native HDR path clamps before HDROutput; use the same sky exposure as other sky draws.
    return vec4<f32>(min(color, vec3<f32>(16384.0)) * moon.up.w, alpha);
}
