import bevy_pbr::render::forward_io::VertexOutput;

struct VoxelParams {
    flags: vec4<f32>, u: vec4<f32>, v: vec4<f32>, origin: vec4<f32>, colour: vec4<f32>,
    normals: vec4<f32>, bump_u: vec4<f32>, bump_v: vec4<f32>, detail_u: vec4<f32>, detail_v: vec4<f32>,
}
@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<uniform> voxel: VoxelParams;

// Native TerrainLayerVS carries independently projected U/V vectors. A single Mikk tangent
// and handedness cannot represent these generally nonorthogonal interpolated vectors.
struct VoxelVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @location(1) world_normal: vec3<f32>,
    @if(VERTEX_UVS_A) @location(2) uv: vec2<f32>,
    @if(VERTEX_UVS_B) @location(3) uv_b: vec2<f32>,
    @if(VERTEX_TANGENTS) @location(4) world_tangent: vec4<f32>,
    @if(VERTEX_COLORS) @location(5) color: vec4<f32>,
    @if(VERTEX_OUTPUT_INSTANCE_INDEX) @location(6) @interpolate(flat) instance_index: u32,
    @if(VISIBILITY_RANGE_DITHER) @location(7) @interpolate(flat) visibility_range_dither: i32,
    @location(8) projected_u: vec3<f32>,
    @location(9) projected_v: vec3<f32>,
}

fn standard(input: VoxelVertexOutput) -> VertexOutput {
    var out: VertexOutput;
    out.position = input.position;
    out.world_position = input.world_position;
    out.world_normal = input.world_normal;
    @if(VERTEX_UVS_A) { out.uv = input.uv; }
    @if(VERTEX_UVS_B) { out.uv_b = input.uv_b; }
    @if(VERTEX_TANGENTS) { out.world_tangent = input.world_tangent; }
    @if(VERTEX_COLORS) { out.color = input.color; }
    @if(VERTEX_OUTPUT_INSTANCE_INDEX) { out.instance_index = input.instance_index; }
    @if(VISIBILITY_RANGE_DITHER) { out.visibility_range_dither = input.visibility_range_dither; }
    return out;
}
