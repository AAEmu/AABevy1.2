//! A remote rider nobody sees has its animator parked by `units::cull_units` (`units::Asleep`). The server sends no
//! movement of an attached unit, so it must still be seated and kept on its mount: otherwise it stays where it got on or
//! where it left the view, is judged out of view there and never wakes, and its mount is drawn alone (live check D1,
//! 2026-11-07: `data/local/transport/live/mount/i-*`, `f-*`).

use super::tests::{riding, stepped_rider};
use super::*;
use crate::ingame::units::{Asleep, RemoteUnit, Views};

const MOUNT: u32 = 5;
const RIDER: u32 = 7;

fn playing(animator: &Animator) -> &str {
    animator.current.as_ref().map_or("-", |p| p.clip.clip.name.as_str())
}

/// An app with a mount standing at `at` (game coordinates), with no animator: its seat bone falls back to its root.
fn app_with_mount(at: Vec3) -> (App, Entity) {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins).insert_resource(Units::default());
    let mount = app.world_mut().spawn((RemoteUnit::fixture(MOUNT, at), placement(at, 0.0, 1.0), Motion::default())).id();
    let mut views = Views::default();
    views.track_unit_entity(MOUNT, mount);
    app.insert_resource(views);
    (app, mount)
}

fn park(app: &mut App, entity: Entity) {
    let animator = app.world_mut().entity_mut(entity).take::<Animator>().expect("awake");
    app.world_mut().entity_mut(entity).insert(Asleep::parked(animator));
}

fn wake(app: &mut App, entity: Entity) {
    let parked = app.world_mut().entity_mut(entity).take::<Asleep>().expect("asleep");
    app.world_mut().entity_mut(entity).insert(parked.into_animator());
}

/// Run I: the rider is seen seated, the camera looks away and the horse rides 18 m on. The sleeping rider is put on the
/// seat every frame (its transform and its unit position, which the culling and the `units` distance read), and when it is
/// seen again it starts in the gait of the mount now, not in an idle-to-run for a start nobody saw.
#[test]
fn a_sleeping_rider_stays_on_its_seat_and_wakes_in_the_mount_s_gait() {
    let start = Vec3::new(15579.5, 15383.6, 126.5);
    let (mut app, mount) = app_with_mount(start);
    app.add_systems(Update, pose_riders);
    let (rider, animator) = stepped_rider(riding());
    let entity = app.world_mut().spawn((RemoteUnit::fixture(RIDER, start), Transform::default(), rider, animator)).id();
    app.update();
    assert_eq!(playing(app.world().get::<Animator>(entity).unwrap()), "idle");
    park(&mut app, entity);
    let moved = Vec3::new(15561.5, 15383.6, 126.4);
    *app.world_mut().get_mut::<Transform>(mount).unwrap() = placement(moved, 0.0, 1.0);
    app.world_mut().get_mut::<Motion>(mount).unwrap().speed = 9.0;
    app.update();
    let shown = app.world().get::<Transform>(entity).unwrap().translation;
    assert!(shown.distance(cry_to_bevy(moved)) < 1e-3, "the sleeping rider stayed behind at {shown:?}");
    assert!(app.world().get::<RemoteUnit>(entity).unwrap().pos.distance(moved) < 1e-3, "its unit position follows the seat");
    assert!(app.world().get::<Rider>(entity).unwrap().slept);
    assert_eq!(playing(app.world().get::<Asleep>(entity).unwrap().animator()), "idle", "its clips wait while it sleeps");
    wake(&mut app, entity);
    app.update();
    assert_eq!(
        playing(app.world().get::<Animator>(entity).unwrap()),
        "run",
        "the rider wakes in the running mount's gait"
    );
    assert!(!app.world().get::<Rider>(entity).unwrap().slept);
}

