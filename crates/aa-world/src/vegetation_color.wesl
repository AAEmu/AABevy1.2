// Native COLOR_VARIATION: final lit RGB, before fog/post. Alpha and material alpha test unchanged.
import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    mesh_functions::get_tag,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
};

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
    let packed = get_tag(in.instance_index);
    let rgb = vec3<f32>(f32(packed & 63u), f32((packed >> 6u) & 63u), f32((packed >> 12u) & 63u)) / 63.0;
    out.color = vec4<f32>(out.color.rgb * (vec3<f32>(0.8) + rgb * 0.4), out.color.a);
    return FragmentOutput(main_pass_post_lighting_processing(pbr, out.color));
}
