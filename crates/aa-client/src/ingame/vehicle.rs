//! Vehicles and ships (`models.sub_type` `VehicleModel` / `ShipModel`: airships, carts, boats, siege weapons), drawn from
//! their `normal` look ([`super::data::Db::vehicle_look`]): a single model, or every part of a prefab (the hull and deck
//! brushes, and the `AnimObject` parts such as an airship's wings, propellers and planks). Wheels go on the body's wheel
//! mounts: `wheelN` bones or helper nodes take the `wheel` model, `wheel_type2N` ones `wheel2`.
//!
//! The wheels spin about their axle from the cart's forward speed (`FUN_390d6b10`) and the body's
//! `Bone_spindle` turns the front axle (`FUN_390cffd0`); see [`spin_wheels`] and [`steer_spindles`].
//!
//! Damage states ([`model_state`] / [`update_damage`]) swap the whole body prefab to the state's
//! `vehicle_models` / `ship_models` row (`damaged25` / `damaged50` / `damaged75` / `dying` / `dead`, falling
//! down to the next lower non-empty row). A death enters dying (5) first, shows that prefab for
//! `models.dying_time`, then switches to dead (6) (`FUN_390e7ad0`, `FUN_390eaac0`). The variant prefab's
//! active particle entities (the fire and smoke of the damaged and dead looks) are drawn with it, and a
//! state that worsens plays the new prefab's `_spawn_effect*` entities once ([`spawn_state_effects`]). A
//! rigged part also plays its `AnimObject`'s authored `<Animation>` (`Animation`, `bPlaying`, `bLoop`;
//! `Part::animation`), the dying cart's tip/roll. The new prefab is read on the workers (`VehicleSwaps`);
//! the old parts stay until it is ready. A dead ship starts [`WreckSink`] and keeps the previous prefab's
//! fire (`carry_ship_fire`); the wreck itself stays until the server's `SCUnitsRemoved` (no client timer).
//!
//! The maximum is unit attribute 6 as `FUN_390b4a70` reads it: the `unit_formulas` base of the slave
//! plus its `unit_modifiers`. It is cached in [`VehicleDamage`] at spawn ([`slave_max_hp`]).
//!
//! A slave the server creates (`SCSlaveCreatedPacket` with `hideSpawnEffect` false) emerges from its
//! `slaves.portal_spawn_fx_id` portal: the unit starts two bounding extents behind its position, behind the portal
//! plane, and moves out over `portal_time` while the portal hangs at the plane (`FUN_3909ea30` / `FUN_39097220`,
//! [`track_summon_portals`] / [`spawn_summon_portals`] / [`animate_summon_emergence`]). The group's `FxSound` items
//! and particle sounds start at the portal through [`super::sounds::Audio`]. A ship (the Eznan Cutter) instead
//! floats up out of the sea with the captured bow-up buoyancy rise (inferred; see [`SummonEmergence`]), clipped at
//! the sea surface. A vehicle that merely streams into view later has no creation packet, so it plays none.
//!
//! Left out: the speed-driven exhaust and splash emitters (`FUN_390c78d0`), propeller spin, turrets.
//! Combat effects anchored to a body
//! bone (`super::attachments`) go away with the old parts on a swap: re-anchoring them by bone name is
//! not implemented (they are a skill's own one-shot effects, not the vehicle's prefab emitters).

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};

use aa_anim::{AnimSet, Animator, ClipLibrary, pose::BoundClip};
use aa_cry::scene::{CryAssets, ModelOptions, Shading, spawn_loaded};
use aa_cry::shade::illum::IllumMaterial;
use aa_cry::workers::Workers;
use aa_cry::{CryLoader, PreparedModel};
use aa_net::state::Units;
use aa_net::slave::SlaveEvent;
use aa_net::{Event, NetEvent};
use aa_world::coords::cry_rot_to_bevy;
use aa_world::cry_to_bevy;
use bevy::math::{Affine3A, Vec4};
use bevy::pbr::MeshMaterial3d;
use bevy::prelude::*;
use bevy::transform::TransformSystems;

use super::cart::{YAW_RATE_PER_STEER, wrap_angle};
use super::combat::Dead;
use super::data::{Db, VehicleLook, model_file};
use super::housing::prefab::{self, Part};
use super::units::RemoteUnit;

/// `_DAT_399a440c` (0x399a440c = 0.52359879): the largest spindle angle, 30 degrees.
pub const SPINDLE_MAX_ANGLE: f32 = std::f32::consts::FRAC_PI_6;

/// `DAT_3999ccf0` (0x3999ccf0): the smoothing rate is `min(dt, 0.1)`.
pub const SPINDLE_LERP: f32 = 0.1;

/// The yaw rate that means full steering in the standstill fallback: the first float of the
/// model's vehicle-physics block (`*(int *)(vtable+0x1e0) + 0x34`). That block is built at
/// runtime from the vehicle data, so the value cannot be read from the unpacked dll; the captured
/// full-lock rate [`YAW_RATE_PER_STEER`] stands in for it.
pub const SPINDLE_YAW_LIMIT: f32 = YAW_RATE_PER_STEER;

/// The prefab-unit model state of `FUN_3989f8b0`: 1 normal, 2 `damaged25`, 3 `damaged50`, 4 `damaged75`,
/// 6 dead. The percentage is C integer division, so it truncates: 7599/10000 is 75 (state 2), 7600/10000
/// is 76 (state 1). `hp == 0` or `max_hp == 0` is dead before the divide. This is only the polled state:
/// 5 (dying) is entered by the death path (`FUN_390e7ad0`), never by the HP percentage.
pub fn model_state(hp: i32, max_hp: i32) -> u8 {
    if hp == 0 || max_hp == 0 {
        return 6;
    }
    let pct = hp * 100 / max_hp;
    if pct > 75 {
        1
    } else if pct > 50 {
        2
    } else if pct < 26 {
        4
    } else {
        3
    }
}

/// The damage state a vehicle is drawn in, the model ref it was built from, the cached maximum health and the
/// dying window (`models.dying_time`).
#[derive(Component, Clone, Debug, PartialEq)]
pub struct VehicleDamage {
    pub state: u8,
    /// The variant ref, so a state change that keeps the same model does not rebuild or fire effects
    /// (`FUN_390e7910` logs "same model" and returns before the effect trigger).
    pub variant: String,
    /// The template maximum (`slave_max_hp`), constant for the vehicle's life: held here so the per-frame
    /// state check does not lock the database.
    pub max_hp: i32,
    /// The state a worker is preparing, so one state change queues one swap.
    pub requested: Option<u8>,
    /// The dying start time (`model+0x274`, in [`Time::elapsed_secs_f64`]): `Some` from entering state 5 until
    /// the dead prefab is applied. While it is set the per-frame update does not recompute the state from HP.
    pub dying_start: Option<f64>,
    /// `models.dying_time` seconds (`template+0x14`, checked by `FUN_390eaac0`); 0 means dying lasts one tick.
    pub dying_time: f32,
    /// The current death has already entered dying, so a later update does not restart it. Cleared when the unit
    /// is alive again (hp > 0 and not dead).
    pub death_seen: bool,
    /// The dead prefab's one-shot `_spawn_effect*` already played for this death: an HP-0 update shows 6 first,
    /// then the death's 6 -> 5 -> 6 route must not play them a second time. Cleared when a living state is adopted.
    pub dead_effect_played: bool,
}

/// How long a destroyed ship takes to sink below the sea after its dead prefab is applied. The
/// 2026-11-02 capture (`2026-11-02-vehicle-visuals-capture.md`, "Eznan Cutter damage and death") shows
/// the hull listing and going under over ~12 s, still burning; the original client has no traced sink
/// function (the ship model update has no damage-state gate and the server stops moving the hull), so
/// the rate is measured from the capture (inferred).
pub const WRECK_SINK_TIME: f64 = 12.0;

/// How far the wreck drops by the end of [`WRECK_SINK_TIME`]. The dead galleon hull is ~20 m tall;
/// the capture's bow is the only part left above the water at 365 s, ~12.6 s after death.
pub const WRECK_SINK_DEPTH: f32 = 14.0;

/// Bow-up pitch at the end of the sink, and the list to port. The capture shows the bow still above
/// water at 365 s while the stern is under, with a visible heel (inferred).
pub const WRECK_SINK_PITCH: f32 = 0.14;
pub const WRECK_SINK_ROLL: f32 = 0.18;

/// A dead ship's sink: the pose it started from, the [`Time::elapsed_secs_f64`] of the dead swap and
/// the offset from the server pose that `units::move_units` applies. The original client's hull
/// list/sink is not in the decompiled ship update (it has no damage-state gate) and the server removes
/// the ship from its physics at death, so the captured motion is client side; the original mechanism
/// (engine buoyancy/physics) is not recovered (inferred).
#[derive(Component, Clone, Copy, Debug)]
pub struct WreckSink {
    pub base: Transform,
    pub started: f64,
    /// Pose relative to the server pose; fixed once [`Self::finished`].
    pub offset: Transform,
    pub finished: bool,
}

impl WreckSink {
    fn at(base: &Transform, now: f64) -> Self {
        Self { base: *base, started: now, offset: Transform::IDENTITY, finished: false }
    }

    /// A wreck that was already dead when it entered view: start at the fully sunk pose (inferred; the
    /// client has no death timestamp for a unit that was culled or reconnected).
    fn sunk(base: &Transform, now: f64) -> Self {
        Self {
            base: *base,
            started: now - WRECK_SINK_TIME,
            offset: Self::sunk_offset(1.0),
            finished: true,
        }
    }

    fn sunk_offset(ease: f32) -> Transform {
        Transform {
            translation: -Vec3::Y * (WRECK_SINK_DEPTH * ease),
            rotation: Quat::from_axis_angle(Vec3::X, WRECK_SINK_PITCH * ease)
                * Quat::from_axis_angle(Vec3::Z, WRECK_SINK_ROLL * ease),
            ..Transform::IDENTITY
        }
    }

    /// The server pose with this sink's offset applied.
    pub fn apply(&self, base: Transform) -> Transform {
        Transform {
            translation: base.translation + self.offset.translation,
            rotation: base.rotation * self.offset.rotation,
            scale: base.scale,
        }
    }
}

/// Starts the sink of a ship that was already dead when it entered view (streamed in as a corpse,
/// reconnected within the server's ~20 s wreck window). The dead prefab alone floats at the surface;
/// the wreck is placed at its end pose, and the 25 % prefab's fire is carried so it burns like the
/// capture's wreck (both inferred: the original client has no death timestamp for such a unit).
pub fn start_dead_ship_sinks(
    mut commands: Commands,
    db: Option<Res<Db>>,
    time: Res<Time>,
    roots: Query<(Entity, &RemoteUnit, &VehicleDamage, &Transform), Added<VehicleDamage>>,
    sinks: Query<(), With<WreckSink>>,
) {
    let Some(db) = db else { return };
    let now = time.elapsed_secs_f64();
    for (entity, remote, damage, transform) in &roots {
        if damage.state != 6 || sinks.contains(entity) || !db.is_ship(remote.model_id) {
            continue;
        }
        let sink = WreckSink::sunk(transform, now);
        if let Some(variants) = db.vehicle_variants(remote.model_id) {
            carry_ship_fire(&mut commands, entity, &variants.for_state(4));
        }
        commands.entity(entity).insert((sink, sink.apply(*transform)));
    }
}

/// The sinks of dead ships: move the offset from the server pose (which `units::move_units` applies)
/// along the capture's gentle bow-up list over [`WRECK_SINK_TIME`]. Only writes a transform while the
/// offset changes; once fully sunk it leaves the fixed offset for `move_units`, so a stationary wreck
/// does not fight it every frame. A heal removes the sink and restores the server pose.
pub fn sink_ship_wrecks(
    time: Res<Time>,
    mut commands: Commands,
    mut sinks: Query<(Entity, &mut WreckSink, &VehicleDamage, &mut Transform)>,
) {
    let now = time.elapsed_secs_f64();
    for (entity, mut sink, damage, mut transform) in &mut sinks {
        if damage.state != 6 {
            commands.entity(entity).remove::<WreckSink>();
            *transform = sink.base;
            continue;
        }
        if sink.finished {
            continue;
        }
        let p = ((now - sink.started) / WRECK_SINK_TIME).clamp(0.0, 1.0) as f32;
        // smoothstep: the hull settles slowly at first, like the captured list
        let ease = p * p * (3.0 - 2.0 * p);
        let offset = WreckSink::sunk_offset(ease);
        if sink.offset != offset {
            sink.offset = offset;
            *transform = sink.apply(sink.base);
        }
        if p >= 1.0 {
            sink.finished = true;
        }
    }
}

/// A body part root spawned by [`spawn`]: replaced when the damage state changes.
#[derive(Component)]
pub struct VehiclePart;

/// A particle entity spawned with a damage variant: replaced when the state changes.
#[derive(Component)]
pub struct VehicleEffect;

/// The unit-space bounding extent of the vehicle's body at spawn (`[bounds+0x28]` of the 1.2 model). The summon
/// emergence puts the portal plane one extent behind the unit and its start two extents behind.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct VehicleExtent(pub f32);

/// The timeline ids (packet `+0xc`, unit `+0x41c`) whose summon portal is still pending, and the
/// `Time::elapsed_secs_f64` of their `SCSlaveCreatedPacket`. `OnSlaveCreated` pushes the timeline id into
/// `SlaveManager`'s pending list unless `hideSpawnEffect` is set (`FUN_3934a6c0` -> `FUN_3934a530`); `FUN_39349600`
/// resolves it against the unit (`FUN_3935e3f0`). A vehicle that only streams into view later has no entry.
#[derive(Resource, Default)]
pub struct SummonPortals {
    pending: HashMap<u16, f64>,
}

/// Most timeline ids kept waiting: a unit that is never drawn must not grow this map for the whole session.
pub const SUMMON_PORTALS_MAX: usize = 64;

/// How long a creation packet waits for its unit to appear. 1.2 resolves against unit existence, not drawing, but an
/// entry that never resolves (created out of range and never streamed in) must expire; a drawn unit appears in well
/// under this window.
pub const SUMMON_PENDING_TTL: f64 = 10.0;

impl SummonPortals {
    fn announce(&mut self, tl_id: u16, now: f64) {
        if self.pending.len() >= SUMMON_PORTALS_MAX && !self.pending.contains_key(&tl_id)
            && let Some(oldest) = self.pending.iter().min_by(|a, b| a.1.total_cmp(b.1)).map(|(id, _)| *id)
        {
            self.pending.remove(&oldest);
        }
        // a reused timeline id restarts the wait instead of keeping the old packet's deadline
        self.pending.insert(tl_id, now);
    }

    fn forget(&mut self, tl_id: u16) {
        self.pending.remove(&tl_id);
    }
}

/// The 1.2 summon emergence (`FUN_3909ea30` / `FUN_39097220`): the unit starts at `start = end - axis * 2 * extent`,
/// behind the portal plane at `end - axis * extent`, and moves to `end` over `slaves.portal_time`.
/// `FUN_39097390` restores `end` and sends the portal effect's end event 5.
///
/// Ships add the captured rise (`FUN_390df580` builds the ship's CryPhysics buoyancy volumes, but it is reached via
/// the model vtable (`FUN_390e2220`/`FUN_390e3530`/`FUN_390eb720`), not directly from `ShipUnitModel::Init`/`Update`
/// (`FUN_390e8b30`/`FUN_390ebc50` do not reference it), so the attribution itself is **inferred**): the root starts
/// below the sea with the hull top at the surface (`ship_models.mass_center_z` + half `mass_box_size_z`), starts
/// pitched bow up and rights over the same `portal_time`, and the clip plane is the horizontal sea surface instead
/// of the native vertical portal plane. The start depth, the horizontal-only sea clip, the 50 degree pitch and the
/// easing are capture approximations, not decompiled values (2026-11-02 Eznan Cutter capture, case
/// `ship-sea-summon`).
#[derive(Component, Clone, Copy, Debug)]
#[allow(dead_code)] // `plane`, `axis` and `extent` are read by `examples/slave_summon` and the tests
pub struct SummonEmergence {
    pub start: Vec3,
    pub end: Vec3,
    /// The clip-plane point and portal anchor: `end - axis * extent` (`FUN_3909ea30` state +0x74); for a ship the
    /// same x/z lifted to the sea surface (the horizontal clip plane then; capture-inferred).
    pub plane: Vec3,
    /// The unit's forward axis projected to the horizontal plane.
    pub axis: Vec3,
    /// The model's bounding extent (`[bounds+0x28]`); the start is two of these behind the unit.
    pub extent: f32,
    pub started: f64,
    pub duration: f32,
    pub ended: bool,
    /// The root rotation at the start (the unit's yaw for land vehicles, pitched bow up for a ship).
    pub start_rotation: Quat,
    /// The unit's rotation at `end` (its yaw only); the ship rights itself to this.
    pub end_rotation: Quat,
    /// The clip-plane normal: the unit's forward for the native vertical portal plane, `+Y` for a ship's sea surface
    /// (the horizontal-only ship clip is capture-inferred).
    pub clip_normal: Vec3,
    /// A ship rises and rights itself (the inferred buoyancy); a land vehicle only slides through the portal.
    pub buoyant: bool,
}

/// Safety cap for a summon effect whose library def never resolved (no client pak or a failed read).
const SUMMON_EFFECT_CAP: f64 = 30.0;

/// The bow-up pitch a summoned ship starts with before righting itself. The 2026-11-02 Eznan Cutter capture shows
/// the mast/yard line at 45-60 degrees, so 50 is the middle; the native value comes from CryPhysics buoyancy and
/// is not a decompiled constant (capture-pending, case `ship-sea-summon`).
pub const SHIP_SUMMON_PITCH: f32 = 50.0 * std::f32::consts::PI / 180.0;

/// A summon portal effect: it stops emitting at `until`, its live particles fade, and the fallback cap
/// `despawn_at` removes it even if the simulation never finishes (the 1.2 end event 5 plus a bounded life).
#[derive(Component, Clone, Copy, Debug)]
pub struct SummonPortalEffect {
    /// The `Time::elapsed_secs_f64` at which emission stops.
    pub until: f64,
    /// The `Time::elapsed_secs_f64` at which the entity is removed whatever the simulation does.
    pub despawn_at: f64,
    /// The end event was sent (the stop is not repeated every frame).
    pub stopped: bool,
}

/// The world anchor of a summon portal group at the clip-plane point (`FUN_3909ea30`): it does not move with the
/// emerging unit. It is despawned with its unit, or after its effects have faded.
#[derive(Component, Clone, Copy, Debug)]
pub struct SummonPortalAnchor {
    pub root: Entity,
    /// `Time::elapsed_secs_f64` when `portal_time` ends; the anchor then goes once its effects are gone.
    pub until: f64,
}

/// The per-mesh `IllumMaterial` clones carrying the portal clip plane (`FUN_3909ea30` model vfunc 0xf8), restored when
/// the emergence window ends. The clone is per vehicle so shared materials of other objects are untouched.
#[derive(Component, Default)]
pub struct EmergenceClip {
    /// `(mesh entity, original handle, clipped handle)`: the restore only touches a mesh that still holds its clone.
    originals: Vec<(Entity, Handle<IllumMaterial>, Handle<IllumMaterial>)>,
    added: Vec<Handle<IllumMaterial>>,
}

/// Remembers the slaves `SCSlaveCreatedPacket` announced without `hideSpawnEffect`, and forgets a timeline id whose
/// later creation is hidden. Entries expire in [`spawn_summon_portals`] so a packet for a unit that never appears
/// cannot linger.
pub fn track_summon_portals(
    events: Option<MessageReader<NetEvent>>,
    mut portals: ResMut<SummonPortals>,
    time: Res<Time>,
) {
    let Some(mut events) = events else {
        return;
    };
    let now = time.elapsed_secs_f64();
    for NetEvent(event) in events.read() {
        match event {
            Event::Slave(SlaveEvent::Created(created)) if created.hide_spawn_effect => {
                // a hidden creation for the same timeline id supersedes a pending visible one
                portals.forget(created.tl_id);
            }
            Event::Slave(SlaveEvent::Created(created)) => portals.announce(created.tl_id, now),
            Event::Slave(SlaveEvent::Removed { tl_id, .. }) => portals.forget(*tl_id),
            _ => {}
        }
    }
}

