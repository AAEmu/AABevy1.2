//! Original mailbox queries and commands. Membership, money and items follow server state.
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::time::{Duration, Instant};

use aa_net::mail::{MailBody, MailCommand, MailEvent, MailHeader, MailSlot, Mailbox, SendMail};
use aa_net::model::{Item, ItemDetail};
use aa_net::state::{Doodads, Inventory};
use aa_net::{Command, Event, NetCommand, NetEvent};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use mlua::{Lua, ObjectLike, Table};
use rusqlite::Connection;

use super::api::{HudBridge, Shared};
use super::tips::{self, Instance};
use crate::Screen;

#[allow(dead_code)]
pub(super) mod fixture;

#[derive(Default)]
pub struct Tables {
    groups: HashSet<u32>,
}

impl Tables {
    pub fn load(conn: &Connection) -> rusqlite::Result<Self> {
        let mut q = conn.prepare("SELECT DISTINCT doodad_func_group_id FROM doodad_funcs WHERE actual_func_type='DoodadFuncNaviOpenMailbox'")?;
        let groups = q.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        Ok(Self { groups })
    }
}

#[derive(Clone)]
struct DraftItem {
    slot: u8,
    item: Item,
}

#[derive(Clone)]
struct DraftBackup {
    fields: Vec<(&'static str, String)>,
    items: [Option<DraftItem>; 10],
}

impl DraftBackup {
    fn capture(input: &Table, items: [Option<DraftItem>; 10]) -> mlua::Result<Self> {
        let mut fields = Vec::new();
        for name in ["receiver", "title", "text", "gold", "silver", "copper", "type", "doodadId", "withReceiver"] {
            fields.push((name, input.get::<Option<String>>(name)?.unwrap_or_default()));
        }
        Ok(Self { fields, items })
    }
}

#[derive(Default)]
struct View {
    initialized: bool,
    mailbox: Mailbox,
    inventory: Inventory,
    selected: Option<(bool, i64)>,
    nearest: u32,
    nearby: HashSet<u32>,
    draft: [Option<DraftItem>; 10],
    draft_changes: Vec<usize>,
    commands: Vec<Queued>,
    pending: Vec<(MailCommand, Instant)>,
    completed_claims: HashSet<i64>,
    sending: bool,
    send_backup: Option<DraftBackup>,
    restore: Option<DraftBackup>,
    notices: Vec<String>,
}

struct Queued {
    command: MailCommand,
    /// Outgoing mail addresses are physical bag slots; retain their exact identity until dispatch.
    expected: Vec<(u8, u64)>,
}

impl View {
    fn refuse(&mut self, backup: DraftBackup, message: &str) -> bool {
        self.restore = Some(backup);
        self.notices.push(message.to_owned());
        false
    }
    fn failed_send(&mut self) {
        self.sending = false;
        self.restore = self.send_backup.take();
    }
    fn body(&self) -> Option<&MailBody> {
        let (sent, id) = self.selected?;
        self.mailbox.body(sent, id)
    }
    fn queue(&mut self, command: MailCommand) -> bool {
        if command.validate().is_err() || self.pending.iter().any(|(c, _)| *c == command) || self.commands.iter().any(|q| q.command == command) {
            return false;
        }
        self.commands.push(Queued { command, expected: vec![] });
        true
    }
    fn current_inbox(&self) -> Option<i64> {
        self.selected.filter(|(sent, _)| !sent).map(|(_, id)| id)
    }
    fn place(&mut self, index: usize, slot: usize) -> bool {
        let Some(index) = index.checked_sub(1).filter(|i| *i < 10) else {
            return false;
        };
        let Some(slot) = slot.checked_sub(1).and_then(|s| u8::try_from(s).ok()) else {
            return false;
        };
        let Some(item) = self.inventory.bag.get(slot as usize).and_then(Option::as_ref).cloned() else {
            return false;
        };
        if item.flags & 1 != 0 || self.draft[index].is_some() || self.draft.iter().flatten().any(|d| d.item.id == item.id) {
            return false;
        }
        self.draft[index] = Some(DraftItem { slot, item });
        self.draft_changes.push(index + 1);
        true
    }
}

#[derive(Clone, Default)]
struct State(Rc<RefCell<View>>);

fn header(lua: &Lua, h: &MailHeader) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set("mail_id", h.mail_id.to_string())?;
    t.set("mail_type", h.mail_type)?;
    t.set("title", h.title.as_str())?;
    t.set("sender", h.sender_name.as_str())?;
    t.set("receiver", h.receiver_name.as_str())?;
    t.set("is_read", h.status != 0)?;
    t.set("hasItem", h.attachments != 0)?;
    t.set("extra", h.extra.to_string())?;
    Ok(t)
}

fn date(lua: &Lua, unix: i64) -> mlua::Result<Table> {
    let os: Table = lua.globals().get("os")?;
    os.call_function("date", ("*t", unix))
}

