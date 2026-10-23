//! Planting: the items that place a doodad (seeds, saplings, livestock, ladders, flags ... every item with a row in
//! `item_spawn_doodads`) are aimed at the ground like a ground skill and send `CSCreateDoodad` instead of a cast.
//!
//! The 1.2 client enters a placement mode when such an item is used: the doodad's model (its start phase) follows the cursor over
//! the ground, a red ring and the refusal text (`SIM_DOODAD_MSG`, the `doodad_sim_*` strings) show where it cannot stand, a left
//! click places it (`CSCreateDoodad`: template, position, rotation, scale, the item's id), the right click or `Escape` cancels.
//! This is the same aiming as a ground skill's ([`super::Aiming`]: spot under the cursor, ring, click, cancel), so it only adds
//! what is specific to a doodad: which items it is for ([`begin`]), the ghost model ([`ghost`]), whether the spot is free
//! ([`check`]) and the packet ([`place`]).
//!
//! The server checks what it can (the item, the farm, the house land, the labor, 30 m from the player). The client only keeps the
//! spot free of other doodads (`sim_radius`), of other characters and creatures, and of water.

use aa_cry::scene::{CryAssets, ModelOptions, spawn_loaded};
use aa_net::farm::CreateDoodad;
use aa_net::state::{Doodads, Units};
use aa_net::{Command, NetCommand, Pos};
use aa_world::coords::cry_rot_to_bevy;
use aa_world::{WaterQuery, cry_to_bevy};
use bevy::prelude::*;

use super::{Aim, Aiming, GroundSkill, Mode, SkillPressed};
use crate::ingame::data::Db;
use crate::ingame::player::{LocalPlayer, Mover};

/// How far from the character a doodad may be placed (the server refuses beyond 30 m), metres.
const MAX_RANGE: f32 = 20.0;
/// How near another character or creature may stand to the spot, metres.
const UNIT_ROOM: f32 = 0.7;
/// The refusals (`ui_texts` 278 / 143) and the all clear (281) the placement mode shows (`doodad_sim_*`).
const TOO_CLOSE: &str = "Too close to other objects.";
const WATER: &str = "Too close to the water.";
const CAN_BE_PLACED: &str = "Can be placed.";

/// What an aimed item places.
#[derive(Clone, Debug)]
pub(super) struct Plant {
    /// The item's id and the doodad template it places.
    pub item_id: u64,
    pub doodad: u32,
    /// The model of the doodad's start phase and the room it keeps around it, metres.
    pub model: Option<String>,
    pub radius: f32,
    /// Why the spot under the cursor cannot be used (`None`: it can).
    pub blocked: Option<&'static str>,
}

/// The aiming of an item that places a doodad; `None` for every other press.
pub(super) fn begin(press: &SkillPressed, db: &Db) -> Option<(GroundSkill, Plant)> {
    let (item_id, template) = press.item?;
    let doodad = db.item_doodad(template)?;
    let (model, radius) = db.doodad_start(doodad);
    let skill = GroundSkill { mode: Mode::Ground, radius: radius.max(0.5), min_range: 0.0, max_range: MAX_RANGE };
    info!("plant: item {template} places doodad {doodad} ({model:?}, room {radius:.1} m)");
    Some((skill, Plant { item_id, doodad, model, radius, blocked: None }))
}

/// Why `spot` cannot hold a doodad that keeps `radius` metres of room, or `None`.
pub(super) fn check(spot: Vec3, radius: f32, own: u32, doodads: &Doodads, units: &Units, water: Option<&WaterQuery>) -> Option<&'static str> {
    let flat = spot.truncate();
    let near = |p: aa_net::Pos, room: f32| flat.distance(Vec2::new(p.x, p.y)) < room && (p.z - spot.z).abs() < 3.0;
    if water.and_then(|w| w.water_at(spot)).is_some_and(|(height, _)| height > spot.z + 0.3) {
        return Some(WATER);
    }
    if doodads.by_id.values().any(|d| near(d.pos, radius)) || units.iter().any(|u| u.obj_id != own && !u.dead && near(u.pos, UNIT_ROOM)) {
        return Some(TOO_CLOSE);
    }
    None
}

/// Keeps `Plant::blocked` up to date and tells the HUD (`SIM_DOODAD_MSG`) what the spot under the cursor lets the player do.
pub(super) fn validate(
    mut aiming: ResMut<Aiming>,
    doodads: Res<Doodads>,
    units: Res<Units>,
    water: Option<Res<WaterQuery>>,
    player: Res<aa_net::state::Player>,
    rt: Option<NonSend<aa_ui::UiRuntime>>,
    mut shown: Local<Option<&'static str>>,
) {
    let own = player.spawn.as_ref().map_or(0, |s| s.obj_id);
    let text = match aiming.0.as_mut() {
        Some(Aim { plant: Some(plant), spot, .. }) => spot.as_mut().map(|(spot, ok)| {
            plant.blocked = check(*spot, plant.radius, own, &doodads, &units, water.as_deref());
            // the ring is red for a spot that is not free
            *ok &= plant.blocked.is_none();
            plant.blocked.unwrap_or(if *ok { CAN_BE_PLACED } else { "Can't be placed." })
        }),
        _ => None,
    };
    if *shown == text {
        return;
    }
    *shown = text;
    if let Some(rt) = rt {
        match text {
            Some(text) => rt.fire_event("SIM_DOODAD_MSG", text),
            None => rt.fire_event("SIM_DOODAD_MSG", ()),
        }
    }
}

