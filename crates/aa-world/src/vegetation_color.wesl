// Native vertex-alpha AO / COLOR_VARIATION: final lit RGB, before fog/post. Alpha and material alpha test unchanged.
import aa_world::vegetation_bindings::parameters;
import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    mesh_functions::get_tag,
    mesh_bindings::mesh,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
};
@if(VISIBILITY_RANGE_DITHER)
import bevy_pbr::render::pbr_functions::visibility_range_dither;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    // Native far-plane dissolve / sprite hand-over (objects/dissolve.rs), as Bevy's own pbr fragment does.
    @if(VISIBILITY_RANGE_DITHER)
    visibility_range_dither(in.position, in.visibility_range_dither);
    var pbr = pbr_input_from_standard_material(in, is_front);
    pbr.material.base_color = alpha_discard(pbr.material.flags, pbr.material.alpha_cutoff, pbr.material.base_color);
    var out: FragmentOutput;
    if (pbr.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        out.color = apply_pbr_lighting(pbr);
    } else {
        out.color = pbr.material.base_color;
    }
    let brightness = parameters(in.instance_index).brightness.x;
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
