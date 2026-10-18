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
}

impl Pet {
    fn identity(&self) -> (u32, u16, u32) {
        (self.object, self.spawn.tl_id, self.spawn.id)
    }
}

#[derive(Default)]
pub(super) struct View {
    current: Option<Pet>,
    pushed: Option<Pet>,
    local_state: Option<((u32, u16, u32), u8)>,
    // Native +58 starts at0; original local action/AI planner is not wired yet.
    commanded_skill: u32,
    mounted: bool,
    requests: Vec<(u32, (u32, u16, u32), MateCommand)>,
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
        let (max_hp, max_mp) = game
            .and_then(|g| super::super::stats::unit_max(g, unit, buffs))
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
        })
    })();
    if data.pets.owned.current.as_ref().map(Pet::identity) != next.as_ref().map(Pet::identity) {
        data.pets.owned.local_state = None;
        data.pets.owned.commanded_skill = 0;
    }
    data.pets.owned.current = next;
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
                        skill.target_type == 6,
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
            out.set("canUse", skill.target_type == 6)?;
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
          if HUD.playerpet and (key=='playerpet' or key==HUD.playerpet.unit_id) then HUD.pet_target()
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
    if old.as_ref().map(Pet::identity) != next.as_ref().map(Pet::identity) {
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
            rt.fire_event("UNIT_NAME_CHANGED", next.object.to_string());
        }
        if old.level != next.level {
            rt.fire_event("LEVEL_CHANGED", (next.level, next.object.to_string()));
        }
    }
    if let Err(error) = rt.refresh_slots() {
        warn!("hud: pet slots: {error}");
    }
}

#[cfg(test)]
mod tests;

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

pub(super) fn commands(data: &mut HudData, player: &Player, units: &Units, mates: &Mates, net: &mut MessageWriter<NetCommand>) {
    let owner = player.spawn.as_ref().map_or(0, |p| p.character().id);
    let actual = current(mates, units, owner).map(|(unit, spawn)| (unit.obj_id, spawn.tl_id, spawn.id));
    for (quoted_owner, identity, command) in std::mem::take(&mut data.pets.owned.requests) {
        if owner != 0 && owner == quoted_owner && data.character_id == owner && actual == Some(identity) {
            net.write(NetCommand(Command::Mate(command)));
        }
    }
}
