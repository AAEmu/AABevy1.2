//! Native stars -> alpha moons -> additive atmosphere -> alpha sky box.
//! The calibrated atmosphere and box RGB are unchanged. Their combined RGB already
//! includes box-alpha weighting; ONE/INV_SRC_ALPHA composes both in one draw.

use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::RenderApp;
use bevy::render::batching::NoAutomaticBatching;
use bevy::render::extract_component::ExtractComponent;
use bevy::render::render_resource::{
    AsBindGroup, CompareFunction, RenderPipelineDescriptor, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;

use super::{SkyMaterial, SkyParams};
use crate::sky::SkyDome;

#[path = "sky_cloud_water.rs"]
pub mod cloud_water;

/// CPU sorting only, after catalogue/moons and before distant cloud sprites.
pub const SKY_COMPOSITE_SORT_BIAS: f32 = crate::night_sky::SKY_SORT_BIAS + 75_000.0;

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug, PartialEq)]
pub struct SkyCompositeMaterial {
    #[uniform(0)]
    pub params: SkyParams,
    #[texture(1)]
    #[sampler(2)]
    pub box_12: Option<Handle<Image>>,
    #[texture(3)]
    #[sampler(4)]
    pub box_34: Option<Handle<Image>>,
    #[texture(5)]
    #[sampler(6)]
    pub box_5: Option<Handle<Image>>,
}

impl From<&SkyMaterial> for SkyCompositeMaterial {
    fn from(source: &SkyMaterial) -> Self {
        Self {
            params: source.params,
            box_12: source.box_12.clone(),
            box_34: source.box_34.clone(),
            box_5: source.box_5.clone(),
        }
    }
}

impl Material for SkyCompositeMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://aa_world/sky_background.wesl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "embedded://aa_world/sky.wesl".into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Premultiplied
    }
    fn depth_bias(&self) -> f32 {
        SKY_COMPOSITE_SORT_BIAS
    }
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        if let Some(depth) = &mut descriptor.depth_stencil {
            depth.depth_write_enabled = Some(false);
            depth.depth_compare = Some(CompareFunction::GreaterEqual);
        }
        Ok(())
    }
}

/// One composite draw using the source dome mesh/transform and borrowed sky textures.
#[derive(Component)]
pub struct SkyComposite;

/// Celestial/atmosphere draw before the water colour copy. Distant cloud sprites
/// use their own native per-view water-half classifier; foreground receivers stay late.
#[derive(Component, Clone, Copy, ExtractComponent)]
#[extract_app(RenderApp)]
#[require(NoAutomaticBatching)]
pub struct SkyPass;

/// Existing diffuse overlays for native opaque deferred decals, before fog/water.
/// Legacy forward decals do not carry this marker.
#[derive(Component, Clone, Copy, ExtractComponent)]
#[extract_app(RenderApp)]
#[require(NoAutomaticBatching)]
pub struct DeferredDecalPass;

/// Native ordinary Illum type13 roads (list5), before deferred overlays/fog/water.
/// The placement producer qualifies this marker; sharing RoadMaterial is insufficient.
#[derive(Component, Clone, Copy, ExtractComponent)]
#[extract_app(RenderApp)]
#[require(NoAutomaticBatching)]
pub struct RoadSurfacePass;

struct Draw {
    parent: Entity,
    entity: Entity,
    material: Handle<SkyCompositeMaterial>,
}
#[derive(Resource, Default)]
struct State {
    draws: Vec<Draw>,
}

pub(super) fn build(app: &mut App) {
    app.add_plugins(MaterialPlugin::<SkyCompositeMaterial>::default())
        .init_resource::<State>()
        .add_systems(
            PostUpdate,
            sync.before(bevy::transform::TransformSystems::Propagate),
        );
    cloud_water::build(app);
    pass::build(app);
}

