import bevy_pbr::render::forward_io::VertexOutput;
import bevy_pbr::prepass::utils::{prepass_depth,prepass_normal};

struct Control {mode:u32,}
@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> control:Control;

@fragment
fn fragment(input:VertexOutput)->@location(0) vec4<f32> {
    let depth=prepass_depth(input.position,0u);
    if depth<=0.0 {return vec4<f32>(0.0,0.0,0.0,1.0);}
    if control.mode==0u {return vec4<f32>(1.0);}
    return vec4<f32>(prepass_normal(input.position,0u)*0.5+vec3<f32>(0.5),1.0);
}
