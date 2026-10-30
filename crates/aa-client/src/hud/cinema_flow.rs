//! The packet flow of a quest component's cinema (`quest_components.cinema_id`).
//!
//! `CSRequestPermissionToPlayCinemaForDirectingMode` (0x0d0) is sent when the directing dialog
//! opens for a component that has a cinema (`dialog.rs`), not when the player confirms. Once the
//! player confirms, `CSStartedCinema` (0x0cf) is sent, the sequence plays, then
//! `CSCompletedCinema` (0x0ce) and the waiting quest request (`CSCompleteQuestContext` /
//! `CSStartQuestContext`). Escape during the movie is ignored (capture row C1-run3: the movie ends
//! normally after 22.22 s and the hand-in follows); the close button was not drivable.

use aa_net::interaction::Interact;

/// A cinema being played for a quest component, and the request that waits for its end.
#[derive(Clone, Debug, PartialEq)]
pub struct Pending {
    pub quest_id: u32,
    pub component_id: u32,
    pub name: String,
    pub duration: f32,
    pub elapsed: f32,
    pub then: Option<Interact>,
}

/// Starting a cinema: the started report.
#[derive(Clone, Debug, PartialEq)]
pub struct Start {
    pub pending: Pending,
    pub packets: Vec<Interact>,
}

/// The end of a cinema: the completed report, then the quest request.
#[derive(Clone, Debug, PartialEq)]
pub struct End {
    pub packets: Vec<Interact>,
}

/// Starts a cinema of `duration` seconds. `None` when the sequence is missing or invalid; the
/// caller then sends its quest request at once, as the 1.2 client does after a cut-scene error.
pub fn start(
    quest_id: u32,
    component_id: u32,
    name: &str,
    duration: f32,
    then: Option<Interact>,
) -> Option<Start> {
    if !duration.is_finite() || duration <= 0.0 {
        return None;
    }
    Some(Start {
        pending: Pending {
            quest_id,
            component_id,
            name: name.to_string(),
            duration,
            elapsed: 0.0,
            then,
        },
        packets: vec![Interact::Cinema { completed: false }],
    })
}

/// Advances a playback; the end brings 0x0ce and the waiting quest request.
pub fn advance(pending: &mut Pending, dt: f32) -> Option<End> {
    if !dt.is_finite() || dt < 0.0 {
        return None;
    }
    pending.elapsed += dt;
    (pending.elapsed >= pending.duration).then(|| {
        let mut packets = vec![Interact::Cinema { completed: true }];
        packets.extend(pending.then.clone());
        End { packets }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete() -> Interact {
        Interact::QuestComplete {
            quest_id: 1065,
            npc_obj_id: 3443,
            doodad_obj_id: 0,
            selected: 0,
        }
    }

    #[test]
    fn a_quest_cinema_reports_start_and_end_then_hands_the_quest_in() {
        let mut start = start(1065, 5336, "fly_east", 21.0, Some(complete())).unwrap();
        assert_eq!(start.packets, [Interact::Cinema { completed: false }]);
        assert!(advance(&mut start.pending, 20.9).is_none());
        assert_eq!(
            advance(&mut start.pending, 0.2).unwrap().packets,
            [Interact::Cinema { completed: true }, complete()]
        );
    }

    #[test]
    fn a_progress_cinema_ends_without_a_quest_request() {
        let mut start = start(1061, 5320, "zq_00_49", 12.0, None).unwrap();
        assert!(advance(&mut start.pending, 11.9).is_none());
        assert_eq!(
            advance(&mut start.pending, 0.2).unwrap().packets,
            [Interact::Cinema { completed: true }]
        );
    }

    #[test]
    fn a_missing_sequence_never_starts_or_hangs() {
        assert!(start(1065, 5336, "missing", 0.0, Some(complete())).is_none());
        assert!(start(1065, 5336, "missing", f32::NAN, Some(complete())).is_none());
        let mut start = start(1065, 5336, "fly_east", 21.0, Some(complete())).unwrap();
        assert!(advance(&mut start.pending, f32::NAN).is_none());
        assert_eq!(start.pending.elapsed, 0.0);
    }
}
