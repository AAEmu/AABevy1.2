//! Houses the server announces (`SCUnitState` of kind housing, any owner): drawn from their prefab, at the model of their
//! current construction stage, and drawn again when a build action moves them to the next stage (`SCHouseBuildProgress`
//! carries the stage's model) or the house is finished.
//!
//! `units.rs` leaves housing units alone; this module keeps one entity per house within [`HOUSE_RADIUS`]. The parts
//! are prepared on the workers like the units' models.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};

use aa_cry::PreparedModel;
use aa_cry::scene::{CryAssets, ModelOptions, spawn_loaded};
use aa_cry::workers::Workers;
use aa_net::Houses;
use aa_net::model::{Unit, UnitKind};
use aa_net::state::Units;
use aa_world::coords::cry_rot_to_bevy;
use aa_world::cry_to_bevy;
use bevy::prelude::*;

use super::data;
use super::prefab::{self, Part};
use crate::ingame::player::{LocalPlayer, placement};

/// Houses are drawn within this many metres of the player (dropped again beyond `* 1.2`).
pub const HOUSE_RADIUS: f32 = 400.0;

/// A drawn house.
#[derive(Component)]
pub struct HouseModel {
    pub obj_id: u32,
    pub model_id: u32,
}

/// The model of a prepared prefab part.
pub struct PreparedPart {
    pub part: Part,
    pub prepared: PreparedModel,
}

/// Prepares the parts of the prefab of `model_id` (callable from any thread). Parts whose model cannot be read are
/// left out.
pub fn prepare(loader: &aa_cry::CryLoader, model_id: u32, options: &ModelOptions) -> Vec<PreparedPart> {
    let Some(element) = data::get().and_then(|d| d.prefabs.get(&model_id)) else { return Vec::new() };
    prefab::load(element)
        .into_iter()
        .filter_map(|part| {
            let options = ModelOptions { material: part.material.clone().filter(|m| aa_pak::Pak::global().exists(m)), ..options.clone() };
            match loader.prepare_model(&part.model, &options) {
                Ok(prepared) => Some(PreparedPart { part, prepared }),
                Err(e) => {
                    debug!("housing: part {} of model {model_id}: {e}", part.model);
                    None
                }
            }
        })
        .collect()
}

/// Spawns prepared parts under `root` (a Bevy-space entity at the prefab's origin).
pub fn spawn_parts(commands: &mut Commands, cry: &mut CryAssets, root: Entity, parts: Vec<PreparedPart>, options: &ModelOptions) {
    for PreparedPart { part, prepared } in parts {
        let loaded = cry.finish_model(prepared);
        let model = spawn_loaded(commands, &part.model, &loaded, options);
        let transform = Transform {
            translation: cry_to_bevy(part.pos),
            rotation: cry_rot_to_bevy(part.rot),
            scale: Vec3::new(part.scale.x, part.scale.z, part.scale.y),
        };
        let holder = commands.spawn((transform, Visibility::default(), ChildOf(root))).id();
        commands.entity(model).insert(ChildOf(holder));
    }
}

/// The model a house unit shows now: the received house state's (it follows the build progress), else the unit's.
pub fn model_of(unit: &Unit, houses: &Houses) -> Option<u32> {
    let UnitKind::Housing { tl_id, template_id, build_step } = unit.kind else { return None };
    let snapshot = houses.by_tl_id.get(&tl_id).filter(|h| !h.demolished).map(|h| h.snapshot.model_id).filter(|m| *m != 0);
    snapshot
        .or((unit.model_id != 0).then_some(unit.model_id))
        .or_else(|| data::get().and_then(|d| d.model_at_step(template_id, i32::from(build_step))))
}

struct Drawn {
    model_id: u32,
    entity: Option<Entity>,
}

#[derive(Resource)]
pub struct DrawnHouses {
    houses: HashMap<u32, Drawn>,
    tx: Sender<(u32, u32, Vec<PreparedPart>)>,
    rx: Mutex<Receiver<(u32, u32, Vec<PreparedPart>)>>,
}

