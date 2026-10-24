use super::{
    super::{
        api::{HudData, Request, Shared},
        enchant, tips,
    },
    catalog,
};
use aa_net::{
    Event, NativeItemRowOutput,
    item_effect::CapturedBagItem,
    items::ItemTask,
    look_conversion::ItemLookRequest,
    model::{Item, ItemDetail},
};
use aa_ui::UiRuntime;
use mlua::{Lua, Table, Value};

#[derive(Default)]
pub(in crate::hud) struct View {
    pub active: bool,
    pub slots: [Option<u8>; 2],
    owner: Option<u32>,
    money: Option<i64>,
    config: aa_net::initial_config::GameConfig,
    notices: Vec<Item>,
    pub(super) target: Option<super::target::Target>,
    pub(super) task: Option<super::target::Task>,
    pub(super) next_task: u64,
    pub(super) fresh: bool,
    pub(super) cancel_aim_pending: bool,
    pub(super) pointer: super::pointer::PointerUse,
}

pub(super) fn item(data: &HudData, slot: u8) -> Option<&Item> {
    enchant::records(data)
        .item(2, slot)
        .filter(|item| item.template_id != 0)
}
pub(super) fn selected(data: &HudData, index: usize) -> Option<&Item> {
    item(data, data.look_conversion.slots[index]?)
}
fn combination(data: &HudData, base: u8, image: u8) -> Result<u32, i16> {
    data.game.get().ok_or(181i16)?.look_conversion.pair(
        item(data, base).ok_or(181i16)?,
        item(data, image).ok_or(181i16)?,
    )
}
fn pair(data: &HudData, base: u8, image: u8) -> Result<u32, i16> {
    let (base_item, image_item) = (
        item(data, base).ok_or(181i16)?,
        item(data, image).ok_or(181i16)?,
    );
    if base == image || catalog::effective_look(base_item) == catalog::effective_look(image_item) {
        return Err(0);
    }
    combination(data, base, image)
}
pub(super) fn eligible(data: &HudData, slot: u8) -> bool {
    let Some(candidate) = item(data, slot) else {
        return false;
    };
    let Some(game) = data.game.get() else {
        return false;
    };
    match data.look_conversion.slots {
        [None, None] => game.look_conversion.base_eligible(candidate),
        [Some(base), None] => pair(data, base, slot).is_ok(),
        [None, Some(image)] => pair(data, slot, image).is_ok(),
        _ => false,
    }
}
pub(super) fn select(data: &mut HudData, index: usize, slot: u8) -> Result<(), i16> {
    if let Some(other) = data.look_conversion.slots[1 - index] {
        if index == 0 {
            pair(data, slot, other)?;
        } else {
            pair(data, other, slot)?;
        }
    } else {
        let item = item(data, slot).ok_or(181i16)?;
        let game = data.game.get().ok_or(181i16)?;
        if index == 0 {
            game.look_conversion.base_group(item).ok_or(738i16)?;
        } else {
            game.look_conversion.image_group(item).ok_or(739i16)?;
        }
    }
    // Native setters store physical keys, not an optimistic replacement Item.
    if let Some(old) = data.look_conversion.slots[index] {
        enchant::lock_bag(data, old, false);
    }
    data.look_conversion.slots[index] = Some(slot);
    enchant::lock_bag(data, slot, true);
    Ok(())
}
pub(super) fn update(lua: &Lua) {
    aa_ui::lua::fire_ui_event(lua, "UPDATE_ITEM_LOOK_CONVERT_MODE", Vec::new());
}
pub(super) fn clear(lua: &Lua, state: &Shared, index: usize) {
    {
        let mut data = state.borrow_mut();
        if let Some(slot) = data.look_conversion.slots[index].take() {
            enchant::lock_bag(&mut data, slot, false);
        }
    }
    update(lua); // The binding emits this even for an already empty key.
}
pub(super) fn switch(lua: &Lua, state: &Shared, active: bool) {
    if active {
        super::target::cancel_target(lua, state);
    }
    {
        let mut data = state.borrow_mut();
        if !active && !data.look_conversion.active {
            return;
        }
        data.look_conversion.active = active;
        if !active {
            for slot in data.look_conversion.slots.into_iter().flatten() {
                enchant::lock_bag(&mut data, slot, false);
            }
            data.look_conversion.slots = [None; 2];
        }
    }
    aa_ui::lua::fire_ui_event(
        lua,
        if active {
            "ENTER_ITEM_LOOK_CONVERT_MODE"
        } else {
            "LEAVE_ITEM_LOOK_CONVERT_MODE"
        },
        Vec::new(),
    );
}