fn body_table(lua: &Lua, view: &View) -> mlua::Result<Option<Table>> {
    let Some(b) = view.body() else { return Ok(None) };
    let (sent, id) = view.selected.unwrap();
    let Some(h) = view.mailbox.header(sent, id) else { return Ok(None) };
    let t = header(lua, h)?;
    t.set("mail_id", b.mail_id.to_string())?;
    t.set("mail_type", b.mail_type)?;
    t.set("receiver", b.receiver_name.as_str())?;
    t.set("title", b.title.as_str())?;
    t.set("text", b.text.as_str())?;
    t.set("chargeMoney", b.billing_amount)?;
    t.set("sendDate", date(lua, b.send_date)?)?;
    t.set("recvDate", date(lua, b.recv_date)?)?;
    Ok(Some(t))
}

pub(super) fn item_info(lua: &Lua, bridge: &Shared, item: &Item) -> mlua::Result<Option<Table>> {
    let d = bridge.borrow();
    let Some(game) = d.game.get() else { return Ok(None) };
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
    tips::item_tip(
        lua,
        game,
        &d.tips,
        item.template_id,
        Some(i64::from(super::store::ui_grade(item.grade))),
        Some(i64::from(item.count)),
        instance,
    )
}

pub fn install(lua: &Lua, bridge: &Shared) -> mlua::Result<()> {
    let state = State::default();
    lua.set_app_data(state.clone());
    let mail: Table = lua.globals().get("X2Mail")?;
    for (name, sent) in [("GetReceivedMailCount", false), ("GetPostedMailCount", true)] {
        let s = state.clone();
        mail.set(name, lua.create_function(move |_, _: Table| Ok(s.0.borrow().mailbox.headers(sent).len()))?)?;
    }
    for (name, sent) in [("GetReceivedMailTitleInfo", false), ("GetPostedMailTitleInfo", true)] {
        let s = state.clone();
        mail.set(
            name,
            lua.create_function(move |lua, (_, index): (Table, usize)| {
                index
                    .checked_sub(1)
                    .and_then(|i| s.0.borrow().mailbox.headers(sent).get(i).cloned())
                    .map(|h| header(lua, &h))
                    .transpose()
            })?,
        )?;
    }
    let s = state.clone();
    mail.set(
        "GetCountUnreadMail",
        lua.create_function(move |_, _: Table| Ok(s.0.borrow().mailbox.unread.received))?,
    )?;
    let s = state.clone();
    mail.set(
        "GetAttachedItemCountById",
        lua.create_function(move |_, (_, sent, id): (Table, bool, String)| {
            Ok(id
                .parse()
                .ok()
                .and_then(|id| s.0.borrow().mailbox.header(sent, id).map(|h| h.attachments))
                .unwrap_or(0))
        })?,
    )?;
    for (name, sent) in [("ReadReceivedMailById", false), ("ReadPostedMailById", true)] {
        let s = state.clone();
        mail.set(
            name,
            lua.create_function(move |_, (_, id): (Table, String)| {
                if let Ok(id) = id.parse::<i64>() {
                    if id > 0 {
                        let mut v = s.0.borrow_mut();
                        v.selected = Some((sent, id));
                        v.queue(MailCommand::Read { is_sent: sent, mail_id: id });
                    }
                }
                Ok(())
            })?,
        )?;
    }
    let s = state.clone();
    mail.set("GetCurrentMailBody", lua.create_function(move |lua, _: Table| body_table(lua, &s.0.borrow()))?)?;
    let s = state.clone();
    mail.set(
        "IsExistCurrentMailBody",
        lua.create_function(move |_, _: Table| Ok(s.0.borrow().body().is_some()))?,
    )?;
    let s = state.clone();
    mail.set(
        "ClearCurrentMail",
        lua.create_function(move |_, _: Table| {
            s.0.borrow_mut().selected = None;
            Ok(())
        })?,
    )?;
    for (name, kind) in [
        ("IsUserMail", 0),
        ("IsBillingMail", 6),
        ("IsBalanceReceiptMail", 7),
        ("IsTaxRateChangedMail", 8),
        ("IsNationReceiptMail", 24),
        ("IsNationRequestFriendMail", 25),
    ] {
        let s = state.clone();
        mail.set(
            name,
            lua.create_function(move |_, (_, id): (Table, String)| {
                let ty = id.parse().ok().and_then(|id| s.0.borrow().mailbox.header(false, id).map(|h| h.mail_type));
                Ok(ty.is_some_and(|ty| if kind == 0 { matches!(ty, 1..=3) } else { ty == kind }))
            })?,
        )?;
    }
    let s = state.clone();
    mail.set(
        "GetCurrentMailType",
        lua.create_function(move |_, _: Table| Ok(s.0.borrow().body().map_or(0, |b| b.mail_type)))?,
    )?;
    for (name, field) in [("GetCurrentMailMoneyStr", 0), ("GetCurrentMailAAPointStr", 1)] {
        let s = state.clone();
        mail.set(
            name,
            lua.create_function(move |_, _: Table| Ok(s.0.borrow().body().map_or(0, |b| if field == 0 { b.money } else { 0 }).to_string()))?,
        )?;
    }
    for (name, all) in [("GetCurrentMailItemCount", false), ("GetCurrentMailAttachedItemCount", true)] {
        let s = state.clone();
        let b = bridge.clone();
        mail.set(
            name,
            lua.create_function(move |_, _: Table| {
                let v = s.0.borrow();
                Ok(if all {
                    v.selected
                        .and_then(|(sent, id)| v.mailbox.header(sent, id))
                        .map_or(0, |h| usize::from(h.attachments))
                } else {
                    let d = b.borrow();
                    v.body().map_or(0, |body| {
                        body.items
                            .iter()
                            .flatten()
                            .filter(|a| a.item.id != 0 && a.item.template_id != 0 && d.game.get().is_some_and(|g| g.items.contains_key(&a.item.template_id)))
                            .count()
                    })
                })
            })?,
        )?;
    }
    for (name, draft) in [("GetCurrentMailItem", false), ("GetMailItem", true)] {
        let s = state.clone();
        let b = bridge.clone();
        mail.set(
            name,
            lua.create_function(move |lua, (_, index): (Table, usize)| {
                let v = s.0.borrow();
                let item = index.checked_sub(1).and_then(|i| {
                    if draft {
                        v.draft.get(i).and_then(Option::as_ref).map(|d| &d.item)
                    } else {
                        v.body()?.items.get(i).and_then(Option::as_ref).map(|a| &a.item)
                    }
                });
                item.map(|item| item_info(lua, &b, item)).transpose().map(Option::flatten)
            })?,
        )?;
    }
    let s = state.clone();
    mail.set(
        "TakeCurrentMailMoney",
        lua.create_function(move |_, _: Table| {
            let mut v = s.0.borrow_mut();
            let id = v.current_inbox().filter(|_| v.body().is_some_and(|b| b.money > 0));
            Ok(id.is_some_and(|id| v.queue(MailCommand::TakeMoney { mail_id: id })))
        })?,
    )?;
    let s = state.clone();
    mail.set(
        "TakeCurrentMailItem",
        lua.create_function(move |_, (_, index): (Table, usize)| {
            let mut v = s.0.borrow_mut();
            let attachment = index.checked_sub(1).and_then(|i| v.body()?.items.get(i)?.clone());
            Ok(v.current_inbox()
                .zip(attachment)
                .is_some_and(|(id, attachment)| v.queue(MailCommand::TakeItem { mail_id: id, attachment })))
        })?,
    )?;
    let s = state.clone();
    mail.set(
        "DeleteMailById",
        lua.create_function(move |_, (_, sent, id): (Table, bool, String)| {
            if let Ok(id) = id.parse() {
                s.0.borrow_mut().queue(MailCommand::Delete { is_sent: sent, mail_id: id });
            }
            Ok(())
        })?,
    )?;
    let s = state.clone();
    mail.set(
        "DeleteCurrentMail",
        lua.create_function(move |_, _: Table| {
            let mut v = s.0.borrow_mut();
            if let Some((sent, id)) = v.selected {
                v.queue(MailCommand::Delete { is_sent: sent, mail_id: id });
            }
            Ok(())
        })?,
    )?;
    let s = state.clone();
    mail.set(
        "TakeAttachmentSequentially",
        lua.create_function(move |_, (_, id): (Table, String)| {
            if let Ok(id) = id.parse() {
                s.0.borrow_mut().queue(MailCommand::TakeAll { mail_id: id });
            }
            Ok(())
        })?,
    )?;
    let s = state.clone();
    mail.set(
        "PayCurrentMailChargeMoney",
        lua.create_function(move |_, (_, auto): (Table, Option<bool>)| {
            let mut v = s.0.borrow_mut();
            if let Some(id) = v.current_inbox().filter(|_| v.body().is_some_and(|b| b.mail_type == 6)) {
                v.queue(MailCommand::PayHousingTax {
                    mail_id: id,
                    auto_use_aa_point: auto.unwrap_or(false),
                });
            }
            Ok(())
        })?,
    )?;
    for name in ["GetCurrencyForAttachment", "GetCurrencyForFee"] {
        mail.set(name, lua.create_function(|_, _: Table| Ok(0))?)?;
    }
    // Return has no 1.2 lane opcode. Do not report a successful return.
    mail.set(
        "ReturnMailById",
        lua.create_function(|lua, (_, _id): (Table, String)| {
            aa_ui::lua::fire_ui_event(
                lua,
                "SYSMSG",
                vec![mlua::Value::String(lua.create_string("Mail return is unavailable on this server.")?)],
            );
            Ok(false)
        })?,
    )?;
    install_draft(lua, &mail, &state)?;
    lua.load(include_str!("mail.lua")).set_name("@mail.lua").exec()?;
    Ok(())
}

