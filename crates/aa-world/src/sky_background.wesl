// Native sky vertices use far depth. The opaque black base covers ClearColor
// without writing depth; stars/moons and the late sky composite follow it.
import bevy_pbr::render::{
    forward_io::{Vertex, VertexOutput, decompress_vertex},
    mesh_functions::get_world_from_local,
    mesh_view_bindings::view,
};

@vertex
fn vertex(input: Vertex) -> VertexOutput {
    let vertex = decompress_vertex(input, input.instance_index);
    let world = get_world_from_local(input.instance_index) * vec4<f32>(vertex.position, 1.0);
    let clip = view.clip_from_world * world;
    var out: VertexOutput;
    out.position = vec4<f32>(clip.xy, 0.0, clip.w);
    out.world_position = world;
    return out;
}

@fragment
fn fragment() -> @location(0) vec4<f32> {
    return vec4<f32>(0.0, 0.0, 0.0, 1.0);
}
