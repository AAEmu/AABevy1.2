// Terrain detail layers over the baked cover colour, see `terrain_layers.rs`.
//
// The StandardMaterial part supplies the cover texel (base_color_texture), lighting, shadows and fog;
// this shader replaces its fragment stage to decode the cover colour, blend the surface type layers into
// the colour and the normal, and then lights the result like any other PBR fragment.

import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    mesh_view_bindings::{view, lights},
    mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT,
    mesh_types::MESH_FLAGS_SHADOW_RECEIVER_BIT,
    shadows::fetch_directional_shadow,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
};

struct TerrainParams {
    // rgb: linear tint of the result, w: distance at which the layers have faded out
    tint: vec4<f32>,
    inspect_normal: u32,
    // four vectors per (surface type * 3 + projection):
    //   0: (diffuse, normal, uv scale u, uv scale v); a texture is slot + 1000 * flags: diffuse flags 0 / 1 / 2
    //       = the 512 / 1024 / 2048 px array, normal flags bit 0 = 2 x 2 copies (half the uv), bit 1 = the
    //       second normal array
    //   1: (uv offset u, uv offset v, detail strength, detail bump scale)
    //   2: (detail normal (like the normal), detail uv scale u, v, material colour r)
    //   3: (detail uv offset u, v, material colour g, b)
    layers: array<vec4<f32>, 384>,
    normal_uv: array<vec4<f32>, 96>,
    specular: array<vec4<f32>, 192>,
    specular_light: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<uniform> terrain: TerrainParams;
@group(constants::MATERIAL_BIND_GROUP) @binding(101) var diffuse_layers: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(102) var diffuse_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(103) var normal_layers: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(104) var normal_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(105) var large_layers: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(106) var large_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(107) var surface_map: texture_2d<u32>;
// the arrays below share the sampler of the first array of their kind
@group(constants::MATERIAL_BIND_GROUP) @binding(108) var huge_layers: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(109) var more_normal_layers: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(110) var gloss_layers: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(111) var more_gloss_layers: texture_2d_array<f32>;

fn sample_gloss(code: f32, uv: vec2<f32>, gx: vec2<f32>, gy: vec2<f32>) -> vec3<f32> {
    if code < 0.0 { return vec3<f32>(1.0); }
    let flags = flags_of(code);
    let slot = i32(code) - flags * 1000;
    let k = select(1.0, 0.5, (flags & 1) != 0);
    if (flags & 2) != 0 {
        return textureSampleGrad(more_gloss_layers, diffuse_sampler, uv*k, slot, gx*k, gy*k).rgb;
    }
    return textureSampleGrad(gloss_layers, diffuse_sampler, uv*k, slot, gx*k, gy*k).rgb;
}

// Bevy (Y up, north = -Z) to the game's space (Z up, north = +Y): the layers are projected in the latter.
fn to_cry(v: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(v.x, -v.z, v.y);
}

// A table texture is slot + 1000 * flags.
fn flags_of(code: f32) -> i32 {
    return i32(floor(code / 1000.0));
}

// A diffuse map: flags 0 / 1 / 2 are the 512 / 1024 / 2048 px arrays.
fn sample_diffuse(code: f32, uv: vec2<f32>, gx: vec2<f32>, gy: vec2<f32>) -> vec3<f32> {
    let flags = flags_of(code);
    let slot = i32(code) - flags * 1000;
    if flags == 0 {
        return textureSampleGrad(diffuse_layers, diffuse_sampler, uv, slot, gx, gy).rgb;
    } else if flags == 1 {
        return textureSampleGrad(large_layers, large_sampler, uv, slot, gx, gy).rgb;
    }
    return textureSampleGrad(huge_layers, diffuse_sampler, uv, slot, gx, gy).rgb;
}

// A normal map (or detail normal map): bit 3 is raw BC1 XYZ in the first diffuse array;
// flags bit 1 is the second BC5 array, bit 0 a map that is stored as 2 x 2
// copies: sampled at half the uv it gives the original map back, with the same texel density and mip levels.
fn sample_normal(code: f32, uv: vec2<f32>, gx: vec2<f32>, gy: vec2<f32>) -> vec3<f32> {
    let flags = flags_of(code);
    let slot = i32(code) - flags * 1000;
    let k = select(1.0, 0.5, (flags & 1) != 0);
    if (flags & 8) != 0 {
        return textureSampleGrad(diffuse_layers, normal_sampler, uv * k, slot, gx * k, gy * k).rgb;
    }
    if (flags & 2) != 0 {
        return vec3<f32>(textureSampleGrad(more_normal_layers, normal_sampler, uv * k, slot, gx * k, gy * k).rg, 0.5);
    }
    return vec3<f32>(textureSampleGrad(normal_layers, normal_sampler, uv * k, slot, gx * k, gy * k).rg, 0.5);
}

fn surface_at(p: vec2<i32>) -> u32 {
    return textureLoad(surface_map, min(p, vec2<i32>(512, 512)), 0).r & 31u;
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);

