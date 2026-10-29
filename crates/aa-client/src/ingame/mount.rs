//! Riders: a unit the server attached to a mount (`SCUnitAttached`: a mate such as a horse or a lion) sits on the mount's seat
//! bone and plays the rider's clips of that kind of mount.
//!
//! The server moves a rider with its mount: it sends no movement for the rider, only for the mount. The rider's own clips
//! come from its character `.cal` (`horse_ba_relaxed_idle`, `horse_mo_normal_walk_f`, `horse_mo_normal_run_f`, and the
//! `lion_`, `wildboar_` and `pangolin_` sets; the `models.mount_pose_id` of the mount says which), the mount's from its own
//! `.cal` as for any creature. The seat is the mount's bone `bone_spine_driver` (attach point 1) or `bone_spine_passenger`.
//!
//! The local player rides the same way ([`LocalMount`]). On the driver's seat of its own mate its keys move the mount:
//! `player::control_player` steers the [`Mover`] at the mount's speed and reports it as the mate's movement (`CSMoveUnit`
//! with the mate's object id; the server ignores a riding character's own movement), and [`drive_mount`] puts the mate's
//! unit where the mover is.

use aa_anim::pose::BoundClip;
use aa_anim::{Animator, Cal, Character, ClipLibrary};
use aa_net::model::{Attachment, Equipment, ModelParams, Movement, UnitKind};
use aa_net::state::{Player, Units};
use aa_net::{Event, NetEvent, Pos};
use aa_world::coords::bevy_to_cry;
use bevy::prelude::*;

use super::anim::{Locomotion, Motion, bind_clip};
use super::camera::OrbitCamera;
use super::data::{Db, MountSpeed};
use super::player::{LocalPlayer, Mover, SendState, placement};
use super::units::{RemoteUnit, Views};

/// The clip prefix of a rider on a mount of this kind (`models.mount_pose_id`). Most mountable models (the donkeys, the elks,
/// the yatas: pose 0) and the poses without a rider set of their own take the horse's.
fn rider_prefix(pose: u32) -> &'static str {
    match pose {
        2 => "lion",
        4 => "wildboar",
        10 => "pangolin",
        _ => "horse",
    }
}

/// The bone of a mount a rider at `point` sits on (`model_attach_point_strings`: `$driver` is `bone_spine_driver`).
fn seat_bone(point: u8) -> &'static str {
    if point <= 1 { "bone_spine_driver" } else { "bone_spine_passenger" }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Gait {
    Idle,
    Walk,
    Run,
}

/// The rider's gait for the mount's ground speed (m/s): idle below 0.3, walking below 3 (or without a run clip), running.
fn gait(speed: f32, has_run: bool) -> Gait {
    if speed < 0.3 {
        Gait::Idle
    } else if speed < 3.0 || !has_run {
        Gait::Walk
    } else {
        Gait::Run
    }
}

/// A unit sitting on a mount (a remote unit, or the local player).
#[derive(Component)]
pub struct Rider {
    mount: u32,
    point: u8,
    idle: BoundClip,
    walk: Option<BoundClip>,
    run: Option<BoundClip>,
    gait: Option<Gait>,
    /// The locomotion the unit had on foot, given back when it gets off.
    on_foot: Option<Locomotion>,
}

impl Rider {
    /// Binds the rider clips of `character` for a seat on `mount`, a unit of model `mount_model`.
    fn bind(db: &Db, clips: &ClipLibrary, character: &Character, animator: &Animator, mount: Attachment, mount_model: u32) -> Option<Rider> {
        let prefix = rider_prefix(db.mount_pose(mount_model));
        let cal = Cal::shared(&character.assets.cal).ok()?;
        let bind = |name: String| bind_clip(&animator.rig, &cal, clips, &name);
        let idle = bind(format!("{prefix}_ba_relaxed_idle")).or_else(|| bind("fist_ba_relaxed_idle".into()))?;
        let run = bind(format!("{prefix}_mo_normal_run_f"));
        let walk = bind(format!("{prefix}_mo_normal_walk_f")).or_else(|| run.clone());
        Some(Rider { mount: mount.obj_id, point: mount.point, idle, walk, run, gait: None, on_foot: None })
    }

