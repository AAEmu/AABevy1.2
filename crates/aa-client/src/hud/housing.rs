//! The original housing windows (`x2ui/housing`: the build window, the construction window, the house window and its
//! sale form, unchanged Lua) over aa-net's housing protocol: `X2House`, the events they listen to, and opening them at a
//! house. The builder itself (the ghost) is `ingame::housing::placement`. See `docs/aa-client-housing.md`.
//!
//! `X2House`'s getters have no house argument: like the original (`HousingManager + 0x78`, the house object id) they
//! answer for the house the player interacts with. A right click on a house in reach starts the interaction
//! (`393BEFD0`): `CSRequestHouseTax`, then `HOUSE_INTERACTION_START("housing", ..)`; `housing_manager.lua` opens the
//! construction window while the house is being built and the house window once it stands. Walking away ends it
//! (`HousingManager::Update`, `HOUSE_INTERACTION_END`). A right click on a house under construction also casts the
//! build action of its stage (`housing_build_steps.skill_id`, `SkillTargetType.Building`) when the bag holds the stage's
//! material: the server takes the item and the labor, advances the house and the window follows
//! (`SCHouseBuildProgress` -> `HOUSE_STEP_INFO_UPDATED`).
use std::cell::RefCell;
use std::rc::Rc;

use aa_net::housing::{ConstructHouseTax, HouseCommand, HouseSnapshot, HouseTaxInfo, HousingEvent};
use aa_net::model::{Unit, UnitKind};
use aa_net::state::{Doodads, Inventory, Units};
use aa_net::{Command, Event, Houses, NetCommand, NetEvent, Shipyards};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use mlua::{Lua, ObjectLike, Table, Value};

use super::api::{HudBridge, Shared};
use super::data::SharedGameData;
use super::input::HudPointerOverUi;
use crate::Screen;
use crate::ingame::data::Db;
use crate::ingame::housing::data::{self as house_data, Design};
use crate::ingame::housing::placement::{Builder, BuilderSignal};
use crate::ingame::player::placement;
use crate::ingame::shipyard::DrawnShipyards;

/// The demolition skill (`interaction_effects.wi_id` 94, `WI_DEMOLISH`) `X2House:Demolish` casts at the house.
const DEMOLISH_SKILL: u32 = 11404;
/// The tax certificates (`TaxItem` feature): one per gold of tax (`housing::hooks::construction_certificates`).
const TAX_CERTIFICATE: u32 = 31891;
const BOUND_TAX_CERTIFICATE: u32 = 31892;
const COPPER_PER_TAX_CERTIFICATE: i64 = 10_000;
/// The appraisal certificate a sale costs: one per 100 gold, at least one (`ops::calculate_sale_certificates`).
const APPRAISAL_CERTIFICATE: u32 = 28085;
const COPPER_PER_APPRAISAL: i64 = 1_000_000;
/// `CURRENCY_GOLD`: taxes and sales are in copper.
const CURRENCY_GOLD: i64 = 0;
/// A house is talked to within its garden plus this (m); the interaction ends a little further away.
const REACH: f32 = 6.0;
const LEAVE: f32 = 10.0;
/// How near the view ray has to pass a house's middle for a right click to pick it, as a share of its garden.
const PICK_SHARE: f32 = 0.8;
/// The least half-size of a shipyard's hull pick (m); the drawn stage model's bounds are usually larger.
const SHIPYARD_PICK_MIN: f32 = 3.0;
/// The half-size a shipyard that is not drawn yet is picked and held by (m).
const SHIPYARD_DEFAULT_HALF: f32 = 6.0;
/// A press and release further apart than this (logical pixels) is a drag, not a click.
const CLICK_SLOP: f32 = 6.0;

/// One decoration placed on the house (`housing_decorations` resolved through its item).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Furniture {
    /// The furniture item (`item_housing_decorations.item_id`).
    item: u32,
    /// `housing_decorations.actability_group_id` (the player's proficiency group, 0 for none).
    actability_group: u32,
    /// `housing_decorations.actability_up` (`GetDecoActabilityPoint`'s `decoPoint`).
    points: u32,
    /// `housing_decorations.deco_actability_group_id` (the furniture limit group).
    deco_group: u32,
}

/// What the window shows of the house interacted with (refreshed by [`sync`]).
#[derive(Clone, Debug, Default, PartialEq)]
struct Current {
    obj_id: u32,
    tl_id: u16,
    design: u32,
    /// The received house state; `None` until the server sent one for this house.
    snapshot: Option<HouseSnapshot>,
    name: String,
    mine: bool,
    tax: Option<HouseTaxInfo>,
    /// Every doodad attached to the house, the built-in furniture included.
    decorations: u32,
    /// The placed furniture the decoration tables know, in attachment order.
    furniture: Vec<Furniture>,
    /// The design's category as `HOUSE_INTERACTION_START` names it.
    view_type: Option<&'static str>,
    always_public: bool,
}

/// `HOUSE_INTERACTION_START`'s view type from `housings.category_id` (`393BEFD0`, `DEC:780640-780672`).
fn view_type(category_id: u32) -> Option<&'static str> {
    match category_id {
        7 => Some("seafarm"),
        8 | 16 | 17 => Some("farm"),
        9 | 10 | 11 | 12 | 15 | 18 => Some("house"),
        _ => None,
    }
}

/// The furniture of `attachments` the decoration tables know (`FUN_3932FD00`'s house list).
fn furniture_of(attachments: &[aa_net::housing::HousingAttachment], tables: Option<&super::data::housing::Tables>) -> Vec<Furniture> {
    let Some(tables) = tables else { return Vec::new() };
    attachments
        .iter()
        .filter_map(|a| {
            let design = tables.decoration_design_for_item(a.item_template_id)?;
            let d = tables.decorations.get(&design)?;
            Some(Furniture {
                item: a.item_template_id,
                actability_group: d.actability_group_id,
                points: d.actability_up,
                deco_group: d.deco_actability_group_id,
            })
        })
        .collect()
}

/// The shipyard the player last interacted with (`X2House`'s getters answer for it, as for a house).
#[derive(Clone, Debug, PartialEq)]
struct CurrentShipyard {
    obj_id: u32,
    id: u64,
    owner_name: String,
    /// `GetShipyardBaseProgress`: all build actions of the template.
    base: i32,
    /// `GetShipyardRemainProgress`: the build actions still to do.
    remain: i32,
}

impl Current {
    fn under_construction(&self) -> bool {
        self.snapshot.as_ref().is_some_and(|s| s.all_actions > 0 && s.current_actions < s.all_actions)
    }

    /// `X2House:GetHousePermission`'s value: the server's (`0` private, `1` guild, `2` public, `3` family) plus one.
    fn permission(&self) -> i64 {
        i64::from(self.snapshot.as_ref().map_or(0, |s| s.permission)) + 1
    }
}

/// What the window is told next (raised once the Lua state is free of borrows).
#[derive(Debug, PartialEq)]
enum Signal {
    Start,
    End,
    Tax,
    ChangeActability,
    Step,
    BuildInfo(ConstructHouseTax),
    BuilderStep,
    BuilderEnd,
    NameChanged,
    Permission,
    InfoUpdated,
    SetSell,
    CancelSell,
    Bought,
    Sold(String),
    Deco,
}

/// A refused action to show (`ERROR_MSG` name and the `$1` argument, usually the item).
#[derive(Clone, Debug, PartialEq)]
struct Notice {
    name: &'static str,
    arg: Option<String>,
}

