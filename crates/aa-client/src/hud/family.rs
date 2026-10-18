//! Original relationship-family UI over received roster callbacks and captured commands.
use super::api::{HudBridge, Shared};
use aa_net::{Event, FamilyCommand, NetCommand, NetEvent};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use mlua::{Lua, Table, Value};

#[allow(dead_code)]
pub(super) mod fixture;
mod invitation;
mod state;
#[cfg(test)]
mod tests;

#[derive(Default)]
pub(super) struct View {
    state: state::View,
    changes: Option<bool>,
    requests: Vec<(u32, FamilyCommand)>,
    invitation: invitation::View,
}

fn reset_actor(view: &mut View) {
    let changes = view.changes;
    let invitation = invitation::cleared(std::mem::take(&mut view.invitation));
    *view = View {
        changes,
        invitation,
        ..default()
    };
}

fn notify(lua: &Lua, notice: state::Notice) -> mlua::Result<()> {
    use state::Notice::*;
    let string = |bytes: Vec<u8>| lua.create_string(bytes).map(Value::String);
    let (event, args) = match notice {
        Added(owner, name, title) => ("FAMILY_MEMBER_ADDED", vec![string(owner)?, string(name)?, string(title)?]),
        Left(name, true) => ("FAMILY_MEMBER_KICKED", vec![string(name)?]),
        Left(name, false) => ("FAMILY_MEMBER_LEFT", vec![string(name)?]),
        Owner(name) => ("FAMILY_OWNER_CHANGED", vec![string(name)?]),
        Removed => ("FAMILY_REMOVED", vec![]),
        Online(name, online) => ("FAMILY_MEMBER_ONLINE", vec![string(name)?, Value::Boolean(online)]),
        Refresh => ("FAMILY_REFRESH", vec![]),
    };
    aa_ui::lua::fire_ui_event(lua, event, args);
    Ok(())
}

/// Native checks at the Lua action boundary, independent of the selected-character send guard.
fn action(view: &View, kind: u8, name: &[u8], title: &[u8]) -> Result<Option<FamilyCommand>, i16> {
    let state = &view.state;
    if state.object == 0 || state.character == 0 || (kind != 4 && view.changes != Some(true)) {
        return Ok(None);
    }
    if kind == 0 {
        if state.family != 0 && !state.is_owner() {
            return Err(0x172);
        }
        return Ok(Some(FamilyCommand::Invite {
            name: state::cstr(name, 128),
            title: state::cstr(title, 104),
        }));
    }
    if state.family == 0 {
        return Err(0x178);
    }
    if kind == 1 {
        return Ok(Some(FamilyCommand::Leave));
    }
    if !state.is_owner() {
        return Err(0x172);
    }
    let member = state.member(name).ok_or(0x16fi16)?;
    Ok(Some(match kind {
        2 => FamilyCommand::Kick {
            member_id: member.character_id,
        },
        3 => FamilyCommand::ChangeTitle {
            member_id: member.character_id,
            title: state::cstr(title, 104),
        },
        4 => {
            if member.character_id == state.character {
                return Err(0x211);
            }
            FamilyCommand::ChangeOwner {
                member_id: member.character_id,
            }
        }
        _ => return Ok(None),
    }))
}

fn error(lua: &Lua, code: i16) -> mlua::Result<()> {
    let Some(key) = super::alerts::error_name(code) else { return Ok(()) };
    let locale: Table = lua.globals().get("X2Locale")?;
    let localize: mlua::Function = locale.get("LocalizeUiText")?;
    let group: Value = lua.globals().get("ERROR_MSG")?;
    let message: Value = localize.call((locale, group, key))?;
    aa_ui::lua::fire_ui_event(lua, "SYSMSG", vec![message]);
    Ok(())
}

pub(super) fn install(lua: &Lua, bridge: &Shared) -> mlua::Result<()> {
    let family: Table = lua.globals().get("X2Family")?;
    let shared = bridge.clone();
    family.set(
        "GetMembers",
        lua.create_function(move |lua, (_, offline): (Value, Value)| {
            let Value::Boolean(offline) = offline else {
                return Ok(mlua::MultiValue::new());
            };
            let data = shared.borrow();
            let out = lua.create_table()?;
            if let Some(roster) = data.family.state.roster() {
                for member in roster.members.iter().filter(|member| offline || member.online) {
                    let row = lua.create_table()?;
                    row.set(1, lua.create_string(&member.name)?)?;
                    row.set(2, lua.create_string(&member.title)?)?;
                    row.set(3, f64::from(member.role))?;
                    row.set(4, member.online)?;
                    out.raw_push(row)?;
                }
            }
            Ok(mlua::MultiValue::from_iter([Value::Table(out)]))
        })?,
    )?;
    let shared = bridge.clone();
    family.set(
        "GetMemberCount",
        lua.create_function(move |_, _: Value| {
            let data = shared.borrow();
            Ok(data.family.state.roster().map_or((0., 0.), |roster| {
                (
                    f64::from(roster.members.iter().filter(|m| m.online).count() as f32),
                    f64::from(roster.members.len() as f32),
                )
            }))
        })?,
    )?;
    let shared = bridge.clone();
    family.set("IsOwner", lua.create_function(move |_, _: Value| Ok(shared.borrow().family.state.is_owner()))?)?;
    let shared = bridge.clone();
    family.set(
        "IsFamily",
        lua.create_function(move |_, _: Value| {
            let data = shared.borrow();
            Ok(data.family.state.object != 0 && data.family.state.family != 0)
        })?,
    )?;
    let shared = bridge.clone();
    family.set(
        "IsMyFamily",
        lua.create_function(move |_, (_, name): (Value, mlua::LuaString)| Ok(shared.borrow().family.state.member(&name.as_bytes()).is_some()))?,
    )?;
    for (method, kind) in [("Invite", 0), ("Leave", 1), ("Kick", 2), ("ChangeTitle", 3), ("ChangeOwner", 4)] {
        let shared = bridge.clone();
        family.set(
            method,
            lua.create_function(move |lua, (_, name, title): (Value, Option<mlua::LuaString>, Option<mlua::LuaString>)| {
                let name = name.map_or_else(Vec::new, |text| text.as_bytes().to_vec());
                let title = title.map_or_else(Vec::new, |text| text.as_bytes().to_vec());
                let mut data = shared.borrow_mut();
                let result = action(&data.family, kind, &name, &title);
                match result {
                    Ok(Some(command)) if command.validate().is_ok() => {
                        let owner = data.family.state.character;
                        data.family.requests.push((owner, command));
                    }
                    Err(code) => {
                        drop(data);
                        error(lua, code)?;
                    }
                    _ => {}
                }
                Ok(())
            })?,
        )?;
    }
    invitation::install(lua, bridge)
}

