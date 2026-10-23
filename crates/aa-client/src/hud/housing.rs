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
use aa_net::model::UnitKind;
use aa_net::state::{Doodads, Inventory, Units};
use aa_net::{Command, Event, Houses, NetCommand, NetEvent};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use mlua::{Lua, ObjectLike, Table, Value};

use super::api::{HudBridge, Shared};
use super::input::HudPointerOverUi;
use crate::Screen;
use crate::ingame::housing::data::{self as house_data, Design};
use crate::ingame::housing::placement::{Builder, BuilderSignal};

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
/// A press and release further apart than this (logical pixels) is a drag, not a click.
const CLICK_SLOP: f32 = 6.0;

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
    decorations: u32,
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
    notices: Vec<String>,
    builder_next: bool,
    builder_prev: bool,
    /// Dev hooks: a scripted interaction, a pinned builder spot and clicks.
    interact: Option<u32>,
    pin: Option<(f32, f32, f32)>,
    clicks: u32,
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
        Value::Integer(n) => *n,
        Value::Number(n) => *n as i64,
        Value::String(s) => s.to_str().ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0),
        _ => 0,
    }
}

fn copper(gold: &Value, silver: &Value, copper: &Value) -> i64 {
    amount(gold) * 10_000 + amount(silver) * 100 + amount(copper)
}