#[derive(Default)]
struct View {
    current: Option<Current>,
    me: u32,
    money: i64,
    zone: String,
    /// Bag counts: tax certificates (bound and not), appraisal certificates.
    tax_certificates: i64,
    appraisals: i64,
    commands: Vec<Command>,
    signals: Vec<Signal>,
    notices: Vec<Notice>,
    builder_next: bool,
    builder_prev: bool,
    /// Dev hooks: a scripted interaction, a pinned builder spot and clicks.
    interact: Option<u32>,
    pin: Option<(f32, f32, f32)>,
    clicks: u32,
    pointer_picked: Option<u32>,
    last_tax: Option<HouseTaxInfo>,
    /// The received actability points the last frame compared; a difference raises the event.
    actability: Vec<(u32, Option<i32>)>,
    /// The shipyard the current interaction is with; the housing material window is not raised for it yet.
    shipyard: Option<CurrentShipyard>,
}

#[derive(Clone, Default)]
struct State(Rc<RefCell<View>>);

/// `CountTaxItemForTax`: the certificates a tax (copper, as the window's decimal string) costs.
fn certificates_for(copper: i64) -> i64 {
    (copper.max(0) + COPPER_PER_TAX_CERTIFICATE - 1) / COPPER_PER_TAX_CERTIFICATE
}

/// `GetHouseSaleItemInfo`: the appraisal certificates a sale price costs.
fn appraisals_for(copper: i64) -> i64 {
    ((copper.max(0) + COPPER_PER_APPRAISAL - 1) / COPPER_PER_APPRAISAL).max(1)
}

/// A money amount the scripts pass as a number or a decimal string.
fn amount(value: &Value) -> i64 {
    match value {
        Value::Integer(n) => i64::from(*n),
        Value::Number(n) => *n as i64,
        Value::String(s) => s.to_str().ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0),
        _ => 0,
    }
}

fn copper(gold: &Value, silver: &Value, copper: &Value) -> i64 {
    amount(gold) * 10_000 + amount(silver) * 100 + amount(copper)
}

/// The material of a construction stage and where the inventory holds it.
#[derive(Clone, Debug, PartialEq)]
struct Material {
    item: u32,
    in_bag: bool,
    /// The client's local check only looks at the bag; an equipped backpack still casts (the
    /// 2026-10-31 capture: "Not enough Stone Pack." but `CSStartSkill` went out and the server
    /// paid with the equipped bundle).
    equipped: bool,
}

/// The stage of a construction `actions` build actions in, and the material it needs.
#[derive(Clone, Debug, PartialEq)]
struct BuildAction {
    skill: u32,
    material: Option<Material>,
}

fn held(inventory: &Inventory, item: u32, count: i32) -> (bool, bool) {
    let total = |items: &mut dyn Iterator<Item = &aa_net::model::Item>| -> i64 {
        items.filter(|i| i.template_id == item).map(|i| i64::from(i.count)).sum()
    };
    let bag = total(&mut inventory.bag.iter().flatten()) >= i64::from(count);
    let equipped = total(&mut inventory.equipment.slots.iter().flatten()) >= i64::from(count);
    (bag, equipped)
}

fn build_action(design: &Design, actions: i32, inventory: &Inventory) -> Option<BuildAction> {
    let step = design.steps.get(design.step_at(actions)?)?;
    if step.item == 0 {
        return Some(BuildAction { skill: step.skill, material: None });
    }
    let count = step.count.max(1);
    let (in_bag, equipped) = held(inventory, step.item, count);
    Some(BuildAction {
        skill: step.skill,
        material: Some(Material { item: step.item, in_bag, equipped }),
    })
}

/// The construction steps of `design` as the windows list them: each stage's material (the item's tooltip table plus
/// `consumeItemStack`), `step` (1 based) and `needActions`; a stage without a material is labor (`isLaborPower`).
fn steps_table(lua: &Lua, design: u32) -> mlua::Result<Table> {
    let out = lua.create_table()?;
    let Some(d) = house_data::get().and_then(|h| h.designs.get(&design)) else { return Ok(out) };
    let x2item: Table = lua.globals().get("X2Item")?;
    for (i, step) in d.steps.iter().enumerate() {
        let entry = if step.item != 0 {
            x2item.call_method::<Option<Table>>("GetItemInfoByType", step.item)?.unwrap_or(lua.create_table()?)
        } else {
            lua.create_table()?
        };
        entry.set("consumeItemStack", step.count.max(1))?;
        entry.set("isLaborPower", step.item == 0)?;
        entry.set("step", i as i64 + 1)?;
        entry.set("needActions", step.actions)?;
        out.set(i + 1, entry)?;
    }
    Ok(out)
}

/// A unix time as the date table the windows read (`os.date("*t")`, `minute` / `second` keys).
fn date(lua: &Lua, unix: i64) -> mlua::Result<Table> {
    let out = lua.create_table()?;
    let os: Table = lua.globals().get("os")?;
    let t = os.call_function::<Option<Table>>("date", ("*t", unix.max(0))).ok().flatten();
    for (key, source) in [("year", "year"), ("month", "month"), ("day", "day"), ("hour", "hour"), ("minute", "min"), ("second", "sec")] {
        out.set(key, t.as_ref().and_then(|t| t.get::<i64>(source).ok()).unwrap_or(0))?;
    }
    Ok(out)
}

/// Whether the taxes are paid in certificates (the server's `TaxItem` feature, on in its default feature set).
fn tax_item(lua: &Lua) -> bool {
    let feature = || -> mlua::Result<bool> {
        let player: Table = lua.globals().get("X2Player")?;
        let set: Table = player.call_method("GetFeatureSet", ())?;
        Ok(set.get::<Option<bool>>("taxItem")?.unwrap_or(true))
    };
    feature().unwrap_or(true)
}

pub fn install(lua: &Lua, bridge: &Shared) -> mlua::Result<()> {
    let state = State::default();
    lua.set_app_data(state.clone());
    let h: Table = lua.globals().get("X2House")?;
    install_house(lua, &h, bridge, &state)?;
    install_construction(lua, &h, bridge, &state)?;
    install_tax(lua, &h, &state)?;
    install_sale(lua, &h, bridge, &state)?;
    install_dev(lua, &state)
}

