//! App-owned renderer hooks. No production registration or public API.
use super::scene::{ProbeView, ReadinessStock, Source};
use bevy::{
    core_pipeline::{
        Core3d, Core3dSystems,
        core_3d::{AlphaMask3d, Opaque3d},
        prepass::{
            AlphaMask3dPrepass, Opaque3dPrepass, ViewPrepassTextures,
            node::{early_prepass, late_prepass},
        },
    },
    ecs::{
        query::ROQueryItem,
        schedule::SystemWithAccess,
        system::{IntoSystem, System, SystemParamItem, lifetimeless::SRes},
    },
    pbr::*,
    prelude::*,
    render::{
        Render, RenderApp, RenderStartup, RenderSystems,
        extract_component::ExtractComponentPlugin,
        render_phase::*,
        render_resource::*,
        renderer::{RenderContext, RenderDevice, ViewQuery},
        sync_world::MainEntity,
        texture::{ColorAttachment, TextureCache},
        view::{ExtractedView, RetainedViewEntity, ViewDepthStencilTexture},
    },
};
use std::{
    collections::{HashMap, HashSet},
    sync::Mutex,
    time::Instant,
};

pub(crate) struct ProbePasses;
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Stage {
    #[default]
    Z,
    Motion,
}
#[derive(Default)]
struct Submissions {
    stage: Stage,
    frame: u64,
    z: HashSet<(Entity, u64, CachedRenderPipelineId)>,
    draws: u64,
    instances: u64,
    indirect: u64,
    witness: u32,
    ranges: HashMap<(Entity, CachedRenderPipelineId), u32>,
}
#[derive(Resource, Default)]
struct Ledger(Mutex<Submissions>);
type PairIdentity = (
    RetainedViewEntity,
    u32,
    CachedRenderPipelineId,
    CachedRenderPipelineId,
    bevy::asset::AssetId<Mesh>,
    bevy::asset::UntypedAssetId,
);
#[derive(Clone)]
struct Pair {
    initial: CachedRenderPipelineId,
    after: CachedRenderPipelineId,
    z: CachedRenderPipelineId,
    motion: CachedRenderPipelineId,
    fail: u32,
}
#[derive(Resource, Default)]
struct Plans {
    pairs: HashMap<PairIdentity, Pair>,
    main: HashMap<CachedRenderPipelineId, Pair>,
    z: HashMap<CachedRenderPipelineId, Pair>,
    original_main: HashMap<CachedRenderPipelineId, CachedRenderPipelineId>,
    original_z: HashMap<CachedRenderPipelineId, CachedRenderPipelineId>,
    stock: HashMap<CachedRenderPipelineId, (CachedRenderPipelineId, CachedRenderPipelineId)>,
    epoch: u64,
    cpu_ns: u128,
    samples: u64,
    assigned:
        HashMap<(RetainedViewEntity, MainEntity), (CachedRenderPipelineId, CachedRenderPipelineId)>,
    sources: HashSet<MainEntity>,
    motion_only:
        HashMap<(RetainedViewEntity, u32, bevy::asset::UntypedAssetId), CachedRenderPipelineId>,
    motion_ids: HashSet<CachedRenderPipelineId>,
}

impl Plugin for ProbePasses {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            ExtractComponentPlugin::<ProbeView>::default(),
            ExtractComponentPlugin::<Source>::default(),
            ExtractComponentPlugin::<ReadinessStock>::default(),
        ));
        let render = app.sub_app_mut(RenderApp);
        render
            .init_resource::<Ledger>()
            .init_resource::<Plans>()
            .init_resource::<super::readiness::Control>()
            .init_resource::<super::readback::Readbacks>()
            .add_systems(RenderStartup, register)
            .add_systems(Render, restore.in_set(RenderSystems::PrepareAssets))
            .add_systems(
                Render,
                plan.in_set(RenderSystems::QueueMeshes)
                    .before(queue_material_meshes)
                    .before(queue_prepass_material_meshes),
            )
            .add_systems(
                Render,
                queue_independent_motion
                    .in_set(RenderSystems::QueueMeshes)
                    .after(queue_prepass_material_meshes)
                    .after(plan),
            )
            .add_systems(
                Render,
                readable_attachments.in_set(RenderSystems::PrepareBindGroups),
            )
            .add_systems(
                Render,
                finish_readiness_transition
                    .in_set(RenderSystems::PrepareBindGroups)
                    .after(plan),
            )
            .add_systems(Render, super::readback::map.in_set(RenderSystems::Cleanup))
            .add_systems(
                Core3d,
                motion
                    .after(Core3dSystems::MainPass)
                    .before(Core3dSystems::PostProcess),
            )
            .add_systems(
                Core3d,
                capture_no_attachments
                    .after(Core3dSystems::MainPass)
                    .before(Core3dSystems::PostProcess),
            );
        render
            .world_mut()
            .schedule_scope(Core3d, |world, schedule| {
                let _ = world;
                // Public graph-node replacement before initialization preserves ALL
                // existing predecessor/successor edges, including private deferred
                // systems. System-type sets cannot accept arbitrary members.
                let early_type = IntoSystem::into_system(early_prepass).system_type();
                let late_type = IntoSystem::into_system(late_prepass).system_type();
                let early = schedule
                    .graph()
                    .systems
                    .iter()
                    .find(|(_, s, _)| s.system_type() == early_type)
                    .unwrap()
                    .0;
                let late = schedule
                    .graph()
                    .systems
                    .iter()
                    .find(|(_, s, _)| s.system_type() == late_type)
                    .unwrap()
                    .0;
                *schedule.graph_mut().systems.get_mut(early).unwrap() =
                    SystemWithAccess::new(Box::new(IntoSystem::into_system(probe_early)));
                *schedule.graph_mut().systems.get_mut(late).unwrap() =
                    SystemWithAccess::new(Box::new(IntoSystem::into_system(probe_late)));
            });
    }
}