    /// Plays the clip of the mount's gait (`speed` its ground speed, `tempo` its animator's speed).
    fn step(&mut self, animator: &mut Animator, speed: f32, tempo: f32) {
        let gait = gait(speed, self.run.is_some());
        if self.gait != Some(gait) {
            let clip = match gait {
                Gait::Idle => Some(self.idle.clone()),
                Gait::Walk => self.walk.clone().or_else(|| self.run.clone()),
                Gait::Run => self.run.clone(),
            };
            if let Some(clip) = clip {
                animator.play(clip, true, 0.2);
                animator.random_idle = None;
            }
            self.gait = Some(gait);
        }
        animator.speed = if gait == Gait::Idle { 1.0 } else { tempo };
    }
}

/// Seats a rider; its on-foot locomotion is kept for later.
fn seat(world: &mut World, entity: Entity, mut rider: Rider) {
    rider.on_foot = world.entity_mut(entity).take::<Locomotion>();
    world.entity_mut(entity).insert(rider);
}

/// Lets a rider walk again.
fn unseat(world: &mut World, entity: Entity) {
    let Some(rider) = world.entity_mut(entity).take::<Rider>() else { return };
    if let Some(mut on_foot) = rider.on_foot {
        on_foot.invalidate();
        world.entity_mut(entity).insert(on_foot);
    }
    if let Some(mut motion) = world.entity_mut(entity).get_mut::<Motion>() {
        *motion = Motion::default();
    }
}

/// Where the seat bone of a mount is now (Bevy coordinates): the bone's place on the mount's last drawn pose, carried to where
/// the mount stands in this frame (`now`), so a rider does not trail a moving mount by a frame.
fn seat_world(mount: Entity, now: &Transform, animator: &Animator, point: u8, globals: &Query<&GlobalTransform>) -> Vec3 {
    let bone = animator.rig.skeleton.bones.iter().position(|b| b.name == seat_bone(point)).and_then(|i| animator.bones.get(i));
    match (globals.get(mount), bone.and_then(|b| globals.get(*b).ok())) {
        (Ok(root), Some(bone)) => now.transform_point(root.affine().inverse().transform_point3(bone.translation())),
        _ => now.translation,
    }
}

/// Seats the units the server attached to a mount and lets the ones that got off walk again.
#[allow(clippy::type_complexity)]
pub fn start_stop_riding(
    mut commands: Commands,
    net: Res<Units>,
    db: Option<Res<Db>>,
    clips: Res<ClipLibrary>,
    units: Query<(Entity, &RemoteUnit, Option<&Rider>, Option<&Character>, Option<&Animator>)>,
) {
    for (entity, unit, rider, character, animator) in &units {
        let attached = net.by_id.get(&unit.obj_id).and_then(|u| u.attached_to);
        match (attached, rider) {
            (Some(a), None) => {
                // wait for the animator (a unit far away has none) and the mount's data
                let (Some(db), Some(character), Some(animator), Some(mount)) = (db.as_deref(), character, animator, net.by_id.get(&a.obj_id)) else { continue };
                let Some(rider) = Rider::bind(db, &clips, character, animator, a, mount.model_id) else { continue };
                debug!("unit {} rides {} (model {}) at point {}", unit.obj_id, a.obj_id, mount.model_id, a.point);
                commands.queue(move |world: &mut World| seat(world, entity, rider));
            }
            (None, Some(_)) => {
                commands.queue(move |world: &mut World| {
                    unseat(world, entity);
                    // it stands where the mount was: the movement packets of the unit itself carry on from here
                    if let Some(mut unit) = world.entity_mut(entity).get_mut::<RemoteUnit>() {
                        let (pos, facing) = (unit.pos, unit.facing);
                        unit.reset_track(pos, facing);
                    }
                });
            }
            _ => {}
        }
    }
}

