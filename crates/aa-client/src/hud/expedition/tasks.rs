//! Original type3/26/27/35 tasks with captured identities and native result rules.
use super::*;
use aa_net::ExpeditionCommand as Command;
use std::collections::BTreeMap;

struct Quote {
    token: u32,
    owner: u32,
    object: u32,
    command: Command,
    inviter: Vec<u8>,
    shown: bool,
}

#[derive(Default)]
pub(super) struct Tasks {
    next: u32,
    quotes: BTreeMap<u8, Quote>,
}

pub(super) fn clear(view: &mut View) {
    view.tasks.quotes.clear();
}

fn insert(view: &mut View, task: u8, command: Command, inviter: Vec<u8>) {
    let (Some(owner), Some(object)) = (view.cache.local_character_id, view.cache.local_object_id) else {
        return;
    };
    if owner == 0 || object == 0 {
        return;
    }
    view.tasks.next = view.tasks.next.wrapping_add(1).max(1);
    view.tasks.quotes.insert(
        task,
        Quote {
            token: view.tasks.next,
            owner,
            object,
            command,
            inviter,
            shown: false,
        },
    );
}

pub(super) fn confirm(view: &mut View, task: u8, command: Command) {
    insert(view, task, command, Vec::new());
}

fn ignored(lua: &Lua) -> bool {
    lua.load("return X2Option:GetOptionItemValue(OPTION_ITEM_IGNORE_EXPEDITION_INVITATION)==1")
        .eval()
        .unwrap_or(false)
}

fn reply(view: &mut View, command: Command) -> Option<i16> {
    if view.cache.own_id != 0 {
        return Some(0x132);
    }
    actions::queue(view, command);
    None
}

pub(super) fn received(lua: Option<&Lua>, bridge: &Shared, event: &ExpeditionEvent) {
    if matches!(event, ExpeditionEvent::ShowRenameUi) {
        let mut data = bridge.borrow_mut();
        let view = &mut data.expedition;
        if !view.tasks.quotes.contains_key(&35) && view.cache.own_id != 0 {
            if let Some(faction) = view.cache.factions.get(&view.cache.own_id) {
                if faction.political_system == 0 && Some(faction.owner_id) == view.cache.local_character_id {
                    let command = Command::Rename {
                        expedition_id: view.cache.own_id,
                        name: faction.name.clone(),
                    };
                    insert(view, 35, command, Vec::new());
                }
            }
        }
        drop(data);
        if let Some(lua) = lua {
            show(lua, bridge);
        }
        return;
    }
    let ExpeditionEvent::Invitation {
        inviter_id,
        inviter_name,
        expedition_id,
        ..
    } = event
    else {
        return;
    };
    let ignore = lua.is_some_and(ignored);
    let mut data = bridge.borrow_mut();
    let view = &mut data.expedition;
    if view.cache.local_character_id.is_none() || view.cache.local_object_id.is_none() {
        return;
    }
    let command = Command::Reply {
        expedition_id: *expedition_id,
        inviter_id: *inviter_id,
        join: false,
    };
    let error = if ignore || view.tasks.quotes.contains_key(&3) {
        reply(view, command)
    } else {
        insert(
            view,
            3,
            command,
            inviter_name.iter().copied().take(128).take_while(|byte| *byte != 0).collect(),
        );
        None
    };
    drop(data);
    if let Some(lua) = lua {
        if let Some(code) = error {
            let _ = actions::error(lua, code);
        }
        show(lua, bridge);
    }
}

fn choose(view: &mut View, token: u32, accepted: bool) -> Option<i16> {
    let (&task, quote) = view.tasks.quotes.iter().find(|(_, quote)| quote.token == token)?;
    if view.cache.local_character_id != Some(quote.owner) || view.cache.local_object_id != Some(quote.object) {
        return None;
    }
    let mut quote = view.tasks.quotes.remove(&task)?;
    // Original Task35 result callback392f8a70 is a no-op. Its Lua OkProc owns Rename.
    if task == 35 {
        return None;
    }
    if let Command::Reply { join, .. } = &mut quote.command {
        *join = accepted;
        return reply(view, quote.command);
    }
    if accepted {
        actions::queue(view, quote.command)
    }
    None
}

