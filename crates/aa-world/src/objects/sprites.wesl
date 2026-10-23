// Vegetation sprites (objects/sprites.rs): every quad faces the camera around the up axis, shows the atlas view
// nearest to the viewing angle (24 around the model, as the native sprites), is alpha tested, lit like the meshes,
// and crossfades with the mesh at the switch distance and dissolves at the view distance with Bevy's own
// visibility-range dither pattern, so the mesh's fade-out and the sprite's fade-in complement each other.
import bevy_pbr::render::{mesh_functions, mesh_view_bindings::view, view_transformations::position_world_to_clip};
@if(!PREPASS_PIPELINE)
import bevy_pbr::render::{mesh_bindings::mesh, pbr_types, pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing}};
@if(PREPASS_PIPELINE && PREPASS_FRAGMENT)
import bevy_pbr::prepass::io as prepass_io;

struct SpriteParams {
    // columns, rows, border of a view (fraction), alpha test
    grid: vec4<f32>,
    // roughness, reflectance
    surface: vec4<f32>,
    // half width, half height, centre height
    shape: vec4<f32>,
}
@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> params: SpriteParams;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var albedo_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var atlas_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var normal_texture: texture_2d<f32>;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    // instance origin; the shader expands the shared quad
    @location(0) center: vec3<f32>,
    @location(1) corner: vec2<f32>,

}

struct Output {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) right: vec3<f32>,
    @location(3) toward: vec3<f32>,
    // dither levels: appearing (0 hidden .. 16 drawn), vanishing (0 drawn .. 16 hidden)
    @location(4) @interpolate(flat) levels: vec2<i32>,
    @location(5) @interpolate(flat) tag: u32,
    @location(6) @interpolate(flat) instance_index: u32,
}

const TAU: f32 = 6.283185307;
// Bevy's DITHER_THRESHOLD_MAP (pbr_functions), so the pattern matches the meshes' visibility-range dither.
const DITHER: vec4<u32> = vec4<u32>(0x0a020800u, 0x060e040cu, 0x09010b03u, 0x050d070fu);

// get_visibility_range_dither_level's mapping of a camera distance onto a 0..16 margin
fn level(d: f32, lo: f32, hi: f32) -> i32 {
    if hi <= lo {
        return select(0, 16, d >= hi);
    }
    return clamp(i32(round((d - lo) / (hi - lo) * 16.0)), 0, 16);
}

@vertex
fn vertex(v: Vertex) -> Output {
    var out: Output;
    let world_from_local = mesh_functions::get_world_from_local(v.instance_index);
    let origin = world_from_local[3].xyz;
    let scale = length(world_from_local[0].xyz);
    let center = origin + vec3<f32>(0.0, params.shape.z * scale, 0.0);
    let tag = mesh_functions::get_tag(v.instance_index);
    let appear = f32(tag >> 19u);
    let band = clamp(appear * 0.025, 2.0, 8.0);
    var to_camera = view.world_position.xyz - center;
    to_camera.y = 0.0;
    let toward = select(vec3<f32>(0.0, 0.0, 1.0), normalize(to_camera), dot(to_camera, to_camera) > 1e-6);
    let up = vec3<f32>(0.0, 1.0, 0.0);
    let right = cross(-toward, up);
    let world = center + right * ((v.corner.x * 2.0 - 1.0) * params.shape.x * scale) + up * ((v.corner.y * 2.0 - 1.0) * params.shape.y * scale);
    out.world_position = vec4<f32>(world, 1.0);
    out.position = position_world_to_clip(world);
    // the view: model-space angle of the viewer (cry: x east, y north) = world angle - instance yaw
    let yaw = atan2(-world_from_local[0].z, world_from_local[0].x);
    let angle = atan2(-toward.z, toward.x) - yaw;
    let slots = i32(params.grid.x * params.grid.y);
    let slot = ((i32(round(angle / TAU * f32(slots))) % slots) + slots) % slots;
    let cols = i32(params.grid.x);
    let cell = vec2<f32>(f32(slot % cols), f32(slot / cols));
    let inner = mix(vec2<f32>(params.grid.z), vec2<f32>(1.0 - params.grid.z), vec2<f32>(v.corner.x, 1.0 - v.corner.y));
    out.uv = (cell + inner) / params.grid.xy;
    // fades are measured from the instance origin, as Bevy measures the mesh parts
    let d = length(view.lod_view_world_position.xyz - origin);
    var vanish = 0;
    @if(VISIBILITY_RANGE_DITHER) {
        vanish = mesh_functions::get_visibility_range_dither_level(v.instance_index, vec4<f32>(origin, 1.0));
    }
    out.levels = vec2<i32>(level(d, max(appear - band, 0.0), appear), vanish);
    if out.levels.x == 0 || out.levels.y >= 16 {
        // not drawn at all at this distance: a degenerate quad outside the depth range
        out.position = vec4<f32>(0.0, 0.0, -1.0, 1.0);
    }
    out.right = right;
    out.toward = toward;
    out.tag = tag;
    out.instance_index = v.instance_index;
    return out;
}

