// The `Hair` shader of the client (`shaders/hwscripts/cryfx/hair.cfx`), see `hair.rs`: Kajiya-Kay strand
// highlights in two lobes along the shifted binormal, wrapped diffuse, a back rim, the ambient strand highlight and
// the character back lights. The diffuse and ambient light come from Bevy's PBR lighting run on a white albedo with
// the specular switched off (no GGX, no environment reflection: the hair path of the engine has none).

import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    mesh_bindings::mesh,
    mesh_functions::get_world_from_local,
    mesh_view_bindings::{view, lights},
    mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT,
    mesh_types::MESH_FLAGS_SHADOW_RECEIVER_BIT,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing, winding_corrected_front_facing},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
    shadows::fetch_directional_shadow,
};
@if(BINDLESS) import bevy_render::bindless::{bindless_samplers_filtering, bindless_textures_2d};

struct HairParams {
    // rgb: PrimarySpecColor, w: PrimarySpecPow
    primary: vec4<f32>,
    // rgb: SecondarySpecColor, w: SecondarySpecPow
    secondary: vec4<f32>,
    // x: PrimarySpecShift, y: SecondarySpecShift, z: BumpScale (shift noise amplitude), w: DetailTilling
    shape: vec4<f32>,
    // x: RimMultiplier, y: DiffuseWrap, z: AmbientSpecMult, w: 1 when the Specular map is bound
    light: vec4<f32>,
    // rgb: Specular (MatSpecColor) times a gain, w: factor on the diffuse colour
    spec: vec4<f32>,
    // x: 1 for the blended fringe pass, y: the cut of the opaque pass on the doubled alpha, z: the world's gain (0 = 1)
    mode: vec4<f32>,
}

struct BackLightParams {
    dirs: array<vec4<f32>, 2>,
    lights: array<vec4<f32>, 2>,
}

struct HairData {
    hair: HairParams,
    back: BackLightParams,
}

@if(BINDLESS) {
struct HairIndices {
    data: u32,          // 50
    spec_map: u32,      // 51
    spec_sampler: u32,  // 52
}

@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<storage> hair_indices: array<HairIndices>;
@group(constants::MATERIAL_BIND_GROUP) @binding(101) var<storage> hair_array: array<HairData>;
}
@else {
@group(constants::MATERIAL_BIND_GROUP) @binding(50) var<uniform> hair_uniform: HairData;
@group(constants::MATERIAL_BIND_GROUP) @binding(51) var hair_spec_map: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(52) var hair_spec_sampler: sampler;
}

const PI: f32 = 3.14159265;

// shadelib.cfi `KajiyaKayAnisotropic`: T is not normalised (the shifted binormal)
fn kajiya_kay(t: vec3<f32>, h: vec3<f32>, power: f32) -> f32 {
    let t_h = dot(t, h);
    return pow(sqrt(max(1.0 - t_h * t_h, 0.01)), power);
}

struct Strand {
    // shifted tangents of the two lobes
    t1: vec3<f32>,
    t2: vec3<f32>,
    n: vec3<f32>,
    v: vec3<f32>,
    rim: f32,
}

// `frag_custom_per_light` for one light of illuminance `e` (lux, no exposure) towards `l`: the wrapped diffuse
// (plus the back rim) in `diffuse`, the two strand highlights in `specular` (before the gloss map and `Specular`).
struct LightTerms {
    diffuse: vec3<f32>,
    specular: vec3<f32>,
}

