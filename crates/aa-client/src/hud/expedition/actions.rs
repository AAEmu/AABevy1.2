//! Native action admission, with captured-character transport and original task dialogs.
use super::*;
use aa_net::{ExpeditionCommand as Command, ExpeditionMember};

fn cstr(bytes: &[u8]) -> &[u8] {
    bytes.split(|byte| *byte == 0).next().unwrap_or_default()
}

fn member<'a>(view: &'a View, name: &[u8]) -> Option<&'a ExpeditionMember> {
    let name = cstr(name);
    // Native XlCompareStringI is the narrow CRT stricmp. The client currently supports
    // en_us/ASCII character creation; locale-dependent high-byte CRT folding is unclaimed.
    view.cache.members.values().find(|row| cstr(&row.name).eq_ignore_ascii_case(name))
}

/// Original en_us/type2 policy-name path, checked against23 executed native cases.
/// Invalid UTF-8 is refused as an app boundary; native malformed decoding is not claimed.
pub(super) fn valid_policy_name(bytes: &[u8]) -> bool {
    let bytes = cstr(bytes);
    let Ok(name) = std::str::from_utf8(bytes) else { return false };
    let space = |value: u32| matches!(value, 9..=13 | 32);
    let count = name.chars().count();
    let minimum = if bytes.first().is_some_and(u8::is_ascii_alphabetic) { 3 } else { 2 };
    if count < minimum || count > 32 || bytes.first() == Some(&b'.') || name.chars().next().is_some_and(|ch| space(ch as u32)) {
        return false;
    }
    // Preserve the original byte-indexed trailing-space test after codepoint length.
    if bytes.get(count - 1).is_some_and(|byte| space(u32::from(*byte))) {
        return false;
    }
    name.chars()
        .all(|ch| space(ch as u32) || matches!(ch as u32, 0x20..=0x7f | 0xa0..=0x17f))
}

pub(super) fn error(lua: &Lua, code: i16) -> mlua::Result<()> {
    let Some(key) = super::super::alerts::error_name(code) else {
        return Ok(());
    };
    let locale: Table = lua.globals().get("X2Locale")?;
    let localize: mlua::Function = locale.get("LocalizeUiText")?;
    let group: Value = lua.globals().get("ERROR_MSG")?;
    let message: Value = localize.call((locale, group, key))?;
    aa_ui::lua::fire_ui_event(lua, "SYSMSG", vec![message]);
    Ok(())
}

pub(super) fn queue(view: &mut View, command: Command) {
    if let (Some(owner), Some(object)) = (view.cache.local_character_id, view.cache.local_object_id) {
        if owner != 0 && object != 0 && command.validate().is_ok() {
            view.requests.push((owner, object, command));
        }
    }
}

fn dismiss(view: &View) -> Result<(Command, u8), i16> {
    if !view.cache.factions.contains_key(&view.cache.own_id) {
        return Err(0x2e);
    }
    if view.cache.own_role != 255 {
        return Err(0x30);
    }
    Ok((Command::Dismiss, 27))
}

