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
use bevy::math::Affine3A;
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

/// The local-space bounds (cry space) of the prepared parts, each in its prefab placement:
/// `FUN_3908d400`'s model box for the builder's terrain check and spot height.
pub fn bounds(parts: &[PreparedPart]) -> Option<(Vec3, Vec3)> {
    let mut out: Option<(Vec3, Vec3)> = None;
    for PreparedPart { part, prepared } in parts {
        let Some((lo, hi)) = prepared.loaded().model.bounds() else { continue };
        let placement = Affine3A::from_scale_rotation_translation(part.scale, part.rot, part.pos);
        for i in 0..8 {
            let corner = Vec3::new(
                if i & 1 == 0 { lo.x } else { hi.x },
                if i & 2 == 0 { lo.y } else { hi.y },
                if i & 4 == 0 { lo.z } else { hi.z },
            );
            let p = placement.transform_point3(corner);
            out = Some(match out {
                None => (p, p),
                Some((a, b)) => (a.min(p), b.max(p)),
            });
        }
    }
    out
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
/// A house the server demolished draws nothing (`SCHouseDemolished`, `FUN_3932A300` sets its demolished flag).
pub fn model_of(unit: &Unit, houses: &Houses) -> Option<u32> {
    let UnitKind::Housing {
        tl_id,
        template_id,
        build_step,
    } = unit.kind
    else {
        return None;
    };
    let house = houses.by_tl_id.get(&tl_id);
    if house.is_some_and(|h| h.demolished) {
        return None;
    }
    let snapshot = house.map(|h| h.snapshot.model_id).filter(|m| *m != 0);
    snapshot
        .or((unit.model_id != 0).then_some(unit.model_id))
        .or_else(|| data::get().and_then(|d| d.model_at_step(template_id, i32::from(build_step))))
}

struct Drawn {
    model_id: u32,
    entity: Option<Entity>,
    /// The drawn root's placement: the unit's position and yaw, scale 1 (the house root, not the unit's scale).
    transform: Option<Transform>,
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

    /// The root transform of the drawn house `obj_id`, if it is drawn. House-parented doodads use this when the
    /// housing unit is no longer in `Units`.
    pub fn transform(&self, obj_id: u32) -> Option<Transform> {
        self.houses
            .get(&obj_id)
            .filter(|d| d.entity.is_some())
            .and_then(|d| d.transform)
    }

    #[cfg(test)]
    pub(crate) fn fixture_house(&mut self, obj_id: u32, transform: Transform) {
        self.houses.insert(
            obj_id,
            Drawn {
                model_id: 0,
                entity: Some(Entity::PLACEHOLDER),
                transform: Some(transform),
            },
        );
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
        let transform = placement(pos, unit.yaw(), 1.0);
        let root = commands
            .spawn((Name::new(format!("house {obj_id} {}", unit.name)), HouseModel { obj_id, model_id }, transform, Visibility::default()))
            .id();
        spawn_parts(&mut commands, &mut cry, root, parts, &ModelOptions::default());
        if let Some(old) = d.entity.replace(root) {
            commands.entity(old).despawn();
        }
        d.transform = Some(transform);
        info!("housing: house {obj_id} {:?} drawn with model {model_id} at ({:.1}, {:.1}, {:.1})", unit.name, pos.x, pos.y, pos.z);
    }
    // gone, demolished or too far
    let gone: Vec<u32> = drawn
        .houses
        .keys()
        .filter(|id| drops(**id, &units, &houses, me))
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
        let d = drawn.houses.entry(unit.obj_id).or_insert(Drawn { model_id: 0, entity: None, transform: None });
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

/// Whether the drawn house `obj_id` leaves the scene: its unit is gone, the server demolished the
/// house (`SCHouseDemolished`), or it is beyond `HOUSE_RADIUS * 1.2`.
fn drops(obj_id: u32, units: &Units, houses: &Houses, me: Vec3) -> bool {
    units.get(obj_id).is_none_or(|u| {
        flat(u, me) > HOUSE_RADIUS * 1.2
            || matches!(u.kind, UnitKind::Housing { tl_id, .. } if houses.by_tl_id.get(&tl_id).is_some_and(|h| h.demolished))
    })
}

/// Leaving the game drops every drawn house.
pub fn clear(mut commands: Commands, mut drawn: ResMut<DrawnHouses>) {
    for (_, d) in drawn.houses.drain() {
        if let Some(e) = d.entity {
            commands.entity(e).despawn();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aa_net::Pos;
    use aa_net::housing::{House, HouseSnapshot};
    use aa_net::model::{Equipment, ModelParams};

    fn unit(obj_id: u32, tl_id: u16, model_id: u32, x: f32) -> Unit {
        Unit {
            bounty_money: None,
            pvp_honor: 0,
            pvp_kills: 0,
            obj_id,
            name: "Farm".into(),
            kind: UnitKind::Housing {
                tl_id,
                template_id: 171,
                build_step: -1,
            },
            owner_name: "Owner".into(),
            pos: Pos::new(x, 0.0, 0.0),
            rotation: [0.0; 3],
            scale: 1.0,
            level: 1,
            model_id,
            equipment: Equipment::default(),
            model_params: ModelParams::default(),
            hp: 100,
            mp: 0,
            max_hp: 100,
            max_mp: 0,
            dead: false,
            race_gender: 0,
            movement: None,
            faction_id: 0,
            expedition_id: 0,
            appellation: 0,
            flags: 0,
            gm_modes: [0; 4],
            skills: vec![],
            passive_buffs: vec![],
            ability_exp: vec![],
            active_abilities: vec![],
            buffs: vec![],
            anim_action: 0,
            house_posture: None,
            turret: None,
            state_attachments: None,
            attached_to: None,
            doodad_bond: None,
        }
    }

    fn houses(tl_id: u16, snapshot_model: u32) -> Houses {
        let mut houses = Houses::default();
        houses.by_tl_id.insert(
            tl_id,
            House {
                snapshot: HouseSnapshot {
                    tl_id,
                    db_id: u32::from(tl_id),
                    obj_id: 900,
                    template_id: 171,
                    model_id: snapshot_model,
                    co_owner_id: 0,
                    owner_id: 42,
                    owner_name: "Owner".into(),
                    account_id: 17,
                    permission: 0,
                    all_actions: 0,
                    current_actions: 0,
                    tax: 0,
                    x: 0,
                    y: 0,
                    z: 0.0,
                    name: "Farm".into(),
                    allow_recover: true,
                    sell_price: 0,
                    sell_to_player_id: 0,
                    sell_to_name: String::new(),
                },
                demolished: false,
                build_pay: None,
                tax: None,
                sale: None,
            },
        );
        houses
    }

    #[test]
    fn a_demolished_house_draws_nothing_and_leaves_the_scene() {
        let mut units = Units::default();
        units.by_id.insert(900, unit(900, 7, 1083, 0.0));
        let mut houses = houses(7, 1082);
        assert_eq!(
            model_of(&units.by_id[&900], &houses),
            Some(1082),
            "the received state's model"
        );
        // Without a snapshot the unit's own model stands.
        houses.by_tl_id.get_mut(&7).unwrap().demolished = false;
        houses.by_tl_id.get_mut(&7).unwrap().snapshot.model_id = 0;
        assert_eq!(
            model_of(&units.by_id[&900], &houses),
            Some(1083),
            "the unit's model"
        );
        // `SCHouseDemolished`: neither the state nor the unit's model is drawn, and the entity drops.
        houses.by_tl_id.get_mut(&7).unwrap().snapshot.model_id = 1082;
        houses.by_tl_id.get_mut(&7).unwrap().demolished = true;
        assert_eq!(model_of(&units.by_id[&900], &houses), None);
        assert!(drops(900, &units, &houses, Vec3::ZERO));
        houses.by_tl_id.get_mut(&7).unwrap().demolished = false;
        assert!(!drops(900, &units, &houses, Vec3::ZERO));
        // Gone unit and out of range still drop.
        assert!(drops(901, &units, &houses, Vec3::ZERO));
        assert!(drops(
            900,
            &units,
            &houses,
            Vec3::new(HOUSE_RADIUS * 1.3, 0.0, 0.0)
        ));
    }
}
