//! One received, uniquely correlated current mate. Local commands never create a pet/unit.
use super::*;
use aa_net::Mates;
use aa_net::mate::MateSpawn;
use aa_net::model::{Unit, UnitKind};
use mlua::MultiValue;
use std::collections::HashMap;

#[derive(Default)]
pub(super) struct Tables {
    levels: Vec<(u32, i32)>,
    skills: HashMap<u32, Skill>,
    seats: HashMap<u32, (bool, bool)>,
    equipment: HashMap<u32, [bool; 4]>,
}

struct Skill {
    combat: u32,
    first_seat: Option<u32>,
    level: i32,
    target_type: u32,
}

impl Tables {
    pub(super) fn load(conn: &Connection) -> rusqlite::Result<Self> {
        let mut out = Self::default();
        let mut query = conn.prepare("SELECT n.id,p.head,p.chest,p.waist,p.feet FROM npcs n JOIN mate_equip_slot_packs p ON p.id=n.mate_equip_slot_pack_id")?;
        let mut rows = query.query([])?;
        while let Some(row) = rows.next()? {
            out.equipment.insert(
                any_int(row, 0)? as u32,
                [
                    any_flag(row, 1)?,
                    any_flag(row, 2)?,
                    any_flag(row, 3)?,
                    any_flag(row, 4)?,
                ],
            );
        }
        let mut query = conn.prepare("SELECT id,total_mate_exp FROM levels ORDER BY id")?;
        let mut rows = query.query([])?;
        while let Some(row) = rows.next()? {
            out.levels.push((any_int(row, 0)? as u32, any_int(row, 1)? as i32));
        }
        let mut first = HashMap::new();
        let mut query = conn.prepare("SELECT id,attach_point_id,mount_skill_id,skill_id FROM mount_attached_skills")?;
        let mut rows = query.query([])?;
        while let Some(row) = rows.next()? {
            first.entry(any_int(row, 2)? as u32).or_insert(any_int(row, 1)? as u32);
        }
        let mut query =
            conn.prepare("SELECT m.id,m.skill_id,s.ability_level,s.target_type_id FROM mount_skills m INNER JOIN skills s ON s.id=m.skill_id ORDER BY m.id")?;
        let mut rows = query.query([])?;
        while let Some(row) = rows.next()? {
            let id = any_int(row, 0)? as u32;
            out.skills.insert(
                id,
                Skill {
                    combat: any_int(row, 1)? as u32,
                    first_seat: first.get(&id).copied(),
                    level: any_int(row, 2)? as i32,
                    target_type: any_int(row, 3)? as u32,
                },
            );
        }
        let mut query = conn.prepare("SELECT owner_id,attach_point_id FROM model_bindings WHERE owner_type='Model' ORDER BY id")?;
        let mut rows = query.query([])?;
        while let Some(row) = rows.next()? {
            let point = any_int(row, 1)?;
            let entry = out.seats.entry(any_int(row, 0)? as u32).or_default();
            if point == 1 {
                entry.0 = true;
            } else if point == 2 {
                entry.1 = true;
            }
        }
        Ok(out)
    }