fn sync(
    mut commands: Commands,
    sources: Query<
        (
            Entity,
            &Mesh3d,
            &MeshMaterial3d<SkyMaterial>,
            Option<&RenderLayers>,
        ),
        With<SkyDome>,
    >,
    mut children: Query<(&mut Mesh3d, &mut RenderLayers), Without<SkyDome>>,
    source_materials: Res<Assets<SkyMaterial>>,
    mut materials: ResMut<Assets<SkyCompositeMaterial>>,
    mut state: ResMut<State>,
) {
    state.draws.retain(|draw| {
        let keep = sources
            .get(draw.parent)
            .is_ok_and(|(_, _, handle, _)| source_materials.contains(handle.id()));
        if !keep {
            commands.entity(draw.entity).try_despawn();
            materials.remove(draw.material.id());
        }
        keep
    });
    for (parent, mesh, handle, layers) in &sources {
        let Some(source) = source_materials.get(handle.id()) else {
            continue;
        };
        let value = SkyCompositeMaterial::from(source);
        if let Some(draw) = state.draws.iter().find(|draw| draw.parent == parent) {
            if materials.get(&draw.material) != Some(&value) {
                materials
                    .insert(draw.material.id(), value)
                    .expect("live sky material");
            }
            if let Ok((mut child_mesh, mut child_layers)) = children.get_mut(draw.entity) {
                if child_mesh.0 != mesh.0 {
                    child_mesh.0 = mesh.0.clone();
                }
                let target_layers = layers.cloned().unwrap_or_default();
                if *child_layers != target_layers {
                    *child_layers = target_layers;
                }
            }
        } else {
            let material = materials.add(value);
            let entity = commands
                .spawn((
                    SkyComposite,
                    SkyPass,
                    mesh.clone(),
                    MeshMaterial3d(material.clone()),
                    Transform::default(),
                    ChildOf(parent),
                    layers.cloned().unwrap_or_default(),
                    NoFrustumCulling,
                    NotShadowCaster,
                    NotShadowReceiver,
                ))
                .id();
            state.draws.push(Draw {
                parent,
                entity,
                material,
            });
        }
    }
}

mod pass {
    use super::{DeferredDecalPass, RoadSurfacePass, SkyPass};
    use super::cloud_water::{CloudWaterPass, OceanLevel, after_water};
    use crate::fog::FogWaterPass;
    use bevy::camera::{MainPassResolutionOverride, Viewport};
    use bevy::core_pipeline::{
        Core3dSystems,
        core_3d::{Transparent3d, main_opaque_pass_3d},
        schedule::Core3d,
    };
    use bevy::pbr::main_transmissive_pass_3d;
    use bevy::prelude::*;
    use bevy::render::{
        Render, RenderApp, RenderSystems,
        camera::ExtractedCamera,
        extract_component::ExtractComponentPlugin,
        render_phase::{SortedRenderPhase, ViewSortedRenderPhases},
        render_resource::{RenderPassDescriptor, StoreOp},
        renderer::{RenderContext, ViewQuery},
        sync_world::{MainEntity, MainEntityHashMap, MainEntityHashSet},
        view::{ExtractedView, RetainedViewEntity, ViewDepthStencilTexture, ViewTarget},
    };
    use std::collections::{HashMap, HashSet};

    #[derive(Resource, Default)]
    struct SkyPhases(ViewSortedRenderPhases<Transparent3d>);

    pub(super) fn build(app: &mut App) {
        app.add_plugins((
            ExtractComponentPlugin::<SkyPass>::default(),
            ExtractComponentPlugin::<DeferredDecalPass>::default(),
            ExtractComponentPlugin::<RoadSurfacePass>::default(),
        ))
            .add_systems(
                PostUpdate,
                tag.before(bevy::transform::TransformSystems::Propagate),
            );
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<SkyPhases>()
            .add_systems(
                Render,
                (
                    restore
                        .in_set(RenderSystems::Queue)
                        .before(RenderSystems::QueueMeshes),
                    prepare
                        .after(RenderSystems::Prepare)
                        .before(RenderSystems::Render),
                ),
            )
            .add_systems(
                Core3d,
                draw.after(main_opaque_pass_3d)
                    .before(main_transmissive_pass_3d)
                    .in_set(Core3dSystems::MainPass),
            );
    }

    fn tag(
        mut commands: Commands,
        sky: Query<
            Entity,
            (
                Or<(
                    With<MeshMaterial3d<crate::night_sky::StarMaterial>>,
                    With<MeshMaterial3d<crate::moons::MoonMaterial>>,
                )>,
                Without<SkyPass>,
            ),
        >,
        clouds: Query<
            Entity,
            (
                With<crate::sky_clouds::CloudSprite>,
                Without<CloudWaterPass>,
            ),
        >,
    ) {
        for entity in &sky {
            commands.entity(entity).insert(SkyPass);
        }
        for entity in &clouds {
            commands.entity(entity).insert(CloudWaterPass);
        }
    }

    // Phase items are retained by Bevy. Restore only the range that we suppressed,
    // before queue/sweep/batching reads them again; GPU offsets are never rewritten.
    fn restore(
        mut sky: ResMut<SkyPhases>,
        mut late: ResMut<ViewSortedRenderPhases<Transparent3d>>,
    ) {
        for (view, phase) in sky.0.iter_mut() {
            if let Some(target) = late.get_mut(view) {
                for (key, saved) in &phase.items {
                    if let Some(item) = target.items.get_mut(key) {
                        item.batch_range = saved.batch_range.clone();
                    }
                }
            }
            phase.clear();
        }
        sky.0.retain(|view, _| late.contains_key(view));
    }

