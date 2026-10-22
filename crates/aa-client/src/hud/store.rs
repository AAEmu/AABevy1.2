//! NPC service context, local merchant catalogs and original store actions.
use std::collections::{HashMap, HashSet};

use aa_net::interaction::InteractEvent;
use aa_net::model::{Item, ItemDetail};
use aa_net::shop::{BuyEntry, SellEntry, Shop, ShopCommand, ShopEvent};
use aa_net::state::{Inventory, Player, Units};
use aa_net::{Command, Event, NetCommand, NetEvent};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use mlua::{Lua, Table};
use rusqlite::Connection;

use super::api::{HudBridge, Shared};
use super::data::{GameData, any_flag, any_int};
use super::tips::{self, Instance};
use crate::Screen;

#[derive(Clone, Debug, PartialEq)]
pub struct Good {
    pub item: u32,
    /// Database grade ID, retained for the wire. Lua uses grade_order.
    pub grade: u8,
    pub currency: u8,
}

#[derive(Default)]
pub struct Tables {
    pub merchants: HashMap<u32, Vec<Good>>,
    pub bankers: HashSet<u32>,
    pub auctioneers: HashSet<u32>,
    refund_multipliers: HashMap<u8, i64>,
}

impl Tables {
    pub fn load(conn: &Connection) -> rusqlite::Result<Self> {
        let mut out = Self::default();
        let mut q = conn.prepare("SELECT m.npc_id,g.item_id,g.grade_id,p.kind_id FROM merchants m JOIN merchant_packs p ON p.id=m.merchant_pack_id JOIN merchant_goods g ON g.merchant_pack_id=m.merchant_pack_id ORDER BY m.id,g.id")?;
        let mut rows = q.query([])?;
        while let Some(r) = rows.next()? {
            let npc = any_int(r, 0)? as u32;
            let item = any_int(r, 1)? as u32;
            // Native GetStoreCurrency: siege stores share honor currency with honor stores.
            let currency = match any_int(r, 3)? {
                1 | 2 => 1,
                3 => 2,
                4 => 3,
                5 => 4,
                _ => 0,
            };
            let goods = out.merchants.entry(npc).or_default();
            if !goods.iter().any(|g| g.item == item) {
                goods.push(Good {
                    item,
                    grade: any_int(r, 2)?.clamp(0, 11) as u8,
                    currency,
                });
            }
        }
        let mut q = conn.prepare("SELECT id,banker,auctioneer FROM npcs")?;
        let mut rows = q.query([])?;
        while let Some(r) = rows.next()? {
            if any_flag(r, 1)? {
                out.bankers.insert(r.get(0)?);
            }
            if any_flag(r, 2)? {
                out.auctioneers.insert(r.get(0)?);
            }
        }
        let mut q = conn.prepare("SELECT id,refund_multiplier FROM item_grades")?;
        let mut rows = q.query([])?;
        while let Some(r) = rows.next()? {
            out.refund_multipliers.insert(r.get(0)?, any_int(r, 1)?);
        }
        Ok(out)
    }

    pub fn has_service(&self, npc: u32) -> bool {
        self.merchants.contains_key(&npc) || self.bankers.contains(&npc) || self.auctioneers.contains(&npc)
    }

    pub fn refund(&self, base: i64, raw_grade: u8) -> i64 {
        let multiplier = self
            .refund_multipliers
            .get(&raw_grade)
            .copied()
            .unwrap_or(100);
        // The lane and original use single precision and truncate each item's unit price.
        (base.saturating_mul(multiplier) as f32 / 100.0) as i32 as i64
    }

    pub(super) fn refund_multiplier(&self, raw_grade: u8) -> Option<i32> {
        self.refund_multipliers.get(&raw_grade).and_then(|value| i32::try_from(*value).ok())
    }
}

pub fn ui_grade(raw: u8) -> u8 {
    match raw {
        0 => 1,
        1 => 0,
        g => g,
    }
}

#[derive(Debug)]
enum Req {
    Buy(Vec<(usize, i32, u8)>),
    Sell(Vec<(usize, u64)>),
    BuyBack(usize),
    ListSold,
    Deposit(i32),
    Withdraw(i32),
}

#[derive(Default, Debug)]
pub struct View {
    pub npc: Option<(u32, u32)>,
    bank: bool,
    requests: Vec<Req>,
    sold: Vec<Item>,
    sold_seen: bool,
    refresh_pending: bool,
}

