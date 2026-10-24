//! Per-release item targeting over actual prepared admission resources.
//! No World/UI borrow crosses a Lua callback; the toolkit owns release delivery.
use super::{
    super::{
        api::{HudBridge, HudData, Shared},
        enchant,
        usable::{Fail, InFlight},
    },
    target,
};
use aa_net::{
    items::ActionKind,
    state::{ActionBar, BuffEntry, Buffs, Inventory, Skills},
};
use aa_ui::UiRuntime;
use bevy::{prelude::*, window::PrimaryWindow};
use mlua::{Function, Lua, MultiValue, ObjectLike, Table, Value};

#[derive(Default)]
pub(super) struct PointerUse {
    interval: bool,
    ready: bool,
    owner: u32,
    actor: u32,
    skills: Skills,
    buffs: Vec<BuffEntry>,
    inventory: Inventory,
    bar: ActionBar,
    flight: InFlight,
    claimed: [bool; 2],
    delivered_left: bool,
    cursor_present: bool,
    failures: Vec<(u32, Fail)>,
    #[cfg(test)]
    copies: [usize; 4],
}

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct PointerPrepared;
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct PointerFinished;

fn button_index(button: MouseButton) -> usize {
    usize::from(button == MouseButton::Right)
}
pub(super) fn claim(data: &mut HudData, button: MouseButton) {
    data.look_conversion.pointer.claimed[button_index(button)] = true;
}
pub(crate) fn blocks(data: &HudData, button: MouseButton) -> bool {
    data.look_conversion.target.is_some()
        || data.look_conversion.pointer.claimed[button_index(button)]
}

fn begin(bridge: NonSend<HudBridge>) {
    let mut data = bridge.0.borrow_mut();
    let cache = &mut data.look_conversion.pointer;
    cache.interval = false;
    cache.claimed = [false; 2];
    cache.delivered_left = false;
}

// Clones are tied to resource changes, not input polling. The native clock can
// advance through bypass_change_detection even when no timer rows change.
impl PointerUse {
    #[allow(clippy::too_many_arguments)]
    fn update(
        &mut self,
        owner: u32,
        skills: &Skills,
        buffs: &[BuffEntry],
        inventory: &Inventory,
        bar: &ActionBar,
        flight: &InFlight,
        changed: [bool; 4],
    ) {
        let initial = !self.ready || self.owner != owner || self.actor != skills.obj_id;
        for (i, copy) in changed.into_iter().enumerate() {
            if initial || copy {
                match i {
                    0 => self.skills.clone_from(skills),
                    1 => {
                        self.buffs.clear();
                        self.buffs.extend_from_slice(buffs);
                    }
                    2 => self.inventory.clone_from(inventory),
                    _ => self.bar.clone_from(bar),
                }
                #[cfg(test)]
                {
                    self.copies[i] += 1;
                }
            }
        }
        self.skills.native_last_frame_ms = skills.native_last_frame_ms;
        self.flight = *flight;
        self.owner = owner;
        self.actor = skills.obj_id;
        self.ready = true;
    }
}

#[allow(clippy::too_many_arguments)]
fn prepare(
    bridge: NonSend<HudBridge>,
    rt: Option<NonSend<UiRuntime>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    skills: Option<Res<Skills>>,
    buffs: Option<Res<Buffs>>,
    inventory: Option<Res<Inventory>>,
    bar: Option<Res<ActionBar>>,
    flight: Option<Res<InFlight>>,
    virt: Res<aa_ui::UiVirtualPointer>,
) {
    let gate = rt.is_some()
        && windows
            .single()
            .is_ok_and(|w| w.resolution.physical_width() > 0 && w.resolution.physical_height() > 0);
    let cursor_present = virt.pos.is_some()
        || windows
            .single()
            .is_ok_and(|w| w.cursor_position().is_some());
    let mut data = bridge.0.borrow_mut();
    let owner = data.character_id;
    let actor = enchant::actor(&data);
    let current =
        !data.disconnected && owner != 0 && enchant::records(&data).owner() == Some(owner);
    let cache = &mut data.look_conversion.pointer;
    cache.interval = gate;
    cache.cursor_present = cursor_present;
    let (Some(skills), Some(buffs), Some(inventory), Some(bar), Some(flight)) =
        (skills, buffs, inventory, bar, flight)
    else {
        cache.ready = false;
        return;
    };
    if !current || actor != Some(skills.obj_id) {
        cache.ready = false;
        return;
    }
    cache.update(
        owner,
        &skills,
        buffs.of(skills.obj_id),
        &inventory,
        &bar,
        &flight,
        [
            skills.is_changed(),
            buffs.is_changed(),
            inventory.is_changed(),
            bar.is_changed(),
        ],
    );
}

