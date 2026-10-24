//! Original proficiency controls over received grades, physical certificates and native SHOW rows.
use super::*;
use aa_net::{ActabilityCommand as Command, ActabilityEvent, Event, NetCommand, NetEvent};
use mlua::Value;

#[allow(dead_code)]
pub(super) mod fixture;

#[derive(Default)]
pub(super) struct View {
    pub(super) active: bool,
    requests: Vec<(u32, Command)>,
    constructed: Option<Construction>,
}

/// Constructor inputs for one HUD build. Getter observations also detect transient
/// modifier changes during multi-frame prebuilding; activation checks the final state.
struct Construction {
    owner: u32,
    received: aa_net::Actabilities,
    modifiers: Vec<(u32, i32)>,
    observed: bool,
    changed: bool,
}

impl Construction {
    fn capture(data: &HudData) -> Option<Self> {
        Some(Self {
            owner: data.character_id,
            received: data.crafting.actabilities.clone(),
            modifiers: data.game.get()?.crafting.groups.keys().map(|&id| (id, super::modify_point(data, id))).collect(),
            observed: false,
            changed: false,
        })
    }

    fn matches(&self, data: &HudData) -> bool {
        self.observed && !self.changed && self.inputs_match(data)
    }

    fn inputs_match(&self, data: &HudData) -> bool {
        self.owner == data.character_id
            && self.received == data.crafting.actabilities
            && self.modifiers.iter().all(|&(id, point)| point == super::modify_point(data, id))
    }
}

/// Loading-time stat updates can change modifiers between the original page
/// constructors. Remember any differing getter input, including A -> B -> A.
pub(super) fn observe_construction(data: &mut HudData) {
    let changed = data.crafting.expert.constructed.as_ref().is_some_and(|built| !built.inputs_match(data));
    if let Some(built) = data.crafting.expert.constructed.as_mut() {
        built.observed = true;
        built.changed |= changed;
    }
}

fn grade(tables: &Tables, grade: u8) -> Option<&super::data::Expert> {
    tables.grades.get(usize::from(grade)).and_then(|id| tables.experts.get(id))
}

fn eligible(tables: &Tables, id: u32) -> bool {
    !matches!(id, 0 | 19 | 21..=30) && tables.groups.get(&id).is_some_and(|g| g.skill_page_visible)
}

fn count(data: &HudData, target: u8, higher: bool) -> usize {
    let Some(tables) = data.game.get().map(|g| &g.crafting) else { return 0 };
    data.crafting
        .actabilities
        .entries
        .iter()
        .filter(|a| eligible(tables, a.id) && if higher { a.step >= target } else { a.step == target })
        .count()
}

fn upgrade(data: &HudData, id: u32) -> bool {
    let Some(tables) = data.game.get().map(|g| &g.crafting) else { return false };
    let Some(entry) = data.crafting.actabilities.get(id) else { return false };
    let Some(point) = entry.point else { return false };
    let Some(current) = grade(tables, entry.step) else { return false };
    let Some(next_step) = entry.step.checked_add(1) else { return false };
    let Some(next) = grade(tables, next_step) else { return false };
    if point < current.up_limit {
        return false;
    }
    if next.expert_limit == 0 {
        return true;
    }
    let Some(expanded) = data.crafting.actabilities.expanded_expert else {
        return false;
    };
    count(data, next_step, true) < next.expert_limit.wrapping_add(i32::from(expanded)).max(0) as usize
}

fn downgrade(data: &HudData, id: u32) -> bool {
    data.crafting.actabilities.get(id).is_some_and(|a| a.step > 0)
}

fn offered(data: &HudData) -> Option<(u32, &super::data::Expansion)> {
    let count = data.crafting.actabilities.expanded_expert?;
    let key = u32::from(count.wrapping_add(1));
    Some((key, data.game.get()?.crafting.expansions.get(&key)?))
}