fn install_draft(lua: &Lua, mail: &Table, state: &State) -> mlua::Result<()> {
    // This predicate is an account/free-trial gate, not the mailbox-distance gate. No trial
    // entitlement is exposed by the current lane. Sending separately validates the mailbox.
    mail.set("CanSendMail", lua.create_function(|_, _: Table| Ok(true))?)?;
    let s = state.clone();
    mail.set(
        "GetMailItemCount",
        lua.create_function(move |_, _: Table| Ok(s.0.borrow().draft.iter().flatten().count()))?,
    )?;
    let s = state.clone();
    mail.set(
        "GetMailEmptyItemIndex",
        lua.create_function(move |_, _: Table| Ok(s.0.borrow().draft.iter().position(Option::is_none).map(|i| i + 1)))?,
    )?;
    let s = state.clone();
    mail.set(
        "ClearMailItem",
        lua.create_function(move |lua, (_, index): (Table, usize)| {
            let mut v = s.0.borrow_mut();
            let Some(index) = index.checked_sub(1).filter(|i| *i < 10) else {
                return Ok(false);
            };
            let removed = v.draft[index].take();
            let exists = removed.is_some();
            if exists {
                v.draft_changes.push(index + 1);
            }
            drop(v);
            if let Some(item) = removed {
                let bag: Table = lua.globals().get("X2Bag")?;
                bag.call_method::<()>("UnlockSlot", usize::from(item.slot) + 1)?;
            }
            Ok(exists)
        })?,
    )?;
    let s = state.clone();
    mail.set(
        "PlaceMailItemFromBag",
        lua.create_function(move |lua, (_, index, bag_id, slot): (Table, usize, i64, usize)| {
            if bag_id != 1 {
                return Ok(false);
            }
            let bag: Table = lua.globals().get("X2Bag")?;
            if bag.call_method::<bool>("IsLocked", slot)? {
                return Ok(false);
            }
            let accepted = s.0.borrow_mut().place(index, slot);
            if accepted {
                bag.call_method::<()>("LockSlot", slot)?;
            }
            Ok(accepted)
        })?,
    )?;
    let s = state.clone();
    mail.set(
        "PlaceMailItem",
        lua.create_function(move |lua, (_, index): (Table, usize)| {
            let held = aa_ui::lua::ui(lua).drag.held.as_ref().map(|h| h.table.clone());
            let Some(h) = held else { return Ok(false) };
            if h.get::<String>("kind").unwrap_or_default() != "bag" || h.get::<bool>("partial").unwrap_or(false) {
                return Ok(false);
            }
            let slot: usize = h.get("slot")?;
            let uid: String = h.get("uid")?;
            let matches =
                s.0.borrow()
                    .inventory
                    .bag
                    .get(slot.saturating_sub(1))
                    .and_then(Option::as_ref)
                    .is_some_and(|item| item.id.to_string() == uid);
            let bag: Table = lua.globals().get("X2Bag")?;
            if !matches || bag.call_method::<bool>("IsLocked", slot)? {
                return Ok(false);
            }
            let accepted = s.0.borrow_mut().place(index, slot);
            if accepted {
                bag.call_method::<()>("LockSlot", slot)?;
                aa_ui::lua::drag::clear(lua);
            }
            Ok(accepted)
        })?,
    )?;
    let s = state.clone();
    mail.set(
        "SendMail",
        lua.create_function(move |lua, (_, input): (Table, Table)| {
            let mut v = s.0.borrow_mut();
            let backup = DraftBackup::capture(&input, v.draft.clone())?;
            if v.sending {
                return Ok(v.refuse(backup, "A mail send is still awaiting a reply."));
            }
            let number = |name: &str| {
                let text = input.get::<Option<String>>(name).ok().flatten().unwrap_or_default();
                if text.is_empty() {
                    Some(0)
                } else {
                    text.parse::<i64>().ok().filter(|n| *n >= 0)
                }
            };
            let money = number("gold")
                .and_then(|n| n.checked_mul(10_000))
                .and_then(|n| {
                    number("silver")
                        .filter(|s| *s < 100)
                        .and_then(|s| s.checked_mul(100))
                        .and_then(|s| n.checked_add(s))
                })
                .and_then(|n| number("copper").filter(|c| *c < 100).and_then(|c| n.checked_add(c)))
                .and_then(|n| i32::try_from(n).ok());
            let Some(money) = money.filter(|m| *m >= 0) else {
                return Ok(v.refuse(backup, "Invalid mail money amount."));
            };
            let mailbox = input.get::<String>("doodadId").ok().and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
            if mailbox == 0 || (mailbox != v.nearest && !v.nearby.contains(&mailbox)) {
                return Ok(v.refuse(backup, "Move closer to the mailbox before sending."));
            }
            let draft: Vec<_> = v.draft.iter().flatten().cloned().collect();
            let command = MailCommand::Send(SendMail {
                mail_type: match input.get::<String>("type")?.as_str() {
                    "mail_normal" => 1,
                    "mail_express" => 2,
                    _ => return Ok(v.refuse(backup, "Unsupported outgoing mail type.")),
                },
                receiver: input.get::<String>("receiver")?,
                title: input.get("title")?,
                text: input.get("text")?,
                money,
                items: draft.iter().map(|d| MailSlot { container: 2, slot: d.slot }).collect(),
                doodad_obj_id: mailbox,
            });
            if !v.queue(command) {
                return Ok(v.refuse(backup, "Mail request could not be sent."));
            }
            v.commands.last_mut().unwrap().expected = draft.iter().map(|d| (d.slot, d.item.id)).collect();
            v.sending = true;
            v.send_backup = Some(backup);
            v.restore = None;
            drop(v);
            lua.load("if mailFrame and mailFrame.writeWindow then mailFrame.writeWindow:Enable(false) end")
                .exec()?;
            Ok(true)
        })?,
    )?;
    Ok(())
}