fn plan(view: &View, kind: u8, name: &[u8], new_role: Option<u8>) -> Result<Option<(Command, u8)>, i16> {
    let cache = &view.cache;
    if cache.local_character_id.is_none() || cache.local_object_id.is_none() {
        return Ok(None);
    }
    let faction = cache.factions.get(&cache.own_id);
    let own = cache.my_policy();
    if kind == 0 {
        if faction.is_none() {
            return Err(0x31);
        }
        if own.is_none_or(|row| row.permissions[1] == 0) {
            return Err(0x33);
        }
        return Ok(Some((Command::Invite { name: cstr(name).to_vec() }, 0)));
    }
    if kind == 1 {
        if cache.own_id == 0 {
            return Err(0x137);
        }
        let failure = if faction.is_none() {
            Some(0x3b)
        } else if cache.own_role == 255 {
            Some(0x136)
        } else {
            None
        };
        return if let Some(failure) = failure {
            if cache.members.len() == 1 {
                dismiss(view).map(Some)
            } else {
                Err(failure)
            }
        } else {
            Ok(Some((Command::Leave, 26)))
        };
    }
    if kind == 6 {
        return dismiss(view).map(Some);
    }
    let target = member(view, name).ok_or(if kind == 4 { 0x148i16 } else { 0x135 })?;
    let member_id = target.character_id;
    Ok(Some(match kind {
        2 => {
            let faction = faction.ok_or(0x3ei16)?;
            if faction.mother_id == 0 {
                return Err(0x3f);
            }
            if own.is_none_or(|row| row.permissions[2] == 0) {
                return Err(0x40);
            }
            if Some(member_id) == cache.local_character_id {
                return Err(0x41);
            }
            if cache.own_role < target.role {
                return Err(0x40);
            }
            if faction.political_system == 0 && member_id == faction.owner_id {
                return Err(0x42);
            }
            (Command::Kick { member_id }, 0)
        }
        3 => {
            let Some(role) = new_role else { return Ok(None) };
            if own.is_none_or(|row| row.permissions[3] == 0) || target.role >= cache.own_role || role >= cache.own_role || role == 255 {
                return Err(0x46);
            }
            (Command::ChangeMemberRole { role, member_id }, 0)
        }
        4 => {
            if faction.is_none() || cache.own_role != 255 {
                return Err(0x20);
            }
            match view.dominion.owner_change_blocked(cache.own_id) {
                Some(true) => return Err(0x238),
                Some(false) => {}
                // Native392f4ec0 needs received active-Dominion participants. Unknown
                // eligibility is an app refusal, never fabricated as an empty context.
                None => return Ok(None),
            }
            (Command::ChangeOwner { member_id }, 0)
        }
        _ => return Ok(None),
    }))
}

fn policy_request(view: &View, table: Table) -> mlua::Result<Result<Option<Command>, i16>> {
    if view.cache.own_role != 255 {
        return Ok(Err(0x46));
    }
    let Some(role) = super::role(table.get(1)?) else {
        return Ok(Ok(None));
    };
    let Value::String(name) = table.get::<Value>(2)? else {
        return Ok(Ok(None));
    };
    let name = name.as_bytes();
    if !valid_policy_name(&name) {
        return Ok(Err(0x128));
    }
    let Some(received) = view.cache.policies.get(&(view.cache.own_id, role)) else {
        return Ok(Ok(None));
    };
    let mut policy = received.clone();
    policy.name = cstr(&name).to_vec();
    // The native stack decoder never initializes dismiss. Keep the actual received
    // value, normalized for the writer, as a deliberate app policy for this undefined byte.
    policy.permissions = policy.permissions.map(|value| u8::from(value != 0));
    for (column, index) in [1, 2, 3, 8, 7, 0, 5, 6].into_iter().enumerate() {
        let Value::Boolean(value) = table.get::<Value>(column + 3)? else {
            return Ok(Ok(None));
        };
        policy.permissions[index] = u8::from(value);
    }
    Ok(Ok(Some(Command::ChangePolicy { policy })))
}