    // the cover texel is chromaticity (r, g) and brightness (b), see `tex2DTerrain` of common.cfi;
    // the result is a gamma space colour, the space the engine blends the layers in
    let raw = pbr_input.material.base_color.rgb;
    let chroma_r = (raw.r + 0.001012) * (31.0 / 30.0);
    let cover_gamma = saturate(vec3<f32>(chroma_r, raw.g, 1.0 - chroma_r - raw.g) * (3.0 * raw.b));
    let cover = pow(cover_gamma, vec3<f32>(2.2));

    // cell-local position in the game's space and its screen space derivatives (for texture gradients)
    let wpos = in.world_position.xyz;
    let pos = vec3<f32>(in.uv.y * 1024.0, in.uv.x * 1024.0, wpos.y);
    let dx = to_cry(dpdx(wpos));
    let dy = to_cry(dpdy(wpos));

    // Native layer masks are interpolated over the actual B-D terrain triangles.
    // Heightmap meshes carry their grid step in UV1; isolated older fixtures use step1.
    let g = vec2<f32>(in.uv.y, in.uv.x) * 512.0;
    var lod_step = 1.0;
    @if(VERTEX_UVS_B) { lod_step = clamp(round(in.uv_b.x),1.0,16.0); }
    let cell = floor(g / lod_step) * lod_step;
    let f = (g - cell) / lod_step;
    let i = vec2<i32>(cell);
    let step = i32(lod_step);
    var ids = array<u32, 4>(
        surface_at(i),
        surface_at(i + vec2<i32>(step, 0)),
        surface_at(i + vec2<i32>(0, step)),
        surface_at(i + vec2<i32>(step, step)),
    );
    let triangle = select(vec4<f32>(1.0-f.x-f.y,f.x,f.y,0.0),
        vec4<f32>(0.0,1.0-f.y,1.0-f.x,f.x+f.y-1.0),f.x+f.y>1.0);
    var weights = array<f32, 4>(triangle.x,triangle.y,triangle.z,triangle.w);
    // merge samples of the same type into the first of them
    for (var a = 0; a < 3; a++) {
        for (var b = a + 1; b < 4; b++) {
            if ids[b] == ids[a] && weights[b] > 0.0 {
                weights[a] += weights[b];
                weights[b] = 0.0;
            }
        }
    }

    // projection weights: the dominant axis of the normal (x, y, z), like the engine's vertex shader
    let n_in = pbr_input.N;
    let nc = abs(to_cry(n_in));
    let axis = vec3<f32>(
        clamp((nc.x - max(nc.y, nc.z)) * 5.0 + 0.5, 0.0, 1.0),
        clamp((nc.y - max(nc.x, nc.z)) * 5.0 + 0.5, 0.0, 1.0),
        clamp((nc.z - max(nc.x, nc.y)) * 5.0 + 0.5, 0.0, 1.0),
    );

