// The `HumanSkin` shader of the client (`shaders/hwscripts/cryfx/humanskin.cfx`), see `skin.rs`: the wrapped,
// coloured skin fall-off of the lights (`SkinfNdotL2`), the shadow rim and the ambient fill, an unnormalised
// Blinn-Phong highlight masked by the gloss map, and the character back lights. The ambient light comes from Bevy's
// PBR lighting run on a white albedo with the specular switched off (no GGX, no environment reflection).

import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    mesh_bindings::mesh,
    mesh_functions::get_world_from_local,
    mesh_view_bindings::{view, lights},
    mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT,
    mesh_types::MESH_FLAGS_SHADOW_RECEIVER_BIT,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
    shadows::fetch_directional_shadow,
};
@if(BINDLESS) import bevy_render::bindless::{bindless_samplers_filtering, bindless_textures_2d};

struct SkinParams {
    // rgb: BSkinColor, w: DiffusionAmount
    bright: vec4<f32>,
    // rgb: MSkinColor, w: SubsurfaceStrenght
    middle: vec4<f32>,
    // rgb: Specular (MatSpecColor), w: Shininess
    spec: vec4<f32>,
    // x: RimPow, y: SSSMapSat, z: gloss map used, w: SSS map bound
    shape: vec4<f32>,
    // x: split gloss (g triples the exponent), y: gloss.r alone masks, z: gloss.b lerps Specular to white
    mode: vec4<f32>,
    // x: gain on all but the back lights, y: one engine light unit (lux / pi), z: factor on the lights' fall-off,
    // w: factor on the ambient
    gain: vec4<f32>,
    // rgb: factor on the base colour for the back lights' albedo
    back_albedo: vec4<f32>,
}

struct BackLightParams {
    dirs: array<vec4<f32>, 2>,
    lights: array<vec4<f32>, 2>,
}

struct SkinData {
    skin: SkinParams,
    back: BackLightParams,
}

@if(BINDLESS) {
struct SkinIndices {
    data: u32,           // 50
    gloss_map: u32,      // 51
    gloss_sampler: u32,  // 52
    sss_map: u32,        // 53
    sss_sampler: u32,    // 54
}

@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<storage> skin_indices: array<SkinIndices>;
@group(constants::MATERIAL_BIND_GROUP) @binding(101) var<storage> skin_array: array<SkinData>;
}
@else {
@group(constants::MATERIAL_BIND_GROUP) @binding(50) var<uniform> skin_uniform: SkinData;
@group(constants::MATERIAL_BIND_GROUP) @binding(51) var skin_gloss_map: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(52) var skin_gloss_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(53) var skin_sss_map: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(54) var skin_sss_sampler: sampler;
}

const PI: f32 = 3.14159265;