pub(super) fn install(lua: &Lua, faction: &Table, bridge: &Shared) -> mlua::Result<()> {
    for (method, kind) in [
        ("InviteToExpedition", 0),
        ("LeaveExpedition", 1),
        ("KickFromExpedition", 2),
        ("ChangeExpeditionMemberRole", 3),
        ("ChangeExpeditionOwner", 4),
        ("DismissExpedition", 6),
    ] {
        let shared = bridge.clone();
        faction.set(
            method,
            lua.create_function(move |lua, (_, name, role): (Value, Value, Value)| {
                let name = match name {
                    Value::String(name) => name.as_bytes().to_vec(),
                    _ if kind == 1 || kind == 6 => Vec::new(),
                    _ => return Ok(()),
                };
                let mut data = shared.borrow_mut();
                let view = &mut data.expedition;
                let result = plan(view, kind, &name, super::role(role));
                match result {
                    Ok(Some((command, 0))) => queue(view, command),
                    Ok(Some((command, task))) => tasks::confirm(view, task, command),
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
    let shared = bridge.clone();
    faction.set(
        "ChangeExpeditionRolePolicy",
        lua.create_function(move |lua, (_, table): (Value, Value)| {
            let Value::Table(table) = table else { return Ok(()) };
            let mut data = shared.borrow_mut();
            match policy_request(&data.expedition, table)? {
                Ok(Some(command)) => queue(&mut data.expedition, command),
                Err(code) => {
                    drop(data);
                    error(lua, code)?;
                }
                _ => {}
            }
            Ok(())
        })?,
    )?;
    Ok(())
}

pub(super) fn flush(data: &mut super::super::api::HudData, player: &aa_net::state::Player) -> Vec<aa_net::Command> {
    let actual = player.spawn.as_ref().map(|spawn| (spawn.character().id, spawn.obj_id));
    let selected = data.character_id;
    let view = &mut data.expedition;
    std::mem::take(&mut view.requests)
        .into_iter()
        .filter_map(|(owner, object, command)| {
            (owner == selected
                && actual == Some((owner, object))
                && view.cache.local_character_id == Some(owner)
                && view.cache.local_object_id == Some(object))
            .then_some(aa_net::Command::Expedition {
                character_id: owner,
                command,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_action_gates_preserve_rank_equality_guard_order_and_confirmation_reroute() {
        let bridge = std::rc::Rc::new(std::cell::RefCell::new(super::super::super::api::HudData::default()));
        fixture::seed(&bridge);
        let mut data = bridge.borrow_mut();
        let view = &mut data.expedition;
        assert!(matches!(plan(view, 0, b"New", None), Ok(Some((Command::Invite { .. }, 0)))));
        assert!(matches!(
            plan(view, 2, b"eLoWeN\0ignored", None),
            Ok(Some((Command::Kick { member_id: 48 }, 0)))
        ));
        assert_eq!(plan(view, 2, b"missing", None).unwrap_err(), 0x135);
        assert_eq!(plan(view, 2, b"Breath fixture", None).unwrap_err(), 0x41);
        assert_eq!(plan(view, 3, b"Elowen", Some(255)).unwrap_err(), 0x46);
        assert!(matches!(
            plan(view, 3, b"Elowen", Some(1)),
            Ok(Some((Command::ChangeMemberRole { role: 1, member_id: 48 }, 0)))
        ));
        assert_eq!(plan(view, 1, b"", None).unwrap_err(), 0x136);
        view.cache.members.retain(|id, _| *id == 47);
        assert!(matches!(plan(view, 1, b"", None), Ok(Some((Command::Dismiss, 27)))));
        view.cache.members.insert(
            48,
            ExpeditionMember {
                character_id: 48,
                name: b"Equal".to_vec(),
                role: 255,
                ..default()
            },
        );
        assert!(
            matches!(plan(view, 2, b"Equal", None), Ok(Some((Command::Kick { member_id: 48 }, 0)))),
            "native kick accepts equal roles when target is not protected owner"
        );
        view.cache.factions.get_mut(&900).unwrap().mother_id = 0;
        view.cache.policies.clear();
        assert_eq!(
            plan(view, 2, b"Equal", None).unwrap_err(),
            0x3f,
            "mother check precedes permission check"
        );
        view.cache.factions.clear();
        assert_eq!(plan(view, 0, b"New", None).unwrap_err(), 0x31);
        assert_eq!(plan(view, 6, b"", None).unwrap_err(), 0x2e);
    }

    #[test]
    fn policy_names_match_executed_original_en_us_validator() {
        for (name, valid) in [
            ("", false),
            ("A", false),
            ("AB", false),
            ("ABC", true),
            ("Leader", true),
            ("12", true),
            (".Abc", false),
            (" Abc", false),
            ("Abc ", false),
            ("A B", true),
            ("A\tB", true),
            ("A\x0bB", true),
            ("éé", true),
            ("é ", true),
            ("Aé B", false),
            ("AéB", true),
            ("ſſ", true),
            ("ƀƀ", false),
            ("一一", false),
            ("!!", true),
            ("\u{a0}\u{a0}", true),
        ] {
            assert_eq!(valid_policy_name(name.as_bytes()), valid, "{name:?}");
        }
        assert!(valid_policy_name("A".repeat(32).as_bytes()));
        assert!(!valid_policy_name("A".repeat(33).as_bytes()));
        assert!(!valid_policy_name(&[255, 255]));
    }
}
