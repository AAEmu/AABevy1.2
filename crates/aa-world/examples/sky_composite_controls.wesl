import bevy_pbr::render::{
    forward_io::{Vertex, VertexOutput, decompress_vertex},
    mesh_functions::get_world_from_local,
    mesh_view_bindings::view,
};
@if(VIEW_TRANSMISSION_TEXTURE) import bevy_pbr::render::mesh_view_bindings::{view_transmission_texture, view_transmission_sampler};
struct Control { color: vec4<f32>, settings: vec4<f32>, }
@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> control: Control;
@vertex
fn vertex(input: Vertex) -> VertexOutput {
    let vertex = decompress_vertex(input, input.instance_index);
    let world = get_world_from_local(input.instance_index) * vec4<f32>(vertex.position, 1.0);
    var out: VertexOutput;
    let clip = view.clip_from_world * world;
    out.position = vec4<f32>(clip.xy, select(clip.z, 0.0, control.settings.x > 0.5), clip.w);
    out.world_position = world;
    return out;
}
@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    @if(CAPTURE_CONTROL) {
        return vec4<f32>(textureSampleLevel(view_transmission_texture, view_transmission_sampler, in.position.xy / vec2<f32>(textureDimensions(view_transmission_texture)), 0.0).rgb, 1.0);
    }
    return control.color;
}
