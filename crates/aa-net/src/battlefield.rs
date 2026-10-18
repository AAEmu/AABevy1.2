//! Battlefield queue against the active Bevy lane, with explicit native transport differences.
//! Received state stays separate from successful local writes.
use crate::{
    opcodes::{cs, sc},
    session::Event,
    wire::{Reader, Result, WireError, Writer},
};

mod joined;
pub use joined::BattlefieldJoined;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BattlefieldCommand {
    Apply { battlefield_id: u32, corps: u8 },
    Cancel,
    /// Captured received invitation, checked again against current session state before send.
    RespondToInvitation { invitation: BattlefieldInvitation, accept: bool },
}
impl BattlefieldCommand {
    pub fn validate(self) -> std::result::Result<(), &'static str> {
        match self {
            Self::Apply { battlefield_id: 0, .. } => Err("battlefield ID must be nonzero"),
            Self::Apply { corps: 3.., .. } => Err("battlefield corps must be 0, 1 or 2"),
            Self::RespondToInvitation { invitation, .. } if invitation.corps > 1 => Err("invitation corps must be 0 or 1"),
            _ => Ok(()),
        }
    }
    pub fn opcode(self) -> u16 {
        match self {
            Self::Apply { .. } => cs::CSApplyToInstantGamePacket,
            Self::Cancel => cs::CSCancelInstantGamePacket,
            Self::RespondToInvitation { .. } => cs::CSJoinInstantGamePacket,
        }
    }
    pub fn write(self, w: &mut Writer) {
        match self {
            Self::Apply { battlefield_id, corps } => { w.u32(battlefield_id).u8(corps); }
            Self::RespondToInvitation { invitation, accept } => { w.bool(accept).u64(if accept { invitation.qualified_id } else { 0 }); }
            Self::Cancel => {}
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
        matches!(op, 0x18b..=0x190)
    }
    pub fn name(&self) -> &'static str {
        match self {
            Self::Applied { .. } => "SCAppliedToInstantGame",
            Self::Cancelled { .. } => "SCCancelInstantGame",
            Self::QueueState(_) => "SCInstantGameState",
            Self::Invited(_) => "SCInviteToInstantGame",
            Self::Joined(_) => "SCInstantGameJoined",
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
    /// Last matching genuine joined packet; later gameplay phases are outside this slice.
    pub joined: Option<BattlefieldJoined>,
    pub last_error: Option<u16>,
    pub last_rejection: Option<String>,
}
impl Battlefield {
    /// Recheck a captured invitation before writing a response. No local state is changed.
    pub fn validate_command(&self, command: BattlefieldCommand) -> std::result::Result<(), &'static str> {
        command.validate()?;
        if let BattlefieldCommand::RespondToInvitation { invitation, .. } = command {
            if self.invitation != Some(invitation) || self.response_pending.is_some() || self.cancel_pending || self.joined.is_some() {
                return Err("battlefield invitation is stale or already answered");
            }
        }
        Ok(())
    }
    pub fn respond_to_invitation(&self, invitation: BattlefieldInvitation, accept: bool) -> std::result::Result<BattlefieldCommand, &'static str> {
        let command = BattlefieldCommand::RespondToInvitation { invitation, accept };
        self.validate_command(command)?;
        Ok(command)
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
    pub fn apply(&mut self, e: &Event) -> bool {
        let before = self.clone();
        match e {
            Event::CharacterSelected(_) | Event::LeftWorld { .. } | Event::Disconnected { .. } | Event::Kicked { .. } => *self = Self::default(),
            Event::Battlefield(BattlefieldEvent::Requested(c)) => {
                self.last_rejection = None;
                match *c {
                    BattlefieldCommand::Apply { battlefield_id, corps } => {
                        self.requesting = Some((battlefield_id, corps));
                    }
                    BattlefieldCommand::Cancel => self.cancel_pending = true,
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
                    *self = Self::default();
                    self.last_error = Some(*error);
                }
            }
            Event::Battlefield(BattlefieldEvent::Cancelled { .. }) => *self = Self::default(),
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
                self.joined = Some(*joined);
                self.response_pending = None;
                self.invitation = None;
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