fn refresh(rt: &UiRuntime, sent: bool, read: bool, kind: i64) {
    rt.fire_event(if sent { "MAIL_SENTBOX_UPDATE" } else { "MAIL_INBOX_UPDATE" }, (read, kind));
}

fn failure(rt: &UiRuntime, message: &str) {
    warn!("hud: mailbox: {message}");
    rt.fire_event("SYSMSG", message.to_owned());
}

/// Native393b1040 opens the selected NaviOpenMailbox locally with (numeric object ID,true).
/// Ctrl-E additionally permits inbox reading without a nearby mailbox; Send validates proximity.
pub(super) fn toggle(rt: &UiRuntime) {
    let Some(state) = rt.lua.app_data_ref::<State>().map(|s| s.clone()) else {
        return;
    };
    let visible = rt
        .lua
        .load("return IsActivatedMailBox and IsActivatedMailBox() or false")
        .eval::<bool>()
        .unwrap_or(false);
    let id = state.0.borrow().nearest;
    if !visible {
        state.0.borrow_mut().queue(MailCommand::List);
    }
    rt.fire_event("TOGGLE_MAIL", (id, true));
}

#[allow(clippy::too_many_arguments)]
fn sync(
    rt: Option<NonSend<UiRuntime>>,
    bridge: NonSend<HudBridge>,
    mailbox: Res<Mailbox>,
    inventory: Res<Inventory>,
    doodads: Res<Doodads>,
    mut events: MessageReader<NetEvent>,
    mut net: MessageWriter<NetCommand>,
) {
    let Some(rt) = rt else {
        events.clear();
        return;
    };
    let Some(state) = rt.lua.app_data_ref::<State>().map(|s| s.clone()) else {
        events.clear();
        return;
    };
    let signals: Vec<_> = events
        .read()
        .filter_map(|e| if let Event::Mail(m) = &e.0 { Some(m.clone()) } else { None })
        .collect();
    let now = Instant::now();
    let nearby = {
        let d = bridge.0.borrow();
        d.game
            .get()
            .map(|g| {
                doodads
                    .by_id
                    .values()
                    .filter(|o| g.mail.groups.contains(&o.func_group_id))
                    .filter_map(|o| {
                        let distance = (o.pos.x - d.pos.x).powi(2) + (o.pos.y - d.pos.y).powi(2);
                        (distance <= 25.0).then_some((distance, o.obj_id))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    let nearest = nearby.iter().min_by(|a, b| a.0.total_cmp(&b.0)).map_or(0, |(_, id)| *id);
    let nearby: HashSet<u32> = nearby.into_iter().map(|(_, id)| id).collect();
    let (before, changes, expired, expired_send, commands) = {
        let mut v = state.0.borrow_mut();
        let before = v.mailbox.clone();
        if !v.initialized || mailbox.is_changed() {
            v.mailbox = mailbox.clone();
        }
        if !v.initialized || inventory.is_changed() {
            v.inventory = inventory.clone();
        }
        v.nearest = nearest;
        v.nearby = nearby.clone();
        v.initialized = true;
        let mut expired = false;
        let mut expired_send = false;
        v.pending.retain(|(command, at)| {
            let age = now.saturating_duration_since(*at);
            if age >= Duration::from_secs(20) {
                expired = true;
                expired_send |= matches!(command, MailCommand::Send(_));
                false
            } else {
                age < Duration::from_millis(500) || mailbox.pending.contains(command)
            }
        });
        (
            before,
            std::mem::take(&mut v.draft_changes),
            expired,
            expired_send,
            std::mem::take(&mut v.commands),
        )
    };
    for index in changes {
        rt.fire_event("MAIL_WRITE_ITEM_UPDATE", index);
    }
    if expired {
        failure(&rt, "Mail request timed out. Refresh the mailbox to retry.");
        if expired_send {
            state.0.borrow_mut().failed_send();
        }
    }
    let mut before = before;
    for event in signals {
        let attachment_before = if let MailEvent::AttachmentTaken { mail_id, .. } = &event {
            before.body(false, *mail_id).cloned()
        } else {
            None
        };
        before.apply(&Event::Mail(event.clone()));
        match event {
            MailEvent::List { is_sent, .. } => refresh(&rt, is_sent, false, 2),
            MailEvent::ListEnd { .. } => {
                refresh(&rt, false, false, 3);
                refresh(&rt, true, false, 3);
            }
            MailEvent::Body { is_sent, is_prepare, body, .. } => {
                if !is_prepare && state.0.borrow().selected == Some((is_sent, body.mail_id)) {
                    refresh(&rt, is_sent, true, 3);
                }
            }
            MailEvent::Got { .. } | MailEvent::Unread(_) => refresh(&rt, false, false, 3),
            MailEvent::Sent { .. } => {
                {
                    let mut v = state.0.borrow_mut();
                    v.sending = false;
                    v.send_backup = None;
                }
                let _ = rt
                    .lua
                    .load("if mailFrame and mailFrame.writeWindow then mailFrame.writeWindow:Enable(true) end")
                    .exec();
                rt.fire_event("MAIL_SENT_SUCCESS", ());
                refresh(&rt, true, false, 3);
            }
            MailEvent::AttachmentTaken {
                mail_id,
                money,
                sequential,
                items,
                ..
            } => {
                if let Some(old) = attachment_before {
                    for item in items {
                        if let Some(index) = old.items.iter().position(|a| a.as_ref().is_some_and(|a| a.item.id == item.item_id)) {
                            rt.fire_event("MAIL_INBOX_ITEM_TAKEN", index + 1);
                        }
                    }
                }
                if money {
                    rt.fire_event("MAIL_INBOX_MONEY_TAKEN", ());
                }
                let empty = mailbox.header(false, mail_id).is_some_and(|h| h.attachments == 0);
                if sequential && empty && state.0.borrow_mut().completed_claims.insert(mail_id) {
                    rt.fire_event("MAIL_INBOX_ATTACHMENT_TAKEN_ALL", mail_id.to_string());
                }
                refresh(&rt, false, false, 3);
            }
            MailEvent::Deleted { is_sent, mail_id, .. } | MailEvent::Removed { is_sent, mail_id } => {
                let selected = state.0.borrow().selected == Some((is_sent, mail_id));
                if selected {
                    state.0.borrow_mut().selected = None;
                    if let Err(e) = rt
                        .lua
                        .load("if mailFrame and mailFrame.readWindow then mailFrame.readWindow:Show(false) end")
                        .exec()
                    {
                        warn!("hud: mail close: {e}");
                    }
                }
                refresh(&rt, is_sent, false, 3);
            }
            MailEvent::StatusUpdated { is_sent, .. } => refresh(&rt, is_sent, false, 3),
            MailEvent::ReceiverOpened { .. } => refresh(&rt, true, false, 3),
            MailEvent::Returned { .. } => {
                rt.fire_event("MAIL_RETURNED", ());
                refresh(&rt, false, false, 3);
            }
            MailEvent::ChargePaid { .. } => rt.fire_event("MAIL_INBOX_TAX_PAID", ()),
            MailEvent::Failed { code, .. } => {
                state.0.borrow_mut().failed_send();
                failure(&rt, &format!("Mail request failed (code {code})."));
            }
            MailEvent::RequestRejected { command, message } => {
                if matches!(command, MailCommand::Send(_)) {
                    state.0.borrow_mut().failed_send();
                }
                failure(&rt, &message);
            }
            MailEvent::Requested(_) => {}
        }
    }
    for queued in commands {
        if queued
            .expected
            .iter()
            .any(|(slot, id)| inventory.bag.get(*slot as usize).and_then(Option::as_ref).is_none_or(|item| item.id != *id))
        {
            state.0.borrow_mut().failed_send();
            failure(&rt, "A mail attachment changed. Attach the item again.");
            continue;
        }
        if let MailCommand::Send(mail) = &queued.command {
            if !nearby.contains(&mail.doodad_obj_id) {
                state.0.borrow_mut().failed_send();
                failure(&rt, "Move closer to the mailbox before sending.");
                continue;
            }
        }
        state.0.borrow_mut().pending.push((queued.command.clone(), now));
        net.write(NetCommand(Command::Mail(queued.command)));
    }
    restore_draft(&rt, &state);
}

fn restore_draft(rt: &UiRuntime, state: &State) {
    let (backup, notices) = {
        let mut v = state.0.borrow_mut();
        (v.restore.take(), std::mem::take(&mut v.notices))
    };
    for notice in notices {
        failure(rt, &notice);
    }
    let Some(backup) = backup else { return };
    let available = rt
        .lua
        .load("return mailFrame~=nil and mailFrame.writeWindow~=nil")
        .eval::<bool>()
        .unwrap_or(false);
    if !available {
        state.0.borrow_mut().restore = Some(backup);
        return;
    }
    let result = (|| -> mlua::Result<()> {
        let fields = rt.lua.create_table()?;
        for (name, value) in &backup.fields {
            fields.set(*name, value.as_str())?;
        }
        rt.lua.globals().set("_AA_MAIL_RESTORE", fields)?;
        rt.lua.load("local w=mailFrame.writeWindow; local f=_AA_MAIL_RESTORE; w:Enable(true); w.roleEdit:SetText(f.receiver); w.titleEdit:SetText(f.title); w.content:SetText(f.text); local m=w.defaultFrame.moneyEditsWindow; m.goldEdit:SetText(f.gold); m.silverEdit:SetText(f.silver); m.copperEdit:SetText(f.copper); w.mailType=f.type; w.checks:Check(f.type=='mail_normal' and 2 or 1); _AA_MAIL_RESTORE=nil").exec()?;
        let slots = {
            let mut v = state.0.borrow_mut();
            v.draft = backup.items.map(|item| {
                item.filter(|item| {
                    v.inventory
                        .bag
                        .get(item.slot as usize)
                        .and_then(Option::as_ref)
                        .is_some_and(|current| current.id == item.item.id)
                })
            });
            v.draft_changes.extend(1..=10);
            v.draft.iter().flatten().map(|item| usize::from(item.slot) + 1).collect::<Vec<_>>()
        };
        let bag: Table = rt.lua.globals().get("X2Bag")?;
        for slot in slots {
            bag.call_method::<()>("LockSlot", slot)?;
        }
        Ok(())
    })();
    if let Err(e) = result {
        warn!("hud: restore mail draft: {e}");
    }
}

pub fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        sync.after(super::sync_hud)
            .before(aa_ui::UiFrame)
            .run_if(in_state(Screen::InGame))
            .run_if(super::hud_up),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use aa_net::mail::{CachedMailBody, MailAttachment};

    fn setup() -> (UiRuntime, Shared, State) {
        let mut rt = UiRuntime::new("en_us", 1280.0, 720.0).unwrap();
        let bridge = Rc::new(RefCell::new(super::super::api::HudData::default()));
        let mut game = super::super::data::GameData::default();
        game.items.insert(5, super::super::data::ItemInfo::default());
        assert!(bridge.borrow().game.0.set(game).is_ok());
        super::super::api::install(&mut rt, &bridge).unwrap();
        let state = rt.lua.app_data_ref::<State>().unwrap().clone();
        (rt, bridge, state)
    }

    fn attachment() -> MailAttachment {
        let mut w = aa_net::wire::Writer::new();
        w.u32(5)
            .u64(9_007_199_254_740_993)
            .u8(0)
            .u8(0)
            .i32(4)
            .u8(0)
            .i64(0)
            .i32(0)
            .u32(0)
            .u8(1)
            .i64(0)
            .i64(0);
        MailAttachment::from_wire(w.as_bytes()).unwrap()
    }

    #[test]
    fn mailbox_queries_and_claims_preserve_string_ids_and_exact_attachments() -> mlua::Result<()> {
        let (rt, _, state) = setup();
        let id = 9_007_199_254_740_999;
        {
            let mut v = state.0.borrow_mut();
            v.mailbox.received.push(MailHeader {
                mail_id: id,
                mail_type: 2,
                title: "Hello".into(),
                sender_name: "Peer".into(),
                receiver_name: "Self".into(),
                attachments: 2,
                ..default()
            });
            v.mailbox.unread.received = 1;
            let mut b = MailBody {
                mail_id: id,
                mail_type: 2,
                title: "Hello".into(),
                text: "A letter".into(),
                receiver_name: "Self".into(),
                money: 5,
                billing_amount: 7,
                send_date: 1_792_276_000,
                recv_date: 1_792_276_001,
                ..default()
            };
            b.items[3] = Some(attachment());
            let mut unknown = attachment();
            unknown.item.template_id = u32::MAX;
            b.items[9] = Some(unknown);
            v.mailbox.bodies.push(CachedMailBody { is_sent: false, body: b });
        }
        rt.lua.load("assert(X2Mail:GetReceivedMailCount()==1 and X2Mail:GetCountUnreadMail()==1); local h=X2Mail:GetReceivedMailTitleInfo(1); assert(h.mail_id=='9007199254740999' and h.hasItem and not h.is_read); X2Mail:ReadReceivedMailById(h.mail_id); local b=X2Mail:GetCurrentMailBody(); assert(b.sender=='Peer' and b.text=='A letter' and b.chargeMoney==7 and type(b.sendDate.year)=='number'); assert(X2Mail:GetCurrentMailMoneyStr()=='5'); assert(X2Mail:GetCurrentMailItemCount()==1 and X2Mail:GetCurrentMailAttachedItemCount()==2); assert(X2Mail:TakeCurrentMailMoney()); assert(not X2Mail:TakeCurrentMailMoney()); assert(X2Mail:TakeCurrentMailItem(4)); assert(not X2Mail:TakeCurrentMailItem(4)); assert(not X2Mail:TakeCurrentMailItem(1))").exec()?;
        let v = state.0.borrow();
        assert_eq!(v.commands.len(), 3);
        assert_eq!(v.mailbox.received[0].attachments, 2);
        assert_eq!(v.mailbox.body(false, id).unwrap().money, 5);
        assert!(matches!(&v.commands[2].command,MailCommand::TakeItem{mail_id,attachment:a} if *mail_id==id&&*a==attachment()));
        Ok(())
    }

    #[test]
    fn outgoing_mail_uses_physical_whole_stack_identity_and_checked_copper() -> mlua::Result<()> {
        let (rt, _, state) = setup();
        {
            let mut v = state.0.borrow_mut();
            v.nearest = 42;
            v.inventory.bag.resize(8, None);
            v.inventory.bag[4] = Some(attachment().item);
        }
        rt.lua.load("HUD.bag[5]={uid='9007199254740993',id=5,count=4}; assert(not X2Mail:PlaceMailItemFromBag(1,2,5)); assert(X2Mail:PlaceMailItemFromBag(1,1,5)); assert(X2Bag:IsLocked(5)); assert(not X2Mail:PlaceMailItemFromBag(2,1,5)); assert(X2Mail:GetMailItemCount()==1); assert(not X2Mail:SendMail({receiver='Peer',title='Invalid',text='',gold='-1',silver='99',copper='1',doodadId='42',type='mail_normal'})); assert(X2Mail:SendMail({receiver='Peer',title='Hello',text='Letter',gold='',silver='1',copper='5',doodadId='42',type='mail_express',withReceiver='1'})); X2Mail:ClearMailItem(1); assert(not X2Bag:IsLocked(5)); assert(X2Mail:GetMailItemCount()==0)").exec()?;
        let v = state.0.borrow();
        assert_eq!(v.commands.len(), 1);
        let q = &v.commands[0];
        assert_eq!(q.expected, [(4, 9_007_199_254_740_993)]);
        assert!(matches!(&q.command,MailCommand::Send(m) if m.money==105&&m.mail_type==2&&m.items==[MailSlot{container:2,slot:4}]&&m.doodad_obj_id==42));
        assert_eq!(v.inventory.bag[4].as_ref().unwrap().count, 4);
        Ok(())
    }

    #[test]
    #[ignore = "loads the original world-stage assets; run in an isolated fixture process"]
    fn original_mailbox_read_claim_and_write_buttons() -> Result<(), Box<dyn std::error::Error>> {
        let mut rt = UiRuntime::new("en_us", 1280.0, 720.0)?;
        rt.load_ui_texts(std::path::Path::new(aa_anim::db::DEFAULT_DB))?;
        let bridge = Rc::new(RefCell::new(super::super::api::HudData::default()));
        {
            let mut d = bridge.borrow_mut();
            d.snapshot.unit_id = "100".into();
            d.snapshot.name = "Self".into();
            d.snapshot.hp = 100;
            d.snapshot.max_hp = 100;
            d.snapshot.level = 1;
            assert!(d.game.0.set(super::super::data::GameData::load(aa_anim::db::DEFAULT_DB)?).is_ok());
        }
        super::super::api::install(&mut rt, &bridge)?;
        rt.run_stage(aa_ui::Stage::World);
        assert!(rt.errors.is_empty(), "{:?}", rt.errors);
        super::super::api::enter_world(&rt, &bridge, &aa_net::state::Chat::default());
        let state = rt.lua.app_data_ref::<State>().unwrap().clone();
        {
            let mut v = state.0.borrow_mut();
            v.nearest = 42;
            v.mailbox.received.push(MailHeader {
                mail_id: 123,
                mail_type: 2,
                title: "Fixture letter".into(),
                sender_name: "Peer".into(),
                receiver_name: "Self".into(),
                attachments: 1,
                ..default()
            });
            v.mailbox.unread.received = 1;
        }
        toggle(&rt);
        refresh(&rt, false, false, 3);
        rt.lua
            .load("assert(mailFrame:IsVisible()); local list=mailFrame.tab.window[1].listCtrl; assert(list.mailIDs[1]=='123'); list:OnSelChanged(1,true)")
            .exec()?;
        assert!(matches!(
            state.0.borrow().commands.last().map(|q| &q.command),
            Some(MailCommand::Read { is_sent: false, mail_id: 123 })
        ));
        {
            let mut v = state.0.borrow_mut();
            v.mailbox.received[0].status = 1;
            v.mailbox.bodies.push(CachedMailBody {
                is_sent: false,
                body: MailBody {
                    mail_id: 123,
                    mail_type: 2,
                    title: "Fixture letter".into(),
                    text: "Original mailbox fixture".into(),
                    receiver_name: "Self".into(),
                    money: 5,
                    send_date: 1_792_276_000,
                    recv_date: 1_792_276_001,
                    ..default()
                },
            });
        }
        refresh(&rt, false, true, 3);
        rt.lua.load("local w=GetReadMailWindow(); assert(w:IsVisible() and w.mailId=='123'); assert(w.content:GetText()=='Original mailbox fixture'); w.defaultFrame.moneyReceiveButton:OnClick('LeftButton')").exec()?;
        assert!(matches!(
            state.0.borrow().commands.last().map(|q| &q.command),
            Some(MailCommand::TakeMoney { mail_id: 123 })
        ));
        let widget: mlua::Value = rt.lua.load("return mailFrame.writeButton").eval()?;
        let id = aa_ui::lua::value_id(&rt.lua, &widget).unwrap();
        aa_ui::lua::call_handler(&rt.lua, id, "OnClick", vec![mlua::Value::String(rt.lua.create_string("LeftButton")?)]);
        rt.lua.load("local w=GetWriteMailWindow(); assert(w:IsVisible()); w.roleEdit:SetText('Peer'); w.titleEdit:SetText('Reply'); w.content:SetText('Native send button'); w.sendButton:OnClick('LeftButton')").exec()?;
        assert!(
            matches!(state.0.borrow().commands.last().map(|q|&q.command),Some(MailCommand::Send(m)) if m.receiver=="Peer"&&m.title=="Reply"&&m.text=="Native send button"&&m.doodad_obj_id==42)
        );
        // A rejected send restores what the original button cleared, without changing inventory.
        state.0.borrow_mut().failed_send();
        restore_draft(&rt, &state);
        rt.lua.load("local w=GetWriteMailWindow(); assert(w:IsEnabled() and w.roleEdit:GetText()=='Peer' and w.titleEdit:GetText()=='Reply' and w.content:GetText()=='Native send button')").exec()?;
        assert!(rt.errors.is_empty(), "{:?}", rt.errors);
        Ok(())
    }
}
