import aa_world::objects::voxels::io::{VoxelVertexOutput, voxel, standard};
import bevy_pbr::render::{
    forward_io::FragmentOutput,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(101) var cover: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(102) var cover_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(103) var diffuse: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(104) var diffuse_sampler: sampler;

@group(constants::MATERIAL_BIND_GROUP) @binding(105) var normal_map: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(106) var normal_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(107) var detail_normal: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(108) var detail_sampler: sampler;

@fragment
fn fragment(in: VoxelVertexOutput, @builtin(front_facing) front: bool) -> FragmentOutput {
    var pbr = pbr_input_from_standard_material(standard(in), front);
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
            if voxel.normals.x > 0.5 {
                let bump_uv = vec2<f32>(dot(voxel.bump_u,cry),dot(voxel.bump_v,cry));
                let raw_normal = textureSample(normal_map,normal_sampler,bump_uv);
                var bump = raw_normal.xyz * 2.0 - 1.0;
                if voxel.normals.x > 1.5 {
                    let xy = raw_normal.yx * 2.0 - 1.0;
                    bump = vec3<f32>(xy,sqrt(saturate(1.0-dot(xy,xy))));
                }
                if voxel.normals.y > 0.5 {
                    let detail_uv = vec2<f32>(dot(voxel.detail_u,cry),dot(voxel.detail_v,cry));
                    let dn = textureSample(detail_normal,detail_sampler,detail_uv).xy * 2.0 - 1.0;
                    // fraglib adds XY *after* base Z reconstruction; do not reconstruct it again.
                    bump = vec3<f32>(bump.xy + dn * voxel.normals.z,bump.z);
                }
                pbr.N = normalize(in.projected_u*bump.x + in.projected_v*bump.y + in.world_normal*bump.z);
            }
        }
    }
    // Isolated numeric fixture: encoded native world normal, bypassing light/BRDF differences.
    if voxel.normals.w > 0.5 { colour = pbr.N * 0.5 + 0.5; }
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
