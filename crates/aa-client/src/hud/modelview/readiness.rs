//! Preview-only preflight of the exact mesh key used by Bevy SetMeshBindGroup.
//! A pipeline can become ready after mesh-group preparation in the same frame.
//! Defer that draw while its required group is absent; never catch/whitelist an
//! error from the real draw. Unmarked views call their original draw unchanged.
use super::host::ModelViewCamera;
use bevy::{
    core_pipeline::{
        core_3d::{AlphaMask3d, Opaque3d, Transparent3d},
        prepass::MotionVectorPrepass,
    },
    pbr::{
        MeshBindGroups, MeshMorphBindGroupKey, MorphIndices, RenderMeshInstances,
        SkinUniforms,
    },
    prelude::*,
    render::{
        RenderApp, RenderStartup,
        extract_component::ExtractComponentPlugin,
        mesh::{MeshMetadataFallbackBuffer, allocator::MeshAllocator},
        render_phase::{Draw, DrawError, DrawFunctions, PhaseItem, TrackedRenderPass},
    },
};
use std::{
    any::TypeId,
    marker::PhantomData,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

#[derive(Resource, Clone, Default)]
struct Deferred(Arc<AtomicU64>);

fn mesh_group_ready<P: PhaseItem>(world: &World, view: Entity, item: &P) -> bool {
    let Some(instances) = world.get_resource::<RenderMeshInstances>() else {
        return false;
    };
    let entity = item.main_entity();
    let Some(mesh) = instances.mesh_asset_id(entity) else {
        return true;
    };
    let (Some(groups), Some(allocator), Some(fallback), Some(skins), Some(morphs)) = (
        world.get_resource::<MeshBindGroups>(),
        world.get_resource::<MeshAllocator>(),
        world.get_resource::<MeshMetadataFallbackBuffer>(),
        world.get_resource::<SkinUniforms>(),
        world.get_resource::<MorphIndices>(),
    ) else {
        return false;
    };
    let slabs = allocator.mesh_slabs(&mesh);
    let metadata = slabs
        .and_then(|s| s.metadata_slab_id)
        .unwrap_or(fallback.slab_id);
    let morph = match morphs {
        MorphIndices::Uniform { current, .. } => {
            if current.contains_key(&entity) {
                MeshMorphBindGroupKey::Uniform(mesh)
            } else {
                MeshMorphBindGroupKey::NoMorphTargets
            }
        }
        MorphIndices::Storage { .. } => match slabs.and_then(|s| s.morph_target_slab_id) {
            Some(morph) => MeshMorphBindGroupKey::Storage((metadata, morph)),
            None => MeshMorphBindGroupKey::NoMorphTargets,
        },
    };
    let phase = match groups {
        MeshBindGroups::CpuPreprocessing(groups) => Some(groups),
        MeshBindGroups::GpuPreprocessing(groups) => groups.get(&TypeId::of::<P>()),
    };
    let Some(phase) = phase else { return false };
    phase
        .get(
            metadata,
            // The host spawns character and carried meshes without Lightmap;
            // marked views cannot see the world's baked/lightmapped meshes.
            None,
            skins.skin_byte_offset(entity).is_some(),
            morph,
            world.get::<MotionVectorPrepass>(view).is_some(),
        )
        .is_some()
}

struct Guard<P: PhaseItem> {
    original: Box<dyn Draw<P>>,
}
impl<P: PhaseItem> Draw<P> for Guard<P> {
    fn prepare(&mut self, world: &World) {
        self.original.prepare(world);
    }
    fn draw<'w>(
        &mut self,
        world: &'w World,
        pass: &mut TrackedRenderPass<'w>,
        view: Entity,
        item: &P,
    ) -> Result<(), DrawError> {
        if world.get::<ModelViewCamera>(view).is_some() && !mesh_group_ready(world, view, item) {
            if let Some(deferred) = world.get_resource::<Deferred>() {
                deferred.0.fetch_add(1, Ordering::Relaxed);
            }
            return Ok(());
        }
        self.original.draw(world, pass, view, item)
    }
}
struct Empty<P>(PhantomData<fn() -> P>);
impl<P: PhaseItem> Draw<P> for Empty<P> {
    fn draw<'w>(
        &mut self,
        _: &'w World,
        _: &mut TrackedRenderPass<'w>,
        _: Entity,
        _: &P,
    ) -> Result<(), DrawError> {
        unreachable!("temporary draw replacement")
    }
}
fn wrap<P: PhaseItem>(world: &mut World) {
    let Some(functions) = world.get_resource::<DrawFunctions<P>>() else {
        return;
    };
    for entry in &mut functions.write().draw_functions {
        let original = std::mem::replace(entry, Box::new(Empty::<P>(PhantomData)));
        *entry = Box::new(Guard { original });
    }
}
pub(super) fn plugin(app: &mut App) {
    app.init_resource::<Deferred>();
    let stats = app.world().resource::<Deferred>().clone();
    if app.get_sub_app(RenderApp).is_some() {
        app.add_plugins(ExtractComponentPlugin::<ModelViewCamera>::default());
        let render = app.get_sub_app_mut(RenderApp).unwrap();
        render.insert_resource(stats).add_systems(
            RenderStartup,
            (wrap::<Opaque3d>, wrap::<AlphaMask3d>, wrap::<Transparent3d>).chain(),
        );
    }
}
pub(super) fn deferred(world: &World) -> u64 {
    world
        .get_resource::<Deferred>()
        .map_or(0, |v| v.0.load(Ordering::Relaxed))
}
