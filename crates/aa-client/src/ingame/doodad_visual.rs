//! Doodad phases and growth.
//!
//! A doodad is drawn with the model of its current phase (`doodad_func_groups.model`, falling back to
//! `doodad_almighties.model`) and, when the phase grows (`DoodadFuncGrowth`), scaled from `start_scale` to `end_scale` as
//! the server's `time_left` runs out (the packet's `time_left` is the phase's exact remaining time).
//! `SCDoodadPhaseChanged` moves a drawn doodad to another phase: the
//! model under the root is replaced in the same frame (the root, its position and its base scale stay), and a phase
//! whose model does not change only gets its new scale.

use aa_cry::scene::{CryAssets, ModelOptions, spawn_loaded};
use aa_net::state::Doodads;
use aa_net::{Event, NetEvent};
use bevy::prelude::*;

use super::data::{Db, DoodadGrowth};
use super::units::{RemoteDoodad, Views};

/// What a drawn doodad shows now and how the current phase grows.
#[derive(Component)]
pub struct DoodadVisual {
    /// The pak path of the drawn model.
    pub path: String,
    /// The model entity under the doodad root.
    pub model: Option<Entity>,
    /// The phase the drawn model belongs to.
    pub func_group_id: u32,
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
    app.add_systems(
        Update,
        (phase_changes, grow).chain().after(super::units::sync_views).run_if(super::world_screens),
    );
}

/// `SCDoodadPhaseChanged`: the new phase's model when it differs, and its growth from the announced `time_left`.
pub fn phase_changes(
    mut events: MessageReader<NetEvent>,
    mut commands: Commands,
    db: Option<Res<Db>>,
    doodads: Res<Doodads>,
    views: Res<Views>,
    mut roots: Query<(&RemoteDoodad, &mut DoodadVisual, &mut Transform)>,
    mut cry: CryAssets,
) {
    let Some(db) = db else { return };
    for NetEvent(event) in events.read() {
        let Event::DoodadPhaseChanged { obj_id, func_group_id, time_left, .. } = event else { continue };
        let Some(entity) = views.doodad_entity(*obj_id) else { continue };
        let Ok((remote, mut visual, mut transform)) = roots.get_mut(entity) else { continue };
        let Some(doodad) = doodads.by_id.get(obj_id) else { continue };
        visual.func_group_id = *func_group_id;
        visual.growth = db.doodad_growth(*func_group_id);
        visual.remaining_ms = *time_left as f32;
        // the phase change announces the whole remaining time of the new phase: the growth ends with it
        visual.total_ms = visual.growth.map_or(0.0, |g| g.total_ms(visual.remaining_ms, false));
        let wanted = db.doodad_phase_model(doodad.template_id, *func_group_id).filter(|p| *p != visual.path);
        if let Some(path) = wanted {
            match swap_model(&mut commands, &mut cry, entity, &path) {
                Some(model) => {
                    // the old model goes only once the new one is there: no frame without a model
                    if let Some(old) = visual.model.replace(model) {
                        commands.entity(old).despawn();
                    }
                    debug!("doodad {} phase {func_group_id}: model {path}", remote.obj_id);
                    visual.path = path;
                }
                None => warn!("doodad {} phase {func_group_id}: cannot draw {path}", remote.obj_id),
            }
        }
        // the new phase's size at once, in the frame the model changed
        transform.scale = Vec3::splat(visual.base_scale * visual.growth_scale());
    }
}

/// Counts the growth down and scales the doodads that grow (packets announce `time_left`, they do not tick it).
pub fn grow(time: Res<Time>, mut roots: Query<(&mut DoodadVisual, &mut Transform)>) {
    let dt_ms = time.delta_secs() * 1000.0;
    for (mut visual, mut transform) in &mut roots {
        if visual.growth.is_none() {
            continue;
        }
        visual.remaining_ms = (visual.remaining_ms - dt_ms).max(0.0);
        let scale = visual.base_scale * visual.growth_scale();
        if (transform.scale.x - scale).abs() > 1e-4 {
            transform.scale = Vec3::splat(scale);
        }
    }
}

/// Loads `path` and puts its model under `root`; `None` (and the old model kept) when it cannot be loaded.
fn swap_model(commands: &mut Commands, cry: &mut CryAssets, root: Entity, path: &str) -> Option<Entity> {
    let options = ModelOptions::default();
    let loader = cry.loader();
    let prepared = loader.prepare_model(path, &options).ok()?;
    let loaded = cry.finish_model(prepared);
    let model = spawn_loaded(commands, path, &loaded, &options);
    commands.entity(model).insert(ChildOf(root));
    Some(model)
}