fn grade_info(lua: &Lua, data: &HudData, index: i32) -> mlua::Result<Table> {
    let definition = u8::try_from(index)
        .ok()
        .and_then(|index| data.game.get().and_then(|g| grade(&g.crafting, index)));
    let expanded = i32::from(data.crafting.actabilities.expanded_expert.unwrap_or(0));
    let out = lua.create_table()?;
    out.set("grade", index)?;
    out.set("name", definition.map_or("", |g| g.name.as_str()))?;
    out.set("upLimit", definition.map_or(10000, |g| g.up_limit))?;
    out.set("expertLimit", definition.map_or(0, |g| g.expert_limit).wrapping_add(expanded))?;
    out.set("advantage", definition.map_or(0, |g| g.advantage))?;
    out.set("castAdvantage", definition.map_or(0., |g| g.cast_advantage))?;
    out.set("colorString", format!("{:08X}", definition.map_or(0, |g| g.color_argb)))?;
    out.set("gaugeColorString", format!("{:08X}", definition.map_or(0, |g| g.gauge_color)))?;
    // Current SHOW rows have raw currency0/price0 regardless of protected playerflags1000.
    // Priced catalog variants remain gated until that actual flag producer is available.
    for (currency, price) in [("upCurrency", "upPrice"), ("downCurrency", "downPrice")] {
        out.set(currency, 0)?;
        out.set(price, "0")?;
    }
    Ok(out)
}

pub(super) fn install(lua: &Lua, bridge: &Shared) -> mlua::Result<()> {
    let constructed = Construction::capture(&bridge.borrow());
    {
        let mut data = bridge.borrow_mut();
        data.crafting.expert.active = false;
        data.crafting.expert.constructed = constructed;
    }
    let ability: Table = lua.globals().get("X2Ability")?;
    let bag: Table = lua.globals().get("X2Bag")?;
    let state = bridge.clone();
    bag.set(
        "CountBagItemByItemType",
        lua.create_function(move |_, (_, id): (Value, u32)| {
            if id == 0 {
                return Ok(0.0);
            }
            let data = state.borrow();
            let count = data.crafting.inventory.as_ref().map_or(0, |inventory| {
                inventory
                    .bag
                    .iter()
                    .take(inventory.bag_slots as usize)
                    .flatten()
                    .filter(|i| i.template_id == id)
                    .fold(0i32, |total, item| total.wrapping_add(item.count))
            });
            Ok(f64::from(count as f32))
        })?,
    )?;
    let state = bridge.clone();
    ability.set(
        "GetMaxGrade",
        lua.create_function(move |_, _: Value| Ok(state.borrow().game.get().map_or(-1, |g| g.crafting.grades.len() as i32 - 1)))?,
    )?;
    let state = bridge.clone();
    ability.set(
        "GetGradeInfo",
        lua.create_function(move |lua, (_, index): (Value, i32)| grade_info(lua, &state.borrow(), index))?,
    )?;
    let state = bridge.clone();
    ability.set(
        "GetAllMyActabilityInfos",
        lua.create_function(move |lua, _: Value| {
            observe_construction(&mut state.borrow_mut());
            let data = state.borrow();
            let out = lua.create_table()?;
            if let Some(game) = data.game.get() {
                for (&id, group) in &game.crafting.groups {
                    if group.skill_page_visible {
                        if let Some(info) = super::my_info(lua, &data, id)? {
                            out.raw_push(info)?;
                        }
                    }
                }
            }
            Ok(out)
        })?,
    )?;
    ability.set(
        "IsLanguageActability",
        lua.create_function(|_, (_, id): (Value, u32)| Ok((21..=30).contains(&id)))?,
    )?;
    let state = bridge.clone();
    ability.set(
        "GetActabilityCountByGrade",
        lua.create_function(move |_, (_, index): (Value, u8)| Ok(count(&state.borrow(), index, false)))?,
    )?;
    for (name, minimum) in [("GetMinActabilityPoint", true), ("GetMaxActabilityPoint", false)] {
        let state = bridge.clone();
        ability.set(
            name,
            lua.create_function(move |_, (_, id): (Value, u32)| {
                let data = state.borrow();
                let step = super::proficiency(&data, id).map_or(0, |(_, s)| s);
                if minimum && step == 0 {
                    return Ok(0);
                }
                let step = if minimum { step - 1 } else { step };
                Ok(data.game.get().and_then(|g| grade(&g.crafting, step)).map_or(10000, |g| g.up_limit))
            })?,
        )?;
    }
    for (name, is_upgrade) in [("CanUpgradeExpert", true), ("CanDowngradeExpert", false)] {
        let state = bridge.clone();
        ability.set(
            name,
            lua.create_function(move |_, (_, id): (Value, u32)| {
                Ok(if is_upgrade {
                    upgrade(&state.borrow(), id)
                } else {
                    downgrade(&state.borrow(), id)
                })
            })?,
        )?;
    }
    let state = bridge.clone();
    ability.set(
        "CanExpandExpert",
        lua.create_function(move |_, _: Value| Ok(offered(&state.borrow()).is_some()))?,
    )?;
    let state = bridge.clone();
    ability.set(
        "GetExpandExpertInfo",
        lua.create_function(move |lua, _: Value| {
            let data = state.borrow();
            let Some((key, definition)) = offered(&data) else { return Ok(None) };
            let out = lua.create_table()?;
            out.set("requireLP", definition.life_points)?;
            if definition.item != 0 {
                out.set("requireItemType", definition.item)?;
                out.set("requireItemCount", definition.count)?;
            }
            let tables = &data.game.get().unwrap().crafting;
            let mut remain = 0;
            while tables.expansions.contains_key(&(key + remain + 1)) {
                remain += 1;
            }
            out.set("remainExpandCnt", remain)?;
            Ok(Some(out))
        })?,
    )?;
    for (name, is_upgrade) in [("UpgradeExpert", true), ("DowngradeExpert", false)] {
        let state = bridge.clone();
        ability.set(
            name,
            lua.create_function(move |_, (_, id): (Value, u32)| {
                let mut data = state.borrow_mut();
                let owner = data.character_id;
                let Some(inventory) = data.crafting.inventory.as_ref() else { return Ok(()) };
                let auto_use_aa_point = inventory.auto_use_aa_point();
                let command = if is_upgrade {
                    Command::Upgrade { id, auto_use_aa_point }
                } else {
                    Command::Downgrade { id, auto_use_aa_point }
                };
                data.crafting.expert.requests.push((owner, command));
                Ok(())
            })?,
        )?;
    }
    let state = bridge.clone();
    ability.set(
        "ExpandExpert",
        lua.create_function(move |_, _: Value| {
            let mut data = state.borrow_mut();
            let owner = data.character_id;
            data.crafting.expert.requests.push((owner, Command::Expand));
            Ok(())
        })?,
    )?;
    Ok(())
}

