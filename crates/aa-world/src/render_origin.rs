//! The world origin the renderer draws from.
//!
//! The game world is up to 35 km wide and the client keeps its entities at their real positions (`coords.rs`): at 16 to 32 km a
//! single precision coordinate holds about 2 mm. The GPU blends a skinned mesh's joint matrices and multiplies every vertex by
//! the view in single precision, so at those coordinates each vertex of a character lands up to a few millimetres off, a
//! different amount every frame (about 0.4 px at 4 m, up to 1 px), and the camera's own translation cancels against the
//! world's in the view matrix (0.1 to 0.4 px) while it moves: the characters ripple and their edges crawl, and textures
//! swim as the player moves. 1.2 has no such noise: its 3D engine works in positions relative to a world origin kept in
//! whole cells (the seamless world's origin cells, `subzone.rs`; its spawn camera is at x 4315 m where the server's
//! position is cells further on).
//!
//! Here the entities keep their real positions for every system of the game; only what the render world takes is moved by
//! [`RenderOrigin`], a whole number of 1024 m cells near the camera's focus: after `Last` (schedule [`RenderOriginOut`]) every
//! world entity's `GlobalTransform` (and the derived data the renderer extracts: the previous transforms, the frusta, the
//! shadow cascades, the previous view) is moved by `-origin`, the render world extracts it, and before `First`
//! ([`RenderOriginIn`]) everything is put back. Whole cells keep `x - origin` exact (both are on the same 2 mm grid), so the
//! round trip changes nothing. The few materials that carry world positions of their own (fog volumes, projected decals and
//! road decals, voxel terrain) take them relative to the origin as well.
//!
//! An entity belongs to the world when its [`RenderLayers`] include layer 0 (or it has none); the isolated scenes of the
//! interface (model views, the plates) keep their own positions, as their cameras do.
//!
//! `AA_RENDER_ORIGIN`: `0` or `off` keeps the old drawing (no origin); a number is the distance (m) the focus may move from the
//! origin before the origin follows it (default [`DEFAULT_RECENTRE`]). Each move costs one frame in which every mesh is sent to
//! the GPU again.

use bevy::app::MainScheduleOrder;
use bevy::camera::primitives::{CascadesFrusta, CubemapFrusta, Frustum};
use bevy::camera::visibility::RenderLayers;
use bevy::core_pipeline::prepass::PreviousViewData;
use bevy::ecs::message::{MessageCursor, Messages};
use bevy::ecs::schedule::ScheduleLabel;
use bevy::light::Cascades;
use bevy::math::{Affine3A, DMat4, DVec3, Vec3A};
use bevy::pbr::PreviousGlobalTransform;
use bevy::prelude::*;
use bevy::shape::HalfSpace;

use crate::stream::WorldFocus;

/// The origin moves in whole cells (m): `x - origin` is then exact for every position of the world.
pub const CELL: f32 = 1024.0;
/// How far (m) the focus may move from the origin before the origin follows it: within 2 km a coordinate holds 0.25 mm.
pub const DEFAULT_RECENTRE: f32 = 2048.0;
/// A focus further out than this (m) is no place in a world (the login screens park the world's camera at 1000 km): no origin.
const FAR_FOCUS: f32 = 200_000.0;

/// The origin's x and z (Bevy axes) for a focus at `focus` (x, z), the origin now `current` and the distance `recentre` the
/// focus may stray (0 or less: no origin at all). Plain rule, no Bevy type.
pub fn choose(focus: Option<[f32; 2]>, current: [f32; 2], recentre: f32) -> [f32; 2] {
    let Some([x, z]) = focus.filter(|f| f.iter().all(|v| v.is_finite() && v.abs() < FAR_FOCUS)) else { return [0.0, 0.0] };
    if recentre <= 0.0 {
        return [0.0, 0.0];
    }
    if (x - current[0]).abs().max((z - current[1]).abs()) <= recentre {
        return current;
    }
    [(x / CELL).round() * CELL, (z / CELL).round() * CELL]
}

/// The distance [`choose`] takes from `AA_RENDER_ORIGIN` (unset: [`DEFAULT_RECENTRE`]; `0`, `off`, `no`: none).
pub fn recentre_from(value: Option<&str>) -> f32 {
    match value.map(str::trim) {
        None | Some("") => DEFAULT_RECENTRE,
        Some(v) if matches!(v.to_ascii_lowercase().as_str(), "off" | "no" | "false") => 0.0,
        Some(v) => v.parse::<f32>().ok().filter(|d| d.is_finite()).map_or(DEFAULT_RECENTRE, |d| d.max(0.0)),
    }
}