    fn prepared_copy(item: &Transparent3d) -> Transparent3d {
        Transparent3d {
            sorting_info: item.sorting_info.clone(),
            distance: item.distance,
            pipeline: item.pipeline,
            entity: item.entity,
            draw_function: item.draw_function,
            batch_range: item.batch_range.clone(),
            extra_index: item.extra_index.clone(),
            indexed: item.indexed,
        }
    }

    fn prepare(
        mut sky: ResMut<SkyPhases>,
        mut late: ResMut<ViewSortedRenderPhases<Transparent3d>>,
        tagged: Query<&MainEntity, With<SkyPass>>,
        clouds: Query<&MainEntity, With<CloudWaterPass>>,
        fog: Query<(&MainEntity, &FogWaterPass)>,
        decals: Query<&MainEntity, With<DeferredDecalPass>>,
        roads: Query<&MainEntity, With<RoadSurfacePass>>,
        views: Query<&ExtractedView, With<Camera3d>>,
        ocean: Res<OceanLevel>,
        mut entities: Local<MainEntityHashSet>,
        mut cloud_entities: Local<MainEntityHashSet>,
        mut before_water: Local<HashSet<RetainedViewEntity>>,
        mut fog_entities: Local<MainEntityHashMap<f32>>,
        mut normal_views: Local<HashMap<RetainedViewEntity, f32>>,
        mut decal_entities: Local<MainEntityHashSet>,
    ) {
        // Bevy 0.20 PBR items use (PLACEHOLDER, MainEntity), rather than the
        // extracted render entity. Retain this small set's capacity across frames.
        entities.clear();
        entities.extend(tagged.iter().copied());
        cloud_entities.clear();
        cloud_entities.extend(clouds.iter().copied());
        before_water.clear();
        fog_entities.clear();
        fog_entities.extend(
            fog.iter()
                .filter(|(_, p)| p.center_height.is_finite())
                .map(|(entity, p)| (*entity, p.center_height)),
        );
        normal_views.clear();
        decal_entities.clear();
        decal_entities.extend(decals.iter().copied());
        decal_entities.extend(roads.iter().copied());
        for view in &views {
            let height = view.world_from_view.translation().y;
            // Native recursive/reflection pass IDs have no port input. Decline
            // mirrored views instead of guessing that their pass ID is zero.
            if !view.invert_culling && height.is_finite() {
                normal_views.insert(view.retained_view_entity, height);
                if ocean.0.is_some_and(|level| !after_water(height, level)) {
                    before_water.insert(view.retained_view_entity);
                }
            }
        }
        for (view, source) in late.iter_mut() {
            let target = sky
                .0
                .entry(*view)
                .or_insert_with(SortedRenderPhase::default);
            let fog_view = normal_views.get(view).copied().zip(ocean.0);
            for item in source.items.values_mut() {
                if entities.contains(&item.entity.1)
                    || (normal_views.contains_key(view) && decal_entities.contains(&item.entity.1))
                    || (before_water.contains(view) && cloud_entities.contains(&item.entity.1))
                    || fog_view.is_some_and(|(eye, level)| {
                        fog_entities.get(&item.entity.1)
                            .is_some_and(|height| !crate::fog::after_water(eye, level, *height))
                    })
                {
                    // Each marker requires NoAutomaticBatching: a range cannot include a
                    // foreground receiver. Preserve the already-prepared GPU instance index.
                    debug_assert!(item.batch_range.len() <= 1, "batched sky draw");
                    target.add_retained(prepared_copy(item));
                    item.batch_range.end = item.batch_range.start;
                }
            }
        }
    }

