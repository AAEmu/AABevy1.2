import aa_world::vegetation_bindings::parameters;
import aa_world::vegetation_wind::bend;
import bevy_render::globals::Globals;
import bevy_pbr::render::{mesh_functions, skinning, view_transformations::position_world_to_clip};
@if(PREPASS_PIPELINE)
import bevy_pbr::prepass::io::{Vertex,VertexOutput,decompress_vertex};
@else
import bevy_pbr::render::forward_io::{Vertex,VertexOutput,decompress_vertex};
@if(PREPASS_PIPELINE)
@group(0) @binding(1) var<uniform> clock: Globals;
@else
@group(0) @binding(11) var<uniform> clock: Globals;

@vertex
fn vertex(input: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let v = decompress_vertex(input,input.instance_index);
    let mesh_matrix = mesh_functions::get_world_from_local(input.instance_index);
    @if(SKINNED)
    let model = skinning::skin_model(v.joint_indices,v.joint_weights,input.instance_index);
    @else
    let model = mesh_matrix;
    let params = parameters(input.instance_index);
    let seconds = select(params.clock.x + clock.time,params.clock.z,params.clock.w != 0.0);
    let flags=u32(params.clock.y);
    // Mirror meshes carry R,G,B and the quantised normal xy in COLOR (never shaded, see below).
    var detail = params.detail;
    var vcolor = vec4<f32>(0.0);
    @if(VERTEX_COLORS) {
        vcolor = v.color;
    } @else {
        detail = vec4<f32>(0.0);
    }
    let local = bend(v.position,model,params.wind,seconds,(flags & 1u) != 0u,(flags & 2u) != 0u,detail,vcolor);
    out.world_position = mesh_functions::mesh_position_local_to_world(model,vec4<f32>(local,1.0));
    out.position = position_world_to_clip(out.world_position.xyz);
    @if(UNCLIPPED_DEPTH_ORTHO_EMULATION) {
        out.unclipped_depth=out.position.z;
        out.position.z=min(out.position.z,1.0);
    }
    @if(VERTEX_UVS_A)
    out.uv=v.uv;
    @if(VERTEX_UVS_B)
    out.uv_b=v.uv_b;
    @if(!PREPASS_PIPELINE || NORMAL_PREPASS_OR_DEFERRED_PREPASS) {
        @if(VERTEX_NORMALS && SKINNED)
        out.world_normal=skinning::skin_normals(model,v.normal);
        @elif(VERTEX_NORMALS)
        out.world_normal=mesh_functions::mesh_normal_local_to_world(v.normal,input.instance_index);
        @if(VERTEX_TANGENTS)
        out.world_tangent=mesh_functions::mesh_tangent_local_to_world(model,v.tangent,input.instance_index);
    }
    @if(VERTEX_COLORS)
    out.color=vec4<f32>(1.0);
    @if(VERTEX_OUTPUT_INSTANCE_INDEX)
    out.instance_index=input.instance_index;
    @if(VISIBILITY_RANGE_DITHER)
    out.visibility_range_dither=mesh_functions::get_visibility_range_dither_level(input.instance_index,mesh_matrix[3]);
    @if(MOTION_VECTOR_PREPASS) {
        let previous=mesh_functions::get_previous_world_from_local(input.instance_index);
        let prev_local=bend(v.position,previous,params.wind,seconds-clock.delta_time,(flags & 1u) != 0u,(flags & 2u) != 0u,detail,vcolor);
        out.previous_world_position=mesh_functions::mesh_position_local_to_world(previous,vec4<f32>(prev_local,1.0));
    }
    return out;
}
