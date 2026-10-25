//! Received affiliation changes through the real plate systems, with no socket or renderer.
use super::*;
use aa_net::{
    Event, NetEvent,
    family::{FamilyEvent, FamilyTitle},
};
use aa_ui::UiRuntime;

fn fixture() -> App {
    let (mut app, bridge) = super::marker_tests::fixture();
    let mut rt = UiRuntime::new("en_us", 1280.0, 720.0).unwrap();
    super::super::api::install(&mut rt, &bridge).unwrap();
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("aa-plate-option-{}-{serial}.json", std::process::id()));
    crate::settings::reopen_fixture(&rt, path).unwrap();
    crate::settings::install_options(&rt.lua).unwrap();
    rt.lua.load("X2Option:SetItemFloatValue(52,1)").exec().unwrap();
    rt.set_ui_texts(std::sync::Arc::new(
        [
            ((92, "community_desc".into()), "$1's $2".into()),
            ((92, "owner_title".into()), "$1 Family $2".into()),
            ((92, "family_leader".into()), "Steward".into()),
        ]
        .into_iter()
        .collect(),
    ))
    .unwrap();
    {
        let mut units = app.world_mut().resource_mut::<Units>();
        let unit = units.by_id.get_mut(&601).unwrap();
        unit.kind = UnitKind::Character { char_id: 501 };
        unit.name = "Remote".into();
    }
    app.add_message::<NetEvent>()
        .insert_non_send(rt)
        .add_systems(Update, track_family_titles.before(update_plates));
    let spawn = app.world().resource::<Player>().spawn.clone().unwrap();
    app.world_mut().write_message(NetEvent(Event::PlayerSpawn(Box::new(spawn))));
    let units: Vec<_> = app.world().resource::<Units>().iter().cloned().collect();
    for unit in units {
        app.world_mut().write_message(NetEvent(Event::UnitSpawned(Box::new(unit))));
    }
    app
}

fn receive(app: &mut App, event: Event) {
    app.world_mut().resource_mut::<Units>().apply(&event);
    app.world_mut().resource_mut::<Player>().apply(&event);
    app.world_mut().write_message(NetEvent(event));
}

fn title(id: u32, role: u8, owner: &[u8], title: &[u8]) -> Event {
    Event::Family(FamilyEvent::Title(FamilyTitle {
        unit_id: id,
        role,
        owner: owner.to_vec(),
        title: title.to_vec(),
    }))
}

#[test]
fn family_receipts_require_a_loaded_object_and_empty_owner_clears() {
    let mut app = fixture();
    receive(&mut app, title(999, 0, b"Unknown", b"Son"));
    receive(&mut app, title(601, 0, b"Ann", b"Son"));
    app.update();
    assert!(
        !app.world().resource::<PlateState>().family.contains_key(&999),
        "an unknown object cannot hold a pending title"
    );
    let (_, text) = app.world().resource::<PlateState>().parts(601).unwrap();
    assert_eq!(app.world().get::<Text2d>(text).unwrap().0, "Remote\n(Ann's Son)");
    receive(&mut app, title(601, 1, b"\0Ignored", b"Ignored"));
    app.update();
    assert!(
        !app.world().resource::<PlateState>().family.contains_key(&601),
        "empty owner explicitly clears"
    );
    assert_eq!(app.world().get::<Text2d>(text).unwrap().0, "Remote");
}

#[test]
fn family_titles_retire_on_object_replacement_removal_and_world_exit() {
    let mut app = fixture();
    receive(&mut app, title(601, 2, b"Ann", b"Aunt"));
    app.update();
    let unit = app.world().resource::<Units>().get(601).unwrap().clone();
    receive(&mut app, Event::UnitsRemoved(vec![601]));
    receive(&mut app, title(601, 0, b"Retired", b"Son"));
    receive(&mut app, Event::UnitSpawned(Box::new(unit.clone())));
    app.update();
    assert!(
        !app.world().resource::<PlateState>().family.contains_key(&601),
        "same numeric object ID starts without an old title"
    );
    receive(&mut app, title(601, 0, b"New", b"Daughter"));
    app.update();
    assert_eq!(app.world().resource::<PlateState>().family.get(&601).unwrap(), "New's Daughter");
    receive(&mut app, Event::UnitSpawned(Box::new(unit)));
    app.update();
    assert!(!app.world().resource::<PlateState>().family.contains_key(&601));
    receive(&mut app, title(601, 0, b"Ann", b"Son"));
    receive(&mut app, Event::LeftWorld { target: 0 });
    app.update();
    assert!(app.world().resource::<PlateState>().family.is_empty());
}