    fn progress(&self, experience: i32) -> Option<[f64; 4]> {
        let mut level = 1;
        for &(next, total) in &self.levels {
            if next > 1 && next <= 100 && experience >= total {
                level = next;
            }
        }
        let start = self.levels.iter().find(|row| row.0 == level)?.1;
        let next = self.levels.iter().find(|row| row.0 == level + 1)?.1;
        let needed = next.wrapping_sub(start);
        let raw = (experience.wrapping_sub(start) as f32) / (needed as f32) * 100.0f32;
        let upper = ((raw + 100.0f32) - (raw - 100.0f32).abs()) * 0.5f32;
        let upper = f64::from(upper);
        let percent = ((upper + upper.abs()) * 0.5) as f32;
        Some([
            f64::from(percent),
            f64::from(experience as f32),
            f64::from(start as f32),
            f64::from(needed as f32),
        ])
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Pet {
    spawn: MateSpawn,
    object: u32,
    model: u32,
    template: u32,
    equipment: aa_net::model::Equipment,
    name: String,
    owner_name: String,
    level: u8,
    hp: i32,
    max_hp: i32,
    mp: i32,
    max_mp: i32,
    dead: bool,
    combat: bool,
    buffs: super::super::panels::BuffLists,
    mounted: bool,
    attached_to_own: bool,
    driving: bool,
    passenger: bool,
    info: Option<super::super::stats::MateDetails>,
}

impl Pet {
    fn identity(&self) -> (u32, u16, u32) {
        (self.object, self.spawn.tl_id, self.spawn.id)
    }
}

/// `DismissPet` waits for the server detach and then the native 1000 ms unmount controller. `deadline` is a safety
/// bound (not a 1.2 value) so a server that never detaches cannot leave the dismiss pending for a later manual unmount.
struct PendingDismiss {
    identity: (u32, u16, u32),
    detached: Option<web_time::Instant>,
    deadline: web_time::Instant,
}

/// A pet-bar slot cast waiting for [`commands`] to check the pet is still the same one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PendingCast {
    skill_id: u32,
    mount_skill_id: u32,
    target_obj_id: u32,
}

#[derive(Default)]
pub(super) struct View {
    current: Option<Pet>,
    pushed: Option<Pet>,
    local_state: Option<((u32, u16, u32), u8)>,
    // Native +58 starts at0; original local action/AI planner is not wired yet.
    commanded_skill: u32,
    mounted: bool,
    /// Where the current pet is and its yaw in radians (kept out of [`Pet`]: they change every frame the pet moves, and
    /// a change of [`Pet`] rebuilds the pet frame) and when the last mount or dismount was asked for. The native mount
    /// reason uses the entity quaternion, which is yaw-only on level ground (the captured cases in
    /// tools/evidence/mount_reason).
    pet_pos: Option<(Vec3, f32)>,
    last_ride: Option<web_time::Instant>,
    /// `DismissPet` removal waits for the server detach and then the native 1000 ms unmount controller.
    pending_dismiss: Option<PendingDismiss>,
    requests: Vec<(u32, (u32, u16, u32), MateCommand)>,
    casts: Vec<(u32, (u32, u16, u32), PendingCast)>,
}

fn current<'a>(mates: &Mates, units: &'a Units, owner: u32) -> Option<(&'a Unit, MateSpawn)> {
    let owned = mates.own.as_ref()?;
    let unit = units.get(owned.obj_id?)?;
    if owner == 0 || !matches!(unit.kind,UnitKind::Mate { tl_id,owner_id,.. } if tl_id!=0 && tl_id==owned.spawn.tl_id && owner_id==owner) {
        return None;
    }
    Some((unit, owned.spawn.clone()))
}

pub(super) fn refresh(world: &World, data: &mut HudData) {
    data.pets.owned.mounted = (|| {
        let units = world.get_resource::<Units>()?;
        let player = world.get_resource::<Player>()?.spawn.as_ref()?;
        let rider = units.get(player.obj_id).unwrap_or(&player.unit);
        Some(
            rider
                .attached_to
                .and_then(|a| units.get(a.obj_id))
                .is_some_and(|u| matches!(u.kind, UnitKind::Mate { .. })),
        )
    })()
    .unwrap_or(false);
    let next = (|| {
        let units = world.get_resource::<Units>()?;
        let mates = world.get_resource::<Mates>()?;
        let (unit, spawn) = current(mates, units, data.character_id)?;
        let game = data.game.get();
        let buffs = world.get_resource::<aa_net::state::Buffs>().map_or(&[][..], |b| b.of(unit.obj_id));
        let native_buffs = world.get_resource::<aa_net::state::Buffs>().and_then(|b| b.native.of(unit.obj_id));
        let (max_hp, max_mp) = game
            .and_then(|g| super::super::stats::mate_max(g, unit, native_buffs))
            .unwrap_or((unit.max_hp, unit.max_mp));
        let me = world.get_resource::<Player>()?.spawn.as_ref()?;
        let rider = units.get(me.obj_id).unwrap_or(&me.unit);
        let mounted = rider
            .attached_to
            .and_then(|a| units.get(a.obj_id))
            .is_some_and(|u| matches!(u.kind, UnitKind::Mate { .. }));
        let attached_to_own = rider.attached_to.is_some_and(|a| a.obj_id == unit.obj_id);
        let driving = rider.attached_to.is_some_and(|a| a.obj_id == unit.obj_id && a.point == 1);
        let passenger = units.iter().any(|u| u.attached_to.is_some_and(|a| a.obj_id == unit.obj_id && a.point == 2));
        let name = if unit.name.is_empty() {
            game.and_then(|g| g.npcs.get(&unit.kind.template_id()))
                .map(|n| n.name.clone())
                .unwrap_or_default()
        } else {
            unit.name.clone()
        };
        let combat = data
            .pets
            .owned
            .current
            .as_ref()
            .filter(|p| p.object == unit.obj_id && p.spawn.tl_id == spawn.tl_id && p.spawn.id == spawn.id)
            .is_some_and(|p| p.combat);
        Some(Pet {
            spawn,
            object: unit.obj_id,
            model: unit.model_id,
            template: unit.kind.template_id(),
            equipment: unit.equipment.clone(),
            name,
            owner_name: unit.owner_name.clone(),
            level: unit.level,
            hp: unit.hp,
            max_hp: max_hp.max(unit.hp).max(1),
            mp: unit.mp,
            max_mp: max_mp.max(unit.mp),
            dead: unit.dead,
            combat,
            buffs: super::super::panels::buff_lists(game, buffs, data.start),
            mounted,
            attached_to_own,
            driving,
            passenger,
            info: game.and_then(|game| super::super::stats::mate_details(game, unit, native_buffs)),
        })
    })();
    if data.pets.owned.current.as_ref().map(Pet::identity) != next.as_ref().map(Pet::identity) {
        data.pets.owned.local_state = None;
        data.pets.owned.commanded_skill = 0;
        data.pets.owned.casts.clear();
    }
    data.pets.owned.current = next;
    data.pets.owned.pet_pos = (|| {
        let unit = current(world.get_resource::<Mates>()?, world.get_resource::<Units>()?, data.character_id)?.0;
        Some((Vec3::new(unit.pos.x, unit.pos.y, unit.pos.z), unit.rotation[2]))
    })();
}

