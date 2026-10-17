// The character back lights, see `backlight.rs`: the PBR lighting of the StandardMaterial, then the two lights of the
// time of day's `Char BackLight` as unshadowed directional lights (the engine's `X2BackLight`: diffuse saturate(N.L)
// and a highlight, added to the light the diffuse map is multiplied with).

import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    mesh_bindings::mesh,
    mesh_functions::get_world_from_local,
    mesh_view_bindings::view,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, calculate_F0, main_pass_post_lighting_processing},
    pbr_types::{PbrInput, STANDARD_MATERIAL_FLAGS_UNLIT_BIT},
};

struct BackLightParams {
    // xyz: direction towards the light in the character's (cry) space, w: fall-off
    dirs: array<vec4<f32>, 2>,
    // rgb: illuminance in lux, w: specular multiplier
    lights: array<vec4<f32>, 2>,
}

struct BackData {
    params: BackLightParams,
    // rgb: factor on the base colour to get the albedo of the back lights (skins, see `skin_tag`)
    albedo: vec4<f32>,
}

// The material extension is bindless like StandardMaterial (see `CharBackExt`): an index table (binding 100) and one
// array with the data of all materials (binding 101); without bindless a plain uniform (binding 50).
@if(BINDLESS) {
struct BackIndices {
    data: u32,  // 50
}

@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<storage> back_indices: array<BackIndices>;
@group(constants::MATERIAL_BIND_GROUP) @binding(101) var<storage> back_array: array<BackData>;
}
@else {
@group(constants::MATERIAL_BIND_GROUP) @binding(50) var<uniform> back_uniform: BackData;
}

const PI: f32 = 3.14159265;

// What the two lights add, in the units `apply_pbr_lighting` returns (it multiplies the lux by the exposure too).
fn back_light(pbr: PbrInput, instance_index: u32) -> vec3<f32> {
@if(BINDLESS)
    let data = back_array[back_indices[mesh[instance_index].material_and_lightmap_bind_group_slot & 0xffffu].data];
@else
    let data = back_uniform;
    let back = data.params;
    let albedo_scale = data.albedo;
    let n = pbr.N;
    let v = pbr.V;
    let base = pbr.material.base_color.rgb * albedo_scale.rgb;
    let metallic = pbr.material.metallic;
    let diffuse_colour = base * (1.0 - metallic);
    let f0 = calculate_F0(base, metallic, pbr.material.reflectance);
    // Blinn-Phong exponent of a GGX lobe of this roughness
    let a = max(pbr.material.perceptual_roughness * pbr.material.perceptual_roughness, 0.045);
    let power = max(2.0 / (a * a) - 2.0, 1.0);
    // the character's own space: the mesh transform (the root of a character is its orientation)
    let world_from_local = get_world_from_local(instance_index);

    var sum = vec3<f32>(0.0);
    for (var i = 0; i < 2; i++) {
        let light = back.lights[i];
        let d = back.dirs[i];
        let l = normalize((world_from_local * vec4<f32>(d.xyz, 0.0)).xyz);
        let n_l = saturate(dot(n, l));
        let n_h = saturate(dot(n, normalize(l + v)));
        let diffuse = diffuse_colour * (n_l / PI);
        let specular = f0 * (((power + 2.0) / (8.0 * PI)) * pow(n_h, power) * n_l * light.w);
        sum += light.rgb * (diffuse + specular) * d.w;
    }
    return sum * view.exposure;
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    pbr_input.material.base_color = alpha_discard(
        pbr_input.material.flags,
        pbr_input.material.alpha_cutoff,
        pbr_input.material.base_color
    );

    var out: FragmentOutput;
    if (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        out.color = apply_pbr_lighting(pbr_input);
        out.color = vec4<f32>(out.color.rgb + back_light(pbr_input, in.instance_index), out.color.a);
    } else {
        out.color = pbr_input.material.base_color;
    }
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