/// Puts the riders on their mounts' seats and plays the rider clips for the gait of the mount.
pub fn pose_riders(
    net: Res<Units>,
    views: Res<Views>,
    mut riders: Query<(&mut RemoteUnit, &mut Transform, &mut Rider, &mut Animator)>,
    mounts: Query<(&RemoteUnit, &Transform, &Animator, &Motion), Without<Rider>>,
    globals: Query<&GlobalTransform>,
) {
    for (mut unit, mut transform, mut rider, mut animator) in &mut riders {
        let mounted = views.unit_entity(rider.mount).and_then(|e| mounts.get(e).ok().map(|m| (e, m)));
        let (pos, facing, speed, tempo) = match mounted {
            Some((e, (mount, now, mount_animator, motion))) => {
                let world = seat_world(e, now, mount_animator, rider.point, &globals);
                (bevy_to_cry(world), mount.facing, motion.speed, mount_animator.speed)
            }
            None => {
                // the mount is not drawn: stay with where the server says it is
                let Some(m) = net.by_id.get(&rider.mount) else { continue };
                (Vec3::new(m.pos.x, m.pos.y, m.pos.z), m.yaw(), 0.0, 1.0)
            }
        };
        unit.pos = pos;
        unit.facing = facing;
        *transform = placement(pos, facing, unit.scale());
        rider.step(&mut animator, speed, tempo);
    }
}

/// The mount the local player sits on (`None` on foot), for the movement, the camera and the HUD.
#[derive(Resource, Default, Clone, Copy, Debug, PartialEq)]
pub struct LocalMount(pub Option<LocalSeat>);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LocalSeat {
    /// The mount's object id and the attach point.
    pub mount: u32,
    pub point: u8,
    /// On the driver's seat (point 1, which the server gives only the mate's owner): the player's keys move the mount.
    pub drives: bool,
    /// The attached unit is a slave (a cart or other land vehicle), not a mate: `cart::drive_cart` owns it.
    pub cart: bool,
    pub speed: MountSpeed,
    /// Height of the seat over the mount's feet (m): the camera's pivot rises by it.
    pub lift: f32,
}

impl LocalMount {
    /// The mount the player drives: its object id and speed.
    pub fn driving(&self) -> Option<(u32, MountSpeed)> {
        self.0
            .filter(|s| s.drives && !s.cart)
            .map(|s| (s.mount, s.speed))
    }

    /// The slave cart the player drives from attach point 1.
    pub fn cart_driver(&self) -> Option<u32> {
        self.0.filter(|s| s.drives && s.cart).map(|s| s.mount)
    }

    /// On a seat it does not steer (a passenger).
    pub fn passenger(&self) -> bool {
        self.0.is_some_and(|s| !s.drives)
    }

    /// How far the camera's pivot rises over the feet: a rider's origin is on the seat, and its head is lower over it than a
    /// standing character's over its feet by about the hips' height ([`SEATED_DROP`]).
    pub fn lift(&self) -> f32 {
        self.0.map_or(0.0, |s| (s.lift - SEATED_DROP).max(0.0))
    }
}

/// How much lower a seated rider's head is over the seat than a standing character's over its feet (m).
const SEATED_DROP: f32 = 0.85;

/// Marks the unit of the mate the local player drives: [`drive_mount`] places it, not its movement packets.
#[derive(Component)]
pub struct Driven;

