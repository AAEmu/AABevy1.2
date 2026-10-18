import bevy_pbr::render::{forward_io::{Vertex,VertexOutput,decompress_vertex},mesh_functions::get_world_from_local,mesh_view_bindings::view};
@if(VIEW_TRANSMISSION_TEXTURE) import bevy_pbr::render::mesh_view_bindings::{view_transmission_texture,view_transmission_sampler};
@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> color:vec4<f32>;
@vertex
fn vertex(input:Vertex)->VertexOutput {
    let vertex=decompress_vertex(input,input.instance_index);
    let world=get_world_from_local(input.instance_index)*vec4<f32>(vertex.position,1.0);
    var out:VertexOutput; out.position=view.clip_from_world*world;out.world_position=world;return out;
}
@fragment
fn fragment(input:VertexOutput)->@location(0) vec4<f32> {
    @if(OVERLAY_CONTROL) { return color; }
    @else {
    // Channel reversal does not commute with coloured fog. This distinguishes
    // fog in the actual water copy from the old fog overlay after that copy.
    let color=textureSampleLevel(view_transmission_texture,view_transmission_sampler,input.position.xy/vec2<f32>(textureDimensions(view_transmission_texture)),0.0).rgb;
    return vec4<f32>(color.bgr,1.0);
    }
}
