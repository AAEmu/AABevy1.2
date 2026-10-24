// SUPPLIED scalar coverage and Bevy velocity encoding. No native instance defaults.
import bevy_pbr::render::{mesh_functions, mesh_bindings::mesh, pbr_bindings, mesh_view_bindings::view, view_transformations::position_world_to_clip};
@if(SUPPLIED_PREPASS) import bevy_pbr::prepass::{io::{Vertex, decompress_vertex}, bindings::previous_view_uniforms};
@if(!SUPPLIED_PREPASS) import bevy_pbr::render::forward_io::{Vertex, decompress_vertex};
@if(BINDLESS) import bevy_pbr::render::pbr_bindings::material_indices;

struct Output {
    @builtin(position) @invariant position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) control: vec4<f32>,
    @location(2) world: vec4<f32>,
    @location(3) previous_world: vec4<f32>,
    @location(4) @interpolate(flat) instance: u32,
}
// Rigid supplied controls only. Skin, morph and outer LOD overrides require the
// production shared invariant position contract; do not route those through here.
@vertex fn vertex(input: Vertex) -> Output {
    let v = decompress_vertex(input, input.instance_index);
    var out: Output;
    out.world = mesh_functions::mesh_position_local_to_world(mesh_functions::get_world_from_local(input.instance_index), vec4(v.position, 1.0));
    out.previous_world = mesh_functions::mesh_position_local_to_world(mesh_functions::get_previous_world_from_local(input.instance_index), vec4(v.position, 1.0));
    out.position = position_world_to_clip(out.world.xyz);
    out.uv = v.uv;
    out.control = v.color;
    out.instance = input.instance_index;
    return out;
}
fn raw_alpha(in: Output) -> f32 { return select(in.control.r, in.control.g, in.uv.x >= 0.5); }
@if(SUPPLIED_PREPASS)
struct PrepassOutput {
    @builtin(frag_depth) depth: f32,
    @if(NORMAL_PREPASS) @location(0) normal: vec4<f32>,
    @if(MOTION_VECTOR_PREPASS) @location(1) velocity: vec2<f32>,
}
@if(SUPPLIED_PREPASS)
@fragment fn fragment(in: Output) -> PrepassOutput {
    @if(SUPPLIED_MOTION && !SUPPLIED_BLEND_MOTION) if raw_alpha(in) < 0.5 { discard; }
    @if(!SUPPLIED_MOTION) if raw_alpha(in) < in.control.b { discard; }
    var out: PrepassOutput;
    out.depth = in.position.z;
    @if(NORMAL_PREPASS) out.normal = vec4(0.5, 0.5, 1.0, 1.0);
    @if(MOTION_VECTOR_PREPASS) {
        let current = view.unjittered_clip_from_world * in.world;
        let previous = previous_view_uniforms.unjittered_clip_from_world * in.previous_world;
        out.velocity = (current.xy / current.w - previous.xy / previous.w) * vec2(0.5, -0.5);
        @if(SUPPLIED_BLEND_MOTION) out.velocity *= raw_alpha(in) * in.control.a;
    }
    return out;
}
@if(!SUPPLIED_PREPASS)
@fragment fn fragment(in: Output) -> @location(0) vec4<f32> {
    @if(!SUPPLIED_AFTER_Z) if in.control.a < in.control.b { discard; }
    @if(BINDLESS) let slot = mesh[in.instance].material_and_lightmap_bind_group_slot & 0xffffu;
    @if(BINDLESS) let colour = pbr_bindings::material_array[material_indices[slot].material].base_color;
    @if(!BINDLESS) let colour = pbr_bindings::material.base_color;
    return vec4(colour.rgb, in.control.a);
}