/// A rider that gets off while nobody sees it walks at once: a parked animator would hold its get-off (and keep it out of
/// `units::move_units`) until it is seen. An awake rider still plays the get-off clips.
#[test]
fn a_sleeping_rider_that_gets_off_walks_at_once() {
    let at = Vec3::new(100.0, 200.0, 10.0);
    let (mut app, _) = app_with_mount(at);
    app.add_systems(Update, step_dismounts);
    let mut riders = Vec::new();
    for sleeping in [true, false] {
        let (mut rider, animator) = stepped_rider(riding());
        assert!(rider.anim.begin_dismount());
        let entity = app.world_mut().spawn((RemoteUnit::fixture(RIDER, at), Transform::default(), rider)).id();
        if sleeping {
            app.world_mut().entity_mut(entity).insert(Asleep::parked(animator));
        } else {
            app.world_mut().entity_mut(entity).insert(animator);
        }
        riders.push((entity, sleeping));
    }
    app.update();
    for (entity, sleeping) in riders {
        assert_eq!(app.world().get::<Rider>(entity).is_none(), sleeping, "sleeping {sleeping}");
        if !sleeping {
            assert_eq!(playing(app.world().get::<Animator>(entity).unwrap()), "get_off_start");
        }
    }
}

/// Run F: the rider gets on behind the camera, so its animator is parked when the attach arrives. It is seated all the
/// same (from the parked animator's rig), in the riding loops (the get-on nobody saw is over by the time it is seen),
/// and placed on the mount; an awake rider seen on foot still plays the get-on (needs `AA_PAK` and `AA_DB`).
#[test]
fn a_rider_that_gets_on_out_of_view_is_seated_without_the_get_on() {
    let (Some(db), Ok(game)) = (Db::open(), aa_anim::GameDb::open()) else {
        eprintln!("no database: skipped");
        return;
    };
    if !aa_cry::pak_available() {
        eprintln!("no pak: skipped");
        return;
    }
    let assets = aa_anim::CharacterDesc::default_for(&game, aa_anim::Race::Nuian, aa_anim::Gender::Male)
        .and_then(|d| d.resolve(&game))
        .expect("a Nuian man");
    let rig = aa_anim::pose::shared_rig(&assets.skeleton).expect("his rig");
    let assets = std::sync::Arc::new(assets);
    let horse_at = Vec3::new(15579.5, 15400.0, 126.5);
    let (mut app, _) = app_with_mount(horse_at);
    app.add_systems(Update, (start_stop_riding, pose_riders).chain()).insert_resource(db).init_resource::<ClipLibrary>();
    let mut horse = crate::ingame::seating::fixture_unit(MOUNT, 43, None);
    horse.model_id = 139;
    let mut units = Units::default();
    units.by_id.insert(MOUNT, horse);
    let mut riders = Vec::new();
    for (obj_id, sleeping) in [(RIDER, true), (RIDER + 1, false)] {
        let mut unit = crate::ingame::seating::fixture_unit(obj_id, 42, None);
        unit.pos = aa_net::Pos::new(15579.5, 15350.0, 131.0);
        units.by_id.insert(obj_id, unit.clone());
        let character = aa_anim::Character { assets: assets.clone(), model_root: Entity::PLACEHOLDER, parts: Vec::new() };
        let entity = app.world_mut().spawn((RemoteUnit::announced(&unit), Transform::default(), character)).id();
        let animator = Animator::new(rig.clone(), Vec::new());
        if sleeping {
            app.world_mut().entity_mut(entity).insert(Asleep::parked(animator));
        } else {
            app.world_mut().entity_mut(entity).insert(animator);
        }
        app.world_mut().resource_mut::<Views>().track_unit_entity(obj_id, entity);
        riders.push((obj_id, entity, sleeping));
    }
    app.insert_resource(units);
    // both are seen on foot, then the server attaches them
    app.update();
    for (obj_id, _, _) in &riders {
        app.world_mut().resource_mut::<Units>().by_id.get_mut(obj_id).unwrap().attached_to =
            Some(Attachment { obj_id: MOUNT, point: 1, reason: 2 });
    }
    app.update();
    app.update();
    for (_, entity, sleeping) in riders {
        let rider = app.world().get::<Rider>(entity).unwrap_or_else(|| panic!("sleeping {sleeping}: not seated"));
        let phase = if sleeping { RidePhase::Riding } else { RidePhase::Mounting };
        assert_eq!(rider.anim.phase, phase, "sleeping {sleeping}");
        let shown = app.world().get::<Transform>(entity).unwrap().translation;
        assert!(shown.distance(cry_to_bevy(horse_at)) < 1e-3, "sleeping {sleeping}: drawn at {shown:?}, not on the horse");
    }
}
