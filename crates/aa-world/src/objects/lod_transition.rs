//! Native brush mesh-pair dissolve (30111AE0 / 301126F0).
//! The shipping DLL uses distance, not e_dissolve_transition_time. See tools/lod_evidence.

use super::{ObjectSector, ObjectState, lod};
use crate::placement::Brush;
use aa_cry::shade::illum::IllumMaterial;
use bevy::{
    asset::{AssetId, embedded_asset},
    camera::visibility::RenderLayers,
    mesh::MeshTag,
    pbr::{ExtendedMaterial, MaterialExtension, MaterialPlugin},
    prelude::*,
    render::render_resource::AsBindGroup,
    shader::ShaderRef,
};
use std::collections::HashMap;

pub const DISTANCE_BAND: f32 = 5.0;

/// The native per-render-node distance state at +16/+20/+21/+22.
/// No elapsed-time input: holding the camera holds a partially dissolved pair.
#[derive(Clone, Copy, Debug)]
pub struct Dissolve {
    pub distance: f32,
    pub from: u8,
    pub to: u8,
    pub approaching: bool,
}

impl Dissolve {
    pub fn new(distance: f32, level: u8) -> Self {
        Self {
            distance,
            from: level,
            to: level,
            approaching: false,
        }
    }

    /// 301126F0 in the normal render pass with narrow-FOV suppression inactive.
    pub fn update(&mut self, distance: f32, requested: u8) -> u8 {
        if self.distance == 0.0 {
            *self = Self::new(distance, requested);
        } else if self.to != requested {
            self.to = requested;
            self.distance = distance;
            self.approaching = requested < self.from;
        } else if self.from != self.to {
            let delta = if self.approaching {
                self.distance - distance
            } else {
                distance - self.distance
            };
            let band = self.band();
            if delta > band {
                self.from = self.to;
            } else if delta < 0.0 {
                self.to = self.from;
            }
        }
        if self.from == self.to {
            return 0;
        }
        let delta = if self.approaching {
            self.distance - distance
        } else {
            distance - self.distance
        };
        ((delta * (1.0 / self.band())).clamp(0.0, 1.0) * 255.0) as u8
    }

    fn band(self) -> f32 {
        if self.approaching {
            DISTANCE_BAND.min(self.distance * 0.4)
        } else {
            DISTANCE_BAND
        }
    }

    fn suppress(&mut self, distance: f32) {
        self.distance = (if self.approaching {
            DISTANCE_BAND
        } else {
            -DISTANCE_BAND
        }) + distance;
        self.from = self.to;
    }

    /// Native caller rejects non-adjacent levels, and jumps too far from the inverse
    /// LOD boundary (30111B95..BA6, 30111CFD..1D23). Keep the existing picker otherwise.
    pub fn eligible(self, next: u8, radius: f32, ratio: u8) -> bool {
        if self.from.abs_diff(next) != 1 || ratio == 0 {
            return false;
        }
        let ratio = f32::from(ratio) * 0.01;
        let boundary = (6.0_f64 * f64::from(radius) * f64::from(self.from.max(next)))
            / (f64::from(ratio) * f64::from(ratio));
        (f64::from(self.distance) - boundary).abs() <= f64::from(DISTANCE_BAND)
    }
}

/// Signed Bevy dither level; complementary members have levels n and n-16.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct MeshLodDither {
    pub level: i32,
    pub secondary: bool,
    pub active: bool,
}

impl MeshLodDither {
    fn tag(self) -> u32 {
        u32::from(self.active) * 256
            + u32::from(self.secondary) * 512
            + (self.level.clamp(-16, 16) + 16) as u32
    }
}

/// Reuses the existing Standard/Illum fragments, including alpha test and lighting.
/// Only the vertex dither input changes. The regular far range retains its shared GPU slots.
#[derive(Asset, AsBindGroup, Reflect, Clone, Debug, Default)]
pub struct LodDither {}
pub type LodStandardMaterial = ExtendedMaterial<StandardMaterial, LodDither>;
pub type LodIllumMaterial = ExtendedMaterial<IllumMaterial, LodDither>;

impl MaterialExtension for LodDither {
    fn vertex_shader() -> ShaderRef {
        "embedded://aa_world/objects/lod_vertex.wesl".into()
    }
    fn prepass_vertex_shader() -> ShaderRef {
        "embedded://aa_world/objects/lod_prepass.wesl".into()
    }
    fn deferred_vertex_shader() -> ShaderRef {
        Self::prepass_vertex_shader()
    }
}

