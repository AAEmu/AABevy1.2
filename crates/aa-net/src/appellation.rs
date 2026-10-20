//! Appellations (titles): the owned list, the gained notice, the change of the shown title and the
//! `CSChangeAppellation` request.
//!
//! The wire layouts are the server's (`packets/world.rs` `SCAppellationsPacket`,
//! `systems/quests/packets.rs` `SCAppellationGainedPacket`, `systems/services/character_packets.rs`
//! `SCAppellationChangedPacket` / `CSChangeAppellationPacket`; C# `CharacterAppellations`):
//!
//! | packet | body |
//! |---|---|
//! | `SCAppellations` 0x1a4 | `i32 count`, then per title `u32 id, bool active`. Sent on entering the world, in chunks of 512 |
//! | `SCAppellationGained` 0x1a5 | `u32 id` (added to the list; it does not become the shown one) |
//! | `SCAppellationChanged` 0x1a6 | `bc obj_id, u32 id`, broadcast to the players around and to the owner (0: no title) |
//! | `CSChangeAppellation` 0x0f9 | `u32 id` (0 takes the title off; an id the character does not own is refused with a warning, the server still answers with the current one) |
use crate::{
    session::{Event, Phase},
    wire::{Reader, Result, WireError},
};

pub const SC_APPELLATIONS: u16 = 0x1a4;
pub const SC_APPELLATION_GAINED: u16 = 0x1a5;
pub const SC_APPELLATION_CHANGED: u16 = 0x1a6;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppellationEvent {
    /// The titles of the character (a chunk of the whole list) and which one is shown.
    List(Vec<(u32, bool)>),
    Gained { id: u32 },
    /// A unit shows another title (0: none).
    Changed { obj_id: u32, id: u32 },
}

impl AppellationEvent {
    pub fn handles(opcode: u16) -> bool {
        matches!(opcode, SC_APPELLATIONS..=SC_APPELLATION_CHANGED)
    }
    pub fn name(&self) -> &'static str {
        match self {
            Self::List(_) => "SCAppellations",
            Self::Gained { .. } => "SCAppellationGained",
            Self::Changed { .. } => "SCAppellationChanged",
        }
    }
    pub fn read(opcode: u16, r: &mut Reader) -> Result<Self> {
        Ok(match opcode {
            SC_APPELLATIONS => {
                let count = usize::try_from(r.i32()?).map_err(|_| WireError::Invalid("negative appellation count".into()))?;
                // Each entry is five bytes: a count the body cannot hold is a truncated packet.
                let mut list = Vec::with_capacity(count.min(r.remaining() / 5));
                for _ in 0..count {
                    list.push((r.u32()?, r.bool()?));
                }
                Self::List(list)
            }
            SC_APPELLATION_GAINED => Self::Gained { id: r.u32()? },
            SC_APPELLATION_CHANGED => Self::Changed { obj_id: r.bc()?, id: r.u32()? },
            _ => return Err(WireError::Invalid("unknown appellation opcode".into())),
        })
    }
}

/// What the player's own title list says (`X2Player:GetAppellationCount`, `GetAppellations`,
/// `GetShowingAppellation`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "bevy", derive(bevy::ecs::resource::Resource))]
pub struct Appellations {
    /// The player's unit, to tell its title change from the others.
    pub own_obj_id: Option<u32>,
    /// The owned titles in the order the server sent them.
    pub owned: Vec<u32>,
    /// The shown title, 0 for none.
    pub active: u32,
    /// Titles gained since the last [`Appellations::take_gained`], in order.
    pub gained: Vec<u32>,
    /// Bumped whenever the owned list, the shown title or the gained list changed.
    pub revision: u32,
}