/// Starts the summon emergence and portal of every waiting slave whose unit is drawn. The original `FUN_39349600`
/// (x2game.dll.c:680653) finds the unit by timeline id (`FUN_3935e3f0`), reads the slave row's `+0x58` (spawn fx
/// group), `+0x54` (scale) and `+0x5c` (time), and calls the world slot `0x2a8` = `FUN_390cf130`
/// (x2game.dll.c:167908). That only starts an emergence when the time is above `DAT_3999c484` (0.0), through
/// `FUN_390ca1e0` -> `FUN_3909ea30` (x2game.dll.c:136679): it anchors the fx at `pos - forward * extent`, scale
/// `unit_scale * portal_scale`, puts the unit at `pos - forward * 2 * extent` and clips it at the plane while
/// `FUN_39097220` (x2game.dll.c:132165) moves it to `pos` over `portal_time`.
///
/// The pending map is keyed by timeline id and resolved in one pass over the drawn roots (no per-pending scan); an
/// entry older than [`SUMMON_PENDING_TTL`] is dropped.
#[allow(clippy::too_many_arguments)]
pub fn spawn_summon_portals(
    mut commands: Commands,
    db: Option<Res<Db>>,
    book: Option<Res<super::combat::skills::SkillBook>>,
    units: Option<Res<Units>>,
    water: Option<Res<aa_world::WaterQuery>>,
    audio: super::sounds::Audio,
    mut portals: ResMut<SummonPortals>,
    time: Res<Time>,
    roots: Query<(Entity, &RemoteUnit, &VehicleDamage, &Transform, Option<&VehicleExtent>)>,
) {
    if portals.pending.is_empty() {
        return;
    }
    let now = time.elapsed_secs_f64();
    portals
        .pending
        .retain(|_, announced| now - *announced <= SUMMON_PENDING_TTL);
    if portals.pending.is_empty() {
        return;
    }
    let (Some(db), Some(book), Some(units)) = (db, book, units) else {
        return;
    };
    let mut resolved = Vec::new();
    for (root, remote, damage, transform, extent) in &roots {
        let Some(unit) = units.by_id.get(&remote.obj_id) else {
            continue;
        };
        let aa_net::model::UnitKind::Slave { tl_id, .. } = unit.kind else {
            continue;
        };
        if !portals.pending.contains_key(&tl_id) {
            continue;
        }
        resolved.push(tl_id);
        let Some(portal) = db.slave_summon_portal(unit.kind.template_id()) else {
            continue;
        };
        // `FUN_390cf130` only starts the emergence when `portal_time > DAT_3999c484` (0.0): slaves 54, 94 and 115
        // (time 0) get no portal.
        if portal.time <= 0.0 {
            continue;
        }
        let extent = extent.map_or(1.0, |extent| extent.0);
        // the unit's forward (cry +Y) in Bevy is -Z, projected to the horizontal plane like the native axis
        let mut axis = transform.rotation * Vec3::NEG_Z;
        axis.y = 0.0;
        let axis = axis.normalize_or_zero();
        let axis = if axis == Vec3::ZERO { Vec3::NEG_Z } else { axis };
        let end = transform.translation;
        let end_rotation = transform.rotation;
        let mut plane = end - axis * extent;
        let mut start = end - axis * (2.0 * extent);
        let mut start_rotation = end_rotation;
        let mut clip_normal = axis;
        let mut buoyant = false;
        // A ship does not slide out of a vertical portal in the capture: it floats up out of the sea. The buoyancy
        // attribution (`FUN_390df580`, reached via the model vtable) and this whole result -- the horizontal sea clip
        // replacing the native vertical plane, the hull-top start depth, the bow-up pitch and the rise shape -- are
        // inferred from the 2026-11-02 Eznan Cutter capture, not decompiled values (case `ship-sea-summon`).
        if db.is_ship(unit.model_id)
            && let Some(surface) = water.as_ref().and_then(|water| {
                water
                    .water_at(aa_world::bevy_to_cry(end))
                    .map(|(height, _)| height)
            })
        {
            let hull_top = db.ship_hull_height(unit.model_id).map_or(1.0, |(_, high)| high.max(0.0));
            start.y = (surface - hull_top).min(end.y);
            // keep the native anchor x/z (`end - axis * extent`, FUN_3909ea30 state +0x74) and only lift it to the
            // sea surface: the horizontal clip plane needs a water point, and the anchor stays where 1.2 puts it
            plane.y = surface;
            clip_normal = Vec3::Y;
            start_rotation = end_rotation * Quat::from_axis_angle(Vec3::X, SHIP_SUMMON_PITCH);
            buoyant = true;
        }
        // the portal hangs at the clip-plane point, oriented by the unit, scaled by unit_scale x portal_scale
        let anchor = commands
            .spawn((
                Transform {
                    translation: plane,
                    rotation: transform.rotation,
                    ..default()
                },
                Visibility::default(),
                SummonPortalAnchor {
                    root,
                    until: now + f64::from(portal.time),
                },
            ))
            .id();
        let unit_scale = transform.scale.y.max(0.0);
        let mut sounds: Vec<String> = Vec::new();
        for item in book.fx_group(portal.fx_group).iter() {
            let mut parent = anchor;
            let at = plane + cry_to_bevy(item.offset);
            let effect = if item.effect.is_empty() {
                None
            } else {
                let effect = commands
                    .spawn((
                        aa_fx::Effect {
                            scale: portal.scale * item.scale * unit_scale,
                            prime: false,
                            item: true,
                            no_body: true,
                            despawn_when_finished: true,
                            ..aa_fx::Effect::new(item.effect.clone())
                        },
                        Transform {
                            translation: cry_to_bevy(item.offset),
                            ..default()
                        },
                        ChildOf(anchor),
                        SummonPortalEffect {
                            until: now + f64::from(portal.time),
                            despawn_at: now + f64::from(portal.time) + SUMMON_EFFECT_CAP,
                            stopped: false,
                        },
                    ))
                    .id();
                parent = effect;
                Some(effect)
            };
            // `FxSound` items and the `fx_particles.sound_id` of the group's particles start with the portal, at the
            // anchor (1.2 hands the whole group to the effect system at once, `FUN_3909ea30` -> `FUN_393eb7f0`).
            // The particle library's own emitter `Sound=""` events play the same way as a skill effect's.
            let mut play = |path: &str| {
                if sounds.iter().any(|seen| seen.eq_ignore_ascii_case(path)) {
                    return;
                }
                sounds.push(path.to_string());
                audio.start(
                    &mut commands,
                    &super::sounds::Play { path, class: super::sounds::Class::Skill, unit: root, parent, at },
                );
            };
            if let Some(super::combat::Sfx::Event(path)) = item.sound.as_ref() {
                play(path);
            }
            if effect.is_some()
                && let Some(sound_book) = audio.book()
            {
                for path in sound_book.particle_sounds(&item.effect).iter() {
                    play(path);
                }
            }
        }
        // the prefab's own inactive `portal_exhaust` follows the moving unit (FUN_39097140 / FUN_39097220)
        spawn_portal_exhaust(&mut commands, root, &damage.variant, now + f64::from(portal.time));
        commands.entity(root).insert((
            SummonEmergence {
                start,
                end,
                plane,
                axis,
                extent,
                started: now,
                duration: portal.time,
                ended: false,
                start_rotation,
                end_rotation,
                clip_normal,
                buoyant,
            },
            // the first rendered frame must already be clipped: hide the model until the material clones are in
            // (`units::cull_units` owns the root's `Visibility` and honours this marker)
            EmergenceHidden,
        ));
    }
    for tl_id in resolved {
        portals.forget(tl_id);
    }
}

/// Plays the prefab's authored `portal_exhaust` particle (inactive in the prefab) on the unit during the emergence,
/// as `FUN_39097140` does at the start and `FUN_39097220` keeps alive each frame; `FUN_39097390` stops it.
fn spawn_portal_exhaust(commands: &mut Commands, root: Entity, variant: &str, until: f64) -> Vec<Entity> {
    let mut spawned = Vec::new();
    for named in super::data::prefab_effects_named(variant) {
        if !named.name.eq_ignore_ascii_case("portal_exhaust") {
            continue;
        }
        let effect = named.effect;
        let entity = commands
            .spawn((
                aa_fx::Effect {
                    scale: effect.scale,
                    prime: false,
                    item: true,
                    despawn_when_finished: true,
                    ..aa_fx::Effect::new(effect.effect.clone())
                },
                Transform {
                    translation: cry_to_bevy(effect.pos),
                    rotation: cry_rot_to_bevy(effect.rot),
                    ..default()
                },
                ChildOf(root),
                SummonPortalEffect {
                    until,
                    despawn_at: until + SUMMON_EFFECT_CAP,
                    stopped: false,
                },
            ))
            .id();
        spawned.push(entity);
    }
    spawned
}

/// Moves an emerging unit from its start to its real position over `portal_time` (`FUN_39097220`), unless the local
/// player is driving it (`Driven`, `cart::drive_cart` owns the transform then). The portal-plane clip (1.2's model
/// vfunc 0xf8) is applied by [`apply_emergence_clip`] and restored by [`restore_emergence_clip`].
///
/// The horizontal slide is the native linear `FUN_39097220` path for every unit. A ship also rises and rights over
/// the same window; that part is eased (buoyancy is strong while the hull is deep and dies out as it floats) and is
/// the capture approximation described on [`SummonEmergence`].
pub fn animate_summon_emergence(
    time: Res<Time>,
    mut roots: Query<(&mut Transform, &mut SummonEmergence), Without<super::mount::Driven>>,
) {
    let now = time.elapsed_secs_f64();
    for (mut transform, mut emergence) in &mut roots {
        if emergence.ended {
            continue;
        }
        let progress = if emergence.duration > 0.0 {
            ((now - emergence.started) as f32 / emergence.duration).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let float = if emergence.buoyant { 1.0 - (1.0 - progress).powi(3) } else { progress };
        transform.translation = Vec3::new(
            emergence.start.x + (emergence.end.x - emergence.start.x) * progress,
            emergence.start.y + (emergence.end.y - emergence.start.y) * float,
            emergence.start.z + (emergence.end.z - emergence.start.z) * progress,
        );
        // Only a ship's rise owns the rotation; a land vehicle's server/remote-driver turn during the portal window
        // must not be overwritten and then snapped back (`FUN_39097220` moves the unit, it does not rotate it).
        if emergence.buoyant {
            transform.rotation = emergence.start_rotation.slerp(emergence.end_rotation, float);
        }
        if progress >= 1.0 {
            transform.translation = emergence.end;
            if emergence.buoyant {
                transform.rotation = emergence.end_rotation;
            }
            emergence.ended = true;
        }
    }
}

/// Gives every mesh of an emerging vehicle a clone of its `IllumMaterial` with the portal clip plane, so the part
/// still behind the ring is discarded (`FUN_3909ea30` model vfunc 0xf8: plane at `pos - axis * extent`, normal
/// `axis`; a ship's plane is the horizontal sea surface, normal `+Y`, capture-inferred instead of the native
/// vertical plane). It runs every frame while the emergence lasts, so parts swapped in during it are clipped too;
/// the material is per vehicle, other objects keep the shared asset.
/// Keeps a summoned vehicle hidden until its clipped material copies are in place. A marker rather than
/// `Visibility`, because [`super::units::cull_units`] rewrites the root's `Visibility` every frame.
#[derive(Component)]
pub struct EmergenceHidden;

pub fn apply_emergence_clip(
    mut commands: Commands,
    illum: Option<ResMut<Assets<IllumMaterial>>>,
    time: Res<Time>,
    mut roots: Query<(Entity, &SummonEmergence, Option<&mut EmergenceClip>)>,
    children: Query<&Children>,
    materials: Query<&MeshMaterial3d<IllumMaterial>>,
) {
    let now = time.elapsed_secs_f64();
    let Some(mut illum) = illum else {
        // no render-side Illum materials (a headless example/test): nothing to clip, do not keep the unit hidden
        for (root, emergence, _) in &mut roots {
            if !emergence.ended {
                commands.entity(root).remove::<EmergenceHidden>();
            }
        }
        return;
    };
    for (root, emergence, clip) in &mut roots {
        if emergence.ended {
            continue;
        }
        let Some(mut clip) = clip else {
            commands.entity(root).insert(EmergenceClip::default());
            continue;
        };
        let plane = Vec4::new(
            emergence.clip_normal.x,
            emergence.clip_normal.y,
            emergence.clip_normal.z,
            -emergence.clip_normal.dot(emergence.plane),
        );
        let mut stack = vec![root];
        let mut clipped_any = false;
        while let Some(entity) = stack.pop() {
            if let Ok(kids) = children.get(entity) {
                stack.extend(kids.iter());
            }
            let Ok(handle) = materials.get(entity) else {
                continue;
            };
            if clip.added.iter().any(|added| added.id() == handle.0.id())
                || clip.originals.iter().any(|(seen, ..)| *seen == entity)
            {
                continue;
            }
            let Some(original) = illum.get(&handle.0) else {
                continue;
            };
            let mut material = original.clone();
            material.extension.clip_plane = plane;
            // The depth-only prepass/shadow pipelines have no fragment stage unless the material can discard
            // (bevy_pbr's prepass: MAY_DISCARD). An opaque vehicle clone becomes alpha-masked so `clip_discard`
            // runs there too; the base alpha is 1, so the visible shading is unchanged.
            if matches!(material.base.alpha_mode, AlphaMode::Opaque) {
                material.base.alpha_mode = AlphaMode::Mask(0.5);
            }
            let clipped = illum.add(material);
            commands.entity(entity).insert(MeshMaterial3d(clipped.clone()));
            clip.originals.push((entity, handle.0.clone(), clipped.clone()));
            clip.added.push(clipped);
            clipped_any = true;
            if clip.originals.len() >= 256 {
                break;
            }
        }
        // the unit was hidden at the start so the first rendered frame is already clipped; if nothing could be
        // clipped (no Illum part), show it after a short grace instead of leaving it invisible
        if clipped_any || (clip.originals.is_empty() && now - emergence.started > 0.25) {
            commands.entity(root).remove::<EmergenceHidden>();
        }
    }
}

/// Restores the original `IllumMaterial` handles when the emergence window ends, including when the local player
/// drives the unit (the animation is skipped then, but the clip still ends after `portal_time`).
pub fn restore_emergence_clip(
    time: Res<Time>,
    mut commands: Commands,
    mut roots: Query<(Entity, &mut SummonEmergence, Option<&mut EmergenceClip>)>,
    alive: Query<()>,
    materials: Query<&MeshMaterial3d<IllumMaterial>>,
) {
    let now = time.elapsed_secs_f64();
    for (root, mut emergence, clip) in &mut roots {
        let elapsed = now - emergence.started;
        if !emergence.ended {
            if elapsed < f64::from(emergence.duration) {
                continue;
            }
            emergence.ended = true;
        }
        // shown again even when no clip was ever inserted (the window ended before the first clip frame)
        commands.entity(root).remove::<EmergenceHidden>();
        let Some(mut clip) = clip else {
            continue;
        };
        for (entity, original, clipped) in clip.originals.drain(..) {
            // stealth or another system may have replaced the material meanwhile: only undo our own clone
            if alive.get(entity).is_ok()
                && materials
                    .get(entity)
                    .is_ok_and(|handle| handle.0.id() == clipped.id())
            {
                commands.entity(entity).insert(MeshMaterial3d(original));
            }
        }
        commands.entity(root).remove::<EmergenceHidden>();
        commands.entity(root).remove::<EmergenceClip>();
    }
}

/// The end of the emergence (`FUN_39097390`): send the portal effect end event 5 (`FUN_393ed520(fx, 5, ...)`) by
/// stopping emission and letting the live particles fade, instead of killing the effect. The effect still has a
/// bounded life: `despawn_at` is reset to `until + the library's longest particle life`, and a safety-cap system
/// removes it even when the stopped simulation never reports finished.
pub fn end_summon_portals(
    time: Res<Time>,
    mut effects: Query<(Entity, &mut SummonPortalEffect, &mut aa_fx::FxRun)>,
) {
    let now = time.elapsed_secs_f64();
    for (_entity, mut effect, mut run) in &mut effects {
        if effect.stopped || now < effect.until {
            continue;
        }
        run.stop_emitting();
        effect.stopped = true;
        effect.despawn_at = now + f64::from(run.max_particle_life().unwrap_or(10.0)) + 0.25;
    }
}

/// Removes a summon effect after `until` + its longest particle life (the fallback cap included), so a continuous
/// effect that never finishes and a stopped sim that is rebuilt cannot linger.
pub fn despawn_summon_portal_effects(
    time: Res<Time>,
    mut commands: Commands,
    effects: Query<(Entity, &SummonPortalEffect)>,
) {
    let now = time.elapsed_secs_f64();
    for (entity, effect) in &effects {
        if now >= effect.despawn_at {
            commands.entity(entity).try_despawn();
        }
    }
}

/// Drops a portal anchor with its unit, or once its effects have faded. A safety window bounds a failed read of the
/// prefab's effects.
pub fn cleanup_summon_portals(
    time: Res<Time>,
    mut commands: Commands,
    anchors: Query<(Entity, &SummonPortalAnchor)>,
    children: Query<&Children>,
    roots: Query<()>,
) {
    let now = time.elapsed_secs_f64();
    for (entity, anchor) in &anchors {
        let root_gone = roots.get(anchor.root).is_err();
        let empty = match children.get(entity) {
            Ok(children) => children.is_empty(),
            Err(_) => true,
        };
        if root_gone || (now >= anchor.until && (empty || now >= anchor.until + 10.0)) {
            commands.entity(entity).try_despawn();
        }
    }
}

/// A body prefab being read on the workers for [`VehicleDamage::requested`].
struct VehicleSwapBuilt {
    root: Entity,
    obj_id: u32,
    state: u8,
    variant: String,
    pieces: Vec<Piece>,
}

/// The damage variants being prepared off the main thread, and their delivery channel. A swap replaces the old
/// parts only after [`finish_damage_swaps`] receives the new ones, so a slow or failed read never leaves the
/// vehicle invisible.
#[derive(Resource)]
pub struct VehicleSwaps {
    tx: Sender<VehicleSwapBuilt>,
    rx: Mutex<Receiver<VehicleSwapBuilt>>,
}

impl Default for VehicleSwaps {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self {
            tx,
            rx: Mutex::new(rx),
        }
    }
}

/// The maximum health of a slave vehicle: unit attribute 6 (`FUN_390b4a70` reads it through
/// `FUN_398a0bc0`), the formula base plus the template's `unit_modifiers`. The unit's own `max_hp` is only a
/// last resort when the database has no `MaxHealth` formula; 1.2 never uses the wire maximum or
/// `SCMySlave.max_hp`.
pub fn slave_max_hp(db: &Db, unit: &aa_net::model::Unit) -> i32 {
    db.slave_max_hp(unit.kind.template_id(), unit.level)
        .unwrap_or(unit.max_hp)
        .max(0)
}

/// The damage state of a unit from its current health (`SCUnitPoints` / `SCUnitState`) and the attribute-6
/// maximum. Used when a vehicle is first announced; [`update_damage`] then uses the cached [`VehicleDamage::max_hp`].
pub fn damage_state(db: &Db, unit: &aa_net::model::Unit) -> u8 {
    model_state(unit.hp, slave_max_hp(db, unit))
}

/// The state a newly announced vehicle is built in. `InitClientUnit` (`FUN_3936b000`) and `OnModelChanged`
/// (`FUN_39364db0`) call the model's Die slot when `hp == 0`, which tries the dying state first
/// (`FUN_390eaa80` -> `FUN_390e7ad0`), so such a unit starts dying (5); every other unit takes the polled state.
pub fn initial_state(db: &Db, unit: &aa_net::model::Unit) -> u8 {
    if unit.hp == 0 {
        5
    } else {
        damage_state(db, unit)
    }
}

/// Adopt `state` without a worker swap: every state but dying ends the dying window, and a living state starts a
/// new effect life (a later death may play the dead prefab's one-shots again).
fn adopt_state(damage: &mut VehicleDamage, state: u8) {
    damage.state = state;
    if state != 5 {
        damage.dying_start = None;
    }
    if !matches!(state, 5 | 6) {
        damage.dead_effect_played = false;
    }
}

/// The wheel model slot of a mount name: `wheel0`, `wheel12` take `wheel` (0), `wheel_type2`, `wheel_type21` take
/// `wheel2` (1). Other names (`bone_steeringwheel`, `wheel_l`) are not mounts.
pub fn wheel_slot(name: &str) -> Option<usize> {
    let name = name.strip_prefix('$').unwrap_or(name).to_ascii_lowercase();
    if let Some(rest) = name.strip_prefix("wheel_type2") {
        return rest.bytes().all(|b| b.is_ascii_digit()).then_some(1);
    }
    let rest = name.strip_prefix("wheel")?;
    (!rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit())).then_some(0)
}

/// The wheel mounts of a model (bones of a `.chr`, helper nodes of a `.cgf`), model space (cry).
fn mounts(model: &aa_cry::mesh::Model) -> Vec<(usize, Affine3A)> {
    let mut out: Vec<(usize, Affine3A)> = Vec::new();
    if let Some(skeleton) = &model.skeleton {
        out.extend(skeleton.bones.iter().filter_map(|b| Some((wheel_slot(&b.name)?, b.bind_model))));
    }
    if out.is_empty() {
        out.extend(model.nodes.iter().enumerate().filter_map(|(i, n)| Some((wheel_slot(&n.name)?, model.world_transform(i)))));
    }
    out
}

/// The wheel mounts of a vehicle's body in unit space (cry: x right, y forward, metres), as the wheels are
/// placed on them; the drive reads its ground-contact probes off these (`cart::model`).
pub(super) fn wheel_mounts(look: &VehicleLook) -> Vec<Vec3> {
    let mut out = Vec::new();
    for part in body_parts(look) {
        if let Ok(model) = aa_cry::mesh::Model::load(&part.model) {
            out.extend(mounts(&model).into_iter().map(|(_, mount)| Vec3::from((affine(&part) * mount).translation)));
        }
    }
    out
}

fn affine(part: &Part) -> Affine3A {
    Affine3A::from_scale_rotation_translation(part.scale, part.rot, part.pos)
}

/// The front and rear lever arms of `Bone_spindle` (`FUN_390cffd0`): the distances from the
/// spindle bone to the furthest front / rear wheel mount along the model's forward axis (cry Y).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpindleArms {
    pub front: f32,
    pub rear: f32,
}

/// A part of the body, or a wheel placed on a body mount.
struct Placed {
    part: Part,
    /// The wheel model slot (`wheel` = 0, `wheel2` = 1); `None` for body parts.
    wheel: Option<usize>,
    /// The lever arms when this body part carries `Bone_spindle`.
    spindle: Option<SpindleArms>,
}

fn spindle_arms(model: &aa_cry::mesh::Model) -> Option<SpindleArms> {
    let skeleton = model.skeleton.as_ref()?;
    let bone = skeleton
        .bones
        .iter()
        .find(|b| b.name.eq_ignore_ascii_case("Bone_spindle"))?;
    let y = bone.bind_model.translation.y;
    let mut front = f32::MIN;
    let mut rear = f32::MAX;
    for (_, mount) in mounts(model) {
        front = front.max(mount.translation.y);
        rear = rear.min(mount.translation.y);
    }
    let (front, rear) = (front - y, y - rear);
    (front >= 0.0 && rear >= 0.0).then_some(SpindleArms { front, rear })
}

/// The `$` helper nodes of a vehicle body in unit (Cry) space: `driver`, `passenger0`, ... A vehicle root has no
/// rider Animator, so `mount::seat_world` reads the prefab helper from here.
#[derive(Component, Default)]
pub struct SeatHelpers(pub HashMap<String, Affine3A>);