pub struct LodTransitionPlugin;
impl Plugin for LodTransitionPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "lod_vertex.wesl");
        embedded_asset!(app, "lod_prepass.wesl");
        app.add_plugins((
            MaterialPlugin::<LodStandardMaterial>::default(),
            MaterialPlugin::<LodIllumMaterial>::default(),
        ))
        .init_resource::<Mirrors>()
        .init_resource::<ObjectState>()
        .init_resource::<NarrowFrames>()
        .add_systems(
            Update,
            (begin, advance).chain().after(super::stream_objects),
        )
        // aa-cry's PostUpdate conversion finishes before wrapping; visibility
        // changes above must precede PostUpdate's propagation/culling.
        .add_systems(Last, (wrap_materials, tags).chain());
    }
}

#[derive(Resource, Default)]
struct Mirrors {
    standard: HashMap<AssetId<StandardMaterial>, AssetId<LodStandardMaterial>>,
    illum: HashMap<AssetId<IllumMaterial>, AssetId<LodIllumMaterial>>,
}

/// 30151E0D..1E34: FOV scale < .7 increments to four; normal FOV decrements to zero.
#[derive(Resource, Default)]
struct NarrowFrames(u8);

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn wrap_materials(
    mut commands: Commands,
    mut cache: ResMut<Mirrors>,
    standard: Res<Assets<StandardMaterial>>,
    illum: Res<Assets<IllumMaterial>>,
    mut a: ResMut<Assets<LodStandardMaterial>>,
    mut b: ResMut<Assets<LodIllumMaterial>>,
    plain: Query<(Entity, &MeshMaterial3d<StandardMaterial>), With<lod::BrushLodPart>>,
    lit: Query<(Entity, &MeshMaterial3d<IllumMaterial>), With<lod::BrushLodPart>>,
) {
    // Ids are weak: cell/world unload releases wrappers and all referenced textures.
    cache.standard.retain(|_, id| a.contains(*id));
    cache.illum.retain(|_, id| b.contains(*id));
    for (e, h) in &plain {
        let Some(base) = standard.get(&h.0) else {
            continue;
        };
        let handle = cache
            .standard
            .get(&h.id())
            .and_then(|id| a.get_strong_handle(*id))
            .unwrap_or_else(|| {
                let h2 = a.add(ExtendedMaterial {
                    base: base.clone(),
                    extension: LodDither {},
                });
                cache.standard.insert(h.id(), h2.id());
                h2
            });
        commands
            .entity(e)
            .remove::<MeshMaterial3d<StandardMaterial>>()
            .insert(MeshMaterial3d(handle));
    }
    for (e, h) in &lit {
        let Some(base) = illum.get(&h.0) else {
            continue;
        };
        let handle = cache
            .illum
            .get(&h.id())
            .and_then(|id| b.get_strong_handle(*id))
            .unwrap_or_else(|| {
                let h2 = b.add(ExtendedMaterial {
                    base: base.clone(),
                    extension: LodDither {},
                });
                cache.illum.insert(h.id(), h2.id());
                h2
            });
        commands
            .entity(e)
            .remove::<MeshMaterial3d<IllumMaterial>>()
            .insert(MeshMaterial3d(handle));
    }
}

#[derive(Component)]
pub(super) struct PendingSwap {
    pub old: Vec<Entity>,
}

struct Pair {
    index: usize,
    bin_index: usize,
    old_level: u8,
    new_level: u8,
    state: Dissolve,
    old: Vec<Entity>,
    new: Vec<Entity>,
}
#[derive(Component)]
struct Pairs {
    old_roots: Vec<Entity>,
    pairs: Vec<Pair>,
}