type PrepassCommands = (
    ChoosePrepass,
    SetPrepassViewBindGroup<0>,
    SetPrepassViewEmptyBindGroup<1>,
    SetMeshBindGroup<2>,
    SetMaterialBindGroup<3>,
    ResourceGate,
    DrawMesh,
    RecordZ,
);
type MainCommands = (
    ChooseMain,
    SetMeshViewBindGroup<0>,
    SetMeshViewBindingArrayBindGroup<1>,
    SetMeshBindGroup<2>,
    SetMaterialBindGroup<3>,
    ResourceGate,
    DrawMesh,
);

fn alias<P: PhaseItem, C: RenderCommand<P> + Send + Sync + 'static, T: 'static>(world: &mut World)
where
    C::Param: bevy::ecs::system::ReadOnlySystemParam,
{
    let draw = RenderCommandState::<P, C>::new(world);
    world
        .resource::<DrawFunctions<P>>()
        .write()
        .add_with::<T, _>(draw);
}
fn register(world: &mut World) {
    let witness = RenderCommandState::<WitnessItem, (DrawMesh, RecordWitness)>::new(world);
    world.insert_resource(WitnessState(Mutex::new(witness)));
    alias::<Opaque3d, MainCommands, DrawMaterial>(world);
    alias::<AlphaMask3d, MainCommands, DrawMaterial>(world);
    alias::<Opaque3dPrepass, PrepassCommands, DrawPrepass>(world);
    alias::<AlphaMask3dPrepass, PrepassCommands, DrawPrepass>(world);
    // Depth-only stock controls still need their empty material group.
    alias::<
        Opaque3dPrepass,
        (
            ChoosePrepass,
            SetPrepassViewBindGroup<0>,
            SetPrepassViewEmptyBindGroup<1>,
            SetMeshBindGroup<2>,
            SetPrepassEmptyMaterialBindGroup<3>,
            DrawMesh,
            RecordZ,
        ),
        DrawDepthOnlyPrepass,
    >(world);
}

fn restore(
    mut main: ResMut<SpecializedMaterialPipelineCache>,
    mut z: ResMut<SpecializedPrepassMaterialPipelineCache>,
    plans: Res<Plans>,
) {
    for (_, cache) in main.iter_mut() {
        for id in cache.values_mut() {
            if let Some(original) = plans.original_main.get(id) {
                *id = *original;
            }
        }
    }
    for (_, cache) in z.iter_mut() {
        for (_, id, _) in cache.values_mut() {
            if let Some(original) = plans.original_z.get(id) {
                *id = *original;
            }
        }
    }
}
fn variant(
    cache: &PipelineCache,
    base: CachedRenderPipelineId,
    kind: u8,
) -> CachedRenderPipelineId {
    let mut d = cache.get_render_pipeline_descriptor(base).clone();
    d.label = Some(format!("supplied mechanism variant {kind}").into());
    if kind == 1 {
        let depth = d.depth_stencil.as_mut().unwrap();
        depth.depth_compare = Some(CompareFunction::Equal);
        depth.depth_write_enabled = Some(false);
        d.fragment
            .as_mut()
            .unwrap()
            .shader_defs
            .push("SUPPLIED_AFTER_Z".into());
    }
    if (kind == 2 || kind == 3)
        && let Some(f) = &mut d.fragment
    {
        for (i, t) in f.targets.iter_mut().enumerate() {
            if let Some(t) = t
                && ((kind == 2 && i == 1) || (kind == 3 && i != 1))
            {
                t.write_mask = ColorWrites::empty();
            }
        }
    }
    if kind == 3 {
        let depth = d.depth_stencil.as_mut().unwrap();
        depth.depth_write_enabled = Some(false);
        depth.depth_compare = Some(CompareFunction::GreaterEqual);
        if let Some(f) = &mut d.fragment {
            f.shader_defs.push("SUPPLIED_MOTION".into());
        }
    }
    cache.queue_render_pipeline(d)
}

