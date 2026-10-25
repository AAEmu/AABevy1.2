//! Original guild creation/rename controls.
use super::*;

fn create(view: &View, name: &[u8], sponsor_id: u32) -> Result<aa_net::ExpeditionCommand, i16> {
    if !actions::valid_policy_name(name) {
        return Err(0x22);
    }
    let cache = &view.cache;
    if cache.own_id != 0 || !cache.factions.contains_key(&cache.current_faction_id) {
        return Err(0x21);
    }
    // Audited original1.2 runtime39ac72b0=1000, also used by the original Lua.
    // Its protected initializer remains unresolved; this is not a received GameConfig field.
    let sponsor_id = if cache.current_faction_id >= 1000 {
        cache.current_faction_id
    } else {
        sponsor_id
    };
    Ok(aa_net::ExpeditionCommand::Create {
        name: name
            .split(|byte| *byte == 0)
            .next()
            .unwrap_or_default()
            .to_vec(),
        sponsor_id,
    })
}

pub(super) fn install(lua: &Lua, faction: &Table, bridge: &Shared) -> mlua::Result<()> {
    let shared = bridge.clone();
    faction.set(
        "GetMyTopLevelFaction",
        lua.create_function(move |_, _: Value| {
            Ok(match shared.borrow().expedition.cache.top_level_faction() {
                Some(id) => MultiValue::from_iter([Value::Number(f64::from(id as f32))]),
                None => MultiValue::new(), // Native malformed cycles do not terminate; app refuses.
            })
        })?,
    )?;
    let shared = bridge.clone();
    faction.set(
        "CreateExpedition",
        lua.create_function(move |lua, (_, name, sponsor): (Value, Value, Value)| {
            let (Value::String(name), Some(sponsor)) = (name, members::integer(sponsor)) else {
                return Ok(());
            };
            let mut data = shared.borrow_mut();
            let scope = data.expedition.creation_scope;
            if scope.is_some_and(|scope| !super::super::dialog::scope_current(&data, scope)) {
                return Ok(());
            }
            let view = &mut data.expedition;
            if view.cache.local_character_id.is_none() || view.cache.local_object_id.is_none() {
                return Ok(());
            }
            match create(view, &name.as_bytes(), sponsor as u32) {
                Ok(command) => {
                    if let Some(scope) = scope {
                        view.creation_requests.push((scope, command));
                    } else {
                        actions::queue(view, command);
                    }
                }
                Err(code) => {
                    drop(data);
                    actions::error(lua, code)?;
                }
            }
            Ok(())
        })?,
    )?;
    let shared = bridge.clone();
    faction.set(
        "RenameExpedition",
        lua.create_function(move |_, (_, name): (Value, Value)| {
            let Value::String(name) = name else {
                return Ok(MultiValue::new());
            };
            let mut data = shared.borrow_mut();
            let view = &mut data.expedition;
            let command = aa_net::ExpeditionCommand::Rename {
                expedition_id: view.cache.own_id,
                name: name
                    .as_bytes()
                    .split(|byte| *byte == 0)
                    .next()
                    .unwrap_or_default()
                    .to_vec(),
            };
            // The direct native wrapper does not validate names, ownership or membership.
            // Its true result reports invocation only; actual actor/write guards stay in queue/flush.
            actions::queue(view, command);
            Ok(MultiValue::from_iter([Value::Boolean(true)]))
        })?,
    )?;
    Ok(())
}

fn valid_service(
    data: &super::super::api::HudData,
    units: &aa_net::state::Units,
    npc: (u32, u32),
) -> bool {
    data.dialog.npc == Some(npc)
        && data.game.get().is_some_and(|game| game.expedition.is_creator(npc.1))
        && units.get(npc.0).is_some_and(|unit| {
            !unit.dead
                && matches!(unit.kind, aa_net::model::UnitKind::Npc { template_id, .. } if template_id == npc.1)
                && data.pos.distance(Vec3::new(unit.pos.x, unit.pos.y, unit.pos.z)) <= 6.0
        })
}

pub(in crate::hud) fn start_selected(
    rt: &UiRuntime,
    bridge: &Shared,
    selected: super::super::dialog::SelectedNpc,
    player: &aa_net::state::Player,
    units: &aa_net::state::Units,
) -> bool {
    let Some(spawn) = &player.spawn else {
        return false;
    };
    let mut data = bridge.borrow_mut();
    let npc = selected.npc();
    if !super::super::dialog::scope_current(&data, selected.scope())
        || data.character_id != spawn.character().id
        || data.expedition.cache.local_character_id != Some(spawn.character().id)
        || data.expedition.cache.local_object_id != Some(spawn.obj_id)
        || !valid_service(&data, units, npc)
    {
        return false;
    }
    data.expedition.creation_npc = Some(npc);
    data.expedition.creation_scope = Some(selected.scope());
    drop(data);
    rt.fire_event(
        "NPC_INTERACTION_START",
        ("expedition", "", npc.0.to_string()),
    );
    true
}

pub(super) fn check_service(_rt: &UiRuntime, bridge: &Shared, units: &aa_net::state::Units) {
    let mut data = bridge.borrow_mut();
    if data
        .expedition
        .creation_npc
        .is_some_and(|npc| !valid_service(&data, units, npc))
        || (data.expedition.creation_npc.is_some()
            && data
                .expedition
                .creation_scope
                .is_some_and(|scope| !super::super::dialog::scope_current(&data, scope)))
    {
        data.expedition.creation_npc = None;
        if let Some(scope) = data.expedition.creation_scope {
            super::super::dialog::close_npc(&mut data, scope);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creation_checks_name_before_membership_and_uses_current_faction_sponsor_override() {
        let mut view = View::default();
        assert_eq!(create(&view, b"AB", 101), Err(0x22));
        assert_eq!(create(&view, b"Silver Vale", 101), Err(0x21));
        view.cache.current_faction_id = 101;
        view.cache.factions.insert(
            101,
            aa_net::ExpeditionFaction {
                id: 101,
                ..default()
            },
        );
        assert_eq!(
            create(&view, b"Silver Vale\0tail", u32::MAX),
            Ok(aa_net::ExpeditionCommand::Create {
                name: b"Silver Vale".to_vec(),
                sponsor_id: u32::MAX
            })
        );
        view.cache.own_id = 900;
        assert_eq!(create(&view, b"AB", 101), Err(0x22));
        assert_eq!(create(&view, b"Silver Vale", 101), Err(0x21));
        view.cache.own_id = 0;
        view.cache.current_faction_id = 1005;
        view.cache.factions.insert(
            1005,
            aa_net::ExpeditionFaction {
                id: 1005,
                ..default()
            },
        );
        assert_eq!(
            create(&view, b"Silver Vale", 101),
            Ok(aa_net::ExpeditionCommand::Create {
                name: b"Silver Vale".to_vec(),
                sponsor_id: 1005
            })
        );
    }
}