/// The world origin of the render world (Bevy axes, y 0): the renderer draws every world position minus this.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct RenderOrigin {
    pub offset: Vec3,
}

/// Runs after `Last`: the render world's view of the world is moved by the origin.
#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
pub struct RenderOriginOut;

/// Runs before `First`: the world is put back.
#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
pub struct RenderOriginIn;

#[derive(Resource, Default)]
struct OriginState {
    recentre: f32,
    /// The offset moved out this frame (zero: nothing to put back).
    applied: Vec3,
    frusta: Vec<(Entity, Frustum)>,
    cubemaps: Vec<(Entity, CubemapFrusta)>,
    cascade_frusta: Vec<(Entity, CascadesFrusta)>,
    cascades: Vec<(Entity, Cascades)>,
    previous_views: Vec<(Entity, PreviousViewData)>,
    fog: MessageCursor<AssetEvent<crate::fog::FogMaterial>>,
    roads: MessageCursor<AssetEvent<crate::roads::RoadMaterial>>,
    voxels: MessageCursor<AssetEvent<crate::objects::voxels::VoxelMaterial>>,
}

/// Draws the world from a nearby origin (see the module). The client adds it; tools and examples draw as before.
pub struct RenderOriginPlugin;

impl Plugin for RenderOriginPlugin {
    fn build(&self, app: &mut App) {
        let recentre = recentre_from(std::env::var("AA_RENDER_ORIGIN").ok().as_deref());
        info!("aa-world: render origin {}", if recentre > 0.0 { format!("follows the focus beyond {recentre} m") } else { "off".to_string() });
        app.insert_resource(RenderOrigin::default())
            .insert_resource(OriginState { recentre, ..default() })
            .init_schedule(RenderOriginOut)
            .init_schedule(RenderOriginIn)
            .add_systems(RenderOriginOut, shift_out)
            .add_systems(RenderOriginIn, shift_in);
        let mut order = app.world_mut().resource_mut::<MainScheduleOrder>();
        order.insert_after(Last, RenderOriginOut);
        order.insert_before(First, RenderOriginIn);
    }
}

fn in_world(layers: Option<&RenderLayers>) -> bool {
    layers.is_none_or(|l| l.intersects(&RenderLayers::default()))
}

fn moved(affine: Affine3A, by: Vec3) -> Affine3A {
    Affine3A { matrix3: affine.matrix3, translation: affine.translation + Vec3A::from(by) }
}

/// A frustum of absolute positions as one of positions relative to `origin` (`n.p + d >= 0` with `p = p' + origin`).
fn frustum_from(frustum: &Frustum, origin: DVec3) -> Frustum {
    let mut out = *frustum;
    for half_space in out.half_spaces.iter_mut() {
        let nd = half_space.normal_d();
        if !nd.w.is_finite() {
            continue;
        }
        let n = DVec3::new(nd.x as f64, nd.y as f64, nd.z as f64);
        *half_space = HalfSpace::new(nd.with_w((nd.w as f64 + n.dot(origin)) as f32));
    }
    out
}

