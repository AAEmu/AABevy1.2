//! Original party/raid frames and actions over server-confirmed membership.
use aa_net::party::{LootingRule, Party, PartyCommand, PartyMember};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use mlua::{Lua, Table};

use super::api::{HudBridge, Request, Shared};
use crate::Screen;

mod members;
mod ping;

pub fn plugin(app: &mut App) {
    ping::plugin(app);
    app.add_systems(
        Update,
        sync.after(super::sync_hud)
            .before(super::api::handle_requests)
            .run_if(super::hud_up)
            .run_if(in_state(Screen::InGame)),
    );
}

pub fn install(lua: &Lua, hud: &Table, bridge: &Shared) -> mlua::Result<()> {
    ping::install(lua, hud)?;
    bridge.borrow_mut().party_pushed = None;
    push(
        lua,
        hud,
        &bridge.borrow().party,
        bridge.borrow().character_id,
        &members::Extras::default(),
    )?;
    let b = bridge.clone();
    hud.set(
        "team_request",
        lua.create_function(
            move |_,
                  (kind, index, name, flag): (
                String,
                Option<u32>,
                Option<String>,
                Option<bool>,
            )| {
                let mut d = b.borrow_mut();
                let p = &d.party;
                let member = || {
                    p.members
                        .iter()
                        .find(|m| Some(u32::from(m.slot) + 1) == index)
                };
                let owner = p.owner_id == d.character_id && p.id != 0;
                let command =
                    match kind.as_str() {
                        "invite"
                            if name
                                .as_ref()
                                .is_some_and(|n| !n.trim().is_empty() && n.len() <= 128) =>
                        {
                            Some(PartyCommand::Invite {
                                team_id: p.id,
                                is_party: flag.unwrap_or(true),
                                target_name: name.unwrap().trim().to_string(),
                            })
                        }
                        "reply" => p
                            .pending_invite
                            .clone()
                            .filter(|i| Some(i.sender_id) == index)
                            .map(|invite| PartyCommand::Reply {
                                invite,
                                accept: flag.unwrap_or(false),
                            }),
                        "leave" if p.id != 0 => Some(PartyCommand::Leave {
                            team_id: p.id,
                            character_id: d.character_id,
                        }),
                        "dismiss" if owner => Some(PartyCommand::Dismiss { team_id: p.id }),
                        "raid" if owner && p.is_party => {
                            Some(PartyCommand::ConvertToRaid { team_id: p.id })
                        }
                        "area" if p.id == 0 || owner => Some(PartyCommand::InviteArea {
                            team_id: p.id,
                            is_party: p.id == 0 || p.is_party,
                        }),
                        "role"
                            if p.id != 0
                                && p.member(d.character_id).is_some()
                                && index.is_some_and(|r| r <= 3) =>
                        {
                            Some(PartyCommand::SetRole {
                                team_id: p.id,
                                member_id: d.character_id,
                                role: index.unwrap() as u8,
                            })
                        }
                        "kick" if owner => member().filter(|m| m.id != d.character_id).map(|m| {
                            PartyCommand::Kick {
                                team_id: p.id,
                                member_id: m.id,
                            }
                        }),
                        "owner" if owner => member()
                            .filter(|m| m.id != d.character_id && m.online)
                            .map(|m| PartyCommand::MakeOwner {
                                team_id: p.id,
                                member_id: m.id,
                            }),
                        "target" => {
                            let obj_id = if index == Some(0) {
                                d.snapshot.unit_id.parse().ok()
                            } else {
                                member().filter(|m| m.online).map(|m| m.obj_id)
                            };
                            if let Some(obj_id) = obj_id.filter(|id| *id != 0) {
                                d.requests.push(Request::Target { obj_id });
                            }
                            None
                        }
                        _ => None,
                    };
                if let Some(command) = command {
                    d.requests.push(Request::Party(command));
                }
                Ok(())
            },
        )?,
    )?;
    let b = bridge.clone();
    hud.set(
        "team_move",
        lua.create_function(move |_, (from, to, group): (u8, u8, Option<bool>)| {
            let mut d = b.borrow_mut();
            let p = &d.party;
            if p.id == 0 || p.is_party || p.owner_id != d.character_id {
                return Ok(());
            }
            let Some(from) = from.checked_sub(1).filter(|i| *i < 50) else {
                return Ok(());
            };
            let to = if group.unwrap_or(false) {
                to.checked_sub(1).filter(|g| *g < 10).and_then(|g| {
                    (g * 5..g * 5 + 5).find(|slot| !p.members.iter().any(|m| m.slot == *slot))
                })
            } else {
                to.checked_sub(1).filter(|i| *i < 50)
            };
            if let Some(command) = to.and_then(|to| p.plan_move(from, to)) {
                d.requests.push(Request::Party(command));
            }
            Ok(())
        })?,
    )?;
    let b = bridge.clone();
    hud.set(
        "team_loot",
        lua.create_function(
            move |_, (method, name, grade, roll): (u8, Option<String>, u8, mlua::Value)| {
                let mut d = b.borrow_mut();
                let p = &d.party;
                if p.id == 0
                    || p.owner_id != d.character_id
                    || !matches!(method, 0..=2 | 255)
                    || grade > 11
                {
                    return Ok(());
                }
                let master = if method == 2 {
                    let Some(m) = name
                        .as_deref()
                        .and_then(|name| p.members.iter().find(|m| m.name == name))
                    else {
                        return Ok(());
                    };
                    m.id
                } else {
                    0
                };
                let roll = match roll {
                    mlua::Value::Boolean(v) => v,
                    mlua::Value::Integer(v) => v != 0,
                    mlua::Value::Number(v) => v != 0.0,
                    _ => p.looting_rule.roll_for_bind_on_pickup,
                };
                let rule = LootingRule {
                    method,
                    minimum_grade: super::store::ui_grade(grade),
                    loot_master: master,
                    roll_for_bind_on_pickup: roll,
                };
                let old = p.looting_rule;
                let flags = u8::from(old.method != rule.method)
                    | (u8::from(old.minimum_grade != rule.minimum_grade) << 1)
                    | (u8::from(old.loot_master != rule.loot_master) << 2)
                    | (u8::from(old.roll_for_bind_on_pickup != rule.roll_for_bind_on_pickup) << 3);
                if flags != 0 {
                    let team_id = p.id;
                    d.requests
                        .push(Request::Party(PartyCommand::ChangeLootingRule {
                            team_id,
                            rule,
                            flags,
                        }));
                }
                Ok(())
            },
        )?,
    )?;
    let b = bridge.clone();
    hud.set(
        "team_marker",
        lua.create_function(move |_, (unit, index): (Option<String>, i32)| {
            let mut d = b.borrow_mut();
            let p = &d.party;
            if p.id != 0 && !p.is_party && p.owner_id != d.character_id {
                return Ok(());
            }
            let command = if index == -2 {
                Some(PartyCommand::SetMarker {
                    team_id: p.id,
                    index,
                    kind: 0,
                    id: 0,
                })
            } else if (1..=12).contains(&index) {
                marker_target(&d, unit.as_deref()).map(|(kind, id)| {
                    let (kind, id) = if p.marks[(index - 1) as usize] == (kind, id) {
                        (0, 0)
                    } else {
                        (kind, id)
                    };
                    PartyCommand::SetMarker {
                        team_id: p.id,
                        index: index - 1,
                        kind,
                        id,
                    }
                })
            } else {
                None
            };
            if let Some(command) = command {
                d.requests.push(Request::Party(command));
            }
            Ok(())
        })?,
    )?;
    Ok(())
}