/// The helper key of a body bone or node: the attach strings name `$driver`, `$passenger0`, ..., but some vehicle
/// bodies (the red ox cart's `transfers_trailer_c.chr`) name the seat bones `driver` / `passenger0` without the `$`.
/// Both spellings are the same helper; other un-prefixed body bones are not helpers (capture-pending S11-C3 checks
/// the original helper lookup).
fn helper_key(name: &str) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    let stripped = lower.strip_prefix('$').unwrap_or(&lower);
    (lower.starts_with('$') || stripped.starts_with("driver") || stripped.starts_with("passenger"))
        .then(|| stripped.to_string())
}

fn helper_nodes(
    model: &aa_cry::mesh::Model,
    placement: Affine3A,
    out: &mut HashMap<String, Affine3A>,
) {
    // `$`-prefixed helpers first, so a bare `driver`/`passenger` bone never displaces an authored `$driver` node
    for prefixed in [true, false] {
        let pass = |name: &str| name.starts_with('$') == prefixed;
        if let Some(skeleton) = &model.skeleton {
            for bone in skeleton.bones.iter().filter(|bone| pass(&bone.name)) {
                if let Some(name) = helper_key(&bone.name) {
                    out.entry(name).or_insert(placement * bone.bind_model);
                }
            }
        }
        for (index, node) in model.nodes.iter().enumerate().filter(|(_, node)| pass(&node.name)) {
            if let Some(name) = helper_key(&node.name) {
                out.entry(name)
                    .or_insert_with(|| placement * model.world_transform(index));
            }
        }
    }
}

fn part_helpers(part: &Part, model: &aa_cry::mesh::Model) -> HashMap<String, Affine3A> {
    let mut helpers = HashMap::new();
    helper_nodes(model, affine(part), &mut helpers);
    helpers
}

/// The body helpers of `look` (reads the body models; used by the tests, [`prepare`] reuses the models it loads).
#[cfg(test)]
pub(super) fn body_helpers(look: &VehicleLook) -> HashMap<String, Affine3A> {
    let mut helpers = HashMap::new();
    for part in body_parts(look) {
        if let Ok(model) = aa_cry::mesh::Model::load(&part.model) {
            helper_nodes(&model, affine(&part), &mut helpers);
        }
    }
    helpers
}

fn body_parts(look: &VehicleLook) -> Vec<Part> {
    if look.normal.to_ascii_lowercase().starts_with("prefab://") {
        prefab::load(&look.normal)
    } else {
        model_file(&look.normal)
            .map(|model| {
                vec![Part {
                    model,
                    material: None,
                    pos: Vec3::ZERO,
                    rot: Quat::IDENTITY,
                    scale: Vec3::ONE,
                    animation: None,
                }]
            })
            .unwrap_or_default()
    }
}

/// The parts `look` is drawn with (body, then wheels) and the spindle arms of the body that has `Bone_spindle`.
fn assemble(look: &VehicleLook) -> Vec<Placed> {
    let mut out: Vec<Placed> = body_parts(look)
        .into_iter()
        .map(|part| Placed {
            part,
            wheel: None,
            spindle: None,
        })
        .collect();
    if look.wheels.iter().any(Option::is_some) {
        let mut wheels = Vec::new();
        for placed in &mut out {
            let Ok(model) = aa_cry::mesh::Model::load(&placed.part.model) else {
                continue;
            };
            placed.spindle = spindle_arms(&model);
            for (slot, mount) in mounts(&model) {
                let Some(wheel) = look.wheels[slot].as_ref().or(look.wheels[0].as_ref()) else {
                    continue;
                };
                let (scale, rot, pos) =
                    (affine(&placed.part) * mount).to_scale_rotation_translation();
                wheels.push(Placed {
                    part: Part {
                        model: wheel.clone(),
                        material: None,
                        pos,
                        rot: rot.normalize(),
                        scale,
                        animation: None,
                    },
                    wheel: Some(slot),
                    spindle: None,
                });
            }
        }
        out.extend(wheels);
    }
    out
}

/// The parts `look` is drawn with, relative to the unit's origin (cry space): the body parts, then the wheels.
pub fn parts(look: &VehicleLook) -> Vec<Part> {
    assemble(look)
        .into_iter()
        .map(|placed| placed.part)
        .collect()
}

/// The model-space bounds of every part of `look` (the slave summon space check).
pub fn bounds(look: &VehicleLook) -> Option<(Vec3, Vec3)> {
    let mut out: Option<(Vec3, Vec3)> = None;
    for part in parts(look) {
        let Ok(model) = aa_cry::mesh::Model::load(&part.model) else {
            warn!("vehicle: summon bounds: cannot load {}", part.model);
            continue;
        };
        let Some((lo, hi)) = model.bounds() else {
            warn!(
                "vehicle: summon bounds: no LOD 0 geometry in {}",
                part.model
            );
            continue;
        };
        let (a, b) = placed_bounds(affine(&part), lo, hi);
        out = Some(match out {
            None => (a, b),
            Some((lo, hi)) => (lo.min(a), hi.max(b)),
        });
    }
    out
}

/// The bounds of a model's eight corners after placing it in unit space.
fn placed_bounds(placement: Affine3A, lo: Vec3, hi: Vec3) -> (Vec3, Vec3) {
    let mut out: Option<(Vec3, Vec3)> = None;
    for i in 0..8 {
        let corner = Vec3::new(
            if i & 1 == 0 { lo.x } else { hi.x },
            if i & 2 == 0 { lo.y } else { hi.y },
            if i & 4 == 0 { lo.z } else { hi.z },
        );
        let p = placement.transform_point3(corner);
        out = Some(match out {
            None => (p, p),
            Some((a, b)) => (a.min(p), b.max(p)),
        });
    }
    out.unwrap_or((lo, hi))
}

/// A prepared part: a static model, or a rigged `.chr` (its models, spawned through [`super::creature::spawn`]).
pub struct Piece {
    part: Part,
    model: PieceModel,
    /// The wheel's radius (m) from its model's bounds (the axle is model-space Y: the smallest
    /// half extent), for a wheel part.
    wheel: Option<f32>,
    spindle: Option<SpindleArms>,
    helpers: HashMap<String, Affine3A>,
    /// Unit-space model bounds (cry axes) of this piece, for the summon emergence extent.
    bounds: Option<(Vec3, Vec3)>,
}

enum PieceModel {
    Static(PreparedModel),
    Rigged(Vec<PreparedModel>),
}

/// Vehicle materials are native `Illum` (the farm wagon's copper boiler uses the gloss map), so they need the Full
/// shading path rather than the world-prop Basic one.
fn model_options(material: Option<String>, cry_space: bool) -> ModelOptions {
    ModelOptions {
        material: material.filter(|m| aa_pak::Pak::global().exists(m)),
        cry_space,
        shading: Shading::Full,
        ..default()
    }
}

fn static_options(part: &Part) -> ModelOptions {
    model_options(part.material.clone(), false)
}

/// The rolling radius (m) of a wheel model: the largest half extent of its bounds. The wheel
/// models keep their axle on model-space Y (`transfers_trailer_a_wheel01`: x 1.036, y 0.332,
/// z 1.026 m), so the other two half extents are the radius.
fn wheel_radius(model: &aa_cry::mesh::Model) -> Option<f32> {
    let (lo, hi) = model.bounds()?;
    let radius = ((hi - lo) * 0.5).max_element();
    (radius > 0.01).then_some(radius)
}

/// Reads the models of `look` (callable from any thread). Parts whose model cannot be read are left out.
pub fn prepare(loader: &CryLoader, look: &VehicleLook) -> Vec<Piece> {
    assemble(look)
        .into_iter()
        .filter_map(
            |Placed {
                 part,
                 wheel,
                 spindle,
             }| {
                // a `.cdf` is a creature wrapper (several dying prefabs use one): creature::spawn reads it
                // as a rigged model, not a static mesh
                let rigged = part.model.ends_with(".cdf")
                    || (part.model.ends_with(".chr")
                        && aa_anim::pose::shared_rig(&part.model).is_ok_and(|r| !r.is_empty()));
                let (model, radius, helpers, bounds) = if rigged {
                    let models: Vec<PreparedModel> =
                        super::creature::model_files(&part.model, None)
                            .into_iter()
                            .filter_map(|(path, material)| {
                                loader.prepare_model(&path, &model_options(material, true)).ok()
                            })
                            .collect();
                    // a `.chr` part is its own main model: read the helpers off the prepared model, not a second load
                    let helpers = part_helpers(&part, &models.first()?.loaded().model);
                    let bounds = models
                        .iter()
                        .find_map(|m| m.loaded().model.bounds())
                        .map(|(lo, hi)| placed_bounds(affine(&part), lo, hi));
                    (PieceModel::Rigged(models), None, helpers, bounds)
                } else {
                    match loader.prepare_model(&part.model, &static_options(&part)) {
                        Ok(m) => {
                            let radius = wheel.and_then(|_| wheel_radius(&m.loaded().model));
                            let helpers = part_helpers(&part, &m.loaded().model);
                            let bounds = m.loaded().model.bounds().map(|(lo, hi)| placed_bounds(affine(&part), lo, hi));
                            (PieceModel::Static(m), radius, helpers, bounds)
                        }
                        Err(e) => {
                            debug!("vehicle part {}: {e}", part.model);
                            return None;
                        }
                    }
                };
                Some(Piece {
                    part,
                    model,
                    wheel: radius,
                    spindle,
                    helpers,
                    bounds,
                })
            },
        )
        .collect()
}

/// The model files [`prepare`] reads (the browser downloads them first).
pub fn model_files(look: &VehicleLook) -> Vec<(String, Option<String>)> {
    parts(look).into_iter().map(|p| (p.model, p.material)).collect()
}

/// A wheel's spin state (`FUN_390d6b10`): the holder rotates about the wheel model's axle
/// (model-space Y, which is holder-local -Z after [`cry_rot_to_bevy`]).
#[derive(Component, Clone, Copy, Debug)]
pub struct WheelSpin {
    /// The vehicle root the wheel belongs to.
    pub root: Entity,
    /// Rolling radius (m) from the wheel model's bounds.
    pub radius: f32,
    /// Angle (rad), advanced by `-forward_speed / radius * dt` and wrapped to 2π.
    pub angle: f32,
    /// The mount rotation the spin is added to.
    pub base: Quat,
}

/// The body's `Bone_spindle` (`FUN_390cffd0`) and the lever arms to its front and rear wheels.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Spindle {
    pub front: f32,
    pub rear: f32,
    /// The smoothed angle stored at model+0x3f8 in the original.
    pub angle: f32,
    /// The bone entity, the body's [`Animator`] and the bone index, found on first use.
    pub joint: Option<(Entity, Entity, usize)>,
}

/// The looping idle of a prefab body (`X2::PrefabUnitModel::UpdateIdleAnim`, `FUN_390d6690`): the
/// body's `fist_ba_relaxed_idle` runs while the model exists, whatever its driver or speed. The 1.2
/// callers skip it in prefab damage state 5 (dying) or 6 (dead): `FUN_390e69d0` (DEC 184925),
/// `FUN_390eb7d0` (DEC 187546), `FUN_390ebaa0` (DEC 187601) and the interaction "opened" path
/// `FUN_390d64a0` (DEC 173191). This branch has no dying state yet (PR #69), so it gates on [`Dead`]
/// (state 6). On the body's Animator; [`start_vehicle_idle`] starts it whenever the model has no clip.
/// A destroyed vehicle stops it and plays the `.cal`'s `dead_01`/`dead_02` ([`stop_dead_vehicles`]).
#[derive(Component, Clone, Debug)]
pub struct VehicleIdle {
    pub clip: BoundClip,
    /// The body's death clip (`dead_01`, else `dead_02`), played once when the unit dies.
    pub dead: Option<BoundClip>,
    /// The unit is dead (native: damage state 5 or 6, but only state 6 exists here until PR #69): the
    /// idle stays stopped until a resurrection clears [`Dead`].
    pub stopped: bool,
}

/// Starts the prefab body's idle when the model has no clip, as 1.2 does on model init and refresh.
/// `UpdateIdleAnim` itself has no driver or speed condition; its 1.2 callers skip it in prefab damage
/// state 5 (dying) or 6 (dead) - not for the Mate and Shipyard model kinds. This branch gates on
/// [`Dead`] (state 6) until PR #69 adds dying; ships and land vehicles get the loop.
pub fn start_vehicle_idle(mut bodies: Query<(&VehicleIdle, &mut Animator)>) {
    for (idle, mut animator) in &mut bodies {
        if !idle.stopped && animator.starts_enabled() && animator.current.is_none() {
            animator.play(idle.clip.clone(), true, 0.0);
        }
    }
}

/// Stops one prefab body's idle and plays its `.cal` death clip.
fn stop_vehicle_idle(animator: &mut Animator, idle: &mut VehicleIdle) {
    if idle.stopped {
        return;
    }
    idle.stopped = true;
    animator.stop_layer(0.0);
    animator.random_idle = None;
    match idle.dead.clone() {
        Some(dead) => {
            animator.speed = 1.0;
            animator.play(dead, false, 0.0);
        }
        // no death clip: leave the body on its last pose rather than restarting the idle
        None => animator.current = None,
    }
}

/// Stops a destroyed vehicle's idle (the prefab body's Animator is a child of the root that carries [`Dead`],
/// which `combat::ensure_corpses` does not visit).
fn stop_dead_vehicles(
    dead: Query<Entity, Added<Dead>>,
    children: Query<&Children>,
    mut bodies: Query<(&mut Animator, &mut VehicleIdle)>,
) {
    for root in &dead {
        let Ok(kids) = children.get(root) else { continue };
        for child in kids.iter() {
            let Ok((mut animator, mut idle)) = bodies.get_mut(child) else { continue };
            stop_vehicle_idle(&mut animator, &mut idle);
        }
    }
}

/// A body that appears after its unit was announced dead (a corpse in view) is stopped as soon as it exists.
fn stop_dead_new_bodies(
    mut bodies: Query<(&mut Animator, &mut VehicleIdle, &ChildOf), Added<VehicleIdle>>,
    parents: Query<&Dead>,
) {
    for (mut animator, mut idle, parent) in &mut bodies {
        if parents.contains(parent.parent()) {
            stop_vehicle_idle(&mut animator, &mut idle);
        }
    }
}

/// A resurrected vehicle starts its idle again (the server removed [`Dead`]).
fn resume_vehicle_idle(
    mut removed: RemovedComponents<Dead>,
    children: Query<&Children>,
    mut bodies: Query<(&mut Animator, &mut VehicleIdle)>,
) {
    for root in removed.read() {
        let Ok(kids) = children.get(root) else { continue };
        for child in kids.iter() {
            let Ok((mut animator, mut idle)) = bodies.get_mut(child) else { continue };
            if !idle.stopped {
                continue;
            }
            idle.stopped = false;
            animator.current = None;
        }
    }
}

/// Registers the vehicle systems. Add once.
pub(crate) struct VehiclePlugin;

impl Plugin for VehiclePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SummonPortals>()
            .add_systems(
                Update,
                (stop_dead_vehicles, stop_dead_new_bodies, resume_vehicle_idle, start_vehicle_idle).chain(),
            )
            .add_systems(
                Update,
                (
                    track_summon_portals,
                    // the portal waits for the frame `units::sync_views` draws the slave's root
                    spawn_summon_portals.after(super::units::sync_views),
                    // the emergence drives the root after the server position, like the 1.2 unit state does
                    animate_summon_emergence.after(super::units::move_units),
                    apply_emergence_clip,
                    end_summon_portals,
                    despawn_summon_portal_effects,
                    restore_emergence_clip,
                    cleanup_summon_portals,
                )
                    .chain(),
            )
            // the spindle turns the animated pose, not the bind pose: run after the clips write the bones
            .add_systems(
                PostUpdate,
                steer_spindles.after(AnimSet).before(TransformSystems::Propagate),
            );
    }
}

/// The motion of a vehicle root, measured from its transform every frame: the wheel and spindle
/// inputs for a driven cart and for remote carts whose position is played back.
#[derive(Component, Clone, Copy, Debug)]
pub struct VehicleMotion {
    last_pos: Vec3,
    last_yaw: f32,
    /// World horizontal velocity (Bevy space, m/s).
    pub velocity: Vec3,
    /// Yaw rate about +Y (rad/s), positive to the left.
    pub yaw_rate: f32,
    /// Speed along the root's forward (-Z), signed (negative in reverse).
    pub forward_speed: f32,
}

impl VehicleMotion {
    pub(crate) fn at(transform: &Transform) -> Self {
        Self {
            last_pos: transform.translation,
            last_yaw: transform.rotation.to_euler(EulerRot::YXZ).0,
            velocity: Vec3::ZERO,
            yaw_rate: 0.0,
            forward_speed: 0.0,
        }
    }
}

/// The wheel angle after `dt` (`FUN_390d6b10`, non-physics branch): `angle -= forward_speed /
/// radius * dt`, wrapped to 2π.
pub fn wheel_angle(angle: f32, forward_speed: f32, radius: f32, dt: f32) -> f32 {
    if radius <= 1e-4 || dt <= 0.0 {
        return angle;
    }
    wrap_angle(angle - forward_speed / radius * dt)
}

/// The spindle angle `FUN_390cffd0` asks for: with the turn radius `R = |v_forward| / |yaw rate|`,
/// the mean of `atan(front / R)` and `atan(rear / R)`, clamped to [`SPINDLE_MAX_ANGLE`] and signed
/// by the direction of the turn (so it flips in reverse). Without yaw rate it is 0; at a standstill
/// the yaw rate over [`SPINDLE_YAW_LIMIT`] stands in.
pub fn spindle_angle(forward_speed: f32, yaw_rate: f32, front: f32, rear: f32) -> f32 {
    if yaw_rate.abs() < 1e-6 {
        return 0.0;
    }
    // reverse flips the sign; at a standstill the yaw rate alone gives it
    let sign = if forward_speed * yaw_rate < 0.0 || (forward_speed == 0.0 && yaw_rate < 0.0) {
        -1.0
    } else {
        1.0
    };
    let radius = forward_speed.abs() / yaw_rate.abs();
    let magnitude = if radius <= 1e-4 {
        (yaw_rate.abs() / SPINDLE_YAW_LIMIT).min(1.0) * SPINDLE_MAX_ANGLE
    } else {
        (((front.abs() / radius).atan() + (rear.abs() / radius).atan()) * 0.5)
            .min(SPINDLE_MAX_ANGLE)
    };
    sign * magnitude
}

/// One frame of the stored spindle angle: `current + (target - current) * min(dt, 0.1)`.
pub fn spindle_step(current: f32, target: f32, dt: f32) -> f32 {
    current + (target - current) * dt.clamp(0.0, SPINDLE_LERP)
}

/// Adds a piece's `$` helpers: the first piece to name one keeps it, so the body's helpers win over a wheel part's.
fn merge_helpers(helpers: &mut HashMap<String, Affine3A>, piece: HashMap<String, Affine3A>) {
    for (name, helper) in piece {
        helpers.entry(name).or_insert(helper);
    }
}

/// Spawns the prepared parts under a new root at `transform`.
pub fn spawn(
    commands: &mut Commands,
    cry: &mut CryAssets,
    clips: &ClipLibrary,
    pieces: Vec<Piece>,
    transform: Transform,
) -> Entity {
    let extent = piece_extent(&pieces);
    let root = commands
        .spawn((
            transform,
            Visibility::default(),
            VehicleMotion::at(&transform),
            VehicleExtent(extent),
        ))
        .id();
    add_parts(commands, cry, clips, pieces, root);
    root
}

/// The unit-space bounding extent of a vehicle's prepared parts: `FUN_3909ea30` reads the model's `[bounds+0x28]`
/// and the summon emergence puts the clip plane there, with the start another extent behind it. This uses the
/// largest horizontal half extent of the combined bounds (cry x / y).
fn piece_extent(pieces: &[Piece]) -> f32 {
    let mut lo = Vec3::splat(f32::MAX);
    let mut hi = Vec3::splat(f32::MIN);
    let mut any = false;
    for piece in pieces {
        if let Some((a, b)) = piece.bounds {
            lo = lo.min(a);
            hi = hi.max(b);
            any = true;
        }
    }
    if !any {
        return 1.0;
    }
    let size = hi - lo;
    (size.x.max(size.y) * 0.5).clamp(0.25, 50.0)
}

/// Spawns the prepared parts as children of `root` and installs the seat helpers and spindle they define.
/// Used for the initial look and for every damage-state body swap; the caller removes the old `Spindle`
/// and the old [`VehiclePart`] children first.
pub fn add_parts(commands: &mut Commands, cry: &mut CryAssets, clips: &ClipLibrary, pieces: Vec<Piece>, root: Entity) {
    let mut helpers = HashMap::new();
    for Piece {
        part,
        model,
        wheel,
        spindle,
        helpers: piece_helpers,
        bounds: _,
    } in pieces
    {
        // the body parts come first: a same-named helper on a wheel part never takes the driver's seat
        merge_helpers(&mut helpers, piece_helpers);
        if let Some(arms) = spindle {
            commands.entity(root).insert(Spindle {
                front: arms.front,
                rear: arms.rear,
                ..default()
            });
        }
        let placed = Transform {
            translation: cry_to_bevy(part.pos),
            rotation: cry_rot_to_bevy(part.rot),
            scale: Vec3::new(part.scale.x, part.scale.z, part.scale.y),
        };
        let entity = match model {
            PieceModel::Static(prepared) => {
                let options = static_options(&part);
                let loaded = cry.finish_model(prepared);
                let model = spawn_loaded(commands, &part.model, &loaded, &options);
                let holder = commands.spawn((placed, Visibility::default())).id();
                commands.entity(model).insert(ChildOf(holder));
                if let Some(radius) = wheel {
                    commands.entity(holder).insert(WheelSpin {
                        root,
                        radius,
                        angle: 0.0,
                        base: placed.rotation,
                    });
                }
                Some(holder)
            }
            PieceModel::Rigged(models) => {
                for m in models {
                    cry.finish_model(m);
                }
                // the vehicle path owns the idle (the 1.2 `UpdateIdleAnim` rule), not the creature spawn;
                // an `AnimObject` that names a playing animation (the dying prefab's `dead_01`) starts that
                // instead (`FUN_39109d20` opens the Animation child and reads `bPlaying`; the clip name and
                // loop are engine AnimObject behaviour).
                let spawned = super::creature::spawn_with_idle(
                    commands,
                    cry,
                    clips,
                    &part.model,
                    None,
                    placed,
                    false,
                    part.animation.as_ref(),
                );
                if let Some(bodies) = &spawned
                    && let Some(idle) = &bodies.idle
                {
                    commands.entity(bodies.root).insert(VehicleIdle {
                        clip: idle.clone(),
                        dead: bodies.dead.clone(),
                        // a one-shot authored animation (dead_01) stays on its last frame; the idle must not replace it
                        stopped: bodies.authored == Some(false),
                    });
                }
                spawned.map(|spawned| spawned.root)
            }
        };
        if let Some(e) = entity {
            commands.entity(e).insert((ChildOf(root), VehiclePart));
        }
    }
    commands.entity(root).insert(SeatHelpers(helpers));
}

