// A distant cloud sprite (`distanceclouds.cfx`): the red channel of the texture is the cloud's density. The colour lerps from
// the sky shading colour to the sun shading colour by how much cloud lies between the pixel and the sun (8 taps towards it in
// the sprite plane), the alpha is density ^ AlphaSaturation. Colours are pre-exposed HDR (CPU side), see `sky_clouds.rs`.

import bevy_pbr::render::{
    forward_io::VertexOutput,
    mesh_view_bindings::view,
};

struct CloudParams {
    // rgb: SkyColorMultiplier x cloud sky shading colour, a: 1 when the clouds are on
    shade_sky: vec4<f32>,
    // rgb: SunColorMultiplier x cloud sun shading colour
    shade_sun: vec4<f32>,
    // rgb: material diffuse colour, a: material opacity
    material: vec4<f32>,
    // x: attenuation, y: step size (uv per tap), z: alpha saturation, w: unused
    shape: vec4<f32>,
    // xyz: direction towards the sun (world), w: 1
    sun: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> cloud: CloudParams;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var cloud_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var cloud_sampler: sampler;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let opacity = textureSample(cloud_texture, cloud_sampler, uv).x;

    // the sprite faces the camera: its plane is spanned by the view's right and up
    let right = view.world_from_view[0].xyz;
    let up = view.world_from_view[1].xyz;
    let toward_sun = vec2<f32>(dot(cloud.sun.xyz, right), -dot(cloud.sun.xyz, up)) * cloud.shape.y;
    var density = 0.0;
    for (var i = 0; i < 8; i = i + 1) {
        density = density + textureSampleLevel(cloud_texture, cloud_sampler, uv + toward_sun * f32(i), 0.0).x;
    }
    let c = exp2(-cloud.shape.x * density);
    let a = pow(opacity, cloud.shape.z) * cloud.material.a;
    let col = mix(cloud.shade_sky.rgb, cloud.shade_sun.rgb, c) * cloud.material.rgb;

    // melt into the haze at the horizon
    let d = normalize(in.world_position.xyz - view.world_position);
    let fade = smoothstep(-0.01, 0.1, d.y);
    return vec4<f32>(col, a * fade * cloud.shade_sky.w);
}
