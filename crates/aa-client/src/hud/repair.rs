//! Original repair controls over received item details. Quotes never mutate wear or money.
use aa_net::model::{Item, ItemDetail};
use aa_net::repair::{RepairCommand, RepairEvent};
use aa_net::state::{Inventory, Player, PlayerStats};
use aa_net::{Command, Event, NetCommand, NetEvent};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use mlua::{Lua, Table};
use rusqlite::Connection;

use super::api::{HudBridge, HudData, Shared};
use super::data::GameData;
use super::panels::SlotSnap;

#[allow(dead_code)]
pub(super) mod fixture;
#[cfg(test)]
mod tests;
mod wear;

#[derive(Default)]
pub(super) struct Tables {
    factor: f32,
}

impl Tables {
    pub(super) fn load(conn: &Connection) -> rusqlite::Result<Self> {
        conn.query_row("SELECT durability_repair_cost_factor FROM item_configs LIMIT 1", [], |r| {
            Ok(Self {
                factor: r.get::<_, f64>(0)? as f32,
            })
        })
    }
}

/// Native397cea70 MULSS order, including separately rounded grade and factor percentages.
/// aa-net's similarly named helper deliberately retains the lane's different rounding order.
pub(super) fn cost(price: i32, refund: i32, factor: f32, current: u8, maximum: u8) -> Option<i32> {
    if maximum == 0 || price < 0 || refund < 0 || !factor.is_finite() || factor < 0.0 {
        return None;
    }
    let factor = factor * 0.01f32;
    let loss = 1.0f32 - f32::from(current) / f32::from(maximum);
    let loss_price = loss * price as f32;
    let base = factor * loss_price;
    let grade = refund as f32 * 0.01f32;
    let rounded = f64::from(base * grade).ceil() as f32;
    Some(if rounded >= 0.0 && f64::from(rounded) <= f64::from(i32::MAX) {
        rounded as i32
    } else {
        0
    })
}