fn shift_out(world: &mut World) {
    let recentre = world.resource::<OriginState>().recentre;
    // what streams the world: the `WorldFocus`, else the first world-layer 3D camera (the active one first), as `stream.rs` has it
    let focus = {
        let mut q = world.query_filtered::<&GlobalTransform, With<WorldFocus>>();
        q.iter(world).next().map(|t| t.translation())
    }
    .or_else(|| {
        let mut q = world.query_filtered::<(&GlobalTransform, &Camera, Option<&RenderLayers>), With<Camera3d>>();
        let cameras: Vec<(Vec3, bool)> =
            q.iter(world).filter(|(_, _, l)| in_world(*l)).map(|(t, c, _)| (t.translation(), c.is_active)).collect();
        cameras.iter().find(|(_, active)| *active).or(cameras.first()).map(|(p, _)| *p)
    });
    let old = world.resource::<RenderOrigin>().offset;
    let [x, z] = choose(focus.map(|p| [p.x, p.z]), [old.x, old.z], recentre);
    let new = Vec3::new(x, 0.0, z);
    let changed = new != old;
    shift_materials(world, old, new);
    if changed {
        world.resource_mut::<RenderOrigin>().offset = new;
        info!("aa-world: render origin ({}, {}) (game x {}, y {})", new.x, new.z, new.x, -new.z);
    }
    if new == Vec3::ZERO {
        world.resource_mut::<OriginState>().applied = Vec3::ZERO;
        if changed {
            // back to real positions: every mesh, joint and light is extracted again
            let mut q = world.query::<&mut GlobalTransform>();
            for mut t in q.iter_mut(world) {
                t.set_changed();
            }
            let mut q = world.query::<&mut Frustum>();
            for mut f in q.iter_mut(world) {
                f.set_changed();
            }
            let mut q = world.query::<&mut CubemapFrusta>();
            for mut f in q.iter_mut(world) {
                f.set_changed();
            }
        }
        return;
    }
    // a new origin: every derived value is sent again (the change marks them), else nothing is marked
    let full = changed;
    let by = -new;
    {
        let mut q = world.query::<(&mut GlobalTransform, Option<&RenderLayers>)>();
        q.par_iter_mut(world).for_each(|(mut t, layers)| {
            if in_world(layers) {
                let shifted = GlobalTransform::from(moved(t.affine(), by));
                if full {
                    *t = shifted;
                } else {
                    *t.bypass_change_detection() = shifted;
                }
            }
        });
        let mut q = world.query::<(&mut PreviousGlobalTransform, Option<&RenderLayers>)>();
        q.par_iter_mut(world).for_each(|(mut t, layers)| {
            if in_world(layers) {
                let shifted = PreviousGlobalTransform(moved(t.0, by));
                if full {
                    *t = shifted;
                } else {
                    *t.bypass_change_detection() = shifted;
                }
            }
        });
    }
    let origin = new.as_dvec3();
    let to_render = DMat4::from_translation(-origin);
    let from_render = DMat4::from_translation(origin);
    let world_views: Vec<Entity> = {
        let mut q = world.query_filtered::<(Entity, Option<&RenderLayers>), With<Camera>>();
        q.iter(world).filter(|(_, l)| in_world(*l)).map(|(e, _)| e).collect()
    };
    let mut state = std::mem::take(&mut *world.resource_mut::<OriginState>());
    state.applied = new;
    state.frusta.clear();
    state.cubemaps.clear();
    state.cascade_frusta.clear();
    state.cascades.clear();
    state.previous_views.clear();
    {
        let mut q = world.query::<(Entity, &mut Frustum, Option<&RenderLayers>)>();
        for (e, mut f, layers) in q.iter_mut(world) {
            if in_world(layers) {
                state.frusta.push((e, *f));
                let shifted = frustum_from(&f, origin);
                if full { *f = shifted } else { *f.bypass_change_detection() = shifted }
            }
        }
        let mut q = world.query::<(Entity, &mut CubemapFrusta, Option<&RenderLayers>)>();
        for (e, mut f, layers) in q.iter_mut(world) {
            if in_world(layers) {
                state.cubemaps.push((e, f.clone()));
                let shifted = CubemapFrusta { frusta: f.frusta.map(|one| frustum_from(&one, origin)) };
                if full { *f = shifted } else { *f.bypass_change_detection() = shifted }
            }
        }
        let mut q = world.query::<(Entity, &mut CascadesFrusta)>();
        for (e, mut f) in q.iter_mut(world) {
            state.cascade_frusta.push((e, f.clone()));
            let target = if full { &mut *f } else { f.bypass_change_detection() };
            for (view, list) in target.frusta.iter_mut() {
                if world_views.contains(view) {
                    for one in list.iter_mut() {
                        *one = frustum_from(one, origin);
                    }
                }
            }
        }
        let mut q = world.query::<(Entity, &mut Cascades)>();
        for (e, mut c) in q.iter_mut(world) {
            state.cascades.push((e, c.clone()));
            let target = if full { &mut *c } else { c.bypass_change_detection() };
            for (view, list) in target.cascades.iter_mut() {
                if world_views.contains(view) {
                    for one in list.iter_mut() {
                        one.world_from_cascade = (to_render * one.world_from_cascade.as_dmat4()).as_mat4();
                        one.clip_from_world = (one.clip_from_world.as_dmat4() * from_render).as_mat4();
                    }
                }
            }
        }
        let mut q = world.query::<(Entity, &mut PreviousViewData, Option<&RenderLayers>)>();
        for (e, mut v, layers) in q.iter_mut(world) {
            if in_world(layers) {
                state.previous_views.push((e, v.clone()));
                let target = if full { &mut *v } else { v.bypass_change_detection() };
                target.view_from_world = (target.view_from_world.as_dmat4() * from_render).as_mat4();
                target.clip_from_world = (target.clip_from_world.as_dmat4() * from_render).as_mat4();
                target.unjittered_clip_from_world = (target.unjittered_clip_from_world.as_dmat4() * from_render).as_mat4();
                target.world_from_clip = (to_render * target.world_from_clip.as_dmat4()).as_mat4();
            }
        }
    }
    *world.resource_mut::<OriginState>() = state;
}

