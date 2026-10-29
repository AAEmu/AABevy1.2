//! The original current-pet HUD and stablehand service over received units/item details.
#[allow(dead_code)]
pub(super) mod active_fixture;
#[allow(dead_code)]
pub(super) mod fixture;
pub(super) mod mount_reason;
pub(super) mod owned;
#[cfg(test)]
mod tests;
use super::api::{HudBridge, HudData, Shared};
use super::data::{any_flag, any_int};
use aa_net::mate::MateCommand;
use aa_net::model::ItemDetail;
use aa_net::state::{Inventory, Player, Units};
use aa_net::{Command, Event, NetCommand, NetEvent};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use mlua::{Lua, Table, Value};
use rusqlite::Connection;
use std::collections::HashSet;

#[derive(Default)]
pub(super) struct Tables {
    owned: owned::Tables,
    summons: HashSet<u32>,
    pub(super) stables: HashSet<u32>,
}

impl Tables {
    pub(super) fn load(conn: &Connection) -> rusqlite::Result<Self> {
        let mut out = Self {
            owned: owned::Tables::load(conn)?,
            ..default()
        };
        let mut query = conn.prepare("SELECT item_id,npc_id FROM item_summon_mates ORDER BY id")?;
        let mut rows = query.query([])?;
        while let Some(row) = rows.next()? {
            if any_int(row, 1)? != 0 {
                out.summons.insert(any_int(row, 0)? as u32);
            }
        }
        let mut query = conn.prepare("SELECT id,stabler FROM npcs")?;
        let mut rows = query.query([])?;
        while let Some(row) = rows.next()? {
            if any_flag(row, 1)? {
                out.stables.insert(any_int(row, 0)? as u32);
            }
        }
        Ok(out)
    }
}

#[derive(Default)]
pub(super) struct View {
    owned: owned::View,
    owner: u32,
    inventory: Option<Inventory>,
    service: Option<(u32, u32)>,
    scope: Option<super::dialog::NpcScope>,
    requests: Vec<(u32, (u32, u32), super::dialog::NpcScope)>,
}

/// A click or hotkey on pet action slot `slot` (1 based): the 1.2 slot gate and, while riding, the mate-caster cast.
pub(super) fn use_skill(data: &mut HudData, slot: usize, target_obj_id: u32) -> Result<(), u8> {
    owned::use_skill(data, slot, target_obj_id)
}

pub(super) fn refresh(world: &World, data: &mut HudData) {
    if data.pets.owner != data.character_id {
        data.pets = View {
            owner: data.character_id,
            ..default()
        };
    }
    if let Some(inventory) = world.get_resource_ref::<Inventory>() {
        if data.pets.inventory.is_none() || inventory.is_changed() {
            data.pets.inventory = Some((*inventory).clone());
        }
    }
    owned::refresh(world, data);
}

/// Native x87 pow constant, followed by separately rounded single-precision additions.
fn revival_cost(level: u8) -> i32 {
    let power = f64::from(level).powf(2.299999952316284) as f32;
    let half = f32::from(level) * 0.5f32;
    ((power + half) + 0.5f32).floor() as i32
}

fn quote(data: &HudData) -> f64 {
    let (Some(game), Some(inventory)) = (data.game.get(), &data.pets.inventory) else {
        return 0.0;
    };
    let mut sum = 0i32;
    for (container, items, capacity) in [
        (2, &inventory.bag, inventory.bag_slots),
        (3, &inventory.bank, inventory.bank_slots),
    ] {
        for (slot, item) in items.iter().take(capacity as usize).enumerate() {
            let Some(item) = item else { continue };
            let Ok(slot) = u8::try_from(slot) else {
                continue;
            };
            if inventory.item_slot_pending(container, slot)
                || !game.pets.summons.contains(&item.template_id)
                || !game
                    .items
                    .get(&item.template_id)
                    .is_some_and(|item| item.impl_id == 11)
            {
                continue;
            }
            if let ItemDetail::Mate { dead, level, .. } = item.detail {
                if dead != 0 {
                    sum = sum.wrapping_add(revival_cost(level));
                }
            }
        }
    }
    f64::from(sum as f32)
}

