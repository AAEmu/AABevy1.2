// Private Illum data/bindings, shared by main and prepass fragments.
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
    motion: MotionData,
    // x: VERTALPHA, y: ALPHAGLOW, z: authored multiplier in engine light units.
    alpha: vec4<f32>,
}

struct MotionData {
    oscillator: vec4<f32>, // rate U/V, amplitude U/V
    opacity_mask: vec4<f32>,
    mode: vec4<f32>,       // x: motion, y: independent opacity
}

@if(BINDLESS) {
struct IllumIndices {
    data: u32,           // 50
    gloss_map: u32,      // 51
    gloss_sampler: u32,  // 52
    env_map: u32,        // 53
    env_sampler: u32,    // 54
    light_specular: u32, // 55
    opacity_map: u32,    // 56
    opacity_sampler: u32,// 57
    clock: u32,          // 58
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
@group(constants::MATERIAL_BIND_GROUP) @binding(55) var illum_light_specular: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(56) var illum_opacity_map: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(57) var illum_opacity_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(58) var illum_clock: texture_2d<f32>;
}

// illum.cfx:873-889. Shared by colour, depth/normal/motion and shadow passes.
fn compose_alpha(current: f32, material: f32, vertex: f32, flags: vec4<f32>) -> f32 {
    if flags.y > 0.5 { return material * vertex; }
    if flags.x > 0.5 { return current * vertex; }
    return current;
}