fn localize(lua: &Lua, category: i64, key: &str, rest: Vec<Value>) -> mlua::Result<Value> {
    let locale: Table = lua.globals().get("X2Locale")?;
    let method: mlua::Function = locale.get("LocalizeUiText")?;
    let mut args = vec![
        Value::Table(locale),
        Value::Integer(category),
        Value::String(lua.create_string(key)?),
    ];
    args.extend(rest);
    method.call(MultiValue::from_iter(args))
}

pub(super) fn install(lua: &Lua, bridge: &Shared) -> mlua::Result<()> {
    lua.load("X2Option:CreateOptionItemFloat(OPTION_ITEM_IGNORE_EXPEDITION_INVITATION,0)")
        .exec()?;
    let hud: Table = lua.globals().get("HUD")?;
    let shared = bridge.clone();
    hud.set("expedition_task_current", {
        let shared = bridge.clone();
        lua.create_function(move |_, token: u32| {
            let data = shared.borrow();
            let view = &data.expedition;
            Ok(view.tasks.quotes.values().any(|quote| {
                quote.token == token
                    && view.cache.local_character_id == Some(quote.owner)
                    && view.cache.local_object_id == Some(quote.object)
            }))
        })?
    })?;
    hud.set(
        "expedition_choose",
        lua.create_function(move |lua, (token, accepted): (u32, bool)| {
            let result = choose(&mut shared.borrow_mut().expedition, token, accepted);
            if let Some(code) = result {
                actions::error(lua, code)?
            }
            Ok(())
        })?,
    )?;
    lua.load(include_str!("tasks.lua")).set_name("@expedition_tasks.lua").exec()
}

pub(super) fn show(lua: &Lua, bridge: &Shared) {
    let ignore = ignored(lua);
    let quotes = {
        let mut data = bridge.borrow_mut();
        let view = &mut data.expedition;
        if ignore && view.tasks.quotes.get(&3).is_some_and(|quote| !quote.shown) {
            let quote = view.tasks.quotes.remove(&3).unwrap();
            let error = reply(view, quote.command);
            drop(data);
            if let Some(code) = error {
                let _ = actions::error(lua, code);
            }
            return;
        }
        let cache = &view.cache;
        view.tasks
            .quotes
            .iter_mut()
            .filter_map(|(&task, quote)| {
                if quote.shown || cache.local_character_id != Some(quote.owner) || cache.local_object_id != Some(quote.object) {
                    return None;
                }
                quote.shown = true;
                let name = match quote.command {
                    Command::Reply { expedition_id, .. } => {
                        cache.factions.get(&expedition_id).map(|row| row.name.clone()).unwrap_or_default()
                    }
                    Command::Rename { ref name, .. } => name.clone(),
                    _ => Vec::new(),
                };
                Some((task, quote.token, quote.inviter.clone(), name))
            })
            .collect::<Vec<_>>()
    };
    for (task, token, inviter, expedition) in quotes {
        let result = (|| -> mlua::Result<()> {
            let info = lua.create_table()?;
            info.set("buttonType", 0)?;
            info.set("isModal", task != 3 && task != 35)?;
            if task == 35 {
                info.set("expeditionName", text(lua, &expedition)?)?;
                let hud: Table = lua.globals().get("HUD")?;
                let show: mlua::Function = hud.get("expedition_show_task")?;
                return show.call((task, token, info));
            }
            let key = match task {
                3 => "ask_expedition_invite",
                26 => "ask_leave_expedition",
                27 => "ask_dismiss_expedition",
                _ => return Ok(()),
            };
            info.set("title", localize(lua, 28, key, vec![])?)?;
            let args = if task == 3 {
                vec![Value::String(text(lua, &inviter)?), Value::String(text(lua, &expedition)?)]
            } else {
                vec![]
            };
            info.set("content", localize(lua, 29, key, args)?)?;
            if task == 3 {
                info.set("btnOkText", localize(lua, 27, "invite", vec![])?)?;
                info.set("btnCancelText", localize(lua, 27, "reject", vec![])?)?;
            }
            let hud: Table = lua.globals().get("HUD")?;
            let show: mlua::Function = hud.get("expedition_show_task")?;
            show.call((task, token, info))
        })();
        if let Err(error) = result {
            warn!("hud: expedition task: {error}");
        }
    }
}
