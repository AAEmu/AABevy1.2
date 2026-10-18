import aa_world::objects::voxels::io::{VoxelVertexOutput, voxel};
import bevy_pbr::render::{
    forward_io::{Vertex, decompress_vertex},
    mesh_functions,
    mesh_view_bindings::view,
    view_transformations::position_world_to_clip,
};

// Same static position/normal transform as Bevy's standard mesh vertex stage.
// UV1 carries the native surface/dominant-axis mask; distance fade belongs at vertices.
@vertex
fn vertex(input: Vertex) -> VoxelVertexOutput {
    let vertex = decompress_vertex(input, input.instance_index);
    let matrix = mesh_functions::get_world_from_local(input.instance_index);
    var out: VoxelVertexOutput;
    out.world_position = mesh_functions::mesh_position_local_to_world(matrix, vec4<f32>(vertex.position,1.0));
    out.position = position_world_to_clip(out.world_position.xyz);
    out.world_normal = mesh_functions::mesh_normal_local_to_world(vertex.normal,input.instance_index);
    out.projected_u = vec3<f32>(0.0);
    out.projected_v = vec3<f32>(0.0);
    if voxel.flags.y > 0.5 {
        // This path qualifies translation-only authored voxel instances. Mesh normals already
        // carry renderer-packed native values in Bevy space; native does not normalize here.
        out.world_normal = vertex.normal;
        if voxel.normals.x > 0.5 {
            let u = normalize(vec3<f32>(voxel.u.x,voxel.u.z,-voxel.u.y));
            let v = normalize(vec3<f32>(voxel.v.x,voxel.v.z,-voxel.v.y));
            out.projected_u = normalize(u - out.world_normal * dot(u,out.world_normal));
            out.projected_v = normalize(v - out.world_normal * dot(v,out.world_normal));
        }
    }
    @if(VERTEX_TANGENTS) { out.world_tangent = mesh_functions::mesh_tangent_local_to_world(matrix,vertex.tangent,input.instance_index); }
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
