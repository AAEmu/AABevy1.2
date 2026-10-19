//! Duels: the challenge, the countdown, the fight and its result (`SCDuel*`, `CSChallengeDuel`,
//! `CSStartDuel`).
//!
//! The wire layouts are the server's (`systems/social/packets/duel.rs`):
//!
//! | packet | body |
//! |---|---|
//! | `SCDuelChallenged` 0x08b | `u32 challenger_character_id` |
//! | `SCDuelStartCountdown` 0x08c | empty |
//! | `SCDuelStarted` 0x08d | `bc challenger_obj, bc challenged_obj` |
//! | `SCDuelEnded` 0x08e | `u32 winner_id, u32 loser_id, bc winner_obj, bc loser_obj, u8 det` (a draw: the two sides in turn) |
//! | `SCDuelState` 0x08f | `bc challenger_obj, bc flag_obj` (flag 0: the duel is over) |
//! | `CSChallengeDuel` 0x050 | `u32 challenged_character_id` |
//! | `CSStartDuel` 0x051 | `u32 challenger_character_id, i16 error` (0 accepts, 507 refuses) |
//!
//! The temporary Red / Blue team factions of a started duel arrive as the usual
//! `SCUnitFactionChanged` (temp flag set) and are applied to units by [`crate::expedition`]'s
//! faction path; this module only tracks the duel itself.
use crate::{
    session::{Event, Phase},
    wire::{Reader, Result, WireError},
};

/// `CSStartDuel` error that refuses a challenge (`TargetRejectedDuel`).
pub const REJECTED: i16 = 507;

/// `DuelDetType`, the `det` of `SCDuelEnded`.
pub mod det {
    pub const LOSE: u8 = 0;
    pub const WIN: u8 = 1;
    /// A side ran away from the flag (or left the world).
    pub const SURRENDER: u8 = 2;
    pub const DRAW: u8 = 3;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DuelEvent {
    /// Another character challenged the player.
    Challenged { challenger_id: u32 },
    StartCountdown,
    Started { challenger_obj_id: u32, challenged_obj_id: u32 },
    /// The first pair is the winner (a draw: either side), the second the loser.
    Ended { winner_id: u32, loser_id: u32, winner_obj_id: u32, loser_obj_id: u32, det: u8 },
    /// The combat flag (`flag_obj_id` 0: gone, the duel is over).
    State { challenger_obj_id: u32, flag_obj_id: u32 },
}

impl DuelEvent {
    pub fn handles(opcode: u16) -> bool {
        matches!(opcode, 0x08b..=0x08f)
    }
    pub fn name(&self) -> &'static str {
        match self {
            Self::Challenged { .. } => "SCDuelChallenged",
            Self::StartCountdown => "SCDuelStartCountdown",
            Self::Started { .. } => "SCDuelStarted",
            Self::Ended { .. } => "SCDuelEnded",
            Self::State { .. } => "SCDuelState",
        }
    }
    pub fn read(opcode: u16, r: &mut Reader) -> Result<Self> {
        Ok(match opcode {
            0x08b => Self::Challenged { challenger_id: r.u32()? },
            0x08c => Self::StartCountdown,
            0x08d => Self::Started { challenger_obj_id: r.bc()?, challenged_obj_id: r.bc()? },
            0x08e => Self::Ended {
                winner_id: r.u32()?,
                loser_id: r.u32()?,
                winner_obj_id: r.bc()?,
                loser_obj_id: r.bc()?,
                det: r.u8()?,
            },
            0x08f => Self::State { challenger_obj_id: r.bc()?, flag_obj_id: r.bc()? },
            _ => return Err(WireError::Invalid("unknown duel opcode".into())),
        })
    }
}

/// What the local character takes from the end of a duel (the keys of `locale.duel`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DuelResult {
    Draw,
    Winner,
    Loser,
    /// The local character ran away from the flag.
    Runaway,
}

impl DuelResult {
    /// `result_draw` / `result_winner` / `result_loser` / `result_runaway` of `locale.duel`.
    pub fn key(self) -> &'static str {
        match self {
            Self::Draw => "result_draw",
            Self::Winner => "result_winner",
            Self::Loser => "result_loser",
            Self::Runaway => "result_runaway",
        }
    }
}

/// An event the UI reports, in order, after the state is updated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DuelNotice {
    /// `DLG_TASK_CHALLENGE_DUEL` with the challenger.
    Challenged { challenger_id: u32 },
    Countdown,
    /// `STARTED_DUEL()`.
    Started,
    /// `ENDED_DUEL()` and the result box.
    Ended(DuelResult),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActiveDuel {
    pub challenger_obj_id: u32,
    pub challenged_obj_id: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "bevy", derive(bevy::ecs::resource::Resource))]
