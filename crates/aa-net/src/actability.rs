//! Authoritative crafting/gathering proficiency, separate from combat ability experience.
use crate::opcodes::{cs, sc};
use crate::session::Event;
use crate::wire::{Reader, Result, WireError, Writer};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActabilityCommand {
    Upgrade { id: u32, auto_use_aa_point: bool },
    Downgrade { id: u32, auto_use_aa_point: bool },
    Expand,
}
impl ActabilityCommand {
    pub fn validate(self) -> std::result::Result<(), &'static str> {
        match self {
            Self::Upgrade { id: 0, .. } | Self::Downgrade { id: 0, .. } => {
                Err("actability ID must be nonzero")
            }
            _ => Ok(()),
        }
    }
    pub fn opcode(self) -> u16 {
        match self {
            Self::Upgrade { .. } => cs::CSUpgradeExpertLimitPacket,
            Self::Downgrade { .. } => cs::CSDowngradeExpertLimitPacket,
            Self::Expand => cs::CSExpandExpertPacket,
        }
    }
    pub fn write(self, w: &mut Writer) {
        match self {
            Self::Upgrade {
                id,
                auto_use_aa_point,
            }
            | Self::Downgrade {
                id,
                auto_use_aa_point,
            } => {
                w.u32(id).bool(auto_use_aa_point);
            }
            Self::Expand => {}
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActabilityValue {
    pub id: u32,
    pub point: i32,
    pub step: u8,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActabilityEvent {
    Snapshot {
        last: bool,
        entries: Vec<ActabilityValue>,
    },
    TierChanged {
        is_upgrade: bool,
        id: u32,
        step: u8,
    },
    Expanded {
        count: u8,
    },
    Requested(ActabilityCommand),
    RequestRejected {
        command: ActabilityCommand,
        message: String,
    },
}
impl ActabilityEvent {
    pub fn handles(opcode: u16) -> bool {
        matches!(
            opcode,
            sc::SCActabilityPacket | sc::SCExpertLimitModifiedPacket | sc::SCExpertExpandedPacket
        )
    }
    pub fn name(&self) -> &'static str {
        match self {
            Self::Snapshot { .. } => "SCActability",
            Self::TierChanged { .. } => "SCExpertLimitModified",
            Self::Expanded { .. } => "SCExpertExpanded",
            Self::Requested(_) => "ActabilityRequested",
            Self::RequestRejected { .. } => "ActabilityRequestRejected",
        }
    }
    pub fn read(opcode: u16, r: &mut Reader) -> Result<Self> {
        Ok(match opcode {
            sc::SCActabilityPacket => {
                let last = r.bool()?;
                let count = usize::from(r.u8()?);
                if count > r.remaining() / 9 {
                    return Err(WireError::Invalid("invalid actability count".into()));
                }
                let mut ids = HashSet::with_capacity(count);
                let mut entries = Vec::with_capacity(count);
                for _ in 0..count {
                    let entry = ActabilityValue {
                        id: r.u32()?,
                        point: r.i32()?,
                        step: r.u8()?,
                    };
                    if entry.id == 0 || !ids.insert(entry.id) {
                        return Err(WireError::Invalid("invalid/duplicate actability ID".into()));
                    }
                    entries.push(entry);
                }
                Self::Snapshot { last, entries }
            }
            sc::SCExpertLimitModifiedPacket => {
                let (is_upgrade, id, step) = (r.bool()?, r.u32()?, r.u8()?);
                if id == 0 {
                    return Err(WireError::Invalid("invalid actability ID".into()));
                }
                Self::TierChanged {
                    is_upgrade,
                    id,
                    step,
                }
            }
            sc::SCExpertExpandedPacket => Self::Expanded { count: r.u8()? },
            _ => {
                return Err(WireError::Invalid(format!(
                    "unknown actability opcode {opcode:03x}"
                )));
            }
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Actability {
    pub id: u32,
    /// None until a snapshot, or after downgrade: the ACK omits the reset point total.
    pub point: Option<i32>,
    pub step: u8,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActabilityFailure {
    pub command: ActabilityCommand,
    pub message: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "bevy", derive(bevy::ecs::resource::Resource))]
pub struct Actabilities {
    pub entries: Vec<Actability>,
    pub loaded: bool,
    pub expanded_expert: Option<u8>,
    pub pending: Vec<ActabilityCommand>,
    pub last_error: Option<ActabilityFailure>,
    snapshot_open: bool,
    unresolved_downgrades: HashMap<u32, u8>,
}
impl Actabilities {
    /// Reconcile an already-applied lane downgrade ACK with the matching static catalog.
    /// `up_limits` must contain ALL expert_limits rows ordered by up_limit ASC, indexed
    /// from step zero (not DB IDs and not the native SHOW-filtered grade list).
    /// No result is inferred from a request, upgrade, stale ACK or an intervening point update.
    /// Native clamps old points; the active lane instead assigns this exact limit.
    pub fn resolve_downgrade_points(&mut self, event: &Event, up_limits: &[i32]) -> bool {
        let Event::Actability(ActabilityEvent::TierChanged { is_upgrade: false, id, step }) = event else { return false };
        if self.unresolved_downgrades.get(id) != Some(step) || up_limits.windows(2).any(|pair| pair[0] > pair[1]) {
            return false;
        }
        let Some(point) = up_limits.get(usize::from(*step)).copied() else { return false };
        let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == *id && entry.step == *step && entry.point.is_none()) else { return false };
        entry.point = Some(point);
        self.unresolved_downgrades.remove(id);
        true
    }
    pub fn get(&self, id: u32) -> Option<&Actability> {
        self.entries.iter().find(|a| a.id == id)
    }
    fn get_mut(&mut self, id: u32) -> &mut Actability {
        let index = self
            .entries
            .iter()
            .position(|a| a.id == id)
            .unwrap_or_else(|| {
                self.entries.push(Actability {
                    id,
                    point: None,
                    step: 0,
                });
                self.entries.len() - 1
            });
        &mut self.entries[index]
    }
    pub fn count_at_step(&self, step: u8) -> usize {
        self.entries.iter().filter(|a| a.step == step).count()
    }
    pub fn apply(&mut self, event: &Event) -> bool {
        let Event::Actability(event) = event else {
            match event {
                Event::CharacterSelected(state) => {
                    *self = Self {
                        expanded_expert: Some(state.expanded_expert),
                        ..Self::default()
                    };
                    return true;
                }
                Event::LeftWorld { .. } | Event::Disconnected { .. } | Event::Kicked { .. } => {
                    let changed = *self != Self::default();
                    *self = Self::default();
                    return changed;
                }
                Event::LaborChanged {
                    action,
                    point,
                    step,
                    ..
                } if *action > 0 => {
                    self.unresolved_downgrades.remove(&(*action as u32));
                    let entry = self.get_mut(*action as u32);
                    // The wire contains a capped delta, never an absolute point total.
                    entry.point = entry.point.map(|old| old.wrapping_add(*point));
                    entry.step = *step;
                    return true;
                }
                _ => return false,
            }
        };
        match event {
            ActabilityEvent::Snapshot { last, entries } => {
                if !self.snapshot_open {
                    self.entries.clear();
                    self.unresolved_downgrades.clear();
                    self.loaded = false;
                }
                for entry in entries {
                    self.unresolved_downgrades.remove(&entry.id);
                    *self.get_mut(entry.id) = Actability {
                        id: entry.id,
                        point: Some(entry.point),
                        step: entry.step,
                    };
                }
                self.snapshot_open = !last;
                self.loaded = *last;
            }
            ActabilityEvent::TierChanged {
                is_upgrade,
                id,
                step,
            } => {
                if *is_upgrade { self.unresolved_downgrades.remove(id); }
                else { self.unresolved_downgrades.insert(*id, *step); }
                let entry = self.get_mut(*id);
                entry.step = *step;
                if !is_upgrade {
                    entry.point = None;
                }
                if let Some(index) = self.pending.iter().position(|p| match p {
                    ActabilityCommand::Upgrade { id: pending_id, .. } => {
                        *is_upgrade && id == pending_id
                    }
                    ActabilityCommand::Downgrade { id: pending_id, .. } => {
                        !is_upgrade && id == pending_id
                    }
                    ActabilityCommand::Expand => false,
                }) {
                    self.pending.remove(index);
                }
            }
            ActabilityEvent::Expanded { count } => {
                self.expanded_expert = Some(*count);
                if let Some(index) = self
                    .pending
                    .iter()
                    .position(|p| matches!(p, ActabilityCommand::Expand))
                {
                    self.pending.remove(index);
                }
            }
            ActabilityEvent::Requested(command) => {
                self.pending.push(*command);
                self.last_error = None;
            }
            ActabilityEvent::RequestRejected { command, message } => {
                self.last_error = Some(ActabilityFailure {
                    command: *command,
                    message: message.clone(),
                });
            }
        }
        true
    }
}

#[cfg(test)]
mod tests;
