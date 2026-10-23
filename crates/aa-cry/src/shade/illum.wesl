// The `Illum` shader of the client (`shaders/hwscripts/cryfx/illum.cfx`), see `illum.rs`: the sun's Lambert light and
// unnormalised Phong highlight masked by the gloss map, the sun-weighted hemisphere ambient, the cube map reflection
// times the light received, `max(highlight, reflection)`, and the character back lights. The ambient light comes from
// Bevy's PBR lighting run on a white albedo with the specular switched off.

import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    mesh_bindings::mesh,
    mesh_functions::get_world_from_local,
    mesh_view_bindings::{view, lights},
    mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT,
    mesh_types::MESH_FLAGS_SHADOW_RECEIVER_BIT,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_bindings,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
    shadows::fetch_directional_shadow,
};
@if(BINDLESS) import bevy_render::bindless::{bindless_samplers_filtering, bindless_textures_2d, bindless_textures_cube};
@if(BINDLESS) import bevy_pbr::render::pbr_bindings::material_indices;
@if(VISIBILITY_RANGE_DITHER) import bevy_pbr::render::pbr_functions::visibility_range_dither;

struct IllumParams {
    // rgb: Specular (x SpecularModifierInHDR), w: Shininess
    spec: vec4<f32>,
    // x: ReflectAmount (0: no reflection), y: FresnelBias, z: FresnelScale
    env: vec4<f32>,
    // x: split gloss, y: gloss map used, z: gloss.b scales the exponent, w: cube map bound
    mode: vec4<f32>,
    // x: gain on all but the back lights, y: factor on the ambient, z: debug
    gain: vec4<f32>,
    // x: vertex RGB squared, y: native Full composition, z: character lighting
    vertex: vec4<f32>,
    // MatEmissiveColor in the ambient accumulator
    emission: vec4<f32>,
}

struct BackLightParams {
    dirs: array<vec4<f32>, 2>,
    lights: array<vec4<f32>, 2>,
}

struct IllumData {
    illum: IllumParams,
    back: BackLightParams,
}

@if(BINDLESS) {
struct IllumIndices {
    data: u32,           // 50
    gloss_map: u32,      // 51
    gloss_sampler: u32,  // 52
    env_map: u32,        // 53
    env_sampler: u32,    // 54
}

@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<storage> illum_indices: array<IllumIndices>;
@group(constants::MATERIAL_BIND_GROUP) @binding(101) var<storage> illum_array: array<IllumData>;
}
@else {
@group(constants::MATERIAL_BIND_GROUP) @binding(50) var<uniform> illum_uniform: IllumData;
@group(constants::MATERIAL_BIND_GROUP) @binding(51) var illum_gloss_map: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(52) var illum_gloss_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(53) var illum_env_map: texture_cube<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(54) var illum_env_sampler: sampler;
}

const PI: f32 = 3.14159265;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
@if(VISIBILITY_RANGE_DITHER)
    visibility_range_dither(in.position, in.visibility_range_dither);
    let slot = mesh[in.instance_index].material_and_lightmap_bind_group_slot & 0xffffu;
@if(BINDLESS)
    let data = illum_array[illum_indices[slot].data];
@else
    let data = illum_uniform;
    let p = data.illum;

    // Bevy normally multiplies the albedo by vertex colour before lighting. Illum applies RGB squared
    // to the composed lighting, and VERTCOLORS does not touch alpha. Clear that automatic multiply.
    var material_in = in;
@if(VERTEX_COLORS)
    material_in.color = vec4<f32>(1.0);
    var vertex_factor = vec3<f32>(1.0);
@if(VERTEX_COLORS)
    vertex_factor = select(vec3<f32>(1.0), in.color.rgb * in.color.rgb, p.vertex.x > 0.5);
    var pbr_input = pbr_input_from_standard_material(material_in, is_front);
    pbr_input.material.base_color = alpha_discard(
        pbr_input.material.flags,
        pbr_input.material.alpha_cutoff,
        pbr_input.material.base_color
    );

@if(VERTEX_UVS_A)
    var uv = in.uv;
@else
    var uv = vec2<f32>(0.0);
@if(BINDLESS)
    uv = (pbr_bindings::material_array[material_indices[slot].material].uv_transform * vec3<f32>(uv, 1.0)).xy;
@else
    uv = (pbr_bindings::material.uv_transform * vec3<f32>(uv, 1.0)).xy;
@if(BINDLESS)
    let gloss_map = textureSample(bindless_textures_2d[illum_indices[slot].gloss_map], bindless_samplers_filtering[illum_indices[slot].gloss_sampler], uv);