#[allow(clippy::type_complexity)]
fn begin(
    mut commands: Commands,
    mut objects: ResMut<ObjectState>,
    pending: Query<(Entity, &ObjectSector, &PendingSwap)>,
    children: Query<&Children>,
    parts: Query<&lod::BrushLodPart>,
    cameras: Query<
        (
            &Camera,
            &GlobalTransform,
            &Projection,
            Option<&RenderLayers>,
        ),
        With<Camera3d>,
    >,
) {
    let view = lod::select_view(cameras.iter());
    for (root, sector, swap) in &pending {
        let key = (sector.cell.0, sector.cell.1, sector.sector, sector.kind);
        let Some(data) = objects.cells.get(&sector.cell) else {
            continue;
        };
        let bin = super::bin_of(data, sector.kind, sector.sector as usize);
        let mut old: HashMap<u32, Vec<Entity>> = HashMap::new();
        for r in &swap.old {
            if let Ok(c) = children.get(*r) {
                for e in c.iter() {
                    if let Ok(p) = parts.get(e) {
                        old.entry(p.index).or_default().push(e);
                    }
                }
            }
        }
        let mut new: HashMap<u32, Vec<Entity>> = HashMap::new();
        if let Ok(c) = children.get(root) {
            for e in c.iter() {
                if let Ok(p) = parts.get(e) {
                    new.entry(p.index).or_default().push(e);
                }
            }
        }
        let mut pairs = Vec::new();
        for (bin_index, &index) in bin.iter().enumerate() {
            let Some(previous) = old.remove(&index) else {
                continue;
            };
            let Some(next) = new.remove(&index) else {
                // Failed replacement: preserve real geometry, rather than punch a hole.
                // Keep the attempted request in the sector snapshot, so a known
                // failed model does not trigger the same rebuild every frame.
                for e in previous {
                    commands.entity(e).insert(ChildOf(root));
                }
                continue;
            };
            let from = parts.get(previous[0]).unwrap().requested;
            let to = parts.get(next[0]).unwrap().requested;
            let input =
                view.and_then(|v| v.distance(&data.objects.brushes[index as usize], sector.cell));
            let state = input.map(|(d, _)| Dissolve::new(d, from));
            if let (Some(mut state), Some((distance, radius))) = (state, input)
                && state.eligible(to, radius, data.objects.brushes[index as usize].lod_ratio)
            {
                state.update(distance, to);
                pairs.push(Pair {
                    index: index as usize,
                    bin_index,
                    old_level: from,
                    new_level: to,
                    state,
                    old: previous,
                    new: next,
                });
            } else {
                for e in previous {
                    commands.entity(e).despawn();
                }
            }
        }
        commands
            .entity(root)
            .remove::<PendingSwap>()
            .insert(Visibility::Inherited);
        if pairs.is_empty() {
            for r in &swap.old {
                commands.entity(*r).despawn();
            }
            if let Some(s) = objects.sectors.get_mut(&key) {
                s.retired.clear();
                s.transitioning = false;
            }
        } else {
            commands.entity(root).insert(Pairs {
                old_roots: swap.old.clone(),
                pairs,
            });
        }
    }
}

fn advance(
    mut commands: Commands,
    mut objects: ResMut<ObjectState>,
    mut narrow: ResMut<NarrowFrames>,
    mut active: Query<(Entity, &ObjectSector, &mut Pairs)>,
    mut dithers: Query<&mut MeshLodDither>,
    cameras: Query<
        (
            &Camera,
            &GlobalTransform,
            &Projection,
            Option<&RenderLayers>,
        ),
        With<Camera3d>,
    >,
) {
    let view = lod::select_view(cameras.iter());
    narrow.0 = if view.is_some_and(lod::View::narrow) {
        (narrow.0 + 1).min(4)
    } else {
        narrow.0.saturating_sub(1)
    };
    let mut count = 0;
    let mut extra = 0;
    for (root, sector, mut active) in &mut active {
        let key = (sector.cell.0, sector.cell.1, sector.sector, sector.kind);
        let Some(data) = objects.cells.get(&sector.cell).cloned() else {
            continue;
        };
        active.pairs.retain_mut(|pair| {
            let brush: &Brush = &data.objects.brushes[pair.index];
            let input = view.and_then(|v| {
                Some((
                    v.distance(brush, sector.cell)?.0,
                    v.requested(brush, sector.cell)?,
                ))
            });
            let reference = if let Some((d, requested)) = input {
                // Only the two loaded meshes participate; a later scan handles another request.
                let requested = if requested == pair.old_level {
                    pair.old_level
                } else {
                    pair.new_level
                };
                let reference = pair.state.update(d, requested);
                if narrow.0 != 0 {
                    pair.state.suppress(d);
                    0
                } else {
                    reference
                }
            } else {
                pair.state.from = pair.new_level;
                pair.state.to = pair.new_level;
                0
            };
            if pair.state.from == pair.state.to {
                let keep_old = pair.state.to == pair.old_level;
                let (keep, drop) = if keep_old {
                    (&pair.old, &pair.new)
                } else {
                    (&pair.new, &pair.old)
                };
                for e in drop {
                    commands.entity(*e).despawn();
                }
                for e in keep {
                    commands
                        .entity(*e)
                        .insert((ChildOf(root), MeshLodDither::default()));
                }
                if keep_old && let Some(s) = objects.sectors.get_mut(&key) {
                    s.brush_lods
                        .as_mut()
                        .unwrap()
                        .retain(pair.bin_index, pair.old_level);
                }
                return false;
            }
            let n = (f32::from(reference) * (16.0 / 255.0)).round() as i32;
            for (entities, level, secondary) in [(&pair.old, n, false), (&pair.new, n - 16, true)] {
                for e in entities {
                    if let Ok(mut d) = dithers.get_mut(*e) {
                        *d = MeshLodDither {
                            level,
                            secondary,
                            active: true,
                        };
                    }
                }
            }
            count += 1;
            extra += pair.old.len();
            true
        });
        if active.pairs.is_empty() {
            for r in &active.old_roots {
                commands.entity(*r).despawn();
            }
            commands.entity(root).remove::<Pairs>();
            if let Some(s) = objects.sectors.get_mut(&key) {
                s.retired.clear();
                s.transitioning = false;
            }
        }
    }
    objects.stats.brush_lod_pairs = count;
    objects.stats.brush_lod_extra_parts = extra;
}

