// Illum roads: StandardMaterial lighting, an independently transformed opacity map,
// and the spline's vertex alpha. The source shader is illum.cfx in the client pak.
import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
};

struct OpacityParams {
    mask: vec4<f32>,
    uv_x: vec4<f32>,
    uv_y: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<uniform> opacity: OpacityParams;
@group(constants::MATERIAL_BIND_GROUP) @binding(101) var opacity_map: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(102) var opacity_sampler: sampler;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    if opacity.uv_x.w > 0.0 {
        let uv = vec2<f32>(dot(opacity.uv_x.xyz, vec3<f32>(in.uv, 1.0)), dot(opacity.uv_y.xyz, vec3<f32>(in.uv, 1.0)));
        let mask = saturate(dot(opacity.mask, textureSample(opacity_map, opacity_sampler, uv)));
        pbr_input.material.base_color.a *= mask;
    }
    var out: FragmentOutput;
    if (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        out.color = apply_pbr_lighting(pbr_input);
    } else {
        out.color = pbr_input.material.base_color;
    }
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