fn can_send(data: &HudData, command: Command) -> bool {
    if data.crafting.actabilities.pending.contains(&command) {
        return false;
    }
    match command {
        Command::Upgrade { id, .. } | Command::Downgrade { id, .. } => {
            let up = matches!(command, Command::Upgrade { .. });
            if !(if up { upgrade(data, id) } else { downgrade(data, id) }) {
                return false;
            }
            let Some(entry) = data.crafting.actabilities.get(id) else { return false };
            let Some(current) = data.game.get().and_then(|g| grade(&g.crafting, entry.step)) else {
                return false;
            };
            // Do not guess the protected premium-payment configuration for priced variants.
            if up {
                current.up_currency == 0 && current.up_price == 0
            } else {
                current.down_currency == 0 && current.down_price == 0
            }
        }
        Command::Expand => {
            let Some((_, offer)) = offered(data) else { return false };
            if offer.life_points >= 0 && data.snapshot.vocation < offer.life_points {
                return false;
            }
            if offer.item == 0 || offer.count <= 0 {
                return true;
            }
            let Some(inventory) = data.crafting.inventory.as_ref() else { return false };
            let count = inventory
                .bag
                .iter()
                .take(inventory.bag_slots as usize)
                .enumerate()
                .filter_map(|(slot, item)| item.as_ref().map(|item| (slot, item)))
                .filter(|(slot, item)| item.template_id == offer.item && u8::try_from(*slot).is_ok_and(|slot| !inventory.item_slot_pending(2, slot)))
                .fold(0i32, |total, (_, item)| total.wrapping_add(item.count));
            count >= offer.count
        }
    }
}

fn dispatch(rt: &aa_ui::UiRuntime, bridge: &Shared, event: &Event) {
    if !bridge.borrow_mut().crafting.actabilities.apply(event) {
        return;
    }
    notify_receipt(rt, bridge, event);
}