/// Getters and setters of the house interacted with.
fn install_house(lua: &Lua, h: &Table, bridge: &Shared, state: &State) -> mlua::Result<()> {
    macro_rules! getter {
        ($name:literal, |$v:ident| $body:expr) => {{
            let s = state.clone();
            h.set($name, lua.create_function(move |_, _: Value| {
                let $v = s.0.borrow();
                Ok($body)
            })?)?;
        }};
    }
    getter!("IsMyHouse", |v| v.current.as_ref().is_some_and(|c| c.mine));
    getter!("IsHouseUnderConstruction", |v| v.current.as_ref().is_some_and(Current::under_construction));
    getter!("GetHouseOwnerName", |v| v.current.as_ref().and_then(|c| c.snapshot.as_ref()).map(|s| s.owner_name.clone()).unwrap_or_default());
    getter!("GetHouseName", |v| v.current.as_ref().map(|c| c.name.clone()).unwrap_or_default());
    getter!("GetHouseZoneName", |v| v.zone.clone());
    getter!("GetHouseAllowRecover", |v| v.current.as_ref().and_then(|c| c.snapshot.as_ref()).is_some_and(|s| s.allow_recover));
    getter!("GetHouseBaseProgress", |v| v.current.as_ref().and_then(|c| c.snapshot.as_ref()).map_or(0, |s| s.all_actions));
    getter!("GetHouseRemainProgress", |v| v.current.as_ref().and_then(|c| c.snapshot.as_ref()).map_or(0, |s| (s.all_actions - s.current_actions).max(0)));
    // no castles, no sieges in 1.2's housing land
    getter!("IsCastle", |_v| false);
    getter!("IsSiegePeriod", |_v| false);
    // The shipyard material window is not raised here yet (it needs `GetShipyardConstructionStepInfo`);
    // these answer from the last shipyard the player interacted with, as the original does.
    getter!("GetShipyardOwnerName", |v| v.shipyard.as_ref().map(|s| s.owner_name.clone()).unwrap_or_default());
    getter!("GetShipyardProtectionPeriod", |_v| 0);
    getter!("GetShipyardRemainProgress", |v| v.shipyard.as_ref().map_or(0, |s| s.remain));
    getter!("GetShipyardBaseProgress", |v| v.shipyard.as_ref().map_or(0, |s| s.base));
    let (s, b) = (state.clone(), bridge.clone());
    h.set(
        "GetHousePermission",
        lua.create_function(move |_, _: Value| {
            let v = s.0.borrow();
            let always_public = v.current.as_ref().is_some_and(|c| template(&b, c.design, |t| t.always_public));
            Ok((v.current.as_ref().map(Current::permission), always_public))
        })?,
    )?;
    let s = state.clone();
    h.set(
        "SetHouseName",
        lua.create_function(move |_, (_, name): (Value, String)| {
            let mut v = s.0.borrow_mut();
            if let Some(tl) = v.current.as_ref().filter(|c| c.mine).map(|c| c.tl_id) {
                v.commands.push(Command::House(HouseCommand::ChangeName { tl, name }));
            }
            Ok(())
        })?,
    )?;
    let (s, b) = (state.clone(), bridge.clone());
    h.set(
        "SetHousePermission",
        lua.create_function(move |_, (_, permission): (Value, i64)| {
            let mut v = s.0.borrow_mut();
            let Some((tl, design, current, mine)) = v
                .current
                .as_ref()
                .map(|c| (c.tl_id, c.design, c.permission(), c.mine))
            else {
                return Ok(());
            };
            // `FUN_3932BF90`: an always-public design refuses every change (0x80), before the owner test.
            if template(&b, design, |t| t.always_public) {
                v.notices.push(Notice { name: "HOUSE_CANNOT_CHANGE_PERMISSION", arg: None });
                return Ok(());
            }
            if !mine || permission == current || !(1..=4).contains(&permission) {
                return Ok(());
            }
            // `SetHousePermission` passes the radio's 1-based value; the packet carries the server enum.
            let permission = (permission - 1) as u8;
            if permission == 3 {
                // `FUN_39300000`: the family permission needs a family (0x178).
                if b.borrow().family.own_id() == 0 {
                    v.notices.push(Notice { name: "FAMILY_NOT_EXIST", arg: None });
                    return Ok(());
                }
            } else if permission == 1 {
                // `FUN_392F80E0`: the guild permission needs an expedition (0x137).
                if b.borrow().expedition.own_id() == 0 {
                    v.notices.push(Notice { name: "CHAT_NOT_IN_EXPEDITION", arg: None });
                    return Ok(());
                }
            }
            v.commands.push(Command::House(HouseCommand::ChangePermission { tl, permission }));
            Ok(())
        })?,
    )?;
    let s = state.clone();
    h.set(
        "SetHouseAllowRecover",
        lua.create_function(move |_, (_, allow): (Value, bool)| {
            let mut v = s.0.borrow_mut();
            let Some(c) = v.current.as_ref().filter(|c| c.mine) else { return Ok(()) };
            // the packet toggles
            if c.snapshot.as_ref().is_some_and(|s| s.allow_recover != allow) {
                let tl = c.tl_id;
                v.commands.push(Command::House(HouseCommand::AllowRecover { tl }));
            }
            Ok(())
        })?,
    )?;
    let s = state.clone();
    h.set(
        "Demolish",
        lua.create_function(move |_, _: Value| {
            let mut v = s.0.borrow_mut();
            if let Some(obj_id) = v.current.as_ref().filter(|c| c.mine).map(|c| c.obj_id) {
                v.commands.push(Command::UseSkill { skill_id: DEMOLISH_SKILL, target_obj_id: obj_id });
            }
            Ok(())
        })?,
    )?;
    // `FUN_394796B0`: both counts exclude the design's built-in furniture (`housing_binding_doodads`).
    let (s, b) = (state.clone(), bridge.clone());
    h.set(
        "GetHouseDecoCount",
        lua.create_function(move |_, _: Value| {
            let v = s.0.borrow();
            let (attached, builtins, absolute) = v.current.as_ref().map_or((0, 0, 0), |c| {
                let (builtins, absolute) = template(&b, c.design, |t| (t.binding_doodads, t.absolute_deco_limit));
                (c.decorations, builtins, absolute)
            });
            // the built-ins may not have streamed in yet (or the house is still a frame): never negative
            Ok((
                (i64::from(attached) - i64::from(builtins)).max(0),
                (i64::from(absolute) - i64::from(builtins)).max(0),
            ))
        })?,
    )?;
    let (s, b) = (state.clone(), bridge.clone());
    h.set(
        "GetActabilityDecoCount",
        lua.create_function(move |_, _: Value| {
            let v = s.0.borrow();
            let Some(c) = v.current.as_ref() else { return Ok(0) };
            let tables = b.borrow();
            Ok(actability_entries(&c.furniture, c.design, tables.game.get().map(|g| &g.housing)).len() as i64)
        })?,
    )?;
    let (s, b) = (state.clone(), bridge.clone());
    h.set(
        "GetDecoActabilityPoint",
        lua.create_function(move |lua, (_, index): (Value, i64)| {
            let v = s.0.borrow();
            let Some(c) = v.current.as_ref() else { return Ok(Value::Nil) };
            let entries = {
                let tables = b.borrow();
                actability_entries(&c.furniture, c.design, tables.game.get().map(|g| &g.housing))
            };
            let Some(&(group, points)) = usize::try_from(index - 1).ok().and_then(|i| entries.get(i)) else {
                return Ok(Value::Nil);
            };
            let name = {
                let data = b.borrow();
                crate::hud::crafting::actability_group_name(&data, group).unwrap_or("").to_owned()
            };
            let t = lua.create_table()?;
            t.set("name", name)?;
            t.set("decoPoint", points)?;
            Ok(Value::Table(t))
        })?,
    )?;
    let (s, b) = (state.clone(), bridge.clone());
    h.set(
        "GetDecoActabilityByType",
        lua.create_function(move |lua, (_, index): (Value, i64)| {
            let v = s.0.borrow();
            let Some(c) = v.current.as_ref() else { return Ok((Value::Nil, Value::Nil)) };
            let entries = {
                let tables = b.borrow();
                actability_entries(&c.furniture, c.design, tables.game.get().map(|g| &g.housing))
            };
            let Some(&(group, points)) = usize::try_from(index - 1).ok().and_then(|i| entries.get(i)) else {
                return Ok((Value::Nil, Value::Nil));
            };
            let data = b.borrow();
            let totals = lua.create_table()?;
            totals.set("type", group)?;
            totals.set("totalDecoPoint", points)?;
            totals.set("totalPoint", crate::hud::crafting::proficiency(&data, group).map_or(0, |(point, _)| point))?;
            let items = lua.create_table()?;
            // the same rows `actability_entries` sums, so the list adds up to `totalDecoPoint`
            for (i, f) in c.furniture.iter().filter(|f| f.actability_group == group && f.deco_group != 0).enumerate() {
                let row = lua.create_table()?;
                let name = data.game.get().and_then(|g| g.items.get(&f.item)).map(|i| i.named.name.clone()).unwrap_or_default();
                row.set("name", name)?;
                row.set("decoPoint", f.points)?;
                items.set(i + 1, row)?;
            }
            Ok((Value::Table(totals), Value::Table(items)))
        })?,
    )?;
    let (s, b) = (state.clone(), bridge.clone());
    h.set(
        "GetDecoActabilityLimitGroup",
        lua.create_function(move |lua, _: Value| {
            let v = s.0.borrow();
            let Some(c) = v.current.as_ref() else { return Ok(Value::Nil) };
            let tables = b.borrow();
            let Some(game) = tables.game.get() else { return Ok(Value::Nil) };
            let Some(design) = game.housing.housings.get(&c.design) else { return Ok(Value::Nil) };
            // `FUN_3947A310`: the getter answers nil when the design has no `housing_deco_limits` row,
            // which `maintain_window.alb` shows as `noneFurniture`; the 170 designs with a NULL (0)
            // `housing_deco_limit_id` are that case.
            if design.housing_deco_limit_id == 0 {
                return Ok(Value::Nil);
            }
            let out = lua.create_table()?;
            let mut row = 1;
            for elem in game.housing.deco_limit_elems(design.housing_deco_limit_id) {
                let Some(name) = game.housing.deco_actability_groups.get(&elem.deco_actability_group_id) else {
                    continue;
                };
                if name.is_empty() {
                    continue;
                }
                let arrange = c.furniture.iter().filter(|f| f.deco_group == elem.deco_actability_group_id).count();
                let entry = lua.create_table()?;
                entry.set("name", name.as_str())?;
                entry.set("arrangeCount", arrange)?;
                entry.set("maxCount", elem.count)?;
                out.set(row, entry)?;
                row += 1;
            }
            Ok(Value::Table(out))
        })?,
    )?;
    Ok(())
}

