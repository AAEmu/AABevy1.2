//! Appellations (titles) over the list aa-net folds from `SCAppellations` / `SCAppellationGained` /
//! `SCAppellationChanged`: the title window of the character window (`x2ui/chracterinfo/appellation.alb`)
//! asks `X2Player:GetAppellationCount`, `GetAppellations(page)`, `GetShowingAppellation` and
//! `ChangeAppellation(type)`; the last one sends `CSChangeAppellation`.
//!
//! The server answers a change with `SCAppellationChanged`; the scripts hear it as `APPELLATION_CHANGED`
//! (with the unit id of the player, the label of the character window refreshes on it) and a gained title as
//! `APPELLATION_GAINED` (the title's name; the chat prints "gained the title").
use super::api::{HudBridge, HudData, Shared};
use aa_net::{Appellations, Command, NetCommand};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use mlua::{Lua, Table};
#[cfg(test)]
mod tests;

/// Titles on a page of the window: its `MAX_COUNT` (6) buttons less the first one, which takes the title off
/// (`appellation.alb` fills button `i + 1` with title `i` and hands the page control `MAX_COUNT - 1`).
pub(super) const PAGE_SIZE: usize = 5;

#[derive(Default)]
pub(super) struct View {
    /// The list as last folded from aa-net (what the getters read).
    titles: Appellations,
    /// The revision whose `APPELLATION_CHANGED` was raised.
    raised: u32,
    requests: Vec<u32>,
    /// A list set by `HUD.appellation_preview` (dev only): aa-net's list no longer overwrites it.
    preview: bool,
}

/// The raw row the script side builds the `APPELLATION_INFO` tables from: `id`, `name`, and, when the title has a
/// buff, `buff_id`, `buff_name`, `path` (icon), `description`, `buff_kind`.
fn row(lua: &Lua, d: &HudData, id: u32) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.raw_set("id", id)?;
    let game = d.game.get();
    let title = game.and_then(|g| g.appellations.get(&id));
    t.raw_set("name", title.map_or("", |a| a.name.as_str()))?;
    if let (Some(g), Some(buff_id)) = (game, title.map(|a| a.buff_id).filter(|b| *b != 0))
        && let Some(buff) = g.buffs.get(&buff_id)
    {
        t.raw_set("buff_id", buff_id)?;
        t.raw_set("buff_name", buff.named.name.as_str())?;
        t.raw_set("path", buff.named.icon.as_str())?;
        t.raw_set("buff_kind", buff.kind)?;
        t.raw_set("description", super::tips::buff_text(g, &d.tips, buff_id, 0).unwrap_or_else(|| buff.desc.clone()))?;
    }
    Ok(t)
}

/// The owned titles of a page (1 based, [`PAGE_SIZE`] each).
fn page_ids(titles: &Appellations, page: i64) -> &[u32] {
    let start = usize::try_from(page.max(1) - 1).unwrap_or(0).saturating_mul(PAGE_SIZE);
    let owned = &titles.owned;
    &owned[start.min(owned.len())..owned.len().min(start.saturating_add(PAGE_SIZE))]
}

pub(super) fn install(lua: &Lua, state: &Shared) -> mlua::Result<()> {
    let hud: Table = lua.globals().get("HUD")?;
    let s = state.clone();
    hud.raw_set("appellation_count", lua.create_function(move |_, ()| Ok(s.borrow().appellation.titles.owned.len()))?)?;
    let s = state.clone();
    hud.raw_set(
        "appellation_page",
        lua.create_function(move |lua, page: Option<i64>| {
            let d = s.borrow();
            let out = lua.create_table()?;
            for &id in page_ids(&d.appellation.titles, page.unwrap_or(1)) {
                out.raw_push(row(lua, &d, id)?)?;
            }
            Ok(out)
        })?,
    )?;
    let s = state.clone();
    hud.raw_set(
        "appellation_active",
        lua.create_function(move |lua, ()| {
            let d = s.borrow();
            match d.appellation.titles.active {
                0 => Ok(None),
                id => row(lua, &d, id).map(Some),
            }
        })?,
    )?;
    // true when the request went out: the server owns the decision (an id the character does not own is refused there)
    let s = state.clone();
    hud.raw_set(
        "appellation_change",
        lua.create_function(move |_, id: u32| {
            let mut d = s.borrow_mut();
            if d.disconnected || d.character_id == 0 || (id != 0 && !d.appellation.titles.is_owned(id)) {
                return Ok(false);
            }
            info!("hud: appellation {id} requested");
            d.appellation.requests.push(id);
            Ok(true)
        })?,
    )?;
    // Dev only (`AA_APPELLATION_PREVIEW=1`): show a list the server did not send, to look at the window without a GM
    // account. The requests still go to the server, which refuses a title the character does not own.
    if std::env::var_os("AA_APPELLATION_PREVIEW").is_some() {
        let s = state.clone();
        hud.raw_set(
            "appellation_preview",
            lua.create_function(move |_, (ids, active): (Vec<u32>, u32)| {
                let mut d = s.borrow_mut();
                d.appellation.preview = true;
                d.appellation.titles.owned = ids;
                d.appellation.titles.active = active;
                d.appellation.titles.revision = d.appellation.titles.revision.wrapping_add(1);
                Ok(())
            })?,
        )?;
    }
    lua.load(include_str!("appellation.lua")).set_name("@appellation.lua").exec()
}

/// Folds aa-net's list into the view and raises the scripts' events.
fn track(mut titles: ResMut<Appellations>, bridge: NonSend<HudBridge>, rt: Option<NonSend<UiRuntime>>, built: Res<super::HudBuilt>) {
    let changed = {
        let mut d = bridge.0.borrow_mut();
        if titles.is_changed() && !d.appellation.preview {
            d.appellation.titles = titles.clone();
        }
        let changed = d.appellation.raised != titles.revision;
        if changed && built.up {
            d.appellation.raised = titles.revision;
        }
        changed && built.up
    };
    let (Some(rt), true) = (rt.as_deref(), built.up) else { return };
    let gained = if titles.gained.is_empty() { Vec::new() } else { titles.bypass_change_detection().take_gained() };
    if changed {
        // the handlers compare the unit with `X2Unit:GetUnitId("player")`
        match rt.lua.load("return X2Unit:GetUnitId('player')").eval::<mlua::Value>() {
            Ok(unit) if !unit.is_nil() => {
                debug!("hud: APPELLATION_CHANGED");
                rt.fire_event("APPELLATION_CHANGED", unit);
            }
            Ok(_) => {}
            Err(error) => debug!("hud: player unit id: {error}"),
        }
    }
    for id in gained {
        let name = {
            let d = bridge.0.borrow();
            d.game.get().and_then(|g| g.appellations.get(&id)).map(|a| a.name.clone()).unwrap_or_default()
        };
        info!("hud: appellation {id} gained ({name})");
        rt.fire_event("APPELLATION_GAINED", name);
    }
}

fn requests(bridge: NonSend<HudBridge>, mut out: MessageWriter<NetCommand>) {
    let ids = std::mem::take(&mut bridge.0.borrow_mut().appellation.requests);
    out.write_batch(ids.into_iter().map(|id| NetCommand(Command::ChangeAppellation { id })));
}

pub(super) fn plugin(app: &mut App) {
    app.add_systems(Update, track.before(aa_ui::UiInput).before(super::sync_hud).before(super::prebuild_hud).before(super::build_hud))
        .add_systems(Update, requests.after(super::sync_hud).after(aa_ui::UiInput));
}
