//! Static workbench recipe maps and the separate live interaction identity.
use super::super::api::HudData;
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Bench {
    pub object: u32,
    pub template: u32,
}

#[derive(Default)]
pub(super) struct Current {
    pub active: Option<Bench>,
    pub recipes: BTreeSet<u32>,
}

/// Called by a resolved crafting interaction, never by arbitrary recipe selection.
pub(super) fn open(data: &mut HudData, object: u32, template: u32) -> Option<usize> {
    if object == 0 || object > 0xff_ffff {
        return None;
    }
    let game = data.game.get()?;
    let recipes = game.crafting.benches.get(&template).cloned().unwrap_or_default();
    let count = recipes.len();
    data.crafting.current = Current {
        active: Some(Bench { object, template }),
        recipes,
    };
    Some(count)
}

/// Native close preserves CURRENT maps until the next workbench opens.
pub(super) fn close(data: &mut HudData) -> bool {
    data.crafting.current.active.take().is_some()
}

pub(super) fn groups(data: &HudData) -> Vec<u32> {
    let Some(game) = data.game.get() else { return Vec::new() };
    data.crafting
        .current
        .recipes
        .iter()
        .filter_map(|id| {
            game.crafting
                .recipes
                .get(id)
                .and_then(|r| game.crafting.categories.get(&r.category))
                .map(|c| c.group)
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(super) fn categories(data: &HudData, group: u32) -> Vec<u32> {
    let Some(game) = data.game.get() else { return Vec::new() };
    let tables = &game.crafting;
    let mut ids = data
        .crafting
        .current
        .recipes
        .iter()
        .filter_map(|id| {
            let recipe = tables.recipes.get(id)?;
            tables.categories.get(&recipe.category).filter(|c| c.group == group).map(|_| recipe.category)
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    ids.sort_by_key(|id| (tables.categories[id].order, *id));
    ids
}

pub(super) fn recipes(data: &HudData, group: u32, category: u32) -> Vec<u32> {
    let Some(game) = data.game.get() else { return Vec::new() };
    let tables = &game.crafting;
    if !tables.categories.get(&category).is_some_and(|c| c.group == group) {
        return Vec::new();
    }
    let mut ids = data
        .crafting
        .current
        .recipes
        .iter()
        .copied()
        .filter(|id| tables.recipes.get(id).is_some_and(|r| r.category == category))
        .collect::<Vec<_>>();
    ids.sort_by_key(|id| (tables.recipes[id].order, *id));
    ids
}
