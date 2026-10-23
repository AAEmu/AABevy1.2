//! The services an NPC offers besides what the quest dialog does, and the choice between them.
//!
//! What the 1.2 client does when the player talks to an NPC (x2game.dll, Ghidra on the unpacked DLL, see
//! `docs/aa-client-portals.md` "NPC services"): `CSStartInteraction` goes out, the server's
//! `SCNpcInteractionSkillList` (`OnNpcInteractionSkillList`, `FUN_393be850`) is kept as a list of skill ids, a "quest"
//! pseudo skill (13877) is put in front when the NPC has quest business, and `FUN_393be280` dispatches the **first** one:
//! a known service skill raises `NPC_INTERACTION_START(<name>)` for the window `npc_interaction.lua` opens, a quest
//! skill raises `NPC_INTERACTION_START("quest", "talk" | "complete" | "start", npcId)`, anything else is cast at the NPC.
//! The lane answers with a single skill (the first flag of the NPC template in its `if / else if` chain), so a temple
//! priest (ability changer, priest) or a smith (merchant, blacksmith) could only ever do one thing. The client has the
//! same NPC flags in its database, so it works out the services itself ([`offers`]); one that the lane's answer drives
//! (the store, the bank ...) is still asked for with `CSStartInteraction`, several go through the original interaction
//! list (`x2ui/interaction.lua`, the one doodads use: `INTERACTION_START`, `X2Interaction:GetInteractionSkillCount/Id/
//! Name`, `SelectInteraction`) with the skill names of the database, and the choice is handled here.
//!
//! The services the client opens itself, by the name `npc_interaction.lua` knows them:
//!
//! | skill | window | what the natives do |
//! |---|---|---|
//! | 12086 Repair | `blacksmith` (the repair window) | `repair.rs` |
//! | 12084 Blessing | `priest` (the blessing dialog) | `X2Priest:GetPriestBuff` / `BuyPriestBuff`: `CSBuyPriestBuff` |
//! | 17063 Offer Prayer | `priest_recover_exp` | `X2Priest:GetCostForRecoverExp` / `RecoverExp`: the skill 17063 at oneself |
//! | 12082 Change Skillsets | `ability_change` | `X2Ability:GetAbilityChangeCost` / `CanBuyAbilityChange` / `SwapAbility` |
//!
//! The rest (store, bank, auction, stabler, expedition, recruiter) keep the handlers of their own modules, which read the
//! `SCNpcInteractionSkillList`; a choice made here reaches them as the same event with that one skill.

use std::collections::HashSet;

use aa_net::interaction::InteractEvent;
use aa_net::shop::ShopCommand;
use aa_net::state::{Player, Units};
use aa_net::{Command, Event, NetCommand, NetEvent};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use mlua::{Lua, Table};
use rusqlite::Connection;

use super::super::api::{HudBridge, HudData, Shared};
use super::super::data::{GameData, any_flag, any_int};
use super::super::dialog::Req;
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
        PRAYER => ("priest_recover_exp", "GetRecoverExpMessageBoxHandleWindow()"),
        ABILITY => ("ability_change", "abilityChangeFrame"),
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
    buffs: Vec<Buff>,
    ability_cost: i64,
    recover_money: Option<Formula>,
    recover_labor: Option<Formula>,
}

impl Tables {
    pub fn load(conn: &Connection) -> rusqlite::Result<Self> {
        let mut out = Self::default();
        let mut q = conn.prepare("SELECT id,priest,blacksmith,repairman,ability_changer FROM npcs")?;
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
        }
        let mut q = conn.prepare("SELECT id,cost FROM priest_buffs ORDER BY id")?;
        let mut rows = q.query([])?;
        while let Some(r) = rows.next()? {
            out.buffs.push(Buff { id: any_int(r, 0)? as u32, cost: any_int(r, 1)? });
        }
        let mut q = conn.prepare("SELECT value FROM content_configs WHERE id=?1")?;
        out.ability_cost = q.query_row([ABILITY_COST_CONFIG], |r| any_int(r, 0)).unwrap_or(0);
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
    out
}

