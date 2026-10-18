import bevy_pbr::render::{
    forward_io::{Vertex, VertexOutput, decompress_vertex},
    mesh_functions,
    mesh_view_bindings::view,
    view_transformations::position_world_to_clip,
};

// Same static position/normal transform as Bevy's standard mesh vertex stage.
// UV1 carries the native surface/dominant-axis mask; distance fade belongs at vertices.
@vertex
fn vertex(input: Vertex) -> VertexOutput {
    let vertex = decompress_vertex(input, input.instance_index);
    let matrix = mesh_functions::get_world_from_local(input.instance_index);
    var out: VertexOutput;
    out.world_position = mesh_functions::mesh_position_local_to_world(matrix, vec4<f32>(vertex.position,1.0));
    out.position = position_world_to_clip(out.world_position.xyz);
    out.world_normal = mesh_functions::mesh_normal_local_to_world(vertex.normal,input.instance_index);
    out.uv = vertex.uv;
    out.color = vertex.color;
    @if(VERTEX_UVS_B) {
        // 128 m single-projection /1024 m multi-projection defaults, passed in UV1.y.
        let dist = distance(view.world_position.xyz,out.world_position.xyz);
        let fade = 1.0 - pow(min(dist / max(0.001,vertex.uv_b.y),1.0),4.0);
        out.uv_b = vec2<f32>(vertex.uv_b.x * fade,0.0);
    }
    @if(VERTEX_OUTPUT_INSTANCE_INDEX) { out.instance_index = input.instance_index; }
    @if(VISIBILITY_RANGE_DITHER) {
        out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(input.instance_index,matrix[3]);
    }
    return out;
}