#[test]
fn family_role_formatting_uses_current_locale_and_c_string_boundaries() {
    let mut app = fixture();
    app.world()
        .non_send::<UiRuntime>()
        .set_ui_texts(std::sync::Arc::new(
            [
                ((92, "community_desc".into()), "$2 of $1".into()),
                ((92, "owner_title".into()), "$2 / $1".into()),
                ((92, "family_leader".into()), "Localized steward".into()),
            ]
            .into_iter()
            .collect(),
        ))
        .unwrap();
    receive(&mut app, title(601, 0, b"Ann\0Old", b"Son\0Old"));
    app.update();
    assert_eq!(app.world().resource::<PlateState>().family.get(&601).unwrap(), "Son of Ann");
    receive(&mut app, title(601, 255, b"Ann", b""));
    app.update();
    assert_eq!(
        app.world().resource::<PlateState>().family.get(&601).unwrap(),
        "Localized steward / Ann"
    );
    receive(&mut app, title(601, 0, b"Ann", b""));
    app.update();
    assert_eq!(
        app.world().resource::<PlateState>().family.get(&601).unwrap(),
        " of Ann",
        "role zero never substitutes the leader title"
    );
}

#[test]
fn original_name_tag_setter_switches_the_existing_plate_without_losing_received_family() {
    let mut app = fixture();
    receive(&mut app, title(601, 0, b"Ann", b"Son"));
    app.update();
    let (root, text) = app.world().resource::<PlateState>().parts(601).unwrap();
    assert_eq!(app.world().get::<Text2d>(text).unwrap().0, "Remote\n(Ann's Son)");
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("X2NameTag:SetNameTagFactionSelection(0)")
        .exec()
        .unwrap();
    app.update();
    assert_eq!(app.world().get::<Text2d>(text).unwrap().0, "Remote");
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("X2NameTag:SetNameTagFactionSelection(1)")
        .exec()
        .unwrap();
    app.update();
    assert_eq!(app.world().get::<Text2d>(text).unwrap().0, "Remote\n(Ann's Son)");
    assert_eq!(app.world().resource::<PlateState>().parts(601).unwrap().0, root);
}

#[test]
fn own_object_removal_blocks_late_receipts_until_the_next_spawn() {
    let mut app = fixture();
    receive(&mut app, title(100, 1, b"Own", b""));
    app.update();
    assert_eq!(app.world().resource::<PlateState>().family.get(&100).unwrap(), "Own Family Steward");
    receive(&mut app, Event::UnitsRemoved(vec![100]));
    receive(&mut app, title(100, 1, b"Old", b""));
    app.update();
    assert!(
        app.world().resource::<PlateState>().family.is_empty(),
        "Player retains a snapshot, which is not a loaded-object receipt"
    );
    let own = app.world().resource::<Player>().spawn.as_ref().unwrap().unit.clone();
    receive(&mut app, Event::OwnUnitSnapshot(Box::new(own)));
    receive(&mut app, title(100, 1, b"Snapshot", b""));
    app.update();
    assert_eq!(
        app.world().resource::<PlateState>().family.get(&100).unwrap(),
        "Snapshot Family Steward"
    );
    let spawn = app.world().resource::<Player>().spawn.clone().unwrap();
    receive(&mut app, Event::PlayerSpawn(Box::new(spawn)));
    receive(&mut app, title(100, 1, b"New", b""));
    app.update();
    assert_eq!(app.world().resource::<PlateState>().family.get(&100).unwrap(), "New Family Steward");
}