/// The longest way to the pet a mount request is sent from (m), and the wait between two requests (s): `CanMount`'s
/// squared distance 625 and two-second cooldown (`3906a440`).
const MOUNT_REACH: f32 = 25.0;
const RIDE_COOLDOWN: f32 = 2.0;

/// A pending dismiss is dropped if the server has not detached the player after this long: a safety bound, not a
/// 1.2 value, so a lost `CSUnMountMate` cannot turn a later manual unmount into an unasked `CSRemoveMate`.
const DISMISS_DEADLINE: std::time::Duration = std::time::Duration::from_secs(5);

fn ride_rested(data: &HudData) -> bool {
    data.pets.owned.last_ride.is_none_or(|t| t.elapsed().as_secs_f32() >= RIDE_COOLDOWN)
}

fn in_mount_reach(data: &HudData) -> bool {
    data.pets.owned.pet_pos.is_some_and(|(p, _)| p.distance_squared(data.pos) <= MOUNT_REACH * MOUNT_REACH)
}

/// `X2Mate:CanMount()` for the own pet: alive, a driver's seat, the player not riding, in reach, not just asked. The
/// native buffs `0x2b05`/`0x2b15` and `[mate+0x3c26]` refusals are not ported.
fn can_mount(data: &HudData) -> bool {
    let Some(pet) = &data.pets.owned.current else { return false };
    let seat = data.game.get().and_then(|g| g.pets.owned.seats.get(&pet.model)).is_some_and(|s| s.0);
    !pet.dead && seat && !data.pets.owned.mounted && in_mount_reach(data) && ride_rested(data)
}

/// `FUN_393642f0` shows the detail word `0x205` (`Target is too far.`) when the mate is beyond `FUN_3935aa10`'s 25 m.
fn mount_refusal(data: &HudData) -> Option<&'static str> {
    let pet = data.pets.owned.current.as_ref()?;
    let seat = data.game.get().and_then(|g| g.pets.owned.seats.get(&pet.model)).is_some_and(|s| s.0);
    (!pet.dead && seat && !data.pets.owned.mounted && !in_mount_reach(data)).then_some("Target is too far.")
}

/// Queues mate requests for the current pet (sent by [`commands`] while it is still the same pet).
fn request(data: &mut HudData, commands: impl IntoIterator<Item = MateCommand>) {
    let Some(identity) = data.pets.owned.current.as_ref().map(Pet::identity) else { return };
    let owner = data.character_id;
    data.pets.owned.requests.extend(commands.into_iter().map(|c| (owner, identity, c)));
    data.pets.owned.last_ride = Some(web_time::Instant::now());
}

