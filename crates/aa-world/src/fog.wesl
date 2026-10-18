import aa_world::fog_math::opacity;
import bevy_pbr::render::{mesh_functions,mesh_view_bindings::view,view_transformations::{position_world_to_clip,position_ndc_to_world}};
@if(DEPTH_PREPASS) import bevy_pbr::prepass::utils::prepass_depth;

struct FogParams {local_from_world:mat4x4<f32>,center:vec4<f32>,color:vec4<f32>,base:vec4<f32>,gradient:vec4<f32>,}
@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> fog:FogParams;
struct Input {@location(0) position:vec3<f32>,@builtin(instance_index) instance_index:u32,}
struct Output {@builtin(position) clip:vec4<f32>,@location(0) world:vec3<f32>,}
@vertex
fn vertex(input:Input)->Output{
    let matrix=mesh_functions::get_world_from_local(input.instance_index);
    let world=mesh_functions::mesh_position_local_to_world(matrix,vec4<f32>(input.position,1.0));
    let clip=position_world_to_clip(world.xyz);
    var out:Output;
    // Analytic depth handles clipping; a far-depth proxy also works when the camera is inside.
    out.clip=vec4<f32>(clip.xy,0.0,clip.w);out.world=world.xyz;return out;
}
@fragment
fn fragment(input:Output)->@location(0) vec4<f32>{
    @if(DEPTH_PREPASS) {
        // The game world uses perspective views; do not reinterpret an orthographic ray origin.
        if view.clip_from_view[3][3] != 0.0 { discard; }
        let eye=view.world_position;
        let direction=normalize(input.world-eye);
        let depth=prepass_depth(input.clip,0u);
        var scene_distance=10000000.0;
        if depth>0.0 {
            let uv=(input.clip.xy-view.viewport.xy)/view.viewport.zw;
            let ndc=vec3<f32>(uv*vec2<f32>(2.0,-2.0)+vec2<f32>(-1.0,1.0),depth);
            let scene=position_ndc_to_world(ndc);
            scene_distance=max(0.0,dot(scene-eye,direction));
        }
        let alpha=opacity(fog.local_from_world,fog.center.xyz,fog.color.w,fog.base,fog.gradient,eye,direction,scene_distance);
        return vec4<f32>(fog.color.rgb*view.exposure,alpha);
    }
    @else { discard; return vec4<f32>(0.0); }
}