/// The stage of a construction `actions` build actions in, and whether that stage's material is in the bag.
fn build_action(design: &Design, actions: i32, inventory: &Inventory) -> Option<(u32, bool)> {
    let step = design.steps.get(design.step_at(actions)?)?;
    let have = step.item == 0 || inventory.bag.iter().flatten().filter(|i| i.template_id == step.item).map(|i| i64::from(i.count)).sum::<i64>() >= i64::from(step.count.max(1));
    Some((step.skill, have))
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
    // shipyards are not built here; their window is never raised
    getter!("GetShipyardOwnerName", |_v| String::new());
    getter!("GetShipyardProtectionPeriod", |_v| 0);
    getter!("GetShipyardRemainProgress", |_v| 0);
    getter!("GetShipyardBaseProgress", |_v| 0);
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
    let s = state.clone();
    h.set(
        "SetHousePermission",
        lua.create_function(move |_, (_, permission): (Value, i64)| {
            let mut v = s.0.borrow_mut();
            let Some(c) = v.current.as_ref().filter(|c| c.mine) else { return Ok(()) };
            if permission != c.permission() && (1..=4).contains(&permission) {
                let tl = c.tl_id;
                v.commands.push(Command::House(HouseCommand::ChangePermission { tl, permission: (permission - 1) as u8 }));
            }
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
    // furniture: the count against the design's limit; no decoration proficiency in this data
    let (s, b) = (state.clone(), bridge.clone());
    h.set(
        "GetHouseDecoCount",
        lua.create_function(move |_, _: Value| {
            let v = s.0.borrow();
            let limit = v.current.as_ref().map_or(0, |c| template(&b, c.design, |t| t.deco_limit));
            Ok((v.current.as_ref().map_or(0, |c| c.decorations), limit))
        })?,
    )?;
    h.set("GetActabilityDecoCount", lua.create_function(|_, _: Value| Ok(0))?)?;
    h.set("GetDecoActabilityLimitGroup", lua.create_function(|lua, _: Value| lua.create_table())?)?;
    h.set(
        "GetDecoActabilityPoint",
        lua.create_function(|lua, _: Value| {
            let t = lua.create_table()?;
            t.set("decoPoint", 0)?;
            t.set("name", "")?;
            Ok(t)
        })?,
    )?;
    h.set(
        "GetDecoActabilityByType",
        lua.create_function(|lua, (_, index): (Value, i64)| {
            let t = lua.create_table()?;
            t.set("totalDecoPoint", 0)?;
            t.set("totalPoint", 0)?;
            t.set("type", index)?;
            Ok((t, lua.create_table()?))
        })?,
    )?;
    Ok(())
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
                v.notices.push("HOUSE_CANNOT_SELL_AS_TOO_LOW_PRICE".into());
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
                v.notices.push("HOUSE_CANNOT_BUY_AS_NOT_FOR_SALE".into());
            } else if i64::from(price) > v.money {
                v.notices.push("HOUSE_CANNOT_BUY_AS_NOT_ENOUGH_MONEY".into());
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

fn notice(rt: &UiRuntime, name: &str) {
    let text = super::alerts::localized(rt, "ERROR_MSG", name).unwrap_or_else(|| name.to_owned());
    rt.fire_event("SYSMSG", text);
}

fn fire(rt: &UiRuntime, state: &State, signal: Signal) {
    let lua = &rt.lua;
    match signal {
        Signal::Start => rt.fire_event("HOUSE_INTERACTION_START", ("housing", "housing")),
        Signal::End => rt.fire_event("HOUSE_INTERACTION_END", ()),
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
                    tax.weeks_without_pay.max(0),
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

/// The house a right click released over the 3D view picked: the nearest whose middle the view ray passes within its
/// garden (not a drag, not on the HUD).
fn pick(ray: Ray3d, units: &Units) -> Option<u32> {
    let data = house_data::get()?;
    let mut best: Option<(f32, u32)> = None;
    for u in units.iter() {
        let UnitKind::Housing { template_id, .. } = u.kind else { continue };
        let radius = data.designs.get(&template_id).map_or(5.0, |d| d.garden_radius.max(3.0)) * PICK_SHARE;
        // the house's middle: a few metres over its floor
        let centre = aa_world::cry_to_bevy(Vec3::new(u.pos.x, u.pos.y, u.pos.z + 3.0));
        let along = (centre - ray.origin).dot(*ray.direction);
        if along <= 0.0 {
            continue;
        }
        let miss = (ray.origin + *ray.direction * along).distance(centre);
        if miss <= radius && best.is_none_or(|(a, _)| along < a) {
            best = Some((along, u.obj_id));
        }
    }
    best.map(|(_, id)| id)
}

fn garden(template_id: u32) -> f32 {
    house_data::get().and_then(|d| d.designs.get(&template_id)).map_or(5.0, |d| d.garden_radius)
}

/// The facts of house unit `obj_id` the window reads.
fn current_of(obj_id: u32, units: &Units, houses: &Houses, doodads: &Doodads, me: u32, tax: Option<HouseTaxInfo>) -> Option<Current> {
    let unit = units.get(obj_id)?;
    let UnitKind::Housing { tl_id, template_id, .. } = unit.kind else { return None };
    let snapshot = houses.by_tl_id.get(&tl_id).filter(|h| !h.demolished).map(|h| h.snapshot.clone());
    let name = if unit.name.is_empty() { snapshot.as_ref().map(|s| s.name.clone()).unwrap_or_default() } else { unit.name.clone() };
    let mine = snapshot.as_ref().is_some_and(|s| s.owner_id == me && me != 0);
    let decorations = aa_net::housing_attachments(doodads, units).filter(|a| a.house_obj_id == obj_id).count() as u32;
    let tax = tax.filter(|t| t.tl_id == tl_id).or_else(|| houses.by_tl_id.get(&tl_id).and_then(|h| h.tax.clone()));
    Some(Current { obj_id, tl_id, design: template_id, snapshot, name, mine, tax, decorations })
}

#[derive(bevy::ecs::system::SystemParam)]
struct Pointer<'w, 's> {
    mouse: Res<'w, ButtonInput<MouseButton>>,
    over_ui: Res<'w, HudPointerOverUi>,
    windows: Query<'w, 's, &'static Window, With<PrimaryWindow>>,
    cameras: Query<'w, 's, (&'static Camera, &'static GlobalTransform), With<Camera3d>>,
}

impl Pointer<'_, '_> {
    /// The view ray of a right click released over the 3D view.
    fn right_click(&self, pressed_at: &mut Option<Vec2>) -> Option<Ray3d> {
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
fn sync(
    rt: Option<NonSend<UiRuntime>>,
    bridge: NonSend<HudBridge>,
    time: Res<Time>,
    units: Res<Units>,
    houses: Res<Houses>,
    doodads: Res<Doodads>,
    inventory: Res<Inventory>,
    pointer: Pointer,
    // the builder's signals are read and written here: a reader and a writer of one message conflict
    (mut builder, mut builder_signals): (ResMut<Builder>, ParamSet<(MessageWriter<BuilderSignal>, MessageReader<BuilderSignal>)>),
    mut events: MessageReader<NetEvent>,
    mut net: MessageWriter<NetCommand>,
    mut pressed_at: Local<Option<Vec2>>,
    mut last_tax: Local<Option<HouseTaxInfo>>,
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
    let picked = pointer.right_click(&mut pressed_at).and_then(|ray| pick(ray, &units));
    let mut v = state.0.borrow_mut();
    v.me = me;
    v.money = money;
    v.zone = zone;
    let count = |t: u32| inventory.bag.iter().flatten().filter(|i| i.template_id == t).map(|i| i64::from(i.count)).sum::<i64>();
    v.tax_certificates = count(TAX_CERTIFICATE) + count(BOUND_TAX_CERTIFICATE);
    v.appraisals = count(APPRAISAL_CERTIFICATE);

    // the builder: its steps, the build window's buttons, the dev hooks
    for signal in builder_signals.p1().read() {
        match signal {
            BuilderSignal::Step => v.signals.push(Signal::BuilderStep),
            BuilderSignal::End => v.signals.push(Signal::BuilderEnd),
            BuilderSignal::Refused(name) => v.notices.push((*name).to_string()),
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
                *last_tax = Some(info.clone());
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

    // the interaction: a right click or the dev hook on a house in reach
    let picked = v.interact.take().or(picked);
    let mut started = None;
    if let Some(obj_id) = picked
        && let Some(unit) = units.get(obj_id)
        && let UnitKind::Housing { tl_id, template_id, .. } = unit.kind
    {
        let dist = Vec2::new(unit.pos.x - pos.x, unit.pos.y - pos.y).length();
        if dist > garden(template_id) + REACH {
            info!("hud: house {obj_id} {:?} is too far away ({dist:.1} m)", unit.name);
        } else {
            if v.current.as_ref().is_none_or(|c| c.obj_id != obj_id) {
                *last_tax = None;
                v.commands.push(Command::House(HouseCommand::RequestTax { tl: tl_id }));
                started = Some(obj_id);
            }
            // a house under construction: the build action of its stage when the bag has the material
            let snapshot = houses.by_tl_id.get(&tl_id).map(|h| &h.snapshot);
            if let (Some(s), Some(design)) = (snapshot.filter(|s| s.all_actions > 0 && s.current_actions < s.all_actions), house_data::get().and_then(|d| d.designs.get(&template_id)))
                && let Some((skill, have)) = build_action(design, s.current_actions, &inventory)
            {
                if have {
                    info!("hud: build action {skill} on house {obj_id} ({} of {} done)", s.current_actions, s.all_actions);
                    v.commands.push(Command::UseSkill { skill_id: skill, target_obj_id: obj_id });
                } else {
                    v.notices.push("NOT_ENOUGH_REQUIRED_ITEM".into());
                }
            }
        }
    }
    if let Some(obj_id) = started {
        let had = v.current.is_some();
        v.current = current_of(obj_id, &units, &houses, &doodads, me, None);
        if had {
            v.signals.insert(0, Signal::End);
        }
        v.signals.push(Signal::Start);
        info!("hud: interacting with house {obj_id}");
    } else if let Some(obj_id) = v.current.as_ref().map(|c| c.obj_id) {
        // walking away or the house going ends it; else the facts follow the server
        let near = units.get(obj_id).filter(|u| matches!(u.kind, UnitKind::Housing { .. })).is_some_and(|u| {
            let garden = if let UnitKind::Housing { template_id, .. } = u.kind { garden(template_id) } else { 0.0 };
            Vec2::new(u.pos.x - pos.x, u.pos.y - pos.y).length() <= garden + LEAVE
        });
        let next = near.then(|| current_of(obj_id, &units, &houses, &doodads, me, last_tax.clone())).flatten();
        match next {
            Some(next) => {
                let deco = v.current.as_ref().is_some_and(|c| c.decorations != next.decorations);
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
    let (signals, notices, commands) = (std::mem::take(&mut v.signals), std::mem::take(&mut v.notices), std::mem::take(&mut v.commands));
    drop(v);
    for signal in signals {
        fire(&rt, &state, signal);
    }
    for name in notices {
        notice(&rt, &name);
    }
    for command in commands {
        net.write(NetCommand(command));
    }
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
    );
}

#[cfg(test)]
mod tests;