/// `FUN_39325EC0`'s house list: one entry per decoration actability group, in placement order, with the
/// `actability_up` of its furniture summed. `FUN_3932FD00` adds nothing for a design without a limit
/// list (no `housing_deco_limits` row: the 170 NULL `housing_deco_limit_id` designs) or for a
/// decoration without a `deco_actability_group_id` (`piVar2[4] != DAT_3a167edc`), so neither reaches the
/// counters.
fn actability_entries(
    furniture: &[Furniture],
    design: u32,
    tables: Option<&super::data::housing::Tables>,
) -> Vec<(u32, u32)> {
    let Some(tables) = tables else {
        return Vec::new();
    };
    let Some(template) = tables.housings.get(&design) else {
        return Vec::new();
    };
    if template.housing_deco_limit_id == 0 {
        return Vec::new();
    }
    let mut out: Vec<(u32, u32)> = Vec::new();
    for f in furniture
        .iter()
        .filter(|f| f.actability_group != 0 && f.deco_group != 0)
    {
        match out
            .iter_mut()
            .find(|(group, _)| *group == f.actability_group)
        {
            Some((_, points)) => *points += f.points,
            None => out.push((f.actability_group, f.points)),
        }
    }
    out
}

/// What the HUD's static housing tables (localized names, taxes, limits) say of `design`, once loaded.
fn template<R: Default>(bridge: &Shared, design: u32, f: impl FnOnce(&super::data::housing::HousingTemplate) -> R) -> R {
    bridge.borrow().game.get().and_then(|g| g.housing.housings.get(&design)).map(f).unwrap_or_default()
}

/// The build window and the construction window: the stages, the builder's steps.
fn install_construction(lua: &Lua, h: &Table, bridge: &Shared, state: &State) -> mlua::Result<()> {
    let b = bridge.clone();
    h.set("GetHousingModelName", lua.create_function(move |_, (_, design): (Value, u32)| Ok(template(&b, design, |t| t.name.clone())))?)?;
    h.set("GetHousingModelConstructionInfo", lua.create_function(|lua, (_, design): (Value, u32)| steps_table(lua, design))?)?;
    let s = state.clone();
    h.set(
        "GetHouseConstructionStepInfo",
        lua.create_function(move |lua, _: Value| {
            let design = s.0.borrow().current.as_ref().map_or(0, |c| c.design);
            steps_table(lua, design)
        })?,
    )?;
    let s = state.clone();
    h.set("NextStepFromBuildCheck", lua.create_function(move |_, _: Value| {
        s.0.borrow_mut().builder_next = true;
        Ok(())
    })?)?;
    let s = state.clone();
    h.set("PrevStepFromBuildCheck", lua.create_function(move |_, _: Value| {
        s.0.borrow_mut().builder_prev = true;
        Ok(())
    })?)?;
    // the build pay of the old interface: nothing to pay besides the tax
    h.set("GetHouseBuildPay", lua.create_function(|_, _: Value| Ok(0))?)?;
    h.set("GetHouseBuildPayStr", lua.create_function(|_, _: Value| Ok("0"))?)?;
    h.set("SetHouseBuildPay", lua.create_function(|_, _: Value| Ok(()))?)?;
    h.set("CalcHouseBuildTotalPayStr", lua.create_function(|_, (_, money): (Value, Value)| Ok(amount(&money).to_string()))?)?;
    Ok(())
}

/// Taxes: certificates, currency.
fn install_tax(lua: &Lua, h: &Table, state: &State) -> mlua::Result<()> {
    for name in ["GetCurrencyForTax", "GetCurrencyForHouseSale"] {
        h.set(name, lua.create_function(|_, _: Value| Ok(CURRENCY_GOLD))?)?;
    }
    h.set("CountTaxItemForTax", lua.create_function(|_, (_, tax): (Value, Value)| Ok(certificates_for(amount(&tax))))?)?;
    let s = state.clone();
    h.set("CountTaxItemInBag", lua.create_function(move |_, _: Value| Ok(s.0.borrow().tax_certificates))?)?;
    h.set(
        "GetTaxItem",
        lua.create_function(|lua, _: Value| {
            let x2item: Table = lua.globals().get("X2Item")?;
            let name = x2item.call_method::<Option<Table>>("GetItemInfoByType", TAX_CERTIFICATE)?.and_then(|t| t.get::<String>("name").ok()).unwrap_or_default();
            let t = lua.create_table()?;
            t.set("name", name)?;
            t.set("itemType", TAX_CERTIFICATE)?;
            Ok(t)
        })?,
    )?;
    Ok(())
}

