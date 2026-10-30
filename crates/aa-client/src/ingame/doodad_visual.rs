//! Doodad phases and growth.
//!
//! A doodad is drawn with the model of its current phase (`doodad_func_groups.model`, falling back to
//! `doodad_almighties.model`) and, when the phase grows (`DoodadFuncGrowth`), scaled from `start_scale` to `end_scale` as
//! the server's `time_left` runs out (the packet's `time_left` is the phase's exact remaining time).
//! `SCDoodadPhaseChanged` moves a drawn doodad to another phase: the
//! model under the root is replaced within a few frames (the root, its position and its base scale stay), and a phase
//! whose model does not change only gets its new scale. A doodad that had no model gets another try on a phase change.
//! A cutdown tree keeps its model and its scale while it falls and gets both after landing (`cutdown.rs`).

use std::collections::{HashMap, HashSet, VecDeque};

use aa_cry::scene::CryAssets;
use aa_cry::shade::illum::IllumMaterial;
use aa_net::state::Doodads;
use aa_net::{Event, NetEvent};
use bevy::prelude::*;

use super::cutdown;
use super::data::{Db, DoodadGrowth};
use super::units::{RemoteDoodad, Views};

/// Model swaps per frame: each reads the pak and builds the model on the main thread.
const MAX_SWAPS: usize = 2;

/// The next doodad whose phase model can be swapped now. Falling trees stay queued (at the back)
/// without using up a swap slot, so another doodad still gets its model this frame. The
/// 2026-10-30 capture sends a cut tree's phase change before the cutdown in the same batch;
/// this keeps that cut phase's swap queued while the fall plays.
fn next_swap(queue: &mut VecDeque<u32>, falling: &HashSet<u32>) -> Option<u32> {
    let queued = queue.len();
    for _ in 0..queued {
        let obj_id = queue.pop_front()?;
        if falling.contains(&obj_id) {
            queue.push_back(obj_id);
        } else {
            return Some(obj_id);
        }
    }
    None
}

/// What a drawn doodad shows now and how the current phase grows.
#[derive(Component)]
pub struct DoodadVisual {
    /// The pak path of the drawn model.
    pub path: String,
    /// The model entity under the doodad root.
    pub model: Option<Entity>,
    /// The phase the drawn model belongs to.
    pub func_group_id: u32,
    /// Phase whose complete geometry is actually present; a pending/failed swap is not ready.
    pub rendered_func_group_id: u32,
    /// Authored bond helpers relative to the doodad, in Cry metres.
    pub bond_helpers: HashMap<String, bevy::math::Affine3A>,
    /// The phase's growth, `None` when it does not grow.
    pub growth: Option<DoodadGrowth>,
    /// Milliseconds of the growth left: the server's `time_left` at the spawn / phase change, counted down locally.
    pub remaining_ms: f32,
    /// The whole phase's duration (see [`DoodadGrowth::total_ms`]): the `time_left` announced by a phase change, or
    /// the longer of the database delay and the `time_left` at a spawn first seen mid-phase.
    pub total_ms: f32,
    /// The spawn packet's `doodad.scale` (the model's own size is multiplied by the growth).
    pub base_scale: f32,
}

impl DoodadVisual {
    /// The growth scale of the phase (`1` when it does not grow).
    pub fn growth_scale(&self) -> f32 {
        self.growth.map_or(1.0, |g| g.scale_at(self.remaining_ms, self.total_ms))
    }
}

pub fn plugin(app: &mut App) {
    app.init_resource::<DoodadHighlights>().add_systems(
        Update,
        // The capture sends SCDoodadPhaseChanged before SCVegetationCutdowning in one batch;
        // `start` runs first so the fall keeps the standing model and scale and the swap waits (see cutdown.rs).
        (cutdown::start, phase_changes, cutdown::fall, cutdown::land, cutdown::settle, grow, highlight)
            .chain()
            .after(super::units::sync_views)
            .run_if(super::world_screens),
    );
}

