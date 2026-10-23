//! Original recipe-book getters over the read-only catalog and received physical bag stacks.
//! Bench interaction and batch presentation are separate from browsing these definitions.
mod batch;
mod bench;
mod catalog;
mod data;
mod expert;
#[allow(dead_code)]
pub(super) mod fixture;
#[cfg(test)]
mod tests;

use super::api::{HudData, Shared};
use aa_net::state::Inventory;
use bevy::prelude::*;
pub(super) use data::Tables;
use mlua::{Lua, Table};

#[derive(Default)]
pub(super) struct View {
    owner: u32,
    inventory: Option<Inventory>,
    actabilities: aa_net::Actabilities,
    item_secure: Option<bool>,
    current: bench::Current,
    batch: batch::Batch,
    casts: batch::CastSlots,
    requests: Vec<batch::Request>,
    expert: expert::View,
}

pub(super) fn refresh(world: &World, data: &mut HudData) {
    if data.crafting.owner != data.character_id {
        data.crafting = View {
            owner: data.character_id,
            ..default()
        };
    }
    if let Some(inventory) = world.get_resource_ref::<Inventory>() {
        if data.crafting.inventory.is_none() || inventory.is_changed() {
            data.crafting.inventory = Some((*inventory).clone());
        }
    }
    if let Some(received) = world.get_resource::<aa_net::Actabilities>() {
        if !data.crafting.expert.active && data.crafting.actabilities != *received {
            data.crafting.actabilities.clone_from(received);
        }
    }
    data.crafting.item_secure = world
        .get_resource::<aa_net::GameConfig>()
        .and_then(|c| c.received.as_ref())
        .and_then(|c| c.feature_bytes.get(5))
        .map(|b| b & 0x20 != 0);
}

pub(super) fn proficiency(data: &HudData, group: u32) -> Option<(i32, u8)> {
    let received = &data.crafting.actabilities;
    if let Some(entry) = received.get(group) {
        return entry.point.map(|point| (point, entry.step));
    }
    received.loaded.then_some((0, 0))
}

pub(super) fn music_grades(data: &HudData) -> Vec<(u8, &str)> {
    let Some(game) = data.game.get() else { return Vec::new() };
    game.crafting.grades.iter().enumerate().filter_map(|(grade, id)| {
        Some((u8::try_from(grade).ok()?, game.crafting.experts.get(id)?.name.as_str()))
    }).collect()
}

pub(super) fn music_upper_limit(data: &HudData, grade: usize) -> Option<i32> {
    data.game.get()?.crafting.raw_limits.get(grade).copied()
}

fn modify_point(data: &HudData, group: u32) -> i32 {
    data.game
        .get()
        .and_then(|g| g.crafting.groups.get(&group))
        .and_then(|g| u8::try_from(g.unit_attr).ok())
        .map_or(0, |attr| super::tipcalc::with_bonuses(0.0, attr, &data.tips.attrs.modifiers) as i32)
    // The native enclosing-owned-spatial-object bonus is not yet represented by a client resource.
}

/// Native397a9420: use the previous visible grade below this grade's upper threshold.
fn labor(tables: &Tables, group: u32, base: i32, effective_points: i32, grade: u8) -> i32 {
    if group == 0 {
        return 0;
    }
    let current = tables.grades.get(usize::from(grade)).and_then(|id| tables.experts.get(id));
    let selected = if current.is_some_and(|e| effective_points < e.up_limit) {
        grade
            .checked_sub(1)
            .and_then(|grade| tables.grades.get(usize::from(grade)))
            .and_then(|id| tables.experts.get(id))
    } else {
        current
    };
    let advantage = selected.map_or(0, |e| e.advantage) as f32;
    let discounted = (1.0f32 - advantage * 0.01f32) * base as f32;
    (discounted + 0.5f32).floor().max(1.0) as i32
}

fn my_info(lua: &Lua, data: &HudData, group: u32) -> mlua::Result<Option<Table>> {
    let Some(definition) = data.game.get().and_then(|g| g.crafting.groups.get(&group)) else {
        return Ok(None);
    };
    // The original native getter displays its constructor's zero entry before a snapshot.
    // This local presentation default does not satisfy recipe/action eligibility below.
    let Some((point, grade)) = proficiency(data, group).or_else(|| (!data.crafting.actabilities.loaded).then_some((0, 0))) else {
        return Ok(None);
    };
    let t = lua.create_table()?;
    t.set("type", group)?;
    t.set("name", definition.name.as_str())?;
    t.set("point", point)?;
    t.set("grade", grade)?;
    t.set("modifyPoint", modify_point(data, group))?;
    Ok(Some(t))
}

pub(super) fn install(lua: &Lua, bridge: &Shared) -> mlua::Result<()> {
    catalog::install(lua, bridge)?;
    let ability: Table = lua.globals().get("X2Ability")?;
    let state = bridge.clone();
    ability.set(
        "GetMyActabilityInfo",
        lua.create_function(move |lua, (_self, group): (mlua::Value, u32)| my_info(lua, &state.borrow(), group))?,
    )?;
    batch::install(lua, bridge)?;
    expert::install(lua, bridge)?;
    Ok(())
}

pub(super) fn plugin(app: &mut App) {
    expert::plugin(app);
    app.add_systems(
        Update,
        batch::update
            .after(super::sync_hud)
            .after(super::dialog::handle_dialog)
            .before(aa_ui::UiFrame)
            .run_if(super::hud_up),
    )
    .add_systems(OnExit(crate::Screen::InGame), |bridge: NonSend<super::api::HudBridge>| {
        bridge.0.borrow_mut().crafting = default();
    });
}

pub(super) fn expert_fixture(){expert::fixture::run();}