fn luminance(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// `SkinfNdotL2` then `pow(x, 1.5) * 1.5`, for the unclamped N.L
fn skin_n_l(n_l: f32, p: SkinParams, dark: vec3<f32>) -> vec3<f32> {
    let translucence = smoothstep(-p.bright.w * dark, vec3<f32>(1.0), vec3<f32>(n_l));
    let lit = mix(p.middle.rgb * (p.middle.w + 1.0), p.bright.rgb, saturate(n_l * n_l));
    let colourise = mix(dark, lit, translucence);
    return pow(max(saturate(translucence) * colourise, vec3<f32>(0.0)), vec3<f32>(1.5)) * 1.5;
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    let slot = mesh[in.instance_index].material_and_lightmap_bind_group_slot & 0xffffu;
@if(BINDLESS)
    let data = skin_array[skin_indices[slot].data];
@else
    let data = skin_uniform;
    let p = data.skin;

    var pbr_input = pbr_input_from_standard_material(in, is_front);
    pbr_input.material.base_color = alpha_discard(
        pbr_input.material.flags,
        pbr_input.material.alpha_cutoff,
        pbr_input.material.base_color
    );

@if(VERTEX_UVS_A)
    let uv = in.uv;
@else
    let uv = vec2<f32>(0.0);
@if(BINDLESS)
    let gloss_map = textureSample(bindless_textures_2d[skin_indices[slot].gloss_map], bindless_samplers_filtering[skin_indices[slot].gloss_sampler], uv);
@else
    let gloss_map = textureSample(skin_gloss_map, skin_gloss_sampler, uv);
@if(BINDLESS)
    let sss_map = textureSample(bindless_textures_2d[skin_indices[slot].sss_map], bindless_samplers_filtering[skin_indices[slot].sss_sampler], uv);
@else
    let sss_map = textureSample(skin_sss_map, skin_sss_sampler, uv);

    var out: FragmentOutput;
    if (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) != 0u {
        out.color = main_pass_post_lighting_processing(pbr_input, pbr_input.material.base_color);
        return out;
    }

    // `cGlossMap`: the Specular map with `GLOSS_MAP`, else the diffuse map's alpha
    let gloss = select(vec4<f32>(pbr_input.material.base_color.a), gloss_map, p.shape.z > 0.5);
    // `cDarkColor = lerp(sss.r, sss.rgb, SSSMapSat)`
    let sss = select(vec3<f32>(0.5), sss_map.rgb, p.shape.w > 0.5);
    let dark = mix(vec3<f32>(saturate(sss.r)), sss, p.shape.y);
    // the fall-off relative to full light: the base colour is the colour of fully lit skin
    let full = skin_n_l(1.0, p, dark);
    let inv_full = 1.0 / max(full, vec3<f32>(1e-3));
    let power = max(p.spec.w * gloss.a * select(1.0, mix(1.0, 3.0, gloss.g), p.mode.x > 0.5), 1.0);

    let n = pbr_input.N;
    let v = pbr_input.V;
    let n_e = dot(n, v);
    if p.mode.w > 0.5 && p.mode.w < 2.5 {
        let d = select(n, normalize(pbr_input.world_normal), p.mode.w > 1.5);
        out.color = vec4<f32>(d * 0.5 + 0.5, 1.0);
        return out;
    }

    // ---- the ambient and environment diffuse light from Bevy: white albedo, no specular at all ----
    var lit = pbr_input;
    lit.material.base_color = vec4<f32>(1.0, 1.0, 1.0, pbr_input.material.base_color.a);
    lit.material.reflectance = vec3<f32>(0.0);
    lit.material.metallic = 0.0;
    lit.material.perceptual_roughness = 1.0;
    lit.material.emissive = vec4<f32>(0.0);
    lit.material.diffuse_transmission = 0.0;
    lit.specular_occlusion = 0.0;
    let irradiance = apply_pbr_lighting(lit).rgb;

    let exposure = view.exposure;
    let unit = p.gain.y * exposure;
    // ---- the lights (`frag_custom_per_light`) ----
    var direct = vec3<f32>(0.0);
    var lambert = vec3<f32>(0.0);
    var specular = vec3<f32>(0.0);
    let view_z = dot(vec4<f32>(
        view.view_from_world[0].z,
        view.view_from_world[1].z,
        view.view_from_world[2].z,
        view.view_from_world[3].z
    ), pbr_input.world_position);
    // Y is up here, Z in the engine
    let up = n.y;
    for (var i: u32 = 0u; i < lights.n_directional_lights; i = i + 1u) {
        let light = &lights.directional_lights[i];
        let l = (*light).direction_to_light;
        var shadow = 1.0;
        let casts = ((*light).flags & DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) != 0u;
        // the skin model is the sun's (the shadow casting light): the other directional lights of the login stage
        // stand for its sky light and stay in Bevy's lighting, with the ambient
        if !casts {
            continue;
        }
        if ((pbr_input.flags & MESH_FLAGS_SHADOW_RECEIVER_BIT) != 0u && casts) {
            shadow = fetch_directional_shadow(i, pbr_input.world_position, pbr_input.world_normal, view_z, pbr_input.frag_coord.xy);
        }
        // the engine's light colours are radiance-like (`albedo * colour * N.L`), Bevy's lux: / PI
        let e = (*light).color.rgb / PI;
        let n_l = dot(n, l);
        let e_l = dot(l, v);
        var diffuse = skin_n_l(n_l, p, dark) * inv_full;
        diffuse = mix(diffuse, diffuse * 0.8, saturate(e_l));
        diffuse *= luminance(e) * 0.9 + e * 0.1;
        let soft = saturate(shadow * saturate(max(n_l * 0.7 + 0.3, e_l)) * 0.7 + shadow * 0.3);
        diffuse *= soft * 0.8 + 0.2;
        direct += diffuse * p.gain.z;
        // the shadow area's rim, colourless engine units
        direct += vec3<f32>((1.0 - abs(n_e)) * (1.0 - soft) * p.shape.x * (up * 0.5 + 0.5) * (1.0 - up)) * inv_full * (unit / exposure);
        lambert += e * (shadow * saturate(n_l));
        // Blinn-Phong, unnormalised; no N.L in the engine, a light behind the surface is faded out here
        let h = normalize(l + v);
        specular += pow(saturate(dot(n, h)), power) * e * (shadow * saturate(n_l * 4.0));
    }

    // ---- the character back lights (`X2BackLight`): plain N.L and a Phong highlight, unshadowed ----
    var back = vec3<f32>(0.0);
    var back_spec = vec3<f32>(0.0);
    let world_from_local = get_world_from_local(in.instance_index);
    let r = reflect(-v, n);
    for (var i = 0; i < 2; i++) {
        let light = data.back.lights[i];
        if max(light.r, max(light.g, light.b)) <= 0.0 {
            continue;
        }
        let d = data.back.dirs[i];
        let l = normalize((world_from_local * vec4<f32>(d.xyz, 0.0)).xyz);
        let e = light.rgb * (d.w / PI);
        back += e * saturate(dot(n, l));
        back_spec += e * (light.w * pow(saturate(dot(r, l)), power));
    }

    // ---- `frag_custom_end` ----
    // the ambient (and the stage's fill lights): Bevy's lighting minus its Lambert part of the sun
    // `bHemisphereLighting`: the ambient times `N.up * 0.25 + 0.75` (half under the chin and nose)
    let ambient = max(irradiance - lambert * exposure, vec3<f32>(0.0)) * (p.gain.w * (up * 0.25 + 0.75));
    // `reColorInShadow * 0.3`, added to the light
    let n_e_s = saturate(n_e);
    let fill = mix(dark * 0.7, mix(p.middle.rgb, (p.bright.rgb + p.middle.rgb) * 0.6, pow(n_e_s, 1.3)), n_e_s) * 0.3;
    let received = ambient + direct * exposure + fill * inv_full * unit;
    // the highlight's mask and colour
    let spec_type = select(gloss.rgb, vec3<f32>(gloss.r), p.mode.y > 0.5);
    let spec_colour = select(p.spec.rgb, mix(p.spec.rgb, vec3<f32>(1.0), gloss.b), p.mode.z > 0.5);
    let albedo = pbr_input.material.base_color.rgb;
    let colour = p.gain.x * (received * albedo + specular * exposure * spec_type * spec_colour)
        + (back * albedo * p.back_albedo.rgb + back_spec * spec_type * spec_colour) * exposure;
    out.color = main_pass_post_lighting_processing(pbr_input, vec4<f32>(colour, pbr_input.material.base_color.a));
    return out;
}