pub(super) fn item_cost(game: &GameData, template: u32, grade: u8, current: u8, maximum: u8) -> Option<i32> {
    cost(
        i32::try_from(game.items.get(&template)?.price).ok()?,
        game.stores.refund_multiplier(grade)?,
        game.repair.factor,
        current,
        maximum,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Target {
    container: u8,
    slot: u8,
    uid: u64,
    template: u32,
    grade: u8,
    current: u8,
    maximum: u8,
    cost: i32,
}

fn target(game: &GameData, container: u8, slot: usize, uid: u64, template: u32, grade: u8, current: u8) -> Option<Target> {
    let slot = u8::try_from(slot).ok()?;
    let (maximum, repairable) = game.tips.items.durability(template, grade)?;
    if !repairable || current >= maximum {
        return None;
    }
    Some(Target {
        container,
        slot,
        uid,
        template,
        grade,
        current,
        maximum,
        cost: item_cost(game, template, grade, current, maximum)?,
    })
}

fn snapshot_target(game: &GameData, container: u8, slot: usize, item: &SlotSnap) -> Option<Target> {
    target(game, container, slot, item.uid, item.id, super::store::ui_grade(item.grade), item.dur?)
}

fn received_target(game: &GameData, container: u8, slot: usize, item: &Item) -> Option<Target> {
    let ItemDetail::Equip(detail) = &item.detail else { return None };
    target(game, container, slot, item.id, item.template_id, item.grade, detail.durability)
}

fn slots(data: &HudData, container: u8) -> &[Option<SlotSnap>] {
    match container {
        1 => &data.panels.equipment,
        2 => &data.panels.bag,
        _ => &[],
    }
}

fn all_targets(data: &HudData) -> Vec<Target> {
    let Some(game) = data.game.get() else { return Vec::new() };
    slots(data, 1)
        .iter()
        .take(28)
        .enumerate()
        .filter_map(|(slot, item)| snapshot_target(game, 1, slot, item.as_ref()?))
        .collect()
}

#[derive(Clone, Debug)]
struct Quote {
    owner: u32,
    npc: Option<(u32, u32)>,
    scope: Option<super::dialog::NpcScope>,
    targets: Vec<Target>,
    all: bool,
    auto: bool,
}

#[derive(Default)]
pub(super) struct View {
    owner: u32,
    mode: bool,
    cancel: bool,
    requests: Vec<Quote>,
    wear: wear::Wear,
}

pub(super) fn refresh(world: &World, data: &mut HudData) {
    if data.repair.owner != data.character_id {
        data.repair = View {
            owner: data.character_id,
            ..default()
        };
    }
    if !data.repair.wear.active {
        if let (Some(game), Some(inventory)) = (data.game.get(), world.get_resource::<Inventory>()) {
            data.repair.wear.hydrate(game, inventory);
        }
    }
}

fn request(data: &mut HudData, container: u8, slot: Option<usize>, auto: bool) {
    if data.character_id == 0 {
        return;
    }
    let targets = if let Some(slot) = slot {
        data.game
            .get()
            .and_then(|game| snapshot_target(game, container, slot, slots(data, container).get(slot)?.as_ref()?))
            .into_iter()
            .collect()
    } else {
        all_targets(data)
    };
    if !targets.is_empty() {
        data.repair.requests.push(Quote {
            owner: data.character_id,
            npc: data.dialog.npc,
            scope: super::dialog::active_scope(data),
            targets,
            all: slot.is_none(),
            auto,
        });
    }
}

fn command(quote: &Quote, active: u32, npc: Option<(u32, u32)>, game: &GameData, inventory: &Inventory, money: i64) -> Result<RepairCommand, &'static str> {
    if quote.owner == 0 || quote.owner != active || quote.npc != npc {
        return Err("The repair interaction changed. Select the item again.");
    }
    let fresh = if quote.all {
        inventory
            .equipment
            .slots
            .iter()
            .take(28)
            .enumerate()
            .filter_map(|(slot, item)| received_target(game, 1, slot, item.as_ref()?))
            .collect::<Vec<_>>()
    } else {
        quote
            .targets
            .iter()
            .filter_map(|t| received_target(game, t.container, usize::from(t.slot), inventory.item_at(t.container, usize::from(t.slot))?))
            .collect()
    };
    if fresh.is_empty() || fresh != quote.targets {
        return Err("The repair items changed. Select them again.");
    }
    let total = fresh
        .iter()
        .try_fold(0i64, |sum, item| sum.checked_add(i64::from(item.cost)))
        .ok_or("The repair quote is too large.")?;
    // The authored English repair window is gold-only. Premium currency selection/purchase
    // is not invented from the auto-use checkbox; actual balances remain received state.
    if money < total {
        return Err("Not enough money to repair this equipment.");
    }
    Ok(if quote.all {
        RepairCommand::All { auto_use_aa_point: quote.auto }
    } else {
        let t = fresh[0];
        RepairCommand::Single {
            container: t.container,
            slot: t.slot,
            auto_use_aa_point: quote.auto,
        }
    })
}

pub(super) fn install(lua: &Lua, bridge: &Shared) -> mlua::Result<()> {
    let hud: Table = lua.globals().get("HUD")?;
    let state = bridge.clone();
    hud.set(
        "repair_mode",
        lua.create_function(move |lua, value: Option<bool>| {
            let mut data = state.borrow_mut();
            if let Some(value) = value {
                if data.repair.mode != value {
                    data.repair.mode = value;
                    aa_ui::cursor::set_repair_mode(lua, value);
                }
            }
            Ok(data.repair.mode)
        })?,
    )?;
    let state = bridge.clone();
    hud.set(
        "repair_cancel",
        lua.create_function(move |_, ()| {
            state.borrow_mut().repair.cancel = true;
            Ok(())
        })?,
    )?;
    let state = bridge.clone();
    hud.set(
        "repair_all_cost",
        lua.create_function(move |_, ()| {
            // Native adds signed item costs and converts the integer total through f32 to Lua.
            let total = all_targets(&state.borrow()).iter().fold(0i32, |sum, item| sum.wrapping_add(item.cost));
            Ok(f64::from(total as f32))
        })?,
    )?;
    let state = bridge.clone();
    hud.set(
        "repair_all",
        lua.create_function(move |_, auto: bool| {
            request(&mut state.borrow_mut(), 1, None, auto);
            Ok(())
        })?,
    )?;
    let state = bridge.clone();
    hud.set(
        "repair_item",
        lua.create_function(move |_, (container, slot, auto): (u8, i64, bool)| {
            if let Some(slot) = slot.checked_sub(1).and_then(|slot| usize::try_from(slot).ok()) {
                if (container == 1 && slot < 28) || (container == 2 && slot < 256) {
                    request(&mut state.borrow_mut(), container, Some(slot), auto);
                }
            }
            Ok(())
        })?,
    )?;
    let state = bridge.clone();
    hud.set(
        "item_durability",
        lua.create_function(move |lua, (container, slot, check): (u8, i64, bool)| {
            let data = state.borrow();
            let Some(game) = data.game.get() else { return Ok(None) };
            let Some(item) = slot
                .checked_sub(1)
                .and_then(|slot| usize::try_from(slot).ok())
                .and_then(|slot| slots(&data, container).get(slot))
                .and_then(Option::as_ref)
            else {
                return Ok(None);
            };
            let Some((maximum, repairable)) = game.tips.items.durability(item.id, super::store::ui_grade(item.grade)) else {
                return Ok(None);
            };
            let Some(current) = item.dur else { return Ok(None) };
            if check && !repairable {
                return Ok(None);
            }
            let table = lua.create_table()?;
            table.set("current", current)?;
            table.set("max", maximum)?;
            Ok(Some(table))
        })?,
    )?;
    let state = bridge.clone();
    hud.set(
        "damaged_items",
        lua.create_function(move |lua, (limit, check): (f32, bool)| {
            // Required native numeric argument is read as float then CVTTSS2SI. Invalid
            // conversions yield INT_MIN, which truncates to an empty result.
            let limit = if limit.is_finite() && f64::from(limit) >= f64::from(i32::MIN) && f64::from(limit) <= f64::from(i32::MAX) {
                limit as i32
            } else {
                i32::MIN
            };
            let data = state.borrow();
            let mut items = Vec::new();
            if let Some(game) = data.game.get() {
                for (slot, item) in slots(&data, 1).iter().take(28).enumerate() {
                    let Some(item) = item else { continue };
                    let Some((maximum, repairable)) = game.tips.items.durability(item.id, super::store::ui_grade(item.grade)) else {
                        continue;
                    };
                    let Some(current) = item.dur else { continue };
                    let ratio = f32::from(current) / f32::from(maximum);
                    if (!check || repairable) && ratio <= 0.25f32 {
                        items.push((ratio, slot, item));
                    }
                }
            }
            items.sort_by(|a, b| a.0.total_cmp(&b.0));
            let types = lua.create_table()?;
            let grades = lua.create_table()?;
            let slots = lua.create_table()?;
            for (_, slot, item) in items.into_iter().take(limit.max(0) as usize) {
                types.raw_push(item.id)?;
                grades.raw_push(super::store::ui_grade(item.grade))?;
                slots.raw_push(slot + 1)?;
            }
            Ok((types, grades, slots))
        })?,
    )?;
    lua.load(include_str!("repair.lua")).set_name("@repair.lua").exec()
}

fn update(
    mut events: MessageReader<NetEvent>,
    rt: Option<NonSend<UiRuntime>>,
    bridge: NonSend<HudBridge>,
    inventory: Res<Inventory>,
    stats: Res<PlayerStats>,
    player: Res<Player>,
    mut net: MessageWriter<NetCommand>,
) {
    let Some(rt) = rt else {
        events.clear();
        return;
    };
    let first = {
        let mut data = bridge.0.borrow_mut();
        let first = !data.repair.wear.active;
        data.repair.wear.active = true;
        first
    };
    for NetEvent(event) in events.read() {
        if let Event::Repair(RepairEvent::RequestRejected { message, .. }) = event {
            rt.fire_event("SYSMSG", message.as_str());
        }
        if !first && let Event::ItemsChanged(tasks) = event {
            let transition = {
                let mut data = bridge.0.borrow_mut();
                let game = data.game.clone();
                game.get().and_then(|game| data.repair.wear.batch(game, tasks))
            };
            if let Some((added, removed)) = transition {
                rt.fire_event("UPDATE_DURABILITY_STATUS", (added, removed));
            }
        }
    }
    let (requests, game, npc) = {
        let mut data = bridge.0.borrow_mut();
        if data.repair.cancel {
            data.repair.mode = false;
            data.repair.cancel = false;
        }
        if player.spawn.is_none() {
            data.repair = default();
        }
        (std::mem::take(&mut data.repair.requests), data.game.clone(), data.dialog.npc)
    };
    aa_ui::cursor::set_repair_mode(&rt.lua, bridge.0.borrow().repair.mode);
    let Some(game) = game.get() else { return };
    for quote in requests {
        if quote.npc.is_some() && !quote.scope.is_some_and(|scope| super::dialog::scope_current(&bridge.0.borrow(),scope)) { continue; }
        match command(
            &quote,
            player.spawn.as_ref().map_or(0, |s| s.character().id),
            npc,
            game,
            &inventory,
            stats.money,
        ) {
            Ok(command) => {
                net.write(NetCommand(Command::Repair(command)));
            }
            Err(message) => rt.fire_event("SYSMSG", message),
        }
    }
}

pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        update
            .after(super::sync_hud)
            .after(super::dialog::drive_npc)
            .after(super::expansion::update)
            .before(aa_ui::UiFrame)
            .run_if(super::hud_up),
    )
    .add_systems(OnExit(crate::Screen::InGame), |bridge: NonSend<HudBridge>, rt: Option<NonSend<UiRuntime>>| {
        bridge.0.borrow_mut().repair = default();
        if let Some(rt) = rt {
            aa_ui::cursor::set_repair_mode(&rt.lua, false);
        }
    });
}

pub(super) fn escape(rt: &UiRuntime) -> bool {
    rt.lua
        .load("if X2Item:IsInRepairMode() then X2Item:LeaveRepairMode(); return true end; return false")
        .eval()
        .unwrap_or(false)
}