#[allow(clippy::type_complexity)]
fn tags(
    mut commands: Commands,
    parts: Query<
        (
            Entity,
            &MeshLodDither,
            Option<&MeshMaterial3d<LodIllumMaterial>>,
        ),
        Changed<MeshLodDither>,
    >,
) {
    for (e, d, illum) in &parts {
        commands.entity(e).insert(if illum.is_some() {
            MeshTag::with_type::<LodIllumMaterial>(d.tag())
        } else {
            MeshTag::with_type::<LodStandardMaterial>(d.tag())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    include!("../../tools/native_lod_cases.rs");

    #[test]
    fn shipping_instruction_oracle_all_steps() {
        for case in NATIVE_CASES {
            let mut state = Dissolve::new(0.0, 0);
            for &(distance, requested, start, from, to, approaching, reference) in *case {
                assert_eq!(
                    state.update(distance, requested),
                    reference,
                    "{distance} {requested}"
                );
                assert_eq!(
                    (state.distance, state.from, state.to, state.approaching),
                    (start, from, to, approaching)
                );
            }
        }
    }

    #[test]
    fn caller_keeps_only_adjacent_pairs_near_inverse_boundary() {
        let s = Dissolve::new(100.0, 0);
        assert!(s.eligible(1, 100.0 / 6.0, 100));
        assert!(!s.eligible(2, 100.0 / 6.0, 100));
        assert!(!s.eligible(1, 200.0 / 6.0, 100));
        assert!(!s.eligible(1, 1.0, 0));
    }

    #[test]
    fn narrow_fov_native_four_frame_counter_and_state_suppression() {
        let mut counter = 0_u8;
        for (degrees, expected) in [
            (60., 0),
            (30., 1),
            (30., 2),
            (30., 3),
            (30., 4),
            (30., 4),
            (60., 3),
            (60., 2),
            (60., 1),
            (60., 0),
            (60., 0),
        ] {
            let view = lod::View::new(Vec3::ZERO, degrees * std::f32::consts::PI / 180.0).unwrap();
            counter = if view.narrow() {
                (counter + 1).min(4)
            } else {
                counter.saturating_sub(1)
            };
            assert_eq!(counter, expected);
        }
        let mut s = Dissolve::new(0.0, 0);
        for (distance, requested, start, from, to, approach) in [
            (100., 0, 95., 0, 0, false),
            (100., 1, 95., 1, 1, false),
            (102.5, 1, 97.5, 1, 1, false),
            (102., 0, 107., 0, 0, true),
        ] {
            s.update(distance, requested);
            s.suppress(distance);
            assert_eq!(
                (s.distance, s.from, s.to, s.approaching),
                (start, from, to, approach)
            );
        }
    }

    #[test]
    fn bayer_pair_covers_each_pixel_once_at_all17_steps() {
        for n in 0..=16 {
            for threshold in 0..16 {
                let old = n + threshold < 16;
                let new = 1 + (n - 16) + threshold > 0;
                assert_ne!(old, new, "n={n} threshold={threshold}");
            }
        }
    }

    fn scene() -> (App, Entity, Entity, Entity, Entity) {
        use super::super::{CellObjectData, Kind, ViewLimits};
        use crate::placement::CellObjects;
        let mut app = App::new();
        app.init_resource::<ObjectState>()
            .init_resource::<NarrowFrames>()
            .add_systems(Update, (begin, advance).chain());
        let half = 100.0 / 6.0 / 3.0_f32.sqrt();
        let brush = Brush {
            matrix: [1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0.],
            model: 0,
            material: -1,
            flags: 0,
            max_view_dist: 300.,
            lod_ratio: 100,
            bbox: [-half, -half, -half, half, half, half],
        };
        let data = CellObjectData::new(
            CellObjects {
                models: vec!["fixture.cgf".into()],
                brushes: vec![brush],
                ..default()
            },
            &ViewLimits::default(),
        );
        app.world_mut()
            .resource_mut::<ObjectState>()
            .cells
            .insert((0, 0), std::sync::Arc::new(data));
        let camera = app
            .world_mut()
            .spawn((
                Camera3d::default(),
                Camera::default(),
                GlobalTransform::from_translation(crate::cry_to_bevy(Vec3::new(
                    half + 100.,
                    0.,
                    0.,
                ))),
                Projection::Perspective(PerspectiveProjection {
                    fov: 60_f32.to_radians(),
                    ..default()
                }),
            ))
            .id();
        let old = app.world_mut().spawn_empty().id();
        let root = app
            .world_mut()
            .spawn((
                ObjectSector {
                    cell: (0, 0),
                    sector: 0,
                    kind: Kind::Brush,
                },
                Visibility::Hidden,
                PendingSwap { old: vec![old] },
            ))
            .id();
        let a = app
            .world_mut()
            .spawn((
                ChildOf(old),
                lod::BrushLodPart {
                    cell: (0, 0),
                    index: 0,
                    requested: 0,
                },
                MeshLodDither::default(),
            ))
            .id();
        let b = app
            .world_mut()
            .spawn((
                ChildOf(root),
                lod::BrushLodPart {
                    cell: (0, 0),
                    index: 0,
                    requested: 1,
                },
                MeshLodDither::default(),
            ))
            .id();
        (app, camera, root, a, b)
    }

    #[test]
    fn ready_pair_holds_then_retires_old_geometry_and_roots() {
        let (mut app, camera, root, a, b) = scene();
        app.update();
        assert!(app.world().get::<Pairs>(root).is_some());
        let initial = *app.world().get::<GlobalTransform>(camera).unwrap();
        app.world_mut()
            .entity_mut(camera)
            .insert(GlobalTransform::from_translation(
                initial.translation() + Vec3::X * 2.5,
            ));
        app.update();
        assert_eq!(app.world().get::<MeshLodDither>(a).unwrap().level, 8);
        assert_eq!(app.world().get::<MeshLodDither>(b).unwrap().level, -8);
        for _ in 0..100 {
            app.update();
        }
        assert_eq!(
            app.world().resource::<ObjectState>().stats.brush_lod_pairs,
            1
        );
        assert_eq!(app.world().get::<MeshLodDither>(a).unwrap().level, 8);
        app.world_mut()
            .entity_mut(camera)
            .insert(GlobalTransform::from_translation(
                initial.translation() + Vec3::X * 5.01,
            ));
        app.update();
        assert!(app.world().get_entity(a).is_err());
        assert!(app.world().get_entity(b).is_ok());
        assert!(app.world().get::<Pairs>(root).is_none());
        assert_eq!(
            app.world().resource::<ObjectState>().stats.brush_lod_pairs,
            0
        );
    }

    #[test]
    fn reversed_camera_keeps_old_geometry_without_a_third_mesh() {
        let (mut app, camera, root, a, b) = scene();
        app.update();
        let initial = *app.world().get::<GlobalTransform>(camera).unwrap();
        app.world_mut()
            .entity_mut(camera)
            .insert(GlobalTransform::from_translation(
                initial.translation() + Vec3::X * 2.5,
            ));
        app.update();
        app.world_mut()
            .entity_mut(camera)
            .insert(GlobalTransform::from_translation(
                initial.translation() - Vec3::X,
            ));
        app.update();
        assert!(app.world().get_entity(b).is_err());
        assert_eq!(app.world().get::<ChildOf>(a).unwrap().parent(), root);
        assert!(!app.world().get::<MeshLodDither>(a).unwrap().active);
    }

    #[test]
    fn failed_replacement_preserves_the_existing_parts() {
        let (mut app, _, root, a, b) = scene();
        app.world_mut().despawn(b);
        app.update();
        assert_eq!(app.world().get::<ChildOf>(a).unwrap().parent(), root);
        assert!(app.world().get::<PendingSwap>(root).is_none());
        assert!(app.world().get::<Pairs>(root).is_none());
        assert_eq!(
            app.world().resource::<ObjectState>().stats.brush_lod_pairs,
            0
        );
    }
}
