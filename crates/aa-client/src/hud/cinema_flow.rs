//! The packet flow of a quest component's cinema (`quest_components.cinema_id`).
//!
//! The 1.2 client sends `CSRequestPermissionToPlayCinemaForDirectingMode` (0x0d0) and
//! `CSStartedCinema` (0x0cf), plays the sequence, then sends `CSCompletedCinema` (0x0ce) and the
//! quest request (`CSCompleteQuestContext` / `CSStartQuestContext`). A skipped movie still ends
//! with 0x0ce but sends no quest request: the directing window cancels the talk instead.

use aa_net::interaction::Interact;

/// A cinema being played for a quest component, and the request that waits for its end.
#[derive(Clone, Debug, PartialEq)]
pub struct Pending {
    pub quest_id: u32,
    pub component_id: u32,
    pub name: String,
    pub duration: f32,
    pub elapsed: f32,
    pub npc_obj_id: u32,
    pub doodad_obj_id: u32,
    pub then: Interact,
}

/// Starting a cinema: the permission request and the started report.
#[derive(Clone, Debug, PartialEq)]
pub struct Start {
    pub pending: Pending,
    pub packets: Vec<Interact>,
}

/// The end of a cinema: the completed report, then (unless skipped) the quest request.
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
    npc_obj_id: u32,
    doodad_obj_id: u32,
    then: Interact,
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
            npc_obj_id,
            doodad_obj_id,
            then,
        },
        packets: vec![
            Interact::CinemaPermission {
                quest_id,
                npc_obj_id,
                doodad_obj_id,
            },
            Interact::Cinema { completed: false },
        ],
    })
}

/// Advances a playback; the end brings 0x0ce and the waiting quest request.
pub fn advance(pending: &mut Pending, dt: f32) -> Option<End> {
    if !dt.is_finite() || dt < 0.0 {
        return None;
    }
    pending.elapsed += dt;
    (pending.elapsed >= pending.duration).then(|| End {
        packets: vec![Interact::Cinema { completed: true }, pending.then.clone()],
    })
}

/// Esc / the cinema window's close button: stop the movie but do not hand the quest in.
pub fn skip(_pending: &Pending) -> End {
    End {
        packets: vec![Interact::Cinema { completed: true }],
    }
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
    fn a_quest_cinema_requests_permission_then_reports_start_and_end() {
        let mut start = start(1065, 5336, "fly_east", 21.0, 3443, 0, complete()).unwrap();
        assert_eq!(
            start.packets,
            [
                Interact::CinemaPermission {
                    quest_id: 1065,
                    npc_obj_id: 3443,
                    doodad_obj_id: 0
                },
                Interact::Cinema { completed: false },
            ]
        );
        assert!(advance(&mut start.pending, 20.9).is_none());
        assert_eq!(
            advance(&mut start.pending, 0.2).unwrap().packets,
            [Interact::Cinema { completed: true }, complete()]
        );
    }

    #[test]
    fn skipping_reports_the_movie_ended_without_handing_the_quest_in() {
        let start = start(1065, 5336, "fly_east", 21.0, 3443, 0, complete()).unwrap();
        assert_eq!(
            skip(&start.pending).packets,
            [Interact::Cinema { completed: true }]
        );
    }

    #[test]
    fn a_missing_sequence_never_starts_or_hangs() {
        assert!(start(1065, 5336, "missing", 0.0, 3443, 0, complete()).is_none());
        assert!(start(1065, 5336, "missing", f32::NAN, 3443, 0, complete()).is_none());
        let mut start = start(1065, 5336, "fly_east", 21.0, 3443, 0, complete()).unwrap();
        assert!(advance(&mut start.pending, f32::NAN).is_none());
        assert_eq!(start.pending.elapsed, 0.0);
    }
}