/// The doodads that glow: the one the use bar is about and the one under the pointer (`hud::doodad` writes them, the
/// doodads whose `use_target_highlight` is off left out).
#[derive(Resource, Default)]
pub struct DoodadHighlights(pub Vec<u32>);

/// What a glowing doodad's materials were before: its model and each mesh's own material.
#[derive(Component)]
struct Glowing {
    model: Entity,
    originals: Vec<(Entity, GlowMaterial)>,
}

enum GlowMaterial {
    Standard(Handle<StandardMaterial>),
    Illum(Handle<IllumMaterial>),
}

/// The tint added to a glowing doodad's materials.
const GLOW: LinearRgba = LinearRgba::new(0.3, 0.26, 0.12, 0.0);

/// Lights the wanted doodads up with private copies of their materials (so the doodads that share the model keep
/// theirs) and puts the others back. The copy is complete, maps included; the tint only goes into the emissive term,
/// modulated by the diffuse map where the material had no emissive map of its own. A flat emissive added to a dark
/// prop (the cart's storage crate) washed the wood out to a pale panel.
#[allow(clippy::too_many_arguments)]
fn highlight(
    mut commands: Commands,
    wanted: Res<DoodadHighlights>,
    glowing: Query<(Entity, &Glowing, &RemoteDoodad, &DoodadVisual)>,
    plain: Query<(Entity, &RemoteDoodad, &DoodadVisual), Without<Glowing>>,
    children: Query<&Children>,
    mut meshes: Query<&mut MeshMaterial3d<StandardMaterial>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut illum_meshes: Query<&mut MeshMaterial3d<IllumMaterial>>,
    mut illum_materials: ResMut<Assets<IllumMaterial>>,
) {
    for (entity, glow, remote, visual) in &glowing {
        if wanted.0.contains(&remote.obj_id) && visual.model == Some(glow.model) {
            continue;
        }
        for (mesh, original) in &glow.originals {
            // aa-cry swaps a tagged StandardMaterial for its Illum mirror in PostUpdate, so the mesh may hold either
            // type by the time the highlight ends: replace the component instead of only editing the old one.
            match original {
                GlowMaterial::Standard(original) => {
                    commands
                        .entity(*mesh)
                        .remove::<MeshMaterial3d<IllumMaterial>>()
                        .insert(MeshMaterial3d(original.clone()));
                }
                GlowMaterial::Illum(original) => {
                    commands
                        .entity(*mesh)
                        .remove::<MeshMaterial3d<StandardMaterial>>()
                        .insert(MeshMaterial3d(original.clone()));
                }
            }
        }
        commands.entity(entity).remove::<Glowing>();
    }
    for (entity, remote, visual) in &plain {
        if !wanted.0.contains(&remote.obj_id) {
            continue;
        }
        let Some(model) = visual.model else { continue };
        let mut originals = Vec::new();
        for mesh in children.iter_descendants(model) {
            if let Ok(mut m) = meshes.get_mut(mesh)
                && let Some(mut lit) = materials.get(&m.0).cloned()
            {
                commands
                    .entity(mesh)
                    .remove::<MeshMaterial3d<IllumMaterial>>();
                tint_standard(&mut lit);
                let original = std::mem::replace(&mut m.0, materials.add(lit));
                originals.push((mesh, GlowMaterial::Standard(original)));
            } else if let Ok(mut m) = illum_meshes.get_mut(mesh)
                && let Some(mut lit) = illum_materials.get(&m.0).cloned()
            {
                commands
                    .entity(mesh)
                    .remove::<MeshMaterial3d<StandardMaterial>>();
                tint_illum(&mut lit);
                let original = std::mem::replace(&mut m.0, illum_materials.add(lit));
                originals.push((mesh, GlowMaterial::Illum(original)));
            }
        }
        commands.entity(entity).insert(Glowing { model, originals });
    }
}

/// The warm tint, multiplied by the diffuse map when the material has no emissive map: the prop keeps its wood grain
/// instead of turning into a flat pale surface.
fn tint_standard(m: &mut StandardMaterial) {
    m.emissive += GLOW;
    if m.emissive_texture.is_none() {
        m.emissive_texture = m.base_color_texture.clone();
    }
}