pub(super) fn tooltip(lua: &Lua, data: &HudData, item: &Item) -> mlua::Result<Option<Table>> {
    let Some(game) = data.game.get() else {
        return Ok(None);
    };
    let instance = match &item.detail {
        ItemDetail::Equip(detail) => tips::Instance {
            durability: Some(detail.durability),
            temper: (detail.temper_physical, detail.temper_magical),
            flags: Some(item.flags),
        },
        _ => tips::Instance {
            flags: Some(item.flags),
            ..Default::default()
        },
    };
    let out = tips::item_tip_with(
        lua,
        game,
        &data.tips,
        item.template_id,
        Some(i64::from(super::super::store::ui_grade(item.grade))),
        Some(i64::from(item.count)),
        instance,
        &tips::ItemExtra::of_item(item),
    )?;
    if let Some(out) = &out {
        let appearance = catalog::effective_look(item);
        out.raw_set("itemGrade", item.grade)?;
        out.raw_set("lookType", appearance)?;
        out.raw_set("lookChanged", appearance != item.template_id)?;
    }
    Ok(out)
}
pub(super) fn info(lua: &Lua, state: &Shared, index: usize) -> mlua::Result<Option<Table>> {
    let data = state.borrow();
    let Some(item) = selected(&data, index) else {
        return Ok(None);
    };
    tooltip(lua, &data, item)
}
pub(super) fn preview(lua: &Lua, state: &Shared) -> mlua::Result<Option<Table>> {
    let data = state.borrow();
    let (Some(base), Some(image)) = (selected(&data, 0), selected(&data, 1)) else {
        return Ok(None);
    };
    let Some(out) = tooltip(lua, &data, base)? else {
        return Ok(None);
    };
    out.raw_set("lookType", catalog::effective_look(image))?;
    out.raw_set("lookChanged", true)?;
    out.raw_set("useAsSkin", false)?;
    // Native preview retains base identity and grade; it does not mutate records.
    Ok(Some(out))
}

pub(super) struct Plan {
    pub gold: i32,
    pub money: bool,
    pub items: Vec<(u32, i32, i32)>,
}
impl Plan {
    pub fn enough_items(&self) -> bool {
        self.items
            .iter()
            .all(|&(_, need, has)| (has as u32) >= (need as u32))
    }
}
pub(super) fn quote(data: &HudData) -> Option<Plan> {
    let [Some(base), Some(image)] = data.look_conversion.slots else {
        return None;
    };
    let group = combination(data, base, image).ok()?;
    let recipe = data.game.get()?.look_conversion.conversion(group)?;
    let records = enchant::records(data);
    if !records.complete_bag() {
        return None;
    }
    let items = recipe
        .required
        .iter()
        .map(|required| {
            let count = records
                .bag_items()
                .filter(|(_, item)| item.template_id == required.item)
                .fold(0i32, |sum, (_, item)| sum.wrapping_add(item.count));
            (required.item, required.count, count)
        })
        .collect();
    Some(Plan {
        gold: recipe.gold,
        money: data.look_conversion.money? >= i64::from(recipe.gold),
        items,
    })
}
pub(super) fn plan(lua: &Lua, state: &Shared) -> mlua::Result<Option<Table>> {
    let Some(plan) = quote(&state.borrow()) else {
        return Ok(None);
    };
    let out = lua.create_table()?;
    out.raw_set("money", plan.gold.to_string())?;
    out.raw_set("enoughMoney", plan.money)?;
    out.raw_set("enoughItems", plan.enough_items())?;
    let items = lua.create_table()?;
    for (index, (id, need, has)) in plan.items.into_iter().enumerate() {
        let row = lua.create_table()?;
        row.raw_set("itemType", id)?;
        row.raw_set("need", need)?;
        row.raw_set("has", has)?;
        items.raw_set(index + 1, row)?;
    }
    out.raw_set("items", items)?;
    Ok(Some(out))
}
pub(super) fn prepare(data: &HudData) -> Result<(u32, ItemLookRequest), i16> {
    let [Some(base), Some(image)] = data.look_conversion.slots else {
        return Err(181);
    };
    let records = enchant::records(data);
    let owner = records
        .owner()
        .filter(|&owner| {
            owner != 0
                && Some(owner) == data.look_conversion.owner
                && owner == data.character_id
                && !data.disconnected
        })
        .ok_or(181i16)?;
    for slot in [base, image] {
        let item = item(data, slot).ok_or(181i16)?;
        if item.flags & 4 != 0 {
            match data
                .look_conversion
                .config
                .received
                .as_ref()
                .and_then(|c| c.feature_bytes.get(5))
            {
                Some(flags) if flags & 0x20 == 0 => {}
                Some(_) => return Err(664),
                None => return Err(1), // missing provider is not a secured result
            }
        }
    }
    combination(data, base, image)?;
    let plan = quote(data).ok_or(737i16)?;
    if !plan.enough_items() {
        return Err(138);
    }
    if !plan.money {
        return Err(26);
    }
    let request = ItemLookRequest::convert(
        CapturedBagItem {
            slot: base,
            item: item(data, base).ok_or(181i16)?.clone(),
        },
        CapturedBagItem {
            slot: image,
            item: item(data, image).ok_or(181i16)?.clone(),
        },
    )
    .map_err(|_| 181i16)?;
    Ok((owner, request))
}
pub(super) fn submit(lua: &Lua, state: &Shared) -> mlua::Result<()> {
    let prepared = prepare(&state.borrow());
    match prepared {
        Ok((character_id, request)) => state.borrow_mut().requests.push(Request::ItemLook {
            character_id,
            request,
        }),
        Err(code) => super::super::enchant::selection_error(lua, code)?,
    }
    Ok(())
}

