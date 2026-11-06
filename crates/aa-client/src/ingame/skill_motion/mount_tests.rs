//! Mount skills with a controller move the mate the player drives: `SCSkillFired` names the mate as the caster, the player's mover
//! carries it (`mount::drive_mount`) and the reports are the mate's `CSMoveUnit`. Over the real `main_world` with the skill rows of
//! the game database; skipped without them.

use super::*;
use crate::ingame::anim::LocalJump;
use crate::ingame::camera::OrbitCamera;
use crate::ingame::dev::DevOptions;
use crate::ingame::mount::{Driven, LocalSeat, drive_mount};
use crate::ingame::player::{self, PlayerInput, SendState};
use crate::ingame::swim::ActorParams;
use crate::ingame::units::Views;
use crate::settings::Actions;
use aa_net::buff::{BuffCaster, caster_kind};
use aa_net::skill::SkillFired;
use aa_world::WaterQuery;
use std::time::Duration;

const ME: u32 = 1;
const MATE: u32 = 9;
/// A flat, dry spot near the spawn (`player::feel_capture_tests`); the leaps and dashes go east from it.
const SPOT: (f32, f32) = (15480.0, 15282.0);
const EAST: f32 = -std::f32::consts::FRAC_PI_2;
const DT: f32 = 1.0 / 60.0;
/// Elegant Leap (controller 7175: 50 degrees, 25 m/s, 800 ms, 25 m along the facing) and Dash (6585: 3000 ms).
const ELEGANT_LEAP: u32 = 17399;
const DASH: u32 = 17152;

struct Ride {
    app: App,
    player: Entity,
    mate: Entity,
}

impl Ride {
    /// The player on its horse at [`SPOT`] facing east, on the driver's seat or a passenger's. `None` without the game files.
    fn new(drives: bool) -> Option<Ride> {
        let book = Book::open()?;
        let terrain = TerrainQuery::new("main_world");
        let at = Vec3::new(SPOT.0, SPOT.1, terrain.height_at(SPOT.0, SPOT.1)?);
        let seat = LocalSeat { mount: MATE, point: if drives { 1 } else { 2 }, drives, cart: false, ship: false, turret: false, speed: MountSpeed::default(), lift: 1.2 };
        let mut app = App::new();
        app.init_resource::<Time>()
            .init_resource::<Actions>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<PlayerInput>()
            .init_resource::<SendState>()
            .init_resource::<LocalJump>()
            .init_resource::<DevOptions>()
            .init_resource::<ClipLibrary>()
            .init_resource::<CameraLag>()
            .init_resource::<Tracker>()
            .init_resource::<Inbox>()
            .insert_resource(Units::default())
            .insert_resource(book)
            .insert_resource(LocalMount(Some(seat)))
            .insert_resource(WaterQuery::new("main_world"))
            .insert_resource(WorldCollision::new("main_world", terrain.clone()))
            .insert_resource(terrain)
            .add_message::<NetCommand>()
            .add_systems(Update, (start_moves, player::control_player, drive_local, drive_mount).chain());
        app.world_mut().spawn(OrbitCamera::default());
        let mover = Mover { pos: at, facing: EAST, vz: 0.0, grounded: true, velocity: Vec3::ZERO, swim: None, wading: 0.0 };
        let player = app.world_mut().spawn((LocalPlayer { obj_id: ME }, mover, placement(at, EAST, 1.0), Motion::default(), ActorParams::default())).id();
        let mut horse = RemoteUnit::fixture(MATE, at);
        horse.facing = EAST;
        let mate = app.world_mut().spawn((horse, placement(at, EAST, 1.0), Motion::default(), Driven)).id();
        let mut views = Views::default();
        views.track_unit_entity(MATE, mate);
        app.insert_resource(views);
        app.world_mut().resource_mut::<SendState>().settle_for_test();
        let mut ride = Ride { app, player, mate };
        // the standing packets of the ride settle before the skill
        for _ in 0..30 {
            ride.tick();
        }
        Some(ride)
    }

    /// The server fires the mate's skill (caster kind `Mount`, the mate itself as the target).
    fn fire(&mut self, skill: u32) {
        let fired = Event::SkillFired(SkillFired {
            skill_id: skill,
            tl: 9,
            caster: BuffCaster { kind: caster_kind::MOUNT, obj_id: MATE },
            target: aa_net::skill::SkillTarget::unit(MATE),
            raw_caster: aa_net::SkillCasterData::Mount { obj_id: MATE, mount_skill_id: 0 },
            raw_target: aa_net::SkillTargetData::Unit(MATE),
            delay_ms: 0,
            channeling_ms: 0,
            fire_anim_id: 0,
            result: Default::default(),
            raw_final_flag: 0,
        });
        let now = self.app.world().resource::<Time>().elapsed_secs_f64();
        observe(&mut self.app.world_mut().resource_mut::<Inbox>(), &fired, now);
    }

    /// One 60 Hz frame: the movement packets it sent, with the object id they name.
    fn tick(&mut self) -> Vec<(u32, MoveCommand)> {
        self.app.world_mut().resource_mut::<Time>().advance_by(Duration::from_secs_f32(DT));
        self.app.update();
        self.app
            .world_mut()
            .resource_mut::<Messages<NetCommand>>()
            .drain()
            .filter_map(|NetCommand(command)| match command {
                Command::Move(movement) => Some((ME, movement)),
                Command::MoveUnit { obj_id, movement } => Some((obj_id, movement)),
                _ => None,
            })
            .collect()
    }