/// Basic Illum uses PBR emission; Full Illum reads its native emission uniform.
fn tint_illum(m: &mut IllumMaterial) {
    m.base.emissive += GLOW;
    if m.base.emissive_texture.is_none() {
        m.base.emissive_texture = m.base.base_color_texture.clone();
    }
    m.extension.params.emission += Vec4::new(GLOW.red, GLOW.green, GLOW.blue, 0.0);
}

/// `SCDoodadPhaseChanged`: the new phase's growth from the announced `time_left` at once, its model when it differs
/// a few per frame ([`MAX_SWAPS`]). A cutdown tree that is falling keeps its model and its scale (the phase's growth
/// scale would be a pop mid-fall) until it lands; the swap then applies both.
#[allow(clippy::too_many_arguments)]
pub fn phase_changes(
    mut events: MessageReader<NetEvent>,
    mut commands: Commands,
    db: Option<Res<Db>>,
    doodads: Res<Doodads>,
    mut views: ResMut<Views>,
    time: Res<Time>,
    mut roots: Query<(&RemoteDoodad, &mut DoodadVisual, &mut Transform)>,
    falling: Query<&RemoteDoodad, With<cutdown::Falling>>,
    mut cry: CryAssets,
    mut swaps: Local<VecDeque<u32>>,
) {
    let Some(db) = db else { return };
    let falling: HashSet<u32> = falling.iter().map(|remote| remote.obj_id).collect();
    for NetEvent(event) in events.read() {
        let updates: Vec<_> = match event {
            Event::DoodadsSpawned(spawned) => spawned.iter().map(|d|(d.obj_id,d.func_group_id,d.time_left,true)).collect(),
            Event::DoodadPhaseChanged { obj_id, func_group_id, time_left, .. } => vec![(*obj_id,*func_group_id,*time_left,false)],
            _ => continue,
        };
        for (obj_id,func_group_id,time_left,snapshot) in updates {
            views.doodad_announced(obj_id,time.elapsed_secs());
            let Some(entity)=views.doodad_entity(obj_id) else { views.retry_doodad(obj_id);continue };
            let Ok((_,mut visual,mut transform))=roots.get_mut(entity) else { continue };
            visual.func_group_id=func_group_id;
            visual.growth=db.doodad_growth(func_group_id);
            visual.remaining_ms=time_left as f32;
            visual.total_ms=visual.growth.map_or(0.0,|g|g.total_ms(visual.remaining_ms,snapshot));
            if !falling.contains(&obj_id) { transform.scale=Vec3::splat(visual.base_scale*visual.growth_scale()); }
            if !swaps.contains(&obj_id) { swaps.push_back(obj_id); }
        }
    }
    for _ in 0..MAX_SWAPS {
        // A cutdown tree keeps the model it was cut in until it lands: leave its swap queued
        // and keep the budget for the other doodads.
        let Some(obj_id) = next_swap(&mut swaps, &falling) else { break };
        // gone, or drawn again from scratch (which already used the current phase)
        let Some(entity) = views.doodad_entity(obj_id) else { continue };
        let Ok((remote, mut visual, mut transform)) = roots.get_mut(entity) else { continue };
        // the scale a falling tree waited with its model: both are the new phase's now
        transform.scale = Vec3::splat(visual.base_scale * visual.growth_scale());
        let Some(doodad) = doodads.by_id.get(&obj_id) else { continue };
        let func_group_id = visual.func_group_id;
        let Some(path) = db.doodad_phase_visual(doodad.template_id, func_group_id) else { continue };
        if path == visual.path {
            visual.rendered_func_group_id = func_group_id;
            continue;
        }
        match swap_model(&mut commands, &mut cry, entity, &path) {
            Some(spawned) => {
                let model = spawned.entity;
                // the old model goes only once the new one is there: no frame without a model
                if let Some(old) = visual.model.replace(model) {
                    commands.entity(old).despawn();
                }
                debug!("doodad {} phase {func_group_id}: model {path}", remote.obj_id);
                visual.path = path;
                visual.bond_helpers = spawned.helpers;
                visual.rendered_func_group_id = func_group_id;
            }
            None => warn!("doodad {} phase {func_group_id}: cannot draw {path}", remote.obj_id),
        }
    }
}