impl Appellations {
    pub fn is_owned(&self, id: u32) -> bool {
        self.owned.contains(&id)
    }
    /// The titles gained since the last call (the "you gained the title" message).
    pub fn take_gained(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.gained)
    }
    pub fn apply(&mut self, event: &Event) -> bool {
        // the lists only grow (or reset): their lengths tell a change, without a copy for every event
        let before = (self.owned.len(), self.active, self.own_obj_id, self.gained.len());
        match event {
            // The server sends the list while selecting the character (`session.rs` `select_loaded`), before the
            // player's unit spawns: the spawn keeps it and only learns the own object id.
            Event::PlayerSpawn(spawn) => self.own_obj_id = Some(spawn.obj_id),
            Event::LoginSucceeded(_)
            | Event::LeftWorld { .. }
            | Event::Disconnected { .. }
            | Event::Kicked { .. }
            | Event::PhaseChanged(Phase::Authenticating | Phase::Connecting) => *self = Self { revision: self.revision, ..Self::default() },
            Event::Appellation(AppellationEvent::List(list)) => {
                for &(id, active) in list {
                    if !self.owned.contains(&id) {
                        self.owned.push(id);
                    }
                    if active {
                        self.active = id;
                    }
                }
            }
            Event::Appellation(AppellationEvent::Gained { id }) => {
                if !self.owned.contains(id) {
                    self.owned.push(*id);
                    self.gained.push(*id);
                }
            }
            Event::Appellation(AppellationEvent::Changed { obj_id, id }) if self.own_obj_id == Some(*obj_id) => self.active = *id,
            _ => {}
        }
        let changed = before != (self.owned.len(), self.active, self.own_obj_id, self.gained.len());
        if changed {
            self.revision = self.revision.wrapping_add(1);
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        game::{ClientMsg, ServerMsg},
        session::PlayerSpawn,
        wire::Writer,
    };

    fn frame(opcode: u16, body: &[u8]) -> Vec<u8> {
        let mut w = Writer::new();
        w.u8(crate::game::MAGIC).u8(crate::game::LEVEL_GAME).u8(0).u8(0).u16(opcode).raw(body);
        w.finish()
    }

    /// The bodies of the server's writers: `SCAppellationsPacket { [(7, true), (0x0102, false)] }`,
    /// `SCAppellationGainedPacket { 0x01020304 }`, `SCAppellationChangedPacket { obj 0x030201, id 9 }`.
    fn goldens() -> [(u16, Vec<u8>, AppellationEvent); 4] {
        [
            (SC_APPELLATIONS, vec![2, 0, 0, 0, 7, 0, 0, 0, 1, 0x02, 0x01, 0, 0, 0], AppellationEvent::List(vec![(7, true), (0x0102, false)])),
            (SC_APPELLATIONS, vec![0, 0, 0, 0], AppellationEvent::List(vec![])),
            (SC_APPELLATION_GAINED, vec![4, 3, 2, 1], AppellationEvent::Gained { id: 0x01020304 }),
            (SC_APPELLATION_CHANGED, vec![1, 2, 3, 9, 0, 0, 0], AppellationEvent::Changed { obj_id: 0x030201, id: 9 }),
        ]
    }

    #[test]
    fn server_goldens_decode_and_every_truncation_fails() {
        for (opcode, body, expected) in goldens() {
            let mut r = Reader::new(&body);
            assert_eq!(AppellationEvent::read(opcode, &mut r).unwrap(), expected);
            assert!(r.is_empty());
            assert_eq!(ServerMsg::decode(&frame(opcode, &body)).unwrap(), ServerMsg::Appellation(expected));
            for n in 0..body.len() {
                assert!(AppellationEvent::read(opcode, &mut Reader::new(&body[..n])).is_err(), "{opcode:x}/{n}");
                assert!(ServerMsg::decode(&frame(opcode, &body[..n])).is_err(), "frame {opcode:x}/{n}");
            }
        }
        assert!(AppellationEvent::read(0x1a7, &mut Reader::new(&[])).is_err());
        // a negative count, and a count the body cannot hold
        assert!(AppellationEvent::read(SC_APPELLATIONS, &mut Reader::new(&[0xff, 0xff, 0xff, 0xff])).is_err());
        assert!(AppellationEvent::read(SC_APPELLATIONS, &mut Reader::new(&[0xff, 0xff, 0xff, 0x7f])).is_err());
    }

    #[test]
    fn the_change_request_encodes_as_the_server_reads_it() {
        let change = ClientMsg::ChangeAppellation { id: 0x01020304 };
        assert_eq!(change.name(), "CSChangeAppellationPacket");
        assert_eq!(change.encode_body(), [4, 3, 2, 1]);
        assert_eq!(change.encode(3), [0xdd, 1, 0, 3, 0xf9, 0, 4, 3, 2, 1]);
        assert_eq!(ClientMsg::ChangeAppellation { id: 0 }.encode_body(), [0, 0, 0, 0]);
    }

    fn spawned() -> Appellations {
        let mut a = Appellations::default();
        let spawn = PlayerSpawn { obj_id: 0x11, state: Default::default(), unit: crate::state::tests::unit(0x11) };
        assert!(a.apply(&Event::PlayerSpawn(Box::new(spawn))));
        a
    }

    #[test]
    fn the_list_the_gain_and_the_change_fold_into_the_titles() {
        let mut a = spawned();
        assert_eq!(a.own_obj_id, Some(0x11));
        assert!(a.apply(&Event::Appellation(AppellationEvent::List(vec![(5, false), (7, true)]))));
        assert_eq!((a.owned.as_slice(), a.active), ([5, 7].as_slice(), 7));
        // a repeated list (a second chunk) changes nothing
        assert!(!a.apply(&Event::Appellation(AppellationEvent::List(vec![(5, false)]))));
        assert!(a.apply(&Event::Appellation(AppellationEvent::Gained { id: 9 })));
        assert!(!a.apply(&Event::Appellation(AppellationEvent::Gained { id: 9 })));
        assert_eq!(a.active, 7);
        assert_eq!(a.take_gained(), vec![9]);
        assert!(a.take_gained().is_empty());
        // another unit's title is not the player's
        assert!(!a.apply(&Event::Appellation(AppellationEvent::Changed { obj_id: 0x12, id: 5 })));
        assert!(a.apply(&Event::Appellation(AppellationEvent::Changed { obj_id: 0x11, id: 5 })));
        assert_eq!(a.active, 5);
        assert!(a.apply(&Event::Appellation(AppellationEvent::Changed { obj_id: 0x11, id: 0 })));
        assert_eq!(a.active, 0);
    }

    #[test]
    fn the_list_sent_before_the_spawn_survives_it() {
        let mut a = Appellations::default();
        a.apply(&Event::Appellation(AppellationEvent::List(vec![(5, false), (7, true)])));
        let spawn = PlayerSpawn { obj_id: 0x11, state: Default::default(), unit: crate::state::tests::unit(0x11) };
        a.apply(&Event::PlayerSpawn(Box::new(spawn)));
        assert_eq!((a.owned.as_slice(), a.active, a.own_obj_id), ([5, 7].as_slice(), 7, Some(0x11)));
    }

    #[test]
    fn leaving_the_world_forgets_the_titles() {
        let mut a = spawned();
        a.apply(&Event::Appellation(AppellationEvent::List(vec![(5, true)])));
        let revision = a.revision;
        assert!(a.apply(&Event::LeftWorld { target: 0 }));
        assert_eq!((a.owned.len(), a.active, a.own_obj_id), (0, 0, None));
        assert!(a.revision != revision);
    }

    #[test]
    fn a_unit_shows_the_title_of_the_change() {
        let mut units = crate::state::Units::default();
        units.apply(&Event::UnitSpawned(Box::new(crate::state::tests::unit(5))));
        assert!(units.apply(&Event::Appellation(AppellationEvent::Changed { obj_id: 5, id: 321 })));
        assert_eq!(units.by_id[&5].appellation, 321);
        assert!(!units.apply(&Event::Appellation(AppellationEvent::Changed { obj_id: 5, id: 321 })));
        assert!(!units.apply(&Event::Appellation(AppellationEvent::Changed { obj_id: 6, id: 1 })));
    }
}
