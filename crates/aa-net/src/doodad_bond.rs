//! Received doodad seating, independent of mount/slave attachment notifications.

use std::collections::{HashMap, HashSet};

use crate::{model::{Unit, UnitKind}, opcodes::sc, session::{Event, Phase}, wire::{Reader, Result, WireError}};

/// Native `BondDoodad`: all fields are received, including signed space/spot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DoodadBond {
    pub owner_obj_id: u32,
    pub attach_point: u8,
    pub kind: u8,
    pub space: i32,
    pub spot: i32,
}

impl DoodadBond {
    pub(crate) fn read_after_point(r: &mut Reader, attach_point: u8) -> Result<Self> {
        Ok(Self { owner_obj_id: r.bc()?, attach_point, kind: r.u8()?, space: r.i32()?, spot: r.i32()? })
    }
}

/// Capture the selected character, actor and entire received bond when queuing an unbond.
/// Only the two object IDs go on the wire; the other fields reject stale queued work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnbondDoodadRequest {
    pub character_id: u32,
    pub actor_obj_id: u32,
    pub bond: DoodadBond,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DoodadBondEvent {
    Bonded { unit_obj_id: u32, bond: DoodadBond },
    /// Native SC139's complete 0xff sentinel. The receive gate does not attach/detach.
    BondAbsent { unit_obj_id: u32 },
    Unbonded { character_obj_id: u32, character_id: u32, doodad_obj_id: u32 },
    /// Successful socket write only. Received history is unchanged.
    UnbondRequested(UnbondDoodadRequest),
    UnbondRejected { request: UnbondDoodadRequest, message: String },
}