pub fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        update
            .after(super::sync_hud)
            .after(super::dialog::handle_dialog)
            .run_if(super::hud_up)
            .run_if(in_state(Screen::InGame)),
    );
}

pub fn install(lua: &Lua, hud: &Table, bridge: &Shared) -> mlua::Result<()> {
    let b = bridge.clone();
    hud.set(
        "store_goods",
        lua.create_function(move |lua, ()| {
            let d = b.borrow();
            let out = lua.create_table()?;
            let Some(game) = d.game.get() else {
                return Ok(out);
            };
            let Some((_, npc)) = d.store.npc else {
                return Ok(out);
            };
            for (index, good) in game
                .stores
                .merchants
                .get(&npc)
                .into_iter()
                .flatten()
                .enumerate()
            {
                if let Some(t) = goods_info(lua, game, &d.tips, good, index)? {
                    out.raw_push(t)?;
                }
            }
            Ok(out)
        })?,
    )?;
    let b = bridge.clone();
    hud.set(
        "store_buy",
        lua.create_function(
            move |_, (indices, counts, currencies): (Table, Table, Table)| {
                let indices = indices
                    .sequence_values::<usize>()
                    .take(16)
                    .collect::<mlua::Result<Vec<_>>>()?;
                let counts = counts
                    .sequence_values::<i32>()
                    .take(16)
                    .collect::<mlua::Result<Vec<_>>>()?;
                let currencies = currencies
                    .sequence_values::<u8>()
                    .take(16)
                    .collect::<mlua::Result<Vec<_>>>()?;
                if !indices.is_empty()
                    && indices.len() == counts.len()
                    && counts.len() == currencies.len()
                {
                    b.borrow_mut().store.requests.push(Req::Buy(
                        indices
                            .into_iter()
                            .zip(counts)
                            .zip(currencies)
                            .map(|((i, n), c)| (i, n, c))
                            .collect(),
                    ));
                }
                Ok(())
            },
        )?,
    )?;
    let b = bridge.clone();
    hud.set(
        "store_sell",
        lua.create_function(move |_, (slots, ids): (Table, Table)| {
            let slots = slots
                .sequence_values::<usize>()
                .take(16)
                .collect::<mlua::Result<Vec<_>>>()?;
            let ids = ids
                .sequence_values::<String>()
                .take(16)
                .collect::<mlua::Result<Vec<_>>>()?;
            if !slots.is_empty() && slots.len() == ids.len() {
                let entries = slots
                    .into_iter()
                    .zip(ids)
                    .map(|(slot, id)| Some((slot, id.parse::<u64>().ok()?)))
                    .collect::<Option<Vec<_>>>();
                if let Some(entries) = entries {
                    b.borrow_mut().store.requests.push(Req::Sell(entries));
                }
            }
            Ok(())
        })?,
    )?;
    let b = bridge.clone();
    hud.set(
        "store_buyback",
        lua.create_function(move |_, index: usize| {
            b.borrow_mut().store.requests.push(Req::BuyBack(index));
            Ok(())
        })?,
    )?;
    let b = bridge.clone();
    hud.set(
        "store_list_sold",
        lua.create_function(move |_, ()| {
            b.borrow_mut().store.requests.push(Req::ListSold);
            Ok(())
        })?,
    )?;
    for (name, deposit) in [("bank_deposit", true), ("bank_withdraw", false)] {
        let b = bridge.clone();
        hud.set(
            name,
            lua.create_function(move |_, amount: String| {
                if let Ok(amount) = amount.parse::<i32>() {
                    if amount > 0 {
                        b.borrow_mut().store.requests.push(if deposit {
                            Req::Deposit(amount)
                        } else {
                            Req::Withdraw(amount)
                        });
                    }
                }
                Ok(())
            })?,
        )?;
    }
    Ok(())
}

fn goods_info(
    lua: &Lua,
    game: &GameData,
    state: &super::tips::TipState,
    good: &Good,
    index: usize,
) -> mlua::Result<Option<Table>> {
    let Some(info) = game.items.get(&good.item) else {
        return Ok(None);
    };
    let Some(t) = tips::item_tip(
        lua,
        game,
        state,
        good.item,
        Some(i64::from(ui_grade(good.grade))),
        Some(1),
        Instance::default(),
    )?
    else {
        return Ok(None);
    };
    t.set("goodIndex", index + 1)?;
    t.set("currency", good.currency)?;
    t.set("cost", info.price)?;
    t.set("money", info.price)?;
    t.set("honorPrice", info.honor_price)?;
    t.set("livingPointPrice", info.living_price)?;
    t.set("maxStack", info.max_stack.max(1))?;
    t.set("path", info.named.icon.as_str())?;
    Ok(Some(t))
}