/// Follows the local player's attachment (`SCUnitAttached` / `SCUnitDetached`): seats it on the mount and puts its mover where
/// the mount is; when it gets off it walks on from there (the server puts it where the mount stood too).
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn track_local_mount(
    mut commands: Commands,
    player: Res<Player>,
    net: Res<Units>,
    db: Option<Res<Db>>,
    clips: Res<ClipLibrary>,
    views: Res<Views>,
    (mut seat_state, mut send): (ResMut<LocalMount>, ResMut<SendState>),
    mut local: Query<(Entity, &mut Mover, Option<&Rider>, Option<&Character>, Option<&Animator>), With<LocalPlayer>>,
    mut cameras: Query<&mut OrbitCamera>,
    mut units: Query<(&mut RemoteUnit, Has<Driven>)>,
) {
    // a mount that left the view is let go of (the server detaches the riders before it despawns a mate)
    let attached = player.spawn.as_ref().and_then(|s| s.unit.attached_to).filter(|a| net.by_id.contains_key(&a.obj_id));
    let Ok((entity, mut mover, rider, character, animator)) = local.single_mut() else { return };
    let old = seat_state.0;
    if old.map(|s| (s.mount, s.point)) != attached.map(|a| (a.obj_id, a.point)) {
        // the mate the player drove goes back to its movement packets, from where it stands
        if let Some(e) = old.filter(|s| s.drives).and_then(|s| views.unit_entity(s.mount)) {
            commands.entity(e).remove::<Driven>();
            if let Ok((mut unit, _)) = units.get_mut(e) {
                let (pos, facing) = (unit.pos, unit.facing);
                unit.reset_track(pos, facing);
            }
        }
        seat_state.0 = attached.map(|a| {
            let (model, cart) = net.by_id.get(&a.obj_id).map_or((0, false), |u| {
                (u.model_id, matches!(u.kind, UnitKind::Slave { .. }))
            });
            let speed = db.as_deref().map_or_else(MountSpeed::default, |db| db.mount_speed(model));
            LocalSeat { mount: a.obj_id, point: a.point, drives: a.point == 1, cart, speed, lift: 0.0 }
        });
        match attached {
            Some(a) => {
                // on the mount: where it is drawn (else where the server has it), facing its way
                let shown = views.unit_entity(a.obj_id).and_then(|e| units.get(e).ok()).map(|(u, _)| (u.pos, u.facing));
                let known = net.by_id.get(&a.obj_id).map(|u| (Vec3::new(u.pos.x, u.pos.y, u.pos.z), u.yaw()));
                if let Some((pos, facing)) = shown.or(known) {
                    mover.pos = pos;
                    mover.facing = facing;
                    if let Ok(mut orbit) = cameras.single_mut() {
                        orbit.yaw = facing;
                    }
                }
                info!("riding {} at point {}: {:?}", a.obj_id, a.point, seat_state.0.map(|s| s.speed));
            }
            None => info!("got off the mount at ({:.1}, {:.1}, {:.1})", mover.pos.x, mover.pos.y, mover.pos.z),
        }
        mover.velocity = Vec3::ZERO;
        mover.vz = 0.0;
        mover.swim = None;
        mover.grounded = true;
        send.send_now();
    }
    // the driven mate's unit (built again when it comes back into view) follows the mover
    if let Some(e) = seat_state.0.filter(|s| s.drives).and_then(|s| views.unit_entity(s.mount))
        && units.get(e).is_ok_and(|(_, driven)| !driven)
    {
        commands.entity(e).insert(Driven);
    }
    // the rider's clips (also for a player entity built again while riding)
    match (attached, rider) {
        (Some(a), None) => {
            let (Some(db), Some(character), Some(animator)) = (db.as_deref(), character, animator) else { return };
            let model = net.by_id.get(&a.obj_id).map_or(0, |u| u.model_id);
            let Some(rider) = Rider::bind(db, &clips, character, animator, a, model) else { return };
            commands.queue(move |world: &mut World| seat(world, entity, rider));
        }
        (Some(a), Some(r)) if r.mount != a.obj_id || r.point != a.point => commands.queue(move |world: &mut World| unseat(world, entity)),
        (None, Some(_)) => commands.queue(move |world: &mut World| unseat(world, entity)),
        _ => {}
    }
}

