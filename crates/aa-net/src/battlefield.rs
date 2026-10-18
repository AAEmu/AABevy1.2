//! Battlefield queue against the active Bevy lane, with explicit native transport differences.
//! Received state stays separate from successful local writes.
use crate::{
    opcodes::{cs, sc},
    session::Event,
    wire::{Reader, Result, WireError, Writer},
};

mod joined;
mod phases;
mod progress;
pub use progress::{BattlefieldPoint, BattlefieldSkillEnable, BattlefieldKill};
pub use phases::{BattlefieldPhase, BattlefieldStart, BattlefieldEnd, BattlefieldTeamResult, BattlefieldMemberResult};
pub use joined::BattlefieldJoined;

/// Native SC196 notification rounds milliseconds to nearest seconds, with half-up ties.
pub fn unearned_win_remaining_seconds(milliseconds:u32)->u32 {
    milliseconds/1000+u32::from(milliseconds%1000>=500)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BattlefieldCommand {
    Apply { battlefield_id: u32, corps: u8 },
    Cancel,
    /// Native result-screen leave request; state clears only on received instance load.
    Leave,
    /// Captured received invitation, checked again against current session state before send.
    RespondToInvitation { invitation: BattlefieldInvitation, accept: bool },
    /// Explicit caller-owned world-ready continuation; never emitted automatically.
    EnteredWorld { invitation: BattlefieldInvitation },
}
impl BattlefieldCommand {
    pub fn validate(self) -> std::result::Result<(), &'static str> {
        match self {
            Self::Apply { battlefield_id: 0, .. } => Err("battlefield ID must be nonzero"),
            Self::Apply { corps: 3.., .. } => Err("battlefield corps must be 0, 1 or 2"),
            Self::RespondToInvitation { invitation, .. } | Self::EnteredWorld { invitation } if invitation.corps > 1 => Err("invitation corps must be 0 or 1"),
            _ => Ok(()),
        }
    }
    pub fn opcode(self) -> u16 {
        match self {
            Self::Apply { .. } => cs::CSApplyToInstantGamePacket,
            Self::Cancel => cs::CSCancelInstantGamePacket,
            Self::Leave => cs::CSLeaveInstantGamePacket,
            Self::RespondToInvitation { .. } => cs::CSJoinInstantGamePacket,
            Self::EnteredWorld { .. } => cs::CSEnteredInstantGameWorldPacket,
        }
    }
    pub fn write(self, w: &mut Writer) {
        match self {
            Self::Apply { battlefield_id, corps } => { w.u32(battlefield_id).u8(corps); }
            Self::RespondToInvitation { invitation, accept } => { w.bool(accept).u64(if accept { invitation.qualified_id } else { 0 }); }
            Self::EnteredWorld { invitation } => { w.u64(invitation.qualified_id); }
            Self::Cancel | Self::Leave => {}
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BattlefieldQueue {
    pub battlefield_id: u32,
    pub corps_size: u8,
    pub counts: [u16; 3],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BattlefieldInvitation {
    pub zone_id: u32,
    pub instance_id: u32,
    pub ruleset_id: u32,
    pub corps: u8,
    pub qualified_id: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BattlefieldEvent {
    Applied {
        battlefield_id: u32,
        corps: u8,
        error: u16,
    },
    /// Native handler ignores the value; lane writes u16 error,u8 branch,u8 padding.
    Cancelled {
        raw: u32,
    },
    QueueState(BattlefieldQueue),
    Invited(BattlefieldInvitation),
    Joined(BattlefieldJoined),
    Started(BattlefieldStart),
    Ended(BattlefieldEnd),
    Point(BattlefieldPoint),
    Kill(BattlefieldKill),
    UnearnedWinRemaining { milliseconds:u32 },
    SkillEnable(BattlefieldSkillEnable),
    InviteTimedOut {
        zone_id: u32,
        instance_id: u32,
        ruleset_id: u32,
        corps: u8,
    },
    Requested(BattlefieldCommand),
    RequestRejected {
        command: BattlefieldCommand,
        message: String,
    },
}
impl BattlefieldEvent {
    /// Original 1.2 SC18c is a signed 16-bit error. Active lane dispatch reads its
    /// distinct four-byte padded payload; never auto-detect from remaining DD04 bytes.
    pub fn read_native_cancel_error(r: &mut Reader) -> Result<i16> {
        r.i16()
    }
    pub fn handles(op: u16) -> bool {
        matches!(op, 0x18b..=0x196)
    }
    pub fn name(&self) -> &'static str {
        match self {
            Self::Applied { .. } => "SCAppliedToInstantGame",
            Self::Cancelled { .. } => "SCCancelInstantGame",
            Self::QueueState(_) => "SCInstantGameState",
            Self::Invited(_) => "SCInviteToInstantGame",
            Self::Joined(_) => "SCInstantGameJoined",
            Self::Started(_) => "SCInstantGameStart",
            Self::Ended(_) => "SCInstantGameEnd",
            Self::Point(_) => "SCInstantGameAddPoint",
            Self::Kill(_) => "SCInstantGameKill",
            Self::UnearnedWinRemaining { .. } => "SCInstantGameRemainTime",
            Self::SkillEnable(_) => "SCInstantGameKillstreak",
            Self::InviteTimedOut { .. } => "SCInstantGameInviteTimeout",
            Self::Requested(_) => "BattlefieldRequested",
            Self::RequestRejected { .. } => "BattlefieldRequestRejected",
        }
    }
    pub fn read(op: u16, r: &mut Reader) -> Result<Self> {
        Ok(match op {
            sc::SCAppliedToInstantGamePacket => Self::Applied {
                battlefield_id: r.u32()?,
                corps: r.u8()?,
                error: r.u16()?,
            },
            sc::SCCancelInstantGamePacket => Self::Cancelled { raw: r.u32()? },
            sc::SCInstantGameStatePacket => Self::QueueState(BattlefieldQueue {
                battlefield_id: r.u32()?,
                corps_size: r.u8()?,
                counts: [r.u16()?, r.u16()?, r.u16()?],
            }),
            sc::SCInviteToInstantGamePacket => Self::Invited(BattlefieldInvitation {
                zone_id: r.u32()?,
                instance_id: r.u32()?,
                ruleset_id: r.u32()?,
                corps: r.u8()?,
                qualified_id: r.u64()?,
            }),
            sc::SCInstantGameJoinedPacket => Self::Joined(BattlefieldJoined::read(r)?),
            sc::SCInstantGameStartPacket => Self::Started(BattlefieldStart::read(r)?),
            sc::SCInstantGameEndPacket => Self::Ended(BattlefieldEnd::read_lane(r)?),
            sc::SCInstantGameAddPointPacket => Self::Point(BattlefieldPoint::read(r)?),
            sc::SCInstantGameKillPacket => Self::Kill(BattlefieldKill::read(r)?),
            sc::SCInstantGameUnkPacket => Self::UnearnedWinRemaining { milliseconds:r.u32()? },
            sc::SCInstantGameKillstreakPacket => Self::SkillEnable(BattlefieldSkillEnable::read(r)?),
            sc::SCInstantGameInviteTimeoutPacket => Self::InviteTimedOut {
                zone_id: r.u32()?,
                instance_id: r.u32()?,
                ruleset_id: r.u32()?,
                corps: r.u8()?,
            },
            _ => return Err(WireError::Invalid(format!("unknown battlefield opcode {op:03x}"))),
        })
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "bevy", derive(bevy::ecs::resource::Resource))]
pub struct Battlefield {
    /// Successfully written apply request, not a received admission.
    pub requesting: Option<(u32, u8)>,
    /// Received matching successful application.
    pub applied: Option<(u32, u8)>,
    pub queue: Option<BattlefieldQueue>,
    pub invitation: Option<BattlefieldInvitation>,
    pub cancel_pending: bool,
    /// Successful local reply write, not a received join or queue transition.
    pub response_pending: Option<(BattlefieldInvitation, bool)>,
    /// Last matching genuine joined packet.
    pub joined: Option<BattlefieldJoined>,
    /// Ruleset identity retained from the matching received invitation.
    pub joined_ruleset_id: Option<u32>,
    pub phase: Option<BattlefieldPhase>,
    pub start: Option<BattlefieldStart>,
    pub opening_at_ms: Option<u32>,
    pub ending_at_ms: Option<u32>,
    /// Phase-admitted matching result (native state7 only).
    pub result: Option<BattlefieldEnd>,
    /// Last genuine raw End receipt, including unmatched or clock-gated context.
    pub last_end: Option<BattlefieldEnd>,
    /// Native Joined constructor initializes both cumulative scores to zero.
    pub scores: [i32; 2],
    /// Native Joined constructor disables all 24 supplied skills.
    pub skills_enabled: [bool; 24],
    pub last_point: Option<BattlefieldPoint>,
    pub last_skill_enable: Option<BattlefieldSkillEnable>,
    pub last_kill: Option<BattlefieldKill>,
    /// Received notice only; never decremented or treated as a phase transition.
    pub last_remaining_time_ms: Option<u32>,
    pub kill_scores: [i32;2],
    /// Native stored getter value; victim notification independently passes zero.
    pub my_killstreak_point: i32,
    /// Actual selected character identity, retained through a match-only instance reset.
    pub local_character_id: Option<u32>,
    pub last_error: Option<u16>,
    pub last_rejection: Option<String>,
}
impl Battlefield {
    /// Recheck a captured invitation before writing a response. No local state is changed.
    pub fn validate_command(&self, command: BattlefieldCommand) -> std::result::Result<(), &'static str> {
        command.validate()?;
        if command == BattlefieldCommand::Leave && self.phase != Some(BattlefieldPhase::Ending) {
            return Err("battlefield leave requires the received results phase");
        }
        if let BattlefieldCommand::RespondToInvitation { invitation, .. } = command {
            if self.invitation != Some(invitation) || self.response_pending.is_some() || self.cancel_pending || self.joined.is_some() {
                return Err("battlefield invitation is stale or already answered");
            }
        }
        if let BattlefieldCommand::EnteredWorld { invitation } = command {
            if self.invitation != Some(invitation) || self.response_pending != Some((invitation, true)) || self.cancel_pending || self.joined.is_some() {
                return Err("battlefield world-ready requires the current accepted invitation");
            }
        }
        Ok(())
    }
    /// Called by the app after its actual world-entered continuation, never by packet receipt.
    pub fn world_ready(&self, invitation: BattlefieldInvitation) -> std::result::Result<BattlefieldCommand, &'static str> {
        let command = BattlefieldCommand::EnteredWorld { invitation };
        self.validate_command(command)?;
        Ok(command)
    }
    pub fn respond_to_invitation(&self, invitation: BattlefieldInvitation, accept: bool) -> std::result::Result<BattlefieldCommand, &'static str> {
        let command = BattlefieldCommand::RespondToInvitation { invitation, accept };
        self.validate_command(command)?;
        Ok(command)
    }

    /// Original IsInInstantGame includes joined, opening, playing and results phases.
    pub fn in_instant_game(&self) -> bool { self.phase.is_some() }

    fn reset_match(&mut self) {
        let local_character_id=self.local_character_id;
        *self=Self::default(); self.local_character_id=local_character_id;
    }
    pub fn accepts_kill(&self, kill:&BattlefieldKill) -> bool {
        matches!(self.phase,Some(BattlefieldPhase::Opening | BattlefieldPhase::Playing))
            && self.joined.is_some_and(|j|j.zone_id==kill.zone_id && j.instance_id==kill.instance_id)
    }
    /// This notification precedes the native phase/instance guard. Killer wins if IDs match both.
    pub fn local_killstreak_notification(&self, kill:&BattlefieldKill) -> Option<i32> {
        let id=self.local_character_id?;
        if id==kill.killer_id {Some(i32::from(kill.killer_killstreak))}
        else if id==kill.victim_id {Some(0)} else {None}
    }
    pub fn accepts_point(&self, point:&BattlefieldPoint) -> bool {
        matches!(self.phase,Some(BattlefieldPhase::Opening | BattlefieldPhase::Playing))
            && self.joined.is_some_and(|j|j.zone_id==point.zone_id && j.instance_id==point.instance_id)
    }
    /// Native notification is emitted only for a matching skill whose enable value changes.
    pub fn accepts_skill_change(&self, update:&BattlefieldSkillEnable) -> bool {
        let Ok(index)=usize::try_from(update.killstreak) else {return false};
        self.phase==Some(BattlefieldPhase::Playing) && index<24
            && self.joined.is_some_and(|j|j.zone_id==update.zone_id && j.instance_id==update.instance_id && j.killstreak_skills[index]==update.skill_id)
            && self.skills_enabled[index]!=update.enabled
    }
    /// Explicit native frame-clock input. No wall-clock timer or fabricated packet is used.
    /// Caller must supply the original mapped server clock, not a packet's `now_ms` field.
    pub fn advance_server_time(&mut self, mapped_now_ms:u32) -> bool {
        if self.phase == Some(BattlefieldPhase::Opening)
            && self.start.is_some_and(|s| mapped_now_ms.wrapping_add(6000) >= s.start_ms) {
            self.phase=Some(BattlefieldPhase::Playing); return true;
        }
        false
    }
    /// Capped received queue getter; constructor default is (0,0,-1).
    /// Pending requested corps stays in `requesting`, separate from received `applied`.
    pub fn joined_infos(&self) -> (u32, u32, i32) {
        let (cur, max) = self
            .queue
            .map(|q| {
                let m = u32::from(q.corps_size);
                (
                    (u32::from(q.counts[0]).min(m) + u32::from(q.counts[1]).min(m) + u32::from(q.counts[2]).min(m * 2)).min(m * 2),
                    m,
                )
            })
            .unwrap_or((0, 0));
        (cur, max, self.joined.map_or_else(|| self.applied.map_or(-1, |(_, c)| i32::from(c)), |j| i32::from(j.corps)))
    }
    /// Clocked native receive fold. Raw `apply` deliberately cannot invent phase timestamps.
    pub fn apply_with_time(&mut self,e:&Event,mapped_now:Option<u32>)->bool {
        let before=self.clone();let previous=self.phase;
        self.apply(e);
        if previous==Some(BattlefieldPhase::Joined) && self.phase==Some(BattlefieldPhase::Opening)
            && matches!(e,Event::Battlefield(BattlefieldEvent::Started(_))) {
            self.opening_at_ms=mapped_now.map(|n|n.wrapping_add(6000));
            // SC191's immediate branch differs from the subsequent frame rule.
            if mapped_now.is_some_and(|n|self.start.is_some_and(|s|s.start_ms<=n)) {self.phase=Some(BattlefieldPhase::Playing);}
        }
        if previous==Some(BattlefieldPhase::Playing) && self.phase==Some(BattlefieldPhase::Ending)
            && matches!(e,Event::Battlefield(BattlefieldEvent::Ended(_))) {self.ending_at_ms=mapped_now;}
        *self!=before
    }
    pub fn apply(&mut self, e: &Event) -> bool {
        let before = self.clone();
        match e {
            Event::Battlefield(BattlefieldEvent::Ended(end)) => self.last_end=Some(end.clone()),
            Event::Battlefield(BattlefieldEvent::Point(point)) => self.last_point=Some(point.clone()),
            Event::Battlefield(BattlefieldEvent::UnearnedWinRemaining { milliseconds }) => self.last_remaining_time_ms=Some(*milliseconds),
            Event::Battlefield(BattlefieldEvent::Kill(kill)) => {
                self.last_kill=Some(kill.clone());
                if self.local_killstreak_notification(kill).is_some() { self.my_killstreak_point=i32::from(kill.killer_killstreak); }
            },
            Event::Battlefield(BattlefieldEvent::SkillEnable(update)) => self.last_skill_enable=Some(*update),
            _ => {}
        }
        match e {
            Event::LoadInstance(_) if self.in_instant_game() => self.reset_match(),
            Event::CharacterSelected(state) => {
                *self=Self::default(); self.local_character_id=(state.character.id!=0).then_some(state.character.id);
            }
            Event::PlayerSpawn(spawn) => self.local_character_id=(spawn.state.character.id!=0).then_some(spawn.state.character.id),
            Event::LeftWorld { .. } | Event::Disconnected { .. } | Event::Kicked { .. } => *self = Self::default(),
            Event::Battlefield(BattlefieldEvent::Requested(c)) => {
                self.last_rejection = None;
                match *c {
                    BattlefieldCommand::Apply { battlefield_id, corps } => {
                        self.requesting = Some((battlefield_id, corps));
                    }
                    BattlefieldCommand::Cancel => self.cancel_pending = true,
                    BattlefieldCommand::EnteredWorld { .. } | BattlefieldCommand::Leave => {},
                    BattlefieldCommand::RespondToInvitation { invitation, accept } => {
                        if self.validate_command(*c).is_ok() { self.response_pending = Some((invitation, accept)); }
                    }
                }
            }
            Event::Battlefield(BattlefieldEvent::RequestRejected { message, .. }) => {
                self.last_rejection = Some(message.clone());
            }
            Event::Battlefield(BattlefieldEvent::Applied { battlefield_id, corps, error })
                if (self.requesting == Some((*battlefield_id, *corps))
                    || (self.response_pending.is_some_and(|(_, accept)| !accept) && self.applied.is_some_and(|(id, _)| id == *battlefield_id) && *corps == 2))
                    && !self.cancel_pending =>
            {
                if *error == 0 {
                    self.applied = Some((*battlefield_id, *corps));
                    self.requesting = None;
                    self.response_pending = None;
                    self.joined = None;
                    self.queue = None;
                    self.invitation = None;
                    self.last_error = None;
                } else {
                    self.reset_match();
                    self.last_error = Some(*error);
                }
            }
            Event::Battlefield(BattlefieldEvent::Cancelled { .. }) => self.reset_match(),
            Event::Battlefield(BattlefieldEvent::QueueState(q))
                if self.applied.is_some_and(|(id, _)| id == q.battlefield_id) && self.invitation.is_none() && self.joined.is_none() && !self.cancel_pending =>
            {
                self.queue = Some(*q)
            }
            Event::Battlefield(BattlefieldEvent::Invited(i))
                if self.applied.is_some_and(|(_, c)| (c <= 1 && c == i.corps) || (c == 2 && i.corps <= 1))
                    && self.invitation.is_none()
                    && self.joined.is_none()
                    && self.response_pending.is_none()
                    && !self.cancel_pending =>
            {
                self.invitation = Some(*i)
            }
            Event::Battlefield(BattlefieldEvent::Joined(joined)) if !self.cancel_pending
                && self.response_pending.is_some_and(|(i, accept)| accept && i.zone_id == joined.zone_id && i.instance_id == joined.instance_id && i.corps == joined.corps) => {
                self.joined_ruleset_id = self.response_pending.map(|(i, _)| i.ruleset_id);
                self.joined = Some(*joined);
                self.phase = Some(BattlefieldPhase::Joined);
                self.opening_at_ms=None;self.ending_at_ms=None;self.start=None;self.result=None;
                self.scores=[0;2]; self.kill_scores=[0;2]; self.skills_enabled=[false;24];
                self.response_pending = None;
                self.invitation = None;
            }
            Event::Battlefield(BattlefieldEvent::Kill(kill)) if self.accepts_kill(kill) && kill.killer_corps<2 => {
                self.kill_scores[usize::from(kill.killer_corps)]=kill.killer_corps_kills;
            }
            Event::Battlefield(BattlefieldEvent::Point(point)) if self.accepts_point(point) => self.scores=point.scores,
            Event::Battlefield(BattlefieldEvent::SkillEnable(update)) if self.accepts_skill_change(update) => {
                self.skills_enabled[update.killstreak as usize]=update.enabled;
            }
            Event::Battlefield(BattlefieldEvent::Started(start)) if self.phase == Some(BattlefieldPhase::Joined)
                && self.joined.is_some_and(|j|j.zone_id==start.zone_id && j.instance_id==start.instance_id) => {
                self.start=Some(*start); self.phase=Some(BattlefieldPhase::Opening);
            }
            Event::Battlefield(BattlefieldEvent::Ended(end)) if self.phase == Some(BattlefieldPhase::Playing)
                && self.joined.is_some_and(|j|j.zone_id==end.zone_id && j.instance_id==end.instance_id) => {
                self.result=Some(end.clone()); self.phase=Some(BattlefieldPhase::Ending);
            }
            Event::Battlefield(BattlefieldEvent::InviteTimedOut {
                zone_id,
                instance_id,
                ruleset_id,
                corps,
            }) if self
                .invitation
                .is_some_and(|i| i.zone_id == *zone_id && i.instance_id == *instance_id && i.ruleset_id == *ruleset_id && i.corps == *corps)
                && !self.cancel_pending
                && !self.response_pending.is_some_and(|(_, accept)| accept) =>
            {
                self.invitation = None;
                self.response_pending = None;
            }
            _ => {}
        }
        *self != before
    }
}
#[cfg(test)]
mod tests;
