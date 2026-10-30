//! Self-check of the quest report cinema flow (quest 1065, cinema 17 `fly_east`): 0x0d0, 0x0cf,
//! the movie's time, 0x0ce, then `CSCompleteQuestContext`. Exits non-zero when the order or the
//! playback changes. `cargo run -p aa-client --example cinema_report`; with `AA_PAK` set it also
//! checks the real `fly_east` sequence is 21 s.

#[path = "../src/hud/cinema_flow.rs"]
mod cinema_flow;

use aa_net::interaction::Interact;
use aa_net::wire::Writer;

fn body(packet: &Interact) -> Vec<u8> {
    let mut w = Writer::new();
    packet.write(&mut w);
    w.finish()
}

fn real_duration() -> Option<f32> {
    if !aa_world::pak_available() {
        return None;
    }
    let xml = aa_pak::Pak::global()
        .read_string("worlds/main_world/cinemas/fly_east.xml")
        .ok()?;
    let doc = roxmltree::Document::parse(&xml).ok()?;
    doc.root_element().attribute("EndTime")?.parse().ok()
}

fn main() {
    let complete = Interact::QuestComplete {
        quest_id: 1065,
        npc_obj_id: 3443,
        doodad_obj_id: 0,
        selected: 0,
    };

    let permission = Interact::CinemaPermission {
        quest_id: 1065,
        npc_obj_id: 3443,
        doodad_obj_id: 0,
    };
    assert_eq!(permission.opcode(), 0x0d0);
    assert_eq!(
        body(&permission),
        [0x29, 0x04, 0, 0, 0x73, 0x0D, 0, 0, 0, 0]
    );
    assert_eq!(Interact::Cinema { completed: false }.opcode(), 0x0cf);
    assert_eq!(Interact::Cinema { completed: true }.opcode(), 0x0ce);
    assert!(body(&Interact::Cinema { completed: false }).is_empty());

    let duration = real_duration().unwrap_or(21.0);
    if aa_world::pak_available() {
        assert!(
            (duration - 21.0).abs() < 0.01,
            "fly_east should be 21 s, got {duration}"
        );
    }
    let mut started =
        cinema_flow::start(1065, 5336, "fly_east", duration, 3443, 0, complete.clone())
            .expect("the report cinema must start");
    assert_eq!(
        started.packets,
        [permission, Interact::Cinema { completed: false }],
        "0x0d0 must precede 0x0cf"
    );
    assert!(
        cinema_flow::advance(&mut started.pending, duration - 0.1).is_none(),
        "the quest must wait for the movie"
    );
    assert_eq!(
        cinema_flow::advance(&mut started.pending, 0.2)
            .expect("the movie must end")
            .packets,
        [Interact::Cinema { completed: true }, complete.clone()],
        "0x0ce must precede CSCompleteQuestContext"
    );

    let started = cinema_flow::start(1065, 5336, "fly_east", duration, 3443, 0, complete).unwrap();
    assert_eq!(
        cinema_flow::skip(&started.pending).packets,
        [Interact::Cinema { completed: true }],
        "Escape ends the movie without handing the quest in"
    );
    assert!(
        cinema_flow::start(
            1065,
            5336,
            "missing",
            0.0,
            3443,
            0,
            Interact::QuestComplete {
                quest_id: 1065,
                npc_obj_id: 3443,
                doodad_obj_id: 0,
                selected: 0
            }
        )
        .is_none()
    );
    println!(
        "CINEMA REPORT: PASS fly_east {duration:.1} s; 0x0d0 -> 0x0cf -> (movie) -> 0x0ce -> CSCompleteQuestContext"
    );
}
