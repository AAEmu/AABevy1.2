//! Per-light native specular multiplier. The buffer is shared by all Illum materials;
//! its slots follow Bevy's current render-entity map, never main-world entity order.

use bevy::ecs::{entity::EntityHashMap, system::SystemParamItem};
use bevy::pbr::{ExtractedPointLight, GlobalClusterableObjectMeta, prepare_lights};
use bevy::prelude::*;
use bevy::render::extract_component::{ExtractComponent, ExtractComponentPlugin};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::{GpuResourceAppExt, Render, RenderApp, RenderSystems};
use bevy::render::{
    render_asset::RenderAssets,
    storage::GpuShaderBuffer,
    texture::{FallbackImage, GpuImage},
};

use super::{IllumData, IllumExt};

/// Native light `specular_multiplier`, applied only to Full Illum's local highlight.
/// Attach to the actual PointLight/SpotLight entity (each RGB projector channel).
/// Missing = 1. Explicit zero disables that light's highlight, not its diffuse/reflection.
#[derive(Component, Clone, Copy, Debug, PartialEq, ExtractComponent)]
#[extract_app(RenderApp)]
pub struct IllumLightSpecular(pub f32);

// A stable allocation avoids rebuilding every material bind group as lights come/go.
// Current world/fx budgets are tens of lights; 65536 slots cost 256 KiB. Larger GPU ids
// explicitly fall back to 1 in the shader instead of reading outside the allocation.
const CAPACITY: usize = 65536;

/// Render-only binding resource; public solely because it occurs in AsBindGroup::Param.
#[doc(hidden)]
#[derive(Resource)]
pub struct IllumLightBuffer {
    buffer: Buffer,
    values: Vec<f32>,
    bytes: Vec<u8>,
}

impl FromWorld for IllumLightBuffer {
    fn from_world(world: &mut World) -> Self {
        let device = world.resource::<RenderDevice>();
        let bytes: Vec<_> = (0..CAPACITY).flat_map(|_| 1.0_f32.to_le_bytes()).collect();
        Self {
            buffer: device.create_buffer_with_data(&BufferInitDescriptor {
                label: Some("Illum local light specular multipliers"),
                contents: &bytes,
                usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            }),
            values: vec![1.0],
            bytes: Vec::new(),
        }
    }
}

pub(super) fn build(app: &mut App) {
    app.add_plugins(ExtractComponentPlugin::<IllumLightSpecular>::default());
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render.init_gpu_resource::<IllumLightBuffer>();
        render.add_systems(
            Render,
            upload
                .in_set(RenderSystems::PrepareResources)
                .after(prepare_lights),
        );
    }
}

fn values_for(
    map: &EntityHashMap<usize>,
    components: impl IntoIterator<Item = (Entity, f32)>,
    previous_len: usize,
) -> Vec<f32> {
    let length = map
        .values()
        .max()
        .map_or(1, |i| i.saturating_add(1))
        .max(previous_len)
        .min(CAPACITY);
    let mut values = vec![1.0; length];
    for (entity, value) in components {
        let Some(&index) = map.get(&entity) else {
            continue;
        };
        if let Some(slot) = values.get_mut(index) {
            // No clamp of finite authored values: zero must survive. Nonfinite values
            // cannot be sent to a shader; use the documented missing-component default.
            *slot = if value.is_finite() { value } else { 1.0 };
        }
    }
    values
}

fn upload(
    meta: Res<GlobalClusterableObjectMeta>,
    components: Query<(Entity, &IllumLightSpecular), With<ExtractedPointLight>>,
    queue: Res<RenderQueue>,
    mut gpu: ResMut<IllumLightBuffer>,
) {
    // prepare_lights clears/sorts/rebuilds this map in CreateViews. PrepareResources
    // follows it, including its deferred command application. Never cache an entity's id.
    let values = values_for(
        &meta.entity_to_index,
        components.iter().map(|(e, v)| (e, v.0)),
        gpu.values.len(),
    );
    if values == gpu.values {
        return;
    }
    gpu.bytes.clear();
    gpu.bytes
        .extend(values.iter().flat_map(|v| v.to_le_bytes()));
    queue.write_buffer(&gpu.buffer, 0, &gpu.bytes);
    gpu.values = values;
}

