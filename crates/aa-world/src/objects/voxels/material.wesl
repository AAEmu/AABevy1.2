import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
};

struct VoxelParams { flags: vec4<f32> }
@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<uniform> voxel: VoxelParams;
@group(constants::MATERIAL_BIND_GROUP) @binding(101) var cover: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(102) var cover_sampler: sampler;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) front: bool) -> FragmentOutput {
    var pbr = pbr_input_from_standard_material(in, front);
    var colour = pbr.material.base_color.rgb;
    if voxel.flags.x > 0.5 {
        // Original common.cfi tex2DTerrain: filter encoded chromaticity/brightness first.
        let raw = textureSample(cover, cover_sampler, in.uv);
        let red = (raw.r + 0.001012) * (31.0 / 30.0);
        let gamma = saturate(vec3<f32>(red, raw.g, 1.0 - red - raw.g) * (3.0 * raw.b));
        let cover_linear = pow(gamma, vec3<f32>(2.2));
        // TerrainVS thresholds g>=0.5 at vertices, then TerrainPS interpolates the flag.
        colour = mix(cover_linear, colour, saturate(in.color.a));
    }
    pbr.material.base_color = vec4<f32>(colour, 1.0);
    var out: FragmentOutput;
    if (pbr.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        out.color = apply_pbr_lighting(pbr);
    } else {
        out.color = pbr.material.base_color;
    }
    out.color = main_pass_post_lighting_processing(pbr, out.color);
    return out;
}
