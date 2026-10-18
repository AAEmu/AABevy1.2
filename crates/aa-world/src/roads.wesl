// Illum roads: StandardMaterial lighting, an independently transformed opacity map,
// and the spline's vertex alpha. The source shader is illum.cfx in the client pak.
import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
};

struct DeferredParams {
    center: vec4<f32>, right: vec4<f32>, up: vec4<f32>, front: vec4<f32>, depth: vec4<f32>,
}
struct OpacityParams {
    mask: vec4<f32>,
    uv_x: vec4<f32>,
    uv_y: vec4<f32>,
    attenuation: vec4<f32>,
    projection: DeferredParams,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<uniform> opacity: OpacityParams;
@group(constants::MATERIAL_BIND_GROUP) @binding(101) var opacity_map: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(102) var opacity_sampler: sampler;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var input = in;
    let native = opacity.projection;
    var deferred_alpha = 1.0;
    var outside_decal = false;
    if native.center.w > 0.0 {
        // Remove the receiver mesh's depth-fighting offset before native projection.
        let relative = in.world_position.xyz - in.world_normal * native.depth.z - native.center.xyz;
        let local = vec3<f32>(dot(relative, native.right.xyz), dot(relative, native.up.xyz), dot(relative, native.front.xyz));
        let uv = local.xy * vec2<f32>(native.right.w, -native.up.w) * 0.5 + 0.5;
        let z = (native.depth.x - local.z) * native.depth.y;
        outside_decal = any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) || z < 0.0 || z > 1.0;
        let falloff = (z - 0.2) * select(1.0, 5.0, z < 0.2);
        deferred_alpha = saturate(1.0 - falloff * falloff);
        input.uv = uv;
        if native.center.w > 1.5 {
            let world_normal = normalize(in.world_normal);
            let normal = vec3<f32>(dot(world_normal, native.right.xyz), dot(world_normal, native.up.xyz), dot(world_normal, native.front.xyz));
            var offset = local.z * normal.xy;
            if native.center.w > 2.5 {
                let native_axis = vec3<f32>(native.front.x, -native.front.z, native.front.y);
                let cosine = dot(native_axis, normal);
                offset *= pow(sqrt(1.0 - cosine * cosine), 8.0);
            }
            // Native g_decalSize.zw uses swapped inverse X/Y extents.
            input.uv = saturate((local.xy - offset) * vec2<f32>(native.up.w, -native.right.w) * 0.5 + 0.5);
        }
    }
    var pbr_input = pbr_input_from_standard_material(input, is_front);
    if opacity.attenuation.w > 0.0 {
        let relative = (in.world_position.xyz - opacity.attenuation.xyz) * opacity.attenuation.w;
        pbr_input.material.base_color.a *= saturate(1.0 - dot(relative, relative));
    }
    if opacity.uv_x.w > 0.0 {
        let uv = vec2<f32>(dot(opacity.uv_x.xyz, vec3<f32>(input.uv, 1.0)), dot(opacity.uv_y.xyz, vec3<f32>(input.uv, 1.0)));
        let mask = saturate(dot(opacity.mask, textureSample(opacity_map, opacity_sampler, uv)));
        if native.center.w > 0.0 {
            // Original deferred opacity replaces diffuse alpha, rather than multiplying it.
            pbr_input.material.base_color.a = native.front.w * mask;
        } else {
            pbr_input.material.base_color.a *= mask;
        }
    }
    if native.center.w > 0.0 {
        pbr_input.material.base_color.a *= deferred_alpha;
        if outside_decal || pbr_input.material.base_color.a < native.depth.w { discard; }
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