/// The prefab's speed-driven emitters. `FUN_390eaf40` collects the type-5 particles named exactly `splash_l`,
/// `splash_r` (the bubble list at +0x2f0) and `exhaust` (the list at +0x2c0); `FUN_390c78d0` scales them from
/// the ship's speed and angular velocity. This slice does not draw them, so a swap leaves them out instead of
/// showing a wake at a standstill. Other active prefab particles are ordinary authored emitters.
fn is_speed_emitter(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    matches!(
        name.as_str(),
        "exhaust" | "splash_l" | "splash_r" | "bubble"
    )
}

fn spawn_effect(commands: &mut Commands, root: Entity, effect: &aa_world::placement::PlacedEffect, one_shot: bool) -> Entity {
    let mut command = commands.spawn((
        aa_fx::Effect {
            scale: effect.scale,
            speed_scale: effect.speed_scale,
            count_scale: effect.count_scale,
            prime: if one_shot { false } else { effect.prime },
            despawn_when_finished: true,
            no_body: true,
            ..aa_fx::Effect::new(effect.effect.clone())
        },
        Transform {
            translation: cry_to_bevy(effect.pos),
            rotation: cry_rot_to_bevy(effect.rot),
            ..default()
        },
        VehicleEffect,
    ));
    let entity = command.id();
    command.insert(ChildOf(root));
    entity
}

/// Whether a swap plays the new prefab's one-shot `_spawn_effect*` elements (`FUN_39109120`). The Die path
/// always passes 1 to `FUN_390e7910`, so entering dying (5) does too, even on the 6 -> 5 route of an HP-0 update
/// followed by the death packet. A worsening state plays them, except that the dead prefab's one-shots are not
/// replayed when that death already showed 6 through the poller.
fn plays_state_effects(old: u8, state: u8, dead_effect_played: bool) -> bool {
    state == 5 || (state > old && !(state == 6 && dead_effect_played))
}

/// Spawns the particle entities of a damage variant: its active standing effects (damaged fire, dead
/// smoke) and, when [`plays_state_effects`] says so, its `_spawn_effect*` and `_dead_effect` bursts
/// (`FUN_39109120`). A unit that arrives already dead only gets the standing smoke, not the explosion. The
/// `_despawn_effect` trigger has no traced call site and is left out.
pub fn spawn_state_effects(commands: &mut Commands, root: Entity, variant: &str, worsening: bool) {
    if !variant.to_ascii_lowercase().starts_with("prefab://") {
        return;
    }
    for named in super::data::prefab_effects_named(variant) {
        let name = named.name.to_ascii_lowercase();
        if name.starts_with("_spawn_effect") {
            if worsening {
                spawn_effect(commands, root, &named.effect, true);
            }
            continue;
        }
        if name.starts_with("_dead_effect") && !worsening {
            continue;
        }
        if name.starts_with("_despawn_effect") || !named.active || is_speed_emitter(&name) {
            continue;
        }
        spawn_effect(commands, root, &named.effect, false);
    }
}

/// The fire the damaged ship prefab was already showing when it died. The 2026-11-02 capture keeps the
/// aft-deck fire burning through the ~12 s sink (`ship_sinking_t358.66`, `ship_sunk_burning_t365`), while
/// the dead prefab (`galleon.dead`) names only the water-destroy burst. A ship swap into dead therefore
/// re-spawns the previous prefab's active fire emitters at the same authored anchors. The original
/// client almost certainly keeps the old prefab's live particle entities instead; the target poses and
/// effect are the damaged prefab's, so the result matches the capture (inferred).
fn carry_ship_fire(commands: &mut Commands, root: Entity, previous: &str) {
    if previous.to_ascii_lowercase().starts_with("prefab://") {
        for named in super::data::prefab_effects_named(previous) {
            let name = named.name.to_ascii_lowercase();
            if named.active && name.contains("fire") && !name.starts_with('_') {
                spawn_effect(commands, root, &named.effect, false);
            }
        }
    }
}

/// The state check of `FUN_390eb610`: while the unit is dying (5) it only checks the dying window
/// (`FUN_390eaac0`); otherwise the unit's health picks the state. A death (`SCUnitDeath`, i.e. `unit.dead`, or
/// an announcement at 0 hp) enters dying first (`FUN_390e7ad0`), even when an earlier HP-0 update already showed
/// the dead prefab. The new variant's prefab is read on the workers (`FUN_390e7b20` -> `FUN_390e7910`); nothing
/// is removed yet, [`finish_damage_swaps`] applies the swap once the models are ready. A state whose row names
/// the same prefab only updates the state, as `FUN_390e7910` returns before the brush swap ("same model").
pub fn update_damage(
    db: Option<Res<Db>>,
    units: Option<Res<Units>>,
    mut cry: CryAssets,
    swaps: ResMut<VehicleSwaps>,
    time: Res<Time>,
    mut roots: Query<(Entity, &RemoteUnit, &mut VehicleDamage)>,
) {
    let (Some(db), Some(units)) = (db, units) else {
        return;
    };
    let now = time.elapsed_secs_f64();
    for (root, remote, mut damage) in &mut roots {
        let Some(unit) = units.by_id.get(&remote.obj_id) else {
            continue;
        };
        if unit.hp > 0 && !unit.dead {
            damage.death_seen = false;
        }
        let mut started = false;
        if unit.dead && !damage.death_seen {
            damage.death_seen = true;
            damage.dying_start = Some(now);
            started = true;
            debug!("vehicle {}: entering dying (5)", remote.obj_id);
        } else if damage.state == 5 && damage.dying_start.is_none() {
            // built already dead at 0 hp (`InitClientUnit` calls Die), so its dying window starts now
            damage.death_seen = true;
            damage.dying_start = Some(now);
            started = true;
        }
        let state = match damage.dying_start {
            // `FUN_390eaac0`: `dying_time <= elapsed` switches to dead; 0 lasts the entry tick only
            Some(start) if !started && now - start >= f64::from(damage.dying_time) => {
                debug!(
                    "vehicle {}: dying expired after {:.3} s",
                    remote.obj_id, damage.dying_time
                );
                6
            }
            Some(_) => 5,
            None => model_state(unit.hp, damage.max_hp),
        };
        if state == damage.state || damage.requested == Some(state) {
            continue;
        }
        let Some(look) = db.vehicle_look(unit.model_id) else {
            adopt_state(&mut damage, state);
            continue;
        };
        let Some(variants) = db.vehicle_variants(unit.model_id) else {
            adopt_state(&mut damage, state);
            continue;
        };
        let variant = variants.for_state(state).to_string();
        if variant == damage.variant {
            adopt_state(&mut damage, state);
            continue;
        }
        debug!(
            "vehicle {}: state {} -> {state} ({variant})",
            remote.obj_id, damage.state
        );
        damage.requested = Some(state);
        let look = VehicleLook {
            normal: variant.clone(),
            wheels: look.wheels,
        };
        let loader = cry.loader();
        let tx = swaps.tx.clone();
        let priority = 0.0;
        let obj_id = remote.obj_id;
        Workers::global().spawn(priority, move || {
            // the browser downloads the models, then their textures, before reading them; natively this runs straight on
            let files = if cfg!(target_arch = "wasm32") {
                let mut files: Vec<String> = model_files(&look)
                    .iter()
                    .flat_map(|(path, material)| {
                        aa_cry::deps::model_files(path, material.as_deref())
                    })
                    .collect();
                files.sort();
                files.dedup();
                files
            } else {
                Vec::new()
            };
            Workers::global().then_after(priority, files.clone(), move || {
                let textures = aa_cry::deps::texture_files(&files);
                Workers::global().then_after(priority, textures, move || {
                    let pieces = prepare(&loader, &look);
                    let _ = tx.send(VehicleSwapBuilt {
                        root,
                        obj_id,
                        state,
                        variant,
                        pieces,
                    });
                });
            });
        });
    }
}

/// Applies the swaps the workers finished: only now are the old parts and effects despawned (the state may have
/// moved on meanwhile, in which case the newer request wins and this build is dropped). Installs the new parts,
/// resets the spindle and the turret bone cache (`TurretGun`; the new skeleton's `bone_yaw` / `bone_pitch` are
/// found again by `turret::aim_turrets`), and plays the new prefab's `_spawn_effect*` when it worsened.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn finish_damage_swaps(
    mut commands: Commands,
    units: Option<Res<Units>>,
    db: Option<Res<Db>>,
    mut cry: CryAssets,
    clips: Res<ClipLibrary>,
    swaps: ResMut<VehicleSwaps>,
    time: Res<Time>,
    mut roots: Query<(Entity, &mut VehicleDamage, &Transform)>,
    stale: Query<(Entity, &ChildOf), Or<(With<VehiclePart>, With<VehicleEffect>)>>,
) {
    let mut done = Vec::new();
    {
        let rx = swaps.rx.lock().unwrap_or_else(|e| e.into_inner());
        while let Ok(built) = rx.try_recv() {
            done.push(built);
        }
    }
    let now = time.elapsed_secs_f64();
    for built in done {
        let Ok((root, mut damage, pose)) = roots.get_mut(built.root) else {
            continue;
        };
        if damage.requested != Some(built.state) {
            continue;
        }
        // the health may have moved on (damage then a heal) while the models were read: keep the old,
        // correct look and let the next frame request the current state. While dying, HP does not drive
        // the state; the dying window decides between 5 and 6.
        let desired = match damage.dying_start {
            Some(start) if now - start >= f64::from(damage.dying_time) => 6,
            Some(_) => 5,
            None => units
                .as_deref()
                .and_then(|units| units.by_id.get(&built.obj_id))
                .map_or(built.state, |unit| model_state(unit.hp, damage.max_hp)),
        };
        if desired != built.state {
            damage.requested = None;
            continue;
        }
        if built.pieces.is_empty() {
            warn!(
                "vehicle {}: the state {} prefab {} has no parts; keeping the old body",
                built.root, built.state, built.variant
            );
            adopt_state(&mut damage, built.state);
            damage.requested = None;
            continue;
        }
        let old = damage.state;
        let effects = plays_state_effects(old, built.state, damage.dead_effect_played);
        for (entity, parent) in &stale {
            if parent.parent() == root {
                commands.entity(entity).try_despawn();
            }
        }
        commands.entity(root).remove::<Spindle>();
        commands.entity(root).remove::<super::turret::TurretGun>();
        add_parts(&mut commands, &mut cry, &clips, built.pieces, root);
        spawn_state_effects(&mut commands, root, &built.variant, effects);
        let ship = units
            .as_deref()
            .and_then(|units| units.by_id.get(&built.obj_id))
            .zip(db.as_deref())
            .is_some_and(|(unit, db)| db.is_ship(unit.model_id));
        if built.state == 6 {
            damage.dead_effect_played = true;
            if ship {
                // the capture keeps the damaged prefab's fire burning through the sink
                carry_ship_fire(&mut commands, root, &damage.variant);
                commands.entity(root).insert(WreckSink::at(pose, now));
            }
        }
        adopt_state(&mut damage, built.state);
        damage.variant = built.variant;
        damage.requested = None;
    }
}

/// Measures each vehicle's motion from its root transform: the position and yaw change since the
/// last frame, in the root's forward frame. A jump over [`TELEPORT_STEP`] is a spawn or teleport,
/// not motion.
pub fn track_motion(time: Res<Time>, mut roots: Query<(&mut VehicleMotion, &Transform)>) {
    let dt = time.delta_secs().min(0.1);
    for (mut motion, transform) in &mut roots {
        let yaw = transform.rotation.to_euler(EulerRot::YXZ).0;
        let delta = transform.translation - motion.last_pos;
        let teleport = Vec2::new(delta.x, delta.z).length() > TELEPORT_STEP;
        let velocity = if dt > 0.0 && !teleport {
            Vec3::new(delta.x, 0.0, delta.z) / dt
        } else {
            Vec3::ZERO
        };
        let yaw_rate = if dt > 0.0 && !teleport {
            wrap_angle(yaw - motion.last_yaw) / dt
        } else {
            0.0
        };
        let mut forward = transform.rotation * Vec3::NEG_Z;
        forward.y = 0.0;
        let forward = forward.normalize_or_zero();
        motion.velocity = velocity;
        motion.yaw_rate = yaw_rate;
        motion.forward_speed = velocity.dot(forward);
        motion.last_pos = transform.translation;
        motion.last_yaw = yaw;
    }
}

/// A root step longer than this (m) is a teleport or a fresh spawn; its wheels do not spin.
const TELEPORT_STEP: f32 = 2.0;

/// Spins every wheel by its root's forward speed (`FUN_390d6b10`).
pub fn spin_wheels(
    time: Res<Time>,
    motion: Query<&VehicleMotion>,
    mut wheels: Query<(&mut WheelSpin, &mut Transform)>,
) {
    let dt = time.delta_secs().min(0.1);
    for (mut wheel, mut transform) in &mut wheels {
        let Ok(motion) = motion.get(wheel.root) else {
            continue;
        };
        wheel.angle = wheel_angle(wheel.angle, motion.forward_speed, wheel.radius, dt);
        transform.rotation = wheel.base * Quat::from_axis_angle(Vec3::NEG_Z, wheel.angle);
    }
}

