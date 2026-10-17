//! Party / raid packets and current membership. IDs are character IDs unless named `obj_id`.
//!
//! Layouts follow `systems/social/packets/team.rs` in the lane server. In particular a joined
//! team has 50 fixed slots and **no member count after its looting rule**: the occupied slots
//! determine how many member records follow. Remote status records are exactly 50 bytes.

use crate::opcodes::{cs, sc};
use crate::session::Event;
use crate::wire::{Pos, Reader, Result, WireError, Writer};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PartyInvite {
    pub team_id: u32,
    pub sender_id: u32,
    pub sender_name: String,
    pub is_party: bool,
    pub is_area: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LootingRule {
    /// 0 free for all, 1 rotate, 2 loot master, 255 public.
    pub method: u8,
    pub minimum_grade: u8,
    pub loot_master: u32,
    pub roll_for_bind_on_pickup: bool,
}

impl Default for LootingRule {
    fn default() -> Self {
        Self { method: 1, minimum_grade: 2, loot_master: 0, roll_for_bind_on_pickup: true }
    }
}

impl LootingRule {
    fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self { method: r.u8()?, minimum_grade: r.u8()?, loot_master: r.u32()?, roll_for_bind_on_pickup: r.bool()? })
    }

    fn write(&self, w: &mut Writer) {
        w.u8(self.method).u8(self.minimum_grade).u32(self.loot_master).bool(self.roll_for_bind_on_pickup);
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PartyMember {
    pub id: u32,
    pub name: String,
    pub race: u8,
    pub gender: u8,
    pub level: u8,
    /// 0 undecided, 1 tank, 2 healer, 3 attacker.
    pub role: u8,
    pub obj_id: u32,
    pub online: bool,
    pub dead: bool,
    /// 0..49, groups of five in a raid.
    pub slot: u8,
    /// True after the server sends the 50-byte remote status. Initial membership has no vitals.
    pub has_status: bool,
    pub zone_id: u32,
    pub instance_id: u32,
    pub hp: i32,
    pub max_hp: i32,
    pub mp: i32,
    pub max_mp: i32,
    pub pos: Pos,
    pub abilities: [u8; 3],
}

impl PartyMember {
    fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            id: r.u32()?,
            name: r.string()?,
            race: r.u8()?,
            gender: r.u8()?,
            level: r.u8()?,
            role: r.u8()?,
            obj_id: r.bc()?,
            ..Self::default()
        })
    }

    fn read_person(r: &mut Reader) -> Result<Self> {
        let mut m = Self {
            id: r.u32()?,
            zone_id: r.u32()?,
            instance_id: r.u32()?,
            level: r.u8()?,
            hp: r.i32()?,
            max_hp: r.i32()?,
            mp: r.i32()?,
            max_mp: r.i32()?,
            pos: r.position()?,
            obj_id: r.bc()?,
            has_status: true,
            ..Self::default()
        };
        r.skip(1)?;
        m.abilities = [r.u8()?, r.u8()?, r.u8()?];
        r.skip(5)?;
        m.online = m.obj_id != 0;
        m.dead = m.hp <= 0;
        Ok(m)
    }

    fn apply_status(&mut self, s: &Self) {
        self.obj_id = s.obj_id;
        self.online = s.online;
        self.dead = s.dead;
        self.level = s.level;
        self.zone_id = s.zone_id;
        self.instance_id = s.instance_id;
        self.hp = s.hp;
        self.max_hp = s.max_hp;
        self.mp = s.mp;
        self.max_mp = s.max_mp;
        self.pos = s.pos;
        self.abilities = s.abilities;
        self.has_status = true;
    }
}

/// The server's current party (or raid); `id == 0` means no membership.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "bevy", derive(bevy::ecs::resource::Resource))]
pub struct Party {
    pub id: u32,
    pub owner_id: u32,
    pub is_party: bool,
    pub members: Vec<PartyMember>,
    pub pending_invite: Option<PartyInvite>,
    /// (kind, id), where 0 empty, 1 character id, 2 object id.
    pub marks: [(u8, u32); 12],
    pub looting_rule: LootingRule,
    pub ping: Option<(Pos, u32)>,
}

impl Party {
    pub fn member(&self, character_id: u32) -> Option<&PartyMember> {
        self.members.iter().find(|m| m.id == character_id)
    }

    pub fn member_by_obj(&self, obj_id: u32) -> Option<&PartyMember> {
        (obj_id != 0).then(|| self.members.iter().find(|m| m.online && m.obj_id == obj_id)).flatten()
    }

    /// Resolve both character IDs from the current zero-based slots, including empty targets.
    /// The application checks authority; the lane requires the owner for member moves.
    pub fn plan_move(&self, from: u8, to: u8) -> Option<PartyCommand> {
        if self.id == 0 || from >= 50 || to >= 50 || from == to {
            return None;
        }
        let source = self.members.iter().find(|m| m.slot == from)?;
        let target = self.members.iter().find(|m| m.slot == to).map_or(0, |m| m.id);
        Some(PartyCommand::MoveMember { team_id: self.id, id_from: source.id, id_to: target, from, to })
    }

    /// Membership can arrive after a nearby unit's initial announcement. Seed that unit's
    /// current values once on joining/refresh; later movement/status packets remain authoritative.
    pub(crate) fn seed_visible(
        &mut self,
        event: &Event,
        player: &crate::state::Player,
        units: &crate::state::Units,
    ) -> bool {
        if !matches!(
            event,
            Event::Party(PartyEvent::Joined(_) | PartyEvent::MemberJoined { .. } | PartyEvent::Refreshed { .. })
        ) {
            return false;
        }
        let mut changed = false;
        for m in &mut self.members {
            if m.has_status || !m.online {
                continue;
            }
            let unit =
                player.spawn.as_ref().filter(|p| p.obj_id == m.obj_id).map(|p| &p.unit).or_else(|| units.get(m.obj_id));
            let Some(u) = unit else { continue };
            let before = m.clone();
            m.pos = u.pos;
            m.hp = u.hp;
            m.mp = u.mp;
            m.level = u.level;
            m.dead = u.dead;
            m.max_hp = m.max_hp.max(u.max_hp);
            m.max_mp = m.max_mp.max(u.max_mp);
            for (to, from) in m.abilities.iter_mut().zip(&u.active_abilities) {
                *to = *from;
            }
            changed |= *m != before;
        }
        changed
    }