fn dispatch(rt: Option<&UiRuntime>, bridge: &Shared, event: &Event) {
    if let Event::Family(aa_net::family::FamilyEvent::Invitation(invite)) = event {
        invitation::received(rt, bridge, invite);
        return;
    }
    {
        let mut data = bridge.borrow_mut();
        let view = &mut data.family;
        match event {
            Event::InitialConfig(config) => view.changes = config.feature_bytes.first().map(|byte| byte & 2 != 0),
            Event::PlayerSpawn(spawn) => {
                if view.state.character != spawn.character().id {
                    reset_actor(view);
                }
                view.invitation = invitation::cleared(std::mem::take(&mut view.invitation));
                view.requests.clear();
                view.state.actor(spawn.character().id, spawn.obj_id, spawn.character().family);
            }
            Event::Family(receipt) => {
                let notices = view.state.receipt(receipt);
                drop(data);
                if let Some(rt) = rt {
                    for notice in notices {
                        if let Err(error) = notify(&rt.lua, notice) {
                            warn!("hud: family event: {error}");
                        }
                    }
                }
                return;
            }
            Event::CharacterSelected(_) | Event::LeftWorld { .. } => reset_actor(view),
            Event::Kicked { .. } | Event::Disconnected { .. } | Event::LoginFailed { .. } => {
                reset_actor(view);
                view.changes = None;
            }
            Event::FamilyRequestRejected { character_id, message, .. } if *character_id == view.state.character => {
                let message = message.clone();
                drop(data);
                if let Some(rt) = rt {
                    rt.fire_event("SYSMSG", message);
                }
                return;
            }
            _ => {}
        }
    }
    if matches!(
        event,
        Event::PlayerSpawn(_) | Event::CharacterSelected(_) | Event::LeftWorld { .. } | Event::Kicked { .. } | Event::Disconnected { .. }
    ) {
        if let Some(rt) = rt {
            if let Err(error) = rt.exec("@family_lifecycle", "HUD.family_close_invitation()") {
                warn!("hud: family close: {error}");
            }
        }
    }
}

fn flush(data: &mut super::api::HudData, player: &aa_net::state::Player) -> Vec<aa_net::Command> {
    let actual = player.spawn.as_ref().map(|spawn| (spawn.character().id, spawn.obj_id));
    let view = &mut data.family;
    std::mem::take(&mut view.requests)
        .into_iter()
        .filter_map(|(owner, command)| {
            (owner != 0 && owner == data.character_id && actual == Some((owner, view.state.object)) && view.state.character == owner)
                .then_some(aa_net::Command::Family { character_id: owner, command })
        })
        .collect()
}

fn track(
    mut events: MessageReader<NetEvent>,
    rt: Option<NonSend<UiRuntime>>,
    built: Res<super::HudBuilt>,
    bridge: NonSend<HudBridge>,
    config: Option<Res<aa_net::GameConfig>>,
) {
    if let Some(config) = config {
        bridge.0.borrow_mut().family.changes = config
            .received
            .as_ref()
            .and_then(|config| config.feature_bytes.first())
            .map(|byte| byte & 2 != 0);
    }
    let rt = if built.up { rt.as_deref() } else { None };
    for NetEvent(event) in events.read() {
        dispatch(rt, &bridge.0, event);
    }
}

fn update(rt: NonSend<UiRuntime>, bridge: NonSend<HudBridge>, player: Res<aa_net::state::Player>, mut out: MessageWriter<NetCommand>) {
    invitation::show(&rt, &bridge.0);
    for command in flush(&mut bridge.0.borrow_mut(), &player) {
        out.write(NetCommand(command));
    }
}

pub(super) fn plugin(app: &mut App) {
    app.add_systems(Update, track.before(super::sync_hud).before(super::prebuild_hud))
        .add_systems(Update, update.after(super::sync_hud).before(aa_ui::UiFrame).run_if(super::hud_up));
}
