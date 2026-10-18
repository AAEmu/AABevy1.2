//! Native type4 task, retaining inviter/title bytes until a single local answer.
use super::*;
use aa_net::family::FamilyInvitation;

#[derive(Default)]
pub(super) struct View {
    next: u32,
    quote: Option<(u32, u32, FamilyInvitation)>,
    shown: bool,
}

pub(super) fn cleared(mut view: View) -> View {
    view.quote = None;
    view.shown = false;
    view
}

fn reply(view: &mut super::View, owner: u32, invite: FamilyInvitation, join: bool) {
    view.requests.push((
        owner,
        FamilyCommand::Reply {
            inviter_id: invite.inviter_id,
            join,
            role: invite.role,
        },
    ));
}

pub(super) fn install(lua: &Lua, bridge: &Shared) -> mlua::Result<()> {
    // The original checkbox uses numeric option74. Establish its native default before the
    // lazily constructed Options window; the existing account store preserves received edits.
    lua.load("X2Option:CreateOptionItemFloat(OPTION_ITEM_IGNORE_FAMILY_INVITATION,0)").exec()?;
    let hud: Table = lua.globals().get("HUD")?;
    let shared = bridge.clone();
    hud.set(
        "family_choose",
        lua.create_function(move |_, (token, join): (u32, bool)| {
            let mut data = shared.borrow_mut();
            let view = &mut data.family;
            let Some((current, owner, _)) = view.invitation.quote.as_ref() else {
                return Ok(());
            };
            if *current != token || *owner == 0 || *owner != view.state.character || view.state.object == 0 {
                return Ok(());
            }
            let (_, owner, invite) = view.invitation.quote.take().unwrap();
            view.invitation.shown = false;
            reply(view, owner, invite, join);
            Ok(())
        })?,
    )?;
    lua.load(include_str!("dialog.lua")).set_name("@family_dialog.lua").exec()
}

pub(super) fn received(rt: Option<&UiRuntime>, bridge: &Shared, invite: &FamilyInvitation) {
    let ignored = rt.is_some_and(|rt| {
        rt.lua
            .load("return X2Option:GetOptionItemValue(OPTION_ITEM_IGNORE_FAMILY_INVITATION)==1")
            .eval::<bool>()
            .unwrap_or(false)
    });
    let mut data = bridge.borrow_mut();
    let view = &mut data.family;
    let owner = view.state.character;
    // No native player/game: discard silently. Existing-family membership is not a refusal.
    if owner == 0 || view.state.object == 0 {
        return;
    }
    let mut invite = invite.clone();
    invite.inviter_name = state::cstr(&invite.inviter_name, 128);
    invite.role = state::cstr(&invite.role, 104);
    if ignored || view.invitation.quote.is_some() {
        reply(view, owner, invite, false);
        return;
    }
    view.invitation.next = view.invitation.next.wrapping_add(1).max(1);
    view.invitation.quote = Some((view.invitation.next, owner, invite));
    view.invitation.shown = false;
    drop(data);
    if let Some(rt) = rt {
        show(rt, bridge);
    }
}

pub(super) fn show(rt: &UiRuntime, bridge: &Shared) {
    let ignored = rt
        .lua
        .load("return X2Option:GetOptionItemValue(OPTION_ITEM_IGNORE_FAMILY_INVITATION)==1")
        .eval::<bool>()
        .unwrap_or(false);
    let quote = {
        let mut data = bridge.borrow_mut();
        let view = &mut data.family;
        if view.invitation.shown {
            return;
        }
        let Some((token, owner, invite)) = view.invitation.quote.as_ref() else {
            return;
        };
        if *owner != view.state.character || view.state.object == 0 {
            return;
        }
        if ignored {
            let (_, owner, invite) = view.invitation.quote.take().unwrap();
            reply(view, owner, invite, false);
            return;
        }
        let quote = (*token, invite.clone());
        view.invitation.shown = true;
        quote
    };
    let result = (|| -> mlua::Result<()> {
        let (token, invite) = quote;
        let info = rt.lua.create_table()?;
        info.set("buttonType", 0)?;
        info.set("isModal", false)?;
        info.set("invitorName", rt.lua.create_string(invite.inviter_name)?)?;
        info.set("familyTitle", rt.lua.create_string(invite.role)?)?;
        let hud: Table = rt.lua.globals().get("HUD")?;
        let show: mlua::Function = hud.get("family_show_invitation")?;
        show.call((token, info))
    })();
    if let Err(error) = result {
        warn!("hud: family invitation: {error}");
    }
}
