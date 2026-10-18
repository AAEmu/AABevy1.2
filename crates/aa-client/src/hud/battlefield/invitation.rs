//! Native task dialog callbacks retain the full invitation in Rust, including its u64 qualifier.
use super::*;
use aa_net::battlefield::BattlefieldInvitation;

#[derive(Default)]
pub(super) struct View {
    next_token: u32,
    quote: Option<(u32, u32, BattlefieldInvitation)>,
    local_response: Option<(BattlefieldInvitation, bool)>,
}

pub(super) fn reset_ui(view: &mut View) {
    view.quote = None;
}

pub(super) fn install(lua: &Lua, bridge: &Shared) -> mlua::Result<()> {
    let hud: Table = lua.globals().get("HUD")?;
    let state = bridge.clone();
    hud.set(
        "battlefield_choose",
        lua.create_function(move |lua, (token, choice): (u32, i32)| {
            let mut data = state.borrow_mut();
            let view = &data.battlefield;
            let Some((current, owner, invitation)) = view.invitation.quote else {
                return Ok(());
            };
            if token != current
                || owner == 0
                || owner != data.character_id
                || owner != view.owner
                || view.cancelled
                || view.invitation.local_response.is_some()
                || view.received.invitation != Some(invitation)
            {
                return Ok(());
            }
            if choice == 3 {
                data.battlefield.invitation.quote = None;
                drop(data);
                cancel(lua, &state);
                return Ok(());
            }
            if !matches!(choice, 1 | 2) {
                return Ok(());
            }
            match permitted(&data) {
                Some(false) => {
                    drop(data);
                    cancel(lua, &state);
                    return Ok(());
                }
                None => return Ok(()),
                Some(true) => {}
            }
            let accept = choice == 1;
            let Ok(command) = data.battlefield.received.respond_to_invitation(invitation, accept) else {
                return Ok(());
            };
            let view = &mut data.battlefield;
            view.invitation.quote = None;
            view.invitation.local_response = Some((invitation, accept));
            view.requests.push((owner, 0, command));
            Ok(())
        })?,
    )?;
    lua.load(include_str!("dialog.lua")).set_name("@battlefield_dialog.lua").exec()
}

pub(super) fn close(rt: &UiRuntime, bridge: &Shared) {
    bridge.borrow_mut().battlefield.invitation.quote = None;
    if let Err(error) = rt.exec("@battlefield_invitation_close", "HUD.battlefield_close_invitation()") {
        warn!("hud: close battlefield invitation: {error}");
    }
}

pub(super) fn show(rt: &UiRuntime, bridge: &Shared) {
    let info = (|| {
        let mut data = bridge.borrow_mut();
        let invite = data.battlefield.received.invitation?;
        if data.battlefield.cancelled || data.battlefield.received.response_pending.is_some() || data.battlefield.invitation.local_response.is_some() {
            return None;
        }
        let game = data.game.get()?;
        let (field, rule) = game.battlefield.rule(invite.ruleset_id)?;
        let name = field.name.clone();
        let faction = *rule.factions.get(usize::from(invite.corps))?;
        let corps = game.factions.get(&faction)?.clone();
        let view = &mut data.battlefield;
        if view.invitation.quote.is_some_and(|(_, owner, old)| owner == view.owner && old == invite) {
            return None;
        }
        view.invitation.next_token = view.invitation.next_token.wrapping_add(1).max(1);
        let token = view.invitation.next_token;
        view.invitation.quote = Some((token, view.owner, invite));
        Some((token, name, corps))
    })();
    let Some((token, name, corps)) = info else { return };
    let result = (|| -> mlua::Result<()> {
        let hud: Table = rt.lua.globals().get("HUD")?;
        let show: mlua::Function = hud.get("battlefield_show_invitation")?;
        let info = rt.lua.create_table()?;
        // Native task vtable+8/+c, consumed by the default decorator before task-specific layout.
        info.set("buttonType", 2)?; // DBT_CUSTOM
        info.set("isModal", false)?;
        info.set("name", name)?;
        info.set("corpsName", corps)?;
        show.call((token, info))
    })();
    if let Err(error) = result {
        warn!("hud: battlefield invitation: {error}");
    }
}

pub(super) fn world_ready(rt: &UiRuntime, bridge: &Shared) {
    let command = {
        let data = bridge.borrow();
        let view = &data.battlefield;
        view.received
            .invitation
            .and_then(|invite| view.received.world_ready(invite).ok())
            .filter(|_| !view.cancelled)
    };
    if let Some(command) = command {
        let mut data = bridge.borrow_mut();
        let owner = data.character_id;
        if owner != 0 && owner == data.battlefield.owner {
            data.battlefield.requests.push((owner, 0, command));
        }
        drop(data);
        // Original state4 world-enter continuation emits this before writing e4, without admission.
        rt.fire_event("UPDATE_INSTANT_GAME_STATE", ());
    }
}

pub(super) fn received(rt: &UiRuntime, bridge: &Shared, before: Option<BattlefieldInvitation>, was_joined: bool) {
    let (now, entered, left) = {
        let mut data = bridge.borrow_mut();
        let view = &mut data.battlefield;
        let now = view.received.invitation;
        if now.is_none() {
            view.invitation.local_response = None;
        }
        let in_game = view.received.in_instant_game();
        if was_joined && !in_game {
            view.interacted_npc = 0;
        }
        (now, !was_joined && in_game, was_joined && !in_game)
    };
    if before.is_some() && now.is_none() {
        close(rt, bridge);
    }
    if before != now && now.is_some() {
        show(rt, bridge);
    }
    if entered {
        rt.fire_event("ENTERED_INSTANT_GAME_ZONE", ());
    }
    if left {
        rt.fire_event("LEAVED_INSTANT_GAME_ZONE", ());
    }
}