#[expect(
    clippy::too_many_arguments,
    reason = "Bevy render system with distinct resource access"
)]
fn plan(
    mut main: ResMut<SpecializedMaterialPipelineCache>,
    mut zcache: ResMut<SpecializedPrepassMaterialPipelineCache>,
    sources: Query<(&MainEntity, &Source)>,
    views: Query<(&ExtractedView, &ProbeView)>,
    cache: Res<PipelineCache>,
    mut plans: ResMut<Plans>,
    ledger: Res<Ledger>,
    mut dirty: ResMut<bevy::render::camera::DirtySpecializations>,
    meshes: Res<RenderMeshInstances>,
    materials: Res<RenderMaterialInstances>,
    tracked_stock: Query<&MainEntity, With<ReadinessStock>>,
    mut readiness: ResMut<super::readiness::Control>,
) {
    let start = Instant::now();
    plans.epoch += 1;
    readiness.begin(plans.epoch, tracked_stock.iter().copied());
    plans.sources = sources
        .iter()
        .filter(|(_, s)| !s.motion_only)
        .map(|(e, _)| *e)
        .collect();
    {
        let mut log = ledger.0.lock().unwrap();
        log.frame = plans.epoch;
        log.z.clear();
        log.ranges.clear();
    }
    for (view, _) in &views {
        let Some(main) = main.get_mut(&view.retained_view_entity) else {
            continue;
        };
        let Some(zitems) = zcache.get_mut(&view.retained_view_entity) else {
            continue;
        };
        for (entity, source) in &sources {
            if source.motion_only {
                continue;
            }
            let (Some(main_id), Some((_, z_id, _))) =
                (main.get_mut(entity), zitems.get_mut(entity))
            else {
                continue;
            };
            if cache.get_render_pipeline(*main_id).is_none()
                || cache.get_render_pipeline(*z_id).is_none()
            {
                continue;
            }
            // Explicit group includes one supplied mesh/material/deformation snapshot.
            // Pipeline identity partitions BOTH automatic batch sets, never only a representative.
            let (Some(mesh), Some(material)) = (
                meshes.mesh_asset_id(*entity),
                materials.instances.get(entity),
            ) else {
                continue;
            };
            let key = (
                view.retained_view_entity,
                source.group,
                *main_id,
                *z_id,
                mesh,
                material.asset_id,
            );
            let pair = if let Some(pair) = plans.pairs.get(&key) {
                pair.clone()
            } else {
                let p = Pair {
                    initial: variant(&cache, *main_id, 0),
                    after: if source.fail == 1 {
                        CachedRenderPipelineId::new(usize::MAX)
                    } else {
                        variant(&cache, *main_id, 1)
                    },
                    z: variant(&cache, *z_id, 2),
                    motion: variant(&cache, *z_id, 3),
                    fail: source.fail,
                };
                plans.original_main.insert(p.initial, *main_id);
                plans.original_z.insert(p.z, *z_id);
                plans.main.insert(p.initial, p.clone());
                plans.z.insert(p.z, p.clone());
                plans.pairs.insert(key, p.clone());
                p
            };
            *main_id = pair.initial;
            *z_id = pair.z;
            if plans
                .assigned
                .insert((view.retained_view_entity, *entity), (pair.initial, pair.z))
                != Some((pair.initial, pair.z))
            {
                dirty.changed_renderables.insert(*entity);
            }
        }
        // All writers to this view's motion texture are split, including stock controls.
        for (entity, (_, id, _)) in zitems.iter_mut() {
            if plans.z.contains_key(id) {
                continue;
            }
            if readiness.prepare(&cache, *entity, id) {
                dirty.changed_renderables.insert(*entity);
            }
            if cache.get_render_pipeline(*id).is_none() {
                continue;
            }
            let base = *id;
            let (z, m) = *plans
                .stock
                .entry(base)
                .or_insert_with(|| (variant(&cache, base, 2), variant(&cache, base, 3)));
            plans.original_z.insert(z, base);
            *id = z;
            plans.stock.insert(z, (z, m));
            if let Some(main_id) = main.get(entity)
                && plans
                    .assigned
                    .insert((view.retained_view_entity, *entity), (*main_id, z))
                    != Some((*main_id, z))
            {
                dirty.changed_renderables.insert(*entity);
            }
        }
    }
    plans.cpu_ns += start.elapsed().as_nanos();
    plans.samples += 1;
    if plans.epoch.is_multiple_of(240) {
        let log = ledger.0.lock().unwrap();
        info!(
            "native-pass-probe: epoch={} pairs={} plan_cpu_us={:.2} submitted={} instance_ranges={} indirect_commands={}",
            plans.epoch,
            plans.pairs.len(),
            plans.cpu_ns as f64 / plans.samples as f64 / 1000.0,
            log.draws,
            log.instances,
            log.indirect
        );
    }
}
fn finish_readiness_transition(
    mut cache: ResMut<PipelineCache>,
    plans: Res<Plans>,
    mut control: ResMut<super::readiness::Control>,
) {
    let split = control.fresh().and_then(|id| plans.stock.get(&id).copied());
    control.finish(&mut cache, split);
}
fn pair_ready(pair: &Pair, cache: &PipelineCache) -> bool {
    cache.get_render_pipeline(pair.after).is_some() && cache.get_render_pipeline(pair.z).is_some()
}
struct ChoosePrepass;
impl<P: CachedRenderPipelinePhaseItem> RenderCommand<P> for ChoosePrepass {
    type Param = (
        SRes<PipelineCache>,
        SRes<Plans>,
        SRes<Ledger>,
        SRes<super::readiness::Control>,
    );
    type ViewQuery = (Entity, Option<&'static ProbeView>);
    type ItemQuery = Option<&'static Source>;
    fn render<'w>(
        item: &P,
        (view_entity, view): ROQueryItem<Self::ViewQuery>,
        source: Option<ROQueryItem<Self::ItemQuery>>,
        (cache, plans, ledger, readiness): SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let (cache, plans) = (cache.into_inner(), plans.into_inner());
        let Some(view) = view else {
            let Some(p) = cache.get_render_pipeline(item.cached_pipeline()) else {
                return RenderCommandResult::Skip;
            };
            pass.set_render_pipeline(p);
            return RenderCommandResult::Success;
        };
        let stage = ledger.0.lock().unwrap().stage;
        if plans.motion_ids.contains(&item.cached_pipeline()) {
            if stage != Stage::Motion {
                return RenderCommandResult::Skip;
            }
            let Some(p) = cache.get_render_pipeline(item.cached_pipeline()) else {
                return RenderCommandResult::Skip;
            };
            pass.set_render_pipeline(p);
            return RenderCommandResult::Success;
        }
        if stage == Stage::Z && !view.z && plans.sources.contains(&item.main_entity()) {
            return RenderCommandResult::Skip;
        }
        let _ = source;
        if plans.sources.contains(&item.main_entity())
            && !plans.z.contains_key(&item.cached_pipeline())
        {
            return RenderCommandResult::Skip;
        }
        let id = if let Some(pair) = plans.z.get(&item.cached_pipeline()) {
            if view.z && (!pair_ready(pair, cache) || (pair.fail == 3 && plans.epoch % 2 == 1)) {
                return RenderCommandResult::Skip;
            }
            if stage == Stage::Z {
                pair.z
            } else {
                pair.motion
            }
        } else if let Some(&(z, motion)) = plans.stock.get(&item.cached_pipeline()) {
            if stage == Stage::Z { z } else { motion }
        } else {
            // The base may finish compiling AFTER plan(). That does not make
            // its unsplit descriptor safe in either pass on a marked view.
            // Keep its queue identity for planning/requeue next frame, but do
            // not submit until a planned, ready split variant is available.
            readiness.selection(item, view_entity, stage == Stage::Motion, None, cache);
            return RenderCommandResult::Skip;
        };
        let Some(p) = cache.get_render_pipeline(id) else {
            return RenderCommandResult::Skip;
        };
        readiness.selection(item, view_entity, stage == Stage::Motion, Some(id), cache);
        pass.set_render_pipeline(p);
        RenderCommandResult::Success
    }
}