pub(in crate::hud) fn observe(rt: Option<&UiRuntime>, state: &Shared, event: &Event) {
    let retire = super::super::enchant::retires_items(event);
    if retire {
        super::target::retire(rt.map(|r| &r.lua), state);
    }
    {
        let mut data = state.borrow_mut();
        let view = &mut data.look_conversion;
        view.config.apply(event);
        match event {
            Event::CharacterSelected(selected) => {
                view.owner = Some(selected.character.id);
                view.money = Some(selected.character.money);
            }
            Event::PlayerSpawn(spawn) => {
                view.owner = Some(spawn.character().id);
                view.money = Some(spawn.character().money);
            }
            Event::GameDecodeFailed { .. }
            | Event::UnhandledGamePacket {
                level: 4, len: 1.., ..
            } => view.money = None,
            Event::ItemsChanged(changes) if changes.truncated => view.money = None,
            _ => {}
        }
        if retire {
            view.notices.clear();
            view.owner = None;
            view.money = None;
        }
    }
    if retire {
        if let Some(rt) = rt {
            switch(&rt.lua, state, false);
        } else {
            let mut data = state.borrow_mut();
            data.look_conversion.active = false;
            data.look_conversion.slots = [None; 2];
        }
    }
    if let Event::ItemLookRejected { message, .. } = event
        && let Some(rt) = rt
    {
        rt.fire_event("SYSMSG", message.as_str());
    }
}
pub(in crate::hud) fn row(
    rt: Option<&UiRuntime>,
    state: &Shared,
    reason: u8,
    task: &ItemTask,
    output: Option<&NativeItemRowOutput>,
) {
    if let ItemTask::Money(delta) = task
        && let Some(money) = &mut state.borrow_mut().look_conversion.money
    {
        *money = money.wrapping_add(i64::from(*delta));
    }
    if reason == 111
        && matches!(task, ItemTask::Update { .. })
        && let Some(item) = output
            .filter(|out| out.admitted)
            .and_then(|out| out.item.as_ref())
            .filter(|item| item.template_id != 0)
    {
        if let Some(rt) = rt {
            if let Err(error) = emit(rt, state, item) {
                bevy::log::warn!("look conversion: callback: {error}");
            }
        } else {
            state
                .borrow_mut()
                .look_conversion
                .notices
                .push(item.clone());
        }
    }
}
fn emit(rt: &UiRuntime, state: &Shared, item: &Item) -> mlua::Result<()> {
    let link = super::super::itemlinks::link(item);
    let info = tooltip(&rt.lua, &state.borrow(), item)?;
    if let Some(link) = link {
        rt.fire_event("ITEM_LOOK_CONVERTED", link);
    }
    if let Some(info) = info {
        aa_ui::lua::fire_ui_event(
            &rt.lua,
            "ITEM_LOOK_CONVERTED_EFFECT",
            vec![Value::Table(info)],
        );
    }
    Ok(())
}
pub(in crate::hud) fn replay(rt: &UiRuntime, state: &Shared) {
    let pending = std::mem::take(&mut state.borrow_mut().look_conversion.notices);
    for item in pending {
        if let Err(error) = emit(rt, state, &item) {
            bevy::log::warn!("look conversion: replay: {error}");
        }
    }
}