/// The mate the player drives stands where the player's mover is and plays its gait for the mover's speed; a passenger's mover
/// goes with the mount.
#[allow(clippy::type_complexity)]
pub fn drive_mount(
    seat: Res<LocalMount>,
    views: Res<Views>,
    net: Res<Units>,
    mut player: Query<&mut Mover, With<LocalPlayer>>,
    mut driven: Query<(&mut RemoteUnit, &mut Transform, Option<&mut Motion>), (With<Driven>, Without<LocalPlayer>)>,
    mounts: Query<&RemoteUnit, Without<Driven>>,
) {
    let (Some(s), Ok(mut mover)) = (seat.0, player.single_mut()) else { return };
    // a cart's driver is moved by `cart::drive_cart`; a cart passenger still follows the cart here
    if s.cart && s.drives {
        return;
    }
    let entity = views.unit_entity(s.mount);
    if !s.drives {
        let shown = entity.and_then(|e| mounts.get(e).ok()).map(|u| (u.pos, u.facing));
        let known = net.by_id.get(&s.mount).map(|u| (Vec3::new(u.pos.x, u.pos.y, u.pos.z), u.yaw()));
        if let Some((pos, facing)) = shown.or(known) {
            mover.pos = pos;
            mover.facing = facing;
        }
        return;
    }
    let Some((mut unit, mut transform, motion)) = entity.and_then(|e| driven.get_mut(e).ok()) else { return };
    unit.pos = mover.pos;
    unit.facing = mover.facing;
    *transform = placement(unit.pos, unit.facing, unit.scale());
    if let Some(mut motion) = motion {
        let speed = mover.velocity.truncate().length();
        let moving = speed > 0.3;
        *motion = Motion { speed: if moving { speed } else { 0.0 }, local_dir: if moving { Vec2::Y } else { Vec2::ZERO }, airborne: false };
    }
}

/// Puts the local player on its mount's seat and plays the rider clips for the mount's gait.
#[allow(clippy::type_complexity)]
pub fn pose_local_rider(
    mut seat: ResMut<LocalMount>,
    views: Res<Views>,
    mut riders: Query<(&Mover, &mut Transform, &mut Rider, &mut Animator), With<LocalPlayer>>,
    mounts: Query<(&Transform, &Animator, &Motion), (With<RemoteUnit>, Without<LocalPlayer>)>,
    globals: Query<&GlobalTransform>,
) {
    let (Some(s), Ok((mover, mut transform, mut rider, mut animator))) = (seat.0, riders.single_mut()) else { return };
    let mounted = views.unit_entity(s.mount).and_then(|e| mounts.get(e).ok().map(|m| (e, m)));
    let (pos, speed, tempo) = match mounted {
        Some((e, (now, mount_animator, motion))) => (bevy_to_cry(seat_world(e, now, mount_animator, s.point, &globals)), motion.speed, mount_animator.speed),
        None => (mover.pos, 0.0, 1.0),
    };
    *transform = placement(pos, mover.facing, 1.0);
    rider.step(&mut animator, speed, tempo);
    let lift = (pos.z - mover.pos.z).max(0.0);
    if (lift - s.lift).abs() > 0.01 {
        debug!("seat of {} at ({:.2}, {:.2}, {:.2}), {lift:.2} m over the mount's feet at ({:.2}, {:.2}, {:.2})", s.mount, pos.x, pos.y, pos.z, mover.pos.x, mover.pos.y, mover.pos.z);
        seat.0 = Some(LocalSeat { lift, ..s });
    }
}