/// The left click: `CSCreateDoodad` at the spot. `false` (nothing sent, the mode goes on) when the spot is not free.
pub(super) fn place(plant: &Plant, spot: Vec3, mover: &Mover, net: &mut MessageWriter<NetCommand>) -> bool {
    if let Some(why) = plant.blocked {
        info!("plant: ({:.1}, {:.1}, {:.1}) is not free: {why}", spot.x, spot.y, spot.z);
        return false;
    }
    // facing the character, like the doodads a GM spawns
    let z_rot = mover.facing + std::f32::consts::PI;
    net.write(NetCommand(Command::CreateDoodad(CreateDoodad { id: plant.doodad, pos: Pos::new(spot.x, spot.y, spot.z), z_rot, scale: 1.0, item_id: plant.item_id })));
    info!("plant: place doodad {} with item {} at ({:.1}, {:.1}, {:.1}), {:.1} m away", plant.doodad, plant.item_id, spot.x, spot.y, spot.z, mover.pos.distance(spot));
    true
}

/// The model that follows the cursor.
#[derive(Component)]
pub(super) struct Ghost;

/// Keeps the ghost model on the spot under the cursor while an item is aimed, and removes it after.
#[allow(clippy::type_complexity)]
pub(super) fn ghost(
    mut commands: Commands,
    aiming: Res<Aiming>,
    mut cry: CryAssets,
    mut ghosts: Query<(Entity, &mut Transform, &mut Visibility), With<Ghost>>,
    player: Query<&Mover, With<LocalPlayer>>,
    mut drawn: Local<Option<(u32, Option<Entity>)>>,
) {
    let wanted = aiming.0.as_ref().and_then(|a| a.plant.as_ref().map(|p| (p, a.spot)));
    let Some((plant, spot)) = wanted else {
        if drawn.take().is_some() {
            for (e, ..) in &ghosts {
                commands.entity(e).despawn();
            }
        }
        return;
    };
    // the model is built once per doodad
    if drawn.is_none_or(|(d, _)| d != plant.doodad) {
        for (e, ..) in &ghosts {
            commands.entity(e).despawn();
        }
        let root = plant.model.as_deref().and_then(|path| {
            let options = ModelOptions::default();
            let prepared = cry.loader().prepare_model(path, &options).ok()?;
            let loaded = cry.finish_model(prepared);
            let model = spawn_loaded(&mut commands, path, &loaded, &options);
            let root = commands.spawn((Name::new("placing doodad"), Ghost, Transform::IDENTITY, Visibility::Hidden)).id();
            commands.entity(model).insert(ChildOf(root));
            Some(root)
        });
        *drawn = Some((plant.doodad, root));
        return;
    }
    let Some((_, Some(_))) = *drawn else { return };
    let facing = player.single().map_or(0.0, |m| m.facing + std::f32::consts::PI);
    for (_, mut transform, mut visibility) in &mut ghosts {
        match spot {
            Some((spot, _)) => {
                transform.translation = cry_to_bevy(spot);
                transform.rotation = cry_rot_to_bevy(Quat::from_rotation_z(facing));
                *visibility = Visibility::Inherited;
            }
            None => *visibility = Visibility::Hidden,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_doodad_keeps_its_room() {
        let mut doodads = Doodads::default();
        doodads.by_id.insert(1, aa_net::model::Doodad { obj_id: 1, pos: Pos::new(10.0, 10.0, 5.0), ..test_doodad() });
        let units = Units::default();
        assert_eq!(check(Vec3::new(10.3, 10.0, 5.0), 0.5, 0, &doodads, &units, None), Some(TOO_CLOSE));
        assert_eq!(check(Vec3::new(11.0, 10.0, 5.0), 0.5, 0, &doodads, &units, None), None);
        // another floor is not in the way
        assert_eq!(check(Vec3::new(10.3, 10.0, 12.0), 0.5, 0, &doodads, &units, None), None);
    }

    fn test_doodad() -> aa_net::model::Doodad {
        aa_net::model::Doodad {
            obj_id: 0,
            template_id: 0,
            owner_obj_id: 0,
            parent_obj_id: 0,
            attach_point: 0,
            pos: Pos::new(0.0, 0.0, 0.0),
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: 1.0,
            has_loot_item: false,
            func_group_id: 0,
            owner_id: 0,
            ucc_id: 0,
            item_template_id: 0,
            type2: 0,
            time_left: 0,
            plant_time: 0,
            quest_glow: 0,
            family: 0,
            puzzle_group: 0,
            owner_type: 0,
            owner_db_id: 0,
            data: 0,
        }
    }
}
