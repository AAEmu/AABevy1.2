// A particle emitter's quads (`material.rs`): texture times vertex colour, soft against the depth prepass.
//
// The vertex colour carries the particle's colour (already lit) and alpha. `fx.gain.x` scales the colour into the camera's HDR
// range (the world's materials output pre-exposed colours, see `FxSettings::gain`), `fx.gain.y` is the soft particle distance
// in metres (0 = off), `fx.gain.z` is the blend mode (0 alpha, 1 premultiplied, 2 additive, 3 multiply, 4 opaque) and
// `fx.gain.w` the alpha test threshold. Mesh particles with a separate opacity map (`fx.mask` weights its channels) sample it
// with the mesh's second texture coordinates, so that the diffuse texture can scroll over a fixed mask.

import bevy_pbr::render::{
    forward_io::VertexOutput,
    view_transformations::depth_ndc_to_view_z,
};
@if(DEPTH_PREPASS) import bevy_pbr::prepass::utils::prepass_depth;
@if(VIEW_TRANSMISSION_TEXTURE) import bevy_pbr::render::mesh_view_bindings as view_bindings;

struct FxParams {
    gain: vec4<f32>,
    mask: vec4<f32>,
    refr: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> fx: FxParams;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var fx_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var fx_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var fx_mask: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(4) var fx_normal: texture_2d<f32>;

fn linear_depth(ndc_z: f32) -> f32 {
    return -depth_ndc_to_view_z(max(ndc_z, 1e-9));
}

// Fade of a particle near solid geometry: 0 where it touches the scene, 1 further than the soft distance in front of it.
@if(DEPTH_PREPASS)
fn soft_fade(frag: vec4<f32>) -> f32 {
    if fx.gain.y <= 0.0 {
        return 1.0;
    }
    let scene = prepass_depth(vec4<f32>(frag.xy, 0.0, 0.0), 0u);
    if scene <= 0.0 {
        return 1.0;
    }
    let gap = linear_depth(scene) - linear_depth(frag.z);
    return clamp(gap / fx.gain.y, 0.0, 1.0);
}
@else
fn soft_fade(frag: vec4<f32>) -> f32 {
    return 1.0;
}

// Opacity of the separate opacity map (1 where the material has none).
@if(VERTEX_UVS_B)
fn mask_opacity(in: VertexOutput) -> f32 {
    if fx.mask.x + fx.mask.y + fx.mask.z + fx.mask.w <= 0.0 {
        return 1.0;
    }
    return dot(textureSample(fx_mask, fx_sampler, in.uv_b), fx.mask);
}
@else
fn mask_opacity(in: VertexOutput) -> f32 {
    return 1.0;
}

// A refraction particle shows the scene behind it, shifted by its normal map: alpha blended over the same scene it copies.
@if(VIEW_TRANSMISSION_TEXTURE)
fn refract_scene(in: VertexOutput, t: vec4<f32>) -> vec4<f32> {
    let a = t.a * in.color.a * soft_fade(in.position);
    let n = textureSample(fx_normal, fx_sampler, in.uv).xy * 2.0 - 1.0;
    let full = vec2<f32>(textureDimensions(view_bindings::view_transmission_texture));
    let shift = n * fx.refr.x * view_bindings::view.viewport.w * a;
    let bg = textureSampleLevel(view_bindings::view_transmission_texture, view_bindings::view_transmission_sampler, (in.position.xy + shift) / full, 0.0);
    return vec4<f32>(bg.rgb, a);
}
@else
fn refract_scene(in: VertexOutput, t: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(0.0);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let t = textureSample(fx_texture, fx_sampler, in.uv);
    if fx.refr.y > 0.5 {
        return refract_scene(in, t);
    }
    let a = t.a * mask_opacity(in) * in.color.a * soft_fade(in.position);
    if a < fx.gain.w {
        discard;
    }
    let rgb = t.rgb * in.color.rgb * fx.gain.x;
    if fx.gain.z > 3.5 {
        return vec4<f32>(rgb, 1.0);
    }
    // Bevy's additive, premultiplied and multiply modes all expect premultiplied colour (additive with alpha 0)
    if fx.gain.z > 1.5 && fx.gain.z < 2.5 {
        return vec4<f32>(rgb * a, 0.0);
    }
    if fx.gain.z > 0.5 {
        return vec4<f32>(rgb * a, a);
    }
    return vec4<f32>(rgb, a);
}