pub(super) fn install(lua: &Lua, bridge: &Shared) -> mlua::Result<()> {
    owned::install(lua, bridge)?;
    let x2: Table = lua.globals().get("X2")?;
    let state = bridge.clone();
    x2.set(
        "GetTotalRepairsForPetItems",
        lua.create_function(move |_, _: Value| Ok(quote(&state.borrow())))?,
    )?;
    let state = bridge.clone();
    x2.set(
        "RepairPetItems",
        lua.create_function(move |_, _: Value| {
            let mut data = state.borrow_mut();
            if let Some(npc) = data
                .pets
                .service
                .filter(|npc| data.dialog.npc == Some(*npc))
            {
                let owner = data.character_id;
                if let Some(scope) = data
                    .pets
                    .scope
                    .filter(|scope| super::dialog::scope_current(&data, *scope))
                {
                    data.pets.requests.push((owner, npc, scope));
                }
            }
            Ok(())
        })?,
    )?;
    Ok(())
}

pub(super) fn push(rt: &UiRuntime, bridge: &Shared) {
    owned::push(rt, bridge);
}

fn valid_stable(data: &HudData, units: &Units, npc: (u32, u32)) -> bool {
    units
        .get(npc.0)
        .is_some_and(|unit| matches!(unit.kind, aa_net::model::UnitKind::Npc { template_id, .. } if template_id == npc.1))
        && data.game.get().is_some_and(|game| game.pets.stables.contains(&npc.1))
}

pub(super) fn start_selected(
    rt: &UiRuntime,
    bridge: &Shared,
    selected: super::dialog::SelectedNpc,
    player: &Player,
    units: &Units,
) -> bool {
    let mut data = bridge.borrow_mut();
    let Some(spawn) = &player.spawn else {
        return false;
    };
    let npc = selected.npc();
    if data.character_id != spawn.character().id
        || !super::dialog::scope_current(&data, selected.scope())
        || !valid_stable(&data, units, npc)
    {
        return false;
    }
    data.pets.service = Some(npc);
    data.pets.scope = Some(selected.scope());
    drop(data);
    rt.fire_event("NPC_INTERACTION_START", ("stabler", "", npc.0.to_string()));
    true
}

pub(super) fn update(
    mut events: MessageReader<NetEvent>,
    rt: Option<NonSend<UiRuntime>>,
    bridge: NonSend<HudBridge>,
    player: Res<Player>,
    units: Res<Units>,
    mates: Option<Res<aa_net::Mates>>,
    mut net: MessageWriter<NetCommand>,
) {
    let Some(rt) = rt else {
        events.clear();
        return;
    };
    let owner = player.spawn.as_ref().map_or(0, |s| s.character().id);
    let mut errors = Vec::new();
    let mut combat = Vec::new();
    let mut equipment = Vec::new();
    {
        let mut data = bridge.0.borrow_mut();
        if owner == 0 {
            data.pets = default();
            events.clear();
            return;
        }
        for NetEvent(event) in events.read() {
            equipment.extend(owned::equipment_slots(&data, event));
            if let Some(change) = owned::event(&mut data, event) {
                combat.push(change);
            }
            if let Event::Mate(aa_net::mate::MateEvent::RequestRejected {
                command: MateCommand::ReviveItems { .. },
                message,
            }) = event
            {
                errors.push(message.clone());
            }
        }
        if data.pets.service.is_some_and(|npc| {
            data.dialog.npc != Some(npc)
                || !valid_stable(&data, &units, npc)
                || !data
                    .pets
                    .scope
                    .is_some_and(|scope| super::dialog::scope_current(&data, scope))
        }) {
            data.pets.service = None;
        }
        for (quoted_owner, npc, scope) in std::mem::take(&mut data.pets.requests) {
            if super::dialog::scope_current(&data, scope)
                && owner == quoted_owner
                && data.character_id == owner
                && data.pets.service == Some(npc)
                && data.dialog.npc == Some(npc)
                && valid_stable(&data, &units, npc)
            {
                net.write(NetCommand(Command::Mate(MateCommand::ReviveItems {
                    npc_obj_id: npc.0,
                })));
            }
        }
        if let Some(mates) = mates {
            owned::commands(&mut data, &player, &units, &mates, &mut net);
        }
    }
    owned::push(&rt, &bridge.0);
    for slot in equipment {
        rt.fire_event("UNIT_NPC_EQUIPMENT_CHANGED", slot);
    }
    for message in errors {
        rt.fire_event("SYSMSG", message);
    }
    for (engaged, object) in combat {
        rt.fire_event("UNIT_COMBAT_STATE_CHANGED", (engaged, super::unit_id::format(object)));
    }
}

pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        update
            .after(super::sync_hud)
            .after(super::dialog::drive_npc)
            .before(aa_ui::UiFrame)
            .run_if(super::hud_up),
    )
    .add_systems(
        OnExit(crate::Screen::InGame),
        |bridge: NonSend<HudBridge>| bridge.0.borrow_mut().pets = default(),
    );
}