fn marker_target(d: &super::api::HudData, unit: Option<&str>) -> Option<(u8, u32)> {
    let unit = unit?;
    if unit == "player" {
        return (d.character_id != 0).then_some((1, d.character_id));
    }
    let object = if unit == "target" {
        d.panels.target.as_ref().map(|t| t.obj_id)
    } else {
        unit.parse().ok()
    };
    if let Some(m) = unit
        .strip_prefix("team")
        .and_then(|s| s.parse::<u8>().ok())
        .and_then(|i| i.checked_sub(1))
        .and_then(|slot| d.party.members.iter().find(|m| m.slot == slot))
    {
        return Some((1, m.id));
    }
    let object = object.filter(|id| *id != 0)?;
    if d.snapshot.unit_id.parse().ok() == Some(object) {
        return Some((1, d.character_id));
    }
    if let Some(m) = d.party.member_by_obj(object) {
        return Some((1, m.id));
    }
    if let Some(t) = d
        .panels
        .target
        .as_ref()
        .filter(|t| t.obj_id == object && t.character_id != 0)
    {
        return Some((1, t.character_id));
    }
    Some((2, object))
}

pub fn load_script(lua: &Lua) -> mlua::Result<()> {
    lua.load(include_str!("party.lua"))
        .set_name("@party.lua")
        .exec()?;
    lua.load(include_str!("party_ping.lua"))
        .set_name("@party_ping.lua")
        .exec()
}