    let dist = distance(view.world_position.xyz, wpos);
    let fade = 1.0 - pow(min(dist / terrain.tint.w, 1.0), 4.0);

    var albedo = cover;
    var normal = n_in;
    var specular = vec3<f32>(0.0);
    var sun: array<vec3<f32>, u32(constants::MAX_DIRECTIONAL_LIGHTS)>;
    if fade > 0.0 && terrain.specular_light.y < 1.5 {
        let view_z = dot(vec4<f32>(view.view_from_world[0].z, view.view_from_world[1].z,
            view.view_from_world[2].z, view.view_from_world[3].z), pbr_input.world_position);
        for (var li = 0u; li < lights.n_directional_lights; li++) {
            let light = lights.directional_lights[li];
            var shadow = 1.0;
            if (pbr_input.flags & MESH_FLAGS_SHADOW_RECEIVER_BIT) != 0u &&
                (light.flags & DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) != 0u {
                shadow = fetch_directional_shadow(li, pbr_input.world_position, pbr_input.world_normal,
                    view_z, pbr_input.frag_coord.xy) * pbr_input.directional_shadow_factor;
            }
            // Native forward terrain.cfx: shadow*.8+.2, unnormalised Phong (no N.L).
            // /PI bridges the existing Bevy lux sun to the client's radiance-like light.
            // No fitted material/highlight gain. The native deferred Blinn pass differs.
            sun[li] = light.color.rgb * ((shadow*0.8+0.2) * terrain.specular_light.x / 3.14159265359);
        }
    }
    if fade > 0.0 {
        for (var k = 0; k < 4; k++) {
            let wt = weights[k];
            if wt < 0.002 {
                continue;
            }
            for (var p = 0; p < 3; p++) {
                // TerrainLayerVS puts fade in Color.g; the fragment takes its sqrt (terrain.cfx453/614).
                let w = sqrt(saturate(wt * axis[p] * fade));
                if w < 0.004 {
                    continue;
                }
                let entry = (ids[k] * 3u + u32(p)) * 4u;
                let a = terrain.layers[entry];
                if a.x < 0.0 {
                    continue;
                }
                let b = terrain.layers[entry + 1u];

                // the projection: the two cell-local coordinates that are the layer's u and v, their
                // screen space derivatives and the tangent frame (u, v) in Bevy space
                var coord: vec2<f32>;
                var cgx: vec2<f32>;
                var cgy: vec2<f32>;
                var tangent: vec3<f32>;
                var bitangent: vec3<f32>;
                if p == 0 {
                    // LayerTexGen along x: u = y, v = -z (negative scale is in the table).
                    coord = pos.yz;
                    cgx = dx.yz;
                    cgy = dy.yz;
                    tangent = vec3<f32>(0.0, 0.0, -1.0);
                    bitangent = vec3<f32>(0.0, -1.0, 0.0);
                } else if p == 1 {
                    // along y: u = x, v = -z
                    coord = pos.xz;
                    cgx = dx.xz;
                    cgy = dy.xz;
                    tangent = vec3<f32>(1.0, 0.0, 0.0);
                    bitangent = vec3<f32>(0.0, -1.0, 0.0);
                } else {
                    // along z: u = x, v = -y
                    coord = pos.xy;
                    cgx = dx.xy;
                    cgy = dy.xy;
                    tangent = vec3<f32>(1.0, 0.0, 0.0);
                    bitangent = vec3<f32>(0.0, 0.0, 1.0);
                }
                let uv = coord * a.zw + b.xy;
                let gx = cgx * a.zw;
                let gy = cgy * a.zw;

                // the layer colour is an offset of the cover in gamma space (terrain.cfx, frag_custom_begin)
                let d = sample_diffuse(a.x, uv, gx, gy);
                // the material's own diffuse colour multiplies the lit layer (`MatDifColor`, frag_custom_end)
                let mat_color = vec3<f32>(terrain.layers[entry + 2u].w, terrain.layers[entry + 3u].z, terrain.layers[entry + 3u].w);
                let c = pow(saturate(cover_gamma + (d - 0.5) * b.z), vec3<f32>(2.2)) * mat_color;
                albedo = mix(albedo, c, w);

                var layer_normal = n_in;
                if a.y >= 0.0 {
                    // Native TerrainLayerVS applies TS_BUMP independently of TS_DIFFUSE.
                    let transform = terrain.normal_uv[ids[k]*3u+u32(p)];
                    let sampled = sample_normal(a.y, uv*transform.xy+transform.zw,
                        gx*transform.xy, gy*transform.xy) * 2.0 - 1.0;
                    var t = sampled.xy;
                    // Common.cfi GetNormalMap reconstructs Z only for the non-DXT1 path.
                    let nz = select(sqrt(max(1.0 - dot(t, t), 0.0)), sampled.z, (flags_of(a.y) & 8) != 0);
                    // a detail normal map is added to the xy of the base normal (fraglib.cfi, bDetailBumpMapping);
                    // the sum is renormalised below
                    let dt = terrain.layers[entry + 2u];
                    if dt.x >= 0.0 {
                        let dc = terrain.layers[entry + 3u];
                        let duv = coord * dt.yz + dc.xy;
                        var dn = sample_normal(dt.x, duv, cgx * dt.yz, cgy * dt.yz).xy;
                        // The shared arrays put ATI2 into semantic XY for GetNormalMap.
                        // Native detailMapSampler reads raw XY, without its D3D10 YX swap.
                        if (flags_of(dt.x) & 4) != 0 { dn = dn.yx; }
                        t += (dn * 2.0 - 1.0) * b.w;
                    }
                    let nt = vec3<f32>(t, nz);
                    let tg = normalize(tangent - n_in * dot(tangent, n_in));
                    let bt = normalize(bitangent - n_in * dot(bitangent, n_in));
                    let nw = normalize(tg * nt.x + bt * nt.y + n_in * nt.z);
                    layer_normal = nw;
                    normal = normalize(mix(normal, nw, w));
                }
                var highlight = vec3<f32>(0.0);
                let si = (ids[k] * 3u + u32(p)) * 2u;
                let authored = terrain.specular[si];
                if terrain.specular_light.y < 1.5 && any(authored.rgb > vec3<f32>(0.0)) {
                    // Gloss uses diffuse baseTC, independently of the TS_BUMP transform.
                    let gloss = sample_gloss(terrain.specular[si+1u].x, uv, gx, gy);
                    for (var li = 0u; li < lights.n_directional_lights; li++) {
                        let r = reflect(-lights.directional_lights[li].direction_to_light, layer_normal);
                        highlight += sun[li] * pow(saturate(dot(pbr_input.V, r)), authored.w);
                    }
                    highlight *= gloss * authored.rgb;
                }
                // Match the existing ordered source-alpha layer composition. A matte
                // layer also attenuates previous highlights; do not sum layer powers.
                specular = mix(specular, highlight, w);
            }
        }
    }

    pbr_input.material.base_color = vec4<f32>(albedo * terrain.tint.rgb, 1.0);
    pbr_input.N = normal;
    if terrain.inspect_normal != 0u {
        pbr_input.material.base_color = vec4<f32>(normal * 0.5 + vec3<f32>(0.5), 1.0);
    }

    var out: FragmentOutput;
    if terrain.inspect_normal == 0u && (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        if terrain.specular_light.y < 1.5 {
            // Authored highlights replace the old uniform PBR reflectance, including on matte layers.
            pbr_input.material.reflectance = vec3<f32>(0.0);
        }
        out.color = apply_pbr_lighting(pbr_input);
        out.color = vec4<f32>(out.color.rgb + specular * view.exposure, out.color.a);
        if terrain.specular_light.y > 0.5 && terrain.specular_light.y < 1.5 {
            out.color = vec4<f32>(specular * view.exposure, 1.0);
        }
    } else {
        out.color = pbr_input.material.base_color;
    }
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