// Separate admission from stock prepass AlphaMode filtering. Only explicitly
// supplied eligible controls enter; AlphaMode NEVER establishes native eligibility.
#[expect(
    clippy::too_many_arguments,
    reason = "Independent Bevy phase queue needs mesh, material and view resources"
)]
fn queue_independent_motion(
    sources: Query<(&MainEntity, &Source)>,
    views: Query<(
        &ExtractedView,
        &bevy::render::view::RenderVisibleEntities,
        &ProbeView,
    )>,
    materials: Res<bevy::render::erased_render_asset::ErasedRenderAssets<PreparedMaterial>>,
    instances: Res<RenderMaterialInstances>,
    mesh_instances: Res<RenderMeshInstances>,
    meshes: Res<bevy::render::render_asset::RenderAssets<bevy::render::mesh::RenderMesh>>,
    allocator: Res<bevy::render::mesh::allocator::MeshAllocator>,
    support: Res<bevy::render::batching::gpu_preprocessing::GpuPreprocessingSupport>,
    pipeline: Res<PrepassPipeline>,
    view_keys: Res<ViewKeyPrepassCache>,
    cache: Res<PipelineCache>,
    draws: Res<DrawFunctions<Opaque3dPrepass>>,
    mut plans: ResMut<Plans>,
    mut phases: ResMut<ViewBinnedRenderPhases<Opaque3dPrepass>>,
) {
    use bevy::core_pipeline::prepass::{OpaqueNoLightmap3dBatchSetKey, OpaqueNoLightmap3dBinKey};
    use bevy::material::key::{ErasedMaterialPipelineKey, ErasedMeshPipelineKey};
    let selected: HashMap<_, _> = sources
        .iter()
        .filter(|(_, s)| s.motion_only)
        .map(|(e, s)| (*e, s))
        .collect();
    for (view, visible, _) in &views {
        let Some(view_key) = view_keys.get(&view.retained_view_entity) else {
            continue;
        };
        if !view_key.contains(MeshPipelineKey::MOTION_VECTOR_PREPASS) {
            continue;
        }
        let Some(phase) = phases.get_mut(&view.retained_view_entity) else {
            continue;
        };
        let Some(visible) = visible.get::<Mesh3d>() else {
            continue;
        };
        for (render_entity, entity) in visible.iter_visible() {
            let Some(source) = selected.get(entity) else {
                continue;
            };
            let Some(instance) = instances.instances.get(entity) else {
                continue;
            };
            let Some(material) = materials.get(instance.asset_id) else {
                continue;
            };
            if plans.epoch == 360 {
                info!(
                    "native-pass-probe: explicitly supplied Blend/Add group={} actual_bindless={}",
                    source.group, material.properties.bindless
                );
            }
            let Some(mesh_instance) = mesh_instances.render_mesh_queue_data(*entity) else {
                continue;
            };
            let Some(mesh) = meshes.get(mesh_instance.mesh_asset_id()) else {
                continue;
            };
            let Some(slabs) = allocator.mesh_slabs(&mesh_instance.mesh_asset_id()) else {
                continue;
            };
            let key = (view.retained_view_entity, source.group, instance.asset_id);
            let id = if let Some(id) = plans.motion_only.get(&key) {
                *id
            } else {
                let specializer = PrepassPipelineSpecializer {
                    pipeline: pipeline.clone(),
                    properties: material.properties.clone(),
                };
                let mesh_key = *view_key
                    | MeshPipelineKey::from_bits_retain(mesh.key_bits.bits())
                    | MeshPipelineKey::MAY_DISCARD
                    | MeshPipelineKey::PREPASS_READS_MATERIAL;
                let key = ErasedMaterialPipelineKey {
                    type_id: instance.asset_id.type_id(),
                    mesh_key: ErasedMeshPipelineKey::new(mesh_key),
                    material_key: material.properties.material_key.clone(),
                };
                let mut d = specializer
                    .specialize(key, &mesh.layout)
                    .expect("supplied rigid motion specialization");
                let depth = d.depth_stencil.as_mut().unwrap();
                depth.depth_write_enabled = Some(false);
                depth.depth_compare = Some(CompareFunction::GreaterEqual);
                let f = d.fragment.as_mut().unwrap();
                f.shader_defs
                    .extend(["SUPPLIED_MOTION".into(), "SUPPLIED_BLEND_MOTION".into()]);
                for (i, t) in f.targets.iter_mut().enumerate() {
                    if let Some(t) = t {
                        t.write_mask = if i == 1 {
                            ColorWrites::ALL
                        } else {
                            ColorWrites::empty()
                        };
                        t.blend = None;
                    }
                }
                let id = cache.queue_render_pipeline(d);
                plans.motion_only.insert(
                    (view.retained_view_entity, source.group, instance.asset_id),
                    id,
                );
                plans.motion_ids.insert(id);
                id
            };
            phase.remove(*entity);
            phase.add(
                OpaqueNoLightmap3dBatchSetKey {
                    pipeline: id,
                    draw_function: draws.read().id::<DrawPrepass>(),
                    material_bind_group_index: Some(material.binding.group.0),
                    slabs,
                },
                OpaqueNoLightmap3dBinKey {
                    asset_id: mesh_instance.mesh_asset_id().into(),
                },
                (*render_entity, *entity),
                mesh_instance.current_uniform_index,
                BinnedRenderPhaseType::mesh(mesh_instance.should_batch(), &support),
            );
        }
    }
}
struct ChooseMain;
impl<P: CachedRenderPipelinePhaseItem> RenderCommand<P> for ChooseMain {
    type Param = (SRes<PipelineCache>, SRes<Plans>, SRes<Ledger>);
    type ViewQuery = (Entity, Option<&'static ProbeView>);
    type ItemQuery = Option<&'static Source>;
    fn render<'w>(
        item: &P,
        (entity, view): ROQueryItem<Self::ViewQuery>,
        source: Option<ROQueryItem<Self::ItemQuery>>,
        (cache, plans, ledger): SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let (cache, plans) = (cache.into_inner(), plans.into_inner());
        let _ = source;
        if plans.sources.contains(&item.main_entity())
            && view.is_some_and(|v| v.z)
            && !plans.main.contains_key(&item.cached_pipeline())
        {
            return RenderCommandResult::Skip;
        }
        let id = if let Some(pair) = plans.main.get(&item.cached_pipeline()) {
            if view.is_some_and(|v| v.z) {
                let log = ledger.0.lock().unwrap();
                if !pair_ready(pair, cache) || !log.z.contains(&(entity, log.frame, pair.z)) {
                    return RenderCommandResult::Skip;
                }
                pair.after
            } else {
                pair.initial
            }
        } else {
            item.cached_pipeline()
        };
        let Some(p) = cache.get_render_pipeline(id) else {
            return RenderCommandResult::Skip;
        };
        pass.set_render_pipeline(p);
        RenderCommandResult::Success
    }
}
struct ResourceGate;
impl<P: CachedRenderPipelinePhaseItem> RenderCommand<P> for ResourceGate {
    type Param = SRes<Plans>;
    type ViewQuery = ();
    type ItemQuery = ();
    fn render<'w>(
        item: &P,
        _: (),
        _: Option<()>,
        plans: SystemParamItem<'w, '_, Self::Param>,
        _: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        if plans
            .main
            .get(&item.cached_pipeline())
            .or_else(|| plans.z.get(&item.cached_pipeline()))
            .is_some_and(|p| p.fail == 2)
        {
            RenderCommandResult::Skip
        } else {
            RenderCommandResult::Success
        }
    }
}
// This command MUST follow actual DrawMesh. Skip stops the enclosing tuple.
struct RecordZ;
impl<P: CachedRenderPipelinePhaseItem> RenderCommand<P> for RecordZ {
    type Param = (SRes<Ledger>, SRes<super::readiness::Control>);
    type ViewQuery = Entity;
    type ItemQuery = ();
    fn render<'w>(
        item: &P,
        view: Entity,
        _: Option<()>,
        (ledger, readiness): SystemParamItem<'w, '_, Self::Param>,
        _: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let mut log = ledger.0.lock().unwrap();
        readiness.submitted(item, view, log.stage == Stage::Motion);
        if log.stage == Stage::Z {
            let frame = log.frame;
            log.z.insert((view, frame, item.cached_pipeline()));
            log.ranges.insert(
                (view, item.cached_pipeline()),
                item.batch_range().len() as u32,
            );
            log.draws += 1;
            log.instances += item.batch_range().len() as u64;
            if matches!(
                item.extra_index(),
                PhaseItemExtraIndex::IndirectParametersIndex { .. }
            ) {
                log.indirect += 1;
            }
        }
        RenderCommandResult::Success
    }
}

