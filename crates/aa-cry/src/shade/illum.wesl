// Recovered FORWARD `Illum` composition (`illum.cfx`/`fraglib.cfi`/`shadelib.cfi`), see `illum.rs`.
// Sun and clustered point/spot Lambert + unnormalised Phong, sun-weighted hemisphere ambient,
// light-modulated cube reflection, max(highlight, reflection), and character back lights.
// Opaque deferred/tiled native passes are distinct and are not reproduced by this forward shader.

import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    mesh_bindings::mesh,
    mesh_functions::get_world_from_local,
    mesh_view_bindings::{view, lights, clustered_lights},
    mesh_view_bindings as view_bindings,
    mesh_view_types::{DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT, POINT_LIGHT_FLAGS_SHADOWS_ENABLED_BIT, POINT_LIGHT_FLAGS_SPOT_LIGHT_Y_NEGATIVE, POINT_LIGHT_FLAGS_AFFECTS_LIGHTMAPPED_MESH_DIFFUSE_BIT},
    clustered_forward as clustering,
    pbr_lighting as lighting,
    mesh_types::MESH_FLAGS_SHADOW_RECEIVER_BIT,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_bindings,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
    shadows::{fetch_directional_shadow, fetch_point_shadow, fetch_spot_shadow},
};
@if(BINDLESS) import bevy_render::bindless::{bindless_samplers_filtering, bindless_textures_2d, bindless_textures_cube};
@if(BINDLESS) import bevy_pbr::render::pbr_bindings::material_indices;
@if(VISIBILITY_RANGE_DITHER) import bevy_pbr::render::pbr_functions::visibility_range_dither;
@if(CONTACT_SHADOWS && DEPTH_PREPASS) import bevy_pbr::render::{pbr_functions::calculate_contact_shadow, mesh_view_types::POINT_LIGHT_FLAGS_CONTACT_SHADOWS_ENABLED_BIT};

import super::illum_bindings::compose_alpha;
@if(BINDLESS) import super::illum_bindings::{illum_indices, illum_array};
@if(!BINDLESS) import super::illum_bindings::{illum_uniform, illum_gloss_map, illum_gloss_sampler, illum_env_map, illum_env_sampler, illum_light_specular, illum_opacity_map, illum_opacity_sampler, illum_clock};

const PI: f32 = 3.14159265;

fn motion_seconds(slot: u32) -> f32 {
@if(BINDLESS)
    let texel = textureLoad(bindless_textures_2d[illum_indices[slot].clock], vec2<i32>(0), 0);
@else
    let texel = textureLoad(illum_clock, vec2<i32>(0), 0);
    let bytes = vec4<u32>(round(texel * 255.0));
    return bitcast<f32>(bytes.x | (bytes.y << 8u) | (bytes.z << 16u) | (bytes.w << 24u));
}

fn local_specular_multiplier(slot: u32, light_id: u32) -> f32 {
    if light_id >= 65536u { return 1.0; }
    let xy = vec2<i32>(i32(light_id % 256u), i32(light_id / 256u));
@if(BINDLESS)
    let texel = textureLoad(bindless_textures_2d[illum_indices[slot].light_specular], xy, 0);
@else
    let texel = textureLoad(illum_light_specular, xy, 0);
    // Linear RGBA8 contains the exact four bytes of an f32. textureLoad bypasses
    // filtering; rounding UNORM values recovers the bytes without half precision loss.
    let bytes = vec4<u32>(round(texel * 255.0));
    return bitcast<f32>(bytes.x | (bytes.y << 8u) | (bytes.z << 16u) | (bytes.w << 24u));
}

// Bridge the existing Bevy luminous-intensity/range data to a forward light's amplitude.
// At d=r/2 Bevy gives I/r² * 4*(15/16)²; GetAttenuation gives 3/4.
// Their ratio is 75/16. This is a unit conversion at the callers' existing anchor,
// not an extra brightness calibration. Native deferred point falloff is a DIFFERENT path.
fn forward_light_amplitude(intensity: vec3<f32>, inverse_range_squared: f32) -> vec3<f32> {
    return intensity * inverse_range_squared * (75.0 / 16.0) / PI;
}

// Original shadelib.cfi:GetAttenuation and illum.cfx:frag_custom_per_light.
// No inverse-square term, BRDF normalization, area-light broadening or extra N.L on specular.
fn forward_light_falloff(distance_squared: f32, inverse_range_squared: f32) -> f32 {
    return saturate(1.0 - distance_squared * inverse_range_squared);
}

fn forward_light_highlight(r: vec3<f32>, l: vec3<f32>, power: f32, shadow: f32) -> f32 {
    return pow(saturate(dot(r, l)), power) * (shadow * 0.8 + 0.2);
}