/// `AA_FAKE_MOUNT=1`, a check for the riders without a server: a horse (model 139) and a rider that looks like the local
/// player appear beside the player, the rider is attached to the horse, and the horse runs a circle at 5 m/s, reported ten
/// times a second like the walkthrough's circle. `AA_FAKE_MOUNT=still` parks the horse 4 m from the player instead (for a
/// close look at the seat).
pub fn fake_mount(time: Res<Time>, player: Res<Player>, mut units: ResMut<Units>, mut events: MessageWriter<NetEvent>, mut state: Local<Option<(f32, f32)>>) {
    const HORSE: u32 = 9_000_001;
    const RIDER: u32 = 9_000_002;
    const RADIUS: f32 = 8.0;
    const SPEED: f32 = 5.0;
    if std::env::var("AA_FAKE_MOUNT").is_err() {
        return;
    }
    let Some(spawn) = &player.spawn else { return };
    let now = time.elapsed_secs();
    let still = std::env::var("AA_FAKE_MOUNT").is_ok_and(|v| v == "still");
    let centre = Pos::new(spawn.unit.pos.x + if still { 4.0 } else { RADIUS + 3.0 }, spawn.unit.pos.y, spawn.unit.pos.z);
    let (started, last_sent) = *state.get_or_insert_with(|| {
        let mut horse = spawn.unit.clone();
        horse.obj_id = HORSE;
        horse.name = "Horse".into();
        horse.kind = UnitKind::Mate { tl_id: 1, template_id: 0, owner_id: 0 };
        horse.model_id = 139;
        horse.equipment = Equipment::default();
        horse.model_params = ModelParams::default();
        horse.pos = Pos::new(centre.x, centre.y, centre.z);
        let mut rider = spawn.unit.clone();
        rider.obj_id = RIDER;
        rider.name = "Rider".into();
        rider.pos = horse.pos;
        units.by_id.insert(HORSE, horse);
        units.by_id.insert(RIDER, rider);
        (now, 0.0)
    });
    // seat the rider once the horse has been announced for a moment
    if now - started > 4.0
        && let Some(rider) = units.by_id.get_mut(&RIDER)
    {
        rider.attached_to.get_or_insert(Attachment { obj_id: HORSE, point: 1 });
    }
    if now - last_sent < 0.1 || now - started < 2.0 || (still && last_sent > 0.0) {
        return;
    }
    let t = if still { 0.0 } else { now - started };
    let angle = t * SPEED / RADIUS;
    let pos = Pos::new(centre.x + RADIUS * angle.cos() - RADIUS, centre.y + RADIUS * angle.sin(), centre.z);
    let yaw = (SPEED * angle.sin()).atan2(SPEED * angle.cos());
    let mut movement = Movement::unit(pos, yaw, (t * 1000.0) as u32);
    if !still {
        movement.velocity = [(-yaw.sin() * SPEED * 1024.0) as i16, (yaw.cos() * SPEED * 1024.0) as i16, 0];
        movement.delta_movement = [0, 127, 0];
    }
    if let Some(horse) = units.by_id.get_mut(&HORSE) {
        horse.pos = pos;
    }
    events.write(NetEvent(Event::UnitMoved { obj_id: HORSE, movement }));
    *state = Some((started, now));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mounts_without_a_rider_set_of_their_own_are_ridden_like_a_horse() {
        assert_eq!(rider_prefix(1), "horse");
        assert_eq!(rider_prefix(0), "horse", "a donkey, an elk");
        assert_eq!(rider_prefix(2), "lion");
        assert_eq!(rider_prefix(4), "wildboar");
        assert_eq!(rider_prefix(10), "pangolin");
    }

    #[test]
    fn the_rider_walks_and_runs_with_the_mount() {
        assert_eq!(gait(0.0, true), Gait::Idle);
        assert_eq!(gait(2.0, true), Gait::Walk);
        assert_eq!(gait(9.0, true), Gait::Run);
        assert_eq!(gait(9.0, false), Gait::Walk, "no run clip");
    }

    #[test]
    fn only_the_driver_s_seat_steers() {
        let seat = |point| {
            LocalMount(Some(LocalSeat {
                mount: 7,
                point,
                drives: point == 1,
                cart: false,
                speed: MountSpeed::default(),
                lift: 1.2,
            }))
        };
        assert_eq!(seat(1).driving(), Some((7, MountSpeed::default())));
        assert!(!seat(1).passenger());
        assert_eq!(seat(1).cart_driver(), None);
        assert_eq!(seat(2).driving(), None);
        assert!(seat(2).passenger());
        let cart = |point| {
            LocalMount(Some(LocalSeat {
                mount: 7,
                point,
                drives: point == 1,
                cart: true,
                speed: MountSpeed::default(),
                lift: 1.2,
            }))
        };
        assert_eq!(cart(1).cart_driver(), Some(7));
        assert_eq!(cart(1).driving(), None, "a cart has its own controller");
        assert_eq!(cart(2).cart_driver(), None);
        assert!(cart(2).passenger());
        assert!((seat(2).lift() - 0.35).abs() < 1e-6, "the seat 1.2 m up, less the seated drop");
        assert_eq!(LocalMount::default().driving(), None);
        assert_eq!(LocalMount::default().lift(), 0.0);
    }
}