fn push_member(lua: &Lua, m: &PartyMember, owner: u32, extras: &members::Extras) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set("id", m.id)?;
    t.set("index", u32::from(m.slot) + 1)?;
    if m.obj_id != 0 {
        t.set("unit_id", m.obj_id.to_string())?;
    }
    t.set("name", m.name.as_str())?;
    t.set("level", m.level)?;
    t.set("race", m.race)?;
    t.set("gender", m.gender)?;
    t.set("hp", m.hp)?;
    t.set("max_hp", m.max_hp.max(m.hp))?;
    t.set("mp", m.mp)?;
    t.set("max_mp", m.max_mp.max(m.mp))?;
    t.set("dead", m.dead)?;
    t.set("online", m.online)?;
    t.set("role", m.role)?;
    t.set("leader", m.id == owner)?;
    t.set("unit_type", "character")?;
    t.set("detail", "character")?;
    t.set("relation", "friendly")?;
    t.set("in_combat", extras.combat.contains(&m.obj_id))?;
    if let Some(buffs) = extras.buffs.get(&m.obj_id) {
        t.set("buffs", super::panels::buffs_table(lua, buffs)?)?;
    }
    t.set("x", m.pos.x)?;
    t.set("y", m.pos.y)?;
    t.set("z", m.pos.z)?;
    t.set(
        "abilities",
        lua.create_sequence_from(m.abilities.iter().copied().filter(|a| (1..=10).contains(a)))?,
    )?;
    Ok(t)
}

fn push(lua: &Lua, hud: &Table, p: &Party, own: u32, extras: &members::Extras) -> mlua::Result<()> {
    let t = lua.create_table()?;
    t.set("id", p.id)?;
    t.set("own_id", own)?;
    t.set("is_party", p.is_party)?;
    t.set("count", p.members.len())?;
    t.set(
        "own_index",
        p.member(own).map_or(0, |m| u32::from(m.slot) + 1),
    )?;
    t.set("loot_master", p.looting_rule.loot_master)?;
    t.set("loot_method", p.looting_rule.method)?;
    t.set(
        "loot_grade",
        super::store::ui_grade(p.looting_rule.minimum_grade),
    )?;
    t.set("loot_bop", p.looting_rule.roll_for_bind_on_pickup)?;
    let marks = lua.create_table()?;
    for (i, (kind, id)) in p.marks.iter().enumerate() {
        let m = lua.create_table()?;
        m.set("kind", *kind)?;
        m.set("id", *id)?;
        marks.set(i + 1, m)?;
    }
    t.set("marks", marks)?;
    let members = lua.create_table()?;
    let units = lua.create_table()?;
    for m in &p.members {
        let entry = push_member(lua, m, p.owner_id, extras)?;
        members.set(u32::from(m.slot) + 1, entry.clone())?;
        units.set(format!("team{}", m.slot + 1), entry.clone())?;
        if m.obj_id != 0 {
            units.set(m.obj_id.to_string(), entry)?;
        }
    }
    t.set("members", members)?;
    if let Some(i) = &p.pending_invite {
        let invite = lua.create_table()?;
        invite.set("sender_id", i.sender_id)?;
        invite.set("name", i.sender_name.as_str())?;
        invite.set("is_party", i.is_party)?;
        t.set("invite", invite)?;
    }
    hud.set("team", t)?;
    hud.set("team_units", units)
}

