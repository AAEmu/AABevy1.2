//! Selected native BAG keys, over receipt-ordered native records. No operation planner.
use super::{
    Mode, Shared,
    catalog::{self, Tables},
    records::Records,
};
use aa_net::{
    Event,
    initial_config::GameConfig,
    item_effect::{CapturedBagItem, ItemEffectMode, ItemEffectRequest},
    model::{Item, ItemDetail},
    state::{Inventory, Skills},
};
use mlua::{Lua, MultiValue, ObjectLike, Table, Value};
use std::collections::HashSet;

#[derive(Clone)]
pub(super) struct State {
    pub slots: [Option<u8>; 3], // target, source, support; physical zero-based BAG slots
    pub(super) records: Records,
    config: GameConfig,
    external_pending: HashSet<u8>,
    selected_pending: HashSet<u8>,
    owner: Option<u32>,
    actor: u32,
    level: u8,
    active: bool,
    auto_use_aa_point: Option<bool>,
}

impl Default for State {
    fn default() -> Self {
        let mut records = Records::default();
        if let Some(catalog) = super::super::data::item_recharge_catalog() {
            records.set_recharge_catalog(catalog);
        }
        Self {
            slots: [None; 3],
            records,
            config: GameConfig::default(),
            external_pending: HashSet::new(),
            selected_pending: HashSet::new(),
            owner: None,
            actor: 0,
            level: 0,
            active: false,
            auto_use_aa_point: None,
        }
    }
}