/// The sale form and buying a house that is for sale.
fn install_sale(lua: &Lua, h: &Table, bridge: &Shared, state: &State) -> mlua::Result<()> {
    let (s, b) = (state.clone(), bridge.clone());
    h.set(
        "GetHouseSaleInfo",
        lua.create_function(move |lua, _: Value| {
            let v = s.0.borrow();
            let t = lua.create_table()?;
            let Some(c) = v.current.as_ref() else { return Ok(t) };
            let (price, target, target_id) = c.snapshot.as_ref().map_or((0, String::new(), 0), |s| (i64::from(s.sell_price.max(0)), s.sell_to_name.clone(), s.sell_to_player_id));
            let on_sale = price > 0;
            let sellable = template(&b, c.design, |t| t.is_sellable);
            t.set("onSale", on_sale)?;
            t.set("price", price.to_string())?;
            t.set("gold", price / 10_000)?;
            t.set("silver", price / 100 % 100)?;
            t.set("copper", price % 100)?;
            t.set("targetName", target)?;
            t.set("canSell", c.mine && sellable && !c.under_construction())?;
            t.set("canBuy", on_sale && !c.mine && (target_id == 0 || target_id == v.me))?;
            Ok(t)
        })?,
    )?;
    let s = state.clone();
    h.set(
        "GetHouseSaleItemInfo",
        lua.create_function(move |lua, (_, g, sv, c): (Value, Value, Value, Value)| {
            let v = s.0.borrow();
            let t = lua.create_table()?;
            t.set("hasCount", v.appraisals)?;
            t.set("needCount", appraisals_for(copper(&g, &sv, &c)))?;
            t.set("itemType", APPRAISAL_CERTIFICATE)?;
            Ok(t)
        })?,
    )?;
    let s = state.clone();
    h.set(
        "SetHouseForSale",
        lua.create_function(move |_, (_, g, sv, c, name): (Value, Value, Value, Value, Option<String>)| {
            let mut v = s.0.borrow_mut();
            let price = copper(&g, &sv, &c);
            let Some(tl) = v.current.as_ref().filter(|c| c.mine).map(|c| c.tl_id) else { return Ok(()) };
            if !(1..=i64::from(i32::MAX)).contains(&price) {
                v.notices.push(Notice { name: "HOUSE_CANNOT_SELL_AS_TOO_LOW_PRICE", arg: None });
                return Ok(());
            }
            v.commands.push(Command::House(HouseCommand::Sell { tl, money_amount: price as u32, sell_to: name.unwrap_or_default() }));
            Ok(())
        })?,
    )?;
    let s = state.clone();
    h.set(
        "CancelHouseForSale",
        lua.create_function(move |_, _: Value| {
            let mut v = s.0.borrow_mut();
            if let Some(tl) = v.current.as_ref().filter(|c| c.mine).map(|c| c.tl_id) {
                v.commands.push(Command::House(HouseCommand::CancelSell { tl }));
            }
            Ok(())
        })?,
    )?;
    let s = state.clone();
    h.set(
        "BuyHouse",
        lua.create_function(move |_, _: Value| {
            let mut v = s.0.borrow_mut();
            let Some((tl, price)) = v.current.as_ref().and_then(|c| c.snapshot.as_ref().map(|s| (c.tl_id, s.sell_price.max(0) as u32))) else { return Ok(()) };
            if price == 0 {
                v.notices.push(Notice { name: "HOUSE_CANNOT_BUY_AS_NOT_FOR_SALE", arg: None });
            } else if i64::from(price) > v.money {
                v.notices.push(Notice { name: "HOUSE_CANNOT_BUY_AS_NOT_ENOUGH_MONEY", arg: None });
            } else {
                v.commands.push(Command::House(HouseCommand::Buy { tl, money_amount: price }));
            }
            Ok(())
        })?,
    )?;
    Ok(())
}

/// Dev hooks for scripted runs (`lua` lines of `docs/aa-client-script.md`): `HUD.house_interact(obj_id)` is the right
/// click on a house, `HUD.house_builder(x, y, yaw)` pins the builder's ghost to a spot, `HUD.house_builder_click()` is a
/// left click in the builder.
fn install_dev(lua: &Lua, state: &State) -> mlua::Result<()> {
    let hud: Table = lua.globals().get("HUD")?;
    let s = state.clone();
    hud.raw_set("house_interact", lua.create_function(move |_, obj_id: u32| {
        s.0.borrow_mut().interact = Some(obj_id);
        Ok(())
    })?)?;
    let s = state.clone();
    hud.raw_set("house_builder", lua.create_function(move |_, (x, y, yaw): (f32, f32, f32)| {
        s.0.borrow_mut().pin = Some((x, y, yaw));
        Ok(())
    })?)?;
    let s = state.clone();
    hud.raw_set("house_builder_click", lua.create_function(move |_, ()| {
        s.0.borrow_mut().clicks += 1;
        Ok(())
    })?)?;
    Ok(())
}

fn notice(rt: &UiRuntime, notice: &Notice) {
    let args: Vec<String> = notice.arg.iter().cloned().collect();
    let text = super::alerts::localized_args(rt, "ERROR_MSG", notice.name, &args).unwrap_or_else(|| notice.name.to_owned());
    rt.fire_event("SYSMSG", text);
}

fn fire(rt: &UiRuntime, state: &State, signal: Signal) {
    let lua = &rt.lua;
    match signal {
        Signal::Start => {
            let (view_type, always_public) = state
                .0
                .borrow()
                .current
                .as_ref()
                .map_or((None, false), |c| (c.view_type, c.always_public));
            rt.fire_event(
                "HOUSE_INTERACTION_START",
                ("housing", view_type.map(str::to_owned), always_public),
            );
        }
        Signal::End => rt.fire_event("HOUSE_INTERACTION_END", ()),
        Signal::ChangeActability => rt.fire_event("CHANGE_ACTABILITY_DECO_NUM", ()),
        Signal::Tax => {
            let tax = state.0.borrow().current.as_ref().and_then(|c| c.tax.clone());
            let Some(tax) = tax else { return };
            let Ok(due) = date(lua, tax.due) else { return };
            let item = tax_item(lua).then_some(TAX_CERTIFICATE);
            rt.fire_event(
                "HOUSE_TAX_INFO",
                (
                    tax.dominion_tax_rate,
                    tax.money_amount2.to_string(),
                    due,
                    tax.weeks_without_pay,
                    tax.is_already_paid,
                    tax.is_heavy_tax_house,
                    tax.money_amount.to_string(),
                    item,
                ),
            );
        }
        Signal::Step => rt.fire_event("HOUSE_STEP_INFO_UPDATED", "housing"),
        Signal::BuildInfo(q) => {
            let item = tax_item(lua).then_some(TAX_CERTIFICATE);
            rt.fire_event(
                "HOUSE_BUILD_INFO",
                (
                    q.design_id,
                    q.base_tax_money_amount.to_string(),
                    q.total_tax_money_amount.to_string(),
                    q.heavy_tax_house_count,
                    q.normal_tax_house_count,
                    q.is_heavy_tax_house,
                    q.deposit_tax_money_amount.to_string(),
                    item,
                ),
            );
        }
        Signal::BuilderStep => rt.fire_event("BUILDER_STEP", ()),
        Signal::BuilderEnd => rt.fire_event("BUILDER_END", ()),
        Signal::NameChanged => rt.fire_event("UNIT_NAME_CHANGED", ()),
        Signal::Permission => rt.fire_event("HOUSE_PERMISSION_UPDATED", ()),
        Signal::InfoUpdated => rt.fire_event("HOUSE_INFO_UPDATED", ()),
        Signal::SetSell => rt.fire_event("HOUSE_SET_SELL_SUCCESS", ()),
        Signal::CancelSell => rt.fire_event("HOUSE_CANCEL_SELL_SUCCESS", ()),
        Signal::Bought => rt.fire_event("HOUSE_BUY_SUCCESS", ()),
        Signal::Sold(name) => rt.fire_event("HOUSE_SALE_SUCCESS", name),
        Signal::Deco => rt.fire_event("HOUSE_DECO_UPDATED", ()),
    }
}