/// How the talk to an NPC goes on when it has no quest business.
#[derive(Debug, PartialEq, Eq)]
pub enum Route {
    /// Nothing to offer.
    None,
    /// One service the lane's answer to `CSStartInteraction` drives.
    Server,
    /// The client chooses: the services in the order they are listed.
    Local(Vec<u32>),
}

pub fn route(game: &GameData, npc: u32) -> Route {
    let skills = offers(game, npc);
    match skills[..] {
        [] => Route::None,
        [STORE | BANK | AUCTION | STABLER | EXPEDITION | RECRUITER] => Route::Server,
        _ => Route::Local(skills),
    }
}

/// The player's services state: the list that is up, the window that is up and what the scripts asked for.
#[derive(Default, Debug)]
pub struct View {
    /// An NPC with services was talked to: the NPC and the services, the list comes up next frame.
    pending: Option<((u32, u32), Vec<u32>)>,
    /// The interaction list that is up: the NPC and its rows (skill id, name).
    menu: Option<((u32, u32), Vec<(u32, String)>)>,
    picked: Vec<i64>,
    cancelled: bool,
    /// The window of a service the client opened, and its NPC.
    active: Option<((u32, u32), u32)>,
    requests: Vec<Request>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Request {
    /// Buy blessing `n` (1 based).
    Bless(usize),
    Pray,
}

impl View {
    /// The talk to `npc` goes on with the choice between `skills`.
    pub fn offer(&mut self, npc: (u32, u32), skills: Vec<u32>) {
        self.pending = Some((npc, skills));
    }