    /// Frames until the skill move is over (at most `secs`): the packets with the second they went out, and the highest the mover
    /// was over the ground.
    fn run(&mut self, secs: f32) -> (Vec<(f32, u32, MoveCommand)>, f32) {
        let mut sent = Vec::new();
        let mut top: f32 = 0.0;
        let mut t = 0.0;
        while t < secs {
            t += DT;
            sent.extend(self.tick().into_iter().map(|(obj, m)| (t, obj, m)));
            let pos = self.mover().pos;
            let ground = self.app.world().resource::<TerrainQuery>().height_at(pos.x, pos.y).unwrap_or(pos.z);
            top = top.max(pos.z - ground);
            if self.app.world().get::<SkillMove>(self.player).is_none() {
                break;
            }
        }
        (sent, top)
    }

    fn mover(&self) -> &Mover {
        self.app.world().get::<Mover>(self.player).unwrap()
    }
}

fn east_of(from: Vec3, to: Vec3) -> f32 {
    to.x - from.x
}

#[test]
fn a_mount_leap_moves_the_driven_mate_and_reports_move_unit() {
    let Some(mut ride) = Ride::new(true) else { return };
    let from = ride.mover().pos;
    ride.fire(ELEGANT_LEAP);
    let first = ride.tick();
    let movement = ride.app.world().get::<SkillMove>(ride.player).expect("the leap runs on the player's mover");
    assert_eq!(movement.unit, Some(MATE));
    assert!(movement.controller_activity());
    assert!(ride.app.world().get::<SkillMove>(ride.mate).is_none(), "drive_mount places the mate, not a move of its own");
    assert!(ride.app.world().resource::<PlayerInput>().moving, "the keys are locked while the controller runs");
    let (rest, top) = ride.run(2.0);
    let sent: Vec<_> = first.into_iter().map(|(obj, m)| (0.0, obj, m)).chain(rest).collect();
    assert!(ride.app.world().get::<SkillMove>(ride.player).is_none(), "the leap is over");
    let to = ride.mover().pos;
    // 25 m east in 0.8 s along a high arc
    assert!((east_of(from, to) - 25.0).abs() < 0.5 && (to.y - from.y).abs() < 0.1, "{from} -> {to}");
    assert!(top > 3.0, "the leap goes up: {top:.2} m");
    let ground = ride.app.world().resource::<TerrainQuery>().height_at(to.x, to.y).unwrap();
    assert!((to.z - ground).abs() < 0.3, "it lands: {} vs {ground}", to.z);
    // the mate stands where the mover is
    let horse = ride.app.world().get::<RemoteUnit>(ride.mate).unwrap();
    assert_eq!((horse.pos, horse.facing), (to, ride.mover().facing));
    // every report is the mate's: every 0.1 s, jump then fall, standing at the end
    assert!(sent.iter().all(|(_, obj, _)| *obj == MATE), "only CSMoveUnit for the mate: {:?}", sent.iter().map(|s| s.1).collect::<Vec<_>>());
    let leap: Vec<_> = sent.iter().filter(|(t, ..)| *t <= 0.8 + DT).collect();
    assert!((8..=11).contains(&leap.len()), "{} reports", leap.len());
    for pair in leap.windows(2) {
        assert!(pair[1].0 - pair[0].0 <= 0.1 + 1.5 * DT, "a gap of {:.3} s", pair[1].0 - pair[0].0);
    }
    let states: Vec<MoveState> = leap.iter().map(|(_, _, m)| m.state).collect();
    let rise = states.iter().position(|s| *s == MoveState::Jump).expect("jump reports");
    let fall = states.iter().position(|s| *s == MoveState::Fall).expect("fall reports");
    assert!(rise < fall, "{states:?}");
    let (_, _, last) = leap.last().unwrap();
    assert!(!last.moving && last.state == MoveState::Ground && last.velocity == [0; 3], "{last:?}");
    assert!((last.pos.x - to.x).abs() < 1e-3 && (last.pos.y - to.y).abs() < 1e-3);
}

#[test]
fn a_mount_dash_runs_at_the_mount_speed_on_the_ground() {
    let Some(mut ride) = Ride::new(true) else { return };
    let from = ride.mover().pos;
    ride.fire(DASH);
    let (sent, top) = ride.run(4.0);
    let to = ride.mover().pos;
    // a horse runs 9 m/s: 27 m in 3 s, along the ground
    assert!((east_of(from, to) - 27.0).abs() < 0.5, "{from} -> {to}");
    assert!(top < 0.3, "a dash keeps to the ground: {top:.2} m over it");
    assert!(sent.iter().all(|(_, obj, _)| *obj == MATE));
    assert!((29..=33).contains(&sent.len()), "{} reports in 3 s", sent.len());
    let (_, _, last) = sent.last().unwrap();
    assert!(!last.moving && last.velocity == [0; 3]);
}

#[test]
fn a_passenger_does_not_move_for_the_mates_skill() {
    let Some(mut ride) = Ride::new(false) else { return };
    let from = ride.mover().pos;
    ride.fire(ELEGANT_LEAP);
    let sent = ride.tick();
    assert!(ride.app.world().get::<SkillMove>(ride.player).is_none());
    assert!(ride.app.world().get::<SkillMove>(ride.mate).is_none(), "the mate's owner moves it; this client follows the reports");
    assert!(sent.is_empty(), "{sent:?}");
    assert_eq!(ride.mover().pos, from);
}

#[test]
fn leaving_the_mate_ends_its_move_without_a_report() {
    let Some(mut ride) = Ride::new(true) else { return };
    ride.fire(DASH);
    ride.tick();
    assert!(ride.app.world().get::<SkillMove>(ride.player).is_some());
    ride.app.insert_resource(LocalMount(None));
    let sent = ride.tick();
    assert!(ride.app.world().get::<SkillMove>(ride.player).is_none());
    assert!(!sent.iter().any(|(obj, _)| *obj == MATE), "{sent:?}");
}