    /// Fold a server event. Membership is only changed by a server acknowledgement.
    pub fn apply(&mut self, event: &Event) -> bool {
        let Event::Party(event) = event else {
            if matches!(event, Event::LeftWorld { .. } | Event::Disconnected { .. } | Event::PlayerSpawn(_)) {
                let changed = *self != Self::default();
                *self = Self::default();
                return changed;
            }
            let (obj_id, hp_mp, pos, level) = match event {
                Event::UnitSpawned(u) => (u.obj_id, Some((u.hp, u.mp)), Some(u.pos), Some(u.level)),
                Event::UnitPoints { obj_id, hp, mp } => (*obj_id, Some((*hp, *mp)), None, None),
                Event::UnitMoved { obj_id, movement } if movement.move_type != 4 => (
                    *obj_id,
                    None,
                    Some(movement.support.as_ref().and_then(|s| s.world.as_ref()).map_or(movement.pos, |(p, _)| *p)),
                    None,
                ),
                Event::Resurrected { obj_id, pos, .. } => (*obj_id, None, Some(*pos), None),
                Event::UnitDeath { obj_id } => {
                    (*obj_id, Some((0, self.member_by_obj(*obj_id).map_or(0, |m| m.mp))), None, None)
                }
                Event::LevelChanged { obj_id, level } => (*obj_id, None, None, Some(*level)),
                _ => return false,
            };
            let Some(m) = self.members.iter_mut().find(|m| obj_id != 0 && m.online && m.obj_id == obj_id) else {
                return false;
            };
            let before = m.clone();
            if let Some((hp, mp)) = hp_mp {
                m.hp = hp;
                m.mp = mp;
                if !m.has_status {
                    m.max_hp = m.max_hp.max(hp);
                    m.max_mp = m.max_mp.max(mp);
                }
            }
            if let Some(pos) = pos {
                m.pos = pos;
            }
            if let Some(level) = level {
                m.level = level;
            }
            match event {
                Event::UnitSpawned(u) => m.dead = u.dead,
                Event::UnitDeath { .. } => m.dead = true,
                Event::Resurrected { .. } => m.dead = false,
                Event::UnitPoints { hp, .. } if *hp > 0 => m.dead = false,
                _ => {}
            }
            if let Event::UnitSpawned(u) = event {
                if !m.has_status {
                    for (to, from) in m.abilities.iter_mut().zip(&u.active_abilities) {
                        *to = *from;
                    }
                }
            }
            return *m != before;
        };
        match event {
            PartyEvent::Invited(invite) => self.pending_invite = Some(invite.clone()),
            PartyEvent::Joined(party) => *self = (**party).clone(),
            PartyEvent::Left { team_id, .. } | PartyEvent::Dismissed { team_id } if *team_id == self.id => {
                *self = Self::default()
            }
            PartyEvent::MemberJoined { team_id, member, party } if *team_id == self.id => {
                if let Some(old) = self.members.iter_mut().find(|m| m.id == member.id) {
                    let slot = old.slot;
                    let mut updated = member.clone();
                    // Login repeats membership before status. Keep the last location/vitals.
                    if old.has_status {
                        updated.apply_status(old);
                    }
                    updated.obj_id = member.obj_id;
                    updated.online = member.obj_id != 0;
                    updated.slot = slot;
                    *old = updated;
                } else {
                    let start = (*party as u8) * 5;
                    if let Some(slot) = (start..start + 5).find(|slot| !self.members.iter().any(|m| m.slot == *slot)) {
                        let mut member = member.clone();
                        member.slot = slot;
                        member.online = member.obj_id != 0;
                        self.members.push(member);
                    }
                }
                self.members.sort_by_key(|m| m.slot);
            }
            PartyEvent::MemberLeft { team_id, member_id, .. } if *team_id == self.id => {
                self.members.retain(|m| m.id != *member_id)
            }
            PartyEvent::Disconnected { team_id, member_id, status } if *team_id == self.id => {
                if let Some(m) = self.members.iter_mut().find(|m| m.id == *member_id) {
                    m.apply_status(status);
                    m.online = false;
                    m.obj_id = 0;
                }
            }
            PartyEvent::RemoteMembers(members) if self.id != 0 => {
                for status in members {
                    if let Some(m) = self.members.iter_mut().find(|m| m.id == status.id) {
                        m.apply_status(status);
                    }
                }
            }
            PartyEvent::OwnerChanged { team_id, owner_id } if *team_id == self.id => self.owner_id = *owner_id,
            PartyEvent::RoleChanged { team_id, member_id, role } if *team_id == self.id => {
                if let Some(m) = self.members.iter_mut().find(|m| m.id == *member_id) {
                    m.role = *role;
                }
            }
            PartyEvent::Refreshed { team_id, member_id, obj_id } if *team_id == self.id => {
                if let Some(m) = self.members.iter_mut().find(|m| m.id == *member_id) {
                    m.obj_id = *obj_id;
                    m.online = *obj_id != 0;
                }
            }
            PartyEvent::BecameRaid { team_id } if *team_id == self.id => self.is_party = false,
            PartyEvent::Moved { team_id, from, to, .. } if *team_id == self.id => {
                for m in &mut self.members {
                    if m.slot == *from {
                        m.slot = *to;
                    } else if m.slot == *to {
                        m.slot = *from;
                    }
                }
                self.members.sort_by_key(|m| m.slot);
            }
            PartyEvent::LootingChanged { team_id, rule, .. } if *team_id == self.id => self.looting_rule = *rule,
            PartyEvent::Marker { team_id, index, kind, id } if *team_id == self.id => {
                if *index == -2 {
                    self.marks = [(0, 0); 12];
                } else if let Ok(i) = usize::try_from(*index) {
                    if let Some(mark) = self.marks.get_mut(i) {
                        *mark = if *id == 0 { (0, 0) } else { (*kind, *id) };
                    }
                }
            }
            PartyEvent::Ping { has_ping, pos, instance_id } => self.ping = has_ping.then_some((*pos, *instance_id)),
            // This is a local notification of a successfully sent reply, to close the invitation.
            PartyEvent::Replied { sender_id } => {
                if self.pending_invite.as_ref().is_some_and(|i| i.sender_id == *sender_id) {
                    self.pending_invite = None;
                } else {
                    return false;
                }
            }
            _ => return false,
        }
        true
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PartyCommand {
    Invite {
        team_id: u32,
        is_party: bool,
        target_name: String,
    },
    /// Nearby friendly/neutral players within 100 m. Area acceptance is a lane TODO.
    InviteArea {
        team_id: u32,
        is_party: bool,
    },
    Reply {
        invite: PartyInvite,
        accept: bool,
    },
    Leave {
        team_id: u32,
        character_id: u32,
    },
    Kick {
        team_id: u32,
        member_id: u32,
    },
    Dismiss {
        team_id: u32,
    },
    MakeOwner {
        team_id: u32,
        member_id: u32,
    },
    ConvertToRaid {
        team_id: u32,
    },
    /// The lane supports changing one's own role only (0 undecided, 1 tank, 2 healer, 3 attacker).
    SetRole {
        team_id: u32,
        member_id: u32,
        role: u8,
    },
    /// Zero-based slots; `id_to: 0` is an empty destination. Prefer `Party::plan_move`.
    MoveMember {
        team_id: u32,
        id_from: u32,
        id_to: u32,
        from: u8,
        to: u8,
    },
    /// Flags: 1 method, 2 minimum grade, 4 loot master (character ID), 8 roll for BoP.
    ChangeLootingRule {
        team_id: u32,
        rule: LootingRule,
        flags: u8,
    },
    /// Index 0..11 or -2 (reset all); kind 0 clear, 1 character ID, 2 object ID.
    /// `team_id: 0` sets a solo marker, echoed only to the sender.
    SetMarker {
        team_id: u32,
        index: i32,
        kind: u8,
        id: u32,
    },
    /// Plain f32 world coordinates, not the packed position encoding. Team 0 sets a solo ping.
    SetPing {
        team_id: u32,
        has_ping: bool,
        pos: Pos,
        instance_id: u32,
    },
}

impl PartyCommand {
    pub fn validate(&self) -> std::result::Result<(), &'static str> {
        match self {
            Self::Invite { target_name, .. } if target_name.trim().is_empty() || target_name.len() > 128 => {
                return Err("invalid party invite name");
            }
            Self::Reply { invite, .. } if invite.sender_id == 0 => return Err("party invitation has no sender"),
            Self::Leave { team_id, character_id } if *team_id == 0 || *character_id == 0 => {
                return Err("invalid party leave target");
            }
            Self::Kick { team_id, member_id } | Self::MakeOwner { team_id, member_id }
                if *team_id == 0 || *member_id == 0 =>
            {
                return Err("invalid party member");
            }
            Self::Dismiss { team_id } | Self::ConvertToRaid { team_id } if *team_id == 0 => {
                return Err("not in a party");
            }
            Self::SetRole { team_id, member_id, role } if *team_id == 0 || *member_id == 0 || *role > 3 => {
                return Err("invalid party role");
            }
            Self::MoveMember { team_id, id_from, id_to, from, to }
                if *team_id == 0 || *id_from == 0 || id_from == id_to || *from >= 50 || *to >= 50 || from == to =>
            {
                return Err("invalid party move");
            }
            Self::ChangeLootingRule { team_id, rule, flags } => {
                if *team_id == 0 || *flags == 0 || flags & !0x0f != 0 {
                    return Err("invalid looting rule flags");
                }
                if flags & 1 != 0 && !matches!(rule.method, 0..=2 | 255) {
                    return Err("invalid looting method");
                }
                if flags & 2 != 0 && rule.minimum_grade > 11 {
                    return Err("invalid looting grade");
                }
            }
            Self::SetMarker { index, kind, id, .. } => {
                if !matches!(index, -2 | 0..=11) || *kind > 2 {
                    return Err("invalid overhead marker");
                }
                if *index == -2 && (*kind != 0 || *id != 0) {
                    return Err("invalid marker reset");
                }
                if *kind == 0 && *id != 0 {
                    return Err("empty marker has a target");
                }
                if *kind == 2 && *id > 0x00ff_ffff {
                    return Err("marker object ID exceeds 24 bits");
                }
            }
            Self::SetPing { pos, .. } if !pos.x.is_finite() || !pos.y.is_finite() || !pos.z.is_finite() => {
                return Err("invalid party ping position");
            }
            _ => {}
        }
        Ok(())
    }

    pub fn opcode(&self) -> u16 {
        match self {
            Self::Invite { .. } => cs::CSInviteToTeamPacket,
            Self::InviteArea { .. } => cs::CSInviteAreaToTeamPacket,
            Self::Reply { .. } => cs::CSReplyToJoinTeamPacket,
            Self::MakeOwner { .. } => cs::CSMakeTeamOwnerPacket,
            Self::ConvertToRaid { .. } => cs::CSConvertToRaidTeamPacket,
            Self::SetRole { .. } => cs::CSSetTeamMemberRolePacket,
            Self::MoveMember { .. } => cs::CSMoveTeamMemberPacket,
            Self::ChangeLootingRule { .. } => cs::CSChangeLootingRulePacket,
            Self::SetMarker { .. } => cs::CSSetOverHeadMarkerPacket,
            Self::SetPing { .. } => cs::CSSetPingPosPacket,
            Self::Leave { .. } | Self::Kick { .. } | Self::Dismiss { .. } => cs::CSAskRiskyTeamActionPacket,
        }
    }

    /// `character_name` is the replying local character, not the inviter.
    pub fn write(&self, w: &mut Writer, character_name: &str) {
        match self {
            Self::Invite { team_id, is_party, target_name } => {
                w.u32(*team_id).bool(*is_party).string(target_name);
            }
            Self::InviteArea { team_id, is_party } => {
                w.u32(*team_id).bool(*is_party);
            }
            Self::Reply { invite, accept } => {
                w.u32(invite.team_id)
                    .bool(invite.is_party)
                    .u32(invite.sender_id)
                    .bool(!accept)
                    .string(character_name)
                    .bool(invite.is_area);
            }
            Self::Leave { team_id, character_id } => {
                w.u32(*team_id).u32(*character_id).u8(0);
            }
            Self::Kick { team_id, member_id } => {
                w.u32(*team_id).u32(*member_id).u8(1);
            }
            Self::Dismiss { team_id } => {
                w.u32(*team_id).u32(0).u8(2);
            }
            Self::MakeOwner { team_id, member_id } => {
                w.u32(*team_id).u32(*member_id);
            }
            Self::ConvertToRaid { team_id } => {
                w.u32(*team_id);
            }
            Self::SetRole { team_id, member_id, role } => {
                w.u32(*team_id).u32(*member_id).u8(*role);
            }
            Self::MoveMember { team_id, id_from, id_to, from, to } => {
                w.u32(*team_id).u32(*id_from).u32(*id_to).u8(*from).u8(*to);
            }
            Self::ChangeLootingRule { team_id, rule, flags } => {
                w.u32(*team_id);
                rule.write(w);
                w.u8(*flags);
            }
            Self::SetMarker { team_id, index, kind, id } => {
                w.u32(*team_id).i32(*index).u8(*kind);
                if *kind == 1 {
                    w.u32(*id);
                } else if *kind == 2 {
                    w.bc(*id);
                }
            }
            Self::SetPing { team_id, has_ping, pos, instance_id } => {
                w.u32(*team_id).bool(*has_ping).f32(pos.x).f32(pos.y).f32(pos.z).u32(*instance_id);
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PartyEvent {
    /// Local request validation failed; no network packet or optimistic state change.
    RequestRejected(String),
    Invited(PartyInvite),
    Joined(Box<Party>),
    Rejected {
        name: String,
        is_party: bool,
    },
    Left {
        team_id: u32,
        kicked: bool,
        dismissed: bool,
    },
    Dismissed {
        team_id: u32,
    },
    MemberJoined {
        team_id: u32,
        member: PartyMember,
        party: i32,
    },
    MemberLeft {
        team_id: u32,
        member_id: u32,
        kicked: bool,
    },
    Disconnected {
        team_id: u32,
        member_id: u32,
        status: PartyMember,
    },
    OwnerChanged {
        team_id: u32,
        owner_id: u32,
    },
    RoleChanged {
        team_id: u32,
        member_id: u32,
        role: u8,
    },
    BecameRaid {
        team_id: u32,
    },
    Moved {
        team_id: u32,
        id_from: u32,
        id_to: u32,
        from: u8,
        to: u8,
    },
    LootingChanged {
        team_id: u32,
        rule: LootingRule,
        flags: u8,
    },
    Refreshed {
        team_id: u32,
        member_id: u32,
        obj_id: u32,
    },
    RemoteMembers(Vec<PartyMember>),
    AreaInvited {
        id: u32,
        accepted: bool,
    },
    Marker {
        team_id: u32,
        index: i32,
        kind: u8,
        id: u32,
    },
    Ping {
        has_ping: bool,
        pos: Pos,
        instance_id: u32,
    },
    ActionAck {
        team_id: u32,
        member_id: u32,
        action: u8,
        value: i32,
        error: i16,
    },
    /// Sent a reply (local); not a server acknowledgement of joining.
    Replied {
        sender_id: u32,
    },
}

impl PartyEvent {
    pub fn handles(opcode: u16) -> bool {
        matches!(opcode, 0x0d4..=0x0e0 | 0x0e2..=0x0e8)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::RequestRejected(_) => "PartyRequestRejected",
            Self::Invited(PartyInvite { is_area: true, .. }) => "SCAskToJoinTeamArea",
            Self::Invited(_) => "SCAskToJoinTeam",
            Self::Joined(_) => "SCJoinedTeam",
            Self::Rejected { .. } => "SCRejectedTeam",
            Self::Left { .. } => "SCLeavedTeam",
            Self::Dismissed { .. } => "SCTeamDismissed",
            Self::MemberJoined { .. } => "SCTeamMemberJoined",
            Self::MemberLeft { .. } => "SCTeamMemberLeaved",
            Self::Disconnected { .. } => "SCTeamMemberDisconnected",
            Self::OwnerChanged { .. } => "SCTeamOwnerChanged",
            Self::RoleChanged { .. } => "SCTeamMemberRoleChanged",
            Self::BecameRaid { .. } => "SCTeamBecameRaidTeam",
            Self::Moved { .. } => "SCTeamMemberMoved",
            Self::LootingChanged { .. } => "SCTeamLootingRuleChanged",
            Self::Refreshed { .. } => "SCRefreshTeamMember",
            Self::RemoteMembers(_) => "SCTeamRemoteMembersEx",
            Self::AreaInvited { .. } => "SCTeamAreaInvited",
            Self::Marker { .. } => "SCOverHeadMarkerSet",
            Self::Ping { .. } => "SCTeamPingPos",
            Self::ActionAck { .. } => "SCTeamAckRiskyAction",
            Self::Replied { .. } => "PartyReplied",
        }
    }

    pub fn read(opcode: u16, r: &mut Reader) -> Result<Self> {
        Ok(match opcode {
            sc::SCAskToJoinTeamPacket | sc::SCAskToJoinTeamAreaPacket => Self::Invited(PartyInvite {
                team_id: r.u32()?,
                sender_id: r.u32()?,
                sender_name: r.string()?,
                is_party: r.bool()?,
                is_area: opcode == sc::SCAskToJoinTeamAreaPacket,
            }),
            sc::SCJoinedTeamPacket => {
                let mut party = Party { id: r.u32()?, owner_id: r.u32()?, is_party: r.bool()?, ..Party::default() };
                let mut counts = [0u8; 10];
                for count in &mut counts {
                    *count = r.u8()?;
                }
                let mut slots = [(0, false); 50];
                for slot in &mut slots {
                    *slot = (r.u32()?, r.bool()?);
                }
                for (i, group) in slots.chunks_exact(5).enumerate() {
                    if counts[i] as usize != group.iter().filter(|s| s.0 != 0).count() {
                        return Err(WireError::Invalid("team slot count mismatch".into()));
                    }
                }
                for mark in &mut party.marks {
                    *mark = read_mark(r)?;
                }
                party.looting_rule = LootingRule::read(r)?;
                let occupied: Vec<_> = slots.iter().enumerate().filter(|(_, s)| s.0 != 0).collect();
                // The smallest TeamMember.Write is 13 bytes (empty name). Check before allocating.
                if occupied.len() > r.remaining() / 13 {
                    return Err(WireError::Invalid("truncated team members".into()));
                }
                for _ in 0..occupied.len() {
                    let mut member = PartyMember::read(r)?;
                    let Some((slot, (_, online))) = occupied.iter().find(|(_, s)| s.0 == member.id).copied() else {
                        return Err(WireError::Invalid("team member has no slot".into()));
                    };
                    if party.members.iter().any(|m| m.id == member.id) {
                        return Err(WireError::Invalid("duplicate team member".into()));
                    }
                    member.slot = slot as u8;
                    member.online = *online;
                    party.members.push(member);
                }
                party.members.sort_by_key(|m| m.slot);
                Self::Joined(Box::new(party))
            }
            sc::SCRejectedTeamPacket => Self::Rejected { name: r.string()?, is_party: r.bool()? },
            sc::SCLeavedTeamPacket => Self::Left { team_id: r.u32()?, kicked: r.bool()?, dismissed: r.bool()? },
            sc::SCTeamDismissedPacket => Self::Dismissed { team_id: r.u32()? },
            sc::SCTeamMemberJoinedPacket => {
                let (team_id, member, party) = (r.u32()?, PartyMember::read(r)?, r.i32()?);
                if !(0..10).contains(&party) {
                    return Err(WireError::Invalid("invalid team group".into()));
                }
                Self::MemberJoined { team_id, member, party }
            }
            sc::SCTeamMemberLeavedPacket => {
                Self::MemberLeft { team_id: r.u32()?, member_id: r.u32()?, kicked: r.bool()? }
            }
            sc::SCTeamMemberDisconnectedPacket => {
                Self::Disconnected { team_id: r.u32()?, member_id: r.u32()?, status: PartyMember::read_person(r)? }
            }
            sc::SCTeamOwnerChangedPacket => Self::OwnerChanged { team_id: r.u32()?, owner_id: r.u32()? },
            sc::SCTeamMemberRoleChangedPacket => {
                Self::RoleChanged { team_id: r.u32()?, member_id: r.u32()?, role: r.u8()? }
            }
            sc::SCTeamBecameRaidTeamPacket => Self::BecameRaid { team_id: r.u32()? },
            sc::SCTeamMemberMovedPacket => {
                let (team_id, id_from, id_to, from, to) = (r.u32()?, r.u32()?, r.u32()?, r.u8()?, r.u8()?);
                if from >= 50 || to >= 50 {
                    return Err(WireError::Invalid("invalid team slot".into()));
                }
                Self::Moved { team_id, id_from, id_to, from, to }
            }
            sc::SCTeamLootingRuleChangedPacket => {
                Self::LootingChanged { team_id: r.u32()?, rule: LootingRule::read(r)?, flags: r.u8()? }
            }
            sc::SCRefreshTeamMemberPacket => {
                Self::Refreshed { team_id: r.u32()?, member_id: r.u32()?, obj_id: r.bc()? }
            }
            sc::SCTeamRemoteMembersExPacket => {
                let count = r.i32()?;
                if !(0..=50).contains(&count) || count as usize > r.remaining() / 50 {
                    return Err(WireError::Invalid(format!("invalid remote team member count {count}")));
                }
                let mut members = Vec::with_capacity(count as usize);
                for _ in 0..count {
                    members.push(PartyMember::read_person(r)?);
                }
                Self::RemoteMembers(members)
            }
            sc::SCTeamAreaInvitedPacket => Self::AreaInvited { id: r.u32()?, accepted: r.bool()? },
            sc::SCOverHeadMarkerSetPacket => {
                let (team_id, index) = (r.u32()?, r.i32()?);
                let (kind, id) = read_mark(r)?;
                Self::Marker { team_id, index, kind, id }
            }
            sc::SCTeamPingPosPacket => {
                Self::Ping { has_ping: r.bool()?, pos: Pos::new(r.f32()?, r.f32()?, r.f32()?), instance_id: r.u32()? }
            }
            sc::SCTeamAckRiskyActionPacket => Self::ActionAck {
                team_id: r.u32()?,
                member_id: r.u32()?,
                action: r.u8()?,
                value: r.i32()?,
                error: r.i16()?,
            },
            _ => return Err(WireError::Invalid(format!("not a party opcode: {opcode:#x}"))),
        })
    }
}

fn read_mark(r: &mut Reader) -> Result<(u8, u32)> {
    let kind = r.u8()?;
    let id = match kind {
        0 => 0,
        1 => r.u32()?,
        2 => r.bc()?,
        _ => return Err(WireError::Invalid(format!("invalid overhead marker type {kind}"))),
    };
    Ok((kind, id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{ClientMsg, ServerMsg};
    use crate::model::Movement;

    fn member_bytes(w: &mut Writer, id: u32) {
        w.u32(id).string(if id == 7 { "Al" } else { "Bob" }).u8(1).u8(2).u8(30).u8(2).bc(id + 100);
    }

    fn person_bytes(w: &mut Writer, id: u32, online: bool) {
        w.u32(id)
            .u32(100)
            .u32(0)
            .u8(30)
            .i32(1000)
            .i32(1200)
            .i32(50)
            .i32(60)
            .position(Pos::new(100.0, 200.0, 50.0))
            .bc(if online { id + 100 } else { 0 })
            .raw(&[0, 1, 2, 3, 0, 0, 0, 0, 0]);
    }

    fn joined_bytes() -> Vec<u8> {
        let mut w = Writer::new();
        w.u32(3).u32(7).bool(true).raw(&[2, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        w.u32(7).bool(true).u32(8).bool(false);
        for _ in 2..50 {
            w.u32(0).bool(false);
        }
        w.u8(1).u32(0x11223344).u8(2).bc(0x55);
        for _ in 2..12 {
            w.u8(0);
        }
        w.u8(1).u8(2).u32(0).bool(true);
        member_bytes(&mut w, 7);
        member_bytes(&mut w, 8);
        w.finish()
    }

    fn joined() -> Party {
        let PartyEvent::Joined(p) =
            PartyEvent::read(sc::SCJoinedTeamPacket, &mut Reader::new(&joined_bytes())).unwrap()
        else {
            panic!()
        };
        *p
    }

    #[test]
    fn commands_follow_server_readers_including_risky_actions() {
        let invite = PartyInvite { team_id: 3, sender_id: 7, sender_name: "Al".into(), is_party: true, is_area: false };
        let cases = [
            (
                PartyCommand::Invite { team_id: 3, is_party: true, target_name: "Bob".into() },
                0x07a,
                vec![3, 0, 0, 0, 1, 3, 0, b'B', b'o', b'b'],
            ),
            (
                PartyCommand::Reply { invite: invite.clone(), accept: true },
                0x07c,
                vec![3, 0, 0, 0, 1, 7, 0, 0, 0, 0, 3, 0, b'B', b'o', b'b', 0],
            ),
            (PartyCommand::InviteArea { team_id: 3, is_party: false }, 0x07b, vec![3, 0, 0, 0, 0]),
            (
                PartyCommand::Reply { invite, accept: false },
                0x07c,
                vec![3, 0, 0, 0, 1, 7, 0, 0, 0, 1, 3, 0, b'B', b'o', b'b', 0],
            ),
            (PartyCommand::Leave { team_id: 3, character_id: 7 }, 0x088, vec![3, 0, 0, 0, 7, 0, 0, 0, 0]),
            (PartyCommand::Kick { team_id: 3, member_id: 8 }, 0x088, vec![3, 0, 0, 0, 8, 0, 0, 0, 1]),
            (PartyCommand::Dismiss { team_id: 3 }, 0x088, vec![3, 0, 0, 0, 0, 0, 0, 0, 2]),
            (PartyCommand::MakeOwner { team_id: 3, member_id: 8 }, 0x07f, vec![3, 0, 0, 0, 8, 0, 0, 0]),
            (PartyCommand::ConvertToRaid { team_id: 3 }, 0x080, vec![3, 0, 0, 0]),
            (PartyCommand::SetRole { team_id: 3, member_id: 7, role: 2 }, 0x085, vec![3, 0, 0, 0, 7, 0, 0, 0, 2]),
            (
                PartyCommand::MoveMember { team_id: 3, id_from: 7, id_to: 8, from: 0, to: 9 },
                0x081,
                vec![3, 0, 0, 0, 7, 0, 0, 0, 8, 0, 0, 0, 0, 9],
            ),
            (
                PartyCommand::ChangeLootingRule {
                    team_id: 3,
                    rule: LootingRule { method: 2, minimum_grade: 4, loot_master: 7, roll_for_bind_on_pickup: false },
                    flags: 15,
                },
                0x083,
                vec![3, 0, 0, 0, 2, 4, 7, 0, 0, 0, 0, 15],
            ),
            (
                PartyCommand::SetMarker { team_id: 3, index: 2, kind: 2, id: 0x0a0b0c },
                0x086,
                vec![3, 0, 0, 0, 2, 0, 0, 0, 2, 12, 11, 10],
            ),
            (
                PartyCommand::SetMarker { team_id: 3, index: -2, kind: 0, id: 0 },
                0x086,
                vec![3, 0, 0, 0, 254, 255, 255, 255, 0],
            ),
            (
                PartyCommand::SetPing { team_id: 0, has_ping: true, pos: Pos::new(1.0, 2.0, 3.0), instance_id: 9 },
                0x087,
                vec![0, 0, 0, 0, 1, 0, 0, 128, 63, 0, 0, 0, 64, 0, 0, 64, 64, 9, 0, 0, 0],
            ),
        ];
        for (command, opcode, body) in cases {
            let packet = ClientMsg::Party { command, character_name: "Bob".into() };
            assert_eq!(packet.opcode(), opcode);
            assert_eq!(packet.encode_body(), body);
        }
    }

    #[test]
    fn joined_reads_fixed_slots_and_all_occupied_members() {
        let bytes = joined_bytes();
        let mut r = Reader::new(&bytes);
        let PartyEvent::Joined(p) = PartyEvent::read(sc::SCJoinedTeamPacket, &mut r).unwrap() else { panic!() };
        assert!(r.is_empty());
        assert_eq!((p.id, p.owner_id, p.is_party), (3, 7, true));
        assert_eq!((p.members[0].id, p.members[0].slot, p.members[0].online), (7, 0, true));
        assert_eq!((p.members[1].id, p.members[1].slot, p.members[1].online), (8, 1, false));
        assert_eq!(p.marks[0], (1, 0x11223344));
        assert_eq!(p.marks[1], (2, 0x55));
        assert_eq!(p.looting_rule, LootingRule::default());
        // A false group count must not cause the next compressed opcode to be read as a name.
        let mut bad = bytes.clone();
        bad[9] = 1;
        assert!(PartyEvent::read(sc::SCJoinedTeamPacket, &mut Reader::new(&bad)).is_err());
        assert!(PartyEvent::read(sc::SCJoinedTeamPacket, &mut Reader::new(&bytes[..bytes.len() - 1])).is_err());
    }

    #[test]
    fn remote_status_is_exactly_fifty_bytes_and_counts_are_validated() {
        let mut w = Writer::new();
        w.i32(2);
        person_bytes(&mut w, 7, true);
        person_bytes(&mut w, 8, false);
        let bytes = w.finish();
        assert_eq!(bytes.len(), 104);
        let mut r = Reader::new(&bytes);
        let PartyEvent::RemoteMembers(m) = PartyEvent::read(sc::SCTeamRemoteMembersExPacket, &mut r).unwrap() else {
            panic!()
        };
        assert!(r.is_empty());
        assert_eq!((m[0].id, m[0].obj_id, m[0].online), (7, 107, true));
        assert_eq!((m[0].hp, m[0].max_hp, m[0].mp, m[0].max_mp), (1000, 1200, 50, 60));
        assert_eq!(m[0].abilities, [1, 2, 3]);
        assert_eq!(m[1].obj_id, 0);
        assert!(!m[1].online);
        for n in [-1i32, 3, 51, i32::MAX] {
            let mut bad = bytes.clone();
            bad[..4].copy_from_slice(&n.to_le_bytes());
            assert!(PartyEvent::read(sc::SCTeamRemoteMembersExPacket, &mut Reader::new(&bad)).is_err());
        }
    }

    #[test]
    fn every_party_packet_preserves_the_next_opcode_in_a_compressed_batch() {
        let mut packets = vec![(sc::SCJoinedTeamPacket, joined_bytes())];
        macro_rules! packet {
            ($op:ident, $body:expr) => {{
                let mut w = Writer::new();
                $body(&mut w);
                packets.push((sc::$op, w.finish()));
            }};
        }
        packet!(SCAskToJoinTeamPacket, |w: &mut Writer| {
            w.u32(0).u32(7).string("Al").bool(true);
        });
        packet!(SCAskToJoinTeamAreaPacket, |w: &mut Writer| {
            w.u32(0).u32(7).string("Al").bool(true);
        });
        packet!(SCRejectedTeamPacket, |w: &mut Writer| {
            w.string("Bob").bool(true);
        });
        packet!(SCLeavedTeamPacket, |w: &mut Writer| {
            w.u32(3).bool(false).bool(true);
        });
        packet!(SCTeamDismissedPacket, |w: &mut Writer| {
            w.u32(3);
        });
        packet!(SCTeamMemberJoinedPacket, |w: &mut Writer| {
            w.u32(3);
            member_bytes(w, 8);
            w.i32(0);
        });
        packet!(SCTeamMemberLeavedPacket, |w: &mut Writer| {
            w.u32(3).u32(8).bool(true);
        });
        packet!(SCTeamMemberDisconnectedPacket, |w: &mut Writer| {
            w.u32(3).u32(8);
            person_bytes(w, 8, false);
        });
        packet!(SCTeamOwnerChangedPacket, |w: &mut Writer| {
            w.u32(3).u32(8);
        });
        packet!(SCTeamMemberRoleChangedPacket, |w: &mut Writer| {
            w.u32(3).u32(8).u8(2);
        });
        packet!(SCTeamBecameRaidTeamPacket, |w: &mut Writer| {
            w.u32(3);
        });
        packet!(SCTeamMemberMovedPacket, |w: &mut Writer| {
            w.u32(3).u32(7).u32(8).u8(0).u8(1);
        });
        packet!(SCTeamLootingRuleChangedPacket, |w: &mut Writer| {
            w.u32(3).u8(1).u8(2).u32(7).bool(true).u8(15);
        });
        packet!(SCRefreshTeamMemberPacket, |w: &mut Writer| {
            w.u32(3).u32(8).bc(108);
        });
        packet!(SCTeamRemoteMembersExPacket, |w: &mut Writer| {
            w.i32(2);
            person_bytes(w, 7, true);
            person_bytes(w, 8, false);
        });
        packet!(SCTeamAreaInvitedPacket, |w: &mut Writer| {
            w.u32(3).bool(true);
        });
        packet!(SCOverHeadMarkerSetPacket, |w: &mut Writer| {
            w.u32(3).i32(2).u8(2).bc(108);
        });
        packet!(SCTeamPingPosPacket, |w: &mut Writer| {
            w.bool(true).f32(1.0).f32(2.0).f32(3.0).u32(5);
        });
        packet!(SCTeamAckRiskyActionPacket, |w: &mut Writer| {
            w.u32(3).u32(8).u8(1).i32(0).i16(0);
        });
        for (opcode, body) in packets {
            let mut raw = Writer::new();
            raw.u16(0).u16(opcode).raw(&body).u16(0).u16(sc::SCOnOffSnowPacket).bool(true);
            let raw = raw.finish();
            let len = raw.len() as u16;
            let mut frame = Writer::new();
            frame.raw(&[0xdd, 4]).u16(2).u8(1).u16(len).u16(!len).raw(&raw);
            let ServerMsg::Batch(batch) = ServerMsg::decode(&frame.finish()).unwrap() else { panic!() };
            assert_eq!(batch.len(), 2, "opcode {opcode:#x}");
            assert!(matches!(batch[0], ServerMsg::Party(_)), "opcode {opcode:#x}");
            assert_eq!(batch[1], ServerMsg::Snow(true), "opcode {opcode:#x}");
        }
    }

    #[test]
    fn state_keeps_identity_and_slots_through_status_disconnect_and_rejoin() {
        let mut party = joined();
        let mut status = Writer::new();
        status.i32(1);
        person_bytes(&mut status, 8, true);
        let e = PartyEvent::read(sc::SCTeamRemoteMembersExPacket, &mut Reader::new(&status.finish())).unwrap();
        assert!(party.apply(&Event::Party(e)));
        let m = party.member(8).unwrap();
        assert_eq!((m.name.as_str(), m.slot, m.hp, m.max_hp, m.online), ("Bob", 1, 1000, 1200, true));
        assert!(
            party.apply(&Event::UnitMoved { obj_id: 108, movement: Movement::unit(Pos::new(5.0, 6.0, 7.0), 0.0, 1) })
        );
        assert_eq!(party.member_by_obj(108).unwrap().pos, Pos::new(5.0, 6.0, 7.0));
        let status = PartyMember { id: 8, ..PartyMember::default() };
        party.apply(&Event::Party(PartyEvent::Disconnected { team_id: 3, member_id: 8, status }));
        assert!(!party.member(8).unwrap().online);
        party.apply(&Event::Party(PartyEvent::MemberJoined {
            team_id: 3,
            member: PartyMember { id: 8, obj_id: 208, name: "Bob".into(), ..PartyMember::default() },
            party: 0,
        }));
        assert_eq!((party.members.len(), party.member(8).unwrap().obj_id, party.member(8).unwrap().slot), (2, 208, 1));
        party.apply(&Event::Party(PartyEvent::MemberLeft { team_id: 999, member_id: 8, kicked: true }));
        assert_eq!(party.members.len(), 2);
        party.apply(&Event::Party(PartyEvent::OwnerChanged { team_id: 3, owner_id: 8 }));
        assert_eq!(party.owner_id, 8);
        party.apply(&Event::Party(PartyEvent::Left { team_id: 3, kicked: true, dismissed: false }));
        assert_eq!(party, Party::default());
    }

    #[test]
    fn invitation_reply_only_clears_the_matching_sender_and_logout_clears_party() {
        let mut party = joined();
        let invite = PartyInvite { sender_id: 7, ..PartyInvite::default() };
        party.apply(&Event::Party(PartyEvent::Invited(invite.clone())));
        assert!(!party.apply(&Event::Party(PartyEvent::Replied { sender_id: 8 })));
        assert_eq!(party.pending_invite, Some(invite));
        party.apply(&Event::Party(PartyEvent::Replied { sender_id: 7 }));
        assert!(party.pending_invite.is_none());
        party.apply(&Event::LeftWorld { target: 1 });
        assert_eq!(party, Party::default());
    }

    #[test]
    fn nearby_death_and_resurrection_do_not_need_remote_status() {
        let mut party = joined();
        assert!(!party.member(7).unwrap().has_status);
        party.apply(&Event::UnitDeath { obj_id: 107 });
        assert!(party.member(7).unwrap().dead);
        party.apply(&Event::Resurrected { obj_id: 107, pos: Pos::new(10.0, 20.0, 30.0), yaw: 0.0 });
        assert!(!party.member(7).unwrap().dead);
        assert_eq!(party.member(7).unwrap().pos, Pos::new(10.0, 20.0, 30.0));
    }

    #[test]
    fn raid_commands_validate_slots_marker_reset_flags_and_finite_positions() {
        let invalid = [
            PartyCommand::ConvertToRaid { team_id: 0 },
            PartyCommand::SetRole { team_id: 3, member_id: 7, role: 4 },
            PartyCommand::MoveMember { team_id: 3, id_from: 7, id_to: 0, from: 0, to: 50 },
            PartyCommand::MoveMember { team_id: 3, id_from: 7, id_to: 7, from: 0, to: 1 },
            PartyCommand::ChangeLootingRule { team_id: 3, rule: LootingRule::default(), flags: 0 },
            PartyCommand::ChangeLootingRule { team_id: 3, rule: LootingRule::default(), flags: 16 },
            PartyCommand::SetMarker { team_id: 3, index: -1, kind: 1, id: 7 }, // would reset all on the server
            PartyCommand::SetMarker { team_id: 3, index: -2, kind: 2, id: 7 },
            PartyCommand::SetMarker { team_id: 3, index: 0, kind: 2, id: 0x0100_0000 },
            PartyCommand::SetPing { team_id: 3, has_ping: true, pos: Pos::new(f32::NAN, 0.0, 0.0), instance_id: 0 },
        ];
        for command in invalid {
            assert!(command.validate().is_err(), "{command:?}");
        }
        assert!(PartyCommand::SetMarker { team_id: 0, index: -2, kind: 0, id: 0 }.validate().is_ok());
    }

    #[test]
    fn raid_reducer_preserves_sparse_slots_and_normalizes_cleared_markers() {
        let mut party = joined();
        let movement = party.plan_move(1, 9).unwrap();
        assert_eq!(movement, PartyCommand::MoveMember { team_id: 3, id_from: 8, id_to: 0, from: 1, to: 9 });
        party.apply(&Event::Party(PartyEvent::BecameRaid { team_id: 3 }));
        party.apply(&Event::Party(PartyEvent::Moved { team_id: 3, id_from: 8, id_to: 0, from: 1, to: 9 }));
        assert_eq!(party.member(8).unwrap().slot, 9);
        assert_eq!(
            party.plan_move(0, 9),
            Some(PartyCommand::MoveMember { team_id: 3, id_from: 7, id_to: 8, from: 0, to: 9 })
        );
        party.apply(&Event::Party(PartyEvent::Moved { team_id: 3, id_from: 7, id_to: 8, from: 0, to: 9 }));
        assert_eq!((party.member(7).unwrap().slot, party.member(8).unwrap().slot), (9, 0));
        assert!(!party.is_party);
        party.apply(&Event::Party(PartyEvent::RoleChanged { team_id: 3, member_id: 8, role: 2 }));
        assert_eq!(party.member(8).unwrap().role, 2);
        party.apply(&Event::Party(PartyEvent::Marker { team_id: 3, index: 0, kind: 1, id: 8 }));
        party.apply(&Event::Party(PartyEvent::Marker { team_id: 3, index: 0, kind: 1, id: 0 }));
        assert_eq!(party.marks[0], (0, 0));
        party.apply(&Event::Party(PartyEvent::Marker { team_id: 3, index: -2, kind: 1, id: 0 }));
        assert_eq!(party.marks, [(0, 0); 12]);
        party.apply(&Event::Party(PartyEvent::Ping { has_ping: true, pos: Pos::new(1.0, 2.0, 3.0), instance_id: 9 }));
        assert_eq!(party.ping, Some((Pos::new(1.0, 2.0, 3.0), 9)));
        party.apply(&Event::Party(PartyEvent::Ping { has_ping: false, pos: Pos::default(), instance_id: 0 }));
        assert_eq!(party.ping, None);
        party.apply(&Event::Party(PartyEvent::Dismissed { team_id: 3 }));
        assert_eq!(party, Party::default());
    }
}