fn source(
    data: &HudData,
    slot_type: i64,
    slot: i64,
    bag: i64,
    action: i64,
    button: &str,
) -> Option<(u32, Option<target::ItemLookSource>)> {
    let index = slot.checked_sub(1).and_then(|n| usize::try_from(n).ok())?;
    let cache = &data.look_conversion.pointer;
    let (template, id) = if slot_type == bag && button == "RightButton" {
        let key = u8::try_from(index).ok()?;
        // Projection may identify a known powder; only prepare_source can
        // establish the full received BAG capture and actual admission.
        let item = enchant::records(data)
            .item(2, key)
            .or_else(|| cache.inventory.bag.get(index)?.as_ref())?;
        (item.template_id, Some(item.id))
    } else if slot_type == action {
        let entry = cache.bar.get(index)?;
        if !matches!(entry.kind, ActionKind::ItemType | ActionKind::ItemId) {
            return None;
        }
        let item = match entry.item_template_id() {
            Some(template) => cache
                .inventory
                .find_template(template)
                .map(|(_, item)| item),
            None => cache
                .inventory
                .find_by_id(entry.id)
                .map(|(_, _, item)| item),
        };
        (
            item.map(|item| item.template_id)
                .or(entry.item_template_id())?,
            item.map(|item| item.id),
        )
    } else {
        return None;
    };
    let skill = match template {
        29880 => 23741,
        29704 => 23457,
        _ => return None,
    };
    Some((
        template,
        id.map(|item_id| target::ItemLookSource {
            item_id,
            template,
            skill,
        }),
    ))
}

fn use_slot(
    lua: &Lua,
    state: &Shared,
    slot_type: i64,
    slot: i64,
    bag: i64,
    action: i64,
    button: &str,
) -> bool {
    let (skill, prepared) = {
        let data = state.borrow();
        let cache = &data.look_conversion.pointer;
        if !cache.interval {
            return false;
        }
        let Some((template, source)) = source(&data, slot_type, slot, bag, action, button) else {
            return false;
        };
        if !cache.ready
            || cache.owner != data.character_id
            || Some(cache.actor) != enchant::actor(&data)
        {
            return true;
        }
        let Some(source) = source else {
            drop(data);
            state.borrow_mut().look_conversion.pointer.failures.push((
                0,
                Fail {
                    result: super::super::usable::code::NEED_REAGENT,
                    value: template,
                },
            ));
            return true;
        };
        (
            source.skill,
            target::prepare_source(
                &data,
                source,
                &cache.skills,
                &cache.buffs,
                &cache.inventory,
                &cache.flight,
            ),
        )
    };
    if let Some(fail) = target::apply_source(lua, state, prepared, None) {
        state
            .borrow_mut()
            .look_conversion
            .pointer
            .failures
            .push((skill, fail));
    }
    true
}

/// Item4 world dispatch only accepts the native stack cursor, not Item_Id.
pub(super) fn world_drop(lua: &Lua, state: &Shared, over_ui: bool) -> mlua::Result<bool> {
    if state.borrow().look_conversion.target.is_none() {
        let data = state.borrow();
        return Ok(data.look_conversion.pointer.claimed[0]);
    }
    {
        let mut data = state.borrow_mut();
        claim(&mut data, MouseButton::Left);
        if data.look_conversion.pointer.delivered_left {
            return Ok(true);
        }
        data.look_conversion.pointer.delivered_left = true;
    }
    if !over_ui {
        let cursor: Table = lua.globals().get("X2Cursor")?;
        let held: Option<Table> = cursor.call_method("GetHeld", ())?;
        if let Some(held) = held
            && held.get::<Option<String>>("kind")?.as_deref() == Some("bag")
            && cursor
                .call_method::<Option<String>>("GetCursorInfo", ())?
                .as_deref()
                == Some("Item")
            && let Some(slot) = held
                .get::<Option<i64>>("slot")?
                .and_then(|slot| slot.checked_sub(1))
                .and_then(|slot| u8::try_from(slot).ok())
        {
            target::choose(lua, state, slot)?;
        }
    }
    Ok(true)
}

fn released(lua: &Lua, state: &Shared, button: MouseButton) -> mlua::Result<()> {
    if state.borrow().look_conversion.target.is_some() {
        claim(&mut state.borrow_mut(), button);
        // GUI DropOnNothing already delivered its candidate; no duplicate.
        // Use the actual GUI pick after resize/OnScale, not a pre-frame hit.
        // CursorNone completes release checking without held-world delivery.
        if button == MouseButton::Left
            && state.borrow().look_conversion.pointer.cursor_present
            && aa_ui::lua::ui(lua).hover.is_none()
        {
            world_drop(lua, state, false)?;
        }
        let cancel = {
            let data = state.borrow();
            data.look_conversion.target.is_some() && !data.look_conversion.fresh
        };
        if cancel {
            target::cancel_target(lua, state);
        }
    }
    Ok(())
}