// Existing world projectors split their RGB cookie into three lights. The atlas stores a
// scalar channel per light; multiplying each coloured light reconstructs the original RGB.
fn local_light_filter(light_id: u32, position: vec3<f32>, is_spot: bool) -> f32 {
    var filter_value = 1.0;
@if(LIGHT_TEXTURES) {
    let light = clustered_lights.data[light_id];
    if light.decal_index != 0xFFFFFFFFu {
        let decal = view_bindings::clustered_decals.decals[light.decal_index];
        let local_position = (decal.local_from_world * vec4<f32>(position, 1.0)).xyz;
        var uv = vec2<f32>(0.0);
        if is_spot {
            if local_position.z >= 0.0 {
                return 0.0;
            }
            uv = (local_position.xy / (local_position.z * light.spot_light_tan_angle)) * vec2<f32>(-0.5, 0.5) + 0.5;
            // Native fraglib.cfi clips the projected frustum, including behind the light.
            if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) {
                return 0.0;
            }
        } else {
            uv = lighting::cubemap_uv(local_position, decal.tag);
        }
        filter_value = textureSampleLevel(view_bindings::clustered_decal_textures[decal.base_color_texture_index], view_bindings::clustered_decal_sampler, uv, 0.0).r;
    }
}
    return filter_value;
}

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
@if(VERTEX_UVS_A) {
    if data.motion.mode.x > 0.5 {
        // Native sub_3822BEB0: type 2 writes amplitude * (rate * seconds),
        // then multiplies by static tiling and offset. Phase is not read.
        // fraglib initializes bumpTC=baseTC; ordinary gloss shares baseTC too.
        material_in.uv += data.motion.oscillator.zw * (data.motion.oscillator.xy * motion_seconds(slot));
    }
}
@if(VERTEX_COLORS)
    material_in.color = vec4<f32>(1.0);
    var vertex_factor = vec3<f32>(1.0);
@if(VERTEX_COLORS)
    vertex_factor = select(vec3<f32>(1.0), in.color.rgb * in.color.rgb, p.vertex.x > 0.5);
    var pbr_input = pbr_input_from_standard_material(material_in, is_front);
@if(VERTEX_UVS_A)
    let opacity_uv = in.uv;
@else
    let opacity_uv = vec2<f32>(0.0);
@if(BINDLESS)
    let opacity_map = textureSample(bindless_textures_2d[illum_indices[slot].opacity_map], bindless_samplers_filtering[illum_indices[slot].opacity_sampler], opacity_uv);
@else
    let opacity_map = textureSample(illum_opacity_map, illum_opacity_sampler, opacity_uv);
    if data.motion.mode.y > 0.5 {
        // VertexLib/Illum use raw IN.baseTC for opacity, never the moving diffuse UV.
        pbr_input.material.base_color.a *= saturate(dot(data.motion.opacity_mask, opacity_map));
    }
@if(VERTEX_UVS_A)
    var uv = material_in.uv;
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

    var vertex_alpha = 1.0;
@if(VERTEX_COLORS)
    vertex_alpha = in.color.a;
    // Raw diffuse is independent of MatDifColor/HDR factors, including black
    // material colour and zero-alpha texels. No division of the factored albedo.
@if(BINDLESS)
    let raw_diffuse = textureSampleBias(bindless_textures_2d[material_indices[slot].base_color_texture], bindless_samplers_filtering[material_indices[slot].base_color_sampler], uv, view.mip_bias);
@else
    let raw_diffuse = textureSampleBias(pbr_bindings::base_color_texture, pbr_bindings::base_color_sampler, uv, view.mip_bias);
@if(BINDLESS)
    let material_alpha = pbr_bindings::material_array[material_indices[slot].material].base_color.a;