/// `X2Mate:MountMate()`: the driver's seat of the own pet. The native client picks the reason (1..4) from how the player
/// comes up to the mount (`3909ae90`), for its mounting animation.
fn mount(data: &mut HudData) -> Option<&'static str> {
    if !can_mount(data) {
        return mount_refusal(data);
    }
    let (Some(pet), Some((mate, yaw))) = (&data.pets.owned.current, data.pets.owned.pet_pos) else { return None };
    let half = yaw * 0.5;
    let reason = super::mount_reason::mount_reason(
        [data.pos.x, data.pos.y, data.pos.z],
        [mate.x, mate.y, mate.z],
        [0.0, 0.0, half.sin(), half.cos()],
    );
    request(data, [MateCommand::Mount { tl_id: pet.spawn.tl_id, point: 1, reason }]);
    None
}

/// Getting off the own pet: `X2Mate:UnMountMate()` (reason 9) from the seat the player is on; `DismissPet()` first gets off
/// with reason 10 and then sends the pet away (the native client sends the removal when that dismount finishes).
fn unmount(data: &mut HudData, dismiss: bool) {
    let Some(pet) = &data.pets.owned.current else { return };
    if !ride_rested(data) {
        return;
    }
    if !dismiss {
        data.pets.owned.pending_dismiss = None;
    }
    let identity = pet.identity();
    let tl_id = pet.spawn.tl_id;
    let attached = pet.attached_to_own;
    let mut out = Vec::new();
    if attached {
        let point = if pet.driving { 1 } else { 2 };
        out.push(MateCommand::Unmount { tl_id, point, reason: if dismiss { 10 } else { 9 } });
    }
    if dismiss {
        let now = web_time::Instant::now();
        let detached = (!attached).then_some(now);
        data.pets.owned.pending_dismiss = Some(PendingDismiss { identity, detached, deadline: now + DISMISS_DEADLINE });
    }
    if !out.is_empty() {
        request(data, out);
    } else if dismiss {
        data.pets.owned.last_ride = Some(web_time::Instant::now());
    }
}

/// Queues the native `CSRemoveMate` once the detach was seen and the 1000 ms unmount controller has run.
fn pending_dismiss(data: &mut HudData, now: web_time::Instant) -> Option<MateCommand> {
    let mut pending = data.pets.owned.pending_dismiss.take()?;
    let pet = data.pets.owned.current.as_ref().filter(|p| p.identity() == pending.identity)?;
    if !pet.attached_to_own {
        pending.detached.get_or_insert(now);
    }
    let ready = pending.detached.is_some_and(|t| now.duration_since(t) >= std::time::Duration::from_millis(1000));
    if now < pending.deadline && ready {
        return Some(MateCommand::Dismiss { tl_id: pet.spawn.tl_id });
    }
    data.pets.owned.pending_dismiss = (now < pending.deadline).then_some(pending);
    None
}

fn table(lua: &Lua, pet: &Pet) -> mlua::Result<Table> {
    let out = lua.create_table()?;
    out.set("unit_id", pet.object.to_string())?;
    out.set("name", pet.name.as_str())?;
    out.set("owner_name", pet.owner_name.as_str())?;
    out.set("level", pet.level)?;
    out.set("hp", pet.hp)?;
    out.set("max_hp", pet.max_hp)?;
    out.set("mp", pet.mp)?;
    out.set("max_mp", pet.max_mp)?;
    out.set("dead", pet.dead)?;
    out.set("in_combat", pet.combat)?;
    out.set("unit_type", "npc")?;
    out.set("detail", "mate")?;
    out.set("relation", "friendly")?;
    out.set("buffs", super::super::panels::buffs_table(lua, &pet.buffs)?)?;
    Ok(out)
}

