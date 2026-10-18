// Native vertex-alpha AO / COLOR_VARIATION: final lit RGB, before fog/post. Alpha and material alpha test unchanged.
import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    mesh_functions::get_tag,
    mesh_bindings::mesh,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
};

struct VegetationUniform { brightness: vec4<f32> }
@if(BINDLESS) {
    struct VegetationBindings { material: u32 }
    @group(constants::MATERIAL_BIND_GROUP) @binding(100) var<storage> vegetation_indices: array<VegetationBindings>;
    @group(constants::MATERIAL_BIND_GROUP) @binding(101) var<storage> vegetation_array: array<VegetationUniform>;
}
@else {
    @group(constants::MATERIAL_BIND_GROUP) @binding(50) var<uniform> vegetation: VegetationUniform;
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr = pbr_input_from_standard_material(in, is_front);
    pbr.material.base_color = alpha_discard(pbr.material.flags, pbr.material.alpha_cutoff, pbr.material.base_color);
    var out: FragmentOutput;
    if (pbr.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        out.color = apply_pbr_lighting(pbr);
    } else {
        out.color = pbr.material.base_color;
    }
    @if(BINDLESS)
    let slot = mesh[in.instance_index].material_and_lightmap_bind_group_slot & 0xffffu;
    @if(BINDLESS)
    let brightness = vegetation_array[vegetation_indices[slot].material].brightness.x;
    @else
    let brightness = vegetation.brightness.x;
    // Native non-terrain branch: AmbientObjectCol.x multiplies vertex AO before variation/fog.
    out.color = vec4<f32>(out.color.rgb * brightness, out.color.a);
    let packed = get_tag(in.instance_index);
    @if(VERTEX_UVS_B) {
        // Raw linear alpha/255 in the otherwise unused UV1; never material opacity/albedo.
        if ((packed & 524288u) == 0u) {
            out.color = vec4<f32>(out.color.rgb * in.uv_b.x, out.color.a);
        }
    }
    if ((packed & 262144u) == 0u) {
        let rgb = vec3<f32>(f32(packed & 63u), f32((packed >> 6u) & 63u), f32((packed >> 12u) & 63u)) / 63.0;
        out.color = vec4<f32>(out.color.rgb * (vec3<f32>(0.8) + rgb * 0.4), out.color.a);
    }
    return FragmentOutput(main_pass_post_lighting_processing(pbr, out.color));
}
