//! The original item target mode is independent of the held drag cursor.
use super::{
    super::{
        api::{HudBridge, HudData, Request, Shared},
        enchant,
        usable::{Env, Fail, InFlight},
    },
    controller as c,
};
use aa_net::{
    item_effect::CapturedBagItem,
    look_conversion::{ItemLookOperation, ItemLookRequest},
    state::{Buffs, Inventory, Skills},
};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use mlua::{Lua, MultiValue, ObjectLike, Table};

#[derive(Clone, Copy, Debug)]
pub(crate) struct ItemLookSource {
    pub item_id: u64,
    pub template: u32,
    pub skill: u32,
}
#[derive(Clone)]
pub(super) struct Target {
    owner: u32,
    actor: u32,
    skill: u32,
    source: CapturedBagItem,
}
#[derive(Clone)]
pub(super) struct Task {
    token: u64,
    owner: u32,
    actor: u32,
    request: ItemLookRequest,
    shown: bool,
}
pub(in crate::hud) enum Admission {
    Unsupported,
    Held,
    Failed(Fail),
    Ready,
}

fn supported(template: u32, skill: u32) -> bool {
    matches!((template, skill), (29880, 23741) | (29704, 23457))
}
fn admission(
    data: &HudData,
    env: &Env,
    flight: &InFlight,
    source: &CapturedBagItem,
    skill: u32,
) -> Admission {
    let Some(game) = data.game.get() else {
        return Admission::Unsupported;
    };
    if data.disconnected
        || data.character_id == 0
        || enchant::records(data).owner() != Some(data.character_id)
        || enchant::actor(data).is_none()
        || env
            .skills
            .is_none_or(|skills| Some(skills.obj_id) != enchant::actor(data))
        || env.inventory.is_none()
        || !supported(source.item.template_id, skill)
        || game
            .items
            .get(&source.item.template_id)
            .is_none_or(|item| item.use_skill != skill)
        || !game.skills.contains_key(&skill)
        || !game.rules.knows(skill)
        || enchant::records(data).item(2, source.slot) != Some(&source.item)
    {
        return Admission::Unsupported;
    }
    if env.held_back(skill, flight) {
        return Admission::Held;
    }
    if let Some((_, fail)) = env.precheck_item(source.item.template_id) {
        return Admission::Failed(fail);
    }
    Admission::Ready
}
pub(in crate::hud) fn admit_request(
    data: &HudData,
    env: &Env,
    flight: &InFlight,
    request: &ItemLookRequest,
) -> Admission {
    let Some(skill) = request.skill_id() else {
        return Admission::Unsupported;
    };
    if request.recheck_native(enchant::records(data)).is_err() {
        return Admission::Unsupported;
    }
    admission(data, env, flight, request.source(), skill)
}
fn skinizing(data: &HudData) -> bool {
    data.look_conversion
        .target
        .as_ref()
        .is_some_and(|target| target.skill == 23741)
}
pub(in crate::hud) fn cancel_target(lua: &Lua, state: &Shared) -> bool {
    let Some(skill) = state
        .borrow()
        .look_conversion
        .target
        .as_ref()
        .map(|target| target.skill)
    else {
        return false;
    };
    //39426af0 invokes the original532 listeners while mode is still4.
    if skill == 23741 {
        aa_ui::lua::fire_ui_event(lua, "LEAVE_ITEM_SKINIZE_MODE", Vec::new());
    }
    state.borrow_mut().look_conversion.target = None;
    state.borrow_mut().look_conversion.fresh = false;
    true
}
pub(in crate::hud) fn escape(rt: &UiRuntime, state: &Shared) -> bool {
    cancel_target(&rt.lua, state)
}
fn clear_held(lua: &Lua) -> mlua::Result<()> {
    lua.globals()
        .get::<Table>("X2Cursor")?
        .call_method("ClearCursor", ())
}
fn cancel_task(lua: Option<&Lua>, state: &Shared) {
    let old = state.borrow_mut().look_conversion.task.take();
    if let Some(task) = old {
        enchant::lock_bag(&mut state.borrow_mut(), task.request.target().slot, false);
        if let Some(lua) = lua {
            let _ = clear_held(lua);
        }
    }
}
pub(super) fn retire(lua: Option<&Lua>, state: &Shared) {
    if let Some(lua) = lua {
        cancel_target(lua, state);
    } else {
        state.borrow_mut().look_conversion.target = None;
    }
    cancel_task(lua, state);
    state.borrow_mut().look_conversion.pointer = Default::default();
}
fn enter(lua: &Lua, state: &Shared, target: Target) {
    enter_modal(lua, state, target, None);
}
fn enter_modal(
    lua: &Lua,
    state: &Shared,
    target: Target,
    aiming: Option<&mut crate::ingame::ground_target::Aiming>,
) {
    //390468b0 consumes the post-admission entry when enchant mode is active.
    if enchant::active(&state.borrow()) {
        return;
    }
    c::switch(lua, state, false);
    cancel_target(lua, state);
    if let Some(aiming) = aiming {
        aiming.cancel();
    } else {
        state.borrow_mut().look_conversion.cancel_aim_pending = true;
    }
    let skin = target.skill == 23741;
    {
        let mut data = state.borrow_mut();
        data.look_conversion.target = Some(target);
        data.look_conversion.fresh = true;
    }
    if skin {
        aa_ui::lua::fire_ui_event(lua, "ENTER_ITEM_SKINIZE_MODE", Vec::new());
    }
}
pub(super) fn choose(lua: &Lua, state: &Shared, slot: u8) -> mlua::Result<bool> {
    let target = state.borrow().look_conversion.target.clone();
    let Some(target) = target else {
        return Ok(false);
    };
    // Ordinary BAG routing rejects an unknown/pending physical record before
    // entering3942fb50; retain targeting for those early exits.
    {
        let mut data = state.borrow_mut();
        if enchant::bag_pending(&data, slot) {
            return Ok(true);
        }
        let Some(item) = enchant::records(&data).item(2, slot) else {
            return Ok(true);
        };
        if item.template_id == 0 {
            data.look_conversion.target = None;
            return Ok(true);
        }
    }
    cancel_target(lua, state);
    let prepared = {
        let data = state.borrow();
        if target.owner != data.character_id || Some(target.actor) != enchant::actor(&data) {
            return Ok(true);
        }
        let records = enchant::records(&data);
        // Native source lookup follows its full ID when it moves within BAG.
        let source = records.bag_items().find_map(|(slot, item)| {
            (item.id == target.source.item.id).then(|| CapturedBagItem {
                slot,
                item: item.clone(),
            })
        });
        let Some(source) = source else {
            return Ok(true);
        };
        if source.item != target.source.item || enchant::bag_pending(&data, source.slot) {
            return Ok(true);
        }
        let Some(item) = c::item(&data, slot) else {
            return Ok(true);
        };
        let recipient = CapturedBagItem {
            slot,
            item: item.clone(),
        };
        let request = if target.skill == 23741 {
            ItemLookRequest::skinize(target.skill, source, recipient)
        } else {
            ItemLookRequest::cleanup(target.skill, source, recipient)
        };
        request.ok()
    };
    let Some(request) = prepared else {
        return Ok(true);
    };
    if request.operation() == ItemLookOperation::Cleanup {
        state.borrow_mut().requests.push(Request::ItemLook {
            character_id: target.owner,
            request,
        });
        return Ok(true);
    }
    // A new valid task replaces the old task, not merely a reentered cursor.
    cancel_task(Some(lua), state);
    {
        let mut data = state.borrow_mut();
        let Some(token) = data.look_conversion.next_task.checked_add(1) else {
            return Ok(true);
        };
        data.look_conversion.next_task = token;
        enchant::lock_bag(&mut data, request.target().slot, true);
        data.look_conversion.task = Some(Task {
            token,
            owner: target.owner,
            actor: target.actor,
            request,
            shown: false,
        });
    }
    show_pending(lua, state)?;
    Ok(true)
}
fn show_pending(lua: &Lua, state: &Shared) -> mlua::Result<()> {
    let task = state.borrow().look_conversion.task.clone();
    let Some(task) = task.filter(|task| !task.shown) else {
        return Ok(());
    };
    let request = &task.request;
    let info = {
        let data = state.borrow();
        let Some(item_info) = c::tooltip(lua, &data, &request.target().item)? else {
            return Ok(());
        };
        let info = lua.create_table()?;
        info.raw_set("itemInfo", item_info)?;
        info.raw_set("scrollType", request.source().item.template_id)?;
        info.raw_set("scrollGrade", request.source().item.grade)?;
        info
    };
    let hud: Table = lua.globals().get("HUD")?;
    let shown: bool = hud.call_function("show_look_task", (task.token.to_string(), info))?;
    if shown
        && let Some(current) = state
            .borrow_mut()
            .look_conversion
            .task
            .as_mut()
            .filter(|current| current.token == task.token)
    {
        current.shown = true;
    }
    Ok(())
}
fn finish(lua: &Lua, state: &Shared, token: u64, accepted: bool) -> mlua::Result<()> {
    let task = {
        let mut data = state.borrow_mut();
        if data
            .look_conversion
            .task
            .as_ref()
            .is_none_or(|task| task.token != token)
        {
            return Ok(());
        }
        let task = data.look_conversion.task.take().unwrap();
        enchant::lock_bag(&mut data, task.request.target().slot, false);
        task
    };
    if !accepted {
        clear_held(lua)?;
        return Ok(());
    }
    let current = {
        let data = state.borrow();
        data.character_id == task.owner
            && enchant::actor(&data) == Some(task.actor)
            && task.request.recheck_native(enchant::records(&data)).is_ok()
    };
    if current {
        state.borrow_mut().requests.push(Request::ItemLook {
            character_id: task.owner,
            request: task.request,
        });
    }
    Ok(())
}
pub(super) fn install(lua: &Lua, state: &Shared) -> mlua::Result<()> {
    let item: Table = lua.globals().get("X2Item")?;
    let s = state.clone();
    item.raw_set(
        "IsInItemSkinizeMode",
        lua.create_function(move |_, _: MultiValue| Ok(skinizing(&s.borrow())))?,
    )?;
    let hud: Table = lua.globals().get("HUD")?;
    let s = state.clone();
    hud.raw_set(
        "look_target",
        lua.create_function(move |lua, (slot, button): (i32, Option<String>)| {
            if s.borrow().look_conversion.target.is_some() {
                super::pointer::claim(
                    &mut s.borrow_mut(),
                    if button.as_deref() == Some("RightButton") {
                        MouseButton::Right
                    } else {
                        MouseButton::Left
                    },
                );
            }
            choose(lua, &s, slot.wrapping_sub(1) as u8)
        })?,
    )?;
    let s = state.clone();
    hud.raw_set(
        "look_task_finish",
        lua.create_function(move |lua, (token, accepted): (String, bool)| {
            if let Ok(token) = token.parse() {
                finish(lua, &s, token, accepted)?;
            }
            Ok(())
        })?,
    )?;
    lua.load(include_str!("target.lua"))
        .set_name("@look_target.lua")
        .exec()?;
    super::pointer::install(lua, state)
}
fn replay_task(state: NonSend<HudBridge>, rt: Option<NonSend<UiRuntime>>) {
    if let Some(rt) = rt.as_deref()
        && let Err(error) = show_pending(&rt.lua, &state.0)
    {
        warn!("look task presentation: {error}");
    }
}
pub(super) fn prepare_source(
    data: &HudData,
    source: ItemLookSource,
    skills: &Skills,
    own_buffs: &[aa_net::state::BuffEntry],
    inventory: &Inventory,
    flight: &InFlight,
) -> Option<(Admission, Target)> {
    let record = enchant::records(data)
        .bag_items()
        .find_map(|(slot, item)| {
            (item.id == source.item_id && item.template_id == source.template).then(|| {
                CapturedBagItem {
                    slot,
                    item: item.clone(),
                }
            })
        })?;
    if enchant::bag_pending(data, record.slot) {
        return None;
    }
    let env = Env::of(data, Some(skills), own_buffs, Some(inventory));
    Some((
        admission(data, &env, flight, &record, source.skill),
        Target {
            owner: data.character_id,
            actor: skills.obj_id,
            skill: source.skill,
            source: record,
        },
    ))
}
pub(super) fn apply_source(
    lua: &Lua,
    state: &Shared,
    prepared: Option<(Admission, Target)>,
    aiming: Option<&mut crate::ingame::ground_target::Aiming>,
) -> Option<Fail> {
    match prepared? {
        (Admission::Ready, target) => {
            enter_modal(lua, state, target, aiming);
            None
        }
        (Admission::Failed(fail), _) => Some(fail),
        _ => None,
    }
}
pub(crate) fn source_use(
    rt: &UiRuntime,
    state: &Shared,
    source: ItemLookSource,
    (skills, buffs, inventory, flight): (&Skills, &Buffs, &Inventory, &InFlight),
    aiming: Option<&mut crate::ingame::ground_target::Aiming>,
) -> Option<Fail> {
    let prepared = {
        let data = state.borrow();
        prepare_source(
            &data,
            source,
            skills,
            buffs.of(skills.obj_id),
            inventory,
            flight,
        )
    };
    apply_source(&rt.lua, state, prepared, aiming)
}
pub(in crate::hud) fn plugin(app: &mut App) {
    super::pointer::plugin(app);
    app.add_systems(
        Update,
        replay_task
            .after(crate::ingame::ground_target::ItemUseRouting)
            .run_if(super::super::hud_up)
            .run_if(in_state(crate::Screen::InGame)),
    );
}

/// Visual fixture: actual ordinary admission over the provided received records,
/// followed by the same post-admission entry. No protected native planner/socket.
pub(super) fn fixture_source(
    rt: &UiRuntime,
    state: &Shared,
    inventory: &Inventory,
    source_id: u64,
) {
    let target = {
        let data = state.borrow();
        let (slot, item) = enchant::records(&data)
            .bag_items()
            .find(|(_, item)| item.id == source_id)
            .unwrap();
        let skill = data.game.get().unwrap().items[&item.template_id].use_skill;
        let skills = Skills {
            obj_id: enchant::actor(&data).unwrap(),
            ..Default::default()
        };
        let env = Env::of(&data, Some(&skills), &[], Some(inventory));
        let source = CapturedBagItem {
            slot,
            item: item.clone(),
        };
        assert!(matches!(
            admission(&data, &env, &InFlight::default(), &source, skill),
            Admission::Ready
        ));
        Target {
            owner: data.character_id,
            actor: skills.obj_id,
            source,
            skill,
        }
    };
    enter(&rt.lua, state, target);
}
#[cfg(test)]
mod tests;