fn readable_attachments(
    mut views: Query<(&ProbeView, &mut ViewPrepassTextures)>,
    device: Res<RenderDevice>,
    mut cache: ResMut<TextureCache>,
) {
    for (_, mut textures) in &mut views {
        let textures = &mut *textures;
        for (slot, is_motion) in [
            (&mut textures.normal, false),
            (&mut textures.motion_vectors, true),
        ] {
            if let Some(attachment) = slot {
                let texture = &attachment.texture.texture;
                let desc = TextureDescriptor {
                    label: Some("probe readable actual attachment"),
                    size: texture.size(),
                    mip_level_count: 1,
                    sample_count: texture.sample_count(),
                    dimension: TextureDimension::D2,
                    format: texture.format(),
                    usage: texture.usage() | TextureUsages::COPY_SRC,
                    view_formats: &[],
                };
                let t = cache.get(&device, desc);
                *attachment = ColorAttachment::new(
                    t,
                    None,
                    None,
                    Some(if is_motion {
                        LinearRgba::new(0.125, -0.25, 0.0, 0.0).into()
                    } else {
                        LinearRgba::BLACK.into()
                    }),
                );
            }
        }
    }
}

type ViewData = (
    &'static ExtractedView,
    &'static ProbeView,
    &'static ViewDepthStencilTexture,
    &'static ViewPrepassTextures,
);
// Exact stock query shape permits calling the retained stock function on other
// cameras. Their normal/motion/background behavior is not copied or replaced.
type StockViewData = (
    (
        &'static bevy::render::camera::ExtractedCamera,
        &'static ExtractedView,
        &'static ViewDepthStencilTexture,
        &'static ViewPrepassTextures,
        &'static bevy::render::view::ViewUniformOffset,
    ),
    (
        Option<&'static bevy::core_pipeline::prepass::DeferredPrepass>,
        Option<&'static bevy::core_pipeline::prepass::BackgroundMotionVectorsPipelineId>,
        Option<&'static bevy::core_pipeline::prepass::BackgroundMotionVectorsBindGroup>,
        Option<&'static bevy::core_pipeline::prepass::PreviousViewUniformOffset>,
        Option<&'static bevy::camera::MainPassResolutionOverride>,
    ),
    (
        Has<bevy::render::occlusion_culling::OcclusionCulling>,
        Has<bevy::render::view::NoIndirectDrawing>,
    ),
);
fn probe_early(
    world: &World,
    view: ViewQuery<StockViewData>,
    opaque: Res<ViewBinnedRenderPhases<Opaque3dPrepass>>,
    alpha: Res<ViewBinnedRenderPhases<AlphaMask3dPrepass>>,
    cache: Res<PipelineCache>,
    mut ctx: RenderContext,
) {
    let entity = view.entity();
    let Some(policy) = world.get::<ProbeView>(entity) else {
        early_prepass(world, view, opaque, alpha, cache, ctx);
        return;
    };
    let ((_, extracted, depth, textures, _), _, _) = view.into_inner();
    run(
        world,
        (entity, extracted, policy, depth, textures),
        &mut ctx,
        Stage::Z,
    );
}
fn probe_late(
    world: &World,
    view: ViewQuery<StockViewData>,
    opaque: Res<ViewBinnedRenderPhases<Opaque3dPrepass>>,
    alpha: Res<ViewBinnedRenderPhases<AlphaMask3dPrepass>>,
    cache: Res<PipelineCache>,
    mut ctx: RenderContext,
) {
    let entity = view.entity();
    let Some(policy) = world.get::<ProbeView>(entity) else {
        late_prepass(world, view, opaque, alpha, cache, ctx);
        return;
    };
    let ((_, extracted, depth, textures, _), _, (occlusion, direct)) = view.into_inner();
    if occlusion && !direct {
        run(
            world,
            (entity, extracted, policy, depth, textures),
            &mut ctx,
            Stage::Z,
        );
    }
}
fn motion(world: &World, view: ViewQuery<ViewData>, mut ctx: RenderContext) {
    let entity = view.entity();
    let (extracted, policy, depth, textures) = view.into_inner();
    run(
        world,
        (entity, extracted, policy, depth, textures),
        &mut ctx,
        Stage::Motion,
    );
}
fn capture_no_attachments(
    world: &World,
    view: ViewQuery<(
        &'static ExtractedView,
        &'static ProbeView,
        &'static ViewDepthStencilTexture,
        Option<&'static ViewPrepassTextures>,
    )>,
    mut ctx: RenderContext,
) {
    if super::arg("--attachments").as_deref() != Some("none") {
        return;
    }
    let entity = view.entity();
    let (extracted, policy, depth, textures) = view.into_inner();
    assert!(
        textures.is_none(),
        "no-attachment control must have no ViewPrepassTextures"
    );
    let opaque = world.resource::<ViewBinnedRenderPhases<Opaque3dPrepass>>();
    let alpha = world.resource::<ViewBinnedRenderPhases<AlphaMask3dPrepass>>();
    assert!(
        !opaque.contains_key(&extracted.retained_view_entity)
            && !alpha.contains_key(&extracted.retained_view_entity)
    );
    let frame = world.resource::<Plans>().epoch;
    if let Some(dir) = super::readback::directory(frame) {
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(format!(
                "v{}-f{frame}-no-attachments-state.txt",
                policy.number
            )),
            "view_prepass_textures=false\nopaque_prepass_phase=false\nalpha_prepass_phase=false\n",
        )
        .unwrap();
    }
    super::readback::capture(
        world,
        &mut ctx,
        (entity, policy.number),
        frame,
        "no-attachments",
        depth,
        None,
    );
}
fn run(
    world: &World,
    (entity, extracted, policy, depth, textures): (
        Entity,
        &ExtractedView,
        &ProbeView,
        &ViewDepthStencilTexture,
        &ViewPrepassTextures,
    ),
    ctx: &mut RenderContext,
    stage: Stage,
) {
    world.resource::<Ledger>().0.lock().unwrap().stage = stage;
    let frame = world.resource::<Plans>().epoch;
    if stage == Stage::Motion {
        super::readback::capture(
            world,
            ctx,
            (entity, policy.number),
            frame,
            "before-motion",
            depth,
            Some(textures),
        );
    }
    use bevy::render::diagnostic::RecordDiagnostics;
    let diagnostics = ctx.diagnostic_recorder();
    let recorder = diagnostics.as_deref();
    let span = recorder.time_span(
        ctx.command_encoder(),
        if stage == Stage::Z {
            "supplied_z"
        } else {
            "supplied_motion"
        },
    );
    let mut colours = vec![
        textures.normal.as_ref().map(|t| t.get_attachment()),
        textures.motion_vectors.as_ref().map(|t| t.get_attachment()),
        None,
        None,
    ];
    if colours.iter().all(Option::is_none) {
        colours.clear()
    }
    // First Z pass initializes the actual owned motion attachment; replay LOADS it.
    if stage == Stage::Motion {
        for attachment in colours.iter_mut().flatten() {
            attachment.ops.load = LoadOp::Load;
        }
    }
    let mut depth_attachment = depth.get_attachment(StoreOp::Store);
    if stage == Stage::Motion {
        // Read-only attachment also lets wgpu reject any accidental replay of
        // a depth-writing original pipeline. Split pipelines never write Z.
        depth_attachment.depth_ops = None;
    }
    let mut pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some(if stage == Stage::Z {
            "supplied Z"
        } else {
            "split motion Load"
        }),
        color_attachments: &colours,
        depth_stencil_attachment: Some(depth_attachment),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    // Camera viewport is shared by all stages (required for Equal).
    if let Some(camera) = world.get::<bevy::render::camera::ExtractedCamera>(entity)
        && let Some(v) = &camera.viewport
    {
        pass.set_camera_viewport(v);
    }
    if let Some(phase) = world
        .resource::<ViewBinnedRenderPhases<Opaque3dPrepass>>()
        .get(&extracted.retained_view_entity)
    {
        phase.render(&mut pass, world, entity).unwrap();
    }
    if let Some(phase) = world
        .resource::<ViewBinnedRenderPhases<AlphaMask3dPrepass>>()
        .get(&extracted.retained_view_entity)
    {
        phase.render(&mut pass, world, entity).unwrap();
    }
    if stage == Stage::Z && frame == 360 {
        let mut witness = world.resource::<WitnessState>().0.lock().unwrap();
        witness.prepare(world);
        let result = witness.draw(world, &mut pass, entity, &WitnessItem { range: 0..1 });
        assert!(result.is_ok());
        assert_eq!(world.resource::<Ledger>().0.lock().unwrap().witness, 0);
        info!(
            "native-pass-probe: actual DrawMesh missing-mesh => draw Ok, trailing recorder 0 (view {})",
            policy.number
        );
    }
    drop(pass);
    span.end(ctx.command_encoder());
    super::readback::capture(
        world,
        ctx,
        (entity, policy.number),
        frame,
        if stage == Stage::Z { "z" } else { "motion" },
        depth,
        Some(textures),
    );
    if stage == Stage::Motion
        && let Some(dir) = super::readback::directory(frame)
    {
        let plans = world.resource::<Plans>();
        let log = world.resource::<Ledger>().0.lock().unwrap();
        let cache = world.resource::<PipelineCache>();
        let mut rows = vec!["group,paired_ready,submitted_z,instance_range\n".to_string()];
        for ((view, group, _, _, _, _), pair) in &plans.pairs {
            if *view == extracted.retained_view_entity {
                rows.push(format!(
                    "{group},{},{},{}\n",
                    pair_ready(pair, cache),
                    log.z.contains(&(entity, frame, pair.z)),
                    log.ranges.get(&(entity, pair.z)).copied().unwrap_or(0)
                ));
            }
        }
        std::fs::write(
            dir.join(format!("v{}-f{frame}-state.csv", policy.number)),
            rows.concat(),
        )
        .unwrap();
        let readiness = world.resource::<super::readiness::Control>();
        readiness.write(
            entity,
            policy.number,
            cache,
            readiness
                .fresh()
                .and_then(|id| plans.stock.get(&id).copied()),
        );
    }
    if stage == Stage::Z
        && let Some(copy) = &textures.depth
    {
        ctx.command_encoder().copy_texture_to_texture(
            depth.texture().as_image_copy(),
            copy.texture.texture.as_image_copy(),
            textures.size,
        );
    }
}

