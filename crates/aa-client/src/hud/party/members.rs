//! What the party and raid frames show of a member besides the roster: the buffs on the unit (the
//! frames' buff row reads `X2Unit:UnitBuff` of `team1`...) and whether it is fighting (the frame's red
//! combat outline reads `X2Unit:UnitCombatState`).
//!
//! The server reports buffs for every unit in view, so a member out of range has none. It reports a
//! fight only for the player (`CombatChanged`), so the others count as fighting for [`QUIET`] seconds
//! after a blow that they gave or took.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use aa_net::party::Party;
use aa_net::state::Buffs;

use super::super::data::GameData;
use super::super::panels::{BuffLists, unit_buff_lists};

/// Seconds a member stays in combat after its last blow.
const QUIET: f64 = 6.0;

/// The buffs and fights of the members, by unit object id.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Extras {
    pub buffs: HashMap<u32, BuffLists>,
    pub combat: HashSet<u32>,
}

/// When each unit last gave or took a blow, on the app's clock.
#[derive(Default)]
pub(super) struct Fights {
    last: HashMap<u32, f64>,
}

impl Fights {
    pub fn note(&mut self, caster: u32, target: u32, now: f64) {
        self.last.insert(caster, now);
        self.last.insert(target, now);
    }

    fn engaged(&self, obj_id: u32, now: f64) -> bool {
        self.last.get(&obj_id).is_some_and(|t| now - t < QUIET)
    }
}

/// The frames' extras for the members of `party` now.
pub(super) fn extras(party: &Party, buffs: Option<&Buffs>, game: Option<&GameData>, start: Instant, fights: &Fights, now: f64) -> Extras {
    let mut out = Extras::default();
    for m in party.members.iter().filter(|m| m.obj_id != 0) {
        if let Some(buffs) = buffs {
            let lists = unit_buff_lists(game, buffs, m.obj_id, start);
            if !lists.good.is_empty() || !lists.bad.is_empty() {
                out.buffs.insert(m.obj_id, lists);
            }
        }
        if fights.engaged(m.obj_id, now) {
            out.combat.insert(m.obj_id);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use aa_net::party::PartyMember;

    fn party() -> Party {
        Party {
            id: 1,
            members: vec![PartyMember { id: 11, obj_id: 101, ..default() }, PartyMember { id: 12, obj_id: 0, ..default() }],
            ..default()
        }
    }

    fn default<T: Default>() -> T {
        T::default()
    }

    #[test]
    fn a_blow_starts_a_fight_that_quiets_down() {
        let mut fights = Fights::default();
        fights.note(101, 900, 10.0);
        let at = |fights: &Fights, now| extras(&party(), None, None, Instant::now(), fights, now);
        assert!(at(&fights, 12.0).combat.contains(&101));
        assert!(!at(&fights, 17.0).combat.contains(&101));
        // a member the client has no unit for cannot be told apart
        fights.note(0, 0, 12.0);
        assert_eq!(at(&fights, 12.5).combat.len(), 1);
    }
}