    /// The service window is up for `npc`.
    pub fn serving(&self, npc: (u32, u32)) -> bool {
        self.active.is_some_and(|a| a.0 == npc)
    }
}

/// What the scripts read of the player and the price lists: `HUD.service_info()`.
fn info(lua: &Lua, d: &HudData) -> mlua::Result<Table> {
    let out = lua.create_table()?;
    let level = i64::from(d.snapshot.level);
    let Some(game) = d.game.get() else { return Ok(out) };
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
    hud.set("service_info", lua.create_function(move |lua, ()| info(lua, &b.borrow()))?)?;
    let b = bridge.clone();
    hud.set(
        "service_request",
        lua.create_function(move |_, (what, n): (String, Option<usize>)| {
            let request = match what.as_str() {
                "bless" => Request::Bless(n.unwrap_or(0)),
                "pray" => Request::Pray,
                _ => return Ok(()),
            };
            b.borrow_mut().store.services.requests.push(request);
            Ok(())
        })?,
    )?;
    let b = bridge.clone();
    hud.set(
        "npc_menu",
        lua.create_function(move |lua, ()| {
            let d = b.borrow();
            let Some((_, rows)) = &d.store.services.menu else { return Ok(None) };
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
            b.borrow_mut().store.services.picked.push(id);
            Ok(())
        })?,
    )?;
    let b = bridge.clone();
    hud.set(
        "npc_menu_cancel",
        lua.create_function(move |_, ()| {
            b.borrow_mut().store.services.cancelled = true;
            Ok(())
        })?,
    )?;
    Ok(())
}

/// The natives of the services' windows, and the interaction list over the doodad one: loaded after `doodad::install`,
/// whose natives it wraps.
pub fn load_script(lua: &Lua) -> mlua::Result<()> {
    lua.load(include_str!("services.lua")).set_name("@services.lua").exec()
}

/// Is the Lua window expression visible?
fn visible(rt: &UiRuntime, expr: &str) -> bool {
    rt.lua.load(format!("local w = {expr}; return w ~= nil and w:IsVisible() == true")).eval::<bool>().unwrap_or(false)
}

fn near(d: &HudData, units: &Units, npc: (u32, u32), reach: f32) -> bool {
    units.get(npc.0).is_some_and(|u| !u.dead && d.pos.distance(Vec3::new(u.pos.x, u.pos.y, u.pos.z)) <= reach)
}

/// The interaction list and the services' windows.
#[allow(clippy::too_many_arguments)]
pub fn update(
    rt: Option<NonSend<UiRuntime>>,
    bridge: NonSend<HudBridge>,
    player: Res<Player>,
    units: Res<Units>,
    mut net: MessageWriter<NetCommand>,
    mut announce: MessageWriter<NetEvent>,
) {
    let Some(rt) = rt else { return };
    let Some(spawn) = player.spawn.as_ref() else { return };
    let own = spawn.obj_id;
    let mut d = bridge.0.borrow_mut();
    // the talk that goes on with a choice
    if let Some((npc, skills)) = d.store.services.pending.take()
        && d.dialog.npc == Some(npc)
    {
        let rows: Vec<(u32, String)> = skills
            .iter()
            .map(|s| (*s, d.game.get().and_then(|g| g.skills.get(s)).map_or_else(String::new, |s| s.named.name.clone())))
            .collect();
        info!("hud: npc {} (template {}) offers {skills:?}", npc.0, npc.1);
        if let [(skill, _)] = rows[..] {
            drop(d);
            start(&rt, &bridge, &mut announce, own, npc, skill);
            d = bridge.0.borrow_mut();
        } else {
            d.store.services.menu = Some((npc, rows));
            drop(d);
            rt.fire_event("INTERACTION_START", ());
            d = bridge.0.borrow_mut();
        }
    }
    // the list: a pick, Cancel, or the player walked off
    let picked = std::mem::take(&mut d.store.services.picked);
    let cancelled = std::mem::take(&mut d.store.services.cancelled);
    if let Some((npc, rows)) = d.store.services.menu.clone() {
        let choice = picked.iter().find_map(|id| rows.iter().find(|r| i64::from(r.0) == *id)).map(|r| r.0);
        let gone = d.dialog.npc != Some(npc) || !near(&d, &units, npc, REACH + 1.0);
        if choice.is_some() || cancelled || gone {
            d.store.services.menu = None;
            if choice.is_none() {
                d.dialog.requests.push(Req::End);
            }
            drop(d);
            hide_list(&rt);
            if let Some(skill) = choice {
                start(&rt, &bridge, &mut announce, own, npc, skill);
            }
            d = bridge.0.borrow_mut();
        }
    }
    // the window of a service: it ends with the talk, the window or the distance
    if let Some((npc, skill)) = d.store.services.active
        && let Some((_, expr)) = window(skill)
    {
        let open = d.dialog.npc == Some(npc) && near(&d, &units, npc, REACH);
        drop(d);
        if !(open && visible(&rt, expr)) {
            bridge.0.borrow_mut().store.services.active = None;
            rt.fire_event("NPC_INTERACTION_END", ());
        }
        d = bridge.0.borrow_mut();
    }
    let requests = std::mem::take(&mut d.store.services.requests);
    let active = d.store.services.active;
    if active.is_some() {
        for request in requests {
            match request {
                Request::Bless(n) => {
                    let Some((npc, _)) = active else { continue };
                    let buff = d.game.get().and_then(|g| g.stores.services.buffs.get(n.wrapping_sub(1)).copied());
                    let Some(buff) = buff else { continue };
                    let cost = i64::from(d.snapshot.level) * buff.cost;
                    if d.snapshot.money < cost {
                        warn!("hud: blessing {} costs {cost}, the player has {}", buff.id, d.snapshot.money);
                        continue;
                    }
                    net.write(NetCommand(Command::Shop(ShopCommand::BuyPriestBuff { priest_buff_id: buff.id, npc_obj_id: npc.0 })));
                }
                Request::Pray => {
                    net.write(NetCommand(Command::UseSkill { skill_id: PRAYER, target_obj_id: own }));
                }
            }
        }
    }
}

fn hide_list(rt: &UiRuntime) {
    if let Ok(f) = rt.lua.globals().get::<mlua::Function>("HideAllButtons") {
        let _ = f.call::<()>(());
    }
}

/// The player chose `skill` of `npc`: a window the client opens itself, or the announcement the handlers of the other
/// services read.
fn start(rt: &UiRuntime, bridge: &HudBridge, announce: &mut MessageWriter<NetEvent>, own: u32, npc: (u32, u32), skill: u32) {
    info!("hud: service {skill} of npc {}", npc.0);
    if let Some((name, _)) = window(skill) {
        bridge.0.borrow_mut().store.services.active = Some((npc, skill));
        rt.fire_event("NPC_INTERACTION_START", (name, "", npc.0.to_string()));
    } else {
        announce.write(NetEvent(Event::Interaction(InteractEvent::SkillList {
            npc_obj_id: npc.0,
            obj_id: own,
            extra_info: 0,
            pick_id: 0,
            mouse_button: 2,
            skills: vec![skill],
            interactable: true,
            modifier_keys: 0,
        })));
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Good, Tables as Stores};
    use super::*;

    fn game() -> GameData {
        let mut game = GameData::default();
        let mut stores = Stores::default();
        stores.merchants.insert(10, vec![Good { item: 1, grade: 0, currency: 0 }]);
        stores.merchants.insert(11, vec![Good { item: 1, grade: 0, currency: 0 }]);
        stores.services.repair.insert(10);
        stores.services.repair.insert(12);
        stores.services.priest.insert(20);
        stores.services.ability.insert(20);
        stores.bankers.insert(30);
        game.stores = stores;
        game
    }

    #[test]
    fn services_follow_the_npc_flags_in_the_lanes_order() {
        let game = game();
        assert_eq!(offers(&game, 10), vec![REPAIR, STORE]);
        assert_eq!(offers(&game, 20), vec![ABILITY, BLESSING, PRAYER]);
        assert_eq!(offers(&game, 99), Vec::<u32>::new());
    }

    #[test]
    fn a_lone_service_of_the_lane_goes_to_the_lane_and_the_rest_is_chosen() {
        let game = game();
        assert_eq!(route(&game, 99), Route::None);
        assert_eq!(route(&game, 11), Route::Server);
        assert_eq!(route(&game, 30), Route::Server);
        // a repairman alone is not something the lane's answer drives
        assert_eq!(route(&game, 12), Route::Local(vec![REPAIR]));
        assert_eq!(route(&game, 10), Route::Local(vec![REPAIR, STORE]));
        assert_eq!(route(&game, 20), Route::Local(vec![ABILITY, BLESSING, PRAYER]));
    }

    #[test]
    fn tables_read_the_flags_prices_and_formulas() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE npcs(id,priest,blacksmith,repairman,ability_changer);
             INSERT INTO npcs VALUES(1,'t','f','f','t'),(2,'f','t','f','f'),(3,'f','f','t','f'),(4,'f','f','f','f');
             CREATE TABLE priest_buffs(id,buff_id,cost,position); INSERT INTO priest_buffs VALUES(1,239,100,1);
             CREATE TABLE content_configs(id,kind_id,value); INSERT INTO content_configs VALUES(39,12,200),(38,12,1000);
             CREATE TABLE formulas(id,formula); INSERT INTO formulas VALUES(6,'0'),(7,'if_negative(pc_level - 51, pc_level, 50 + (pc_level - 50) * 20)');",
        )
        .unwrap();
        let t = Tables::load(&db).unwrap();
        assert_eq!((t.priest.len(), t.repair.len(), t.ability.len()), (1, 2, 1));
        assert_eq!(t.buffs, vec![Buff { id: 1, cost: 100 }]);
        assert_eq!(t.ability_cost, 200);
        let vars = |level: f64| std::collections::HashMap::from([("pc_level", level)]);
        let labor = t.recover_labor.as_ref().unwrap();
        assert_eq!((labor.eval(&vars(30.0)), labor.eval(&vars(55.0))), (30.0, 150.0));
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
        assert_eq!(shared.borrow().store.services.requests, vec![Request::Bless(1), Request::Pray]);
    }
}
