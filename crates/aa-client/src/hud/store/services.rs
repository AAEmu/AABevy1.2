//! Original NPC service windows. Received offers and selection belong to dialog::npc.
//! Catalog flags below describe prices/capabilities; they do not advertise services.

use std::collections::HashSet;

use aa_net::shop::ShopCommand;
use aa_net::state::{Player, Units};
use aa_net::{Command, NetCommand};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use mlua::{Lua, Table};
use rusqlite::Connection;

use super::super::api::{HudBridge, HudData, Shared};
use super::super::data::{GameData, any_flag, any_int};
use super::super::dialog::{self, NpcScope, SelectedNpc};
use super::super::formula::Formula;

pub const ABILITY: u32 = 12082;
pub const AUCTION: u32 = 12083;
pub const BLESSING: u32 = 12084;
pub const REPAIR: u32 = 12086;
pub const STORE: u32 = 12087;
pub const RECRUITER: u32 = 12660;
pub const STABLER: u32 = 12195;
pub const EXPEDITION: u32 = 13736;
pub const BANK: u32 = 13238;
pub const PRAYER: u32 = 17063;
pub const LOOK_CONVERTER: u32 = 23478;

/// How far a service window may be from its NPC before it closes, metres (the lane serves repairs, blessings and
/// prayers within 5 m).
const REACH: f32 = 5.0;
/// The `content_configs` row of the ability change price per level (copper), x2game's `FUN_395720e0(0x27)`.
const ABILITY_COST_CONFIG: i64 = 39;
/// The `formulas` rows of the prayer's money and labor price (`MoneyForRecoverExp`, `LaborPowerForRecoverExp`).
const RECOVER_MONEY_FORMULA: i64 = 6;
const RECOVER_LABOR_FORMULA: i64 = 7;