pub(super) fn install(lua: &Lua, bridge: &Shared) -> mlua::Result<()> {
    info::install(lua, bridge)?;
    equipment::install(lua, bridge)?;
    let mate: Table = lua.globals().get("X2Mate")?;
    for name in [
        "IsPlayerPetExists",
        "IsPlayerMounting",
        "IsMountablePet",
        "IsHavePassengerSeat",
        "HavePassenger",
        "IsAttackablePet",
    ] {
        let state = bridge.clone();
        mate.set(
            name,
            lua.create_function(move |_, _: Value| {
                let data = state.borrow();
                if name == "IsPlayerMounting" {
                    return Ok(data.pets.owned.mounted);
                }
                let Some(pet) = &data.pets.owned.current else { return Ok(false) };
                let tables = data.game.get().map(|g| &g.pets.owned);
                Ok(match name {
                    "IsPlayerPetExists" => true,
                    "IsPlayerMounting" => pet.mounted,
                    "HavePassenger" => pet.passenger,
                    "IsAttackablePet" => tables.is_some_and(|t| pet.spawn.skills.iter().any(|id| t.skills.get(id).is_some_and(|s| s.first_seat.is_none()))),
                    "IsMountablePet" => tables.and_then(|t| t.seats.get(&pet.model)).is_some_and(|s| s.0),
                    "IsHavePassengerSeat" => tables.and_then(|t| t.seats.get(&pet.model)).is_some_and(|s| s.1),
                    _ => false,
                })
            })?,
        )?;
    }
    let state = bridge.clone();
    mate.set("CanMount", lua.create_function(move |_, _: Value| Ok(can_mount(&state.borrow())))?)?;
    let state = bridge.clone();
    mate.set("CanDismiss", lua.create_function(move |_, _: Value| Ok(state.borrow().pets.owned.current.is_some()))?)?;
    let state = bridge.clone();
    mate.set("MountMate", lua.create_function(move |lua, _: Value| {
        if let Some(text) = mount(&mut state.borrow_mut())
            && let Ok(sysmsg) = lua.globals().get::<mlua::Function>("SYSMSG")
        {
            let _ = sysmsg.call::<()>(text);
        }
        Ok(())
    })?)?;
    for (name, dismiss) in [("UnMountMate", false), ("DismissPet", true)] {
        let state = bridge.clone();
        mate.set(
            name,
            lua.create_function(move |_, _: Value| {
                unmount(&mut state.borrow_mut(), dismiss);
                Ok(())
            })?,
        )?;
    }
    let state = bridge.clone();
    mate.set(
        "PetState",
        lua.create_function(move |_, _: Value| {
            let data = state.borrow();
            Ok(data.pets.owned.current.as_ref().filter(|p| !p.attached_to_own).and_then(|p| {
                let mode = data.pets.owned.local_state.filter(|s| s.0 == p.identity()).map_or(p.spawn.user_state, |s| s.1);
                ["idle", "passive", "protective", "aggressive"].get(mode as usize).copied()
            }))
        })?,
    )?;
    let state = bridge.clone();
    mate.set(
        "GetNumMountSkills",
        lua.create_function(move |_, _: Value| {
            let data = state.borrow();
            Ok(data.pets.owned.current.as_ref().zip(data.game.get()).map_or(0, |(p, g)| {
                p.spawn
                    .skills
                    .iter()
                    .filter(|id| g.pets.owned.skills.get(id).is_some_and(|s| s.first_seat.is_none() || s.first_seat == Some(1)))
                    .count()
            }))
        })?,
    )?;
    let state = bridge.clone();
    mate.set(
        "SetMateName",
        lua.create_function(move |_, (_, name): (Value, String)| {
            let mut data = state.borrow_mut();
            if let Some(pet) = &data.pets.owned.current {
                let identity = pet.identity();
                let command = MateCommand::Rename { tl_id: pet.spawn.tl_id, name };
                if command.validate().is_ok() {
                    let owner = data.character_id;
                    data.pets.owned.requests.push((owner, identity, command));
                }
            }
            Ok(())
        })?,
    )?;
    let state = bridge.clone();
    mate.set(
        "GetMountSkill",
        lua.create_function(move |lua, (_, index): (Value, usize)| {
            let entry = {
                let data = state.borrow();
                data.pets.owned.current.as_ref().zip(data.game.get()).and_then(|(pet, game)| {
                    let (_, skill) = displayed(&game.pets.owned, pet, index)?;
                    Some((
                        skill.combat,
                        skill.level,
                        i32::from(pet.level) >= skill.level,
                        skill.first_seat.is_some(),
                        usable(skill, pet),
                    ))
                })
            };
            let Some((combat, level, learned, driver, usable)) = entry else {
                return Ok(None);
            };
            let hud: Table = lua.globals().get("HUD")?;
            let get: mlua::Function = hud.get("skill_tip")?;
            let Some(info) = get.call::<Option<Table>>(combat)? else { return Ok(None) };
            info.set("learned", learned)?;
            info.set("skillUsable", i32::from(usable))?;
            info.set("needdiver", driver)?;
            info.set("learnLevel", level)?;
            Ok(Some(info))
        })?,
    )?;
    let state = bridge.clone();
    mate.set(
        "GetAutoStartMountSkill",
        lua.create_function(move |_, _: MultiValue| {
            Ok(if state.borrow().pets.owned.current.is_some() {
                MultiValue::from_iter([Value::Boolean(false)])
            } else {
                MultiValue::new()
            })
        })?,
    )?;
    let state = bridge.clone();
    mate.set(
        "PassiveMode",
        lua.create_function(move |_, _: Value| {
            let mut data = state.borrow_mut();
            let Some(pet) = &data.pets.owned.current else { return Ok(false) };
            let identity = pet.identity();
            let tl_id = pet.spawn.tl_id;
            let owner = data.character_id;
            data.pets.owned.local_state = Some((identity, 1));
            data.pets.owned.requests.push((owner, identity, MateCommand::UserState { tl_id, state: 1 }));
            Ok(true)
        })?,
    )?;
    for (name, mode) in [("StandMode", 0), ("ProtectiveMode", 2), ("AggressiveMode", 3)] {
        let state = bridge.clone();
        mate.set(name, lua.create_function(move |_, _: Value| Ok(request_mode(&mut state.borrow_mut(), mode)))?)?;
    }
    let x2: Table = lua.globals().get("X2")?;
    let state = bridge.clone();
    x2.set(
        "GetPetExpToNextLevel",
        lua.create_function(move |_, _: Value| {
            let data = state.borrow();
            let progress = data
                .pets
                .owned
                .current
                .as_ref()
                .zip(data.game.get())
                .and_then(|(p, g)| g.pets.owned.progress(p.spawn.experience));
            Ok(progress.map_or_else(MultiValue::new, |values| MultiValue::from_iter(values.into_iter().map(Value::Number))))
        })?,
    )?;
    // Unit aliases use the same real unit getters as player/target/team in ingame.lua.
    let hud: Table = lua.globals().get("HUD")?;
    let state = bridge.clone();
    hud.set(
        "pet_slot",
        lua.create_function(move |lua, index: usize| {
            let data = state.borrow();
            let Some((pet, game)) = data.pets.owned.current.as_ref().zip(data.game.get()) else {
                return Ok(None);
            };
            let Some((_, skill)) = displayed(&game.pets.owned, pet, index) else {
                return Ok(None);
            };
            let Some(info) = game.skills.get(&skill.combat) else { return Ok(None) };
            let out = lua.create_table()?;
            out.set("id", skill.combat)?;
            out.set("icon", info.named.icon.as_str())?;
            out.set("learned", i32::from(pet.level) >= skill.level)?;
            out.set("canUse", usable(skill, pet))?;
            out.set("type", "pet_skill")?;
            Ok(Some(out))
        })?,
    )?;
    hud.set("pet_target", {
        let state = bridge.clone();
        lua.create_function(move |_, ()| {
            let mut data = state.borrow_mut();
            if let Some(p) = &data.pets.owned.current {
                let obj_id = p.object;
                data.requests.push(super::super::api::Request::Target { obj_id });
            }
            Ok(())
        })?
    })?;
    lua.load(
        r#"
        local content=AAUI.SlotContent
        AAUI.SlotContent=function(kind,index)
          if kind==ISLOT_PET_ACTION then
            local slot=HUD.pet_slot(index)
            if slot then slot.tooltip=X2Mate:GetMountSkill(index) end
            return slot
          end
          return content(kind,index)
        end
        local target=X2Unit.TargetUnit
        X2Unit.TargetUnit=function(self,key)
          if HUD.playerpet and (key=='playerpet' or HUD.private_unit_id(key)==HUD.playerpet.unit_id) then HUD.pet_target()
          elseif target then return target(self,key) end
        end
        local info=X2Unit.GetUnitInfoById
        X2Unit.GetUnitInfoById=function(self,id)
          if HUD.playerpet and id==HUD.playerpet.unit_id then
            return {owner_name=HUD.playerpet.owner_name}
          end
          if info then return info(self,id) end
        end
    "#,
    )
    .set_name("@owned_pet.lua")
    .exec()?;
    let pet = bridge.borrow().pets.owned.current.clone();
    hud.set("playerpet", pet.as_ref().map(|pet| table(lua, pet)).transpose()?)?;
    bridge.borrow_mut().pets.owned.pushed = pet;
    Ok(())
}