/// First activation has already copied the received state in sync_hud. Deliver
/// its real receipts without applying point deltas to that state a second time.
fn notify_receipt(rt: &aa_ui::UiRuntime, bridge: &Shared, event: &Event) {
    let notify = {
        let mut data = bridge.borrow_mut();
        let limits = if matches!(event, Event::Actability(ActabilityEvent::TierChanged { is_upgrade: false, .. })) {
            data.game.get().map(|g| g.crafting.raw_limits.clone()).unwrap_or_default()
        } else {
            Vec::new()
        };
        // Explicit lane reconciliation: its downgrade ACK omits points and resets to this raw row.
        data.crafting.actabilities.resolve_downgrade_points(event, &limits);
        match event {
            Event::Actability(ActabilityEvent::TierChanged { id, is_upgrade, step }) => data.game.get().and_then(|g| {
                g.crafting.groups.get(id).filter(|g| g.skill_page_visible).map(|group| {
                    (
                        *id,
                        *is_upgrade,
                        group.name.clone(),
                        grade(&g.crafting, *step).map_or(String::new(), |g| g.name.clone()),
                    )
                })
            }),
            _ => None,
        }
    };
    if let Some(args) = notify {
        rt.fire_event("ACTABILITY_EXPERT_GRADE_CHANGED", args);
    }
    if matches!(event, Event::Actability(ActabilityEvent::Snapshot { last: true, .. })) {
        rt.fire_event("ACTABILITY_REFRESH_ALL", ());
    }
    if let Event::LaborChanged { action, point, .. } = event {
        let args = (|| {
            if *action <= 0 || *point == 0 {
                return None;
            }
            let data = bridge.borrow();
            let id = *action as u32;
            let group = data.game.get()?.crafting.groups.get(&id).filter(|g| g.skill_page_visible)?;
            let base = data.crafting.actabilities.get(id)?.point?;
            Some((
                f64::from(id as f32),
                group.name.clone(),
                f64::from(*point as f32),
                f64::from(base.wrapping_add(super::modify_point(&data, id)) as f32),
            ))
        })();
        if let Some(args) = args {
            rt.fire_event("ACTABILITY_EXPERT_CHANGED", args);
        }
    }
    if matches!(event, Event::Actability(ActabilityEvent::Expanded { .. })) {
        rt.fire_event("ACTABILITY_EXPERT_EXPANDED", ());
    }
    if let Event::Actability(ActabilityEvent::RequestRejected { message, .. }) = event {
        rt.fire_event("SYSMSG", message.as_str());
    }
}

fn update(
    mut events: MessageReader<NetEvent>,
    rt: Option<NonSend<aa_ui::UiRuntime>>,
    bridge: NonSend<super::super::api::HudBridge>,
    mut out: MessageWriter<NetCommand>,
) {
    let Some(rt) = rt else {
        events.clear();
        return;
    };
    let (first, refresh) = {
        let mut data = bridge.0.borrow_mut();
        let first = !data.crafting.expert.active;
        data.crafting.expert.active = true;
        let constructed = if first { data.crafting.expert.constructed.take() } else { None };
        let refresh = first
            && data.crafting.actabilities.loaded
            && constructed.is_none_or(|built| !built.matches(&data));
        (first, refresh)
    };
    if first {
        let mut received_all = false;
        for NetEvent(event) in events.read() {
            notify_receipt(&rt, &bridge.0, event);
            received_all |= matches!(event, Event::Actability(ActabilityEvent::Snapshot { last: true, .. }));
        }
        // Fresh constructors already queried this exact state. A synthetic repeat
        // has no native packet behind it and triggers the shipped language bug.
        if !received_all && refresh {
            rt.fire_event("ACTABILITY_REFRESH_ALL", ());
        } else if !received_all {
            // Unlike the rows, the original button constructor never calls its
            // Update method. Initialize it without replaying the language event.
            if let Err(error) = rt.exec(
                "@proficiency-initialization",
                "local button=actabilityTab and actabilityTab.expandBtn;local initialize=button and rawget(button,'Update');assert(type(initialize)=='function','original proficiency expansion initializer missing');initialize(button)",
            ) {
                warn!("hud: proficiency initialization: {error}");
            }
        }
    } else {
        for NetEvent(event) in events.read() {
            dispatch(&rt, &bridge.0, event);
        }
    }
    let mut data = bridge.0.borrow_mut();
    let mut sent = Vec::new();
    for (owner, command) in std::mem::take(&mut data.crafting.expert.requests) {
        if owner != 0 && owner == data.character_id && !sent.contains(&command) && can_send(&data, command) {
            out.write(NetCommand(aa_net::Command::Actability(command)));
            sent.push(command);
        }
    }
}

pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        update
            .after(super::super::sync_hud)
            .before(super::batch::update)
            .before(aa_ui::UiFrame)
            .run_if(super::super::hud_up),
    );
}

#[cfg(test)]
mod tests;