pub struct Duels {
    /// The player's own character (to tell winner from loser).
    pub character_id: Option<u32>,
    /// The challenger the player has not answered yet.
    pub challenge: Option<u32>,
    pub countdown: bool,
    pub active: Option<ActiveDuel>,
    /// The combat flag doodad while the duel runs.
    pub flag_obj_id: Option<u32>,
}

impl Duels {
    /// Whether `obj_id` fights in the running duel.
    pub fn involves(&self, obj_id: u32) -> bool {
        self.active.is_some_and(|d| d.challenger_obj_id == obj_id || d.challenged_obj_id == obj_id)
    }
    /// The player answered the challenge (the request itself is a [`crate::Command`]).
    pub fn answered(&mut self, challenger_id: u32) -> bool {
        if self.challenge == Some(challenger_id) {
            self.challenge = None;
            return true;
        }
        false
    }
    pub fn apply(&mut self, event: &Event) -> bool {
        self.apply_with_notices(event).0
    }
    pub fn apply_with_notices(&mut self, event: &Event) -> (bool, Vec<DuelNotice>) {
        let before = self.clone();
        let mut notices = Vec::new();
        match event {
            Event::CharacterSelected(state) => self.character_id = Some(state.character.id),
            Event::PlayerSpawn(spawn) => self.character_id = Some(spawn.state.character.id),
            Event::LoginSucceeded(_)
            | Event::LeftWorld { .. }
            | Event::Disconnected { .. }
            | Event::Kicked { .. }
            | Event::PhaseChanged(Phase::Authenticating | Phase::Connecting) => *self = Self::default(),
            Event::Duel(receipt) => match *receipt {
                DuelEvent::Challenged { challenger_id } => {
                    self.challenge = Some(challenger_id);
                    notices.push(DuelNotice::Challenged { challenger_id });
                }
                DuelEvent::StartCountdown => {
                    self.countdown = true;
                    notices.push(DuelNotice::Countdown);
                }
                DuelEvent::Started { challenger_obj_id, challenged_obj_id } => {
                    self.countdown = false;
                    self.challenge = None;
                    self.active = Some(ActiveDuel { challenger_obj_id, challenged_obj_id });
                    notices.push(DuelNotice::Started);
                }
                DuelEvent::Ended { winner_id, loser_id, det, .. } => {
                    self.countdown = false;
                    self.active = None;
                    self.flag_obj_id = None;
                    self.challenge = None;
                    notices.push(DuelNotice::Ended(self.result(winner_id, loser_id, det)));
                }
                DuelEvent::State { flag_obj_id, .. } => self.flag_obj_id = (flag_obj_id != 0).then_some(flag_obj_id),
            },
            _ => {}
        }
        (*self != before, notices)
    }
    /// A draw is sent to each side with itself second; elsewhere the first id won. A surrender is
    /// the second id running away.
    fn result(&self, winner_id: u32, loser_id: u32, det: u8) -> DuelResult {
        if det == det::DRAW {
            DuelResult::Draw
        } else if self.character_id == Some(loser_id) {
            if det == det::SURRENDER { DuelResult::Runaway } else { DuelResult::Loser }
        } else if self.character_id == Some(winner_id) {
            DuelResult::Winner
        } else {
            DuelResult::Draw
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{game::{ClientMsg, ServerMsg}, wire::Writer};

    fn frame(opcode: u16, body: &[u8]) -> Vec<u8> {
        let mut w = Writer::new();
        w.u8(crate::game::MAGIC).u8(crate::game::LEVEL_GAME).u8(0).u8(0).u16(opcode).raw(body);
        w.finish()
    }

    #[test]
    fn server_goldens_decode_and_every_truncation_fails() {
        // The bodies of the server's `duel_packets_are_byte_exact` test.
        let cases: [(u16, Vec<u8>, DuelEvent); 5] = [
            (0x08b, vec![4, 3, 2, 1], DuelEvent::Challenged { challenger_id: 0x01020304 }),
            (0x08c, vec![], DuelEvent::StartCountdown),
            (
                0x08d,
                vec![0x11, 0, 0, 0x22, 0x22, 0x22],
                DuelEvent::Started { challenger_obj_id: 0x11, challenged_obj_id: 0x222222 },
            ),
            (
                0x08e,
                vec![1, 0, 0, 0, 2, 0, 0, 0, 1, 2, 3, 0x10, 0, 0, 3],
                DuelEvent::Ended { winner_id: 1, loser_id: 2, winner_obj_id: 0x030201, loser_obj_id: 0x10, det: 3 },
            ),
            (0x08f, vec![0x11, 0, 0, 0, 0, 0], DuelEvent::State { challenger_obj_id: 0x11, flag_obj_id: 0 }),
        ];
        for (opcode, body, expected) in cases {
            let mut r = Reader::new(&body);
            assert_eq!(DuelEvent::read(opcode, &mut r).unwrap(), expected);
            assert!(r.is_empty());
            assert_eq!(ServerMsg::decode(&frame(opcode, &body)).unwrap(), ServerMsg::Duel(expected));
            for n in 0..body.len() {
                assert!(DuelEvent::read(opcode, &mut Reader::new(&body[..n])).is_err(), "{opcode:x}/{n}");
                assert!(ServerMsg::decode(&frame(opcode, &body[..n])).is_err(), "frame {opcode:x}/{n}");
            }
        }
        assert!(DuelEvent::read(0x090, &mut Reader::new(&[])).is_err());
    }

    #[test]
    fn requests_encode_as_the_server_reads_them() {
        let challenge = ClientMsg::ChallengeDuel { challenged_id: 7 };
        assert_eq!(challenge.name(), "CSChallengeDuelPacket");
        assert_eq!(challenge.encode_body(), [7, 0, 0, 0]);
        assert_eq!(challenge.encode(3), [0xdd, 1, 0, 3, 0x50, 0, 7, 0, 0, 0]);
        let refuse = ClientMsg::StartDuel { challenger_id: 9, error: REJECTED };
        assert_eq!(refuse.name(), "CSStartDuelPacket");
        assert_eq!(refuse.encode_body(), [9, 0, 0, 0, 0xFB, 0x01]);
        let accept = ClientMsg::StartDuel { challenger_id: 9, error: 0 };
        assert_eq!(accept.encode_body(), [9, 0, 0, 0, 0, 0]);
    }

    fn me() -> Duels {
        Duels { character_id: Some(10), ..Default::default() }
    }
    fn ended(winner_id: u32, loser_id: u32, det: u8) -> Event {
        Event::Duel(DuelEvent::Ended { winner_id, loser_id, winner_obj_id: 1, loser_obj_id: 2, det })
    }

    #[test]
    fn the_duel_runs_from_challenge_to_result() {
        let mut d = me();
        let (changed, notices) = d.apply_with_notices(&Event::Duel(DuelEvent::Challenged { challenger_id: 20 }));
        assert!(changed);
        assert_eq!(notices, vec![DuelNotice::Challenged { challenger_id: 20 }]);
        assert_eq!(d.challenge, Some(20));
        assert!(d.answered(20));
        assert!(!d.answered(20));
        d.apply(&Event::Duel(DuelEvent::StartCountdown));
        assert!(d.countdown);
        let (_, notices) = d.apply_with_notices(&Event::Duel(DuelEvent::Started { challenger_obj_id: 5, challenged_obj_id: 6 }));
        assert_eq!(notices, vec![DuelNotice::Started]);
        assert!(!d.countdown && d.involves(5) && d.involves(6) && !d.involves(7));
        d.apply(&Event::Duel(DuelEvent::State { challenger_obj_id: 5, flag_obj_id: 77 }));
        assert_eq!(d.flag_obj_id, Some(77));
        let (_, notices) = d.apply_with_notices(&ended(10, 20, det::LOSE));
        assert_eq!(notices, vec![DuelNotice::Ended(DuelResult::Winner)]);
        assert_eq!(d, me());
    }

    #[test]
    fn results_follow_the_side_and_the_det() {
        for (winner, loser, det, expected) in [
            (10, 20, det::WIN, DuelResult::Winner),
            (20, 10, det::WIN, DuelResult::Loser),
            (20, 10, det::SURRENDER, DuelResult::Runaway),
            (10, 20, det::SURRENDER, DuelResult::Winner),
            (10, 20, det::DRAW, DuelResult::Draw),
            (20, 10, det::DRAW, DuelResult::Draw),
        ] {
            let (_, notices) = me().apply_with_notices(&ended(winner, loser, det));
            assert_eq!(notices, vec![DuelNotice::Ended(expected)], "{winner} {loser} {det}");
        }
    }

    #[test]
    fn leaving_the_world_forgets_the_duel() {
        let mut d = me();
        d.apply(&Event::Duel(DuelEvent::Started { challenger_obj_id: 5, challenged_obj_id: 6 }));
        d.apply(&Event::LeftWorld { target: 0 });
        assert_eq!(d, Duels::default());
    }

    #[test]
    fn the_temporary_team_factions_come_and_go_with_the_duel() {
        use crate::ExpeditionEvent::UnitFactionChanged;
        let mut world = crate::state::WorldState::default();
        let mut unit = crate::state::tests::unit(7);
        unit.faction_id = 101;
        world.apply(&Event::UnitSpawned(Box::new(unit)));
        let change = |old, new, temporary| Event::Expedition(UnitFactionChanged { obj_id: 7, name: vec![], old_faction_id: old, new_faction_id: new, temporary });
        world.apply(&change(101, 159, 1));
        assert_eq!(world.units.by_id[&7].faction_id, 159);
        world.apply(&change(159, 101, 0));
        assert_eq!(world.units.by_id[&7].faction_id, 101);
    }
}
