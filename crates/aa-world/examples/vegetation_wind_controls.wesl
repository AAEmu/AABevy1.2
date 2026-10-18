import aa_world::vegetation_wind::{lookup,bend_info};
import bevy_pbr::render::forward_io::{VertexOutput,FragmentOutput};
@group(constants::MATERIAL_BIND_GROUP) @binding(50) var<uniform> sample: vec4<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(51) var<uniform> wind: vec4<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(52) var<uniform> clock: vec4<f32>;
@fragment
fn fragment(in:VertexOutput) -> FragmentOutput {
    var value=vec3<f32>(lookup(i32(sample.x)));
    if sample.w != 0.0 { value=bend_info(wind,clock.xyz,clock.w).xyz; }
    return FragmentOutput(vec4<f32>(value*sample.y+sample.z,1.0));
}