impl Default for DrawnHouses {
    fn default() -> Self {
        let (tx, rx) = channel();
        DrawnHouses { houses: HashMap::new(), tx, rx: Mutex::new(rx) }
    }
}

impl DrawnHouses {
    /// The entity of the house unit `obj_id`, if it is drawn.
    pub fn entity(&self, obj_id: u32) -> Option<Entity> {
        self.houses.get(&obj_id).and_then(|d| d.entity)
    }
}

/// Keeps one entity per house unit near the player, at the model of its construction stage.
pub fn sync_houses(
    mut commands: Commands,
    mut cry: CryAssets,
    mut drawn: ResMut<DrawnHouses>,
    units: Res<Units>,
    houses: Res<Houses>,
    player: Query<&Transform, With<LocalPlayer>>,
) {
    let Some(me) = player.single().ok().map(|t| aa_world::bevy_to_cry(t.translation)) else { return };
    let arrived: Vec<_> = {
        let rx = drawn.rx.lock().unwrap_or_else(|e| e.into_inner());
        std::iter::from_fn(|| rx.try_recv().ok()).collect()
    };
    for (obj_id, model_id, parts) in arrived {
        let Some(unit) = units.get(obj_id) else { continue };
        let Some(d) = drawn.houses.get_mut(&obj_id) else { continue };
        if d.model_id != model_id {
            continue; // the stage changed while the worker ran: the newer job comes
        }
        if parts.is_empty() {
            warn!("housing: house {obj_id} {:?} (model {model_id}) has nothing to draw", unit.name);
        }
        let pos = Vec3::new(unit.pos.x, unit.pos.y, unit.pos.z);
        let root = commands
            .spawn((Name::new(format!("house {obj_id} {}", unit.name)), HouseModel { obj_id, model_id }, placement(pos, unit.yaw(), 1.0), Visibility::default()))
            .id();
        spawn_parts(&mut commands, &mut cry, root, parts, &ModelOptions::default());
        if let Some(old) = d.entity.replace(root) {
            commands.entity(old).despawn();
        }
        info!("housing: house {obj_id} {:?} drawn with model {model_id} at ({:.1}, {:.1}, {:.1})", unit.name, pos.x, pos.y, pos.z);
    }
    // gone, or too far
    let gone: Vec<u32> = drawn
        .houses
        .keys()
        .filter(|id| units.get(**id).is_none_or(|u| flat(u, me) > HOUSE_RADIUS * 1.2))
        .copied()
        .collect();
    for id in gone {
        if let Some(Drawn { entity: Some(e), .. }) = drawn.houses.remove(&id) {
            commands.entity(e).despawn();
        }
    }
    let loader = cry.loader();
    for unit in units.iter().filter(|u| matches!(u.kind, UnitKind::Housing { .. })) {
        if flat(unit, me) > HOUSE_RADIUS {
            continue;
        }
        let Some(model_id) = model_of(unit, &houses) else { continue };
        let d = drawn.houses.entry(unit.obj_id).or_insert(Drawn { model_id: 0, entity: None });
        if d.model_id == model_id {
            continue;
        }
        d.model_id = model_id;
        let (obj_id, loader, tx) = (unit.obj_id, loader.clone(), drawn.tx.clone());
        Workers::global().spawn(flat(unit, me), move || {
            let parts = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| prepare(&loader, model_id, &ModelOptions::default()))).unwrap_or_default();
            let _ = tx.send((obj_id, model_id, parts));
        });
    }
}

fn flat(unit: &Unit, me: Vec3) -> f32 {
    Vec2::new(unit.pos.x - me.x, unit.pos.y - me.y).length()
}

/// Leaving the game drops every drawn house.
pub fn clear(mut commands: Commands, mut drawn: ResMut<DrawnHouses>) {
    for (_, d) in drawn.houses.drain() {
        if let Some(e) = d.entity {
            commands.entity(e).despawn();
        }
    }
}
