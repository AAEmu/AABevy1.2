// Terrain detail layers over the baked cover colour, see `terrain_layers.rs`.
//
// The StandardMaterial part supplies the cover texel (base_color_texture), lighting, shadows and fog;
// this shader replaces its fragment stage to decode the cover colour, blend the surface type layers into
// the colour and the normal, and then lights the result like any other PBR fragment.

import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    mesh_view_bindings::view,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
};

struct TerrainParams {
    // rgb: linear tint of the result, w: distance at which the layers have faded out
    tint: vec4<f32>,
    // four vectors per (surface type * 3 + projection):
    //   0: (diffuse slot, normal slot (+ 1000: in the detail array), uv scale u, uv scale v)
    //   1: (uv offset u, uv offset v, detail strength, detail bump scale)
    //   2: (detail normal slot, detail uv scale u, v, material colour r)
    //   3: (detail uv offset u, v, material colour g, b)
    layers: array<vec4<f32>, 192>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<uniform> terrain: TerrainParams;
@group(constants::MATERIAL_BIND_GROUP) @binding(101) var diffuse_layers: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(102) var diffuse_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(103) var normal_layers: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(104) var normal_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(105) var detail_layers: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(106) var detail_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(107) var surface_map: texture_2d<u32>;

// Bevy (Y up, north = -Z) to the game's space (Z up, north = +Y): the layers are projected in the latter.
fn to_cry(v: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(v.x, -v.z, v.y);
}

fn surface_at(p: vec2<i32>) -> u32 {
    return textureLoad(surface_map, min(p, vec2<i32>(512, 512)), 0).r & 15u;
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

    // the four surrounding heightmap samples and their bilinear weights
    let g = vec2<f32>(in.uv.y, in.uv.x) * 512.0;
    let cell = floor(g);
    let f = g - cell;
    let i = vec2<i32>(cell);
    var ids = array<u32, 4>(
        surface_at(i),
        surface_at(i + vec2<i32>(1, 0)),
        surface_at(i + vec2<i32>(0, 1)),
        surface_at(i + vec2<i32>(1, 1)),
    );
    var weights = array<f32, 4>((1.0 - f.x) * (1.0 - f.y), f.x * (1.0 - f.y), (1.0 - f.x) * f.y, f.x * f.y);
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
    if fade > 0.001 {
        for (var k = 0; k < 4; k++) {
            let wt = weights[k];
            if wt < 0.002 {
                continue;
            }
            for (var p = 0; p < 3; p++) {
                // the engine blends each layer pass with sqrt(weight) to hide the seams between layers
                let w = sqrt(saturate(wt * axis[p])) * fade;
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
                    // along x: u = y, v = z
                    coord = pos.yz;
                    cgx = dx.yz;
                    cgy = dy.yz;
                    tangent = vec3<f32>(0.0, 0.0, -1.0);
                    bitangent = vec3<f32>(0.0, 1.0, 0.0);
                } else if p == 1 {
                    // along y: u = x, v = z
                    coord = pos.xz;
                    cgx = dx.xz;
                    cgy = dy.xz;
                    tangent = vec3<f32>(1.0, 0.0, 0.0);
                    bitangent = vec3<f32>(0.0, 1.0, 0.0);
                } else {
                    // along z: u = x, v = y
                    coord = pos.xy;
                    cgx = dx.xy;
                    cgy = dy.xy;
                    tangent = vec3<f32>(1.0, 0.0, 0.0);
                    bitangent = vec3<f32>(0.0, 0.0, -1.0);
                }
                let uv = coord * a.zw + b.xy;
                let gx = cgx * a.zw;
                let gy = cgy * a.zw;

                // the layer colour is an offset of the cover in gamma space (terrain.cfx, frag_custom_begin)
                let d = textureSampleGrad(diffuse_layers, diffuse_sampler, uv, i32(a.x), gx, gy).rgb;
                // the material's own diffuse colour multiplies the lit layer (`MatDifColor`, frag_custom_end)
                let mat_color = vec3<f32>(terrain.layers[entry + 2u].w, terrain.layers[entry + 3u].z, terrain.layers[entry + 3u].w);
                let c = pow(saturate(cover_gamma + (d - 0.5) * b.z), vec3<f32>(2.2)) * mat_color;
                albedo = mix(albedo, c, w);

                if a.y >= 0.0 {
                    // normal maps that are only 256 px live in the detail array (slot + 1000)
                    var t: vec2<f32>;
                    if a.y >= 1000.0 {
                        t = textureSampleGrad(detail_layers, detail_sampler, uv, i32(a.y) - 1000, gx, gy).rg * 2.0 - 1.0;
                    } else {
                        t = textureSampleGrad(normal_layers, normal_sampler, uv, i32(a.y), gx, gy).rg * 2.0 - 1.0;
                    }
                    let nz = sqrt(max(1.0 - dot(t, t), 0.0));
                    // a detail normal map is added to the xy of the base normal (fraglib.cfi, bDetailBumpMapping);
                    // the sum is renormalised below
                    let dt = terrain.layers[entry + 2u];
                    if dt.x >= 0.0 {
                        let dc = terrain.layers[entry + 3u];
                        let duv = coord * dt.yz + dc.xy;
                        let dn = textureSampleGrad(detail_layers, detail_sampler, duv, i32(dt.x), cgx * dt.yz, cgy * dt.yz).rg;
                        t += (dn * 2.0 - 1.0) * b.w;
                    }
                    let nt = vec3<f32>(t, nz);
                    let tg = normalize(tangent - n_in * dot(tangent, n_in));
                    let bt = normalize(bitangent - n_in * dot(bitangent, n_in));
                    let nw = normalize(tg * nt.x + bt * nt.y + n_in * nt.z);
                    normal = normalize(mix(normal, nw, w));
                }
            }
        }
    }

    pbr_input.material.base_color = vec4<f32>(albedo * terrain.tint.rgb, 1.0);
    pbr_input.N = normal;

    var out: FragmentOutput;
    if (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        out.color = apply_pbr_lighting(pbr_input);
    } else {
        out.color = pbr_input.material.base_color;
    }
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