// Alpha test and the dither of both fades.
fn visible(in: Output, texel: vec4<f32>) -> bool {
    let coords = vec2<u32>(floor(in.position.xy)) % 4u;
    let threshold = i32((DITHER[coords.y] >> (coords.x * 8u)) & 0xffu);
    return texel.a >= params.grid.w && in.levels.x + threshold >= 16 && in.levels.y + threshold < 16;
}

@if(!PREPASS_PIPELINE)
@fragment
fn fragment(in: Output) -> @location(0) vec4<f32> {
    let texel = textureSample(albedo_texture, atlas_sampler, in.uv);
    let encoded = textureSample(normal_texture, atlas_sampler, in.uv).xy * 2.0 - 1.0;
    if !visible(in, texel) {
        discard;
    }
    let normal = normalize(in.right * encoded.x + vec3<f32>(0.0, encoded.y, 0.0)
        + in.toward * sqrt(max(0.0, 1.0 - dot(encoded, encoded))));
    var pbr = pbr_types::pbr_input_new();
    pbr.material.base_color = vec4<f32>(texel.rgb, 1.0);
    pbr.material.perceptual_roughness = params.surface.x;
    pbr.material.reflectance = vec3<f32>(params.surface.y);
    pbr.material.flags = pbr_types::STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT | pbr_types::STANDARD_MATERIAL_FLAGS_ALPHA_MODE_OPAQUE;
    pbr.frag_coord = in.position;
    pbr.world_position = in.world_position;
    pbr.world_normal = normal;
    pbr.N = normal;
    pbr.V = normalize(view.world_position.xyz - in.world_position.xyz);
    pbr.flags = mesh[in.instance_index].flags;
    var color = apply_pbr_lighting(pbr);
    // the group brightness and COLOR_VARIATION of vegetation_color.wesl, on the lit colour
    color = vec4<f32>(color.rgb * params.surface.z, 1.0);
    let packed = in.tag;
    if (packed & 262144u) == 0u {
        let rgb = vec3<f32>(f32(packed & 63u), f32((packed >> 6u) & 63u), f32((packed >> 12u) & 63u)) / 63.0;
        color = vec4<f32>(color.rgb * (vec3<f32>(0.8) + rgb * 0.4), 1.0);
    }
    return main_pass_post_lighting_processing(pbr, color);
}

@if(PREPASS_PIPELINE && !PREPASS_FRAGMENT)
@fragment
fn fragment(in: Output) {
    let texel = textureSample(albedo_texture, atlas_sampler, in.uv);
    if !visible(in, texel) {
        discard;
    }
}

@if(PREPASS_PIPELINE && PREPASS_FRAGMENT)
@fragment
fn fragment(in: Output) -> prepass_io::FragmentOutput {
    let texel = textureSample(albedo_texture, atlas_sampler, in.uv);
    if !visible(in, texel) {
        discard;
    }
    var out: prepass_io::FragmentOutput;
    @if(NORMAL_PREPASS) {
        out.normal = vec4<f32>(in.toward * 0.5 + 0.5, 1.0);
    }
    @if(MOTION_VECTOR_PREPASS) {
        out.motion_vector = vec2<f32>(0.0);
    }
    return out;
}