fn sold_info(
    lua: &Lua,
    game: &GameData,
    state: &super::tips::TipState,
    sold: &[Item],
) -> mlua::Result<Table> {
    let out = lua.create_table()?;
    for item in sold {
        let instance = match &item.detail {
            ItemDetail::Equip(e) => Instance {
                durability: Some(e.durability),
                temper: (e.temper_physical, e.temper_magical),
                flags: Some(item.flags),
            },
            _ => Instance {
                flags: Some(item.flags),
                ..default()
            },
        };
        if let Some(t) = tips::item_tip(
            lua,
            game,
            state,
            item.template_id,
            Some(i64::from(ui_grade(item.grade))),
            Some(i64::from(item.count)),
            instance,
        )? {
            out.raw_push(t)?;
        }
    }
    Ok(out)
}

fn build_command(
    req: Req,
    npc: (u32, u32),
    game: &GameData,
    inv: &Inventory,
    sold: &[Item],
) -> Option<ShopCommand> {
    Some(match req {
        Req::ListSold => ShopCommand::ListSold { npc_obj_id: npc.0 },
        Req::BuyBack(index) if index > 0 && index <= sold.len() => ShopCommand::Buy {
            npc_obj_id: npc.0,
            doodad_obj_id: 0,
            buy: vec![],
            buy_back: vec![(index - 1) as i32],
            use_aa_point: false,
        },
        Req::Buy(entries) if !entries.is_empty() => {
            let catalog = game.stores.merchants.get(&npc.1)?;
            let mut buy = Vec::new();
            for (index, count, currency) in entries {
                let good = catalog.get(index.checked_sub(1)?)?;
                if count <= 0 || currency != good.currency || currency > 2 {
                    return None;
                }
                buy.push(BuyEntry {
                    item_id: good.item,
                    grade: good.grade,
                    count,
                    currency,
                });
            }
            ShopCommand::Buy {
                npc_obj_id: npc.0,
                doodad_obj_id: 0,
                buy,
                buy_back: vec![],
                use_aa_point: false,
            }
        }
        Req::Sell(slots) if !slots.is_empty() => {
            let mut items = Vec::new();
            for (slot, expected_id) in slots {
                let index = slot.checked_sub(1)?;
                let item = inv.bag.get(index)?.as_ref()?;
                if item.id != expected_id
                    || expected_id == 0
                    || !game.items.get(&item.template_id)?.sellable
                    || index > u8::MAX as usize
                {
                    return None;
                }
                items.push(SellEntry {
                    container: aa_net::items::container::INVENTORY,
                    slot: index as u8,
                    item_id: item.id,
                });
            }
            ShopCommand::Sell {
                npc_obj_id: npc.0,
                items,
            }
        }
        _ => return None,
    })
}