// This private binding adapter supplies the render-world buffer without adding a
// field to public IllumExt. All materials reference the SAME stable allocation.
#[derive(AsBindGroup)]
#[data(50, IllumData, binding_array(101))]
#[bindless(index_table(range(50..56), binding(100)))]
struct Bindings {
    params: super::IllumParams,
    back: super::BackLightParams,
    #[texture(51)]
    #[sampler(52)]
    gloss_map: Option<Handle<Image>>,
    #[texture(53, dimension = "cube")]
    #[sampler(54)]
    env_map: Option<Handle<Image>>,
    #[storage(55, read_only, buffer, binding_array(102))]
    lights: Buffer,
}

impl From<&Bindings> for IllumData {
    fn from(b: &Bindings) -> Self {
        Self {
            illum: b.params,
            back: b.back,
        }
    }
}

impl AsBindGroup for IllumExt {
    type Data = ();
    type Param = (
        (
            Res<'static, RenderAssets<GpuImage>>,
            Res<'static, FallbackImage>,
            Res<'static, RenderAssets<GpuShaderBuffer>>,
        ),
        Res<'static, IllumLightBuffer>,
    );

    fn label() -> &'static str {
        "IllumExt"
    }
    fn bind_group_data(&self) -> Self::Data {}
    fn bindless_slot_count() -> Option<BindlessSlabResourceLimit> {
        Bindings::bindless_slot_count()
    }
    fn bindless_supported(device: &RenderDevice) -> bool {
        Bindings::bindless_supported(device)
    }
    fn bindless_descriptor() -> Option<BindlessDescriptor> {
        Bindings::bindless_descriptor()
    }
    fn bind_group_layout_entries(
        device: &RenderDevice,
        force_no_bindless: bool,
    ) -> Vec<BindGroupLayoutEntry> {
        Bindings::bind_group_layout_entries(device, force_no_bindless)
    }
    fn build_bind_group(
        &self,
        layout: &BindGroupLayout,
        device: &RenderDevice,
        (param, gpu): &mut SystemParamItem<'_, '_, Self::Param>,
        force_no_bindless: bool,
        output: &mut BindGroupBuilder,
    ) -> Result<(), AsBindGroupError> {
        Bindings {
            params: self.params,
            back: self.back,
            gloss_map: self.gloss_map.clone(),
            env_map: self.env_map.clone(),
            lights: gpu.buffer.clone(),
        }
        .build_bind_group(layout, device, param, force_no_bindless, output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_gpu_indices_missing_components_dead_and_reindexed_lights() {
        let mut world = World::new();
        let a = world.spawn_empty().id();
        let b = world.spawn_empty().id();
        let dead = world.spawn_empty().id();
        let mut map = EntityHashMap::default();
        map.insert(a, 4);
        map.insert(b, 1);
        assert_eq!(
            values_for(&map, [(dead, 0.), (a, 0.02), (b, 0.5)], 1),
            vec![1., 0.5, 1., 1., 0.02]
        );
        map.insert(a, 1);
        map.insert(b, 0);
        assert_eq!(
            values_for(&map, [(a, 1. / 3.), (b, 0.)], 5),
            vec![0., 1. / 3., 1., 1., 1.]
        );
        assert_eq!(values_for(&map, [(a, 0.)], 5), vec![1., 0., 1., 1., 1.]);
        map.clear();
        assert_eq!(values_for(&map, [(a, 0.), (b, 0.02)], 5), vec![1.; 5]);
        world.despawn(a);
        let reused = world.spawn_empty().id();
        assert_ne!(reused, a);
        map.insert(reused, 0);
        assert_eq!(
            values_for(&map, [(a, 0.), (reused, 0.5)], 5),
            vec![0.5, 1., 1., 1., 1.]
        );
        assert_eq!(values_for(&map, [(reused, f32::NAN)], 1), vec![1.]);
    }
}
