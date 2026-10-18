import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
};

struct VoxelParams { flags: vec4<f32>, u: vec4<f32>, v: vec4<f32>, origin: vec4<f32>, colour: vec4<f32> }
@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<uniform> voxel: VoxelParams;
@group(constants::MATERIAL_BIND_GROUP) @binding(101) var cover: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(102) var cover_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(103) var diffuse: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(104) var diffuse_sampler: sampler;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) front: bool) -> FragmentOutput {
    var pbr = pbr_input_from_standard_material(in, front);
    var colour = pbr.material.base_color.rgb;
    var base_gamma = pow(max(colour,vec3<f32>(0.0)),vec3<f32>(1.0/2.2));
    if voxel.flags.x > 0.5 {
        // Original common.cfi tex2DTerrain: filter encoded chromaticity/brightness first.
        let raw = textureSample(cover, cover_sampler, in.uv);
        let red = (raw.r + 0.001012) * (31.0 / 30.0);
        let gamma = saturate(vec3<f32>(red, raw.g, 1.0 - red - raw.g) * (3.0 * raw.b));
        let cover_linear = pow(gamma, vec3<f32>(2.2));
        // TerrainVS thresholds g>=0.5 at vertices, then TerrainPS interpolates the flag.
        colour = mix(cover_linear, colour, saturate(in.color.a));
        base_gamma = mix(gamma,base_gamma,saturate(in.color.a));
    }
    var alpha = 1.0;
    @if(VERTEX_UVS_B) {
        if voxel.flags.y > 0.5 {
            let bevy_local = in.world_position.xyz - voxel.origin.xyz;
            let cry = vec4<f32>(bevy_local.x,-bevy_local.z,bevy_local.y,1.0);
            let uv = vec2<f32>(dot(voxel.u,cry),dot(voxel.v,cry));
            let detail = textureSample(diffuse,diffuse_sampler,uv).rgb;
            colour = pow(saturate(base_gamma + (detail - 0.5) * voxel.flags.z),vec3<f32>(2.2)) * voxel.colour.rgb;
            alpha = sqrt(saturate(in.uv_b.x));
        }
    }
    pbr.material.base_color = vec4<f32>(colour, alpha);
    var out: FragmentOutput;
    if (pbr.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        out.color = apply_pbr_lighting(pbr);
    } else {
        out.color = pbr.material.base_color;
    }
    out.color = main_pass_post_lighting_processing(pbr, out.color);
    return out;
}
