//! Original party/raid frames and actions over server-confirmed membership.
use aa_net::party::{Party, PartyCommand, PartyMember};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use mlua::{Lua, Table};

use super::api::{HudBridge, Request, Shared};
use crate::Screen;

pub fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        sync.after(super::sync_hud)
            .before(super::api::handle_requests)
            .run_if(super::hud_up)
            .run_if(in_state(Screen::InGame)),
    );
}

pub fn install(lua: &Lua, hud: &Table, bridge: &Shared) -> mlua::Result<()> {
    bridge.borrow_mut().party_pushed = None;
    push(
        lua,
        hud,
        &bridge.borrow().party,
        bridge.borrow().character_id,
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
    Ok(())
}

pub fn load_script(lua: &Lua) -> mlua::Result<()> {
    lua.load(include_str!("party.lua"))
        .set_name("@party.lua")
        .exec()
}

fn push_member(lua: &Lua, m: &PartyMember, owner: u32) -> mlua::Result<Table> {
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
    t.set("x", m.pos.x)?;
    t.set("y", m.pos.y)?;
    t.set("z", m.pos.z)?;
    t.set(
        "abilities",
        lua.create_sequence_from(m.abilities.iter().copied().filter(|a| (1..=10).contains(a)))?,
    )?;
    Ok(t)
}

fn push(lua: &Lua, hud: &Table, p: &Party, own: u32) -> mlua::Result<()> {
    let t = lua.create_table()?;
    t.set("id", p.id)?;
    t.set("is_party", p.is_party)?;
    t.set("count", p.members.len())?;
    t.set(
        "own_index",
        p.member(own).map_or(0, |m| u32::from(m.slot) + 1),
    )?;
    t.set("loot_master", p.looting_rule.loot_master)?;
    t.set("loot_method", p.looting_rule.method)?;
    let members = lua.create_table()?;
    let units = lua.create_table()?;
    for m in &p.members {
        let entry = push_member(lua, m, p.owner_id)?;
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
    let (old, new, own) = {
        let d = bridge.0.borrow();
        if d.party_pushed.as_ref() == Some(&d.party) && signals.is_empty() {
            return;
        }
        (
            d.party_pushed.clone().unwrap_or_default(),
            d.party.clone(),
            d.character_id,
        )
    };
    let Ok(hud) = rt.lua.globals().get::<Table>("HUD") else {
        return;
    };
    if let Err(e) = push(&rt.lua, &hud, &new, own) {
        warn!("hud: party: {e}");
        return;
    }
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
                    (
                        "moved",
                        p.is_party,
                        i64::from(from / 5 + 1),
                        i64::from(to / 5 + 1),
                    ),
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
    if old.looting_rule != new.looting_rule {
        rt.fire_event("LOOTING_RULE_MASTER_CHANGED", ());
        rt.fire_event("LOOTING_RULE_METHOD_CHANGED", ());
    }
    if old.pending_invite != new.pending_invite {
        if let Err(e) = rt.exec("@party_invitation", "HUD.show_team_invite()") {
            warn!("hud: party invitation: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