pub(super) fn push(rt: &UiRuntime, bridge: &Shared) {
    let (old, next) = {
        let mut data = bridge.borrow_mut();
        if data.pets.owned.current == data.pets.owned.pushed {
            return;
        }
        let next = data.pets.owned.current.clone();
        (std::mem::replace(&mut data.pets.owned.pushed, next.clone()), next)
    };
    let result = (|| -> mlua::Result<()> {
        let hud: Table = rt.lua.globals().get("HUD")?;
        hud.set("playerpet", next.as_ref().map(|pet| table(&rt.lua, pet)).transpose()?)
    })();
    if let Err(error) = result {
        warn!("hud: owned pet: {error}");
        return;
    }
    let identity_changed = old.as_ref().map(Pet::identity) != next.as_ref().map(Pet::identity);
    info::push(rt, bridge);
    if identity_changed {
        if old.is_some() {
            rt.fire_event("DISMISS_PET", ());
        }
        if next.is_some() {
            rt.fire_event("SPAWN_PET", ());
        }
    } else if let (Some(old), Some(next)) = (&old, &next) {
        if old.driving != next.driving {
            rt.fire_event(if next.driving { "MOUNT_PET" } else { "UNMOUNT_PET" }, true);
        }
        if old.passenger != next.passenger {
            rt.fire_event(if next.passenger { "PASSENGER_MOUNT_PET" } else { "PASSENGER_UNMOUNT_PET" }, ());
        }
        if old.name != next.name {
            rt.fire_event("UNIT_NAME_CHANGED", super::super::unit_id::format(next.object));
        }
        if old.level != next.level {
            rt.fire_event("LEVEL_CHANGED", (next.level, super::super::unit_id::format(next.object)));
        }
    }
    if identity_changed { info::push(rt, bridge); }
    equipment::push(rt, old.as_ref(), next.as_ref(), identity_changed);
    if let Err(error) = rt.refresh_slots() {
        warn!("hud: pet slots: {error}");
    }
}