@else
    let gloss_map = textureSample(illum_gloss_map, illum_gloss_sampler, uv);

    var out: FragmentOutput;
    let unlit = (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) != 0u;
    if p.vertex.y < 0.5 || unlit {
        var colour = pbr_input.material.base_color;
        if !unlit {
            colour = apply_pbr_lighting(pbr_input);
        }
        out.color = main_pass_post_lighting_processing(pbr_input, vec4<f32>(colour.rgb * vertex_factor, colour.a));
        return out;
    }

    // `cGlossMap`: white without `GLOSS_MAP`
    let gloss = select(vec4<f32>(1.0), gloss_map, p.mode.y > 0.5);
    let power = max(p.spec.w * select(1.0, gloss.b, p.mode.z > 0.5), 0.0001);

    let n = pbr_input.N;
    let v = pbr_input.V;
    let n_e = saturate(abs(dot(n, v)));
    // `vReflVec`
    let r = reflect(-v, n);

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
    // ---- the sun (`frag_custom_per_light`): the shadow casting light; the login stage's other lights stand for the
    // sky light and stay in Bevy's lighting, with the ambient ----
    var lambert = vec3<f32>(0.0);
    var specular = vec3<f32>(0.0);
    var sun_half = 0.0;
    let view_z = dot(vec4<f32>(
        view.view_from_world[0].z,
        view.view_from_world[1].z,
        view.view_from_world[2].z,
        view.view_from_world[3].z
    ), pbr_input.world_position);
    for (var i: u32 = 0u; i < lights.n_directional_lights; i = i + 1u) {
        let light = &lights.directional_lights[i];
        if ((*light).flags & DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) == 0u {
            continue;
        }
        let l = (*light).direction_to_light;
        var shadow = 1.0;
        if (pbr_input.flags & MESH_FLAGS_SHADOW_RECEIVER_BIT) != 0u {
            shadow = fetch_directional_shadow(i, pbr_input.world_position, pbr_input.world_normal, view_z, pbr_input.frag_coord.xy);
        }
        // the engine's light colours are radiance-like (`albedo * colour * N.L`), Bevy's lux: / PI
        let e = (*light).color.rgb / PI;
        let n_l = dot(n, l);
        sun_half = n_l;
        lambert += e * (shadow * saturate(n_l));
        // `Phong` has no extra N.L multiplier in the original shader.
        specular += pow(saturate(dot(r, l)), power) * e * (shadow * 0.8 + 0.2);
    }

    // ---- the character back lights (`X2BackLight`): plain N.L and a Phong highlight, unshadowed ----
    var back = vec3<f32>(0.0);
    var back_spec = vec3<f32>(0.0);
    let world_from_local = get_world_from_local(in.instance_index);
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
    // the ambient (and the stage's fill lights): Bevy's lighting minus its Lambert part of the sun, in the hemisphere
    // (`N.up * 0.25 + 0.75`, Y is up here)
    let ambient = max(irradiance - lambert * exposure, vec3<f32>(0.0)) * (p.gain.y * (n.y * 0.25 + 0.75)) + p.emission.rgb * exposure;
    // weighted by the sun: `amb * 0.6 + (saturate(N.L * 0.5 + 0.5) + 0.3) * amb * 0.4`
    let ambient_pass = ambient * (0.6 + 0.4 * (saturate(sun_half * 0.5 + 0.5) + 0.3));
    let sun = lambert * exposure;
    let gain = p.gain.x;
    let albedo = pbr_input.material.base_color.rgb;
    let diffuse = (gain * (ambient_pass + sun) + back * exposure) * albedo;

    let spec_type = select(gloss.rgb, vec3<f32>(gloss.r), p.mode.x > 0.5);
    let gloss_type = select(gloss.rgb, vec3<f32>(gloss.g), p.mode.x > 0.5);
    let highlight = (gain * specular + back_spec) * exposure * spec_type * p.spec.rgb;

    // the reflection: the cube map (CryEngine axes: Z up) x `ReflectAmount` x Fresnel x the light received x gloss
    // (sampled outside the branch: derivatives need uniform control flow)
    let r_cry = vec3<f32>(r.x, -r.z, r.y);
@if(BINDLESS)
    let env = textureSample(bindless_textures_cube[illum_indices[slot].env_map], bindless_samplers_filtering[illum_indices[slot].env_sampler], r_cry).rgb;
@else
    let env = textureSample(illum_env_map, illum_env_sampler, r_cry).rgb;
    var reflection = vec3<f32>(0.0);
    if p.env.x > 0.0 && p.mode.w > 0.5 {
        let fresnel = p.env.y + pow(1.0 - n_e, 5.0) * p.env.z;
        reflection = env * (p.env.x * fresnel) * (gain * (ambient + sun) + back * exposure) * gloss_type;
    }

    var colour = diffuse + max(highlight, reflection);
    if p.gain.z > 0.5 && p.gain.z < 1.5 {
        colour = reflection;
    } else if p.gain.z > 1.5 && p.gain.z < 2.5 {
        colour = highlight;
    }
    out.color = main_pass_post_lighting_processing(pbr_input, vec4<f32>(colour * vertex_factor, pbr_input.material.base_color.a));
    return out;
}
