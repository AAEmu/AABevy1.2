//! Battlefield queue against the active Bevy lane, with explicit native transport differences.
//! Received state stays separate from successful local writes.
use crate::{
    opcodes::{cs, sc},
    session::Event,
    wire::{Reader, Result, WireError, Writer},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BattlefieldCommand {
    Apply { battlefield_id: u32, corps: u8 },
    Cancel,
}
impl BattlefieldCommand {
    pub fn validate(self) -> std::result::Result<(), &'static str> {
        match self {
            Self::Apply { battlefield_id: 0, .. } => Err("battlefield ID must be nonzero"),
            Self::Apply { corps: 3.., .. } => Err("battlefield corps must be 0, 1 or 2"),
            _ => Ok(()),
        }
    }
    pub fn opcode(self) -> u16 {
        match self {
            Self::Apply { .. } => cs::CSApplyToInstantGamePacket,
            Self::Cancel => cs::CSCancelInstantGamePacket,
        }
    }
    pub fn write(self, w: &mut Writer) {
        if let Self::Apply { battlefield_id, corps } = self {
            w.u32(battlefield_id).u8(corps);
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
        matches!(op, 0x18b..=0x18f)
    }
    pub fn name(&self) -> &'static str {
        match self {
            Self::Applied { .. } => "SCAppliedToInstantGame",
            Self::Cancelled { .. } => "SCCancelInstantGame",
            Self::QueueState(_) => "SCInstantGameState",
            Self::Invited(_) => "SCInviteToInstantGame",
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
    pub last_error: Option<u16>,
    pub last_rejection: Option<String>,
}
impl Battlefield {
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
        (cur, max, self.applied.map_or(-1, |(_, c)| i32::from(c)))
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
                }
            }
            Event::Battlefield(BattlefieldEvent::RequestRejected { message, .. }) => {
                self.last_rejection = Some(message.clone());
            }
            Event::Battlefield(BattlefieldEvent::Applied { battlefield_id, corps, error })
                if self.requesting == Some((*battlefield_id, *corps)) && !self.cancel_pending =>
            {
                if *error == 0 {
                    self.applied = self.requesting.take();
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
                if self.applied.is_some_and(|(id, _)| id == q.battlefield_id) && self.invitation.is_none() && !self.cancel_pending =>
            {
                self.queue = Some(*q)
            }
            Event::Battlefield(BattlefieldEvent::Invited(i))
                if self.applied.is_some_and(|(_, c)| (c <= 1 && c == i.corps) || (c == 2 && i.corps <= 1))
                    && self.invitation.is_none()
                    && !self.cancel_pending =>
            {
                self.invitation = Some(*i)
            }
            Event::Battlefield(BattlefieldEvent::InviteTimedOut {
                zone_id,
                instance_id,
                ruleset_id,
                corps,
            }) if self
                .invitation
                .is_some_and(|i| i.zone_id == *zone_id && i.instance_id == *instance_id && i.ruleset_id == *ruleset_id && i.corps == *corps)
                && !self.cancel_pending =>
            {
                self.invitation = None
            }
            _ => {}
        }
        *self != before
    }
}
#[cfg(test)]
mod tests;