#[cfg(test)]
mod tests;
mod info;

fn displayed<'a>(tables: &'a Tables, pet: &Pet, index: usize) -> Option<(usize, &'a Skill)> {
    let context = u32::from(pet.driving);
    pet.spawn
        .skills
        .iter()
        .enumerate()
        .filter_map(|(raw, id)| {
            let skill = tables.skills.get(id)?;
            (skill.first_seat.is_none() || skill.first_seat == Some(context) || (context == 0 && skill.first_seat == Some(1))).then_some((raw, skill))
        })
        .nth(index.checked_sub(1)?)
}

/// `FUN_3933c340`'s display state: a shown row is usable only while riding (live capture 2026-10-28: on foot every
/// slot reports `skillUsable` 0 and mounted 1; the server's cooldowns, not modeled here, also clear it). A
/// `skills.target_type_id` 6 row skips the decompile's general validator (`3933b7a7`) and stays ready in either
/// state; no captured row had that type.
fn usable(skill: &Skill, pet: &Pet) -> bool {
    skill.target_type == 6 || pet.driving
}

/// `FUN_3933e630`: a pet action slot the current mate shows, learned at its level and attached for the riding state
/// becomes a `CSStartSkill` with the mate as caster (`FUN_39399630` -> `FUN_393989e0`). Live capture 2026-10-28: while
/// riding, a targeted row sends the current target and, with none, still sends target kind 0 / object 0 (skill 11016,
/// mount_skill 1: `08 2b 00 00 03 81 55 00 01 00 00 00 00 00 00 00 00`). On foot every slot sends nothing and shows the
/// 0x2c "Can't be used right now." refusal; 1.2 has no on-foot `pet_manual` pending mode. The decompile's
/// self-targeted rows (`skills.target_type_id` 6) aim at the mate; the capture's five rows are all target_type_id 0.
pub(super) fn use_skill(data: &mut HudData, slot: usize, target_obj_id: u32) -> Result<(), u8> {
    // A pet_action_bar_button hotkey can fire with no current mate, or before the game tables
    // load; there is no native slot to press then, so stay silent instead of a 0x2c refusal.
    let Some((pet, game)) = data.pets.owned.current.as_ref().zip(data.game.get()) else {
        return Ok(());
    };
    let Some((raw, skill)) = displayed(&game.pets.owned, pet, slot) else {
        return Err(super::super::usable::code::NOT_NOW);
    };
    if i32::from(pet.level) < skill.level {
        return Err(super::super::usable::code::NOT_NOW);
    }
    // The attach row's first entry must equal the riding flag; no row means either state. 1.2
    // computes the flag as player+0x3af4 == 1 && player+0x3af0 == this mate (FUN_3933b670 /
    // FUN_3933ad90); whether a passenger seat (point 2) also counts is unverified (live capture
    // 2026-10-28 had one client), so this stays driver-only.
    let riding = u32::from(pet.driving);
    if skill.first_seat.is_some_and(|seat| seat != riding) {
        return Err(super::super::usable::code::NOT_NOW);
    }
    if !pet.driving {
        return Err(super::super::usable::code::NOT_NOW);
    }
    // A target_type_id 6 row names the mate (decompile only; no captured row had that type). Any
    // other row uses the current target, and with none 1.2 still sends kind 0 / object 0 (live
    // capture 2026-10-28; its self rows were cast with no target selected, so what a type-0 self
    // row sends with a target selected is unverified).
    let target_obj_id = if skill.target_type == 6 {
        pet.object
    } else {
        target_obj_id
    };
    let identity = pet.identity();
    let owner = data.character_id;
    data.pets.owned.casts.push((
        owner,
        identity,
        PendingCast {
            skill_id: skill.combat,
            mount_skill_id: pet.spawn.skills[raw],
            target_obj_id,
        },
    ));
    Ok(())
}