impl DoodadBondEvent {
    pub fn handles(opcode: u16) -> bool { matches!(opcode, sc::SCBondDoodadPacket | sc::SCUnbondDoodadPacket) }
    pub fn name(&self) -> &'static str {
        match self {
            Self::Bonded { .. } | Self::BondAbsent { .. } => "SCBondDoodad",
            Self::Unbonded { .. } => "SCUnbondDoodad",
            Self::UnbondRequested(_) => "UnbondDoodadRequested",
            Self::UnbondRejected { .. } => "UnbondDoodadRejected",
        }
    }
    pub fn read(opcode: u16, r: &mut Reader) -> Result<Self> {
        Ok(match opcode {
            sc::SCBondDoodadPacket => {
                let unit_obj_id = r.bc()?;
                let point = r.u8()?;
                if point == 255 { Self::BondAbsent { unit_obj_id } }
                else { Self::Bonded { unit_obj_id, bond: DoodadBond::read_after_point(r, point)? } }
            }
            sc::SCUnbondDoodadPacket => Self::Unbonded { character_obj_id: r.bc()?, character_id: r.u32()?, doodad_obj_id: r.bc()? },
            _ => return Err(WireError::Invalid(format!("unknown doodad bond opcode {opcode:03x}"))),
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "bevy", derive(bevy::ecs::resource::Resource))]
pub struct DoodadBonds {
    pub by_unit: HashMap<u32, DoodadBond>,
    actors: HashMap<u32, u32>,
    doodads: HashSet<u32>,
    selected_id: u32,
    own_obj_id: u32,
    own_known: bool,
    // A successful local send opens the native receive gate, not received free history.
    own_rebond_eligible: bool,
}

impl DoodadBonds {
    pub fn own(&self) -> Option<&DoodadBond> {
        if self.actors.get(&self.own_obj_id) != Some(&self.selected_id) { return None; }
        self.by_unit.get(&self.own_obj_id)
    }

    /// False means unknown history, not an available seat. Only an actual own snapshot or
    /// matched bond receipt restores certainty after decode loss/removal.
    pub fn own_known(&self) -> bool { self.own_known }

    pub(crate) fn matches_request(&self, request: &UnbondDoodadRequest) -> bool {
        self.own_known && self.selected_id == request.character_id && self.own_obj_id == request.actor_obj_id
            && self.actors.get(&request.actor_obj_id) == Some(&request.character_id)
            && self.own() == Some(&request.bond)
    }

    fn invalidate_object(&mut self, id: u32) {
        if id == self.own_obj_id || self.own().is_some_and(|b| b.owner_obj_id == id) {
            self.own_known = false;
            self.own_rebond_eligible = false;
        }
        self.by_unit.retain(|actor, bond| *actor != id && bond.owner_obj_id != id);
    }

    fn snapshot(&mut self, unit: &Unit) {
        if self.actors.contains_key(&unit.obj_id) || self.doodads.remove(&unit.obj_id) {
            self.invalidate_object(unit.obj_id);
        }
        self.actors.remove(&unit.obj_id);
        let UnitKind::Character { char_id } = unit.kind else { return };
        if char_id == 0 || unit.obj_id == 0 { return; }
        self.actors.insert(unit.obj_id, char_id);
        if let Some(bond) = unit.doodad_bond { self.by_unit.insert(unit.obj_id, bond); }
        if unit.obj_id == self.own_obj_id && char_id == self.selected_id {
            self.own_known = true;
            self.own_rebond_eligible = false;
        }
    }

    pub fn apply(&mut self, event: &Event) -> bool {
        // Unrelated movement traffic never clones or allocates these tables.
        match event {
            Event::CharacterSelected(state) => {
                let selected_id = state.character.id;
                let changed = *self != Self::default() || selected_id != 0;
                *self = Self { selected_id, ..Self::default() };
                changed
            }
            Event::LeftWorld { .. } | Event::Disconnected { .. } | Event::Kicked { .. }
            | Event::PhaseChanged(Phase::Idle | Phase::Authenticating | Phase::WorldSelect | Phase::CharacterSelect | Phase::ReadyToConnect | Phase::Connecting) => {
                if *self == Self::default() { return false; }
                *self = Self::default(); true
            }
            Event::PlayerSpawn(player) if player.state.character.id == self.selected_id && self.selected_id != 0
                && player.obj_id == player.unit.obj_id && matches!(player.unit.kind, UnitKind::Character { char_id } if char_id == self.selected_id) => {
                if self.own_obj_id != 0 && self.own_obj_id != player.obj_id { self.invalidate_object(self.own_obj_id); }
                self.own_obj_id = player.obj_id;
                self.snapshot(&player.unit); true
            }
            Event::OwnUnitSnapshot(unit) if unit.obj_id == self.own_obj_id && self.own_obj_id != 0
                && matches!(unit.kind, UnitKind::Character { char_id } if char_id == self.selected_id) => { self.snapshot(unit); true }
            Event::UnitSpawned(unit) if self.own_obj_id != 0 => { self.snapshot(unit); true }
            Event::DoodadBond(DoodadBondEvent::UnbondRequested(request)) if self.matches_request(request) => {
                let changed = !self.own_rebond_eligible;
                self.own_rebond_eligible = true;
                changed
            }
            Event::DoodadBond(DoodadBondEvent::Bonded { unit_obj_id, bond }) => {
                let own = *unit_obj_id == self.own_obj_id && self.actors.get(unit_obj_id) == Some(&self.selected_id);
                // Native3935a520 clears locally after sending. Keep received history until a
                // real receipt, but allow that next own SC139 through native393566c0's gate.
                if !self.actors.contains_key(unit_obj_id) || bond.owner_obj_id == 0
                    || (self.by_unit.contains_key(unit_obj_id) && !(own && self.own_rebond_eligible)) { return false; }
                self.by_unit.insert(*unit_obj_id, *bond);
                if own {
                    self.own_known = true;
                    self.own_rebond_eligible = false;
                }
                true
            }
            Event::DoodadBond(DoodadBondEvent::Unbonded { character_obj_id, character_id, doodad_obj_id }) => {
                if self.actors.get(character_obj_id) != Some(character_id)
                    || !self.by_unit.get(character_obj_id).is_some_and(|b| b.owner_obj_id == *doodad_obj_id) { return false; }
                self.by_unit.remove(character_obj_id);
                if *character_obj_id == self.own_obj_id { self.own_rebond_eligible = false; }
                // A matched receipt preserves known history; unmatched receipts cannot repair it.
                // SC13A has no bond generation: an old same-owner ACK is indistinguishable.
                true
            }
            Event::GameDecodeFailed { .. } | Event::UnhandledGamePacket { level: crate::game::LEVEL_COMPRESSED, len: 1.., .. } => {
                let changed = self.own_known || !self.by_unit.is_empty();
                self.by_unit.clear(); self.own_known = false; self.own_rebond_eligible = false; changed
            }
            Event::UnitsRemoved(ids) => {
                let mut changed = false;
                for id in ids { changed |= self.actors.remove(id).is_some() || self.by_unit.contains_key(id) || self.by_unit.values().any(|b| b.owner_obj_id == *id); self.invalidate_object(*id); }
                changed
            }
            Event::DoodadsRemoved(ids) => {
                let mut changed = false;
                for id in ids { changed |= self.doodads.remove(id) || self.by_unit.values().any(|b| b.owner_obj_id == *id); self.invalidate_object(*id); }
                changed
            }
            Event::DoodadsSpawned(doodads) if self.own_obj_id != 0 => {
                for doodad in doodads {
                    let existed = !self.doodads.insert(doodad.obj_id);
                    let actor = self.actors.remove(&doodad.obj_id).is_some();
                    if existed || actor { self.invalidate_object(doodad.obj_id); }
                }
                !doodads.is_empty()
            }
            _ => false,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;