/// The house or shipyard a right click released over the 3D view picked: the nearest whose middle the view ray
/// passes within its garden, or whose hull the ray enters (not a drag, not on the HUD).
fn pick(ray: Ray3d, units: &Units, drawn: Option<&DrawnShipyards>) -> Option<u32> {
    let data = house_data::get();
    let mut best: Option<(f32, u32)> = None;
    for u in units.iter() {
        let along = match u.kind {
            UnitKind::Housing { template_id, .. } => {
                let radius = data
                    .and_then(|d| d.designs.get(&template_id))
                    .map_or(5.0, |d| d.garden_radius.max(3.0))
                    * PICK_SHARE;
                // the house's middle: a few metres over its floor
                let centre = aa_world::cry_to_bevy(Vec3::new(u.pos.x, u.pos.y, u.pos.z + 3.0));
                let along = (centre - ray.origin).dot(*ray.direction);
                if along <= 0.0 {
                    continue;
                }
                let miss = (ray.origin + *ray.direction * along).distance(centre);
                if miss > radius {
                    continue;
                }
                along
            }
            UnitKind::Shipyard { .. } => {
                // A hull-sized pick, not the drydock's `build_radius`: the drawn stage model's bounds.
                let bounds = drawn.and_then(|d| d.bounds(u.obj_id)).unwrap_or((
                    -Vec3::splat(SHIPYARD_DEFAULT_HALF),
                    Vec3::splat(SHIPYARD_DEFAULT_HALF),
                ));
                let Some(along) = shipyard_hit(&ray, u, bounds) else {
                    continue;
                };
                along
            }
            _ => continue,
        };
        if best.is_none_or(|(a, _)| along < a) {
            best = Some((along, u.obj_id));
        }
    }
    best.map(|(_, id)| id)
}

/// The horizontal half-extents of a stage model's bounds, each at least [`SHIPYARD_PICK_MIN`] metres.
fn half_extents((lo, hi): (Vec3, Vec3)) -> Vec2 {
    Vec2::new(lo.x.abs().max(hi.x.abs()), lo.y.abs().max(hi.y.abs()))
        .max(Vec2::splat(SHIPYARD_PICK_MIN))
}

/// The distance at which the view ray enters the drawn stage model's bounds (unit-local cry metres).
fn shipyard_hit(ray: &Ray3d, unit: &Unit, bounds: (Vec3, Vec3)) -> Option<f32> {
    let scale = unit.scale.max(0.01);
    let transform = placement(
        Vec3::new(unit.pos.x, unit.pos.y, unit.pos.z),
        unit.yaw(),
        scale,
    );
    let inverse = transform.compute_affine().inverse();
    let origin = aa_world::bevy_to_cry(inverse.transform_point3(ray.origin));
    let direction = aa_world::bevy_to_cry(inverse.transform_vector3(*ray.direction));
    // a metre of slack around the authored bounds
    let (lo, hi) = bounds;
    ray_box(
        origin,
        direction,
        lo - Vec3::splat(1.0),
        hi + Vec3::splat(1.0),
    )
    // the local direction keeps the 1/scale length of the inverse, so `t` is already the world
    // distance along the ray
}

/// The first distance at which the ray enters the axis-aligned box, if it does.
fn ray_box(origin: Vec3, direction: Vec3, lo: Vec3, hi: Vec3) -> Option<f32> {
    let mut tmin = f32::NEG_INFINITY;
    let mut tmax = f32::INFINITY;
    for i in 0..3 {
        let d = direction[i];
        if d.abs() < 1e-6 {
            if origin[i] < lo[i] || origin[i] > hi[i] {
                return None;
            }
            continue;
        }
        let (mut t1, mut t2) = ((lo[i] - origin[i]) / d, (hi[i] - origin[i]) / d);
        if t1 > t2 {
            std::mem::swap(&mut t1, &mut t2);
        }
        tmin = tmin.max(t1);
        tmax = tmax.min(t2);
        if tmin > tmax {
            return None;
        }
    }
    (tmax >= 0.0).then(|| tmin.max(0.0))
}

fn garden(template_id: u32) -> f32 {
    house_data::get().and_then(|d| d.designs.get(&template_id)).map_or(5.0, |d| d.garden_radius)
}

/// The facts of house unit `obj_id` the window reads; `tables` is the HUD's static housing data, when loaded.
fn current_of(
    obj_id: u32,
    units: &Units,
    houses: &Houses,
    doodads: &Doodads,
    me: u32,
    tax: Option<HouseTaxInfo>,
    tables: Option<&super::data::housing::Tables>,
) -> Option<Current> {
    let unit = units.get(obj_id)?;
    let UnitKind::Housing { tl_id, template_id, .. } = unit.kind else { return None };
    let snapshot = houses.by_tl_id.get(&tl_id).filter(|h| !h.demolished).map(|h| h.snapshot.clone());
    let name = if unit.name.is_empty() { snapshot.as_ref().map(|s| s.name.clone()).unwrap_or_default() } else { unit.name.clone() };
    let mine = snapshot.as_ref().is_some_and(|s| s.owner_id == me && me != 0);
    let mut attachments: Vec<_> = aa_net::housing_attachments(doodads, units).filter(|a| a.house_obj_id == obj_id).collect();
    // The native house list keeps placement order; the doodad store is a map, so use the object id as the stable proxy.
    attachments.sort_unstable_by_key(|a| a.doodad_obj_id);
    let decorations = attachments.len() as u32;
    let furniture = furniture_of(&attachments, tables);
    let (view_type, always_public) = tables
        .and_then(|t| t.housings.get(&template_id))
        .map_or((None, false), |t| (view_type(t.category_id), t.always_public));
    let tax = tax.filter(|t| t.tl_id == tl_id).or_else(|| houses.by_tl_id.get(&tl_id).and_then(|h| h.tax.clone()));
    Some(Current { obj_id, tl_id, design: template_id, snapshot, name, mine, tax, decorations, furniture, view_type, always_public })
}

#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct Pointer<'w, 's> {
    mouse: Res<'w, ButtonInput<MouseButton>>,
    over_ui: Res<'w, HudPointerOverUi>,
    windows: Query<'w, 's, &'static Window, With<PrimaryWindow>>,
    cameras: Query<'w, 's, (&'static Camera, &'static GlobalTransform), (With<Camera3d>, Without<crate::hud::ModelViewCamera>)>,
}

