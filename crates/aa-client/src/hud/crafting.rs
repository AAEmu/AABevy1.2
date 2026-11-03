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

/// A catalog with only workbench recipe maps, for tests that do not need the pak or database.
#[cfg(test)]
pub(super) fn fixture_tables(benches: impl IntoIterator<Item = (u32, Vec<u32>)>) -> Tables {
    let mut tables = Tables::default();
    for (template, recipes) in benches {
        tables.benches.insert(template, recipes.into_iter().collect());
    }
    tables
}

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

/// Opens the bench at `object` (template `template`), the local entry of native `FUN_394dc360`.
/// Returns the unique recipe count for `CRAFTING_START`, or `None` when the object is invalid or
/// the catalog is not loaded.
pub(super) fn open_bench(data: &mut HudData, object: u32, template: u32) -> Option<usize> {
    bench::open(data, object, template)
}

/// The live workbench identity `(object, template)`, or `None` after a close.
#[cfg(test)]
pub(super) fn active_bench(data: &HudData) -> Option<(u32, u32)> {
    data.crafting.current.active.map(|b| (b.object, b.template))
}

/// The CURRENT recipe count (native `DAT_3a5ad86c`).
#[cfg(test)]
pub(super) fn current_recipe_count(data: &HudData) -> usize {
    data.crafting.current.recipes.len()
}

/// The profession icon (`actability_groups.icon_path`) of the first recipe a bench template offers:
/// the icon the shared action bar shows for the workbench action. This is a presentation choice
/// (the original action's own icon id is the unresolved protected action key); the window itself
/// uses the same actability group icon.
pub(super) fn bench_icon(data: &HudData, template: u32) -> Option<String> {
    let tables = &data.game.get()?.crafting;
    let recipe = tables.benches.get(&template)?.iter().next()?;
    let group = tables.recipes.get(recipe)?.actability_group;
    let icon = tables.groups.get(&group)?.icon.clone();
    (!icon.is_empty()).then_some(icon)
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

/// The localized `actability_groups` name of a proficiency group (the house actability getters read it).
pub(super) fn actability_group_name(data: &HudData, group: u32) -> Option<&str> {
    data.game.get()?.crafting.groups.get(&group).map(|g| g.name.as_str())
}

/// Apply a received actability event to the HUD's copy (the housing self-check and its test drive
/// the `CHANGE_ACTABILITY_DECO_NUM` change detector with it).
pub(super) fn apply_actability(data: &mut HudData, event: &aa_net::Event) -> bool {
    data.crafting.actabilities.apply(event)
}

/// The received actability points, as `CHANGE_ACTABILITY_DECO_NUM` compares them (order independent).
pub(super) fn actability_signature(data: &HudData) -> Vec<(u32, Option<i32>)> {
    let mut entries: Vec<_> = data.crafting.actabilities.entries.iter().map(|a| (a.id, a.point)).collect();
    entries.sort_unstable_by_key(|e| e.0);
    entries
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
        lua.create_function(move |lua, (_self, group): (mlua::Value, u32)| {
            expert::observe_construction(&mut state.borrow_mut());
            my_info(lua, &state.borrow(), group)
        })?,
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
