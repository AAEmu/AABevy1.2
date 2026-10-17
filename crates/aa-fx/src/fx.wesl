// A particle emitter's quads (`material.rs`): texture times vertex colour, soft against the depth prepass.
//
// The vertex colour carries the particle's colour (already lit) and alpha. `fx.gain.x` scales the colour into the camera's HDR
// range (the world's materials output pre-exposed colours, see `FxSettings::gain`), `fx.gain.y` is the soft particle distance
// in metres (0 = off), `fx.gain.z` is the blend mode (0 alpha, 1 premultiplied, 2 additive, 3 multiply).

import bevy_pbr::render::{
    forward_io::VertexOutput,
    view_transformations::depth_ndc_to_view_z,
};
@if(DEPTH_PREPASS) import bevy_pbr::prepass::utils::prepass_depth;

struct FxParams {
    gain: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> fx: FxParams;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var fx_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var fx_sampler: sampler;

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

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let t = textureSample(fx_texture, fx_sampler, in.uv);
    let a = t.a * in.color.a * soft_fade(in.position);
    let rgb = t.rgb * in.color.rgb * fx.gain.x;
    // Bevy's additive, premultiplied and multiply modes all expect premultiplied colour (additive with alpha 0)
    if fx.gain.z > 1.5 && fx.gain.z < 2.5 {
        return vec4<f32>(rgb * a, 0.0);
    }
    if fx.gain.z > 0.5 {
        return vec4<f32>(rgb * a, a);
    }
    return vec4<f32>(rgb, a);
}