pub(super) struct Prepared {
    pub character_id: u32,
    pub request: ItemEffectRequest,
    pub input_ready: bool,
}
impl State {
    pub fn clear(&mut self) {
        self.clear_from(0);
    }
    fn clear_from(&mut self, index: usize) {
        for slot in &mut self.slots[index..] {
            if let Some(slot) = slot.take() {
                self.selected_pending.remove(&slot);
            }
        }
    }
    pub fn release_pending(&mut self, container: u8, slot: u8) {
        if container == 2 {
            self.selected_pending.remove(&slot);
            self.external_pending.remove(&slot);
        }
    }
    pub fn presence(&self) -> bool {
        self.slots[0].is_some() && self.slots[1].is_some()
    }
    pub fn sync_pending(&mut self, inventory: &Inventory) {
        self.external_pending = inventory
            .pending_item_slot_keys()
            .filter_map(|key| {
                let bytes = key.to_le_bytes();
                (bytes[1] == 2).then_some(bytes[3])
            })
            .collect();
    }
    fn locked(&self, slot: u8) -> bool {
        self.selected_pending.contains(&slot) || self.external_pending.contains(&slot)
    }
    /// Snapshot/lifecycle observations, before individual item-row callbacks.
    /// True only when a qualified detail receipt releases its physical pending key.
    pub fn observe(&mut self, event: &Event) -> bool {
        self.config.apply(event);
        match event {
            Event::CharacterSelected(selected) => {
                let config = std::mem::take(&mut self.config);
                *self = Self {
                    config,
                    owner: Some(selected.character.id),
                    auto_use_aa_point: Some(selected.character.auto_use_aa_point),
                    ..Default::default()
                };
            }
            Event::PlayerSpawn(spawn) => {
                if self.owner != Some(spawn.character().id) {
                    let config = std::mem::take(&mut self.config);
                    *self = Self {
                        config,
                        owner: Some(spawn.character().id),
                        auto_use_aa_point: Some(spawn.character().auto_use_aa_point),
                        ..Default::default()
                    };
                }
                self.actor = spawn.obj_id;
                self.level = spawn.character().level;
                self.active = true;
            }
            Event::LevelChanged { obj_id, level } if *obj_id == self.actor => self.level = *level,
            Event::InventoryInit { .. } => {
                self.clear();
            }
            Event::InventoryChunk(chunk)
                if chunk.truncated
                    || chunk.chunks > 5
                    || chunk.items.len() != usize::from(chunk.chunks) * 10
                    || (matches!(chunk.container, 2 | 3) && chunk.first_slot() + usize::from(chunk.chunks) * 10 > 130) =>
            {
                self.clear();
            }
            Event::GameDecodeFailed { .. }
            | Event::UnhandledGamePacket { level: 4, len: 1.., .. }
            | Event::UnhandledGamePacket {
                opcode: 0x93, len: 1.., ..
            } => {
                self.clear();
            }
            Event::ItemsChanged(tasks) if tasks.truncated => {
                self.clear();
            }
            Event::UnitsRemoved(ids) if self.actor != 0 && ids.contains(&self.actor) => {
                self.active = false;
                self.clear();
            }
            event if super::retires(event) => {
                let config = std::mem::take(&mut self.config);
                *self = Self {
                    config,
                    ..Default::default()
                };
            }
            _ => {}
        }
        self.records.observe(event);
        let released_detail = match event {
            Event::ItemDetailUpdated(update) => self.records.apply_detail(update),
            _ => false,
        };
        released_detail
    }
    pub fn apply_row(&mut self, task: &aa_net::items::ItemTask) -> Option<super::records::RowOutput> {
        if let aa_net::items::ItemTask::AutoUseAaPoint(raw) = task {
            self.auto_use_aa_point = Some(*raw == 1);
        }
        self.records.apply_row(task)
    }
    /// Standalone fixture convenience; production dispatches between each row.
    pub fn receive(&mut self, event: &Event) -> bool {
        let released_detail = self.observe(event);
        if let Event::ItemsChanged(changes) = event {
            if !changes.truncated {
                for task in &changes.tasks {
                    self.apply_row(task);
                }
                self.records.finish(changes);
            }
        }
        released_detail
    }
    fn complete(&self) -> bool {
        // Original BAG storage has130 records. Refuse a whole-BAG quote whose
        // enabled header would scan past it; never reinterpret unsafe native data.
        self.records.complete_bag()
    }
    fn item(&self, slot: u8) -> Option<&Item> {
        if !self.active || !self.complete() {
            return None;
        }
        self.records.item(2, slot).filter(|item| item.template_id != 0)
    }
    fn selected(&self, index: usize) -> Option<&Item> {
        self.item(self.slots[index]?)
    }
    fn common(&self, skills: &Skills, slot: u8) -> Result<&Item, i16> {
        // Explicit conservative application policy: refuse any observed local cast.
        // Skills::casting is not the source-complete native Game cast pair/Timeline.
        if !self.active || skills.casting_of(self.actor).is_some() {
            return Err(1);
        }
        if self.locked(slot) {
            return Err(0xb5);
        }
        let item = self.item(slot).ok_or(0xb5i16)?;
        if item.flags & 4 != 0 {
            match self.config.received.as_ref().and_then(|c| c.feature_bytes.get(5)) {
                Some(flags) if flags & 0x20 == 0 => {}
                Some(_) => return Err(0x298),
                None => return Err(1), // unknown feature provider, never assumed disabled
            }
        }
        Ok(item)
    }
    fn target(&self, tables: &Tables, skills: &Skills, mode: Mode, slot: u8) -> Result<(), i16> {
        let item = self.common(skills, slot)?;
        let desc = tables.items.get(&item.template_id).ok_or(0xb5i16)?;
        let valid = match mode {
            Mode::Grade => desc.grade_enchantable,
            Mode::Gem => matches!(item.detail, ItemDetail::Equip(_)),
            Mode::Unsocket => matches!(item.detail, ItemDetail::Equip(_)) && catalog::gem_count(item) > 0,
            Mode::Socket => {
                matches!(item.detail, ItemDetail::Equip(_))
                    && tables.socket_minimum.is_some_and(|n| i32::from(desc.level) >= n)
                    && tables
                        .slot
                        .get(&item.template_id)
                        .and_then(|s| tables.socket_count.get(&(*s, item.grade)))
                        .is_some_and(|&n| n != 0)
            }
            Mode::Off => return Err(1),
        };
        if valid {
            Ok(())
        } else if mode == Mode::Unsocket && matches!(item.detail, ItemDetail::Equip(_)) {
            Err(0x2e7)
        } else {
            Err(0xb5)
        }
    }
    fn source(&self, tables: &Tables, skills: &Skills, mode: Mode, slot: u8) -> Result<(), i16> {
        let item = self.common(skills, slot)?;
        let desc = tables.items.get(&item.template_id).ok_or(1i16)?;
        if matches!(desc.impl_id, 15 | 21) && !tables.modifier_known(item.template_id) {
            return Err(1);
        }
        //394e1880 selector -1 returns static use_skill directly outside its
        // equipment/model/special-skill exceptions, including actual impl0/15/21.
        // The latter exception providers and nonzero actability remain unknown.
        if matches!(desc.impl_id, 1 | 2 | 22 | 24 | 11 | 14)
            || i32::from(self.level) < desc.requirement
            || (desc.limit != 0 && i32::from(self.level) > desc.limit)
            || (desc.actability_group != 0 && desc.actability_requirement != 0)
        {
            return Err(1);
        }
        if match mode {
            Mode::Grade => matches!(desc.skill, 22520 | 22522 | 22523 | 22524 | 24290 | 24998 | 24999 | 25503),
            Mode::Socket => desc.skill == 23728,
            Mode::Unsocket => desc.skill == 23729,
            Mode::Gem => desc.skill == 11285,
            Mode::Off => false,
        } {
            Ok(())
        } else {
            Err(1)
        }
    }
    fn support(&self, tables: &Tables, skills: &Skills, mode: Mode, slot: u8) -> Result<(), i16> {
        if mode != Mode::Grade {
            return Err(1);
        }
        let item = self.common(skills, slot)?;
        let (Some(target), Some(source)) = (self.selected(0), self.selected(1)) else {
            return Err(1);
        };
        if tables.support_allows(target, source, item) {
            Ok(())
        } else {
            Err(1)
        }
    }
    fn validate(&self, tables: &Tables, skills: &Skills, mode: Mode, index: usize, slot: u8) -> Result<(), i16> {
        if self.slots[index].is_some() {
            return Err(1);
        }
        match index {
            0 => {
                self.target(tables, skills, mode, slot)?;
                if self.slots[1].is_some() {
                    if !tables.pair_allows(mode, self.item(slot).ok_or(1i16)?, self.selected(1).ok_or(1i16)?) {
                        return Err(1);
                    }
                }
            }
            1 => {
                self.source(tables, skills, mode, slot)?;
                if self.slots[0].is_some() {
                    if !tables.pair_allows(mode, self.selected(0).ok_or(1i16)?, self.item(slot).ok_or(1i16)?) {
                        return Err(1);
                    }
                }
            }
            _ => self.support(tables, skills, mode, slot)?,
        }
        Ok(())
    }
    fn set(&mut self, tables: &Tables, skills: &Skills, mode: Mode, index: usize, slot: u8) -> Result<(), i16> {
        self.validate(tables, skills, mode, index, slot)?;
        if index < 2 {
            self.clear_from(2);
        }
        self.slots[index] = Some(slot);
        self.selected_pending.insert(slot);
        Ok(())
    }
    fn enchantable(&self, tables: &Tables, skills: &Skills, mode: Mode, slot: u8) -> bool {
        match (self.slots[0], self.slots[1], self.slots[2]) {
            (None, None, _) => self.target(tables, skills, mode, slot).is_ok() || self.source(tables, skills, mode, slot).is_ok(),
            (None, Some(_), _) => self.validate(tables, skills, mode, 0, slot).is_ok(),
            (Some(_), None, _) => self.validate(tables, skills, mode, 1, slot).is_ok(),
            (Some(_), Some(_), None) => self.validate(tables, skills, mode, 2, slot).is_ok(),
            _ => false,
        }
    }
    /// Revalidate the actual selected physical records, without treating this
    /// view's own selection locks as a competing operation. This is a stronger
    /// app preflight, not the protected ordinary native skill planner.
    pub fn prepare(&self, tables: &Tables, skills: &Skills, mode: Mode) -> Result<Prepared, i16> {
        if !self.presence() || !self.active || !self.complete() {
            return Err(1);
        }
        let mut checked = self.clone();
        checked.clear();
        for (index, slot) in self.slots.iter().enumerate() {
            if let Some(slot) = slot {
                checked.set(tables, skills, mode, index, *slot)?;
            }
        }
        let capture = |index: usize| -> Result<CapturedBagItem, i16> {
            let slot = checked.slots[index].ok_or(1i16)?;
            let item = checked.item(slot).ok_or(1i16)?;
            // Stronger app policy: request only existing positive stacks. Native
            // validators can still expose valid-template nonpositive records.
            if item.id == 0 || item.count <= 0 {
                return Err(0xb5);
            }
            Ok(CapturedBagItem { slot, item: item.clone() })
        };
        let source = capture(1)?;
        let target = capture(0)?;
        let support = self.slots[2].map(|_| capture(2)).transpose()?;
        let descriptor = tables.items.get(&source.item.template_id).ok_or(1i16)?;
        let transport_mode = match mode {
            Mode::Grade => {
                // Require the same real providers used by the target quote.
                self.config.received.as_ref().and_then(|c| c.feature_bytes.get(5)).ok_or(1i16)?;
                tables.cost(&target.item).filter(|cost| *cost >= 0).ok_or(1i16)?;
                ItemEffectMode::Grade {
                    auto_use_aa_point: self.auto_use_aa_point.ok_or(1i16)?,
                }
            }
            Mode::Socket | Mode::Unsocket => ItemEffectMode::Socket,
            Mode::Gem => ItemEffectMode::Gem,
            Mode::Off => return Err(1),
        };
        let request = ItemEffectRequest::new(transport_mode, descriptor.skill, source, target, support).map_err(|_| 1i16)?;
        Ok(Prepared {
            character_id: self.owner.filter(|id| *id != 0).ok_or(1i16)?,
            request,
            input_ready: !descriptor.reagent,
        })
    }
    ///39046020 after the caller has reset its active/captured operation. Restore
    /// physical slots from current records; do not settle or resend anything.
    pub fn restore(&mut self, tables: &Tables, skills: &Skills, mode: Mode, request: &ItemEffectRequest, result_seen: bool) -> bool {
        self.clear();
        // Stronger whole-BAG provider guard, also used by ordinary selection.
        if !self.complete() || self.records.item(2, request.target().slot).is_none_or(|item| item.template_id == 0) {
            return false;
        }
        let _ = self.set(tables, skills, mode, 0, request.target().slot);
        if mode == Mode::Gem && result_seen {
            return false;
        }
        for (index, captured) in [(1, Some(request.source())), (2, request.support())] {
            let Some(captured) = captured else { continue };
            if self.records.item(2, captured.slot).is_some_and(|item| item.template_id != 0) {
                // A valid old slot is tried even if its item/template changed.
                // A rejected setter does not permit a same-template fallback.
                let _ = self.set(tables, skills, mode, index, captured.slot);
                continue;
            }
            let fallback = self.records.bag_items().find_map(|(slot, item)| {
                (item.template_id == captured.item.template_id && item.grade == captured.item.grade && !self.locked(slot)).then_some(slot)
            });
            if let Some(slot) = fallback {
                let _ = self.set(tables, skills, mode, index, slot);
            }
        }
        self.presence()
    }
}
fn number(value: Option<&Value>) -> Option<i32> {
    let value = match value? {
        Value::Integer(n) => *n as f32,
        Value::Number(n) => *n as f32,
        _ => return None,
    };
    Some(catalog::cvtt(f64::from(value)))
}
pub(super) fn update(lua: &Lua, presence: bool) {
    update_working(lua, presence, false);
}
pub(super) fn update_working(lua: &Lua, presence: bool, working: bool) {
    aa_ui::lua::fire_ui_event(
        lua,
        "UPDATE_ENCHANT_ITEM_MODE",
        vec![Value::Boolean(presence), Value::Boolean(working)],
    );
}
fn set(data: &mut super::super::api::HudData, index: usize, slot: u8) -> Result<(), i16> {
    let mode = data.enchant.mode;
    let tables = &data.game.get().ok_or(1i16)?.enchant;
    data.enchant.selection.set(tables, &data.cooldowns, mode, index, slot)
}
pub(super) fn error(lua: &Lua, code: i16) -> mlua::Result<()> {
    if let Some(key) = super::super::alerts::error_name(code) {
        let locale: Table = lua.globals().get("X2Locale")?;
        let group: Value = lua.globals().get("ERROR_MSG")?;
        let text: Value = locale.call_method("LocalizeUiText", (group, key))?;
        aa_ui::lua::fire_ui_event(lua, "SYSMSG", vec![text]);
    }
    Ok(())
}
fn info(lua: &Lua, state: &Shared, index: usize) -> mlua::Result<MultiValue> {
    let data = state.borrow();
    let view = &data.enchant.selection;
    let (Some(game), Some(item)) = (data.game.get(), view.selected(index)) else {
        return Ok(MultiValue::new());
    };
    let currency = if index == 0 && data.enchant.mode == Mode::Grade {
        let Some(flags) = view.config.received.as_ref().and_then(|c| c.feature_bytes.get(5)) else {
            return Ok(MultiValue::new());
        };
        let Some(grade) = game.enchant.grades.get(&item.grade) else {
            return Ok(MultiValue::new());
        };
        Some(if flags & 0x10 != 0 { grade.currency } else { 0 })
    } else {
        None
    };
    let instance = match &item.detail {
        ItemDetail::Equip(e) => super::super::tips::Instance {
            durability: Some(e.durability),
            temper: (e.temper_physical, e.temper_magical),
            flags: Some(item.flags),
        },
        _ => super::super::tips::Instance {
            flags: Some(item.flags),
            ..Default::default()
        },
    };
    let Some(table) = super::super::tips::item_tip(
        lua,
        game,
        &data.tips,
        item.template_id,
        Some(i64::from(super::super::store::ui_grade(item.grade))),
        Some(i64::from(item.count)),
        instance,
    )?
    else {
        return Ok(MultiValue::new());
    };
    table.raw_set("itemGrade", f64::from(item.grade))?;
    //39422d42..e57: native slot/raw-grade/level lookup; raw socket IDs are
    // indexed through maxSocket including zero entries, independent of prefix count.
    if let Some(maximum) = game
        .enchant
        .slot
        .get(&item.template_id)
        .and_then(|slot| game.enchant.socket_count.get(&(*slot, item.grade)))
        .copied()
        .filter(|&n| n != 0)
        .filter(|_| {
            game.enchant
                .items
                .get(&item.template_id)
                .is_some_and(|d| game.enchant.socket_minimum.is_some_and(|min| i32::from(d.level) >= min))
        })
    {
        let sockets = lua.create_table()?;
        sockets.raw_set("maxSocket", f64::from(maximum))?;
        if let ItemDetail::Equip(detail) = &item.detail {
            let ids = lua.create_table()?;
            for (i, id) in detail.gem_ids.iter().take(usize::from(maximum)).enumerate() {
                ids.raw_set(i + 1, f64::from(*id as f32))?;
            }
            sockets.raw_set("socketItem", ids)?;
        }
        table.raw_set("socketInfo", sockets)?;
    }
    let total = view
        .records
        .bag_items()
        .map(|(_, item)| item)
        .filter(|i| i.template_id == item.template_id && i.grade == item.grade)
        .fold(0i32, |sum, i| sum.wrapping_add(i.count));
    table.raw_set("total", f64::from(total as f32))?;
    if let Some(currency) = currency {
        let Some(cost) = game.enchant.cost(item) else {
            return Ok(MultiValue::new());
        };
        table.raw_set("grade_enchant_cost", f64::from(cost as f32))?;
        table.raw_set("grade_enchant_currency", f64::from(currency as f32))?;
    }
    Ok(MultiValue::from_iter([Value::Table(table)]))
}
pub(super) fn install(lua: &Lua, state: &Shared) -> mlua::Result<()> {
    let api: Table = lua.globals().get("X2ItemEnchant")?;
    let item: Table = lua.globals().get("X2Item")?;
    // Preserve the existing shop lock provider while exposing selected/native
    // pending BAG keys to ordinary drag input. This does not settle an item task.
    if let Ok(bag) = lua.globals().get::<Table>("X2Bag") {
        let original = bag.get::<Option<mlua::Function>>("IsLocked")?;
        let s = state.clone();
        bag.raw_set(
            "IsLocked",
            lua.create_function(move |_, args: MultiValue| {
                let locked = number(args.get(1))
                    .map(|n| n.wrapping_sub(1))
                    .filter(|&n| n >= 0 && n <= 255)
                    .is_some_and(|n| s.borrow().enchant.selection.locked(n as u8));
                if locked {
                    Ok(true)
                } else {
                    original.as_ref().map_or(Ok(false), |f| f.call::<bool>(args))
                }
            })?,
        )?;
    }
    let s = state.clone();
    item.raw_set(
        "IsEnchantable",
        lua.create_function(move |lua, args: MultiValue| {
            let Some(index) = number(args.get(1)).map(|v| v.wrapping_sub(1)) else {
                return Ok(MultiValue::new());
            };
            if index < 0 {
                aa_ui::lua::fire_ui_event(
                    lua,
                    "UI_ERROR_MESSAGE",
                    vec![Value::String(lua.create_string("invalid item slot index")?)],
                );
                return Ok(MultiValue::from_iter([Value::Boolean(false)]));
            }
            let data = s.borrow();
            let yes = data.game.get().is_some_and(|g| {
                data.enchant
                    .selection
                    .enchantable(&g.enchant, &data.cooldowns, data.enchant.mode, index as u8)
            });
            Ok(MultiValue::from_iter([Value::Boolean(yes)]))
        })?,
    )?;
    let s = state.clone();
    api.raw_set(
        "SetItemSlotFromBag",
        lua.create_function(move |lua, args: MultiValue| {
            let Some(slot) = number(args.get(1)).map(|v| v.wrapping_sub(1) as u8) else {
                return Ok(MultiValue::new());
            };
            let selected = {
                let mut data = s.borrow_mut();
                (0..3).any(|index| set(&mut data, index, slot).is_ok())
            };
            if selected {
                let presence = s.borrow().enchant.selection.presence();
                update(lua, presence);
            }
            Ok(MultiValue::from_iter([Value::Boolean(selected)]))
        })?,
    )?;
    for (name, index) in [
        ("SetTargetItemSlotFromPick", 0),
        ("SetEnchantItemSlotFromPick", 1),
        ("SetSupportItemSlotFromPick", 2),
    ] {
        let s = state.clone();
        api.raw_set(
            name,
            lua.create_function(move |lua, _: MultiValue| {
                let cursor: Table = lua.globals().get("X2Cursor")?;
                let held: Option<Table> = cursor.call_method("GetHeld", ())?;
                let Some(held) = held else { return Ok(MultiValue::new()) };
                let kind: Option<String> = held.get("kind")?;
                let item_action = kind.as_deref() == Some("action") && held.get::<Option<String>>("what")?.as_deref() == Some("item");
                if !matches!(kind.as_deref(), Some("bag" | "equip" | "bank" | "item")) && !item_action {
                    return Ok(MultiValue::new());
                }
                let slot = number(Some(&held.get::<Value>("slot")?)).map(|v| v.wrapping_sub(1) as u8);
                cursor.call_method::<()>("ClearCursor", ())?; //3942e790 BEFORE validation
                let result = match (kind.as_deref(), slot) {
                    (Some("bag"), Some(slot)) => set(&mut s.borrow_mut(), index, slot),
                    _ => Err(0xb5),
                };
                match result {
                    Ok(()) => {
                        let presence = s.borrow().enchant.selection.presence();
                        update(lua, presence)
                    }
                    Err(code) => error(lua, code)?,
                }
                Ok(MultiValue::from_iter([Value::Boolean(result.is_ok())]))
            })?,
        )?;
    }
    for (name, index) in [("ClearTargetItemSlot", 0), ("ClearEnchantItemSlot", 1), ("ClearSupportItemSlot", 2)] {
        let s = state.clone();
        api.raw_set(
            name,
            lua.create_function(move |lua, _: MultiValue| {
                let presence = {
                    let mut data = s.borrow_mut();
                    data.enchant.selection.clear_from(index);
                    index == 2 && data.enchant.selection.presence()
                };
                update(lua, presence);
                Ok(())
            })?,
        )?;
    }
    for (name, index) in [("GetTargetItemInfo", 0), ("GetEnchantItemInfo", 1), ("GetSupportItemInfo", 2)] {
        let s = state.clone();
        api.raw_set(name, lua.create_function(move |lua, _: MultiValue| info(lua, &s, index))?)?;
    }
    let s = state.clone();
    api.raw_set(
        "GetRatioInfos",
        lua.create_function(move |lua, _: MultiValue| {
            let data = s.borrow();
            let view = &data.enchant.selection;
            if data.enchant.mode != Mode::Grade || !view.presence() {
                return Ok(MultiValue::new());
            }
            let (Some(game), Some(target)) = (data.game.get(), view.selected(0)) else {
                return Ok(MultiValue::new());
            };
            let support = match view.slots[2] {
                None => None,
                Some(_) => {
                    let Some(item) = view.selected(2) else {
                        return Ok(MultiValue::new());
                    };
                    Some(item.template_id)
                }
            };
            let Some((base, variation)) = game.enchant.ratios(target.grade, support) else {
                return Ok(MultiValue::new());
            };
            let out = lua.create_table()?;
            for (i, key) in ["success", "break", "downgrade"].into_iter().enumerate() {
                out.raw_set(key, base[i])?;
                if let Some(variation) = variation {
                    out.raw_set(format!("{key}_var_ratio"), f64::from(variation[i] as f32))?;
                }
            }
            Ok(MultiValue::from_iter([Value::Table(out)]))
        })?,
    )?;
    let s = state.clone();
    api.raw_set(
        "Execute",
        lua.create_function(move |lua, _: MultiValue| {
            let prepared = {
                let data = s.borrow();
                if !data.enchant.selection.presence() {
                    drop(data);
                    aa_ui::lua::fire_ui_event(
                        lua,
                        "UI_ERROR_MESSAGE",
                        vec![Value::String(lua.create_string("invalid item slot index")?)],
                    );
                    return Ok(());
                }
                data.game
                    .get()
                    .ok_or(1i16)
                    .and_then(|game| data.enchant.selection.prepare(&game.enchant, &data.cooldowns, data.enchant.mode))
            };
            let prepared = match prepared {
                Ok(prepared) => prepared,
                Err(code) => {
                    error(lua, code)?;
                    return Ok(());
                }
            };
            {
                let mut data = s.borrow_mut();
                data.requests.push(super::super::api::Request::ItemEffect {
                    character_id: prepared.character_id,
                    request: prepared.request.clone(),
                });
                data.enchant.operation = Some(super::controller::Operation::new(prepared));
            }
            // Native accepted-helper UPDATE means working, not payment or success.
            // The explicit app write fence arms receipts only after Requested.
            update_working(lua, false, true);
            Ok(())
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests;
