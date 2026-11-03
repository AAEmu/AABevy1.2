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
/// Native `FUN_397a9880`: `gain = (int)(point * 100f * 0.01f)`, capped at the grade-0 up_limit
/// (10000 when the catalog is missing). A missing entry inserts that gain at grade 0 and returns
/// it. Otherwise the limit is grade 0 for language ids 21..30 (`FUN_397a7c60`), the last grade for
/// id 19 (`FUN_397a8d80`), or the current grade's up_limit; the new total is clamped to
/// `[0, limit]`, the grade is recomputed by `FUN_397a8640`, and the actual difference is returned.
fn gain_of(point: i32) -> i32 {
    (point as f32 * 100.0 * 0.01) as i32
}

fn grade0_limit(up_limits: &[i32]) -> i32 {
    up_limits.first().copied().unwrap_or(10000)
}

fn gain_limit(id: u32, step: u8, up_limits: &[i32]) -> i32 {
    if (21..=30).contains(&id) {
        grade0_limit(up_limits)
    } else if id == 19 {
        up_limits.last().copied().unwrap_or(10000)
    } else {
        up_limits.get(usize::from(step)).copied().unwrap_or(10000)
    }
}

fn add_point(entry: &mut Actability, id: u32, point: i32, up_limits: &[i32], new_entry: bool) -> i32 {
    let gain = gain_of(point).min(grade0_limit(up_limits));
    if new_entry {
        entry.point = Some(gain);
        entry.step = 0;
        return gain;
    }
    let Some(old) = entry.point else {
        // Native always has an int total; the lane's downgrade ACK omits it. Keep the unknown
        // total (the catalog reconciliation fills it) and report the capped gain.
        return gain;
    };
    let limit = gain_limit(id, entry.step, up_limits);
    let new = old.wrapping_add(gain).clamp(0, limit);
    entry.point = Some(new);
    entry.step = grade_for(up_limits, new);
    new.wrapping_sub(old)
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
    /// ALL `expert_limits` up_limits ordered by up_limit ASC, indexed from step zero. The client
    /// installs this from its static catalog; native `FUN_397a9880` clamps a gained total to the
    /// current grade's limit and `FUN_397a8640` recomputes the grade from the total.
    pub up_limits: Vec<i32>,
    snapshot_open: bool,
    unresolved_downgrades: HashMap<u32, u8>,
}
/// Native `FUN_397a8640`: the first grade whose up_limit is at or above `point` (0 when none).
fn grade_for(up_limits: &[i32], point: i32) -> u8 {
    up_limits
        .iter()
        .position(|&limit| point <= limit)
        .and_then(|step| u8::try_from(step).ok())
        .unwrap_or(0)
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
        self.apply_with_gain(event).0
    }

    /// Apply an event and return the actual actability gain native `FUN_397a9880` reports for
    /// `SCCharacterLaborPowerChanged` / `SCAddActionPoint` (None for other events).
    pub fn apply_with_gain(&mut self, event: &Event) -> (bool, Option<i32>) {
        let Event::Actability(event) = event else {
            match event {
                Event::CharacterSelected(state) => {
                    let up_limits = std::mem::take(&mut self.up_limits);
                    *self = Self {
                        expanded_expert: Some(state.expanded_expert),
                        up_limits,
                        ..Self::default()
                    };
                    return (true, None);
                }
                Event::LeftWorld { .. } => {
                    // Keep the static limits across the world reset; they are catalog data,
                    // not character state. Compare the remaining state for change detection.
                    let up_limits = std::mem::take(&mut self.up_limits);
                    let changed = *self != Self::default();
                    *self = Self { up_limits, ..Self::default() };
                    return (changed, None);
                }
                Event::Disconnected { .. } | Event::Kicked { .. } => {
                    let changed = *self != Self::default();
                    *self = Self::default();
                    return (changed, None);
                }
                Event::LaborChanged { action, point, .. } if *action > 0 && *point != 0 => {
                    self.unresolved_downgrades.remove(&(*action as u32));
                    let limits = self.up_limits.clone();
                    let new_entry = self.get(*action as u32).is_none();
                    let entry = self.get_mut(*action as u32);
                    let gain = add_point(entry, *action as u32, *point, &limits, new_entry);
                    return (true, Some(gain));
                }
                Event::ActionPoint { action, value } if *action > 0 => {
                    self.unresolved_downgrades.remove(&(*action as u32));
                    let limits = self.up_limits.clone();
                    let new_entry = self.get(*action as u32).is_none();
                    let entry = self.get_mut(*action as u32);
                    let gain = add_point(entry, *action as u32, *value, &limits, new_entry);
                    return (true, Some(gain));
                }
                _ => return (false, None),
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
        (true, None)
    }
}

#[cfg(test)]
mod tests;