fn request_mode(data: &mut HudData, mode: u8) -> bool {
    let Some(pet) = &data.pets.owned.current else { return false };
    if mode >= 2
        && !data.game.get().is_some_and(|g| {
            pet.spawn
                .skills
                .iter()
                .any(|id| g.pets.owned.skills.get(id).is_some_and(|s| s.first_seat.is_none()))
        })
    {
        return false;
    }
    let identity = pet.identity();
    let old = data.pets.owned.local_state.filter(|s| s.0 == identity).map_or(pet.spawn.user_state, |s| s.1);
    if mode != 1 && old == mode && data.pets.owned.commanded_skill == 0 {
        return false;
    }
    let command = MateCommand::UserState {
        tl_id: pet.spawn.tl_id,
        state: mode,
    };
    let owner = data.character_id;
    data.pets.owned.local_state = Some((identity, mode));
    data.pets.owned.commanded_skill = 0;
    data.pets.owned.requests.push((owner, identity, command));
    true
}

pub(super) fn event(data: &mut HudData, event: &Event) -> Option<(bool, u32)> {
    if let Event::CombatChanged { obj_id, engaged } = event {
        let pet = data.pets.owned.current.as_mut().filter(|p| p.object == *obj_id)?;
        pet.combat = *engaged;
        return Some((*engaged, *obj_id));
    }
    None
}

pub(super) fn commands(
    data: &mut HudData,
    player: &Player,
    units: &Units,
    mates: &Mates,
    net: &mut MessageWriter<NetCommand>,
    mount_reason: &mut crate::ingame::MountReason,
) {
    if let Some(command) = pending_dismiss(data, web_time::Instant::now()) {
        net.write(NetCommand(Command::Mate(command)));
    }
    let owner = player.spawn.as_ref().map_or(0, |p| p.character().id);
    let actual = current(mates, units, owner).map(|(unit, spawn)| (unit.obj_id, spawn.tl_id, spawn.id));
    for (quoted_owner, identity, command) in std::mem::take(&mut data.pets.owned.requests) {
        if owner != 0 && owner == quoted_owner && data.character_id == owner && actual == Some(identity) {
            // the original computes the get-on reason when it sends CSMountMate (`FUN_3909ae90`); the get-on state of
            // player_full_body.xml takes it once SCUnitAttached seats the player
            if let MateCommand::Mount { reason, .. } = &command {
                mount_reason.0 = *reason;
            }
            net.write(NetCommand(Command::Mate(command)));
        }
    }
    for (quoted_owner, identity, cast) in std::mem::take(&mut data.pets.owned.casts) {
        if owner != 0
            && owner == quoted_owner
            && data.character_id == owner
            && actual == Some(identity)
        {
            net.write(NetCommand(Command::UseMateSkill {
                skill_id: cast.skill_id,
                mate_obj_id: identity.0,
                mount_skill_id: cast.mount_skill_id,
                target_obj_id: cast.target_obj_id,
            }));
        }
    }
}

mod equipment;

pub(super) fn equipment_slots(data: &HudData, event: &Event) -> Vec<i64> {
    equipment::slots(data, event)
}