/// Turns the body's `Bone_spindle` by [`spindle_angle`] and [`spindle_step`] (`FUN_390cffd0`), over the rotation
/// the idle has just sampled (PostUpdate, after [`AnimSet`]): the same local Z turn on the bind pose would be
/// overwritten every frame by the animation.
pub fn steer_spindles(
    time: Res<Time>,
    motion: Query<&VehicleMotion>,
    mut spindles: Query<(Entity, &mut Spindle, &Children)>,
    animators: Query<&Animator>,
    mut joints: Query<&mut Transform, Without<Spindle>>,
) {
    let dt = time.delta_secs().min(0.1);
    for (entity, mut spindle, children) in &mut spindles {
        if spindle.joint.is_none() {
            for child in children.iter() {
                let Ok(animator) = animators.get(child) else {
                    continue;
                };
                let Some(index) = animator
                    .rig
                    .skeleton
                    .find("Bone_spindle")
                    .or_else(|| animator.rig.skeleton.find_ignore_case("Bone_spindle"))
                else {
                    continue;
                };
                let Some(&joint) = animator.bones.get(index) else {
                    continue;
                };
                spindle.joint = Some((joint, child, index));
                break;
            }
        }
        let (Some((joint, animator_entity, index)), Ok(motion)) = (spindle.joint, motion.get(entity)) else {
            continue;
        };
        let target = spindle_angle(
            motion.forward_speed,
            motion.yaw_rate,
            spindle.front,
            spindle.rear,
        );
        spindle.angle = spindle_step(spindle.angle, target, dt);
        let Ok(animator) = animators.get(animator_entity) else {
            continue;
        };
        let Some(animated) = animator.pose().rot.get(index).copied() else {
            continue;
        };
        if let Ok(mut transform) = joints.get_mut(joint) {
            transform.rotation = animated * Quat::from_rotation_z(spindle.angle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::SystemState;

    #[test]
    fn vehicle_parts_request_full_illum_shading() {
        let part = Part {
            model: "objects/env/06_unit/05_carriage/transfers_trailer_a_body.chr".into(),
            material: None,
            pos: Vec3::ZERO,
            rot: Quat::IDENTITY,
            scale: Vec3::ONE,
            animation: None,
        };
        assert_eq!(static_options(&part).shading, Shading::Full);
        let rigged = model_options(None, true);
        assert_eq!(rigged.shading, Shading::Full, "the body .chr keeps the same Full Illum path");
        assert!(rigged.cry_space);
    }

    #[test]
    fn the_body_helper_wins_over_a_same_named_wheel_helper() {
        let body = Affine3A::from_translation(Vec3::new(-0.02, -0.50, 1.35));
        let wheel = Affine3A::from_translation(Vec3::new(9.0, 9.0, 9.0));
        let mut helpers = HashMap::new();
        merge_helpers(&mut helpers, HashMap::from([("driver".to_string(), body)]));
        merge_helpers(&mut helpers, HashMap::from([("driver".to_string(), wheel), ("wheel0".to_string(), wheel)]));
        assert_eq!(helpers["driver"], body, "the body's $driver helper wins");
        assert_eq!(helpers["wheel0"], wheel, "the wheel's own helper is kept");
    }

    #[test]
    #[ignore = "requires the client pak"]
    fn the_wagon_body_exposes_its_driver_helper() {
        let look = VehicleLook {
            normal: "prefab://prefabs/vehicle_tractor.xml/vehicle_tractor.tractor_body".into(),
            wheels: [
                Some("objects/env/06_unit/05_carriage/transfers_trailer_a_wheel01.cgf".into()),
                Some("objects/env/06_unit/05_carriage/transfers_trailer_a_wheel02.cgf".into()),
            ],
        };
        let helpers = body_helpers(&look);
        let driver = helpers.get("driver").expect("the body's $driver helper");
        let expected = Vec3::new(-0.02, -0.50, 1.35);
        assert!(
            driver.translation.distance(expected.into()) < 0.01,
            "driver at {:?}",
            driver.translation
        );
    }

    #[test]
    fn helper_names_match_with_or_without_the_dollar_prefix() {
        assert_eq!(helper_key("$driver").as_deref(), Some("driver"));
        assert_eq!(helper_key("$Passenger3").as_deref(), Some("passenger3"));
        assert_eq!(
            helper_key("driver").as_deref(),
            Some("driver"),
            "the red ox cart names it without $"
        );
        assert_eq!(helper_key("passenger0").as_deref(), Some("passenger0"));
        assert_eq!(helper_key("Bone_spindle"), None);
        assert_eq!(helper_key("dummy_passenger"), None);
    }

    /// The red ox cart's body names its two seats without the `$` the attach strings use; with only a `$` prefix scan
    /// its passenger seat fell back to the cart origin.
    #[test]
    #[ignore = "requires the client pak and the game database"]
    fn the_red_ox_cart_exposes_both_seat_helpers() {
        let Some(db) = super::super::data::Db::open() else { return };
        let look = db.vehicle_look(1553).expect("the red ox cart's look");
        let helpers = body_helpers(&look);
        let driver = helpers.get("driver").expect("the cart's driver helper");
        let passenger = helpers.get("passenger0").expect("the cart's passenger helper");
        assert!(driver.translation.length() > 0.1);
        assert!(
            passenger.translation.distance(driver.translation.into()) > 0.1,
            "the passenger and driver helpers are different places"
        );
    }

    #[test]
    fn wheel_mounts_are_named_wheel_n_and_wheel_type2_n() {
        assert_eq!(wheel_slot("wheel0"), Some(0));
        assert_eq!(wheel_slot("Wheel12"), Some(0));
        assert_eq!(wheel_slot("$wheel1"), Some(0));
        assert_eq!(wheel_slot("wheel_type2"), Some(1));
        assert_eq!(wheel_slot("wheel_type21"), Some(1));
        assert_eq!(wheel_slot("wheel"), None);
        assert_eq!(wheel_slot("bone_steeringwheel"), None);
        assert_eq!(wheel_slot("wheel_l"), None);
    }

    #[test]
    #[ignore = "requires the client pak"]
    fn the_airship_has_its_moving_parts_and_a_cart_its_wheels() {
        let airship = parts(&VehicleLook { normal: "prefab://Prefabs/airship.xml/airship.normal".into(), wheels: [None, None] });
        let models: Vec<&str> = airship.iter().map(|p| p.model.as_str()).collect();
        assert!(models.contains(&"objects/env/06_unit/02_airship/airship_base.cgf"), "{models:?}");
        assert!(models.contains(&"objects/env/06_unit/02_airship/airship_propeller_l.chr"), "{models:?}");
        assert_eq!(models.len(), 7, "{models:?}");

        let wheel = "objects/env/06_unit/05_carriage/transfers_trailer_a_wheel01.cgf";
        let wheel2 = "objects/env/06_unit/05_carriage/transfers_trailer_a_wheel02.cgf";
        let tractor_look = VehicleLook {
            normal: "prefab://prefabs/vehicle_tractor.xml/vehicle_tractor.tractor_body".into(),
            wheels: [Some(wheel.into()), Some(wheel2.into())],
        };
        let tractor: Vec<Part> = parts(&tractor_look);
        let wheels: Vec<&Part> = tractor.iter().filter(|p| p.model == wheel || p.model == wheel2).collect();
        assert_eq!(wheels.len(), 4, "{tractor:?}");
        assert_eq!(wheels.iter().filter(|p| p.model == wheel2).count(), 2);
        // left and right of the body
        assert!(wheels.iter().any(|p| p.pos.x < -0.3) && wheels.iter().any(|p| p.pos.x > 0.3), "{wheels:?}");

        // the body's Bone_spindle sits between the front (y 1.61) and rear (y -2.545) mounts
        let arms = assemble(&tractor_look)
            .iter()
            .find_map(|p| p.spindle)
            .expect("Bone_spindle");
        assert!((arms.front - 2.389).abs() < 0.01, "{arms:?}");
        assert!((arms.rear - 1.766).abs() < 0.01, "{arms:?}");

        let front = aa_cry::mesh::Model::load(wheel).unwrap();
        let rear = aa_cry::mesh::Model::load(wheel2).unwrap();
        assert!((wheel_radius(&front).unwrap() - 0.518).abs() < 0.01);
        assert!((wheel_radius(&rear).unwrap() - 0.885).abs() < 0.01);
    }

    /// A spawned Farm Wagon (model 1008) and its root, with the vehicle systems ticking.
    fn wagon() -> Option<(App, Entity)> {
        use bevy::time::TimeUpdateStrategy;
        if !aa_cry::pak_available() {
            return None; // no client pak
        }
        let db = super::super::data::Db::open()?;
        let look = db.vehicle_look(1008).expect("the Farm Wagon's look");
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            bevy::asset::AssetPlugin::default(),
            aa_cry::CryPlugin,
            aa_anim::AnimPlugin,
            VehiclePlugin,
        ));
        app.init_asset::<Mesh>()
            .init_asset::<Image>()
            .init_asset::<StandardMaterial>()
            .init_asset::<bevy::mesh::skinning::SkinnedMeshInverseBindposes>()
            .insert_resource(TimeUpdateStrategy::ManualDuration(std::time::Duration::from_secs_f32(1.0 / 30.0)));
        let mut state = SystemState::<(Commands, CryAssets, Res<ClipLibrary>)>::new(app.world_mut());
        let pieces = {
            let (_, mut cry, _) = state.get_mut(app.world_mut()).unwrap();
            prepare(&cry.loader(), &look)
        };
        let root = {
            let (mut commands, mut cry, clips) = state.get_mut(app.world_mut()).unwrap();
            spawn(&mut commands, &mut cry, &clips, pieces, Transform::default())
        };
        state.apply(app.world_mut());
        app.update();
        Some((app, root))
    }

    /// A vehicle root (model `model`) for slave template `template`, announced in `Units` at the origin with the
    /// summon portal/emergence systems ticking. The tests send the `SCSlaveCreatedPacket` themselves.
    fn vehicle_app(template: u32, model: u32) -> Option<(App, Db, Entity)> {
        use bevy::time::TimeUpdateStrategy;
        if !aa_cry::pak_available() {
            return None;
        }
        let db = Db::open()?;
        let book = super::super::combat::skills::SkillBook::open()?;
        let look = db.vehicle_look(model).expect("the vehicle's look");
        let max_hp = db.slave_max_hp(template, 50).expect("the slave's MaxHealth");
        let unit = slave_unit_kind(7, template, template, model, max_hp, max_hp);
        let mut units = Units::default();
        units.apply(&aa_net::Event::UnitSpawned(Box::new(unit.clone())));
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            bevy::asset::AssetPlugin::default(),
            aa_cry::CryPlugin,
            aa_anim::AnimPlugin,
        ))
        .init_asset::<Mesh>()
        .init_asset::<Image>()
        .init_asset::<StandardMaterial>()
        .init_asset::<bevy::mesh::skinning::SkinnedMeshInverseBindposes>()
        .insert_resource(TimeUpdateStrategy::ManualDuration(std::time::Duration::from_secs_f32(1.0 / 30.0)))
        .insert_resource(db.clone())
        .insert_resource(book)
        .insert_resource(units)
        .init_resource::<SummonPortals>()
        .add_message::<NetEvent>();
        app.add_systems(
            Update,
            (
                track_summon_portals,
                spawn_summon_portals,
                animate_summon_emergence,
                apply_emergence_clip,
                end_summon_portals,
                despawn_summon_portal_effects,
                restore_emergence_clip,
                cleanup_summon_portals,
            )
                .chain(),
        );
        let mut state = SystemState::<(Commands, CryAssets, Res<ClipLibrary>)>::new(app.world_mut());
        let pieces = {
            let (_, mut cry, _) = state.get_mut(app.world_mut()).unwrap();
            prepare(&cry.loader(), &look)
        };
        let root = {
            let (mut commands, mut cry, clips) = state.get_mut(app.world_mut()).unwrap();
            let root = spawn(&mut commands, &mut cry, &clips, pieces, Transform::default());
            commands.entity(root).insert((
                RemoteUnit::announced(&unit),
                VehicleDamage {
                    state: 1,
                    variant: look.normal.clone(),
                    max_hp,
                    requested: None,
                    dying_start: None,
                    dying_time: 0.0,
                    death_seen: false,
                    dead_effect_played: false,
                },
            ));
            root
        };
        state.apply(app.world_mut());
        Some((app, db, root))
    }

    /// The owner test's Red Farm Hauler (slave template 146, model 1553).
    fn hauler() -> Option<(App, Db, Entity)> {
        vehicle_app(146, 1553)
    }

    fn portal_effects(app: &mut App) -> Vec<(String, bool, f32)> {
        app.world_mut()
            .query::<(&aa_fx::Effect, &SummonPortalEffect)>()
            .iter(app.world())
            .map(|(effect, _)| (effect.name.clone(), effect.prime, effect.scale))
            .collect()
    }

    fn portal_markers(app: &mut App) -> Vec<(String, bool, f64)> {
        app.world_mut()
            .query::<(&aa_fx::Effect, &SummonPortalEffect)>()
            .iter(app.world())
            .map(|(effect, marker)| (effect.name.clone(), marker.stopped, marker.despawn_at))
            .collect()
    }

    fn created(tl_id: u16, hide: bool) -> NetEvent {
        NetEvent(Event::Slave(SlaveEvent::Created(aa_net::slave::SlaveCreated {
            owner_obj_id: 1,
            tl_id,
            obj_id: 7,
            hide_spawn_effect: hide,
            raw_id: 0,
            creator_name: "owner".into(),
        })))
    }

    /// The emergence clip plane reaches a per-vehicle `IllumMaterial` clone and the original handle is restored at
    /// the end (the shader discards behind the plane; the galleon/cart screenshot shows it).
    #[test]
    fn the_emergence_clip_plane_is_applied_and_restored() {
        use bevy::asset::AssetPlugin;
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<IllumMaterial>();
        let original = app
            .world_mut()
            .resource_mut::<Assets<IllumMaterial>>()
            .add(IllumMaterial::default());
        let root = app
            .world_mut()
            .spawn(SummonEmergence {
                start: Vec3::new(0.0, 0.0, 2.0),
                end: Vec3::ZERO,
                plane: Vec3::new(0.0, 0.0, 1.0),
                axis: Vec3::new(0.0, 0.0, -1.0),
                extent: 1.0,
                started: 0.0,
                duration: 1.0,
                ended: false,
                start_rotation: Quat::IDENTITY,
                end_rotation: Quat::IDENTITY,
                clip_normal: Vec3::new(0.0, 0.0, -1.0),
                buoyant: false,
            })
            .id();
        let mesh = app
            .world_mut()
            .spawn((MeshMaterial3d(original.clone()), ChildOf(root)))
            .id();
        app.add_systems(Update, (apply_emergence_clip, restore_emergence_clip).chain());
        app.update();
        app.update();
        let clipped = app
            .world()
            .get::<MeshMaterial3d<IllumMaterial>>(mesh)
            .expect("the mesh material")
            .0
            .clone();
        assert_ne!(clipped.id(), original.id(), "the mesh got its own clipped material");
        let material = app
            .world()
            .resource::<Assets<IllumMaterial>>()
            .get(&clipped)
            .expect("the clipped material");
        assert_eq!(material.extension.clip_plane, Vec4::new(0.0, 0.0, -1.0, 1.0));
        app.world_mut().get_mut::<SummonEmergence>(root).unwrap().ended = true;
        app.update();
        assert_eq!(
            app.world()
                .get::<MeshMaterial3d<IllumMaterial>>(mesh)
                .expect("the mesh material")
                .0
                .id(),
            original.id(),
            "the original material is restored"
        );
        assert!(app.world().get::<EmergenceClip>(root).is_none());
    }

    /// A material replaced while the emergence runs (stealth) is not restored over: only the clipped clone is undone.
    #[test]
    fn the_restore_does_not_undo_a_replaced_material() {
        use bevy::asset::AssetPlugin;
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<IllumMaterial>();
        let original = app
            .world_mut()
            .resource_mut::<Assets<IllumMaterial>>()
            .add(IllumMaterial::default());
        let root = app
            .world_mut()
            .spawn(SummonEmergence {
                start: Vec3::new(0.0, 0.0, 2.0),
                end: Vec3::ZERO,
                plane: Vec3::new(0.0, 0.0, 1.0),
                axis: Vec3::new(0.0, 0.0, -1.0),
                extent: 1.0,
                started: 0.0,
                duration: 1.0,
                ended: false,
                start_rotation: Quat::IDENTITY,
                end_rotation: Quat::IDENTITY,
                clip_normal: Vec3::new(0.0, 0.0, -1.0),
                buoyant: false,
            })
            .id();
        let mesh = app
            .world_mut()
            .spawn((MeshMaterial3d(original), ChildOf(root)))
            .id();
        app.add_systems(Update, (apply_emergence_clip, restore_emergence_clip).chain());
        app.update();
        app.update();
        let other = app
            .world_mut()
            .resource_mut::<Assets<IllumMaterial>>()
            .add(IllumMaterial::default());
        app.world_mut()
            .entity_mut(mesh)
            .insert(MeshMaterial3d(other.clone()));
        app.world_mut().get_mut::<SummonEmergence>(root).unwrap().ended = true;
        app.update();
        assert_eq!(
            app.world()
                .get::<MeshMaterial3d<IllumMaterial>>(mesh)
                .expect("the mesh material")
                .0
                .id(),
            other.id(),
            "a replaced material is left alone"
        );
    }

    /// `SCSlaveCreatedPacket` without `hideSpawnEffect` on slave 146 (Red Farm Hauler) plays the `slaves` row's
    /// `portal_spawn_fx_id` 1263 (`portal_tank_spawn`) once, at the clip-plane point, and moves the cart out of the
    /// plane over `portal_time` 1.0 s (`FUN_3909ea30` / `FUN_39097220`).
    #[test]
    fn a_created_vehicle_plays_its_summon_portal_once() {
        let Some((mut app, db, root)) = hauler() else { return };
        let portal = db.slave_summon_portal(146).expect("the Red Farm Hauler's summon portal");
        assert_eq!(portal, super::super::data::SummonPortal { fx_group: 1263, scale: 3.0, time: 1.0 });
        let book = super::super::combat::skills::SkillBook::open().expect("the game database");
        let items: Vec<String> = book
            .fx_group(portal.fx_group)
            .iter()
            .filter(|item| !item.effect.is_empty())
            .map(|item| item.effect.clone())
            .collect();
        assert_eq!(items.len(), 2, "{items:?}");
        assert!(items.iter().any(|name| name.ends_with("portal_tank_main")), "{items:?}");
        assert!(items.iter().any(|name| name.ends_with("portal_tank_main_start")), "{items:?}");
        assert!(
            book.fx_group(portal.fx_group).iter().any(|item| item.effect.is_empty() && item.sound.is_some()),
            "the group's FxSound item"
        );

        // a non-identity hull yaw: the emergence axis must be the hull's forward, not world -Z
        let yaw = 0.8;
        app.world_mut().get_mut::<Transform>(root).unwrap().rotation = Quat::from_rotation_y(yaw);
        app.world_mut().write_message(created(1, false));
        app.update();
        let effects = portal_effects(&mut app);
        assert_eq!(effects.len(), 2, "{effects:?}");
        for (name, primed, scale) in &effects {
            assert!(items.iter().any(|item| item == name), "{name}");
            assert!(!primed, "a summon portal must not start primed");
            assert!((*scale - 3.0).abs() < 1e-6, "{name} scale {scale}");
        }
        // the cart starts two extents behind its position, the portal at one extent
        let emergence = *app.world().get::<SummonEmergence>(root).expect("the emergence");
        let end = emergence.end;
        let axis = emergence.axis;
        assert!(emergence.extent > 0.5, "extent {}", emergence.extent);
        assert!(axis.y.abs() < 1e-6 && (axis.length() - 1.0).abs() < 1e-4, "{axis:?}");
        let rotation = app.world().get::<Transform>(root).expect("the root").rotation;
        // the model's forward is cry +Y (`cry_to_bevy(+Y)` is Bevy -Z); the axis must be the unit's forward
        let mut hull_forward = rotation * aa_world::cry_to_bevy(Vec3::Y);
        hull_forward.y = 0.0;
        let hull_forward = hull_forward.normalize_or_zero();
        assert!((axis - hull_forward).length() < 1e-5, "axis {axis:?} vs hull forward {hull_forward:?}");
        assert!((emergence.plane - (end - axis * emergence.extent)).length() < 1e-4);
        assert!((emergence.start - (end - axis * (2.0 * emergence.extent))).length() < 1e-4);
        assert_eq!(emergence.clip_normal, axis, "a land vehicle keeps the vertical portal plane");
        assert_eq!(emergence.start_rotation, emergence.end_rotation, "a land vehicle does not tilt");
        assert!(!emergence.buoyant);
        let transform = *app.world().get::<Transform>(root).expect("the root");
        // the emergence may run in this same schedule, so allow one frame of progress from the start
        let path = (emergence.end - emergence.start).length().max(1e-4);
        let progress = (transform.translation - emergence.start).length() / path;
        assert!(progress < 0.1, "the cart starts behind the plane: {progress}");
        // the portal hangs at the plane point, oriented by the unit, not at the cart centre
        let (anchor_entity, _anchor, anchor_transform) = app
            .world_mut()
            .query::<(Entity, &SummonPortalAnchor, &Transform)>()
            .iter(app.world())
            .map(|(entity, anchor, transform)| (entity, *anchor, *transform))
            .find(|(_, anchor, _)| anchor.root == root)
            .expect("the portal anchor");
        assert!((anchor_transform.translation - emergence.plane).length() < 1e-4, "{anchor_transform:?}");
        assert!(anchor_transform.rotation.angle_between(transform.rotation) < 1e-4);
        let children = app.world().get::<Children>(anchor_entity).expect("the portal's children");
        assert_eq!(children.len(), 2, "the portal particles hang at the anchor");
        // the pending timeline id was consumed: later frames play it once
        for _ in 0..5 {
            app.update();
        }
        assert_eq!(portal_effects(&mut app).len(), 2, "the portal plays once");
        let moved = app.world().get::<Transform>(root).unwrap().translation;
        assert!((moved - emergence.start).length() > 0.0, "the cart is emerging");
        // the land path is translation-only: a turn the server/remote driver makes during the portal window is not
        // overwritten and then snapped back
        let turned = Quat::from_rotation_y(1.1);
        app.world_mut().get_mut::<Transform>(root).unwrap().rotation = turned;
        for _ in 0..5 {
            app.update();
        }
        assert!(
            app.world().get::<Transform>(root).unwrap().rotation.angle_between(turned) < 1e-6,
            "the land emergence does not write the rotation"
        );
        // `portal_time` is 1.0 s: the cart reaches its position and the effects stop emitting
        for _ in 0..40 {
            app.update();
        }
        let emergence = *app.world().get::<SummonEmergence>(root).expect("the emergence");
        assert!(emergence.ended, "the emergence ends after portal_time");
        assert!((app.world().get::<Transform>(root).unwrap().translation - end).length() < 1e-4);
        let markers = portal_markers(&mut app);
        assert_eq!(markers.len(), 2, "the effect is not killed by the end event");
        assert!(markers.iter().all(|(_, stopped, _)| *stopped), "the end event stopped emission: {markers:?}");
        // a continuous/never-finished effect still has a bounded life: the fallback cap despawns it
        for _ in 0..360 {
            app.update();
        }
        assert!(portal_markers(&mut app).is_empty(), "the stopped effect is despawned by its cap");
    }

    /// A ship's summon (Eznan Cutter, slave 21) floats up out of the sea instead of sliding out of a vertical
    /// portal: the root starts below the surface with the hull top at the water, pitched bow up; the clip plane is
    /// the sea surface; midway it is partly righted and still clipped; after `portal_time` 7 s it is upright at its
    /// position on the surface. The rise/righting shape is the 2026-11-02 capture approximation documented on
    /// [`SummonEmergence`].
    #[test]
    fn a_summoned_ship_starts_below_the_water_tilted_and_ends_upright() {
        use aa_world::{TerrainQuery, WaterQuery};
        let Some((mut app, db, root)) = vehicle_app(21, 393) else { return };
        assert!(db.is_ship(393), "the galleon is a ShipModel");
        let portal = db.slave_summon_portal(21).expect("the Eznan Cutter's portal");
        assert_eq!((portal.fx_group, portal.scale, portal.time), (619, 10.0, 7.0));
        let sea = WaterQuery::fixture_sea("test", 2.0, TerrainQuery::fixture_flat("test", 0.0));
        app.insert_resource(sea);
        app.init_asset::<IllumMaterial>();
        // the ship's end position is on the sea surface; a child with an Illum material proves the sea clip is
        // applied for the whole window (the ship's own parts are StandardMaterial until PostUpdate's swap, which
        // this headless app does not run)
        app.world_mut().get_mut::<Transform>(root).unwrap().translation.y = 2.0;
        let original = app
            .world_mut()
            .resource_mut::<Assets<IllumMaterial>>()
            .add(IllumMaterial::default());
        let mesh = app
            .world_mut()
            .spawn((Mesh3d(Handle::default()), MeshMaterial3d(original.clone()), ChildOf(root)))
            .id();
        app.world_mut().write_message(created(1, false));
        app.update();
        app.update();
        let emergence = *app.world().get::<SummonEmergence>(root).expect("the ship's emergence");
        assert!(emergence.buoyant, "a ship floats up; the land path slides");
        assert_eq!(emergence.clip_normal, Vec3::Y, "the sea surface is the clip plane");
        assert!((emergence.plane.y - 2.0).abs() < 1e-4, "{:?}", emergence.plane);
        assert!((emergence.end.y - 2.0).abs() < 1e-4, "the ship ends on the surface: {:?}", emergence.end);
        assert!(emergence.start.y < 2.0, "the ship starts below the sea: {:?}", emergence.start);
        assert!(emergence.start.y < emergence.end.y, "it rises to its position: {:?} {:?}", emergence.start, emergence.end);
        // the horizontal start is still the native two extents behind, so the hull also slides through the portal
        let behind = emergence.end - emergence.axis * (2.0 * emergence.extent);
        assert!((emergence.start.x - behind.x).abs() < 1e-4 && (emergence.start.z - behind.z).abs() < 1e-4);
        // the anchor keeps the native x/z (end - axis * extent) and only moves to the water height; here the end is
        // on the water too, so it is the native plane point exactly
        assert!(
            (Vec2::new(emergence.plane.x, emergence.plane.z) - Vec2::new(behind.x, behind.z)).length() > 1e-4,
            "the anchor is not at the start"
        );
        assert!(
            (emergence.plane - (emergence.end - emergence.axis * emergence.extent)).length() < 1e-4,
            "the anchor stays native: {:?}",
            emergence.plane
        );
        // bow up: the local forward (-Z) of the start rotation points up
        let forward = emergence.start_rotation * Vec3::NEG_Z;
        assert!(forward.y > 0.5, "{forward:?}");
        let tilt = emergence.start_rotation.angle_between(emergence.end_rotation);
        assert!((tilt - SHIP_SUMMON_PITCH).abs() < 0.02, "start pitch {tilt}");
        // the clip plane the shader gets is the horizontal sea surface (normal +Y, height 2)
        let clip = Vec4::new(
            emergence.clip_normal.x,
            emergence.clip_normal.y,
            emergence.clip_normal.z,
            -emergence.clip_normal.dot(emergence.plane),
        );
        assert_eq!(clip, Vec4::new(0.0, 1.0, 0.0, -2.0));
        // ~3.5 s of 7: partly righted, between start and end, still clipped
        for _ in 0..103 {
            app.update();
        }
        let emergence = *app.world().get::<SummonEmergence>(root).expect("the emergence");
        assert!(!emergence.ended, "the ship is still emerging at 3.5 s");
        let transform = *app.world().get::<Transform>(root).expect("the root");
        assert!(
            transform.translation.y > emergence.start.y && transform.translation.y < emergence.end.y,
            "mid-window height {:?} between {:?} and {:?}",
            transform.translation.y,
            emergence.start.y,
            emergence.end.y
        );
        let from_start = emergence.start_rotation.angle_between(transform.rotation);
        let to_end = transform.rotation.angle_between(emergence.end_rotation);
        assert!(from_start > 1e-2 && to_end > 1e-2, "partly righted: {from_start} / {to_end}");
        assert!(to_end < tilt - 1e-2, "not upright yet: {to_end} of {tilt}");
        let mid = app
            .world()
            .get::<MeshMaterial3d<IllumMaterial>>(mesh)
            .expect("the mesh material")
            .0
            .clone();
        let mid_plane = app
            .world()
            .resource::<Assets<IllumMaterial>>()
            .get(&mid)
            .expect("the clipped material")
            .extension
            .clip_plane;
        assert_eq!(mid_plane, Vec4::new(0.0, 1.0, 0.0, -2.0), "still clipped at 3.5 s");
        // the rest of `portal_time`: upright at its position on the surface, the clip restored
        for _ in 0..135 {
            app.update();
        }
        let emergence = *app.world().get::<SummonEmergence>(root).expect("the emergence");
        assert!(emergence.ended, "the ship's emergence ends after portal_time");
        let transform = *app.world().get::<Transform>(root).expect("the root");
        assert!((transform.translation - emergence.end).length() < 1e-4, "{:?}", transform.translation);
        assert!(
            transform.rotation.angle_between(emergence.end_rotation) < 1e-4,
            "upright: {:?} vs {:?}",
            transform.rotation,
            emergence.end_rotation
        );
        assert_eq!(
            app.world()
                .get::<MeshMaterial3d<IllumMaterial>>(mesh)
                .expect("the mesh material")
                .0
                .id(),
            original.id(),
            "the sea clip is restored after portal_time"
        );
    }

    /// The fx group's sounds (`FxSound` items and each particle's `fx_particles.sound_id`) start once per summon,
    /// through the same audio system as the skill effects, at the portal.
    #[test]
    fn a_summon_portal_plays_its_fx_sounds_once_at_the_portal() {
        use super::super::combat::Sfx;
        use super::super::sounds::SoundBook;
        let Some((mut app, db, root)) = hauler() else { return };
        let portal = db.slave_summon_portal(146).expect("the Red Farm Hauler's portal");
        let book = super::super::combat::skills::SkillBook::open().expect("the game database");
        let expected: Vec<String> = book
            .fx_group(portal.fx_group)
            .iter()
            .filter_map(|item| match item.sound.as_ref() {
                Some(Sfx::Event(path)) => Some(path.to_string()),
                _ => None,
            })
            .collect();
        assert!(
            expected.iter().any(|path| path.eq_ignore_ascii_case("Sounds/x2eventsounds:portal:portal_tank")),
            "{expected:?}"
        );
        app.insert_resource(SoundBook::open());
        app.world_mut().write_message(created(1, false));
        for _ in 0..3 {
            app.update();
        }
        let count = |app: &mut App| {
            app.world_mut()
                .query::<&aa_audio::Sound>()
                .iter(app.world())
                .filter(|sound| expected.iter().any(|path| path.eq_ignore_ascii_case(&sound.event)))
                .count()
        };
        assert_eq!(count(&mut app), expected.len(), "one sound per group item");
        let anchor = app
            .world_mut()
            .query::<(Entity, &SummonPortalAnchor)>()
            .iter(app.world())
            .find(|(_, anchor)| anchor.root == root)
            .map(|(entity, _)| entity)
            .expect("the portal anchor");
        let hearing: Vec<Entity> = app
            .world_mut()
            .query::<(&aa_audio::Sound, &ChildOf)>()
            .iter(app.world())
            .filter(|(sound, _)| expected.iter().any(|path| path.eq_ignore_ascii_case(&sound.event)))
            .map(|(_, child)| child.parent())
            .collect();
        assert!(hearing.contains(&anchor), "the FxSound item hangs at the portal: {hearing:?}");
        for _ in 0..20 {
            app.update();
        }
        assert_eq!(count(&mut app), expected.len(), "the portal sound starts once per summon");
    }

    /// The sea portal's sound is the particle's `fx_particles.sound_id` (group 619 item 890 -> sound 2413
    /// `portal_ship_15`), not an `FxSound` item; it must take the same path once per summon.
    #[test]
    fn the_sea_portal_plays_its_particle_sound_once() {
        use super::super::combat::Sfx;
        use super::super::sounds::SoundBook;
        let Some((mut app, db, _root)) = vehicle_app(21, 393) else { return };
        let portal = db.slave_summon_portal(21).expect("the Eznan Cutter's portal");
        assert_eq!(portal.fx_group, 619);
        let book = super::super::combat::skills::SkillBook::open().expect("the game database");
        let expected = "Sounds/x2eventsounds:portal:portal_ship_15";
        let sounds: Vec<String> = book
            .fx_group(portal.fx_group)
            .iter()
            .filter_map(|item| match item.sound.as_ref() {
                Some(Sfx::Event(path)) => Some(path.to_string()),
                _ => None,
            })
            .collect();
        assert!(
            sounds.iter().any(|path| path.eq_ignore_ascii_case(expected)),
            "the particle sound_id path: {sounds:?}"
        );
        app.insert_resource(SoundBook::open());
        app.world_mut().write_message(created(1, false));
        for _ in 0..3 {
            app.update();
        }
        let count = |app: &mut App| {
            app.world_mut()
                .query::<&aa_audio::Sound>()
                .iter(app.world())
                .filter(|sound| sound.event.eq_ignore_ascii_case(expected))
                .count()
        };
        assert_eq!(count(&mut app), 1, "the sea portal sound starts once");
        for _ in 0..20 {
            app.update();
        }
        assert_eq!(count(&mut app), 1, "the sea portal sound starts once per summon");
    }

    /// The portal belongs to creation, not to coming into view: a vehicle the server only streams in later
    /// (`SCUnitState` with no `SCSlaveCreatedPacket`) plays none, and `hideSpawnEffect` suppresses it.
    #[test]
    fn a_vehicle_streamed_in_later_plays_no_summon_portal() {
        let Some((mut app, _db, root)) = hauler() else { return };
        for _ in 0..5 {
            app.update();
        }
        assert!(portal_effects(&mut app).is_empty(), "a streamed-in vehicle has no creation packet");
        assert!(app.world().get::<SummonEmergence>(root).is_none(), "no emergence");
        app.world_mut().write_message(created(1, true));
        for _ in 0..5 {
            app.update();
        }
        assert!(portal_effects(&mut app).is_empty(), "hideSpawnEffect suppresses the portal");
        assert!(app.world().get::<SummonEmergence>(root).is_none(), "hideSpawnEffect skips the emergence");
    }

    /// The `slaves` row of the owner test's `/slave spawn 146`, a slave without a portal (the rowboat, 15), and the
    /// zero-time rows (`FUN_390cf130` only starts an emergence when the time is above 0).
    #[test]
    fn slave_146_has_the_portal_tank_spawn_group() {
        let Some(db) = Db::open() else { return };
        assert_eq!(
            db.slave_summon_portal(146),
            Some(super::super::data::SummonPortal { fx_group: 1263, scale: 3.0, time: 1.0 })
        );
        assert_eq!(db.slave_summon_portal(15), None);
        for template in [54, 94, 115] {
            let portal = db.slave_summon_portal(template).expect("a zero-time portal row");
            assert_eq!(portal.time, 0.0, "slave {template}");
        }
        // the sea variant (Eznan Cutter, 21) uses the water-column group, not the tank ring
        let cutter = db.slave_summon_portal(21).expect("the Eznan Cutter's summon portal");
        assert_eq!((cutter.fx_group, cutter.scale, cutter.time), (619, 10.0, 7.0));
        let book = super::super::combat::skills::SkillBook::open().expect("the game database");
        let items: Vec<String> = book
            .fx_group(cutter.fx_group)
            .iter()
            .map(|item| item.effect.clone())
            .filter(|name| !name.is_empty())
            .collect();
        assert!(
            items.iter().any(|name| name.contains("shipportal_open_main_x15")),
            "the cutter's water column: {items:?}"
        );
        assert!(!items.iter().any(|name| name.contains("portal_tank")), "{items:?}");
    }

    /// A slave whose `portal_time` is 0 (slave 54, the tank) gets no portal and no emergence.
    #[test]
    fn a_zero_portal_time_starts_no_emergence() {
        let Some((mut app, db, root)) = vehicle_app(54, 1008) else { return };
        assert_eq!(db.slave_summon_portal(54).map(|p| p.time), Some(0.0));
        app.world_mut().write_message(created(1, false));
        app.update();
        assert!(portal_effects(&mut app).is_empty(), "portal_time 0 plays no portal");
        assert!(app.world().get::<SummonEmergence>(root).is_none(), "portal_time 0 starts no emergence");
        assert!(
            app.world().resource::<SummonPortals>().pending.is_empty(),
            "the timeline id is consumed"
        );
    }

    /// The red cargo/boiler is the `add` subset at the model's **+Y** end (the bed `wood` is behind it): cry +Y is
    /// the forward the emergence moves along, so the captured "cargo end first" is the +forward direction, not a
    /// flipped axis.
    #[test]
    fn the_red_cargo_end_is_the_model_forward_end() {
        if !aa_cry::pak_available() {
            return;
        }
        let model = aa_cry::mesh::Model::load("objects/env/06_unit/05_carriage/transfers_trailer_c.chr")
            .expect("the Red Farm Hauler body");
        let mtl = aa_cry::material::Material::load("objects/env/06_unit/05_carriage/transfers_trailer_c.mtl")
            .expect("its materials");
        let mesh = model.meshes.first().expect("the body mesh");
        let subset = |name: &str| {
            mesh.subsets
                .iter()
                .find(|s| mtl.sub(s.material_id as usize).is_some_and(|m| m.name == name))
                .unwrap_or_else(|| panic!("the {name} subset"))
        };
        let cargo = subset("add");
        let bed = subset("wood");
        assert!(cargo.center.y > 0.0, "cargo at +Y: {:?}", cargo.center);
        assert!(cargo.center.y > bed.center.y, "cargo ahead of the bed: {cargo:?} {bed:?}");
        assert_eq!(aa_world::cry_to_bevy(Vec3::Y), Vec3::NEG_Z, "cry +Y is the Bevy forward");
    }

    /// The fallback cap despawns a summon effect that is not under a portal anchor (the prefab's `portal_exhaust` is
    /// parented to the unit root), even when no simulation ever reports finished.
    #[test]
    fn a_summon_effect_is_despawned_at_its_cap() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        let entity = app
            .world_mut()
            .spawn((
                aa_fx::Effect::new("T.a"),
                SummonPortalEffect {
                    until: 0.0,
                    despawn_at: 0.0,
                    stopped: true,
                },
            ))
            .id();
        app.add_systems(Update, despawn_summon_portal_effects);
        app.update();
        assert!(app.world().get_entity(entity).is_err(), "the cap removes the effect");
    }

    /// A later `hideSpawnEffect` creation for the same timeline id clears the pending wait, and an entry whose unit
    /// never appears expires after [`SUMMON_PENDING_TTL`].
    #[test]
    fn a_hidden_creation_clears_the_pending_entry_and_old_entries_expire() {
        let Some((mut app, _db, _root)) = hauler() else { return };
        app.world_mut().write_message(created(99, false));
        app.update();
        assert!(app.world().resource::<SummonPortals>().pending.contains_key(&99));
        app.world_mut().write_message(created(99, true));
        app.update();
        assert!(
            !app.world().resource::<SummonPortals>().pending.contains_key(&99),
            "a hidden creation clears the pending timeline id"
        );
        app.world_mut().write_message(created(98, false));
        app.update();
        assert!(app.world().resource::<SummonPortals>().pending.contains_key(&98));
        for _ in 0..320 {
            app.update();
        }
        assert!(
            !app.world().resource::<SummonPortals>().pending.contains_key(&98),
            "an unresolved packet expires"
        );
    }

    /// The local player's drive (`Driven`, `cart::drive_cart`) owns the transform: the emergence must not fight it.
    #[test]
    fn a_driven_vehicle_keeps_its_emergence_from_moving_it() {
        let Some((mut app, _db, root)) = hauler() else { return };
        app.world_mut().write_message(created(1, false));
        app.update();
        app.world_mut().entity_mut(root).insert(super::super::mount::Driven);
        let placed = Vec3::new(4.0, 0.0, 0.0);
        app.world_mut().get_mut::<Transform>(root).unwrap().translation = placed;
        app.update();
        assert_eq!(
            app.world().get::<Transform>(root).unwrap().translation,
            placed,
            "drive_cart owns the transform of a driven cart"
        );
        assert!(app.world().get::<SummonEmergence>(root).is_some());
    }

    /// `FUN_39097140` plays the prefab's authored `portal_exhaust` on the emerging unit (the galleons have one;
    /// the tractor does not).
    #[test]
    fn the_prefabs_portal_exhaust_is_spawned_on_the_unit() {
        if !aa_cry::pak_available() {
            return;
        }
        let variant = "prefab://Prefabs/galleon.xml/galleon.step_10galleon_normal";
        let named = super::super::data::prefab_effects_named(variant);
        let exhaust = named
            .iter()
            .find(|named| named.name.eq_ignore_ascii_case("portal_exhaust"))
            .expect("the galleon's portal_exhaust");
        assert!(
            exhaust.effect.effect.eq_ignore_ascii_case("X2_EFFECTS.e_effect.caravel_portal_water"),
            "{}",
            exhaust.effect.effect
        );
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        let root = app.world_mut().spawn(Transform::default()).id();
        let mut state = SystemState::<Commands>::new(app.world_mut());
        let spawned = {
            let mut commands = state.get_mut(app.world_mut()).unwrap();
            spawn_portal_exhaust(&mut commands, root, variant, 10.0)
        };
        state.apply(app.world_mut());
        assert_eq!(spawned.len(), 1);
        let effect = app.world().get::<aa_fx::Effect>(spawned[0]).expect("the exhaust effect");
        assert!(effect.name.eq_ignore_ascii_case("X2_EFFECTS.e_effect.caravel_portal_water"));
        assert_eq!(app.world().get::<ChildOf>(spawned[0]).unwrap().parent(), root);
        assert!(app.world().get::<SummonPortalEffect>(spawned[0]).is_some());
    }

    /// `UpdateIdleAnim` (`FUN_390d6690`): the Farm Wagon's body loops `fist_ba_relaxed_idle`, so its engine
    /// bones move while the cart stands still. The vehicle idle owns the loop (no generic creature idle).
    #[test]
    fn the_wagon_body_idles_its_engine() {
        let Some((mut app, _)) = wagon() else { return };
        let body = app
            .world_mut()
            .query_filtered::<Entity, With<VehicleIdle>>()
            .iter(app.world())
            .next()
            .expect("the wagon body carries its idle");
        let animator = app.world().get::<aa_anim::Animator>(body).expect("the body's animator");
        let playing = animator.current.as_ref().expect("the idle is playing");
        assert!(
            playing.clip.source.as_deref().is_some_and(|s| s.contains("fist_ba_relaxed_idle")),
            "the body plays its .cal idle: {:?}",
            playing.clip.source
        );
        let bone = find_named(app.world_mut(), "Bone_engine01").expect("the body's Bone_engine01");
        let first = app.world().get::<Transform>(bone).expect("the engine bone transform").rotation;
        for _ in 0..20 {
            app.update();
        }
        let second = app.world().get::<Transform>(bone).expect("the engine bone transform").rotation;
        assert!(first.angle_between(second) > 0.02, "the engine bone moved from {first:?} to {second:?}");
    }

    /// The idle writes `Bone_spindle` every frame; the steering has to turn that sampled rotation, not the bind
    /// pose it was found with. Driving with a yaw rate must leave the bone turned away from the idle pose.
    #[test]
    fn the_idle_does_not_overwrite_the_steered_front_axle() {
        let Some((mut app, root)) = wagon() else { return };
        for _ in 0..10 {
            app.update();
        }
        {
            let mut motion = app.world_mut().get_mut::<VehicleMotion>(root).expect("the vehicle root's motion");
            motion.forward_speed = 4.0;
            motion.yaw_rate = 0.4;
            motion.velocity = Vec3::new(4.0, 0.0, 0.0);
        }
        for _ in 0..30 {
            app.update();
        }
        let spindle = *app.world().get::<Spindle>(root).expect("the wagon's spindle");
        assert!(spindle.angle.abs() > 0.05, "the front axle should steer, angle {}", spindle.angle);
        let (joint, animator_entity, index) = spindle.joint.expect("the spindle bone");
        let animated = app.world().get::<Animator>(animator_entity).expect("the body's animator").pose().rot[index];
        let bone = app.world().get::<Transform>(joint).expect("the spindle bone transform").rotation;
        let turned = animated.angle_between(bone);
        assert!(turned > 0.05, "the idle overwrote the steered spindle (the bone is {turned} from its pose)");
    }

    /// The dying prefab's `AnimObject` starts its authored `dead_01` on its own (`Animation="dead_01"`,
    /// `bPlaying="1"` in `vehicle_tractor.tractor_body_dying`; `FUN_39109d20` opens the Animation child and
    /// reads `bPlaying`, and the clip name/loop are engine AnimObject behaviour): the cart tips and rolls
    /// without waiting for a `Dead` marker. The clip is 3.33 s and the body's bones move.
    #[test]
    fn the_dying_prefab_plays_its_authored_dead_01_and_tips_the_body() {
        if !aa_cry::pak_available() {
            return;
        }
        let Some(db) = Db::open() else { return };
        let Some(look) = db.vehicle_look(1553) else { return };
        let variants = db.vehicle_variants(1553).expect("the Red Farm Hauler's rows");
        let variant = variants.for_state(5).to_string();
        assert!(variant.ends_with("tractor_body_dying"), "{variant}");
        let max_hp = db.slave_max_hp(146, 50).expect("the Red Farm Hauler's MaxHealth");
        let unit = slave_unit_kind(7, 146, 146, 1553, max_hp, max_hp);
        let mut app = damage_app(
            &db,
            VehicleLook { normal: variant.clone(), wheels: look.wheels },
            unit,
            5,
            variant,
            max_hp,
            false,
        );
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(1.0 / 30.0),
        ));
        app.update();
        let body = app
            .world_mut()
            .query_filtered::<Entity, With<VehicleIdle>>()
            .iter(app.world())
            .next()
            .expect("the dying body carries its idle");
        let source = app
            .world()
            .get::<aa_anim::Animator>(body)
            .and_then(|a| a.current.as_ref())
            .and_then(|p| p.clip.source.as_deref())
            .map(str::to_string);
        assert!(
            source.as_deref().is_some_and(|s| s.contains("dead_01")),
            "the prefab's authored dead_01 starts without Dead: {source:?}"
        );
        assert!(
            app.world().get::<VehicleIdle>(body).is_some_and(|idle| idle.stopped),
            "a one-shot authored animation keeps the idle from replacing it"
        );
        let bones = |app: &mut App| {
            app.world_mut()
                .query::<(&Name, &Transform)>()
                .iter(app.world())
                .map(|(name, transform)| (name.as_str().to_string(), transform.rotation))
                .collect::<std::collections::HashMap<_, _>>()
        };
        let before = bones(&mut app);
        for _ in 0..30 {
            app.update();
        }
        let after = bones(&mut app);
        let moved = before
            .iter()
            .filter_map(|(name, rotation)| after.get(name).map(|now| (name, rotation.angle_between(*now))))
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .expect("the body's bones");
        assert!(
            moved.1 > 0.02,
            "dead_01 tips the cart: only {moved:?} moved"
        );
    }

    /// A destroyed vehicle stops its idle and plays the body's `dead_01` (its Animator is a child of the root that
    /// carries `Dead`, so `combat::ensure_corpses` alone never reaches it); a resurrection starts the idle again.
    #[test]
    fn a_destroyed_wagon_stops_its_idle_and_plays_dead_01() {
        let Some((mut app, root)) = wagon() else { return };
        for _ in 0..5 {
            app.update();
        }
        let body = app
            .world_mut()
            .query_filtered::<Entity, With<VehicleIdle>>()
            .iter(app.world())
            .next()
            .expect("the wagon body carries its idle");
        let clip = |app: &App| {
            app.world()
                .get::<Animator>(body)
                .and_then(|a| a.current.as_ref())
                .and_then(|p| p.clip.source.as_deref())
                .map(str::to_string)
        };
        assert!(clip(&app).is_some_and(|s| s.contains("fist_ba_relaxed_idle")), "the idle starts: {:?}", clip(&app));
        app.world_mut().entity_mut(root).insert(Dead { at: 0.0 });
        app.update();
        assert!(app.world().get::<VehicleIdle>(body).unwrap().stopped, "the idle stays stopped on death");
        assert!(clip(&app).is_some_and(|s| s.contains("dead_01")), "the body plays dead_01: {:?}", clip(&app));
        app.world_mut().entity_mut(root).remove::<Dead>();
        for _ in 0..2 {
            app.update();
        }
        assert!(!app.world().get::<VehicleIdle>(body).unwrap().stopped, "resurrection re-enables the idle");
        assert!(clip(&app).is_some_and(|s| s.contains("fist_ba_relaxed_idle")), "the idle resumes: {:?}", clip(&app));
    }

    fn find_named(world: &mut World, name: &str) -> Option<Entity> {
        let mut query = world.query::<(&Name, Entity)>();
        query.iter(world).find(|(n, _)| n.as_str() == name).map(|(_, e)| e)
    }

    #[test]
    fn a_wheel_advances_against_the_forward_speed() {
        // 4 m/s on a 0.5 m wheel for 0.1 s: 0.8 rad backwards (FUN_390d6b10)
        assert!((wheel_angle(0.0, 4.0, 0.5, 0.1) + 0.8).abs() < 1e-5);
        assert!((wheel_angle(0.0, 2.0, 0.5, 0.05) + 0.2).abs() < 1e-5);
        let mut angle = 0.0;
        for _ in 0..100 {
            angle = wheel_angle(angle, 4.0, 0.5, 0.1);
        }
        assert!(
            angle.abs() <= std::f32::consts::PI + 1e-5,
            "wrapped to 2π: {angle}"
        );
    }

    #[test]
    fn a_reversing_wheel_turns_the_other_way() {
        let forward = wheel_angle(0.0, 4.0, 0.5, 0.1);
        let reverse = wheel_angle(0.0, -4.0, 0.5, 0.1);
        assert!(reverse > 0.0 && forward < 0.0);
        assert!((forward + reverse).abs() < 1e-6);
    }

    #[test]
    fn the_spindle_follows_the_turn_radius() {
        // 4 m/s at 0.4 rad/s: R = 10 m; atan(2.389 / 10) and atan(1.766 / 10), averaged
        let expected = ((2.389f32 / 10.0).atan() + (1.766f32 / 10.0).atan()) * 0.5;
        let angle = spindle_angle(4.0, 0.4, 2.389, 1.766);
        assert!((angle - expected).abs() < 1e-5, "{angle} vs {expected}");
        assert!(angle > 0.0 && angle < SPINDLE_MAX_ANGLE);
    }

    #[test]
    fn the_spindle_flips_in_reverse() {
        let forward = spindle_angle(4.0, 0.4, 2.389, 1.766);
        let reverse = spindle_angle(-4.0, 0.4, 2.389, 1.766);
        assert!((forward + reverse).abs() < 1e-5, "{forward} {reverse}");
        // a right turn (negative yaw rate) is negative going forward
        assert!(spindle_angle(4.0, -0.4, 2.389, 1.766) < 0.0);
    }

    #[test]
    fn a_hard_turn_is_clamped_to_thirty_degrees() {
        // R = 0.04 m: atan(2.389 / R) is far past the limit
        assert!((spindle_angle(0.2, 5.0, 2.389, 1.766) - SPINDLE_MAX_ANGLE).abs() < 1e-6);
        assert!((spindle_angle(-0.2, 5.0, 2.389, 1.766) + SPINDLE_MAX_ANGLE).abs() < 1e-6);
        // standstill fallback: yaw rate over the limit, here the captured full-lock rate
        assert!((spindle_angle(0.0, 0.4, 2.389, 1.766) - SPINDLE_MAX_ANGLE).abs() < 1e-6);
        assert_eq!(spindle_angle(0.0, 0.0, 2.389, 1.766), 0.0);
        assert!((spindle_angle(0.0, -0.4, 2.389, 1.766) + SPINDLE_MAX_ANGLE).abs() < 1e-6, "right at a standstill");
    }

    #[test]
    fn the_spindle_is_smoothed_by_min_dt_100ms() {
        assert!((spindle_step(0.0, 1.0, 1.0) - 0.1).abs() < 1e-6);
        assert!((spindle_step(0.0, 1.0, 0.05) - 0.05).abs() < 1e-6);
        let mut angle = 0.0;
        for _ in 0..100 {
            angle = spindle_step(angle, 1.0, 0.1);
        }
        assert!((angle - 1.0).abs() < 1e-4, "converges: {angle}");
    }

    /// `FUN_3989f8b0`: 100/76 -> 1, 75/51 -> 2, 50/26 -> 3, 25 and the truncated 0 -> 4, hp 0 or max 0 -> 6.
    #[test]
    fn the_damage_state_follows_the_decompiled_thresholds() {
        for (hp, max_hp, state) in [
            (100, 100, 1),
            (76, 100, 1),
            (75, 100, 2),
            (51, 100, 2),
            (50, 100, 3),
            (26, 100, 3),
            (25, 100, 4),
            (9, 1000, 4),
            (0, 100, 6),
            (0, 0, 6),
            (100, 0, 6),
        ] {
            assert_eq!(model_state(hp, max_hp), state, "{hp}/{max_hp}");
        }
        // C integer division truncates: 7599 * 100 / 10000 is 75, not 76
        assert_eq!(model_state(7599, 10000), 2);
        assert_eq!(model_state(7600, 10000), 1);
        assert_eq!(model_state(5099, 10000), 3);
        assert_eq!(model_state(5100, 10000), 2);
        assert_eq!(model_state(2599, 10000), 4);
        assert_eq!(model_state(2600, 10000), 3);
    }

    /// `FUN_390e7910(5,1)` plays the prefab effects on every death, including 6 -> 5, but the dead prefab's
    /// one-shots are not replayed when the HP-0 poller already showed 6 for this death.
    #[test]
    fn the_die_path_plays_effects_and_the_dead_ones_only_once() {
        assert!(plays_state_effects(6, 5, false), "6 -> 5 plays dying");
        assert!(plays_state_effects(6, 5, true), "6 -> 5 still plays dying");
        assert!(plays_state_effects(1, 6, false), "a first dead state plays");
        assert!(
            !plays_state_effects(5, 6, true),
            "the dead one-shots do not replay after the poller showed them"
        );
        assert!(!plays_state_effects(4, 2, false), "a heal plays none");
        assert!(plays_state_effects(1, 4, false), "a worse state plays");
    }

    fn slave_unit(obj_id: u32, template_id: u32, model_id: u32, hp: i32) -> aa_net::model::Unit {
        aa_net::model::Unit {
            obj_id,
            name: format!("slave {obj_id}"),
            kind: aa_net::model::UnitKind::Slave {
                id: template_id,
                tl_id: 1,
                template_id,
                summoner_id: 1,
            },
            owner_name: String::new(),
            pos: aa_net::Pos::new(0.0, 0.0, 0.0),
            rotation: [0.0; 3],
            scale: 1.0,
            level: 50,
            model_id,
            equipment: aa_net::model::Equipment::default(),
            model_params: aa_net::model::ModelParams::default(),
            hp,
            mp: 0,
            max_hp: hp,
            max_mp: 0,
            dead: false,
            race_gender: 0,
            movement: None,
            faction_id: 0,
            expedition_id: 0,
            appellation: 0,
            flags: 0,
            gm_modes: [0; 4],
            bounty_money: None,
            pvp_honor: 0,
            pvp_kills: 0,
            skills: Vec::new(),
            passive_buffs: Vec::new(),
            ability_exp: Vec::new(),
            active_abilities: Vec::new(),
            buffs: Vec::new(),
            anim_action: 0,
            house_posture: None,
            turret: None,
            state_attachments: None,
            attached_to: None,
            doodad_bond: None,
        }
    }

    fn slave_unit_kind(
        obj_id: u32,
        kind: u32,
        template_id: u32,
        model_id: u32,
        hp: i32,
        max_hp: i32,
    ) -> aa_net::model::Unit {
        let mut unit = slave_unit(obj_id, template_id, model_id, hp);
        unit.kind = aa_net::model::UnitKind::Slave {
            id: kind,
            tl_id: 1,
            template_id,
            summoner_id: 1,
        };
        unit.max_hp = max_hp;
        unit
    }

    /// The state of a real slave template on the `SCUnitPoints` path (`Units::apply`), max from attribute 6.
    #[test]
    fn an_hp_update_changes_the_state_of_a_real_slave_template() {
        let Some(db) = Db::open() else {
            eprintln!("skip: no client database");
            return;
        };
        let max_hp = db
            .slave_max_hp(21, 50)
            .expect("Eznan Cutter template 21's MaxHealth");
        assert_eq!(max_hp, 52_216);
        let unit = slave_unit(7, 21, 393, max_hp);
        let mut units = Units::default();
        units.apply(&aa_net::Event::UnitSpawned(Box::new(unit)));
        let state = |units: &Units| units.by_id.get(&7).map(|unit| damage_state(&db, unit));
        assert_eq!(state(&units), Some(1));

        let update = |units: &mut Units, hp: i32| {
            units.apply(&aa_net::Event::UnitPoints {
                obj_id: 7,
                hp,
                mp: 0,
            });
        };
        update(&mut units, max_hp * 3 / 4); // 75 %
        assert_eq!(state(&units), Some(2));
        update(&mut units, max_hp / 2); // 50 %
        assert_eq!(state(&units), Some(3));
        update(&mut units, max_hp / 4); // 25 %
        assert_eq!(state(&units), Some(4));
        update(&mut units, 0);
        assert_eq!(state(&units), Some(6), "dead");
        assert_eq!(units.by_id.get(&99), None, "not in view");
    }

    /// The unit's own `max_hp` (wire / `SCMySlave`) does not override unit attribute 6: a 500-HP slave of
    /// template 21 is at 0 % of 52216, state 4, even when the wire says its maximum is 1000.
    #[test]
    fn the_owned_slave_report_does_not_override_the_attribute() {
        let Some(db) = Db::open() else { return };
        let unit = slave_unit(7, 21, 393, 500);
        assert_eq!(unit.max_hp, 500, "the wire maximum");
        assert_eq!(slave_max_hp(&db, &unit), 52_216);
        assert_eq!(damage_state(&db, &unit), 4);
    }

    /// The damaged and dead tractor prefabs carry the `_spawn_effect*` trigger and the standing fire / smoke.
    #[test]
    fn the_damage_prefabs_name_their_effects() {
        if !aa_cry::pak_available() {
            return;
        }
        let damaged = super::super::data::prefab_effects_named(
            "prefab://prefabs/vehicle_tractor.xml/vehicle_tractor.tractor_body_damage75",
        );
        assert!(
            damaged
                .iter()
                .any(|e| e.name.eq_ignore_ascii_case("_spawn_effect_dmg") && !e.active),
            "the damage trigger"
        );
        assert!(
            damaged.iter().any(|e| e.name == "_spawn" && e.active),
            "the standing fire"
        );
        let dead = super::super::data::prefab_effects_named(
            "prefab://prefabs/vehicle_tractor.xml/vehicle_tractor.tractor_body_dead",
        );
        assert!(
            dead.iter().any(|e| e.name == "_spawn" && e.active),
            "the burning smoke"
        );
        assert!(dead.iter().any(|e| e.name == "_dead_effect" && e.active));
        // the exact names `FUN_390eaf40` collects; a prefab's other active emitters are not speed-driven
        assert!(is_speed_emitter("exhaust"));
        assert!(is_speed_emitter("splash_l"));
        assert!(is_speed_emitter("splash_r"));
        assert!(!is_speed_emitter("_spawn"));
        assert!(!is_speed_emitter("caravel_cruise_pool"));
        assert!(!is_speed_emitter("airship_smoke"));
    }

    /// The Red Farm Hauler (slave 146, model 1553, `vehicle_models` 65) is the owner's destroyed vehicle:
    /// dying carries the fire particle and dead carries the burning smoke and the explosion. Both must be
    /// spawned as `aa_fx::Effect` entities, not just the mesh swap.
    #[test]
    fn the_red_farm_haulers_dying_and_dead_states_spawn_their_effects() {
        if !aa_cry::pak_available() {
            return;
        }
        let Some(db) = Db::open() else { return };
        let Some(look) = db.vehicle_look(1553) else {
            return;
        };
        let max_hp = db
            .slave_max_hp(146, 50)
            .expect("Red Farm Hauler template 146's MaxHealth");
        assert_eq!(max_hp, 7_296);
        let unit = slave_unit_kind(7, 146, 146, 1553, max_hp, max_hp);
        let mut units = Units::default();
        units.apply(&aa_net::Event::UnitSpawned(Box::new(unit.clone())));
        let mut app = damage_app(&db, look.clone(), unit, 1, look.normal, max_hp, true);
        app.insert_resource(units);
        app.update();
        let root = app.world().resource::<DamageRoot>().0;

        set_dying_time(&mut app, 60.0);
        die(&mut app);
        let dying = wait_for_state(&mut app, 5);
        assert!(dying.variant.ends_with("tractor_body_dying"));
        let names = effect_names(&mut app, root);
        assert!(
            names
                .iter()
                .any(|name| name == "x2_effects.housing_test.housing_fire_b"),
            "the dying fire: {names:?}"
        );
        assert!(
            !names.iter().any(|name| name.starts_with("x2_sounds.")),
            "the sound particle is filtered: {names:?}"
        );

        end_dying_window_in(&mut app, 0.2);
        let dead = wait_for_state(&mut app, 6);
        assert!(dead.variant.ends_with("tractor_body_dead"));
        let effects = effect_flags(&mut app, root);
        assert!(
            effects
                .iter()
                .any(|(name, _)| name == "x2_effects.fire_smoke.burning_smoke_01"),
            "the dead smoke: {effects:?}"
        );
        assert!(
            effects.iter().any(|(name, despawn)| {
                name == "x2_effects.housing_test.small_ship_exp" && *despawn
            }),
            "the dead explosion is a burst that retires: {effects:?}"
        );
    }

    /// A slave that arrives already dead only gets the dead prefab's standing smoke; the `_dead_effect`
    /// explosion waits for a worsening transition into 6.
    #[test]
    fn a_slave_announced_dead_does_not_play_the_explosion() {
        if !aa_cry::pak_available() {
            return;
        }
        let dead = "prefab://prefabs/vehicle_tractor.xml/vehicle_tractor_c.tractor_body_dead";
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        let root = app.world_mut().spawn(()).id();
        app.add_systems(Startup, move |mut commands: Commands| {
            spawn_state_effects(&mut commands, root, dead, false);
        });
        app.update();
        let names = effect_names(&mut app, root);
        assert!(
            names
                .iter()
                .any(|name| name == "x2_effects.fire_smoke.burning_smoke_01"),
            "the standing smoke: {names:?}"
        );
        assert!(
            !names
                .iter()
                .any(|name| name == "x2_effects.housing_test.small_ship_exp"),
            "no explosion on arrival: {names:?}"
        );
    }

    /// The Eznan Cutter (slave 21, model 393): death has no explosion (`dying_time` 0, and `galleon.dead`
    /// has no `_dead_effect`), the damaged prefab's aft-deck fire keeps burning, the hull sinks over
    /// [`WRECK_SINK_TIME`] with the captured bow-up list, and the dead prefab's water-destroy effect is
    /// drawn. See the 2026-11-02 capture, "Eznan Cutter damage and death".
    #[test]
    fn a_dead_ship_sinks_burning_without_an_explosion() {
        if !aa_cry::pak_available() {
            return;
        }
        let Some(db) = Db::open() else { return };
        let Some(look) = db.vehicle_look(393) else { return };
        let variants = db.vehicle_variants(393).expect("the Eznan Cutter's rows");
        let max_hp = db.slave_max_hp(21, 50).expect("Eznan Cutter template 21's MaxHealth");
        let hp = max_hp * 25 / 100;
        let unit = slave_unit_kind(7, 21, 21, 393, hp, max_hp);
        let variant = variants.for_state(4).to_string();
        assert!(variant.ends_with("galleon.damaged75"), "{variant}");
        let mut units = Units::default();
        units.apply(&aa_net::Event::UnitSpawned(Box::new(unit.clone())));
        let mut app = damage_app(
            &db,
            VehicleLook { normal: variant.clone(), wheels: look.wheels },
            unit,
            4,
            variant,
            max_hp,
            true,
        );
        app.insert_resource(units);
        app.update();
        let root = app.world().resource::<DamageRoot>().0;
        let fire = |app: &mut App| effect_names(app, root);
        {
            let names = fire(&mut app);
            assert!(
                names.iter().any(|name| name == "x2_effects.e_effect.ship_damage_fireflame"),
                "the 25 % prefab burns: {names:?}"
            );
        }
        die(&mut app);
        let dead = wait_for_state(&mut app, 6);
        assert!(dead.variant.ends_with("galleon.dead"), "{}", dead.variant);
        let names = fire(&mut app);
        assert!(
            names.iter().any(|name| name == "x2_effects.e_effect.ship_damage_fireflame"),
            "the damaged prefab's fire keeps burning through the sink: {names:?}"
        );
        assert!(
            names.iter().any(|name| name == "x2_effects.e_effect.caraval_water_destroy"),
            "the dead prefab's water-destroy effect: {names:?}"
        );
        assert!(
            !names.iter().any(|name| name == "x2_effects.housing_test.small_ship_exp"),
            "no explosion: {names:?}"
        );
        // the captured ~12 s sink: lower the hull and list it
        let sink = *app.world().get::<WreckSink>(root).expect("the dead ship sinks");
        assert_eq!(sink.base.translation, Vec3::ZERO);
        let now = app.world().resource::<Time>().elapsed_secs_f64();
        app.world_mut().get_mut::<WreckSink>(root).unwrap().started = now - WRECK_SINK_TIME;
        app.update();
        let transform = *app.world().get::<Transform>(root).expect("the root");
        assert!(
            transform.translation.y < -10.0,
            "the dead hull sinks well below the waterline: {transform:?}"
        );
        assert!(
            transform.rotation.angle_between(sink.base.rotation) > 0.05,
            "the dead hull lists: {transform:?}"
        );
        assert!(app.world().get::<WreckSink>(root).is_some_and(|sink| sink.finished));
        // a fully sunk wreck keeps its fixed offset; the sink does not rewrite the root every frame
        app.update();
        assert_eq!(
            app.world().get::<Transform>(root).expect("the root"),
            &transform,
            "the finished sink leaves the pose alone"
        );
        // a heal (or resurrection) makes the sink stop and restores the server pose
        {
            let mut units = app.world_mut().resource_mut::<Units>();
            let unit = units.by_id.get_mut(&7).expect("the ship unit");
            unit.dead = false;
            unit.hp = max_hp;
        }
        app.world_mut().get_mut::<VehicleDamage>(root).unwrap().state = 1;
        app.update();
        assert!(app.world().get::<WreckSink>(root).is_none(), "the sink ends on a heal");
        assert_eq!(
            app.world().get::<Transform>(root).expect("the root").translation,
            sink.base.translation,
            "the server pose is restored"
        );
    }

    /// A ship that is already dead when it enters view (streamed corpse, reconnect) is built in the
    /// dead look and has no death swap to start the wreck, so `start_dead_ship_sinks` places it at the
    /// end pose (inferred) and carries the burning 25 % prefab's fire.
    #[test]
    fn an_announced_dead_ship_starts_at_its_sunk_pose_and_burns() {
        if !aa_cry::pak_available() {
            return;
        }
        let Some(db) = Db::open() else { return };
        let Some(look) = db.vehicle_look(393) else { return };
        let variants = db.vehicle_variants(393).expect("the Eznan Cutter's rows");
        let max_hp = db.slave_max_hp(21, 50).expect("Eznan Cutter template 21's MaxHealth");
        let mut unit = slave_unit_kind(7, 21, 21, 393, 0, max_hp);
        unit.dead = true;
        let variant = variants.for_state(6).to_string();
        let mut units = Units::default();
        units.apply(&aa_net::Event::UnitSpawned(Box::new(unit.clone())));
        let mut app = damage_app(
            &db,
            VehicleLook { normal: variant.clone(), wheels: look.wheels },
            unit,
            6,
            variant,
            max_hp,
            true,
        );
        app.insert_resource(units);
        app.update();
        let root = app.world().resource::<DamageRoot>().0;
        let sink = *app.world().get::<WreckSink>(root).expect("the announced dead ship sinks");
        assert!(sink.finished, "it starts at its end pose");
        let transform = *app.world().get::<Transform>(root).expect("the root");
        assert!(
            transform.translation.y < -10.0,
            "the wreck starts under the waterline: {transform:?}"
        );
        let names = effect_names(&mut app, root);
        assert!(
            names.iter().any(|name| name == "x2_effects.e_effect.ship_damage_fireflame"),
            "the announced-dead wreck burns: {names:?}"
        );
    }

    /// The Eznan Cutter's damage stages and wreck use the real prefab effects: the 25 % hull
    /// (`galleon.damaged75`) burns at its aft deck, the dead look has no `_dead_effect` explosion and
    /// carries the water-destroy burst. Its hp25/50/75 repair doodads (`slave_healing_point_doodads`)
    /// and the 16 death debris (`slave_drop_doodads`) all have readable models, so SCDoodadCreated
    /// draws them.
    #[test]
    fn the_eznan_cutters_damage_effects_and_doodads_are_readable() {
        if !aa_cry::pak_available() {
            return;
        }
        let Some(db) = Db::open() else { return };
        let damaged = super::super::data::prefab_effects_named(
            "prefab://Prefabs/galleon.xml/galleon.damaged75",
        );
        assert!(
            damaged.iter().any(|effect| {
                effect.active
                    && effect.effect.effect.eq_ignore_ascii_case(
                        "X2_EFFECTS.e_effect.ship_damage_fireflame",
                    )
            }),
            "the 25 % hull's aft-deck fire"
        );
        let dead = super::super::data::prefab_effects_named(
            "prefab://Prefabs/galleon.xml/galleon.dead",
        );
        assert!(
            !dead.iter().any(|effect| effect.name.eq_ignore_ascii_case("_dead_effect")),
            "no explosion on ship death"
        );
        assert!(dead.iter().any(|effect| {
            effect.active
                && effect
                    .effect
                    .effect
                    .eq_ignore_ascii_case("X2_EFFECTS.e_effect.caraval_water_destroy")
        }));
        // slave 21's four hp repair doodads and its 16 debris doodads (12 planks + chest + helm +
        // figurehead + net mass)
        for (template, phase) in [
            (2510, 5510),
            (2511, 5511),
            (2512, 5512),
            (2513, 5342),
            (1327, 2140),
            (1322, 2130),
            (1323, 2132),
            (1324, 2134),
            (1330, 2146),
        ] {
            let reference = db
                .doodad_phase_visual(template, phase)
                .unwrap_or_else(|| panic!("doodad {template} phase {phase} has a model"));
            let parts = super::super::housing::prefab::load(&reference);
            let models = if parts.is_empty() {
                vec![reference.clone()]
            } else {
                parts.into_iter().map(|part| part.model).collect()
            };
            for model in models {
                assert!(
                    aa_cry::mesh::Model::load(&model).is_ok(),
                    "doodad {template} model {model} is readable"
                );
            }
        }
    }

    /// The dead `_dead_effect` burst is spawned unprimed and actually retires when its simulation ends; the
    /// continuous smoke stays.
    #[test]
    fn the_dead_burst_despawns_when_it_finishes() {
        if !aa_cry::pak_available() {
            return;
        }
        use bevy::asset::AssetPlugin;
        use bevy::transform::TransformPlugin;
        let dead = "prefab://prefabs/vehicle_tractor.xml/vehicle_tractor_c.tractor_body_dead";
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            TransformPlugin,
            aa_fx::FxPlugin,
        ))
        .init_asset::<Mesh>()
        .init_asset::<StandardMaterial>()
        .init_asset::<Image>()
        .init_asset::<bevy::mesh::skinning::SkinnedMeshInverseBindposes>();
        app.world_mut().spawn((
            Camera3d::default(),
            Transform::from_xyz(0.0, 0.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
        ));
        let root = app.world_mut().spawn(Transform::default()).id();
        app.add_systems(Startup, move |mut commands: Commands| {
            spawn_state_effects(&mut commands, root, dead, true);
        });
        app.update();
        let explosion = effect_entity(
            &mut app,
            root,
            "X2_EFFECTS.housing_test.small_ship_exp",
        )
        .expect("the spawned explosion");
        // run the 6 s burst at 30x so the test does not take its real time
        app.world_mut()
            .resource_mut::<Time<Virtual>>()
            .set_relative_speed(30.0);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while app.world().get_entity(explosion).is_ok() {
            assert!(
                std::time::Instant::now() < deadline,
                "the finished burst never retired"
            );
            app.update();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(
            effect_entity(
                &mut app,
                root,
                "X2_EFFECTS.fire_smoke.burning_smoke_01"
            )
            .is_some(),
            "the continuous smoke stays"
        );
    }

    /// A slave first announced already damaged is built in its state's prefab with the same attribute-6
    /// maximum the per-frame check will use, including a template with no `unit_modifiers` row.
    #[test]
    fn a_slave_first_seen_damaged_plans_its_state() {
        let Some(db) = Db::open() else {
            eprintln!("skip: no client database");
            return;
        };
        let damaged = slave_unit(7, 60, 1008, 7_296 / 4);
        let plan = super::super::prepare::plan_unit(&db, &damaged);
        let super::super::prepare::UnitPlan::Vehicle(plan) = plan else {
            panic!("the Farm Wagon is a vehicle");
        };
        assert_eq!((plan.state, plan.max_hp), (4, 7_296));
        assert!(
            plan.look.normal.ends_with("tractor_body_damage75"),
            "{}",
            plan.look.normal
        );

        // template 105 has no unit_modifiers attribute-6 row: the formula's 4796 at level 50 alone
        let no_row = slave_unit(8, 105, 1293, 4_796 / 4);
        let plan = super::super::prepare::plan_unit(&db, &no_row);
        let super::super::prepare::UnitPlan::Vehicle(plan) = plan else {
            panic!("template 105 is a vehicle");
        };
        assert_eq!((plan.state, plan.max_hp), (4, 4_796));

        // a unit announced at 0 hp is dead already: `InitClientUnit` calls Die, so it is built dying
        let dead = slave_unit(9, 60, 1008, 0);
        let plan = super::super::prepare::plan_unit(&db, &dead);
        let super::super::prepare::UnitPlan::Vehicle(plan) = plan else {
            panic!("the Farm Wagon is a vehicle");
        };
        assert_eq!((plan.state, plan.max_hp), (5, 7_296));
        assert_eq!(plan.dying_time, 3.3);
        assert!(
            plan.look.normal.ends_with("tractor_body_dying"),
            "{}",
            plan.look.normal
        );
    }

    #[derive(Resource)]
    struct DamageRoot(Entity);

    fn damage_app(
        db: &Db,
        look: VehicleLook,
        unit: aa_net::model::Unit,
        state: u8,
        variant: String,
        max_hp: i32,
        finish: bool,
    ) -> App {
        use bevy::asset::AssetPlugin;
        use bevy::mesh::skinning::SkinnedMeshInverseBindposes;
        let dying_time = db.vehicle_dying_time(unit.model_id);
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            aa_cry::CryPlugin,
            aa_anim::AnimPlugin,
        ))
        .init_asset::<Mesh>()
        .init_asset::<StandardMaterial>()
        .init_asset::<Image>()
        .init_asset::<SkinnedMeshInverseBindposes>()
        .insert_resource(db.clone())
        .init_resource::<Units>()
        .init_resource::<VehicleSwaps>();
        app.add_systems(
            Startup,
            move |mut commands: Commands, mut cry: CryAssets, clips: Res<ClipLibrary>| {
                let loader = cry.loader();
                let pieces = prepare(&loader, &look);
                assert!(!pieces.is_empty(), "the vehicle's prefab has parts");
                let root = spawn(
                    &mut commands,
                    &mut cry,
                    &clips,
                    pieces,
                    Transform::default(),
                );
                commands.entity(root).insert((
                    RemoteUnit::announced(&unit),
                    VehicleDamage {
                        state,
                        variant: variant.clone(),
                        max_hp,
                        requested: None,
                        dying_start: None,
                        dying_time,
                        death_seen: state == 5,
                        dead_effect_played: false,
                    },
                ));
                // a freshly announced unit draws its state's standing effects (units::spawn_unit)
                spawn_state_effects(&mut commands, root, &variant, false);
                commands.insert_resource(DamageRoot(root));
            },
        );
        if finish {
            app.add_systems(
                Update,
                (
                    update_damage,
                    finish_damage_swaps,
                    start_dead_ship_sinks,
                    sink_ship_wrecks,
                )
                    .chain(),
            );
        } else {
            app.add_systems(Update, update_damage);
        }
        app
    }

    fn set_dying_time(app: &mut App, seconds: f32) {
        let root = app.world().resource::<DamageRoot>().0;
        app.world_mut()
            .get_mut::<VehicleDamage>(root)
            .expect("the vehicle root")
            .dying_time = seconds;
    }

    /// Shorten the running dying window so it ends `seconds` from now, keeping its original start time.
    fn end_dying_window_in(app: &mut App, seconds: f32) {
        let now = app.world().resource::<Time>().elapsed_secs_f64();
        let root = app.world().resource::<DamageRoot>().0;
        let mut damage = app
            .world_mut()
            .get_mut::<VehicleDamage>(root)
            .expect("the vehicle root");
        let start = damage.dying_start.expect("a running dying window");
        damage.dying_time = (now - start) as f32 + seconds.max(0.0);
    }

    fn die(app: &mut App) {
        app.world_mut()
            .resource_mut::<Units>()
            .apply(&aa_net::Event::UnitDeath { obj_id: 7 });
    }

    fn wait_for_state(app: &mut App, wanted: u8) -> VehicleDamage {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            app.update();
            let root = app.world().resource::<DamageRoot>().0;
            let damage = app
                .world()
                .get::<VehicleDamage>(root)
                .expect("the vehicle root")
                .clone();
            if damage.state == wanted {
                return damage;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the swap to state {wanted} never finished (state {})",
                damage.state
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    fn set_hp(app: &mut App, hp: i32) {
        app.world_mut()
            .resource_mut::<Units>()
            .by_id
            .get_mut(&7)
            .expect("the vehicle")
            .hp = hp;
    }

    fn parts_under(app: &mut App, root: Entity) -> Vec<Entity> {
        app.world_mut()
            .query::<(Entity, &ChildOf, &VehiclePart)>()
            .iter(app.world())
            .filter(|(_, parent, _)| parent.parent() == root)
            .map(|(entity, _, _)| entity)
            .collect()
    }

    fn effect_names(app: &mut App, root: Entity) -> Vec<String> {
        effect_flags(app, root)
            .into_iter()
            .map(|(name, _)| name)
            .collect()
    }

    fn effect_flags(app: &mut App, root: Entity) -> Vec<(String, bool)> {
        app.world_mut()
            .query::<(&ChildOf, &aa_fx::Effect, &VehicleEffect)>()
            .iter(app.world())
            .filter(|(parent, _, _)| parent.parent() == root)
            .map(|(_, effect, _)| {
                (
                    effect.name.to_ascii_lowercase(),
                    effect.despawn_when_finished,
                )
            })
            .collect()
    }

    fn effect_entity(app: &mut App, root: Entity, name: &str) -> Option<Entity> {
        app.world_mut()
            .query::<(Entity, &ChildOf, &aa_fx::Effect, &VehicleEffect)>()
            .iter(app.world())
            .find(|(_, parent, effect, _)| {
                parent.parent() == root && effect.name.eq_ignore_ascii_case(name)
            })
            .map(|(entity, _, _, _)| entity)
    }

    fn hp_update(app: &mut App, hp: i32) {
        app.world_mut()
            .resource_mut::<Units>()
            .apply(&aa_net::Event::UnitPoints { obj_id: 7, hp, mp: 0 });
    }

    /// A damage state change swaps the body off the main thread: the old parts and effects go only once the
    /// new prefab is ready, the new prefab's `_spawn_effect*` plays on a worsening, and a heal swaps back
    /// without playing one.
    #[test]
    fn update_damage_swaps_the_body_and_effects_and_a_heal_swaps_back() {
        if !aa_cry::pak_available() {
            return;
        }
        let Some(db) = Db::open() else {
            eprintln!("skip: no client database");
            return;
        };
        let Some(look) = db.vehicle_look(1008) else {
            return;
        };
        let max_hp = db.slave_max_hp(60, 50).expect("the Farm Wagon's MaxHealth");
        assert_eq!(max_hp, 7_296);
        let unit = slave_unit_kind(7, 7, 60, 1008, max_hp, max_hp);
        let mut units = Units::default();
        units.apply(&aa_net::Event::UnitSpawned(Box::new(unit.clone())));
        let mut app = damage_app(
            &db,
            look.clone(),
            unit,
            1,
            look.normal.clone(),
            max_hp,
            true,
        );
        app.insert_resource(units);
        app.update();
        let root = app.world().resource::<DamageRoot>().0;
        let old_parts = parts_under(&mut app, root);
        assert!(!old_parts.is_empty());
        let fake_effect = app.world_mut().spawn((VehicleEffect, ChildOf(root))).id();

        set_hp(&mut app, max_hp / 4);
        let damage = wait_for_state(&mut app, 4);
        assert!(
            damage.variant.ends_with("tractor_body_damage75"),
            "{}",
            damage.variant
        );
        for part in old_parts {
            assert!(
                app.world().get_entity(part).is_err(),
                "an old part outlives the swap"
            );
        }
        assert!(
            app.world().get_entity(fake_effect).is_err(),
            "an old effect outlives the swap"
        );
        assert!(
            !parts_under(&mut app, root).is_empty(),
            "the new body has parts"
        );
        let one_shots = app
            .world_mut()
            .query::<(&VehicleEffect, &aa_fx::Effect)>()
            .iter(app.world())
            .filter(|(_, effect)| effect.despawn_when_finished)
            .count();
        assert!(
            one_shots > 0,
            "the damage prefab's _spawn_effect_dmg plays once"
        );

        let limits = db.turret_limits(1008).expect("the tractor's turret block");
        app.world_mut()
            .entity_mut(root)
            .insert(super::super::turret::TurretGun {
                limits,
                bones: None,
                bones_checked: true,
            });
        set_hp(&mut app, max_hp * 3 / 4);
        let damage = wait_for_state(&mut app, 2);
        assert!(
            damage.variant.ends_with("tractor_body"),
            "{}",
            damage.variant
        );
        assert!(
            app.world()
                .get::<super::super::turret::TurretGun>(root)
                .is_none(),
            "a swap drops the stale turret bones so the next aim resolves them again"
        );
        assert_eq!(
            app.world_mut()
                .query::<(&VehicleEffect, &aa_fx::Effect)>()
                .iter(app.world())
                .filter(|(_, effect)| effect.despawn_when_finished)
                .count(),
            0,
            "healing back plays no _spawn_effect"
        );
    }

    /// While the worker reads the new prefab the old body is untouched: `update_damage` only records the request,
    /// so a slow read never shows an empty vehicle.
    #[test]
    fn update_damage_keeps_the_old_body_until_the_swap_finishes() {
        if !aa_cry::pak_available() {
            return;
        }
        let Some(db) = Db::open() else { return };
        let Some(look) = db.vehicle_look(1008) else {
            return;
        };
        let max_hp = db.slave_max_hp(60, 50).expect("the Farm Wagon's MaxHealth");
        let unit = slave_unit_kind(7, 7, 60, 1008, max_hp, max_hp);
        let mut units = Units::default();
        units.apply(&aa_net::Event::UnitSpawned(Box::new(unit.clone())));
        let mut app = damage_app(
            &db,
            look.clone(),
            unit,
            1,
            look.normal.clone(),
            max_hp,
            false,
        );
        app.insert_resource(units);
        app.update();
        let root = app.world().resource::<DamageRoot>().0;
        let old_parts = parts_under(&mut app, root);
        assert!(!old_parts.is_empty());

        set_hp(&mut app, max_hp / 4);
        app.update();
        for part in &old_parts {
            assert!(
                app.world().get_entity(*part).is_ok(),
                "the old body is despawned before the new prefab is ready"
            );
        }
        let damage = app
            .world()
            .get::<VehicleDamage>(root)
            .expect("the vehicle root");
        assert_eq!((damage.state, damage.requested), (1, Some(4)));
        assert_eq!(
            parts_under(&mut app, root),
            old_parts,
            "no new parts before they are ready"
        );
    }

    fn synthetic_turret_bones(app: &mut App, root: Entity) -> (Entity, Entity, Entity) {
        let bone = |name: &str| aa_cry::Bone {
            name: name.into(),
            controller_id: name
                .bytes()
                .fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b))),
            parent: None,
            limb_id: 0,
            bind_model: default(),
            bind_local: default(),
            physics: [default(); 2],
        };
        let rig = std::sync::Arc::new(aa_anim::Rig::new(aa_cry::Skeleton {
            bones: vec![bone("bone_yaw"), bone("bone_pitch")],
        }));
        let holder = app
            .world_mut()
            .spawn((ChildOf(root), Transform::default()))
            .id();
        let yaw = app
            .world_mut()
            .spawn((ChildOf(holder), Transform::default()))
            .id();
        let pitch = app
            .world_mut()
            .spawn((ChildOf(holder), Transform::default()))
            .id();
        app.world_mut()
            .entity_mut(holder)
            .insert(Animator::new(rig, vec![yaw, pitch]));
        (holder, yaw, pitch)
    }

    /// After a swap the turret's cached `bone_yaw` / `bone_pitch` entities are gone; with the cache reset,
    /// `aim_turrets` resolves the new prefab's bones on the next frame.
    #[test]
    fn a_swap_reresolves_the_turret_bones() {
        if !aa_cry::pak_available() {
            return;
        }
        let Some(db) = Db::open() else { return };
        let Some(look) = db.vehicle_look(1008) else {
            return;
        };
        // The turret gate reads the slave kind from the template id: template 73 is a real
        // kind-7 slave on the same tractor model (1008), template 60 is a kind-6 farm wagon.
        let max_hp = db.slave_max_hp(73, 50).expect("the cotton-field cart's MaxHealth");
        let unit = slave_unit_kind(7, 7, 73, 1008, max_hp, max_hp);
        let mut units = Units::default();
        units.apply(&aa_net::Event::UnitSpawned(Box::new(unit.clone())));
        let mut app = damage_app(
            &db,
            look.clone(),
            unit,
            1,
            look.normal.clone(),
            max_hp,
            true,
        );
        app.insert_resource(units);
        app.update();
        let root = app.world().resource::<DamageRoot>().0;
        let (old_holder, old_yaw, old_pitch) = synthetic_turret_bones(&mut app, root);
        let limits = db.turret_limits(1008).expect("the tractor's turret block");
        app.world_mut().entity_mut(root).insert((
            super::super::turret::TurretGun {
                limits,
                bones: Some(super::super::turret::BonePair {
                    yaw: old_yaw,
                    pitch: old_pitch,
                    bind_yaw: Quat::IDENTITY,
                    bind_pitch: Quat::IDENTITY,
                    bind_aim: Default::default(),
                }),
                bones_checked: true,
            },
            super::super::turret::TurretTarget(aa_net::TurretAim {
                pitch: 0.2,
                yaw: 0.3,
            }),
        ));

        set_hp(&mut app, max_hp / 4);
        wait_for_state(&mut app, 4);
        assert!(
            app.world()
                .get::<super::super::turret::TurretGun>(root)
                .is_none()
        );
        app.world_mut().entity_mut(old_holder).despawn();
        let (_new_holder, new_yaw, new_pitch) = synthetic_turret_bones(&mut app, root);
        app.add_systems(Update, super::super::turret::aim_turrets);
        app.update();
        let gun = app
            .world()
            .get::<super::super::turret::TurretGun>(root)
            .expect("the new turret cache");
        let bones = gun.bones.expect("the new prefab's bone_yaw / bone_pitch");
        assert_eq!((bones.yaw, bones.pitch), (new_yaw, new_pitch));
        assert_ne!(bones.yaw, old_yaw, "the new bones, not the removed ones");
    }

    fn tractor_app(finish: bool) -> Option<(App, Db, i32)> {
        if !aa_cry::pak_available() {
            return None;
        }
        let db = Db::open()?;
        let look = db.vehicle_look(1008)?;
        let max_hp = db.slave_max_hp(60, 50).expect("the Farm Wagon's MaxHealth");
        let unit = slave_unit_kind(7, 7, 60, 1008, max_hp, max_hp);
        let mut units = Units::default();
        units.apply(&aa_net::Event::UnitSpawned(Box::new(unit.clone())));
        let mut app = damage_app(&db, look.clone(), unit, 1, look.normal, max_hp, finish);
        app.insert_resource(units);
        app.update();
        Some((app, db, max_hp))
    }

    /// `SCUnitDeath` enters dying (5) with the `dying` prefab, keeps it for `models.dying_time`, then switches
    /// to dead (6) (`FUN_390e7ad0`, `FUN_390eaac0`).
    #[test]
    fn a_death_enters_dying_for_its_time_then_switches_to_dead() {
        let Some((mut app, _db, _max_hp)) = tractor_app(true) else {
            return;
        };
        // long enough that a slow prefab build cannot expire the window before state 5 is observed
        set_dying_time(&mut app, 60.0);
        die(&mut app);
        let dying = wait_for_state(&mut app, 5);
        assert!(
            dying.variant.ends_with("tractor_body_dying"),
            "{}",
            dying.variant
        );
        // a 5 s window still shows dying a fifth of the way in, then the shortened window expires to dead
        end_dying_window_in(&mut app, 5.0);
        app.update();
        std::thread::sleep(std::time::Duration::from_millis(1000));
        app.update();
        let root = app.world().resource::<DamageRoot>().0;
        assert_eq!(
            app.world().get::<VehicleDamage>(root).expect("the root").state,
            5,
            "dying does not end early"
        );
        end_dying_window_in(&mut app, 0.2);
        let dead = wait_for_state(&mut app, 6);
        assert!(
            dead.variant.ends_with("tractor_body_dead"),
            "{}",
            dead.variant
        );
    }

    /// A model without a `dying` row (the real sigetank, model 1465) dies in the next lower prefab
    /// (`FUN_397a7980` walks 5 -> 4 -> 3 -> 2 -> 1).
    #[test]
    fn a_death_without_a_dying_prefab_uses_the_damaged_one() {
        if !aa_cry::pak_available() {
            return;
        }
        let Some(db) = Db::open() else { return };
        let Some(look) = db.vehicle_look(1465) else {
            return;
        };
        let variants = db.vehicle_variants(1465).expect("the sigetank's state rows");
        assert!(variants.dying.is_none());
        let max_hp = db.slave_max_hp(60, 50).expect("a maximum");
        let unit = slave_unit_kind(7, 7, 60, 1465, max_hp, max_hp);
        let mut units = Units::default();
        units.apply(&aa_net::Event::UnitSpawned(Box::new(unit.clone())));
        let mut app = damage_app(&db, look.clone(), unit, 1, look.normal, max_hp, true);
        app.insert_resource(units);
        app.update();
        set_dying_time(&mut app, 60.0);
        die(&mut app);
        let dying = wait_for_state(&mut app, 5);
        assert!(
            dying.variant.ends_with("sigetank_rus_a_damage75"),
            "{}",
            dying.variant
        );
    }

    /// A model with neither a `dying` nor a `dead` row keeps its `normal` prefab through 5 and 6. The early
    /// `variant == damage.variant` path must still end the dying window, or a revive can never leave 6.
    #[test]
    fn a_dead_model_without_dead_or_dying_rows_heals_again() {
        if !aa_cry::pak_available() {
            return;
        }
        let Some(db) = Db::open() else { return };
        let Some(look) = db.vehicle_look(657) else {
            return;
        };
        let variants = db.vehicle_variants(657).expect("the airship's state rows");
        assert!(variants.dying.is_none() && variants.dead.is_none());
        let max_hp = db.slave_max_hp(60, 50).expect("a maximum");
        let unit = slave_unit_kind(7, 7, 60, 657, max_hp, max_hp);
        let mut units = Units::default();
        units.apply(&aa_net::Event::UnitSpawned(Box::new(unit.clone())));
        let mut app = damage_app(
            &db,
            look.clone(),
            unit,
            1,
            look.normal.clone(),
            max_hp,
            true,
        );
        app.insert_resource(units);
        app.update();
        set_dying_time(&mut app, 60.0);
        die(&mut app);
        let dying = wait_for_state(&mut app, 5);
        assert_eq!(dying.variant, look.normal, "no dying prefab: normal");
        end_dying_window_in(&mut app, 0.2);
        wait_for_state(&mut app, 6);
        hp_update(&mut app, max_hp);
        let healed = wait_for_state(&mut app, 1);
        assert_eq!(healed.variant, look.normal);
        assert!(
            app.world()
                .get::<VehicleDamage>(app.world().resource::<DamageRoot>().0)
                .expect("the root")
                .dying_start
                .is_none(),
            "the heal cleared the stale dying window"
        );
    }

    /// The polled HP-0 update may show dead first (`FUN_390eb610` recomputes from HP); the later death still
    /// forces dying, then dead (`0x390e7ad0` has no state guard).
    #[test]
    fn an_hp_zero_update_before_the_death_still_enters_dying() {
        let Some((mut app, _db, _max_hp)) = tractor_app(true) else {
            return;
        };
        set_dying_time(&mut app, 60.0);
        hp_update(&mut app, 0);
        let polled = wait_for_state(&mut app, 6);
        assert!(
            polled.variant.ends_with("tractor_body_dead"),
            "{}",
            polled.variant
        );
        die(&mut app);
        let dying = wait_for_state(&mut app, 5);
        assert!(
            dying.variant.ends_with("tractor_body_dying"),
            "{}",
            dying.variant
        );
        end_dying_window_in(&mut app, 0.2);
        wait_for_state(&mut app, 6);
    }

    /// `dying_time == 0` switches to dead on the update after the death, not in the entry tick
    /// (`FUN_390eaac0` checks `limit <= elapsed`).
    #[test]
    fn a_zero_dying_time_lasts_one_tick() {
        let Some((mut app, _db, _max_hp)) = tractor_app(false) else {
            return;
        };
        set_dying_time(&mut app, 0.0);
        die(&mut app);
        app.update();
        let root = app.world().resource::<DamageRoot>().0;
        let damage = app.world().get::<VehicleDamage>(root).expect("the root");
        assert_eq!(damage.requested, Some(5), "the death tick enters dying");
        app.update();
        let damage = app.world().get::<VehicleDamage>(root).expect("the root");
        assert_eq!(damage.requested, Some(6), "the next update expires dying");
    }

    /// A vehicle announced at 0 hp is built dying (`InitClientUnit` / `OnModelChanged` call Die), and its
    /// dying window then expires to dead like any other death.
    #[test]
    fn a_vehicle_announced_at_zero_hp_starts_dying() {
        if !aa_cry::pak_available() {
            return;
        }
        let Some(db) = Db::open() else { return };
        let unit = slave_unit(7, 60, 1008, 0);
        let super::super::prepare::UnitPlan::Vehicle(plan) =
            super::super::prepare::plan_unit(&db, &unit)
        else {
            panic!("the Farm Wagon is a vehicle");
        };
        assert_eq!(plan.state, 5);
        assert!(plan.look.normal.ends_with("tractor_body_dying"));
        let mut units = Units::default();
        units.apply(&aa_net::Event::UnitSpawned(Box::new(unit.clone())));
        let mut app = damage_app(
            &db,
            plan.look.clone(),
            unit,
            plan.state,
            plan.look.normal.clone(),
            plan.max_hp,
            true,
        );
        app.insert_resource(units);
        app.update();
        set_dying_time(&mut app, 60.0);
        let dying = wait_for_state(&mut app, 5);
        assert!(dying.variant.ends_with("tractor_body_dying"));
        end_dying_window_in(&mut app, 0.2);
        let dead = wait_for_state(&mut app, 6);
        assert!(dead.variant.ends_with("tractor_body_dead"));
    }
}