struct WitnessItem {
    range: std::ops::Range<u32>,
}
impl PhaseItem for WitnessItem {
    fn entity(&self) -> Entity {
        Entity::PLACEHOLDER
    }
    fn main_entity(&self) -> MainEntity {
        Entity::PLACEHOLDER.into()
    }
    fn draw_function(&self) -> DrawFunctionId {
        DrawFunctionId(0)
    }
    fn batch_range(&self) -> &std::ops::Range<u32> {
        &self.range
    }
    fn batch_range_mut(&mut self) -> &mut std::ops::Range<u32> {
        &mut self.range
    }
    fn extra_index(&self) -> PhaseItemExtraIndex {
        PhaseItemExtraIndex::None
    }
    fn batch_range_and_extra_index_mut(
        &mut self,
    ) -> (&mut std::ops::Range<u32>, &mut PhaseItemExtraIndex) {
        unreachable!("witness is never queued/batched")
    }
}
#[derive(Resource)]
struct WitnessState(Mutex<RenderCommandState<WitnessItem, (DrawMesh, RecordWitness)>>);
struct RecordWitness;
impl RenderCommand<WitnessItem> for RecordWitness {
    type Param = SRes<Ledger>;
    type ViewQuery = ();
    type ItemQuery = ();
    fn render<'w>(
        _: &WitnessItem,
        _: (),
        _: Option<()>,
        ledger: SystemParamItem<'w, '_, Self::Param>,
        _: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        ledger.0.lock().unwrap().witness += 1;
        RenderCommandResult::Success
    }
}