@else
    let material_alpha = pbr_bindings::material.base_color.a;
    pbr_input.material.base_color.a = compose_alpha(pbr_input.material.base_color.a, material_alpha, vertex_alpha, data.alpha);
    pbr_input.material.base_color = alpha_discard(
        pbr_input.material.flags,
        pbr_input.material.alpha_cutoff,
        pbr_input.material.base_color
    );
    // fraglib's ambient pass adds glow AFTER composed vertex RGB and before fog.
    let alpha_glow = raw_diffuse.rgb * (raw_diffuse.a * data.alpha.z * data.alpha.y * view.exposure);

    var out: FragmentOutput;
    let unlit = (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) != 0u;
    if p.vertex.y < 0.5 || unlit {
        var colour = pbr_input.material.base_color;
        if !unlit {
            colour = apply_pbr_lighting(pbr_input);
        }
        out.color = main_pass_post_lighting_processing(pbr_input, vec4<f32>(colour.rgb * vertex_factor + alpha_glow, colour.a));
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

    // Remove precisely the white-albedo Bevy point/spot contribution before adding the
    // native forward light sum. Leaving it in `ambient` would apply hemisphere/sun weighting
    // to local diffuse and count it twice. Keep environment, SSAO and synthetic stage fills.
    var white: lighting::LightingInput;
    white.layers[lighting::LAYER_BASE].N = n;
    white.layers[lighting::LAYER_BASE].R = r;
    white.layers[lighting::LAYER_BASE].NdotV = max(dot(n, v), 0.0001);
    white.layers[lighting::LAYER_BASE].perceptual_roughness = 1.0;
    white.layers[lighting::LAYER_BASE].roughness = 1.0;
    white.P = pbr_input.world_position.xyz;
    white.V = v;
    white.diffuse_color = vec3<f32>(1.0);
    white.F0_metallic = vec3<f32>(1.0);
    white.F_ab = lighting::F_AB(1.0, white.layers[lighting::LAYER_BASE].NdotV);
    let cluster_index = clustering::view_fragment_cluster_index(pbr_input.frag_coord.xy, view_z, pbr_input.is_orthographic);
    let ranges = clustering::unpack_clusterable_object_index_ranges(cluster_index);
    var bevy_local = vec3<f32>(0.0);
    var local_diffuse = vec3<f32>(0.0);
    for (var i = ranges.first_point_light_index_offset; i < ranges.first_rect_light_index_offset; i += 1u) {
        let light_id = clustering::get_clusterable_object_id(i);
        let light = clustered_lights.data[light_id];
        let is_spot = i >= ranges.first_spot_light_index_offset;
        let delta = light.position_radius.xyz - white.P;
        let d2 = dot(delta, delta);
        // At the centre a punctual direction is undefined; keep the result finite.
        let l = delta * inverseSqrt(max(d2, 1e-12));
        var shadow = 1.0;
        if (pbr_input.flags & MESH_FLAGS_SHADOW_RECEIVER_BIT) != 0u && (light.flags & POINT_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) != 0u {
            if is_spot {
                shadow = fetch_spot_shadow(light_id, pbr_input.world_position, pbr_input.world_normal, light.shadow_map_near_z, pbr_input.frag_coord.xy);
            } else {
                shadow = fetch_point_shadow(light_id, pbr_input.world_position, pbr_input.world_normal, pbr_input.frag_coord.xy);
            }
        }
@if(CONTACT_SHADOWS && DEPTH_PREPASS) {
        let steps = view_bindings::contact_shadows_settings.linear_steps;
        if steps > 0u && (pbr_input.flags & MESH_FLAGS_SHADOW_RECEIVER_BIT) != 0u && shadow > 0.0 && (light.flags & POINT_LIGHT_FLAGS_CONTACT_SHADOWS_ENABLED_BIT) != 0u {
            shadow *= calculate_contact_shadow(white.P, pbr_input.frag_coord.xy, l, steps);
        }
}
@if(LIGHTMAP)
        let enable_diffuse = (light.flags & POINT_LIGHT_FLAGS_AFFECTS_LIGHTMAPPED_MESH_DIFFUSE_BIT) != 0u;
@else
        let enable_diffuse = true;
        if is_spot {
            bevy_local += lighting::spot_light(light_id, &white, enable_diffuse) * shadow;
        } else {
            bevy_local += lighting::point_light(light_id, &white, enable_diffuse, true) * shadow;
        }
        var filter_value = local_light_filter(light_id, white.P, is_spot);
        if is_spot {
            // Bevy's smooth circular cone bridges native projectors; the authored cookie
            // is preserved above. This is not the original square projector/cone setup.
            var direction = vec3<f32>(light.light_custom_data.x, 0.0, light.light_custom_data.y);
            direction.y = sqrt(max(0.0, 1.0 - dot(direction, direction)));
            if (light.flags & POINT_LIGHT_FLAGS_SPOT_LIGHT_Y_NEGATIVE) != 0u { direction.y = -direction.y; }
            let cone = saturate(dot(-direction, l) * light.light_custom_data.z + light.light_custom_data.w);
            filter_value *= cone * cone;
        }
        let e = forward_light_amplitude(light.color_inverse_square_range.rgb, light.color_inverse_square_range.w)
            * (forward_light_falloff(d2, light.color_inverse_square_range.w) * filter_value);
        if enable_diffuse { local_diffuse += e * (shadow * saturate(dot(n, l))); }
        specular += e * forward_light_highlight(r, l, power, shadow) * local_specular_multiplier(slot, light_id);
    }
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
    let ambient = max(irradiance - (lambert + bevy_local) * exposure, vec3<f32>(0.0)) * (p.gain.y * (n.y * 0.25 + 0.75)) + p.emission.rgb * exposure;
    // weighted by the sun: `amb * 0.6 + (saturate(N.L * 0.5 + 0.5) + 0.3) * amb * 0.4`
    let ambient_pass = ambient * (0.6 + 0.4 * (saturate(sun_half * 0.5 + 0.5) + 0.3));
    let direct = (lambert + local_diffuse) * exposure;
    let gain = p.gain.x;
    let albedo = pbr_input.material.base_color.rgb;
    let diffuse = (gain * (ambient_pass + direct) + back * exposure) * albedo;

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
        reflection = env * (p.env.x * fresnel) * (gain * (ambient + direct) + back * exposure) * gloss_type;
    }

    var colour = diffuse + max(highlight, reflection);
    if p.gain.z > 0.5 && p.gain.z < 1.5 {
        colour = reflection;
    } else if p.gain.z > 1.5 && p.gain.z < 2.5 {
        colour = highlight;
    }
    out.color = main_pass_post_lighting_processing(pbr_input, vec4<f32>(colour * vertex_factor + alpha_glow, pbr_input.material.base_color.a));
    return out;
}