fn shift_in(world: &mut World) {
    let mut state = std::mem::take(&mut *world.resource_mut::<OriginState>());
    let back = state.applied;
    if back != Vec3::ZERO {
        let mut q = world.query::<(&mut GlobalTransform, Option<&RenderLayers>)>();
        q.par_iter_mut(world).for_each(|(mut t, layers)| {
            if in_world(layers) {
                *t.bypass_change_detection() = GlobalTransform::from(moved(t.affine(), back));
            }
        });
        let mut q = world.query::<(&mut PreviousGlobalTransform, Option<&RenderLayers>)>();
        q.par_iter_mut(world).for_each(|(mut t, layers)| {
            if in_world(layers) {
                t.bypass_change_detection().0 = moved(t.0, back);
            }
        });
        for (e, original) in state.frusta.drain(..) {
            if let Some(mut f) = world.get_mut::<Frustum>(e) {
                *f.bypass_change_detection() = original;
            }
        }
        for (e, original) in state.cubemaps.drain(..) {
            if let Some(mut f) = world.get_mut::<CubemapFrusta>(e) {
                *f.bypass_change_detection() = original;
            }
        }
        for (e, original) in state.cascade_frusta.drain(..) {
            if let Some(mut f) = world.get_mut::<CascadesFrusta>(e) {
                *f.bypass_change_detection() = original;
            }
        }
        for (e, original) in state.cascades.drain(..) {
            if let Some(mut c) = world.get_mut::<Cascades>(e) {
                *c.bypass_change_detection() = original;
            }
        }
        for (e, original) in state.previous_views.drain(..) {
            if let Some(mut v) = world.get_mut::<PreviousViewData>(e) {
                *v.bypass_change_detection() = original;
            }
        }
        state.applied = Vec3::ZERO;
    }
    *world.resource_mut::<OriginState>() = state;
}

/// The materials that hold world positions of their own: one added since the last frame is moved by the origin it was made
/// under (`old`), then, when the origin moves, every one by the step.
fn shift_materials(world: &mut World, old: Vec3, new: Vec3) {
    let mut state = std::mem::take(&mut *world.resource_mut::<OriginState>());
    shift_assets(world, &mut state.fog, old, new, |m: &mut crate::fog::FogMaterial, d| m.move_by(-d));
    shift_assets(world, &mut state.roads, old, new, |m: &mut crate::roads::RoadMaterial, d| m.extension.move_by(-d));
    shift_assets(world, &mut state.voxels, old, new, |m: &mut crate::objects::voxels::VoxelMaterial, d| {
        m.extension.params.origin -= d.extend(0.0)
    });
    *world.resource_mut::<OriginState>() = state;
}

fn shift_assets<A: Asset>(world: &mut World, cursor: &mut MessageCursor<AssetEvent<A>>, old: Vec3, new: Vec3, shift: impl Fn(&mut A, Vec3)) {
    let Some(messages) = world.get_resource::<Messages<AssetEvent<A>>>() else { return };
    let added: Vec<AssetId<A>> = cursor
        .read(messages)
        .filter_map(|e| match e {
            AssetEvent::Added { id } => Some(*id),
            _ => None,
        })
        .collect();
    let Some(mut assets) = world.get_resource_mut::<Assets<A>>() else { return };
    if old != Vec3::ZERO {
        for id in added {
            if let Some(mut a) = assets.get_mut(id) {
                shift(&mut *a, old);
            }
        }
    }
    let step = new - old;
    if step != Vec3::ZERO {
        let ids: Vec<AssetId<A>> = assets.ids().collect();
        for id in ids {
            if let Some(mut a) = assets.get_mut(id) {
                shift(&mut *a, step);
            }
        }
    }
}
