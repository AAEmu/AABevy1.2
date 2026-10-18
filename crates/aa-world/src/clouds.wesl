// A distant cloud sprite (`distanceclouds.cfx`): the red channel of the texture is the cloud's density. The colour lerps from
// the sky shading colour to the sun shading colour by how much cloud lies between the pixel and the sun (8 taps towards it in
// the sprite plane), the alpha is density ^ AlphaSaturation. Colours are pre-exposed HDR (CPU side), see `sky_clouds.rs`.

import bevy_pbr::render::{
    forward_io::{Vertex, decompress_vertex},
    mesh_functions::get_world_from_local,
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

struct CloudVertex {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) right: vec3<f32>,
    @location(2) up: vec3<f32>,
}

@vertex
fn vertex(input: Vertex) -> CloudVertex {
    let vertex = decompress_vertex(input, input.instance_index);
    let world_from_local = get_world_from_local(input.instance_index);
    let world = world_from_local * vec4<f32>(vertex.position, 1.0);
    let clip = view.clip_from_world * world;
    var out: CloudVertex;
    // Original DistanceCloudsVS sets z=w. Bevy uses reverse Z: far depth is zero.
    out.clip = vec4<f32>(clip.xy, 0.0, clip.w);
    out.uv = vertex.uv;
    out.right = normalize(world_from_local[0].xyz);
    out.up = normalize(world_from_local[1].xyz);
    return out;
}

@fragment
fn fragment(in: CloudVertex) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let opacity = textureSample(cloud_texture, cloud_sampler, uv).x;

    // Native packed tangent / binormal are radial right / -up, independent of camera roll.
    let toward_sun = vec2<f32>(dot(cloud.sun.xyz, in.right), -dot(cloud.sun.xyz, in.up)) * cloud.shape.y;
    var density = 0.0;
    for (var i = 0; i < 8; i = i + 1) {
        density = density + textureSampleLevel(cloud_texture, cloud_sampler, uv + toward_sun * f32(i), 0.0).x;
    }
    let c = exp2(-cloud.shape.x * density);
    let a = pow(opacity, cloud.shape.z) * cloud.material.a;
    let col = mix(cloud.shade_sky.rgb, cloud.shade_sun.rgb, c) * cloud.material.rgb;

    return vec4<f32>(col, a * cloud.shade_sky.w);
}