fn sync(
    rt: Option<NonSend<UiRuntime>>,
    bridge: NonSend<HudBridge>,
    mut events: MessageReader<aa_net::NetEvent>,
    units: Option<Res<aa_net::state::Units>>,
    mut mark_units: Local<[Option<u32>; 12]>,
    buffs: Option<Res<aa_net::state::Buffs>>,
    time: Res<Time>,
    mut damage: MessageReader<crate::ingame::combat::DamageLanded>,
    mut fights: Local<members::Fights>,
    mut shown: Local<members::Extras>,
) {
    use aa_net::party::PartyEvent as E;
    let signals: Vec<_> = events
        .read()
        .filter_map(|aa_net::NetEvent(e)| {
            if let aa_net::Event::Party(e) = e {
                Some(e.clone())
            } else {
                None
            }
        })
        .collect();
    let Some(rt) = rt else { return };
    let (old, new, own, own_obj, target, state_changed) = {
        let d = bridge.0.borrow();
        (
            d.party_pushed.clone().unwrap_or_default(),
            d.party.clone(),
            d.character_id,
            d.snapshot.unit_id.parse::<u32>().ok(),
            d.panels.target.clone(),
            d.party_pushed.as_ref() != Some(&d.party),
        )
    };
    let old_mark_units = *mark_units;
    let new_mark_units = std::array::from_fn(|i| {
        marker_object(
            new.marks[i],
            &new,
            own,
            own_obj,
            target.as_ref(),
            units.as_deref(),
        )
    });
    let now = time.elapsed_secs_f64();
    for d in damage.read() {
        fights.note(d.damage.caster_obj_id, d.damage.target_obj_id, now);
    }
    let extras = {
        let d = bridge.0.borrow();
        members::extras(&new, buffs.as_deref(), d.game.get(), d.start, &fights, now)
    };
    if !state_changed && signals.is_empty() && old_mark_units == new_mark_units && extras == *shown {
        return;
    }
    *shown = extras.clone();
    let Ok(hud) = rt.lua.globals().get::<Table>("HUD") else {
        return;
    };
    if let Err(e) = push(&rt.lua, &hud, &new, own, &extras) {
        warn!("hud: party: {e}");
        return;
    }
    let mapped = rt.lua.create_table().and_then(|t| {
        for (i, obj) in new_mark_units.iter().enumerate() {
            if let Some(obj) = obj {
                t.set(i + 1, obj.to_string())?;
            }
        }
        hud.get::<Table>("team")?.set("marker_units", t)
    });
    if let Err(e) = mapped {
        warn!("hud: marker unit IDs: {e}");
    }
    *mark_units = new_mark_units;
    bridge.0.borrow_mut().party_pushed = Some(new.clone());
    // Only roster/authority changes rebuild frames. Their own OnUpdate reads changing vitals.
    let roster = |p: &Party| {
        p.members
            .iter()
            .map(|m| (m.id, m.slot, m.obj_id, m.online, m.role))
            .collect::<Vec<_>>()
    };
    let mut native_roster = false;
    let mut departed = false;
    // The final resource may already have lost members removed later in this packet batch.
    let index = |p: &Party, id| {
        p.member(id)
            .or_else(|| old.member(id))
            .map_or(0, |m| i64::from(m.slot) + 1)
    };
    let name = |p: &Party, id| {
        p.member(id)
            .or_else(|| old.member(id))
            .map_or_else(String::new, |m| m.name.clone())
    };
    for e in signals {
        if matches!(e, E::Left { .. } | E::Dismissed { .. }) {
            if departed {
                continue;
            }
            departed = true;
        }
        let p = if new.id == 0 { &old } else { &new };
        let change = match e {
            E::Joined(ref p) if p.id == new.id => {
                Some(("joined_by_self", name(p, p.owner_id), 0, 0))
            }
            E::Left {
                kicked, dismissed, ..
            } => Some((
                if dismissed {
                    "dismissed"
                } else if kicked {
                    "kicked_by_self"
                } else {
                    "leaved_by_self"
                },
                String::new(),
                0,
                0,
            )),
            E::Dismissed { .. } => Some(("dismissed", String::new(), 0, 0)),
            E::MemberJoined { ref member, .. } => {
                Some(("joined", member.name.clone(), index(&new, member.id), 0))
            }
            E::MemberLeft {
                member_id, kicked, ..
            } => Some((
                if kicked { "kicked" } else { "leaved" },
                name(&old, member_id),
                index(&old, member_id),
                0,
            )),
            E::OwnerChanged { owner_id, .. } => Some((
                "owner_changed",
                name(&new, owner_id),
                index(&old, old.owner_id),
                index(&new, owner_id),
            )),
            E::Refreshed { member_id, .. } => Some((
                "refreshed",
                name(&new, member_id),
                index(&new, member_id),
                0,
            )),
            E::Moved { from, to, .. } => {
                native_roster = true;
                rt.fire_event(
                    "TEAM_MEMBERS_CHANGED",
                    ("moved", p.is_party, i64::from(from + 1), i64::from(to + 1)),
                );
                None
            }
            E::Disconnected { member_id, .. } => {
                rt.fire_event(
                    "TEAM_MEMBER_DISCONNECTED",
                    (
                        p.is_party,
                        old.member(member_id).map_or(0, |m| m.obj_id).to_string(),
                        index(p, member_id),
                    ),
                );
                None
            }
            E::RoleChanged {
                member_id, role, ..
            } => {
                rt.fire_event(
                    "TEAM_ROLE_CHANGED",
                    (index(&new, member_id), i64::from(role)),
                );
                None
            }
            E::Rejected { ref name, is_party } => {
                rt.fire_event(
                    "TEAM_MEMBERS_CHANGED",
                    ("invitation_rejected", is_party, name.as_str()),
                );
                None
            }
            E::RequestRejected(ref message) => {
                warn!("hud: team request: {message}");
                None
            }
            _ => None,
        };
        if let Some((reason, name, a, b)) = change {
            native_roster = true;
            rt.fire_event("TEAM_MEMBERS_CHANGED", (reason, p.is_party, name, a, b));
            info!(
                "hud: team {} {} ({} members)",
                new.id,
                reason,
                new.members.len()
            );
        }
    }
    for m in &new.members {
        if let Some(before) = old.member(m.id).filter(|before| before.obj_id != m.obj_id) {
            rt.fire_event(
                "TEAM_MEMBER_UNIT_ID_CHANGED",
                (
                    i64::from(m.slot) + 1,
                    before.obj_id.to_string(),
                    m.obj_id.to_string(),
                ),
            );
        }
    }
    let reason = if native_roster {
        None
    } else if old.id != new.id {
        Some(if new.id == 0 {
            "leaved_by_self"
        } else {
            "joined_by_self"
        })
    } else if old.owner_id != new.owner_id {
        Some("owner_changed")
    } else if roster(&old) != roster(&new) {
        Some("members_changed")
    } else {
        None
    };
    if let Some(reason) = reason {
        let p = if new.id == 0 { &old } else { &new };
        let owner = p.member(p.owner_id).map_or("", |m| m.name.as_str());
        let old_index = old
            .member(old.owner_id)
            .map_or(0, |m| i64::from(m.slot) + 1);
        let new_index = new
            .member(new.owner_id)
            .map_or(0, |m| i64::from(m.slot) + 1);
        rt.fire_event(
            "TEAM_MEMBERS_CHANGED",
            (reason, p.is_party, owner, old_index, new_index),
        );
        info!(
            "hud: team {} {} ({} members)",
            new.id,
            reason,
            new.members.len()
        );
    }
    if old.id == new.id && old.is_party && !new.is_party && new.id != 0 {
        rt.fire_event("CONVERT_TO_RAID_TEAM", ());
    }
    if old.looting_rule.method != new.looting_rule.method {
        rt.fire_event(
            "LOOTING_RULE_METHOD_CHANGED",
            i64::from(new.looting_rule.method),
        );
    }
    if old.looting_rule.loot_master != new.looting_rule.loot_master {
        rt.fire_event(
            "LOOTING_RULE_MASTER_CHANGED",
            name(&new, new.looting_rule.loot_master),
        );
    }
    if old.looting_rule.minimum_grade != new.looting_rule.minimum_grade {
        rt.fire_event(
            "LOOTING_RULE_GRADE_CHANGED",
            i64::from(super::store::ui_grade(new.looting_rule.minimum_grade)),
        );
    }
    if old.looting_rule.roll_for_bind_on_pickup != new.looting_rule.roll_for_bind_on_pickup {
        rt.fire_event(
            "LOOTING_RULE_BOP_CHANGED",
            i64::from(new.looting_rule.roll_for_bind_on_pickup),
        );
    }
    for i in 0..12 {
        if old.marks[i] != new.marks[i] || old_mark_units[i] != new_mark_units[i] {
            if let Some(obj) = old_mark_units[i] {
                rt.fire_event("SET_OVERHEAD_MARK", (obj.to_string(), i as i64 + 1, false));
            }
            if let Some(obj) = new_mark_units[i] {
                rt.fire_event("SET_OVERHEAD_MARK", (obj.to_string(), i as i64 + 1, true));
            }
        }
    }
    if old.pending_invite != new.pending_invite {
        if let Err(e) = rt.exec("@party_invitation", "HUD.show_team_invite()") {
            warn!("hud: party invitation: {e}");
        }
    }
}