#[allow(clippy::too_many_arguments)]
fn update(
    rt: Option<NonSend<UiRuntime>>,
    bridge: NonSend<HudBridge>,
    player: Res<Player>,
    units: Res<Units>,
    inventory: Res<Inventory>,
    shop: Res<Shop>,
    mut events: MessageReader<NetEvent>,
    mut net: MessageWriter<NetCommand>,
) {
    let Some(rt) = rt else { return };
    for NetEvent(event) in events.read() {
        if let Event::Interaction(InteractEvent::SkillList {
            npc_obj_id,
            skills,
            interactable,
            ..
        }) = event
        {
            let service = {
                let mut d = bridge.0.borrow_mut();
                let npc = d.dialog.npc.filter(|n| n.0 == *npc_obj_id);
                if !interactable {
                    None
                } else if skills.contains(&12087) {
                    if let Some(npc) = npc {
                        d.store = View {
                            npc: Some(npc),
                            ..default()
                        };
                    }
                    npc.map(|_| "store")
                } else if skills.contains(&13238) {
                    if let Some(npc) = npc {
                        d.store = View {
                            npc: Some(npc),
                            bank: true,
                            ..default()
                        };
                    }
                    npc.map(|_| "bank")
                } else {
                    None
                }
            };
            if let Some(service) = service {
                rt.fire_event(
                    "NPC_INTERACTION_START",
                    (service, "", npc_obj_id.to_string()),
                );
            }
        }
        if let Event::Shop(ShopEvent::RequestRejected(message)) = event {
            warn!("hud: shop request: {message}");
            rt.fire_event("STORE_TRADE_FAILED", ());
        }
    }
    let (context, me, bank) = {
        let d = bridge.0.borrow();
        (d.store.npc, d.pos, d.store.bank)
    };
    let Some(npc) = context else { return };
    let visible = rt
        .lua
        .globals()
        .get::<mlua::Function>(if bank {
            "IsActivatedBank"
        } else {
            "IsActivatedStore"
        })
        .and_then(|f| f.call::<bool>(()))
        .unwrap_or(false);
    let close = !visible
        || !units
            .get(npc.0)
            .is_some_and(|u| me.distance(Vec3::new(u.pos.x, u.pos.y, u.pos.z)) <= 3.0)
        || player.spawn.is_none()
        || bridge.0.borrow().dialog.npc != Some(npc);
    if close {
        bridge.0.borrow_mut().store = View::default();
        rt.fire_event("NPC_INTERACTION_END", ());
        return;
    }
    let requests = std::mem::take(&mut bridge.0.borrow_mut().store.requests);
    for request in requests {
        match request {
            Req::Deposit(amount) | Req::Withdraw(amount) if bank => {
                let command = if matches!(request, Req::Deposit(_)) {
                    aa_net::bank::BankCommand::DepositMoney {
                        amount,
                        aa_point: 0,
                    }
                } else {
                    aa_net::bank::BankCommand::WithdrawMoney {
                        amount,
                        aa_point: 0,
                    }
                };
                net.write(NetCommand(Command::Bank(command)));
                continue;
            }
            _ if bank => continue,
            _ => {}
        }
        let command = {
            let d = bridge.0.borrow();
            d.game
                .get()
                .and_then(|g| build_command(request, npc, g, &inventory, &shop.sold))
        };
        if let Some(command) = command {
            if matches!(command, ShopCommand::ListSold { .. }) {
                bridge.0.borrow_mut().store.refresh_pending = true;
            }
            info!("hud: shop request {command:?}");
            net.write(NetCommand(Command::Shop(command)));
        } else {
            rt.fire_event("STORE_TRADE_FAILED", ());
        }
    }
    if bank {
        return;
    }
    if shop.loaded {
        let push = {
            let mut d = bridge.0.borrow_mut();
            let changed = !d.store.sold_seen || d.store.sold != shop.sold;
            d.store.sold_seen = true;
            d.store.sold.clone_from(&shop.sold);
            if !shop.dirty {
                d.store.refresh_pending = false;
            }
            changed
        };
        if push {
            let list = {
                let d = bridge.0.borrow();
                d.game
                    .get()
                    .map(|g| sold_info(&rt.lua, g, &d.tips, &shop.sold))
            };
            match list {
                Some(Ok(list)) => rt.fire_event("STORE_SOLD_LIST", list),
                Some(Err(e)) => warn!("hud: sold list: {e}"),
                None => {}
            }
        }
    }
    if shop.dirty && !bridge.0.borrow().store.refresh_pending {
        bridge.0.borrow_mut().store.refresh_pending = true;
        net.write(NetCommand(Command::Shop(ShopCommand::ListSold {
            npc_obj_id: npc.0,
        })));
    }
}

pub fn load_script(lua: &Lua) -> mlua::Result<()> {
    lua.load(include_str!("store.lua"))
        .set_name("@store.lua")
        .exec()?;
    lua.load(include_str!("bank.lua"))
        .set_name("@bank.lua")
        .exec()
}

#[cfg(test)]
mod tests {
    use super::super::data::{ItemInfo, Named};
    use super::*;

