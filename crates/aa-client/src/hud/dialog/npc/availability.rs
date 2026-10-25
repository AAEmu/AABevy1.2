//! Actor lifetime/status evidence, independent of SC07b's interactable flag.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Constructed,
    Wire(u8),
    Unknown,
}

#[derive(Debug, Default)]
pub(super) struct Availability {
    actors: HashMap<u32, (u32, Status)>,
    // After lost history, an announcement may be an existing native actor's
    // repeated snapshot. It cannot establish a new constructor default.
    incomplete: bool,
}

impl Availability {
    pub(super) fn observe(&mut self, event: &Event) {
        match event {
            Event::CharacterSelected(_)
            | Event::Disconnected { .. }
            | Event::LeftWorld { .. }
            | Event::Kicked { .. }
            | Event::LoginFailed { .. } => *self = Self::default(),
            Event::LoadInstance(_) | Event::GameDecodeFailed { .. } => self.lose_history(),
            Event::UnhandledGamePacket { level, len, .. }
                if *level == aa_net::game::LEVEL_COMPRESSED && *len > 0 =>
            {
                self.lose_history();
            }
            Event::UnitSpawned(unit) => {
                if let UnitKind::Npc { template_id, .. } = unit.kind {
                    let status = if self.incomplete {
                        Status::Unknown
                    } else {
                        Status::Constructed
                    };
                    let entry = self
                        .actors
                        .entry(unit.obj_id)
                        .or_insert((template_id, status));
                    if entry.0 != template_id {
                        *entry = (template_id, status);
                    }
                } else {
                    self.actors.remove(&unit.obj_id);
                }
            }
            Event::UnitsRemoved(ids) => {
                for id in ids {
                    self.actors.remove(id);
                }
            }
            Event::Interaction(InteractEvent::NpcStatusChanged {
                npc_obj_id,
                status_wire_byte,
            }) => {
                // Native resolves an existing actor; an early status does not
                // create one or carry forward to later object-ID reuse.
                if let Some((_, status)) = self.actors.get_mut(npc_obj_id) {
                    *status = Status::Wire(*status_wire_byte);
                }
            }
            _ => {}
        }
    }

    fn lose_history(&mut self) {
        self.incomplete = true;
        for (_, status) in self.actors.values_mut() {
            *status = Status::Unknown;
        }
    }

    /// The 0/1 wire projection is the explicitly accepted Rust/lane convention.
    /// Concrete CryNetwork BOOL normalization remains unproved. In particular,
    /// raw 2/255 cannot stand in for the native post-archive actor byte.
    #[allow(dead_code)] // Consumed only with separately qualified context providers.
    pub(super) fn value(&self, npc: (u32, u32)) -> Option<u8> {
        let (template, status) = self.actors.get(&npc.0)?;
        if *template != npc.1 {
            return None;
        }
        match status {
            Status::Constructed => Some(1),
            Status::Wire(byte @ 0..=1) => Some(*byte),
            Status::Wire(_) | Status::Unknown => None,
        }
    }
}