/// Counts the growth down and scales the doodads that grow (packets announce `time_left`, they do not tick it). A tree that
/// is falling or lying keeps the scale it was cut with: [`phase_changes`] gives it the next phase's scale with its model.
pub fn grow(
    time: Res<Time>,
    mut roots: Query<(
        &mut DoodadVisual,
        &mut Transform,
        Has<cutdown::Falling>,
        Has<cutdown::Fallen>,
    )>,
) {
    let dt_ms = time.delta_secs() * 1000.0;
    for (mut visual, mut transform, falling, fallen) in &mut roots {
        if visual.growth.is_none() {
            continue;
        }
        visual.remaining_ms = (visual.remaining_ms - dt_ms).max(0.0);
        if falling || fallen {
            continue;
        }
        let scale = visual.base_scale * visual.growth_scale();
        if (transform.scale.x - scale).abs() > 1e-4 {
            transform.scale = Vec3::splat(scale);
        }
    }
}

/// Loads `path` and puts its model under `root`; `None` (and the old model kept) when it cannot be loaded.
fn swap_model(commands: &mut Commands, cry: &mut CryAssets, root: Entity, path: &str) -> Option<super::doodad_model::Spawned> {
    let prepared = super::doodad_model::prepare(&cry.loader(), path)?;
    let spawned = super::doodad_model::spawn(commands, cry, prepared);
    commands.entity(spawned.entity).insert(ChildOf(root));
    Some(spawned)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn visual(model: Entity) -> DoodadVisual {
        DoodadVisual {
            path: String::new(),
            model: Some(model),
            func_group_id: 1,
            rendered_func_group_id: 1,
            bond_helpers: HashMap::new(),
            growth: None,
            remaining_ms: 0.0,
            total_ms: 0.0,
            base_scale: 1.0,
        }
    }

    #[test]
    fn illum_highlight_uses_a_private_material_and_keeps_the_base_texture() {
        let mut app = App::new();
        app.init_resource::<DoodadHighlights>()
            .init_resource::<Assets<StandardMaterial>>()
            .init_resource::<Assets<IllumMaterial>>()
            .add_systems(Update, highlight);
        let texture = Handle::<Image>::default();
        let mut full = IllumMaterial {
            base: StandardMaterial {
                base_color_texture: Some(texture.clone()),
                ..default()
            },
            ..default()
        };
        // Full shading: the native Illum composition reads its own emission and keeps the base maps.
        full.extension.params.vertex = Vec4::new(0.0, 1.0, 1.0, 0.0);
        let original = app
            .world_mut()
            .resource_mut::<Assets<IllumMaterial>>()
            .add(full);
        let root = app.world_mut().spawn(RemoteDoodad { obj_id: 7 }).id();
        let model = app.world_mut().spawn(ChildOf(root)).id();
        let mesh = app
            .world_mut()
            .spawn((MeshMaterial3d(original.clone()), ChildOf(model)))
            .id();
        let other = app.world_mut().spawn(MeshMaterial3d(original.clone())).id();
        app.world_mut().entity_mut(root).insert(visual(model));
        app.world_mut().resource_mut::<DoodadHighlights>().0.push(7);
        app.update();
        let highlighted = app
            .world()
            .get::<MeshMaterial3d<IllumMaterial>>(mesh)
            .unwrap()
            .0
            .clone();
        assert_ne!(highlighted, original);
        assert_eq!(
            app.world()
                .get::<MeshMaterial3d<IllumMaterial>>(other)
                .unwrap()
                .0,
            original
        );
        let materials = app.world().resource::<Assets<IllumMaterial>>();
        let lit = materials.get(&highlighted).unwrap();
        assert_eq!(
            lit.base.base_color_texture,
            Some(texture.clone()),
            "the highlighted copy keeps the diffuse map"
        );
        assert_eq!(lit.base.emissive_texture, Some(texture));
        assert_eq!(lit.base.emissive, LinearRgba::BLACK + GLOW);
        assert_eq!(
            lit.extension.params.emission,
            Vec4::new(GLOW.red, GLOW.green, GLOW.blue, 0.0)
        );
        app.update();
        assert_eq!(
            app.world()
                .get::<MeshMaterial3d<IllumMaterial>>(mesh)
                .unwrap()
                .0,
            highlighted
        );
        app.world_mut().resource_mut::<DoodadHighlights>().0.clear();
        app.update();
        assert!(app.world().get::<Glowing>(root).is_none());
        assert_eq!(
            app.world()
                .get::<MeshMaterial3d<IllumMaterial>>(mesh)
                .unwrap()
                .0,
            original
        );
    }

    #[test]
    fn standard_highlight_keeps_the_diffuse_map_and_restores_after_the_illum_swap() {
        let mut app = App::new();
        app.init_resource::<DoodadHighlights>()
            .init_resource::<Assets<StandardMaterial>>()
            .init_resource::<Assets<IllumMaterial>>()
            .add_systems(Update, highlight);
        let texture = Handle::<Image>::default();
        let original = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                base_color_texture: Some(texture.clone()),
                ..default()
            });
        let root = app.world_mut().spawn(RemoteDoodad { obj_id: 7 }).id();
        let model = app.world_mut().spawn(ChildOf(root)).id();
        let mesh = app
            .world_mut()
            .spawn((MeshMaterial3d(original.clone()), ChildOf(model)))
            .id();
        app.world_mut().entity_mut(root).insert(visual(model));
        app.world_mut().resource_mut::<DoodadHighlights>().0.push(7);
        app.update();
        let highlighted = app
            .world()
            .get::<MeshMaterial3d<StandardMaterial>>(mesh)
            .unwrap()
            .0
            .clone();
        assert_ne!(highlighted, original);
        {
            let materials = app.world().resource::<Assets<StandardMaterial>>();
            let lit = materials.get(&highlighted).unwrap();
            assert_eq!(
                lit.base_color_texture,
                Some(texture.clone()),
                "the highlighted copy keeps the diffuse map"
            );
            assert_eq!(
                lit.emissive_texture,
                Some(texture),
                "the glow follows the diffuse map instead of flattening it"
            );
            assert_eq!(lit.emissive, LinearRgba::BLACK + GLOW);
        }
        // aa-cry's swap_materials replaces a tagged StandardMaterial with its Illum mirror in PostUpdate.
        let mirror = app
            .world_mut()
            .resource_mut::<Assets<IllumMaterial>>()
            .add(IllumMaterial::default());
        app.world_mut()
            .entity_mut(mesh)
            .remove::<MeshMaterial3d<StandardMaterial>>()
            .insert(MeshMaterial3d::<IllumMaterial>(mirror));
        app.world_mut().resource_mut::<DoodadHighlights>().0.clear();
        app.update();
        assert_eq!(
            app.world()
                .get::<MeshMaterial3d<StandardMaterial>>(mesh)
                .unwrap()
                .0,
            original
        );
        assert!(
            app.world()
                .get::<MeshMaterial3d<IllumMaterial>>(mesh)
                .is_none()
        );
    }

    #[test]
    fn a_falling_tree_does_not_block_another_doodads_swap() {
        let mut queue = VecDeque::from([1, 2, 3]);
        let falling = HashSet::from([1]);
        assert_eq!(next_swap(&mut queue, &falling), Some(2));
        assert_eq!(next_swap(&mut queue, &falling), Some(3));
        assert_eq!(next_swap(&mut queue, &falling), None);
        assert_eq!(queue, VecDeque::from([1]), "the falling tree stays queued");
    }
}