pub(super) fn install(lua: &Lua, state: &Shared) -> mlua::Result<()> {
    state.borrow_mut().look_conversion.pointer = Default::default();
    let aaui: Table = lua.globals().get("AAUI")?;
    let original: Option<Function> = aaui.get("UseSlot")?;
    let bag = lua.globals().get::<i64>("ISLOT_BAG")?;
    let action = lua.globals().get::<i64>("ISLOT_ACTION")?;
    let shared = state.clone();
    aaui.set(
        "UseSlot",
        lua.create_function(move |lua, args: MultiValue| {
            let intercepted = match (args.front(), args.get(1), args.get(2)) {
                (
                    Some(Value::Integer(kind)),
                    Some(Value::Integer(slot)),
                    Some(Value::String(button)),
                ) => use_slot(
                    lua,
                    &shared,
                    *kind,
                    *slot,
                    bag,
                    action,
                    button.to_str()?.as_ref(),
                ),
                (Some(kind), Some(slot), Some(Value::String(button))) => {
                    let number = |v: &Value| match v {
                        Value::Integer(n) => Some(*n),
                        Value::Number(n) if n.is_finite() && n.fract() == 0.0 => Some(*n as i64),
                        _ => None,
                    };
                    number(kind).zip(number(slot)).is_some_and(|(kind, slot)| {
                        use_slot(
                            lua,
                            &shared,
                            kind,
                            slot,
                            bag,
                            action,
                            button
                                .to_str()
                                .map(|b| b.to_string())
                                .unwrap_or_default()
                                .as_str(),
                        )
                    })
                }
                _ => false,
            };
            if !intercepted && let Some(original) = &original {
                return original.call::<MultiValue>(args);
            }
            Ok(MultiValue::new())
        })?,
    )?;
    let shared = state.clone();
    lua.globals().get::<Table>("HUD")?.set(
        "look_world_drop",
        lua.create_function(move |lua, over: bool| world_drop(lua, &shared, over))?,
    )?;
    let shared = state.clone();
    aa_ui::input::set_release_callback(lua, move |lua, button| released(lua, &shared, button))
}

fn finish(
    bridge: NonSend<HudBridge>,
    rt: Option<NonSend<UiRuntime>>,
    aiming: Option<ResMut<crate::ingame::ground_target::Aiming>>,
    mut failed: MessageWriter<super::super::UseFailed>,
) {
    let mut data = bridge.0.borrow_mut();
    if data.look_conversion.cancel_aim_pending {
        if let Some(mut aiming) = aiming {
            aiming.cancel();
        }
        data.look_conversion.cancel_aim_pending = false;
    }
    let active_frame = data.look_conversion.pointer.interval;
    data.look_conversion.pointer.interval = false;
    if active_frame && rt.is_some() && !data.disconnected && enchant::actor(&data).is_some() {
        data.look_conversion.fresh = false;
    }
    for (skill, fail) in data.look_conversion.pointer.failures.drain(..) {
        failed.write(super::super::UseFailed {
            skill,
            result: fail.result,
            value: fail.value,
            server: false,
        });
    }
}

pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        PreUpdate,
        begin
            .after(super::super::input::pointer_over_ui)
            .before(super::super::drag::hold_world)
            .before(crate::ingame::ground_target::AimInput)
            .run_if(in_state(crate::Screen::InGame)),
    )
    .add_systems(
        Update,
        prepare
            .in_set(PointerPrepared)
            .after(crate::ingame::ground_target::ItemUseRouting)
            .after(super::super::sync_hud)
            .after(super::super::drag::scripted_drags)
            .after(super::super::session::scripted_session)
            .before(aa_ui::UiFrame)
            .run_if(super::super::hud_up)
            .run_if(in_state(crate::Screen::InGame)),
    )
    .add_systems(
        Update,
        finish
            .in_set(PointerFinished)
            .after(aa_ui::UiFrame)
            .run_if(in_state(crate::Screen::InGame)),
    )
    .add_systems(
        OnExit(crate::Screen::InGame),
        |bridge: NonSend<HudBridge>| {
            // ground_target::leave withdraws the actual resource on this edge.
            let mut data = bridge.0.borrow_mut();
            data.look_conversion.cancel_aim_pending = false;
            data.look_conversion.pointer.interval = false;
            data.look_conversion.pointer.ready = false;
        },
    );
}

#[cfg(test)]
mod tests;
