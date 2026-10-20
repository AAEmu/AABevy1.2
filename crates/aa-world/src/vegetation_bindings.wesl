import bevy_pbr::render::mesh_bindings::mesh;

struct VegetationUniform {
    brightness: vec4<f32>,
    wind: vec4<f32>,
    clock: vec4<f32>,
    detail: vec4<f32>,
}
@if(BINDLESS) {
    struct VegetationBindings { material: u32 }
    @group(constants::MATERIAL_BIND_GROUP) @binding(100) var<storage> vegetation_indices: array<VegetationBindings>;
    @group(constants::MATERIAL_BIND_GROUP) @binding(101) var<storage> vegetation_array: array<VegetationUniform>;
}
@else {
    @group(constants::MATERIAL_BIND_GROUP) @binding(50) var<uniform> vegetation: VegetationUniform;
}
fn parameters(instance: u32) -> VegetationUniform {
    @if(BINDLESS) {
        let slot = mesh[instance].material_and_lightmap_bind_group_slot & 0xffffu;
        return vegetation_array[vegetation_indices[slot].material];
    }
    @else { return vegetation; }
}