fn hair_light(s: Strand, p: HairParams, l: vec3<f32>, e: vec3<f32>, spec_mult: f32, occlusion: f32) -> LightTerms {
    let h = normalize(s.v + l);
    let e_l = saturate(dot(l, s.v));
    let wrap = p.light.y;
    let n_l = saturate(dot(s.n, l) * wrap + (1.0 - wrap));
    // the engine's light colours are radiance-like (`albedo * colour * N.L`), Bevy's lux: / PI
    let c = e * (occlusion / PI);
    var out: LightTerms;
    out.diffuse = c * (n_l + saturate(1.0 - e_l) * s.rim);
    let spec = kajiya_kay(s.t1, h, p.primary.w) * p.primary.rgb + kajiya_kay(s.t2, h, p.secondary.w) * p.secondary.rgb;
    out.specular = spec * c * n_l * spec_mult;
    return out;
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    let slot = mesh[in.instance_index].material_and_lightmap_bind_group_slot & 0xffffu;
@if(BINDLESS)
    let data = hair_array[hair_indices[slot].data];
@else
    let data = hair_uniform;
    let p = data.hair;

    var pbr_input = pbr_input_from_standard_material(in, is_front);
    pbr_input.material.base_color = alpha_discard(
        pbr_input.material.flags,
        pbr_input.material.alpha_cutoff,
        pbr_input.material.base_color
    );

    // the Specular map: rgb = gloss mask, a = strand shift noise (sampled at `uv * DetailTilling`)
@if(VERTEX_UVS_A)
    let uv = in.uv;
@else
    let uv = vec2<f32>(0.0);
@if(BINDLESS)
    let gloss = textureSample(bindless_textures_2d[hair_indices[slot].spec_map], bindless_samplers_filtering[hair_indices[slot].spec_sampler], uv);
@else
    let gloss = textureSample(hair_spec_map, hair_spec_sampler, uv);
@if(BINDLESS)
    let noise = textureSample(bindless_textures_2d[hair_indices[slot].spec_map], bindless_samplers_filtering[hair_indices[slot].spec_sampler], uv * p.shape.w).a;
@else
    let noise = textureSample(hair_spec_map, hair_spec_sampler, uv * p.shape.w).a;
    // the blended fringe (the engine's front / back passes): only the strand ends the opaque mask cuts away,
    // `clip(alpha - 0.05)`, blended with `saturate(alpha * 2)` (the base colour's alpha is already doubled)
    let fringe = p.mode.x > 0.5;
    let alpha2 = pbr_input.material.base_color.a;
    if fringe && (alpha2 >= p.mode.y || alpha2 < 0.1) {
        discard;
    }
    let out_alpha = select(pbr_input.material.base_color.a, saturate(alpha2), fringe);
    let has_map = p.light.w > 0.5;
    let gloss_rgb = select(vec3<f32>(0.5), gloss.rgb, has_map);
    let shift_noise = select(0.5, noise, has_map);

    var out: FragmentOutput;
    if (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) != 0u {
        out.color = main_pass_post_lighting_processing(pbr_input, pbr_input.material.base_color);
        return out;
    }

    // ---- the strand frame (hair.cfx `HairVS` / `frag_custom_begin`) ----
    let n = pbr_input.N;
    let v = pbr_input.V;
    let front = winding_corrected_front_facing(pbr_input.flags, is_front);
    // the normal as the mesh has it (Bevy turns it to the viewer on back faces)
    let n_mesh = select(-n, n, front);
@if(VERTEX_TANGENTS)
    let t_world = in.world_tangent;
@else
    let t_world = vec4<f32>(normalize(cross(in.world_normal, vec3<f32>(0.0, 1.0, 0.0)) + vec3<f32>(1e-4, 0.0, 0.0)), 1.0);
    // the strand runs along the binormal (the texture's V axis): Bevy's bitangent
    let b = normalize(cross(in.world_normal, t_world.xyz) * t_world.w);
    // `vTangent.w`: handedness times `dot(eye, N) * 0.8 + 0.2`; with the normal turned to the eye it shifts along
    // `N_mesh * w * |eye factor|`
    let eye = dot(v, normalize(in.world_normal)) * 0.8 + 0.2;
    let shift_dir = n_mesh * (t_world.w * abs(eye));
    // the noise remap of the source, applied twice: `((2a - 1) * 2 - 1) * BumpScale`
    let s0 = ((shift_noise * 2.0 - 1.0) * 2.0 - 1.0) * p.shape.z;
    var s: Strand;
    s.t1 = b + shift_dir * (s0 + p.shape.x);
    s.t2 = b + shift_dir * (s0 + p.shape.y);
    s.n = n;
    s.v = v;
    // `GetFresnel(N.E, 0, 2)^1.5 * RimMultiplier`
    let n_e = saturate(dot(n, v));
    s.rim = pow(pow(1.0 - n_e, 2.0), 1.5) * p.light.x;

    // ---- diffuse + ambient light from Bevy: white albedo, no specular at all ----
    let albedo = pbr_input.material.base_color.rgb * p.spec.w;
    var lit = pbr_input;
    lit.material.base_color = vec4<f32>(1.0, 1.0, 1.0, pbr_input.material.base_color.a);
    lit.material.reflectance = vec3<f32>(0.0);
    lit.material.metallic = 0.0;
    lit.material.perceptual_roughness = 1.0;
    lit.material.emissive = vec4<f32>(0.0);
    lit.specular_occlusion = 0.0;
    // the light falling on the hair, exposure applied
    let irradiance = apply_pbr_lighting(lit).rgb;

    // ---- what the engine adds per light ----
    // the engine's diffuse of the lights (wrap and rim included) and the Lambert part of them in Bevy's lighting
    var direct = vec3<f32>(0.0);
    var lambert = vec3<f32>(0.0);
    var specular = vec3<f32>(0.0);
    let view_z = dot(vec4<f32>(
        view.view_from_world[0].z,
        view.view_from_world[1].z,
        view.view_from_world[2].z,
        view.view_from_world[3].z
    ), pbr_input.world_position);
    for (var i: u32 = 0u; i < lights.n_directional_lights; i = i + 1u) {
        let light = &lights.directional_lights[i];
        let l = (*light).direction_to_light;
        var shadow = 1.0;
        if ((pbr_input.flags & MESH_FLAGS_SHADOW_RECEIVER_BIT) != 0u
                && ((*light).flags & DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) != 0u) {
            shadow = fetch_directional_shadow(i, pbr_input.world_position, pbr_input.world_normal, view_z, pbr_input.frag_coord.xy);
        }
        let e = (*light).color.rgb;
        let terms = hair_light(s, p, l, e, 1.0, shadow);
        direct += terms.diffuse;
        lambert += e * (shadow * saturate(dot(n, l)) / PI);
        specular += terms.specular;
        // the strand highlight that stays in the shadow (`shadow area default specular`)
        let up = normalize(vec3<f32>(v.x, 1.0, v.z));
        let shadow_spec = kajiya_kay(s.t2, up, p.primary.w * 0.3) * p.primary.rgb;
        let lit_amount = saturate(shadow * saturate(dot(n, l) * 3.0));
        specular += 0.2 * shadow_spec * (1.0 - lit_amount) * (e / PI);
    }

    // the two character back lights (`%_RT_OBJECT_BACKLIGHT`), unshadowed, in the character's own space
    let world_from_local = get_world_from_local(in.instance_index);
    var back_direct = vec3<f32>(0.0);
    var back_specular = vec3<f32>(0.0);
    for (var i = 0; i < 2; i++) {
        let light = data.back.lights[i];
        if max(light.r, max(light.g, light.b)) <= 0.0 {
            continue;
        }
        let d = data.back.dirs[i];
        let l = normalize((world_from_local * vec4<f32>(d.xyz, 0.0)).xyz);
        let terms = hair_light(s, p, l, light.rgb * d.w, light.w, 1.0);
        back_direct += terms.diffuse;
        back_specular += terms.specular;
    }

    let exposure = view.exposure;
    // `(cAmbientAcc + cDiffuseAcc)`: everything the hair receives
    // Bevy's lighting minus its Lambert part of the directional lights: the ambient (and point lights)
    let ambient = max(irradiance - lambert * exposure, vec3<f32>(0.0));
    let received = ambient + direct * exposure;
    // the ambient strand highlight: the view direction as half vector
    let ambient_spec = (kajiya_kay(s.t1, v, p.primary.w) * p.primary.rgb + kajiya_kay(s.t2, v, p.secondary.w) * p.secondary.rgb)
        * p.light.z * received;

    // the world's gain (`mode.z`, 0 = 1: the login stage) on all but the back lights, like the other character materials
    let gain = select(p.mode.z, 1.0, p.mode.z <= 0.0);
    let back = (back_direct * albedo + back_specular * gloss_rgb * p.spec.rgb) * exposure;
    let colour = gain * (received * albedo + (specular * exposure + ambient_spec) * gloss_rgb * p.spec.rgb) + back;
    out.color = main_pass_post_lighting_processing(pbr_input, vec4<f32>(colour, out_alpha));
    return out;
}