pub(super) fn marker_object(
    mark: (u8, u32),
    party: &Party,
    own: u32,
    own_obj: Option<u32>,
    target: Option<&super::panels::TargetSnap>,
    units: Option<&aa_net::state::Units>,
) -> Option<u32> {
    let (kind, id) = mark;
    if id == 0 {
        return None;
    }
    if kind == 2 {
        return Some(id);
    }
    if kind != 1 {
        return None;
    }
    if id == own {
        return own_obj.filter(|obj| *obj != 0);
    }
    if let Some(obj) = party
        .member(id)
        .filter(|m| m.online)
        .map(|m| m.obj_id)
        .filter(|obj| *obj != 0)
    {
        return Some(obj);
    }
    if let Some(t) = target.filter(|t| t.character_id == id) {
        return Some(t.obj_id);
    }
    units?
        .iter()
        .find(|u| matches!(u.kind,aa_net::model::UnitKind::Character {char_id} if char_id==id))
        .map(|u| u.obj_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raid() -> Party {
        Party {
            id: 1,
            owner_id: 11,
            is_party: false,
            members: vec![
                PartyMember {
                    id: 11,
                    obj_id: 101,
                    name: "Self".into(),
                    online: true,
                    slot: 5,
                    ..default()
                },
                PartyMember {
                    id: 12,
                    obj_id: 102,
                    name: "Friend".into(),
                    online: true,
                    slot: 9,
                    ..default()
                },
            ],
            ..default()
        }
    }

    #[test]
    fn raid_controls_resolve_slots_roles_loot_and_marker_identity()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut rt = UiRuntime::new("en_us", 1280.0, 720.0)?;
        let bridge = std::rc::Rc::new(std::cell::RefCell::new(
            super::super::api::HudData::default(),
        ));
        {
            let mut d = bridge.borrow_mut();
            d.party = raid();
            d.character_id = 11;
            d.snapshot.unit_id = "101".into();
        }
        super::super::api::install(&mut rt, &bridge)?;
        rt.lua.load("X2Team:SetRole(2); X2Team:MoveTeamMember(6,10); X2Team:MoveTeamMemberToParty(10,3); X2Team:ChangeLootingRule(2,'Friend',1,false); X2Unit:SetOverHeadMarker('team10',2); X2Unit:RemoveAllOverHeadMarker(); X2Team:SetRole(4); X2Team:MoveTeamMember(0,2); X2Team:MoveTeamMember(6,51)").exec()?;
        assert_eq!(
            bridge.borrow().requests,
            vec![
                Request::Party(PartyCommand::SetRole {
                    team_id: 1,
                    member_id: 11,
                    role: 2
                }),
                Request::Party(PartyCommand::MoveMember {
                    team_id: 1,
                    id_from: 11,
                    id_to: 12,
                    from: 5,
                    to: 9
                }),
                Request::Party(PartyCommand::MoveMember {
                    team_id: 1,
                    id_from: 12,
                    id_to: 0,
                    from: 9,
                    to: 10
                }),
                Request::Party(PartyCommand::ChangeLootingRule {
                    team_id: 1,
                    rule: LootingRule {
                        method: 2,
                        minimum_grade: 0,
                        loot_master: 12,
                        roll_for_bind_on_pickup: false
                    },
                    flags: 15
                }),
                Request::Party(PartyCommand::SetMarker {
                    team_id: 1,
                    index: 1,
                    kind: 1,
                    id: 12
                }),
                Request::Party(PartyCommand::SetMarker {
                    team_id: 1,
                    index: -2,
                    kind: 0,
                    id: 0
                }),
            ]
        );
        // These calls never change confirmed membership or loot data before an echo.
        assert_eq!(bridge.borrow().party, raid());
        bridge.borrow_mut().requests.clear();
        bridge.borrow_mut().party.owner_id = 12;
        rt.lua.load("X2Team:MoveTeamMember(6,10); X2Team:ChangeLootingRule(0,'',2,true); X2Unit:SetOverHeadMarker('player',1); X2Team:SetRole(3)").exec()?;
        assert_eq!(
            bridge.borrow().requests,
            vec![Request::Party(PartyCommand::SetRole {
                team_id: 1,
                member_id: 11,
                role: 3
            })]
        );
        Ok(())
    }

    #[test]
    fn raid_acknowledgements_publish_native_move_loot_and_mark_arguments()
    -> Result<(), Box<dyn std::error::Error>> {
        use bevy::ecs::system::RunSystemOnce;
        let mut rt = UiRuntime::new("en_us", 1280.0, 720.0)?;
        let bridge = std::rc::Rc::new(std::cell::RefCell::new(
            super::super::api::HudData::default(),
        ));
        {
            let mut d = bridge.borrow_mut();
            d.party = raid();
            d.character_id = 11;
            d.snapshot.unit_id = "101".into();
        }
        super::super::api::install(&mut rt, &bridge)?;
        {
            let mut d = bridge.borrow_mut();
            d.party_pushed = Some(d.party.clone());
            d.party.marks[0] = (1, 12);
            d.party.looting_rule = LootingRule {
                method: 2,
                minimum_grade: 0,
                loot_master: 12,
                roll_for_bind_on_pickup: false,
            };
            d.party.members[0].slot = 9;
            d.party.members[1].slot = 5;
        }
        rt.lua.load("notices={}; for _,event in ipairs({'TEAM_MEMBERS_CHANGED','LOOTING_RULE_METHOD_CHANGED','LOOTING_RULE_MASTER_CHANGED','LOOTING_RULE_GRADE_CHANGED','LOOTING_RULE_BOP_CHANGED','SET_OVERHEAD_MARK'}) do local e=event; UIParent:SetEventHandler(e,function(...) notices[e]={...} end) end").exec()?;
        let mut app = App::new();
        app.add_message::<aa_net::NetEvent>();
        app.add_message::<crate::ingame::combat::DamageLanded>().init_resource::<Time>();
        app.insert_non_send(rt);
        app.insert_non_send(HudBridge(bridge));
        app.world_mut()
            .write_message(aa_net::NetEvent(aa_net::Event::Party(
                aa_net::party::PartyEvent::Moved {
                    team_id: 1,
                    id_from: 11,
                    id_to: 12,
                    from: 5,
                    to: 9,
                },
            )));
        app.world_mut().run_system_once(sync).unwrap();
        app.world().non_send::<UiRuntime>().lua.load("assert(notices.TEAM_MEMBERS_CHANGED[1]=='moved' and notices.TEAM_MEMBERS_CHANGED[3]==6 and notices.TEAM_MEMBERS_CHANGED[4]==10); assert(notices.LOOTING_RULE_METHOD_CHANGED[1]==2); assert(notices.LOOTING_RULE_MASTER_CHANGED[1]=='Friend'); assert(notices.LOOTING_RULE_GRADE_CHANGED[1]==1 and notices.LOOTING_RULE_BOP_CHANGED[1]==0); assert(notices.SET_OVERHEAD_MARK[1]=='102' and notices.SET_OVERHEAD_MARK[2]==1 and notices.SET_OVERHEAD_MARK[3]==true); assert(X2Unit:GetOverHeadMarkerUnitId(1)=='102'); local method,name,grade,roll=X2Team:GetLootingRule(); assert(method==2 and name=='Friend' and grade==1 and roll==false)").exec()?;
        Ok(())
    }

    #[test]
    fn a_batched_leader_transfer_and_disband_keeps_names_and_notifies_once()
    -> Result<(), Box<dyn std::error::Error>> {
        use aa_net::party::PartyEvent as E;
        use bevy::ecs::system::RunSystemOnce;
        let mut rt = UiRuntime::new("en_us", 1280.0, 720.0)?;
        let bridge = std::rc::Rc::new(std::cell::RefCell::new(
            super::super::api::HudData::default(),
        ));
        super::super::api::install(&mut rt, &bridge)?;
        bridge.borrow_mut().party_pushed = Some(Party {
            id: 1,
            owner_id: 12,
            is_party: true,
            members: vec![
                PartyMember {
                    id: 11,
                    name: "Self".into(),
                    slot: 0,
                    ..default()
                },
                PartyMember {
                    id: 12,
                    name: "Friend".into(),
                    slot: 1,
                    ..default()
                },
            ],
            ..default()
        });
        rt.lua.load("changes={}; UIParent:SetEventHandler('TEAM_MEMBERS_CHANGED',function(reason,isParty,name) changes[#changes+1]={reason=reason,name=name} end)").exec()?;
        let mut app = App::new();
        app.add_message::<aa_net::NetEvent>();
        app.add_message::<crate::ingame::combat::DamageLanded>().init_resource::<Time>();
        app.insert_non_send(rt);
        app.insert_non_send(HudBridge(bridge));
        for e in [
            E::OwnerChanged {
                team_id: 1,
                owner_id: 11,
            },
            E::MemberLeft {
                team_id: 1,
                member_id: 12,
                kicked: false,
            },
            E::Left {
                team_id: 1,
                kicked: false,
                dismissed: true,
            },
            E::Dismissed { team_id: 1 },
        ] {
            app.world_mut()
                .write_message(aa_net::NetEvent(aa_net::Event::Party(e)));
        }
        app.world_mut().run_system_once(sync).unwrap();
        app.world().non_send::<UiRuntime>().lua.load("assert(#changes==3); assert(changes[1].reason=='owner_changed' and changes[1].name=='Self'); assert(changes[2].reason=='leaved' and changes[2].name=='Friend'); assert(changes[3].reason=='dismissed')").exec()?;
        Ok(())
    }

    #[test]
    fn sparse_raid_slots_and_commands_use_character_ids() -> Result<(), Box<dyn std::error::Error>>
    {
        let mut rt = UiRuntime::new("en_us", 1280.0, 720.0)?;
        let bridge = std::rc::Rc::new(std::cell::RefCell::new(
            super::super::api::HudData::default(),
        ));
        {
            let mut d = bridge.borrow_mut();
            d.character_id = 11;
            d.snapshot.unit_id = "101".into();
            d.party = Party {
                id: 1,
                owner_id: 11,
                is_party: false,
                members: vec![
                    PartyMember {
                        id: 11,
                        obj_id: 101,
                        name: "Self".into(),
                        online: true,
                        slot: 5,
                        ..default()
                    },
                    PartyMember {
                        id: 12,
                        obj_id: 102,
                        name: "Friend".into(),
                        online: true,
                        slot: 9,
                        hp: 50,
                        max_hp: 100,
                        ..default()
                    },
                ],
                ..default()
            };
        }
        super::super::api::install(&mut rt, &bridge)?;
        rt.lua.load("assert(X2Team:GetTeamPlayerIndex()==6 and X2Team:GetTeamPlayerPartyHeadIndex()==6); assert(X2Team:CountTeamMembersInParty(2)==2); assert(X2Unit:UnitIsTeamMember('team10')); assert(X2Unit:UnitName('team10')=='Friend'); assert(X2Unit:UnitHealth('102')==50); assert(X2Unit:UnitTeamAuthority('team6')=='leader'); X2Team:MakeTeamOwner(10); X2Unit:TargetUnit('team10')").exec()?;
        assert_eq!(
            bridge.borrow().requests,
            vec![
                Request::Party(PartyCommand::MakeOwner {
                    team_id: 1,
                    member_id: 12
                }),
                Request::Target { obj_id: 102 }
            ]
        );
        Ok(())
    }
}