    fn draw(
        world: &World,
        view: ViewQuery<(
            &ExtractedCamera,
            &ExtractedView,
            &ViewTarget,
            &ViewDepthStencilTexture,
            Option<&MainPassResolutionOverride>,
        )>,
        sky: Res<SkyPhases>,
        mut ctx: RenderContext,
    ) {
        let entity = view.entity();
        let (camera, extracted, target, depth, resolution) = view.into_inner();
        let Some(phase) = sky.0.get(&extracted.retained_view_entity) else {
            return;
        };
        if phase.items.is_empty() {
            return;
        }
        let mut pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
            label: Some("aa_world_sky_before_water"),
            color_attachments: &[Some(target.get_color_attachment())],
            depth_stencil_attachment: Some(depth.get_attachment(StoreOp::Store)),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        if let Some(viewport) =
            Viewport::from_viewport_and_override(camera.viewport.as_ref(), resolution)
        {
            pass.set_camera_viewport(&viewport);
        }
        if let Err(error) = phase.render(&mut pass, world, entity) {
            error!("Sky before water draw failed: {error:?}");
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use bevy::core_pipeline::core_3d::TransparentSortingInfo3d;
        use bevy::render::{
            batching::NoAutomaticBatching,
            render_phase::{
                Draw, DrawError, DrawFunctions, PhaseItemExtraIndex, TrackedRenderPass,
            },
            render_resource::CachedRenderPipelineId,
            view::RetainedViewEntity,
        };

        struct UnusedDraw;
        impl Draw<Transparent3d> for UnusedDraw {
            fn draw<'w>(
                &mut self,
                _: &'w World,
                _: &mut TrackedRenderPass<'w>,
                _: Entity,
                _: &Transparent3d,
            ) -> Result<(), DrawError> {
                Ok(())
            }
        }

        #[test]
        fn prepared_offsets_restore_across_frames_and_views_retire_without_leaking() {
            let mut app = App::new();
            app.init_resource::<SkyPhases>()
                .init_resource::<OceanLevel>()
                .init_resource::<ViewSortedRenderPhases<Transparent3d>>()
                .add_systems(Update, (restore, prepare).chain());
            let sky = app.world_mut().spawn_empty().id();
            let marker = app.world_mut().spawn((SkyPass, MainEntity::from(sky))).id();
            let glass = app.world_mut().spawn_empty().id();
            let a = app.world_mut().spawn_empty().id();
            let b = app.world_mut().spawn_empty().id();
            let a = RetainedViewEntity::new(a.into(), None, 0);
            let b = RetainedViewEntity::new(b.into(), None, 0);
            let functions = DrawFunctions::<Transparent3d>::default();
            let draw = functions.write().add(UnusedDraw);
            let item = |entity: Entity, start| Transparent3d {
                sorting_info: TransparentSortingInfo3d::Sorted {
                    mesh_center: Vec3::ZERO,
                    depth_bias: 0.,
                },
                distance: 10.,
                pipeline: CachedRenderPipelineId::INVALID,
                entity: (Entity::PLACEHOLDER, entity.into()),
                draw_function: draw,
                batch_range: start..start + 1,
                extra_index: PhaseItemExtraIndex::DynamicOffset(512),
                indexed: true,
            };
            {
                let mut phases = app
                    .world_mut()
                    .resource_mut::<ViewSortedRenderPhases<Transparent3d>>();
                let phase = phases.entry(a).or_default();
                phase.add_retained(item(sky, 41));
                phase.add_retained(item(glass, 80));
                // The sky was not queued for the second view (visibility/layers).
                phases.entry(b).or_default().add_retained(item(glass, 93));
            }
            for _ in 0..4 {
                app.update();
                let phases = app
                    .world()
                    .resource::<ViewSortedRenderPhases<Transparent3d>>();
                assert_eq!(
                    phases[&a].items[&(Entity::PLACEHOLDER, sky.into())].batch_range,
                    41..41
                );
                assert_eq!(
                    phases[&a].items[&(Entity::PLACEHOLDER, glass.into())].batch_range,
                    80..81
                );
                assert_eq!(
                    phases[&b].items[&(Entity::PLACEHOLDER, glass.into())].batch_range,
                    93..94
                );
                let early = app.world().resource::<SkyPhases>();
                let saved = &early.0[&a].items[&(Entity::PLACEHOLDER, sky.into())];
                assert_eq!(saved.batch_range, 41..42);
                assert!(matches!(
                    saved.extra_index,
                    PhaseItemExtraIndex::DynamicOffset(512)
                ));
                assert!(saved.indexed);
                assert_eq!(early.0[&a].items.len(), 1);
                assert!(early.0[&b].items.is_empty());
            }
            app.world_mut().entity_mut(marker).remove::<SkyPass>();
            app.update();
            assert!(app.world().resource::<SkyPhases>().0[&a].items.is_empty());
            assert_eq!(
                app.world()
                    .resource::<ViewSortedRenderPhases<Transparent3d>>()[&a]
                    .items[&(Entity::PLACEHOLDER, sky.into())]
                    .batch_range,
                41..42
            );
            app.world_mut()
                .resource_mut::<ViewSortedRenderPhases<Transparent3d>>()
                .remove(&a);
            app.update();
            assert!(!app.world().resource::<SkyPhases>().0.contains_key(&a));
        }

        #[test]
        fn fog_halves_are_per_volume_and_view_and_restore_prepared_ranges() {
            use bevy::render::render_resource::TextureFormat;
            let mut app = App::new();
            app.init_resource::<SkyPhases>()
                .insert_resource(OceanLevel(Some(100.0)))
                .init_resource::<ViewSortedRenderPhases<Transparent3d>>()
                .add_systems(Update, (restore, prepare).chain());
            let sources: Vec<_> = (0..5).map(|_| app.world_mut().spawn_empty().id()).collect();
            let markers: Vec<_> = [90.0, 100.0, 110.0]
                .into_iter()
                .zip(&sources)
                .map(|(height, source)| {
                    app.world_mut()
                        .spawn((
                            MainEntity::from(*source),
                            FogWaterPass {
                                center_height: height,
                            },
                        ))
                        .id()
                })
                .collect();
            let overlay = app
                .world_mut()
                .spawn((MainEntity::from(sources[3]), DeferredDecalPass))
                .id();
            assert!(app.world().get::<NoAutomaticBatching>(overlay).is_some());
            let keys: Vec<_> = (0..2)
                .map(|_| {
                    RetainedViewEntity::new(app.world_mut().spawn_empty().id().into(), None, 0)
                })
                .collect();
            let views: Vec<_> = [90.0, 110.0]
                .into_iter()
                .zip(&keys)
                .map(|(height, key)| {
                    app.world_mut()
                        .spawn((
                            Camera3d::default(),
                            ExtractedView {
                                retained_view_entity: *key,
                                clip_from_view: Mat4::IDENTITY,
                                world_from_view: GlobalTransform::from(Transform::from_xyz(
                                    0.0, height, 0.0,
                                )),
                                clip_from_world: None,
                                target_format: TextureFormat::Rgba8Unorm,
                                viewport: UVec4::new(0, 0, 128, 128),
                                color_grading: default(),
                                invert_culling: false,
                            },
                        ))
                        .id()
                })
                .collect();
            let functions = DrawFunctions::<Transparent3d>::default();
            let draw = functions.write().add(UnusedDraw);
            for key in &keys {
                // Prepared native opaque-overlay draw precedes fog; the final item is foreground alpha.
                for i in [3, 0, 1, 2, 4] {
                    let source = sources[i];
                    app.world_mut()
                        .resource_mut::<ViewSortedRenderPhases<Transparent3d>>()
                        .entry(*key)
                        .or_default()
                        .add_retained(Transparent3d {
                            sorting_info: TransparentSortingInfo3d::Sorted {
                                mesh_center: Vec3::new(0.0, 500.0, 0.0),
                                depth_bias: if i == 3 { -500_000.0 } else { -300_000.0 },
                            },
                            distance: 10.0,
                            pipeline: CachedRenderPipelineId::INVALID,
                            entity: (Entity::PLACEHOLDER, source.into()),
                            draw_function: draw,
                            batch_range: i as u32 + 40..i as u32 + 41,
                            extra_index: PhaseItemExtraIndex::DynamicOffset(1536),
                            indexed: true,
                        });
                }
            }
            // Original normal-pass goldens, opaque overlay and untagged foreground item.
            for (heights, expected) in [
                (
                    [90.0, 110.0],
                    [
                        [false, true, true, true, false],
                        [true, true, false, true, false],
                    ],
                ),
                (
                    [110.0, 90.0],
                    [
                        [true, true, false, true, false],
                        [false, true, true, true, false],
                    ],
                ),
            ] {
                for (view, height) in views.iter().zip(heights) {
                    app.world_mut()
                        .get_mut::<ExtractedView>(*view)
                        .unwrap()
                        .world_from_view =
                        GlobalTransform::from(Transform::from_xyz(0.0, height, 0.0));
                }
                for _ in 0..3 {
                    app.update();
                    for (view, selected) in keys.iter().zip(expected) {
                        let early = &app.world().resource::<SkyPhases>().0[view];
                        assert_eq!(
                            early.items.get_index(0).unwrap().1.entity.1,
                            sources[3].into()
                        );
                        let late = &app
                            .world()
                            .resource::<ViewSortedRenderPhases<Transparent3d>>()[view];
                        for (i, source) in sources.iter().enumerate() {
                            let key = (Entity::PLACEHOLDER, (*source).into());
                            assert_eq!(early.items.contains_key(&key), selected[i]);
                            let range = i as u32 + 40..i as u32 + 41;
                            let item = if selected[i] {
                                &early.items[&key]
                            } else {
                                &late.items[&key]
                            };
                            assert_eq!(item.batch_range, range);
                            let expected_bias = if i == 3 { -500_000.0 } else { -300_000.0 };
                            assert!(matches!(item.sorting_info,
                                TransparentSortingInfo3d::Sorted { depth_bias, .. }
                                    if depth_bias == expected_bias));
                            assert!(matches!(
                                item.extra_index,
                                PhaseItemExtraIndex::DynamicOffset(1536)
                            ));
                        }
                    }
                }
            }
            app.world_mut()
                .get_mut::<ExtractedView>(views[0])
                .unwrap()
                .invert_culling = true;
            app.update();
            assert!(
                app.world().resource::<SkyPhases>().0[&keys[0]]
                    .items
                    .is_empty()
            );
            assert!(
                !app.world().resource::<SkyPhases>().0[&keys[1]]
                    .items
                    .is_empty()
            );
            // Ocean loss restores fog; native opaque decals still precede water independently.
            app.world_mut().resource_mut::<OceanLevel>().0 = None;
            app.update();
            assert!(
                app.world().resource::<SkyPhases>().0[&keys[0]]
                    .items
                    .is_empty()
            );
            assert_eq!(
                app.world().resource::<SkyPhases>().0[&keys[1]].items.len(),
                1
            );
            for marker in markers {
                app.world_mut().entity_mut(marker).despawn();
            }
            app.world_mut().entity_mut(overlay).despawn();
            app.world_mut().resource_mut::<OceanLevel>().0 = Some(100.0);
            app.update();
            for view in keys {
                let late = &app
                    .world()
                    .resource::<ViewSortedRenderPhases<Transparent3d>>()[&view];
                assert!(late.items.values().all(|item| item.batch_range.len() == 1));
                assert!(
                    app.world().resource::<SkyPhases>().0[&view]
                        .items
                        .is_empty()
                );
            }
        }

        #[test]
        fn ordinary_roads_keep_prepared_offsets_and_exclude_legacy_and_mirror_views() {
            use bevy::render::render_resource::TextureFormat;
            let mut app = App::new();
            app.init_resource::<SkyPhases>()
                .init_resource::<OceanLevel>()
                .init_resource::<ViewSortedRenderPhases<Transparent3d>>()
                .add_systems(Update, (restore, prepare).chain());
            let road = app.world_mut().spawn_empty().id();
            let legacy = app.world_mut().spawn_empty().id();
            let marker = app.world_mut().spawn((RoadSurfacePass, MainEntity::from(road))).id();
            assert!(app.world().get::<NoAutomaticBatching>(marker).is_some());
            let functions = DrawFunctions::<Transparent3d>::default();
            let draw = functions.write().add(UnusedDraw);
            let views: Vec<_> = [false, true].into_iter().map(|mirror| {
                let key = RetainedViewEntity::new(app.world_mut().spawn_empty().id().into(), None, 0);
                app.world_mut().spawn((Camera3d::default(), ExtractedView {
                    retained_view_entity: key, clip_from_view: Mat4::IDENTITY,
                    world_from_view: GlobalTransform::IDENTITY, clip_from_world: None,
                    target_format: TextureFormat::Rgba8Unorm, viewport: UVec4::new(0,0,128,128),
                    color_grading: default(), invert_culling: mirror,
                }));
                let mut phases = app.world_mut().resource_mut::<ViewSortedRenderPhases<Transparent3d>>();
                let phase = phases.entry(key).or_default();
                for (entity, start, bias) in [(road, 19, -600_000.0), (legacy, 24, 17.0)] {
                    phase.add_retained(Transparent3d {
                        sorting_info: TransparentSortingInfo3d::Sorted { mesh_center: Vec3::ZERO, depth_bias: bias },
                        distance: bias, pipeline: CachedRenderPipelineId::INVALID,
                        entity: (Entity::PLACEHOLDER, entity.into()), draw_function: draw,
                        batch_range: start..start+1, extra_index: PhaseItemExtraIndex::DynamicOffset(768), indexed: true,
                    });
                }
                key
            }).collect();
            for _ in 0..3 {
                app.update();
                let early = app.world().resource::<SkyPhases>();
                let late = app.world().resource::<ViewSortedRenderPhases<Transparent3d>>();
                let key = (Entity::PLACEHOLDER, road.into());
                let selected = &early.0[&views[0]].items[&key];
                assert_eq!(selected.batch_range, 19..20);
                assert!(matches!(selected.extra_index, PhaseItemExtraIndex::DynamicOffset(768)));
                assert_eq!(late[&views[0]].items[&key].batch_range, 19..19);
                assert!(early.0[&views[1]].items.is_empty());
                for view in &views {
                    assert_eq!(late[view].items[&(Entity::PLACEHOLDER, legacy.into())].batch_range,24..25);
                }
            }
            app.world_mut().despawn(marker);
            app.update();
            for view in views {
                assert!(app.world().resource::<SkyPhases>().0[&view].items.is_empty());
                assert_eq!(app.world().resource::<ViewSortedRenderPhases<Transparent3d>>()[&view]
                    .items[&(Entity::PLACEHOLDER, road.into())].batch_range, 19..20);
            }
        }

        #[test]
        fn cloud_halves_change_per_view_and_restore_on_crossing_or_ocean_loss() {
            use bevy::render::render_resource::TextureFormat;
            let mut app = App::new();
            app.init_resource::<SkyPhases>()
                .insert_resource(OceanLevel(Some(80.0)))
                .init_resource::<ViewSortedRenderPhases<Transparent3d>>()
                .add_systems(Update, (restore, prepare).chain());
            let cloud = app.world_mut().spawn_empty().id();
            let marker = app
                .world_mut()
                .spawn((CloudWaterPass, MainEntity::from(cloud)))
                .id();
            let a = RetainedViewEntity::new(app.world_mut().spawn_empty().id().into(), None, 0);
            let b = RetainedViewEntity::new(app.world_mut().spawn_empty().id().into(), None, 0);
            let view = |id, height| ExtractedView {
                retained_view_entity: id,
                clip_from_view: Mat4::IDENTITY,
                world_from_view: GlobalTransform::from(Transform::from_xyz(0.0, height, 0.0)),
                clip_from_world: None,
                target_format: TextureFormat::Rgba8Unorm,
                viewport: UVec4::new(0, 0, 128, 128),
                color_grading: default(),
                invert_culling: false,
            };
            let camera_a = app
                .world_mut()
                .spawn((Camera3d::default(), view(a, 90.0)))
                .id();
            let camera_b = app
                .world_mut()
                .spawn((Camera3d::default(), view(b, 70.0)))
                .id();
            let functions = DrawFunctions::<Transparent3d>::default();
            let draw = functions.write().add(UnusedDraw);
            for (key, start) in [(a, 41), (b, 81)] {
                app.world_mut()
                    .resource_mut::<ViewSortedRenderPhases<Transparent3d>>()
                    .entry(key)
                    .or_default()
                    .add_retained(Transparent3d {
                        sorting_info: TransparentSortingInfo3d::Sorted {
                            mesh_center: Vec3::new(0.0, 550.0, 0.0),
                            depth_bias: -900_000.0,
                        },
                        distance: 10.0,
                        pipeline: CachedRenderPipelineId::INVALID,
                        entity: (Entity::PLACEHOLDER, cloud.into()),
                        draw_function: draw,
                        batch_range: start..start + 1,
                        extra_index: PhaseItemExtraIndex::DynamicOffset(1024),
                        indexed: true,
                    });
            }
            let early =
                |app: &App, key| app.world().resource::<SkyPhases>().0[&key].items.len() == 1;
            app.update();
            assert!(!early(&app, a));
            assert!(early(&app, b));
            // A shared source cloud must change halves independently in each view.
            app.world_mut()
                .get_mut::<ExtractedView>(camera_a)
                .unwrap()
                .world_from_view = GlobalTransform::from(Transform::from_xyz(0.0, 70.0, 0.0));
            app.world_mut()
                .get_mut::<ExtractedView>(camera_b)
                .unwrap()
                .world_from_view = GlobalTransform::from(Transform::from_xyz(0.0, 90.0, 0.0));
            app.update();
            assert!(early(&app, a));
            assert!(!early(&app, b));
            app.world_mut().resource_mut::<OceanLevel>().0 = None;
            app.update();
            assert!(!early(&app, a) && !early(&app, b));
            for (key, start) in [(a, 41), (b, 81)] {
                let phases = app
                    .world()
                    .resource::<ViewSortedRenderPhases<Transparent3d>>();
                assert_eq!(
                    phases[&key].items[&(Entity::PLACEHOLDER, cloud.into())].batch_range,
                    start..start + 1
                );
            }
            app.world_mut().resource_mut::<OceanLevel>().0 = Some(100.0);
            app.update();
            assert!(early(&app, a) && early(&app, b));
            app.world_mut()
                .get_mut::<ExtractedView>(camera_a)
                .unwrap()
                .invert_culling = true;
            app.update();
            assert!(!early(&app, a) && early(&app, b));
            app.world_mut()
                .entity_mut(marker)
                .remove::<CloudWaterPass>();
            app.update();
            assert!(!early(&app, a) && !early(&app, b));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_updates_rebinds_and_unloads_with_its_source() {
        let mut app = App::new();
        app.init_resource::<Assets<SkyMaterial>>()
            .init_resource::<Assets<SkyCompositeMaterial>>()
            .init_resource::<State>()
            .add_systems(Update, sync);
        let source = app
            .world_mut()
            .resource_mut::<Assets<SkyMaterial>>()
            .add(SkyMaterial {
                params: default(),
                box_12: None,
                box_34: None,
                box_5: None,
            });
        let parent = app
            .world_mut()
            .spawn((SkyDome, Mesh3d::default(), MeshMaterial3d(source.clone())))
            .id();
        app.update();
        let (child, material) = {
            let state = app.world().resource::<State>();
            assert_eq!(state.draws.len(), 1);
            (state.draws[0].entity, state.draws[0].material.clone())
        };
        assert_eq!(app.world().get::<ChildOf>(child).unwrap().parent(), parent);
        let mut params = SkyParams::default();
        params.horizon.x = 3.5;
        app.world_mut()
            .resource_mut::<Assets<SkyMaterial>>()
            .get_mut(&source)
            .unwrap()
            .params = params;
        app.update();
        assert_eq!(
            app.world()
                .resource::<Assets<SkyCompositeMaterial>>()
                .get(&material)
                .unwrap()
                .params,
            params
        );
        assert_eq!(
            app.world().resource::<Assets<SkyCompositeMaterial>>().len(),
            1
        );
        let replacement = SkyMaterial {
            params: SkyParams {
                box_sky: Vec4::ONE,
                ..params
            },
            box_12: None,
            box_34: None,
            box_5: Some(Handle::default()),
        };
        let expected = SkyCompositeMaterial::from(&replacement);
        let rebound = app
            .world_mut()
            .resource_mut::<Assets<SkyMaterial>>()
            .add(replacement);
        app.world_mut()
            .get_mut::<MeshMaterial3d<SkyMaterial>>(parent)
            .unwrap()
            .0 = rebound;
        app.update();
        assert_eq!(
            app.world()
                .resource::<Assets<SkyCompositeMaterial>>()
                .get(&material),
            Some(&expected)
        );
        app.world_mut()
            .entity_mut(parent)
            .insert(RenderLayers::layer(3));
        app.update();
        assert_eq!(
            app.world().get::<RenderLayers>(child),
            Some(&RenderLayers::layer(3))
        );
        assert_eq!(
            app.world().resource::<Assets<SkyCompositeMaterial>>().len(),
            1
        );
        app.world_mut().despawn(parent);
        app.update();
        assert!(app.world().get_entity(child).is_err());
        assert!(
            app.world()
                .resource::<Assets<SkyCompositeMaterial>>()
                .is_empty()
        );
    }

    #[test]
    fn native_atmosphere_then_cloud_equals_combined_premultiplied_pass() {
        for alpha in [0., 0.5, 1.] {
            for moon_alpha in [0., 0.3, 1.] {
                let stars = Vec3::new(0.1, 0.2, 0.3);
                let moon = Vec3::new(0., 0.05, 0.1);
                let atmosphere = Vec3::new(0.2, 0.3, 0.4);
                let cloud = Vec3::new(0.4, 0.2, 0.1);
                let celestial = moon * moon_alpha + stars * (1. - moon_alpha);
                let native = cloud * alpha + (celestial + atmosphere) * (1. - alpha);
                let combined = atmosphere.lerp(cloud, alpha) + celestial * (1. - alpha);
                assert!((native - combined).abs().max_element() < 1e-6);
            }
        }
    }

    #[test]
    fn composite_sorts_after_origin_based_moons_before_clouds_and_glass() {
        use bevy::core_pipeline::core_3d::TransparentSortingInfo3d;
        use bevy::render::render_phase::ViewRangefinder3d;
        for at in [
            Vec3::new(-38000., 500., -35000.),
            Vec3::new(30000., 800., 10000.),
        ] {
            for look in [Vec3::X, -Vec3::X, Vec3::Z, -Vec3::Z] {
                let camera = Transform::from_translation(at).looking_to(look, Vec3::Y);
                let range = ViewRangefinder3d::from_world_from_view(&camera.compute_affine());
                let key = |center, bias| {
                    TransparentSortingInfo3d::Sorted {
                        mesh_center: center,
                        depth_bias: bias,
                    }
                    .sort_distance(&range)
                };
                let moon = key(Vec3::ZERO, crate::night_sky::SKY_SORT_BIAS + 4.);
                let composite = key(at, SKY_COMPOSITE_SORT_BIAS);
                let cloud = key(at + look * 2000., crate::night_sky::SKY_SORT_BIAS + 100000.);
                let glass = key(at + look * 50., 0.);
                assert!(moon < composite && composite < cloud && cloud < glass);
            }
        }
    }
}
