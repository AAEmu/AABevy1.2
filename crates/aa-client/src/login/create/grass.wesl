// The grass of the step 3 field, see `grass.rs`: the PBR lighting of the StandardMaterial, then a power curve on the lit
// colour (before the fog and the tone mapping).

import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    mesh_bindings::mesh,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
};

struct GrassData {
    // x: exponent, y: reference radiance, z: linear part, w: saturation
    curve: vec4<f32>,
}

@if(BINDLESS) {
struct GrassIndices {
    data: u32,  // 50
}

@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<storage> grass_indices: array<GrassIndices>;
@group(constants::MATERIAL_BIND_GROUP) @binding(101) var<storage> grass_array: array<GrassData>;
}
@else {
@group(constants::MATERIAL_BIND_GROUP) @binding(50) var<uniform> grass_uniform: GrassData;
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    pbr_input.material.base_color = alpha_discard(
        pbr_input.material.flags,
        pbr_input.material.alpha_cutoff,
        pbr_input.material.base_color
    );

@if(BINDLESS)
    let data = grass_array[grass_indices[mesh[in.instance_index].material_and_lightmap_bind_group_slot & 0xffffu].data];
@else
    let data = grass_uniform;

    var out: FragmentOutput;
    if (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        out.color = apply_pbr_lighting(pbr_input);
        let x = max(out.color.rgb, vec3<f32>(0.0));
        let luma = dot(x, vec3<f32>(0.2126, 0.7152, 0.0722));
        let c = mix(vec3<f32>(luma), x, data.curve.w);
        out.color = vec4<f32>(x * (data.curve.z + pow(c / data.curve.y, vec3<f32>(data.curve.x - 1.0))), out.color.a);
    } else {
        out.color = pbr_input.material.base_color;
    }
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
