//! Quest cinemas: a quest step with an `ObjCinema` act (`quest_act_obj_cinemas`, 12 steps of the 1.2
//! story, `cinemas.name` `id_300_01` ...) is done by watching its cinema. The server never starts one:
//! the client plays it when the step begins and reports `CSStartedCinema` / `CSCompletedCinema`; the
//! server counts the act on the second (`systems/quests/acts.rs`, `ObjCinema`).
//!
//! Playback of these sequences and their cinematic actors is still pending. Receiving a quest step
//! does not mean the player watched or skipped its cinema: no completion packet may be sent until
//! playback ends or the player explicitly skips it.
use std::collections::{HashMap, HashSet};

use aa_net::quest::{Quest, status};
use aa_net::{Event, NetEvent};
use bevy::prelude::*;

pub(super) fn plugin(app: &mut App) {
    app.add_systems(Update, track.run_if(in_state(crate::Screen::InGame)));
}

/// `quest_components.id` -> `cinemas.name` of the steps with a cinema act.
fn load() -> HashMap<u32, String> {
    let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
    let query = || -> rusqlite::Result<HashMap<u32, String>> {
        let conn = rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
        let mut q = conn.prepare(
            "SELECT a.quest_component_id, c.name FROM quest_acts a JOIN quest_act_obj_cinemas o ON o.id = a.act_detail_id \
             JOIN cinemas c ON c.id = o.cinema_id WHERE a.act_detail_type = 'QuestActObjCinema'",
        )?;
        q.query_map([], |r| Ok((r.get::<_, i64>(0)? as u32, r.get::<_, String>(1)?)))?.collect()
    };
    query().unwrap_or_else(|e| {
        warn!("hud: quest cinemas from {path}: {e}");
        HashMap::new()
    })
}

#[derive(Default)]
struct Cinemas {
    steps: Option<HashMap<u32, String>>,
    /// (quest instance, step) already reported.
    played: HashSet<(i64, u32)>,
}

impl Cinemas {
    /// The cinema to report for a quest, once per step.
    fn due(&mut self, q: &Quest) -> Option<String> {
        if q.status != status::PROGRESS {
            return None;
        }
        let name = self.steps.get_or_insert_with(load).get(&q.component_id)?.clone();
        self.played.insert((q.id, q.component_id)).then_some(name)
    }
}

fn track(mut events: MessageReader<NetEvent>, mut cinemas: Local<Cinemas>) {
    for NetEvent(event) in events.read() {
        let quests: &[Quest] = match event {
            Event::QuestStarted(q) | Event::QuestUpdated(q) => std::slice::from_ref(q),
            Event::QuestsLoaded(list) => list,
            Event::CharacterSelected(_) | Event::Disconnected { .. } => {
                cinemas.played.clear();
                continue;
            }
            _ => continue,
        };
        for q in quests {
            if let Some(name) = cinemas.due(q) {
                warn!("hud: quest {} step {}: cinema {name} requires playback (not reported watched)", q.template_id, q.component_id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cinema_step_is_reported_once() {
        let mut c = Cinemas { steps: Some(HashMap::from([(26187, "id_300_03".to_string())])), ..Cinemas::default() };
        let q = Quest { id: 9, template_id: 3000, status: status::PROGRESS, component_id: 26187, ..Quest::default() };
        assert_eq!(c.due(&q).as_deref(), Some("id_300_03"));
        assert_eq!(c.due(&q), None);
        // another step, a finished quest: nothing
        assert_eq!(c.due(&Quest { component_id: 1, ..q.clone() }), None);
        assert_eq!(c.due(&Quest { id: 10, status: status::READY, ..q }), None);
    }

    #[test]
    fn receiving_a_cinema_step_does_not_complete_an_unwatched_cinema() {
        let mut app = App::new();
        app.add_message::<NetEvent>().add_message::<aa_net::NetCommand>().add_systems(Update, track);
        app.world_mut().write_message(NetEvent(Event::QuestStarted(Quest {
            id: 9, template_id: 3000, status: status::PROGRESS, component_id: 26187, ..Quest::default()
        })));
        app.update();
        assert!(app.world().resource::<Messages<aa_net::NetCommand>>().is_empty());
    }
}