/// The name `npc_interaction.lua` knows the window of a service by, for the services the client opens itself.
fn window(skill: u32) -> Option<(&'static str, &'static str)> {
    Some(match skill {
        // the original names the repair window BLACK_SMITH
        REPAIR => ("blacksmith", "repairWindow"),
        BLESSING => ("priest", "GetDivineBlessingMessageBoxHandleWindow()"),
        PRAYER => (
            "priest_recover_exp",
            "GetRecoverExpMessageBoxHandleWindow()",
        ),
        ABILITY => ("ability_change", "abilityChangeFrame"),
        LOOK_CONVERTER => ("look_converter", "changeItemLookWindow"),
        _ => None?,
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Buff {
    id: u32,
    /// Copper per level of the player.
    cost: i64,
}

#[derive(Default)]
pub struct Tables {
    priest: HashSet<u32>,
    /// `npcs.blacksmith` or `repairman`: what the lane accepts repairs at.
    repair: HashSet<u32>,
    ability: HashSet<u32>,
    look_converter: HashSet<u32>,
    buffs: Vec<Buff>,
    ability_cost: i64,
    recover_money: Option<Formula>,
    recover_labor: Option<Formula>,
}

impl Tables {
    pub fn load(conn: &Connection) -> rusqlite::Result<Self> {
        let mut out = Self::default();
        let mut q = conn.prepare(
            "SELECT id,priest,blacksmith,repairman,ability_changer,look_converter FROM npcs",
        )?;
        let mut rows = q.query([])?;
        while let Some(r) = rows.next()? {
            let id = any_int(r, 0)? as u32;
            if any_flag(r, 1)? {
                out.priest.insert(id);
            }
            if any_flag(r, 2)? || any_flag(r, 3)? {
                out.repair.insert(id);
            }
            if any_flag(r, 4)? {
                out.ability.insert(id);
            }
            if any_flag(r, 5)? {
                out.look_converter.insert(id);
            }
        }
        let mut q = conn.prepare("SELECT id,cost FROM priest_buffs ORDER BY id")?;
        let mut rows = q.query([])?;
        while let Some(r) = rows.next()? {
            out.buffs.push(Buff {
                id: any_int(r, 0)? as u32,
                cost: any_int(r, 1)?,
            });
        }
        let mut q = conn.prepare("SELECT value FROM content_configs WHERE id=?1")?;
        out.ability_cost = q
            .query_row([ABILITY_COST_CONFIG], |r| any_int(r, 0))
            .unwrap_or(0);
        let mut q = conn.prepare("SELECT id,formula FROM formulas WHERE id IN (?1,?2)")?;
        let mut rows = q.query([RECOVER_MONEY_FORMULA, RECOVER_LABOR_FORMULA])?;
        while let Some(r) = rows.next()? {
            let formula = Formula::parse(&r.get::<_, String>(1)?).ok();
            match any_int(r, 0)? {
                RECOVER_MONEY_FORMULA => out.recover_money = formula,
                _ => out.recover_labor = formula,
            }
        }
        Ok(out)
    }
}

/// The services NPC template `npc` offers, in the order of the lane's `if / else if` chain.
pub fn offers(game: &GameData, npc: u32) -> Vec<u32> {
    let (stores, t) = (&game.stores, &game.stores.services);
    let mut out = Vec::new();
    let mut add = |has: bool, skills: &[u32]| {
        if has {
            out.extend_from_slice(skills);
        }
    };
    add(stores.bankers.contains(&npc), &[BANK]);
    add(t.ability.contains(&npc), &[ABILITY]);
    add(stores.auctioneers.contains(&npc), &[AUCTION]);
    add(t.priest.contains(&npc), &[BLESSING, PRAYER]);
    add(t.repair.contains(&npc), &[REPAIR]);
    add(stores.merchants.contains_key(&npc), &[STORE]);
    add(game.pets.stables.contains(&npc), &[STABLER]);
    add(game.expedition.is_creator(npc), &[EXPEDITION]);
    add(game.battlefield.field(npc).is_some(), &[RECRUITER]);
    add(t.look_converter.contains(&npc), &[LOOK_CONVERTER]);
    out
}

/// The player's services state: the list that is up, the window that is up and what the scripts asked for.
#[derive(Default, Debug)]
pub struct View {
    menu: Option<(NpcScope, Vec<(u32, String)>)>,
    picked: Vec<(NpcScope, i64)>,
    cancelled: Option<NpcScope>,
    active: Option<(NpcScope, u32)>,
    requests: Vec<(NpcScope, Request)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Request {
    Bless(usize),
    Pray,
}

impl View {
    pub fn serving(&self, npc: (u32, u32)) -> bool {
        self.active.is_some_and(|a| a.0.npc() == npc)
    }
}

pub(in crate::hud) fn allows(data: &HudData, skill: u32) -> Option<NpcScope> {
    let (scope, action) = data.store.services.active?;
    (action == skill && dialog::scope_current(data, scope)).then_some(scope)
}

pub(in crate::hud) fn record_offer(data: &mut HudData, scope: NpcScope, skills: &[u32]) {
    let rows = skills
        .iter()
        .map(|id| {
            (
                *id,
                data.game
                    .get()
                    .and_then(|g| g.skills.get(id))
                    .map_or_else(String::new, |s| s.named.name.clone()),
            )
        })
        .collect();
    data.store.services.menu = Some((scope, rows));
}

/// What the scripts read of the player and the price lists: `HUD.service_info()`.
fn info(lua: &Lua, d: &HudData) -> mlua::Result<Table> {
    let out = lua.create_table()?;
    let level = i64::from(d.snapshot.level);
    let Some(game) = d.game.get() else {
        return Ok(out);
    };
    let t = &game.stores.services;
    let vars = std::collections::HashMap::from([
        ("pc_level", level as f64),
        ("level", level as f64),
        ("experience", f64::from(d.snapshot.recoverable_exp)),
        ("recoverable_exp", f64::from(d.snapshot.recoverable_exp)),
    ]);
    let price = |f: &Option<Formula>| f.as_ref().map_or(0.0, |f| f.eval(&vars).max(0.0).floor());
    let buffs = lua.create_table()?;
    for (i, b) in t.buffs.iter().enumerate() {
        let e = lua.create_table()?;
        e.set("id", b.id)?;
        // the cost the client works out: the player's level times the row's cost
        e.set("cost", level * b.cost)?;
        buffs.set(i + 1, e)?;
    }
    out.set("buffs", buffs)?;
    out.set("money", d.snapshot.money.to_string())?;
    out.set("recoverable", d.snapshot.recoverable_exp)?;
    out.set("recover_money", price(&t.recover_money))?;
    out.set("recover_labor", price(&t.recover_labor))?;
    out.set("ability_cost", level * t.ability_cost)?;
    Ok(out)
}

pub fn install(lua: &Lua, hud: &Table, bridge: &Shared) -> mlua::Result<()> {
    let b = bridge.clone();
    hud.set(
        "service_info",
        lua.create_function(move |lua, ()| info(lua, &b.borrow()))?,
    )?;
    let b = bridge.clone();
    hud.set(
        "service_request",
        lua.create_function(move |_, (what, n): (String, Option<usize>)| {
            let request = match what.as_str() {
                "bless" => Request::Bless(n.unwrap_or(0)),
                "pray" => Request::Pray,
                _ => return Ok(()),
            };
            let mut data = b.borrow_mut();
            let skill = if matches!(request, Request::Bless(_)) {
                BLESSING
            } else {
                PRAYER
            };
            if let Some(scope) = allows(&data, skill) {
                data.store.services.requests.push((scope, request));
            }
            Ok(())
        })?,
    )?;
    let b = bridge.clone();
    hud.set(
        "npc_menu",
        lua.create_function(move |lua, ()| {
            let d = b.borrow();
            let Some((scope, rows)) = &d.store.services.menu else {
                return Ok(None);
            };
            if d.doodad.has_menu() || !dialog::npc_menu_current(&d, *scope) {
                return Ok(None);
            }
            let out = lua.create_table()?;
            for (i, (id, name)) in rows.iter().enumerate() {
                let row = lua.create_table()?;
                row.set("id", *id)?;
                row.set("name", name.as_str())?;
                out.set(i + 1, row)?;
            }
            Ok(Some(out))
        })?,
    )?;
    let b = bridge.clone();
    hud.set(
        "npc_menu_pick",
        lua.create_function(move |_, id: i64| {
            let mut data = b.borrow_mut();
            if let Some((scope, _)) = data
                .store
                .services
                .menu
                .as_ref()
                .filter(|(scope, _)| dialog::npc_menu_current(&data, *scope))
            {
                let scope = *scope;
                data.store.services.picked.push((scope, id));
            }
            Ok(())
        })?,
    )?;
    let b = bridge.clone();
    hud.set(
        "npc_menu_cancel",
        lua.create_function(move |_, ()| {
            let mut data = b.borrow_mut();
            data.store.services.cancelled = dialog::active_scope(&data);
            Ok(())
        })?,
    )?;
    Ok(())
}

/// The natives of the services' windows, and the interaction list over the doodad one: loaded after `doodad::install`,
/// whose natives it wraps.
pub fn load_script(lua: &Lua) -> mlua::Result<()> {
    lua.load(include_str!("services.lua"))
        .set_name("@services.lua")
        .exec()
}

/// Is the Lua window expression visible?
fn visible(rt: &UiRuntime, expr: &str) -> bool {
    rt.lua
        .load(format!(
            "local w = {expr}; return w ~= nil and w:IsVisible() == true"
        ))
        .eval::<bool>()
        .unwrap_or(false)
}

fn near(d: &HudData, units: &Units, npc: (u32, u32), reach: f32) -> bool {
    units
        .get(npc.0)
        .is_some_and(|u| !u.dead && d.pos.distance(Vec3::new(u.pos.x, u.pos.y, u.pos.z)) <= reach)
}

/// Consume only scoped GUI choices/commands; never publish a synthetic receipt.
pub fn update(
    rt: Option<NonSend<UiRuntime>>,
    bridge: NonSend<HudBridge>,
    player: Res<Player>,
    units: Res<Units>,
    mut net: MessageWriter<NetCommand>,
) {
    let Some(rt) = rt else { return };
    let Some(spawn) = player.spawn.as_ref() else {
        return;
    };
    let active = bridge.0.borrow().store.services.active;
    if let Some((scope, skill)) = active {
        let valid = {
            let data = bridge.0.borrow();
            dialog::scope_current(&data, scope) && near(&data, &units, scope.npc(), REACH)
        };
        let shown = window(skill).is_some_and(|(_, expr)| visible(&rt, expr));
        if !valid || !shown {
            let mut data = bridge.0.borrow_mut();
            data.store.services.active = None;
            dialog::close_npc(&mut data, scope);
        }
    }
    let mut data = bridge.0.borrow_mut();
    for (scope, id) in std::mem::take(&mut data.store.services.picked) {
        if let Ok(skill) = u32::try_from(id) {
            dialog::choose_npc(&mut data, scope, skill);
        }
    }
    if let Some(scope) = data.store.services.cancelled.take() {
        dialog::close_npc(&mut data, scope);
    }
    for (scope, request) in std::mem::take(&mut data.store.services.requests) {
        if !dialog::scope_current(&data, scope) || data.character_id != spawn.character().id {
            continue;
        }
        match request {
            Request::Bless(n) if allows(&data, BLESSING) == Some(scope) => {
                let Some(buff) = data
                    .game
                    .get()
                    .and_then(|g| g.stores.services.buffs.get(n.wrapping_sub(1)).copied())
                else {
                    continue;
                };
                if data.snapshot.money < i64::from(data.snapshot.level) * buff.cost {
                    continue;
                }
                net.write(NetCommand(Command::Shop(ShopCommand::BuyPriestBuff {
                    priest_buff_id: buff.id,
                    npc_obj_id: scope.npc().0,
                })));
            }
            Request::Pray if allows(&data, PRAYER) == Some(scope) => {
                net.write(NetCommand(Command::UseSkill {
                    skill_id: PRAYER,
                    target_obj_id: spawn.obj_id,
                }));
            }
            _ => {}
        }
    }
}

fn hide_list(rt: &UiRuntime) {
    if let Ok(f) = rt.lua.globals().get::<mlua::Function>("HideAllButtons") {
        let _ = f.call::<()>(());
    }
}

/// A registered local callback attempt, selected from the actual received vector.
pub(in crate::hud) fn start_selected(
    rt: &UiRuntime,
    bridge: &Shared,
    selected: SelectedNpc,
) -> bool {
    let Some((name, _)) = window(selected.skill()) else {
        return false;
    };
    {
        let mut data = bridge.borrow_mut();
        if !dialog::scope_current(&data, selected.scope()) {
            return false;
        }
        data.store.services.active = Some((selected.scope(), selected.skill()));
        data.store.services.requests.clear();
    }
    rt.fire_event(
        "NPC_INTERACTION_START",
        (name, "", selected.npc().0.to_string()),
    );
    true
}

#[cfg(test)]
mod tests {
    use super::super::{Good, Tables as Stores};
    use super::*;

    fn game() -> GameData {
        let mut game = GameData::default();
        let mut stores = Stores::default();
        stores.merchants.insert(
            10,
            vec![Good {
                item: 1,
                grade: 0,
                currency: 0,
            }],
        );
        stores.merchants.insert(
            11,
            vec![Good {
                item: 1,
                grade: 0,
                currency: 0,
            }],
        );
        stores.services.repair.insert(10);
        stores.services.repair.insert(12);
        stores.services.priest.insert(20);
        stores.services.ability.insert(20);
        stores.bankers.insert(30);
        stores.services.look_converter.extend([10, 40]);
        game.stores = stores;
        game
    }

    #[test]
    fn catalog_flags_classify_capabilities_without_advertising_received_services() {
        let game = game();
        assert_eq!(offers(&game, 10), vec![REPAIR, STORE, LOOK_CONVERTER]);
        assert_eq!(offers(&game, 20), vec![ABILITY, BLESSING, PRAYER]);
        assert_eq!(offers(&game, 40), vec![LOOK_CONVERTER]);
        assert_eq!(offers(&game, 99), Vec::<u32>::new());
    }

    #[test]
    fn tables_read_the_flags_prices_and_formulas() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE npcs(id,priest,blacksmith,repairman,ability_changer,look_converter);
             INSERT INTO npcs VALUES(1,'t','f','f','t','f'),(2,'f','t','f','f','t'),(3,'f','f','t','f','f'),(4,'f','f','f','f','f');
             CREATE TABLE priest_buffs(id,buff_id,cost,position); INSERT INTO priest_buffs VALUES(1,239,100,1);
             CREATE TABLE content_configs(id,kind_id,value); INSERT INTO content_configs VALUES(39,12,200),(38,12,1000);
             CREATE TABLE formulas(id,formula); INSERT INTO formulas VALUES(6,'0'),(7,'if_negative(pc_level - 51, pc_level, 50 + (pc_level - 50) * 20)');",
        )
        .unwrap();
        let t = Tables::load(&db).unwrap();
        assert_eq!((t.priest.len(), t.repair.len(), t.ability.len()), (1, 2, 1));
        assert_eq!(t.look_converter, HashSet::from([2]));
        assert_eq!(t.buffs, vec![Buff { id: 1, cost: 100 }]);
        assert_eq!(t.ability_cost, 200);
        let vars = |level: f64| std::collections::HashMap::from([("pc_level", level)]);
        let labor = t.recover_labor.as_ref().unwrap();
        assert_eq!(
            (labor.eval(&vars(30.0)), labor.eval(&vars(55.0))),
            (30.0, 150.0)
        );
    }

    #[test]
    fn converter_service_opens_and_closes_the_original_npc_window()
    -> Result<(), Box<dyn std::error::Error>> {
        let (rt, shared, path, _) = crate::hud::expansion::fixture::runtime(1280., 720.)?;
        let bridge = HudBridge(shared);
        let player = Player {
            spawn: Some(crate::hud::expansion::fixture::spawn()),
        };
        let mut unit = player.spawn.as_ref().unwrap().unit.clone();
        unit.obj_id = 200;
        unit.kind = aa_net::model::UnitKind::Npc {
            template_id: 1063,
            owner_id: 0,
        };
        let selected = crate::hud::dialog::fixture::selected(
            &mut bridge.0.borrow_mut(),
            &player,
            &unit,
            LOOK_CONVERTER,
        );
        assert!(start_selected(&rt, &bridge.0, selected));
        rt.lua.load("assert(changeItemLookWindow and changeItemLookWindow:IsVisible());assert(X2Item:IsInItemLookConvertMode());assert(npcInteraction.interactionWindows[16]~=nil)").exec()?;
        assert_eq!(
            bridge.0.borrow().store.services.active,
            Some((selected.scope(), LOOK_CONVERTER))
        );
        rt.fire_event("NPC_INTERACTION_END", ());
        rt.lua.load("assert(not X2Item:IsInItemLookConvertMode());assert(changeItemLookWindow==nil or not changeItemLookWindow:IsVisible())").exec()?;
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }

    #[test]
    fn the_natives_answer_from_the_price_lists() {
        let lua = Lua::new();
        let mut data = HudData::default();
        let mut stores = Stores::default();
        stores.services.buffs = vec![Buff { id: 1, cost: 100 }];
        stores.services.ability_cost = 200;
        let mut game = GameData::default();
        game.stores = stores;
        assert!(data.game.0.set(game).is_ok());
        data.snapshot.level = 30;
        data.snapshot.money = 4999;
        let shared = std::rc::Rc::new(std::cell::RefCell::new(data));
        let hud = lua.create_table().unwrap();
        install(&lua, &hud, &shared).unwrap();
        lua.globals().set("HUD", hud).unwrap();
        lua.load(
            "X2Priest={}; X2Ability={}; X2Interaction={}; CURRENCY_GOLD=0; HUD.request=function() end
             function HideAllButtons() end",
        )
        .exec()
        .unwrap();
        lua.load(include_str!("services.lua")).exec().unwrap();
        lua.load(
            "assert(X2Priest:GetPriestBuffCount()==1 and X2Priest:GetPriestBuff(1).cost==3000)
             assert(X2Priest:GetCostForRecoverExp()==nil)
             assert(X2Ability:GetAbilityChangeCost()==6000)
             assert(X2Ability:CanBuyAbilityChange()==nil)
             X2Priest:BuyPriestBuff(1); X2Priest:RecoverExp()
             assert(X2Ability:GetCurrencyForAbilityChange()==0)",
        )
        .exec()
        .unwrap();
        assert!(
            shared.borrow().store.services.requests.is_empty(),
            "price getters do not establish an NPC service lifetime"
        );
    }
}