    #[test]
    fn catalog_preserves_native_currency_order_and_first_duplicate() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE merchants(id,npc_id,merchant_pack_id); CREATE TABLE merchant_packs(id,kind_id); CREATE TABLE merchant_goods(id,merchant_pack_id,item_id,grade_id); CREATE TABLE npcs(id,banker); CREATE TABLE item_grades(id,refund_multiplier);
            INSERT INTO merchants VALUES(2,10,2),(1,10,1),(3,11,3),(4,12,4);
            INSERT INTO merchant_packs VALUES(1,0),(2,2),(3,3),(4,1);
            INSERT INTO merchant_goods VALUES(2,1,100,0),(1,1,101,1),(3,2,100,5),(4,2,102,2),(5,3,103,0),(6,4,104,0);
            INSERT INTO npcs VALUES(1,'f'),(2,'t'); INSERT INTO item_grades VALUES(0,100),(1,75),(2,115);").unwrap();
        let tables = Tables::load(&db).unwrap();
        assert_eq!(
            tables.merchants[&10],
            vec![
                Good {
                    item: 101,
                    grade: 1,
                    currency: 0
                },
                Good {
                    item: 100,
                    grade: 0,
                    currency: 0
                },
                Good {
                    item: 102,
                    grade: 2,
                    currency: 1
                }
            ]
        );
        assert_eq!(tables.merchants[&11][0].currency, 2);
        assert_eq!(tables.merchants[&12][0].currency, 1);
        assert!(tables.has_service(2));
        assert!(!tables.has_service(1));
        assert_eq!(tables.refund(9, 2), 10);
        assert_eq!(tables.refund(0, 0), 0);
        assert_eq!((ui_grade(0), ui_grade(1)), (1, 0));
    }

    fn fixture() -> GameData {
        let mut game = GameData::default();
        game.items.insert(
            100,
            ItemInfo {
                named: Named {
                    name: "Supply".into(),
                    icon: "ui/icon/test.dds".into(),
                },
                price: 25,
                refund: 0,
                sellable: true,
                max_stack: 20,
                ..default()
            },
        );
        game.stores.merchants.insert(
            10,
            vec![Good {
                item: 100,
                grade: 1,
                currency: 0,
            }],
        );
        game
    }

    #[test]
    fn shop_uses_catalog_grade_and_actual_inventory_identity() {
        let game = fixture();
        let mut inv = Inventory::default();
        inv.bag = vec![Some(Item {
            id: 999,
            template_id: 100,
            count: 3,
            ..default()
        })];
        let Some(ShopCommand::Buy { buy, .. }) =
            build_command(Req::Buy(vec![(1, 2, 0)]), (500, 10), &game, &inv, &[])
        else {
            panic!()
        };
        assert_eq!(
            buy,
            vec![BuyEntry {
                item_id: 100,
                grade: 1,
                count: 2,
                currency: 0
            }]
        );
        let Some(ShopCommand::Sell { items, .. }) =
            build_command(Req::Sell(vec![(1, 999)]), (500, 10), &game, &inv, &[])
        else {
            panic!()
        };
        assert_eq!(
            items,
            vec![SellEntry {
                container: 2,
                slot: 0,
                item_id: 999
            }]
        );
        for req in [
            Req::Buy(vec![(0, 1, 0)]),
            Req::Buy(vec![(2, 1, 0)]),
            Req::Buy(vec![(1, 0, 0)]),
            Req::Buy(vec![(1, 1, 1)]),
            Req::Sell(vec![(0, 999)]),
            Req::Sell(vec![(2, 999)]),
            Req::Sell(vec![(1, 888)]),
            Req::BuyBack(1),
        ] {
            assert!(build_command(req, (500, 10), &game, &inv, &[]).is_none());
        }
        let sold = vec![inv.bag[0].clone().unwrap()];
        let Some(ShopCommand::Buy { buy_back, .. }) =
            build_command(Req::BuyBack(1), (500, 10), &game, &inv, &sold)
        else {
            panic!()
        };
        assert_eq!(buy_back, vec![0]);
    }

    #[test]
    fn zero_grade_is_crude_and_zero_refund_stays_zero_in_real_tooltip() {
        let lua = Lua::new();
        let game = fixture();
        let tip = tips::item_tip(&lua, &game, &default(), 100, Some(0), Some(3), default())
            .unwrap()
            .unwrap();
        assert_eq!(tip.get::<i64>("itemGrade").unwrap(), 0);
        assert_eq!(tip.get::<i64>("refund").unwrap(), 0);
        let goods = goods_info(&lua, &game, &default(), &game.stores.merchants[&10][0], 0)
            .unwrap()
            .unwrap();
        assert_eq!(goods.get::<i64>("itemGrade").unwrap(), 0);
        assert_eq!(goods.get::<i64>("cost").unwrap(), 25);
        assert_eq!(goods.get::<i64>("goodIndex").unwrap(), 1);
    }

    #[test]
    fn bank_window_routes_filtered_and_partial_moves_without_mutating_slots() {
        let lua = Lua::new();
        lua.load(r#"
            X2Bank={}; X2Bag={}; X2Util={}; X2Item={}; X2Cursor={}; FAKE={}; CURRENCY_GOLD=0
            HUD={bank_slots=3,bag_slots=3,state={bank_money='12345'},bank={[1]={id=10,grade=1,count=8,stack=true,group=7}},bag={[1]={id=20,count=1}}}
            moves={}; currentHeld=nil; oldClicks=0
            function HUD.drag_move(...) moves[#moves+1]={...} end
            function HUD.shift_down() return false end
            function HUD.item_tip(id,grade,count) return {lookType=id,itemGrade=grade,stack=count} end
            function FAKE.fire_later(e) end
            function FAKE.fire_now(e,...) end
            function X2Item:GetItemIconSet() return {} end
            function X2Cursor:GetHeld() return currentHeld end
            function X2Cursor:SetHeld(h) currentHeld=h end
            function X2Cursor:ClearCursor() currentHeld=nil end
            function X2Bag:HandleClick() oldClicks=oldClicks+1 end
            function X2Bag:HandleUse() end
            function X2Bag:SlotByIdx(i) return HUD.bag[i] and i end
            function X2Bag:CurrentTabIdx() return 1 end
            function X2Bag:TabCount() return 1 end
            function X2Bag:IsLocked() return false end
            function IsActivatedBank() return true end
            function BagItemToEmptyBankSlot(slot) X2Bag:MoveToEmptyBankSlot(slot); return true end
        "#).exec().unwrap();
        lua.load(include_str!("bank.lua")).exec().unwrap();
        lua.load(r#"
            assert(X2Util:GetMyBankMoneyString()=='12345')
            assert(X2Bank:Capacity()==3 and X2Bank:CountItems()==1)
            assert(X2Bank:GetBagItemInfo(1).itemGrade==1)
            X2Bank:CreateTab(2,'Materials',{7})
            assert(X2Bank:SlotByIdx(1)==1 and X2Bank:SlotByIdx(2)==nil)
            X2Bank:PickupBankItemPartial(1,3)
            X2Bag:HandleClick(2)
            assert(#moves==1 and moves[1][1]==3 and moves[1][2]==1 and moves[1][3]==2 and moves[1][4]==2 and moves[1][5]==3)
            X2Bag:HandleUse(1)
            assert(#moves==2 and moves[2][1]==2 and moves[2][2]==1 and moves[2][3]==3 and moves[2][4]==2)
            X2Bank:HandleUse(1)
            assert(#moves==3 and moves[3][1]==3 and moves[3][3]==2)
            assert(HUD.bank[1].count==8 and HUD.bag[1].id==20 and HUD.bank[2]==nil)
            X2Bag:HandleClick(1); assert(oldClicks==1)
            X2Bank:RemoveTab(2); assert(X2Bank:CurrentTabIdx()==1 and X2Bank:TabCount()==1)
        "#).exec().unwrap();
    }

    #[test]
    fn sell_cart_keeps_locked_identity_when_a_server_update_replaces_the_slot() {
        let lua = Lua::new();
        let shared = std::rc::Rc::new(std::cell::RefCell::new(
            super::super::api::HudData::default(),
        ));
        let hud = lua.create_table().unwrap();
        install(&lua, &hud, &shared).unwrap();
        lua.globals().set("HUD", hud).unwrap();
        lua.load("X2Bag={}; X2Store={}; X2Interaction={}; HUD.bag={[1]={uid='999'}}")
            .exec()
            .unwrap();
        lua.load(include_str!("store.lua")).exec().unwrap();
        // The original submit callback queues the sale before unlocking its cart rows.
        lua.load("X2Bag:LockSlot(1); assert(X2Bag:IsLocked(1)); HUD.bag[1]={uid='888'}; X2Store:SellStoreItem({1},1,false); X2Bag:UnlockSlot(1); assert(not X2Bag:IsLocked(1))").exec().unwrap();
        let req = shared.borrow_mut().store.requests.pop().unwrap();
        let Req::Sell(entries) = &req else { panic!() };
        assert_eq!(entries, &vec![(1, 999)]);
        let mut inv = Inventory::default();
        inv.bag = vec![Some(Item {
            id: 888,
            template_id: 100,
            ..default()
        })];
        assert!(build_command(req, (500, 10), &fixture(), &inv, &[]).is_none());
    }
}