impl Pointer<'_, '_> {
    /// The view ray of a right click released over the 3D view.
    fn right_click(&self, pressed_at: &mut Option<Vec2>, blocked: bool) -> Option<Ray3d> {
        if blocked { *pressed_at = None; return None; }
        let cursor = self.windows.single().ok()?.cursor_position();
        if self.mouse.just_pressed(MouseButton::Right) {
            *pressed_at = if self.over_ui.0 { None } else { cursor };
        }
        if !self.mouse.just_released(MouseButton::Right) {
            return None;
        }
        let (start, end) = (pressed_at.take()?, cursor?);
        if start.distance(end) > CLICK_SLOP || self.over_ui.0 {
            return None;
        }
        let (camera, transform) = self.cameras.iter().find(|(c, _)| c.is_active)?;
        camera.viewport_to_world(transform, end).ok()
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn sync(
    rt: Option<NonSend<UiRuntime>>,
    bridge: NonSend<HudBridge>,
    time: Res<Time>,
    units: Res<Units>,
    houses: Res<Houses>,
    doodads: Res<Doodads>,
    inventory: Res<Inventory>,
    shipyards: Res<Shipyards>,
    db: Option<Res<Db>>,
    drawn_yards: Option<Res<DrawnShipyards>>,
    pointer: Pointer,
    // the builder's signals are read and written here: a reader and a writer of one message conflict
    (mut builder, mut builder_signals): (ResMut<Builder>, ParamSet<(MessageWriter<BuilderSignal>, MessageReader<BuilderSignal>)>),
    mut events: MessageReader<NetEvent>,
    mut net: MessageWriter<NetCommand>,
    mut pressed_at: Local<Option<Vec2>>,
) {
    let Some(rt) = rt else {
        events.clear();
        builder_signals.p1().clear();
        return;
    };
    let Some(state) = rt.lua.app_data_ref::<State>().map(|s| s.clone()) else {
        events.clear();
        builder_signals.p1().clear();
        return;
    };
    let (me, pos, money, zone) = {
        let d = bridge.0.borrow();
        (d.character_id, d.pos, d.snapshot.money, d.snapshot.zone.clone())
    };
    let blocked = super::blocks_look_pointer(&bridge.0.borrow(), MouseButton::Right);
    let picked = pointer
        .right_click(&mut pressed_at, blocked)
        .and_then(|ray| pick(ray, &units, drawn_yards.as_deref()));
    let mut v = state.0.borrow_mut();
    let mut last_tax = v.last_tax.take();
    v.pointer_picked = picked;
    v.me = me;
    v.money = money;
    v.zone = zone;
    let count = |t: u32| inventory.bag.iter().flatten().filter(|i| i.template_id == t).map(|i| i64::from(i.count)).sum::<i64>();
    v.tax_certificates = count(TAX_CERTIFICATE) + count(BOUND_TAX_CERTIFICATE);
    v.appraisals = count(APPRAISAL_CERTIFICATE);
    // Raise `CHANGE_ACTABILITY_DECO_NUM` (native 0x23c) when the received actability points change.
    // `FUN_3907BA10`, which raises 0x23c, only resets the struct at `DAT_39aff990`; it does not prove
    // the trigger. The refresh-on-change here is capture-pending (C7).
    let actability = {
        let d = bridge.0.borrow();
        crate::hud::crafting::actability_signature(&d)
    };
    if v.actability != actability {
        v.actability = actability;
        v.signals.push(Signal::ChangeActability);
    }

    // the builder: its steps, the build window's buttons, the dev hooks
    for signal in builder_signals.p1().read() {
        match signal {
            BuilderSignal::Step => v.signals.push(Signal::BuilderStep),
            BuilderSignal::End => v.signals.push(Signal::BuilderEnd),
            BuilderSignal::Refused(name) => v.notices.push(Notice { name, arg: None }),
            BuilderSignal::Locked(seconds) => v.notices.push(Notice { name: "CANNOT_LOCATE_DOODAD_NOW", arg: Some(seconds.to_string()) }),
        }
    }
    if let Some((x, y, yaw)) = v.pin.take() {
        builder.pinned = Some((Vec2::new(x, y), yaw));
    }
    builder.clicks += std::mem::take(&mut v.clicks);
    if std::mem::take(&mut v.builder_next) {
        builder.commit(time.elapsed_secs(), &mut net, &mut builder_signals.p0());
    if builder.build.is_none() {
            crate::ingame::housing::placement::rotation_mode(Some(&bridge), false);
        }
    }
    if std::mem::take(&mut v.builder_prev) {
        let changes = builder.build.as_ref().is_some_and(|b| {
            matches!(
                b.stage,
                crate::ingame::housing::placement::Stage::Quote
                    | crate::ingame::housing::placement::Stage::Confirm
            )
        });
        builder.back_to_position(&mut builder_signals.p0());
    if changes {
            crate::ingame::housing::placement::rotation_mode(Some(&bridge), false);
        }
    }

    // the server
    let open = v.current.as_ref().map(|c| (c.obj_id, c.tl_id));
    for NetEvent(event) in events.read() {
        let Event::Housing(e) = event else {
            if let (Event::UnitNameChanged { obj_id, .. }, Some((open_obj, _))) = (event, open)
                && *obj_id == open_obj
            {
                v.signals.push(Signal::NameChanged);
            }
            continue;
        };
        let ours = |tl: &u16| open.is_some_and(|(_, t)| t == *tl);
        match e {
            HousingEvent::ConstructHouseTax(q) => {
                if builder.quoted(q.design_id, q.total_tax_money_amount) {
                    crate::ingame::housing::placement::rotation_mode(Some(&bridge), false);
                    v.signals.push(Signal::BuildInfo(q.clone()));
                }
            }
            HousingEvent::TaxInfo(info) if ours(&info.tl_id) => {
                last_tax = Some(info.clone());
                v.signals.push(Signal::Tax);
            }
            HousingEvent::BuildProgress { tl_id, .. } if ours(tl_id) => v.signals.push(Signal::Step),
            HousingEvent::PermissionChanged { tl_id, .. } if ours(tl_id) => v.signals.push(Signal::Permission),
            HousingEvent::RecoverToggle { tl_id, .. } | HousingEvent::OwnerNameChanged { tl_id, .. } if ours(tl_id) => v.signals.push(Signal::InfoUpdated),
            HousingEvent::SetForSale(sale) if ours(&sale.tl_id) => v.signals.extend([Signal::SetSell, Signal::InfoUpdated]),
            HousingEvent::ResetForSale { tl_id, .. } if ours(tl_id) => v.signals.extend([Signal::CancelSell, Signal::InfoUpdated]),
            HousingEvent::Sold { tl_id, new_owner_id, house_name, .. } => {
                if *new_owner_id == me && me != 0 {
                    v.signals.push(Signal::Bought);
                } else if ours(tl_id) {
                    v.signals.push(Signal::Sold(house_name.clone()));
                }
            }
            HousingEvent::State(s) if ours(&s.tl_id) => v.signals.push(Signal::InfoUpdated),
            _ => {}
        }
    }

    let game = bridge.0.borrow().game.clone();
    interact_and_refresh(
        &mut v,
        None,
        (&units, &houses, &doodads, &inventory),
        (pos, me),
        &mut last_tax,
        &game,
        &shipyards,
        db.as_deref(),
        drawn_yards.as_deref(),
    );
    v.last_tax = last_tax;
    drop(v);
    flush_view(&rt, &state, &mut net);
}

fn interact_and_refresh(
    v: &mut View,
    picked: Option<u32>,
    (units, houses, doodads, inventory): (&Units, &Houses, &Doodads, &Inventory),
    (pos, me): (Vec3, u32),
    last_tax: &mut Option<HouseTaxInfo>,
    game: &SharedGameData,
    shipyards: &Shipyards,
    db: Option<&Db>,
    drawn: Option<&DrawnShipyards>,
) {
    // the interaction: a right click or the dev hook on a house in reach
    let picked = v.interact.take().or(picked);
    let mut started = None;
    if let Some(obj_id) = picked
        && let Some(unit) = units.get(obj_id)
        && let UnitKind::Housing {
            tl_id, template_id, ..
        } = unit.kind
    {
        let dist = Vec2::new(unit.pos.x - pos.x, unit.pos.y - pos.y).length();
        if dist > garden(template_id) + REACH {
            info!(
                "hud: house {obj_id} {:?} is too far away ({dist:.1} m)",
                unit.name
            );
        } else {
            if v.current.as_ref().is_none_or(|c| c.obj_id != obj_id) {
                *last_tax = None;
                v.commands
                    .push(Command::House(HouseCommand::RequestTax { tl: tl_id }));
                v.shipyard = None;
                started = Some(obj_id);
            }
            // a house under construction: the build action of its stage when the bag has the material
            let snapshot = houses.by_tl_id.get(&tl_id).map(|h| &h.snapshot);
            if let (Some(s), Some(design)) = (
                snapshot.filter(|s| s.all_actions > 0 && s.current_actions < s.all_actions),
                house_data::get().and_then(|d| d.designs.get(&template_id)),
            ) && let Some(action) = build_action(design, s.current_actions, inventory)
            {
                let mut cast = true;
                if let Some(material) = action.material {
                    if !material.in_bag {
                        let name = game
                            .get()
                            .and_then(|g| g.items.get(&material.item))
                            .map(|i| i.named.name.clone())
                            .unwrap_or_default();
                        v.notices.push(Notice { name: "NOT_ENOUGH_REQUIRED_ITEM", arg: Some(name) });
                        // the local check looks at the bag only; an equipped backpack still casts
                        cast = material.equipped;
                    }
                }
                if cast {
                    info!(
                        "hud: build action {} on house {obj_id} ({} of {} done)",
                        action.skill, s.current_actions, s.all_actions
                    );
                    v.commands.push(Command::UseSkill {
                        skill_id: action.skill,
                        target_obj_id: obj_id,
                    });
                }
            }
        }
    }
    if let Some(obj_id) = picked
        && let Some(unit) = units.get(obj_id)
        && let UnitKind::Shipyard { id, template_id } = unit.kind
        && let Some(state) = shipyards.of(id)
    {
        let (base, remain) = db
            .and_then(|d| d.shipyard_template(template_id))
            .map_or((0, 0), |t| crate::ingame::shipyard::progress(&t, state));
        v.shipyard = Some(CurrentShipyard {
            obj_id,
            id,
            owner_name: state.owner_name.clone(),
            base,
            remain,
        });
        // switching from a house closes its window, as the house-to-house switch below does
        if v.current.take().is_some() {
            *last_tax = None;
            v.signals.insert(0, Signal::End);
        }
    }
    if let Some(obj_id) = started {
        let had = v.current.is_some();
        let tables = game.get().map(|g| &g.housing);
        v.current = current_of(obj_id, units, houses, doodads, me, None, tables);
        if had {
            v.signals.insert(0, Signal::End);
        }
        v.signals.push(Signal::Start);
        info!("hud: interacting with house {obj_id}");
    } else if let Some(obj_id) = v.current.as_ref().map(|c| c.obj_id) {
        // walking away or the house going ends it; else the facts follow the server
        let near = units
            .get(obj_id)
            .filter(|u| matches!(u.kind, UnitKind::Housing { .. }))
            .is_some_and(|u| {
                let garden = if let UnitKind::Housing { template_id, .. } = u.kind {
                    garden(template_id)
                } else {
                    0.0
                };
                Vec2::new(u.pos.x - pos.x, u.pos.y - pos.y).length() <= garden + LEAVE
            });
        let next = if near {
            let tables = game.get().map(|g| &g.housing);
            current_of(obj_id, units, houses, doodads, me, last_tax.clone(), tables)
        } else {
            None
        };
        match next {
            Some(next) => {
                let deco = v
                    .current
                    .as_ref()
                    .is_some_and(|c| c.decorations != next.decorations);
                if deco {
                    v.signals.push(Signal::Deco);
                }
                v.current = Some(next);
            }
            None => {
                v.current = None;
                *last_tax = None;
                v.signals.push(Signal::End);
            }
        }
    }
    if let Some(current) = v.shipyard.as_mut() {
        let yard = current.obj_id;
        // the same hull half-size as the pick, taken to its farthest corner: walking away is radial
        let half = drawn
            .and_then(|d| d.bounds(yard))
            .map(half_extents)
            .unwrap_or(Vec2::splat(SHIPYARD_DEFAULT_HALF));
        let radius = half.length();
        let near = units
            .get(yard)
            .filter(|u| matches!(u.kind, UnitKind::Shipyard { .. }))
            .is_some_and(|u| {
                Vec2::new(u.pos.x - pos.x, u.pos.y - pos.y).length() <= radius + LEAVE
            });
        if !near {
            v.shipyard = None;
        } else if let Some(state) = shipyards.of(current.id) {
            current.owner_name = state.owner_name.clone();
            if let Some(UnitKind::Shipyard { template_id, .. }) = units.get(yard).map(|u| u.kind.clone())
                && let Some(template) = db.and_then(|d| d.shipyard_template(template_id))
            {
                (current.base, current.remain) = crate::ingame::shipyard::progress(&template, state);
            }
        }
    }
}

fn flush_view(rt: &UiRuntime, state: &State, net: &mut MessageWriter<NetCommand>) {
    let mut v = state.0.borrow_mut();
    let (signals, notices, commands) = (
        std::mem::take(&mut v.signals),
        std::mem::take(&mut v.notices),
        std::mem::take(&mut v.commands),
    );
    drop(v);
    for signal in signals {
        fire(rt, state, signal);
    }
    for item in notices {
        notice(rt, &item);
    }
    for command in commands {
        net.write(NetCommand(command));
    }
}

#[allow(clippy::too_many_arguments)]
fn pointer_interaction(
    rt: Option<NonSend<UiRuntime>>,
    bridge: NonSend<HudBridge>,
    units: Res<Units>,
    houses: Res<Houses>,
    doodads: Res<Doodads>,
    inventory: Res<Inventory>,
    shipyards: Res<Shipyards>,
    db: Option<Res<Db>>,
    drawn_yards: Option<Res<DrawnShipyards>>,
    mut net: MessageWriter<NetCommand>,
) {
    let Some(rt) = rt else { return };
    let Some(state) = rt.lua.app_data_ref::<State>().map(|s| s.clone()) else {
        return;
    };
    let picked = state.0.borrow_mut().pointer_picked.take();
    let (blocked, pos, me) = {
        let data = bridge.0.borrow();
        (
            super::blocks_look_pointer(&data, MouseButton::Right),
            data.pos,
            data.character_id,
        )
    };
    if blocked || picked.is_none() {
        return;
    }
    let game = bridge.0.borrow().game.clone();
    let mut v = state.0.borrow_mut();
    let mut last_tax = v.last_tax.take();
    interact_and_refresh(
        &mut v,
        picked,
        (&units, &houses, &doodads, &inventory),
        (pos, me),
        &mut last_tax,
        &game,
        &shipyards,
        db.as_deref(),
        drawn_yards.as_deref(),
    );
    v.last_tax = last_tax;
    drop(v);
    flush_view(&rt, &state, &mut net);
}

/// The map icon of a house design (`markers.lua` SINGLES): the fish farm category (7, "양식장"), the farm and
/// scarecrow garden categories (16 to 18), a house otherwise.
fn map_icon_kind(category_id: u32) -> &'static str {
    match category_id {
        7 => "housing_fishfarm",
        16..=18 => "housing_farm",
        _ => "housing_house",
    }
}

/// The player's own houses on the map: (x, y, icon, name), wherever they are (the server sends them at login).
pub(super) fn own_house_icons(houses: &Houses, me: u32) -> Vec<(f32, f32, &'static str, String)> {
    let data = crate::ingame::housing::data::get();
    let mut out: Vec<_> = houses
        .by_tl_id
        .values()
        .filter(|h| !h.demolished && me != 0 && h.snapshot.owner_id == me)
        .map(|h| {
            let category = data.and_then(|d| d.designs.get(&h.snapshot.template_id)).map_or(0, |d| d.category_id);
            let pos = h.snapshot.position();
            (pos.x, pos.y, map_icon_kind(category), h.snapshot.name.clone())
        })
        .collect();
    out.sort_by(|a, b| a.3.cmp(&b.3).then(a.0.total_cmp(&b.0)));
    out
}

pub fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        sync.after(super::sync_hud).before(aa_ui::UiFrame).run_if(in_state(Screen::InGame)).run_if(super::hud_up),
    ).add_systems(Update, pointer_interaction.after(super::PointerFinished).after(sync).run_if(in_state(Screen::InGame)).run_if(super::hud_up));
}

#[allow(dead_code)]
pub(super) mod fixture;
#[cfg(test)]
mod tests;
