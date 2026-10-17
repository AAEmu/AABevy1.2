//! Combat visuals: what a skill looks like on the units that use it and the ones it hits.
//!
//! The server only reports results ([`aa_net::Event`]): a skill started, a skill fired, the steps of a plot, damage, death.
//! This module turns them into what the original client shows, from the data `skills` describes (see [`skills`] and
//! `docs/aa-client-combat.md`):
//!
//! * **Clips.** A cast loops the skill's `start_anim` clip until it fires; firing plays the one-shot `fire_anim` clip (the id
//!   the server sends in `SCSkillFired`, else the skill's, the two-handed one for a two-handed weapon); every step of a plot
//!   plays the clip of its plot effect. The unit is [`Acting`] meanwhile ([`super::anim::drive_locomotion`] leaves it alone) and
//!   returns to its locomotion clip afterwards. A unit that moves plays the clip on its upper body instead (the `move_ub`
//!   variant of the `anims` row, else the clip itself, on aa-anim's upper body layer) and keeps running or walking below it.
//!   The local player turns to the target.
//! * **Effects.** The skill's effect group starts its particle effects (aa-fx) at the moments the data names ([`skills::ev`]):
//!   `form` while casting, `throw` / `launch` when it fires (at the release moment of the clip, from the model's anim events,
//!   [`clips`]), `hit` on the target when the damage lands, at the bones the effect items name. Effects that last are held until
//!   the moment they end at.
//! * **Projectiles.** A skill with a projectile (Shoot Arrow's arrow, a fireball) sends it from the caster's hand to the target's
//!   hit point as an effect that flies for the time the server says the effects take to land (`delay_ms` of `SCSkillFired`),
//!   which is when the damage arrives and the hit effect plays.
//! * **Reactions.** A unit that takes damage plays its hit clip; a unit that dies plays `dead_01` and stays down ([`Dead`]) until
//!   the server removes it. Units that are announced dead (a corpse in view) lie down at once.
//!
//! Everything is looked up once per skill and cached (`SkillBook`); a frame costs a few comparisons plus the effects in flight.

mod clips;
mod skills;

use std::collections::HashMap;
use std::sync::Arc;

use aa_anim::{Animator, ClipLibrary};
use aa_cry::scene::{CryAssets, LoadedModel, MaterialHook, ModelOptions, spawn_loaded};
use aa_fx::{Effect, FxLibraries, LibStatus};
use aa_net::skill::{hit_type, target_kind};
use aa_net::state::Units;
use aa_net::{Command, Event, MoveCommand, NetCommand, NetEvent, Pos};
use aa_world::cry_to_bevy;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use super::anim::{Locomotion, Motion, bind_clip};
use super::player::{LocalPlayer, Mover, approach_angle};
use super::units::RemoteUnit;
use super::weapons::{Drawn, hold_weapons};
use crate::Screen;
use skills::{FxItem, Projectile, SkillBook, bone_names, ev};

/// What a unit is busy with, so locomotion does not take its clip away.
#[derive(Component, Clone, Copy, Debug)]
pub struct Acting {
    /// When it is over (seconds on the app clock).
    pub until: f64,
    /// The clip plays on the upper body (an animator layer) while locomotion keeps the legs: the unit was moving.
    pub layered: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    /// The looping cast clip.
    Cast,
    /// A fire / attack clip.
    Attack,
    /// A hit reaction (never interrupts the others).
    Hurt,
}

/// The unit is dead: it plays its death clip once and stays on its last frame until the server removes it.
#[derive(Component, Clone, Copy, Debug)]
pub struct Dead {
    /// When it died (seconds on the app clock); a corpse that was announced dead has a time long ago.
    pub at: f64,
}

/// The death clip was started.
#[derive(Component)]
struct Corpse;

/// Where the local player turns for a moment (while it acts, or has just asked to).
#[derive(Component)]
struct FaceTarget {
    entity: Option<Entity>,
    pos: Option<Vec3>,
    until: f64,
}

/// A particle effect of a skill in the world: follows its bone for as long as it lives.
#[derive(Component)]
struct Fx {
    /// The unit and the bone entity the effect rides, with the offset in the unit's frame.
    follow: Option<(Entity, Entity, Vec3)>,
    /// Keep the orientation it was spawned with (a hit effect turned towards the attacker).
    fixed_rotation: bool,
    /// Seconds it may still live: a safety net for effects that never finish.
    ttl: f32,
}

/// A projectile in flight: an anchor entity the effects of the projectile ride.
#[derive(Component)]
struct Flying {
    from: Vec3,
    to_unit: Option<(Entity, u8)>,
    to_pos: Vec3,
    elapsed: f32,
    duration: f32,
}

/// The skill uses in progress (by timeline id) and what is waiting for its time.
#[derive(Resource, Default)]
struct Combat {
    casts: HashMap<u16, Cast>,
    timers: Vec<Timer>,
    /// When the last scheduled hit on a unit lands (a death waits for it).
    last_hit: HashMap<u32, f64>,
    /// A skill the server refused because the player did not face the target: when to ask again (once the player turned).
    retry: Option<Retry>,
    /// The meshes of effects that emit geometry (the arrow of a bow shot), by effect name; `None` when there is none.
    geometry: HashMap<String, Option<Arc<LoadedModel>>>,
}

struct Retry {
    at: f64,
    skill: u32,
    target: u32,
}

#[derive(Default)]
struct Cast {
    caster: u32,
    target: u32,
    target_pos: Option<Vec3>,
    started: f64,
    fired: Option<f64>,
    /// Seconds from the fire until the effects land (`SCSkillFired`).
    delay: f64,
    /// Seconds into the fire clip where the server's timeline sits (`combat_sync`).
    sync: f64,
    /// Effects held until the moment they end at.
    held: Vec<(Entity, u8)>,
    /// The effects that play on the target when the damage lands (the plot step's group, else the skill's).
    hit_fx: Arc<Vec<FxItem>>,
    /// The cast clip loops.
    casting: bool,
}

struct Timer {
    at: f64,
    what: Later,
}

enum Later {
    /// A skill effect of a cast (held ones are tracked in the cast).
    Fx { item: FxItem, tl: u16, on: Entity, toward: Option<Vec3> },
    /// A projectile leaves.
    Launch { projectile: Projectile, caster: Entity, target: Option<Entity>, target_pos: Option<Vec3>, duration: f32 },
    /// Damage lands.
    Hit { caster: Option<Entity>, target: Entity, items: Arc<Vec<FxItem>>, hurt: Option<&'static str> },
    /// A unit dies.
    Die { obj: u32 },
}

/// `AA_COMBAT_LOG=1`: one line per combat event.
fn verbose() -> bool {
    static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::var_os("AA_COMBAT_LOG").is_some())
}

/// `AA_COMBAT_FULLBODY=1`: units on the move play their action clips on the whole body, as before the upper body layer
/// (to compare shots).
fn full_body_only() -> bool {
    static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::var_os("AA_COMBAT_FULLBODY").is_some())
}

/// `AA_COMBAT_SLOWMO=<speed>`: the game clock slows to that speed (0.2 = five times slower) from the first skill the local player
/// fires, so a shot can be caught mid-flight (`--shot-after` counts real seconds).
fn slowmo() -> Option<f32> {
    static V: std::sync::OnceLock<Option<f32>> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::var("AA_COMBAT_SLOWMO").ok().and_then(|v| v.parse().ok()))
}

/// `AA_COMBAT_SHOT=<png>@<start|fire|fire2|plot|hit|death>+<seconds>`: a screenshot that many game seconds after the first such event (the local
/// player's skill started, the local
/// player fired (`fire2`: for the second time; `plot`: a step of the player's plot skill), a unit other than the player was hit, a unit died), then exit. Catches a flying arrow without guessing the
/// network latency.
#[derive(Resource)]
struct DevShot {
    path: String,
    on: String,
    after: f64,
    at: Option<f64>,
    taken: Option<f64>,
    /// How many times the player fired (`fire2` is the second time).
    fires: u32,
}

impl DevShot {
    fn from_env() -> Option<DevShot> {
        let spec = std::env::var("AA_COMBAT_SHOT").ok()?;
        let (path, rest) = spec.split_once('@')?;
        let (on, after) = rest.split_once('+')?;
        Some(DevShot { path: path.to_string(), on: on.to_string(), after: after.parse().ok()?, at: None, taken: None, fires: 0 })
    }

    fn event(&mut self, on: &str, now: f64) {
        let on = if on == "fire" {
            self.fires += 1;
            if self.fires == 2 { "fire2" } else { on }
        } else {
            on
        };
        if self.on == on && self.at.is_none() {
            self.at = Some(now + self.after);
        }
    }
}

fn dev_shot(mut commands: Commands, time: Res<Time>, real: Res<Time<Real>>, mut shot: ResMut<DevShot>, mut exit: MessageWriter<AppExit>) {
    let now = time.elapsed_secs_f64();
    match (shot.at, shot.taken) {
        (Some(at), None) if now >= at => {
            if let Some(dir) = std::path::Path::new(&shot.path).parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            commands.spawn(bevy::render::view::screenshot::Screenshot::primary_window()).observe(bevy::render::view::screenshot::save_to_disk(shot.path.clone()));
            shot.taken = Some(real.elapsed_secs_f64());
            info!("combat: screenshot -> {}", shot.path);
        }
        (_, Some(taken)) if real.elapsed_secs_f64() - taken > 1.5 && std::path::Path::new(&shot.path).exists() => {
            exit.write(AppExit::Success);
        }
        _ => {}
    }
}

pub fn plugin(app: &mut App) {
    if let Some(shot) = DevShot::from_env() {
        app.insert_resource(shot).add_systems(Update, dev_shot);
    }
    match SkillBook::open() {
        Some(book) => {
            app.insert_resource(book);
        }
        None => warn!("combat: the game database is missing: no combat visuals"),
    }
    app.init_resource::<Combat>()
        .add_systems(OnExit(Screen::InGame), reset)
        .add_systems(
            Update,
            (
                (handle_events, retry_cast, run_timers, mark_corpses, ensure_corpses, end_actions, release_layers, fly_projectiles, hold_weapons)
                    .chain()
                    .before(super::anim::drive_locomotion)
                    .run_if(resource_exists::<SkillBook>),
                turn_to_target.before(super::player::control_player),
            )
                .run_if(super::world_screens),
        )
        .add_systems(PostUpdate, follow_fx.before(TransformSystems::Propagate));
}

/// Leaving the game: nothing of the fights stays.
#[allow(clippy::type_complexity)]
fn reset(mut commands: Commands, mut combat: ResMut<Combat>, fx: Query<Entity, Or<(With<Fx>, With<Flying>)>>) {
    *combat = Combat::default();
    for e in &fx {
        commands.entity(e).despawn();
    }
}

/// The scene as the combat code needs it: units by object id, their clips and bones, spawning.
#[derive(SystemParam)]
struct Stage<'w, 's> {
    commands: Commands<'w, 's>,
    clips: Res<'w, ClipLibrary>,
    book: Res<'w, SkillBook>,
    net_units: Res<'w, Units>,
    remote: Query<'w, 's, (Entity, &'static RemoteUnit)>,
    local: Query<'w, 's, (Entity, &'static LocalPlayer)>,
    #[allow(clippy::type_complexity)]
    actors: Query<'w, 's, (&'static mut Animator, Option<&'static Locomotion>, Option<&'static Acting>, Option<&'static Motion>, Has<Dead>)>,
    xforms: Query<'w, 's, &'static GlobalTransform>,
}

impl Stage<'_, '_> {
    fn entity(&self, obj: u32) -> Option<Entity> {
        if obj == 0 {
            return None;
        }
        self.remote.iter().find(|(_, r)| r.obj_id == obj).map(|(e, _)| e).or_else(|| self.local.iter().find(|(_, l)| l.obj_id == obj).map(|(e, _)| e))
    }

    fn is_local(&self, e: Entity) -> bool {
        self.local.contains(e)
    }

    /// Where a unit is (Bevy world), from its entity or, when it has none (too far to be drawn), from the server's position.
    fn point(&self, obj: u32) -> Option<Vec3> {
        match self.entity(obj) {
            Some(e) => self.xforms.get(e).ok().map(|g| g.translation()),
            None => self.net_units.get(obj).map(|u| cry_to_bevy(Vec3::new(u.pos.x, u.pos.y, u.pos.z))),
        }
    }

    /// The bone entity of an effect bone id (`None` for the origin and for rigs without such a bone).
    fn bone(&self, unit: Entity, id: u8) -> Option<Entity> {
        let (animator, ..) = self.actors.get(unit).ok()?;
        bone_names(id).iter().find_map(|name| {
            let i = animator.rig.skeleton.bones.iter().position(|b| b.name == *name)?;
            animator.bones.get(i).copied()
        })
    }

    /// World position of an effect bone of a unit: the bone, else the unit's origin raised to `fallback_height`.
    fn bone_point(&self, unit: Entity, id: u8, fallback_height: f32) -> Option<Vec3> {
        match self.bone(unit, id).and_then(|b| self.xforms.get(b).ok()) {
            Some(g) => Some(g.translation()),
            None => self.xforms.get(unit).ok().map(|g| g.translation() + Vec3::Y * fallback_height * g.scale().y),
        }
    }

    /// The model's clip `name` bound to the unit's animator and its length.
    fn clip(&self, unit: Entity, name: &str) -> Option<(aa_anim::BoundClip, Arc<aa_anim::Cal>)> {
        let (animator, loco, ..) = self.actors.get(unit).ok()?;
        let cal = loco?.clips.cal.clone();
        let clip = bind_clip(&animator.rig, &cal, &self.clips, name)?;
        Some((clip, cal))
    }

    /// The upper body variant of clip `name` for the unit's model: the `move_ub` clip of its `anims` row (`bow_co_sk_launch_ub`),
    /// which the `.cal` of most models names `..._mub`, else as given. `None` when the row or the model has none.
    fn upper_clip(&self, unit: Entity, name: &str) -> Option<aa_anim::BoundClip> {
        let upper = self.book.upper_clip(name)?;
        skills::upper_candidates(&upper).find_map(|n| self.clip(unit, &n).map(|(clip, _)| clip))
    }

    /// The anim events of clip `name` of the unit's model.
    fn events(&self, unit: Entity, name: &str) -> clips::ClipEvents {
        let Some(cal) = self.actors.get(unit).ok().and_then(|(_, loco, ..)| loco.map(|l| l.clips.cal.clone())) else { return clips::ClipEvents::default() };
        clips::ClipEvents::of(&cal, name)
    }

    /// Plays a clip on a unit as an action. `hold` is how long a looping clip may last. Returns the clip's length, `None` when the
    /// unit cannot (no such clip, dead, busy with something a hit reaction must not interrupt).
    fn play(&mut self, unit: Entity, name: &str, kind: Act, looping: bool, hold: f64, now: f64) -> Option<f32> {
        // a unit on the move plays the clip on its upper body and keeps running below it
        let mask = {
            let (_, loco, acting, motion, dead) = self.actors.get(unit).ok()?;
            // a hit reaction never interrupts an action or a run
            if dead || loco.is_none() || (kind == Act::Hurt && (acting.is_some() || motion.is_some_and(|m| m.speed > 0.3))) {
                return None;
            }
            let moving = kind != Act::Hurt && !full_body_only() && motion.is_some_and(|m| m.speed > 0.3);
            loco.filter(|l| moving && l.clips.can_move()).and_then(|l| l.clips.upper.clone())
        };
        let layered = mask.is_some();
        let clip = if mask.is_some() { self.upper_clip(unit, name) } else { None };
        let Some(clip) = clip.or_else(|| self.clip(unit, name).map(|(clip, _)| clip)) else {
            if verbose() {
                info!("combat: unit has no clip {name}");
            }
            return None;
        };
        let duration = clip.clip.duration;
        let (mut animator, ..) = self.actors.get_mut(unit).ok()?;
        if let Some(mask) = mask {
            animator.play_layer(mask, clip, looping, 0.1);
        } else {
            animator.stop_layer(0.0);
            animator.play(clip, looping, 0.08);
            animator.speed = 1.0;
            animator.random_idle = None;
        }
        // a hit reaction is a flinch: the start of its clip, then locomotion fades back in
        let until = now + if looping { hold } else if kind == Act::Hurt { f64::from(duration).min(0.5) } else { f64::from(duration).min(4.0) };
        self.commands.entity(unit).insert(Acting { until, layered });
        if kind != Act::Hurt && !name.starts_with("fist_") && !name.starts_with("all_") {
            // the weapon the clip is for comes out of its sheath and stays out for a while
            self.commands.entity(unit).insert(Drawn { until: until + 4.0, bow: name.starts_with("bow_") });
        }
        if verbose() {
            info!("combat: {name} on {unit:?} ({kind:?}, {duration:.2} s{})", if layered { ", upper body" } else { "" });
        }
        Some(duration)
    }

    /// Starts one effect item on a unit. `toward` is the other party (a hit effect turns to it). Returns the effect entity.
    fn spawn_item(&mut self, item: &FxItem, on: Entity, toward: Option<Vec3>, hold: bool) -> Option<Entity> {
        let bone = self.bone(on, item.bone);
        let unit_xf = *self.xforms.get(on).ok()?;
        let (scale, rotation, _) = unit_xf.to_scale_rotation_translation();
        let anchor = match bone.and_then(|b| self.xforms.get(b).ok()) {
            Some(g) => g.translation(),
            // no bone: at the feet (id 0) or a chest high point (a bone this rig lacks)
            None => unit_xf.translation() + if item.bone == 0 { Vec3::ZERO } else { Vec3::Y * scale.y },
        };
        let offset = cry_to_bevy(item.offset);
        let mut transform = Transform { translation: anchor + rotation * offset * scale.y, rotation, scale: Vec3::splat(scale.y) };
        let mut fixed_rotation = false;
        if item.axis == 4
            && let Some(from) = toward
        {
            let dir = Vec3::new(from.x - anchor.x, 0.0, from.z - anchor.z).normalize_or_zero();
            if dir != Vec3::ZERO {
                transform.rotation = Transform::IDENTITY.looking_to(dir, Vec3::Y).rotation;
                fixed_rotation = true;
            }
        }
        let effect = Effect { scale: item.scale, prime: false, looping: hold, despawn_when_finished: !hold, ..Effect::new(item.effect.clone()) };
        let follow = bone.map(|b| (on, b, offset));
        let id = self.commands.spawn((Name::new(format!("fx {}", item.effect)), effect, transform, Fx { follow, fixed_rotation, ttl: if hold { 12.0 } else { 6.0 } })).id();
        Some(id)
    }
}

/// The loaders the projectile meshes need.
#[derive(SystemParam)]
struct Models<'w> {
    cry: CryAssets<'w>,
    libs: ResMut<'w, FxLibraries>,
}

impl Combat {
    /// The mesh a particle effect emits (`Geometry` of its definition), as a glowing unlit model; loaded once.
    fn geometry_of(&mut self, models: &mut Models, effect: &str) -> Option<Arc<LoadedModel>> {
        if let Some(known) = self.geometry.get(effect) {
            return known.clone();
        }
        let (library, name) = aa_fx::split_name(effect)?;
        models.libs.request(library);
        let LibStatus::Ready(lib) = models.libs.status(library) else { return None };
        let model = lib.get(name).and_then(|def| def.geometry()).and_then(|(path, _material)| {
            let hook = MaterialHook::new(|_, m| {
                // what the particle does with it: an additive glow
                m.unlit = true;
                m.alpha_mode = AlphaMode::Add;
                m.base_color_texture = None;
                m.base_color = Color::linear_rgb(2.2, 1.8, 1.0);
                m.double_sided = true;
                m.cull_mode = None;
            });
            let options = ModelOptions { material_hook: Some(hook), ..default() };
            models.cry.load_model(path, &options).ok()
        });
        self.geometry.insert(effect.to_string(), model.clone());
        model
    }

    fn after(&mut self, at: f64, what: Later) {
        self.timers.push(Timer { at, what });
    }

    /// Releases (despawns) the held effects whose end moment is `moment` or earlier.
    fn release(&mut self, commands: &mut Commands, tl: u16, moment: u8) {
        let Some(cast) = self.casts.get_mut(&tl) else { return };
        cast.held.retain(|&(e, end)| {
            if end <= moment {
                commands.entity(e).try_despawn();
                false
            } else {
                true
            }
        });
    }

    fn cast_for(&mut self, tl: u16, caster: u32, target: u32, now: f64) -> &mut Cast {
        if self.casts.len() > 256 {
            self.casts.retain(|_, c| now - c.started < 20.0);
        }
        self.casts.entry(tl).or_insert_with(|| Cast { caster, target, started: now, ..Cast::default() })
    }
}

/// Starts the items of `moments` of a cast now, or `delay` seconds from now.
#[allow(clippy::too_many_arguments)]
fn start_items(st: &mut Stage, combat: &mut Combat, tl: u16, items: &[FxItem], moments: &[u8], caster: Entity, target: Option<Entity>, now: f64, delay: f64, skip_short: bool) {
    let toward_caster = st.xforms.get(caster).ok().map(|g| g.translation());
    let toward_target = target.and_then(|t| st.xforms.get(t).ok()).map(|g| g.translation());
    for item in items.iter().filter(|i| moments.contains(&i.start)) {
        // buff visuals (damage over time, auras) belong to the buff, not to the skill
        if item.effect.to_ascii_lowercase().contains("_dot") {
            continue;
        }
        // an instant skill never shows what only lives until it fires
        if skip_short && item.end <= ev::LAUNCH && item.end > item.start {
            continue;
        }
        let (on, toward) = if item.on_target { (target, toward_caster) } else { (Some(caster), toward_target) };
        let Some(on) = on else { continue };
        combat.after(now + delay, Later::Fx { item: item.clone(), tl, on, toward });
    }
}

/// Where a projectile or a hit goes: the unit, else the server's position of it, else a spot.
fn target_of(st: &Stage, obj: u32, pos: Option<Vec3>) -> (Option<Entity>, Option<Vec3>) {
    (st.entity(obj), pos.or_else(|| st.point(obj)))
}

/// Server events to clips and effects.
#[allow(clippy::too_many_arguments)]
fn handle_events(
    mut events: MessageReader<NetEvent>,
    time: Res<Time>,
    mut st: Stage,
    mut combat: ResMut<Combat>,
    mut clock: ResMut<Time<Virtual>>,
    mut shot: Option<ResMut<DevShot>>,
) {
    let now = time.elapsed_secs_f64();
    for NetEvent(event) in events.read() {
        if verbose() && matches!(event, Event::SkillStarted(_) | Event::SkillFired(_) | Event::PlotEvent(_) | Event::UnitDamaged(_) | Event::UnitDeath { .. } | Event::SkillEnded { .. }) {
            info!("combat: {event:?}");
        }
        match event {
            Event::SkillStarted(s) if s.failed() => {
                // not facing the target: turn, tell the server, and ask once more (the real client turns on its own)
                let own = st.local.iter().any(|(_, l)| l.obj_id == s.caster.obj_id);
                if own && s.result == aa_net::skill::result::OUTOF_ANGLE && s.target.obj_id != 0 && combat.retry.is_none() {
                    let target = st.entity(s.target.obj_id);
                    if let Some(me) = st.entity(s.caster.obj_id) {
                        face(&mut st, me, target, None, now);
                    }
                    combat.retry = Some(Retry { at: now + 0.2, skill: s.skill_id, target: s.target.obj_id });
                }
            }
            Event::SkillStarted(s) => {
                let Some(caster) = st.entity(s.caster.obj_id) else { continue };
                if let Some(shot) = shot.as_mut()
                    && st.is_local(caster)
                {
                    shot.event("start", now);
                }
                let Some(visual) = st.book.skill(s.skill_id) else { continue };
                let target_obj = s.target.obj_id;
                let target_pos = s.target.pos.map(|p| cry_to_bevy(Vec3::new(p.x, p.y, p.z)));
                let (target, target_pos) = target_of(&st, target_obj, target_pos);
                let cast = combat.cast_for(s.tl, s.caster.obj_id, target_obj, now);
                cast.target_pos = target_pos;
                cast.hit_fx = visual.fx.clone();
                if s.cast_ms > 0 {
                    // the cast: its clip loops until the skill fires; the charge-up effects play meanwhile
                    cast.casting = true;
                    let hold = f64::from(s.cast_ms) / 1000.0 + 1.0;
                    if let Some(name) = &visual.start_anim {
                        st.play(caster, name, Act::Cast, true, hold, now);
                    }
                    face(&mut st, caster, target, target_pos, now);
                    if !visual.plot {
                        start_items(&mut st, &mut combat, s.tl, &visual.fx, &[ev::START, ev::FORM], caster, target, now, 0.0, false);
                    }
                }
            }
            Event::SkillFired(f) => {
                if let Some(speed) = slowmo()
                    && st.local.iter().any(|(_, l)| l.obj_id == f.caster.obj_id)
                {
                    clock.set_relative_speed(speed);
                }
                if let Some(shot) = shot.as_mut()
                    && st.local.iter().any(|(_, l)| l.obj_id == f.caster.obj_id)
                {
                    shot.event("fire", now);
                }
                let Some(caster) = st.entity(f.caster.obj_id) else { continue };
                let Some(visual) = st.book.skill(f.skill_id) else { continue };
                let target_pos = f.target.pos.map(|p| cry_to_bevy(Vec3::new(p.x, p.y, p.z)));
                let target_obj = if f.target.kind == target_kind::UNIT || f.target.kind == target_kind::DOODAD { f.target.obj_id } else { 0 };
                let (target, target_pos) = target_of(&st, target_obj, target_pos);
                let fresh = !combat.casts.contains_key(&f.tl);
                let was_casting = combat.casts.get(&f.tl).is_some_and(|c| c.casting);
                let cast = combat.cast_for(f.tl, f.caster.obj_id, target_obj, now);
                cast.target_pos = target_pos;
                cast.fired = Some(now);
                cast.delay = f64::from(f.delay_ms.max(0)) / 1000.0;
                cast.casting = false;
                if cast.hit_fx.is_empty() {
                    cast.hit_fx = visual.fx.clone();
                }
                // the clip: the server's choice, else the skill's (the two handed one for a two handed weapon is a plot detail)
                let name = (f.fire_anim_id != 0).then(|| st.book.anim(f.fire_anim_id)).flatten().or_else(|| visual.fire_clip(false, false).cloned());
                let mut release = 0.0;
                if let Some(name) = &name {
                    let events = st.events(caster, name);
                    release = f64::from(events.release().unwrap_or(0.0));
                    combat.casts.get_mut(&f.tl).unwrap().sync = f64::from(events.combat_sync.unwrap_or(0.0));
                    st.play(caster, name, Act::Attack, false, 0.0, now);
                } else if let Some(Some(a)) = (was_casting).then(|| st.actors.get(caster).ok().map(|(_, _, a, ..)| a)) {
                    // casting ends with nothing to play: let locomotion resume
                    if a.is_some() {
                        st.commands.entity(caster).remove::<Acting>();
                    }
                }
                if release == 0.0 {
                    release = f64::from(visual.effect_delay_ms) / 1000.0;
                }
                face(&mut st, caster, target, target_pos, now);
                combat.release(&mut st.commands, f.tl, ev::LAUNCH);
                if !visual.plot {
                    // an instant skill has not shown its start and form effects yet
                    if fresh {
                        start_items(&mut st, &mut combat, f.tl, &visual.fx, &[ev::START, ev::FORM], caster, target, now, 0.0, true);
                    }
                    start_items(&mut st, &mut combat, f.tl, &visual.fx, &[ev::THROW], caster, target, now, 0.0, false);
                    start_items(&mut st, &mut combat, f.tl, &visual.fx, &[ev::LAUNCH], caster, target, now, release, false);
                    if let Some(projectile) = &visual.projectile {
                        let flight = (f64::from(f.delay_ms.max(0)) / 1000.0 - release).max(0.06);
                        combat.after(now + release, Later::Launch { projectile: projectile.clone(), caster, target, target_pos, duration: flight as f32 });
                    }
                }
            }
            Event::PlotEvent(p) => {
                let Some(caster) = st.entity(p.caster_obj_id) else { continue };
                if let Some(shot) = shot.as_mut()
                    && st.is_local(caster)
                {
                    shot.event("plot", now);
                }
                let target_pos = p.target_pos.map(|p| cry_to_bevy(Vec3::new(p.x, p.y, p.z)));
                let (target, target_pos) = target_of(&st, p.target_obj_id, target_pos);
                let visual = st.book.skill(p.skill_id);
                let cast = combat.cast_for(p.tl, p.caster_obj_id, p.target_obj_id, now);
                cast.target_pos = target_pos;
                if p.target_obj_id != 0 {
                    cast.target = p.target_obj_id;
                }
                if p.casting_ms > 0
                    && let Some(name) = visual.as_ref().and_then(|v| v.start_anim.clone())
                {
                    cast.casting = true;
                    st.play(caster, &name, Act::Cast, true, f64::from(p.casting_ms) / 1000.0 + 1.0, now);
                }
                let Some(step) = st.book.plot_event(p.event_id) else { continue };
                let mut release = 0.0;
                if let Some(name) = &step.anim {
                    let events = st.events(caster, name);
                    release = f64::from(events.release().unwrap_or(0.0));
                    if st.play(caster, name, Act::Attack, false, 0.0, now).is_some() {
                        combat.casts.get_mut(&p.tl).unwrap().casting = false;
                    }
                    combat.casts.get_mut(&p.tl).unwrap().sync = f64::from(events.combat_sync.unwrap_or(0.0));
                }
                face(&mut st, caster, target, target_pos, now);
                if !step.fx.is_empty() {
                    combat.casts.get_mut(&p.tl).unwrap().hit_fx = step.fx.clone();
                }
                start_items(&mut st, &mut combat, p.tl, &step.fx, &[ev::START, ev::FORM, ev::THROW], caster, target, now, 0.0, true);
                start_items(&mut st, &mut combat, p.tl, &step.fx, &[ev::LAUNCH], caster, target, now, release, false);
                if let Some(projectile) = &step.projectile {
                    combat.after(now + release, Later::Launch { projectile: projectile.clone(), caster, target, target_pos, duration: 0.25 });
                }
            }
            Event::UnitDamaged(d) => {
                // buff ticks (kind 2) have no blow to show
                if d.action.kind > 1 {
                    continue;
                }
                let Some(target) = st.entity(d.target_obj_id) else { continue };
                if let Some(shot) = shot.as_mut()
                    && !st.is_local(target)
                {
                    shot.event("hit", now);
                }
                let cast = combat.casts.get(&d.action.tl);
                let items = cast.map(|c| c.hit_fx.clone()).or_else(|| st.book.skill(d.action.skill_id).map(|v| v.fx.clone())).unwrap_or_default();
                // when the blow lands: a flying shot after its flight, a swing at the clip's sync point, a plot step as it comes
                let at = match cast {
                    Some(c) if c.delay > 0.0 => c.fired.unwrap_or(now) + c.delay,
                    Some(c) if d.action.kind == 0 && c.fired.is_some() => c.fired.unwrap_or(now) + c.sync,
                    _ => now,
                }
                .max(now);
                let missed = hit_type::is_miss(d.hit_type) || d.damage <= 0;
                let hurt = (!missed).then_some(if hit_type::is_critical(d.hit_type) { "all_re_combat_critical" } else { "all_re_combat_hit" });
                let caster = st.entity(d.caster_obj_id);
                let last = combat.last_hit.entry(d.target_obj_id).or_default();
                *last = last.max(at);
                combat.after(at, Later::Hit { caster, target, items: if missed { Arc::default() } else { items }, hurt });
            }
            Event::UnitDeath { obj_id } => {
                if let Some(shot) = shot.as_mut() {
                    shot.event("death", combat.last_hit.get(obj_id).copied().unwrap_or(now).max(now));
                }
                let at = combat.last_hit.get(obj_id).copied().unwrap_or(now).max(now);
                combat.after(at, Later::Die { obj: *obj_id });
            }
            Event::Resurrected { obj_id, .. } => {
                if let Some(e) = st.entity(*obj_id) {
                    st.commands.entity(e).remove::<(Dead, Corpse, Acting)>();
                    if let Ok((_, Some(_), ..)) = st.actors.get(e) {
                        // locomotion starts the right clip again
                        st.commands.queue(move |w: &mut World| {
                            if let Some(mut l) = w.get_mut::<Locomotion>(e) {
                                l.invalidate();
                            }
                        });
                    }
                }
                combat.last_hit.remove(obj_id);
            }
            Event::SkillEnded { tl } | Event::CastingStopped { tl, .. } | Event::PlotEnded { tl } | Event::PlotStopped { tl, .. } => {
                let (tl, stopped) = (*tl, !matches!(event, Event::SkillEnded { .. } | Event::PlotEnded { .. }));
                combat.release(&mut st.commands, tl, ev::END);
                if let Some(c) = combat.casts.get(&tl)
                    && c.casting
                    && stopped
                    && let Some(e) = st.entity(c.caster)
                {
                    // an interrupted cast: back to locomotion
                    st.commands.entity(e).remove::<Acting>();
                }
                // the damage of this timeline may still be on its way: keep the cast until the timers ran out
                if let Some(c) = combat.casts.get_mut(&tl) {
                    c.started = now - 15.0;
                }
            }
            _ => {}
        }
    }
}

/// The local player turns to the target while it acts.
fn face(st: &mut Stage, caster: Entity, target: Option<Entity>, pos: Option<Vec3>, now: f64) {
    if st.is_local(caster) && (target.is_some() || pos.is_some()) && target != Some(caster) {
        st.commands.entity(caster).insert(FaceTarget { entity: target, pos, until: now + 0.8 });
    }
}

/// Starts what is due: effects, projectiles, hits and deaths.
fn run_timers(time: Res<Time>, mut st: Stage, mut combat: ResMut<Combat>, mut models: Models) {
    let now = time.elapsed_secs_f64();
    if combat.timers.is_empty() {
        return;
    }
    let (due, later): (Vec<Timer>, Vec<Timer>) = std::mem::take(&mut combat.timers).into_iter().partition(|t| t.at <= now);
    combat.timers = later;
    for Timer { what, .. } in due {
        match what {
            Later::Fx { item, tl, on, toward } => {
                let hold = item.start < ev::LAUNCH && item.end > item.start;
                if let Some(e) = st.spawn_item(&item, on, toward, hold)
                    && item.end > item.start
                    && let Some(cast) = combat.casts.get_mut(&tl)
                {
                    cast.held.push((e, item.end));
                }
            }
            Later::Launch { projectile, caster, target, target_pos, duration } => launch(&mut st, &mut combat, &mut models, &projectile, caster, target, target_pos, duration),
            Later::Hit { caster, target, items, hurt } => {
                let toward = caster.and_then(|c| st.xforms.get(c).ok()).map(|g| g.translation());
                for item in items.iter().filter(|i| i.start == ev::HIT && !i.effect.to_ascii_lowercase().contains("_dot")) {
                    let on = if item.on_target { Some(target) } else { caster };
                    if let Some(on) = on {
                        let toward = if item.on_target { toward } else { st.xforms.get(target).ok().map(|g| g.translation()) };
                        st.spawn_item(item, on, toward, false);
                    }
                }
                if let Some(clip) = hurt {
                    st.play(target, clip, Act::Hurt, false, 0.0, now);
                }
            }
            Later::Die { obj } => {
                if let Some(e) = st.entity(obj) {
                    st.commands.entity(e).remove::<(Acting, FaceTarget)>().insert(Dead { at: now });
                    if verbose() {
                        info!("combat: unit {obj} died");
                    }
                }
                combat.last_hit.remove(&obj);
            }
        }
    }
}

/// Sends a projectile from the caster's launch bone to the target's hit point.
#[allow(clippy::too_many_arguments)]
fn launch(st: &mut Stage, combat: &mut Combat, models: &mut Models, projectile: &Projectile, caster: Entity, target: Option<Entity>, target_pos: Option<Vec3>, duration: f32) {
    // the launch bone: the projectile's own, else the first effect item's (the left hand of a bow, the right hand of a spell)
    let src_bone = if projectile.src_bone != 0 { projectile.src_bone } else { projectile.items.first().map_or(9, |i| i.bone) };
    let Some(from) = st.bone_point(caster, src_bone, 1.0) else { return };
    let dest_bone = if projectile.dest_bone != 0 { projectile.dest_bone } else { 19 };
    let to = match target {
        Some(t) => st.bone_point(t, dest_bone, 1.0),
        None => None,
    }
    .or(target_pos.map(|p| p + Vec3::Y));
    let Some(to) = to else { return };
    let rotation = Transform::from_translation(from).looking_at(to, Vec3::Y).rotation;
    let anchor = st
        .commands
        .spawn((
            Name::new(format!("projectile {}", projectile.id)),
            Transform { translation: from, rotation, scale: Vec3::ONE },
            Visibility::default(),
            Flying { from, to_unit: target.map(|t| (t, dest_bone)), to_pos: to, elapsed: 0.0, duration: duration.max(0.05) },
            Fx { follow: None, fixed_rotation: true, ttl: duration + 3.0 },
        ))
        .id();
    for item in projectile.items.iter().filter(|i| i.start <= ev::LAUNCH && !i.effect.to_ascii_lowercase().contains("_dot")) {
        let effect = Effect { scale: item.scale, prime: false, looping: true, despawn_when_finished: false, ..Effect::new(item.effect.clone()) };
        st.commands.spawn((Name::new(format!("fx {}", item.effect)), effect, Transform::IDENTITY, ChildOf(anchor)));
        // the emitter that throws the mesh (the arrow itself) is not drawn by aa-fx: draw the mesh
        if let Some(model) = combat.geometry_of(models, &item.effect) {
            let root = spawn_loaded(&mut st.commands, &item.effect, &model, &ModelOptions::default());
            // the emitter's SizeX / SizeZ 1.2 widen the arrow (more here: the glow it has in the real client is missing)
            st.commands.entity(root).insert((ChildOf(anchor), Transform::from_rotation(aa_cry::CRY_TO_BEVY).with_scale(Vec3::new(2.5, 1.0, 2.5))));
            if verbose() {
                info!("combat: projectile mesh of {}", item.effect);
            }
        }
    }
    if verbose() {
        info!("combat: projectile {} flies {:.2} s over {:.1} m", projectile.id, duration, from.distance(to));
    }
}

/// Moves the projectiles to their targets; on arrival they stop and their trail fades.
fn fly_projectiles(time: Res<Time>, mut commands: Commands, mut flying: Query<(Entity, &mut Transform, &mut Flying, &mut Fx)>, xforms: Query<&GlobalTransform>, actors: Query<&Animator>) {
    let dt = time.delta_secs();
    for (entity, mut transform, mut f, mut fx) in &mut flying {
        f.elapsed += dt;
        // homing: the target's hit bone as it moves
        if let Some((unit, bone_id)) = f.to_unit
            && let Ok(animator) = actors.get(unit)
            && let Some(point) = bone_names(bone_id).iter().find_map(|n| {
                let i = animator.rig.skeleton.bones.iter().position(|b| b.name == *n)?;
                xforms.get(*animator.bones.get(i)?).ok().map(|g| g.translation())
            })
        {
            f.to_pos = point;
        }
        let t = (f.elapsed / f.duration).min(1.0);
        let at = f.from.lerp(f.to_pos, t);
        let dir = f.to_pos - at;
        if dir.length_squared() > 1e-4 {
            transform.rotation = Transform::IDENTITY.looking_to(dir.normalize(), Vec3::Y).rotation;
        }
        transform.translation = at;
        if t >= 1.0 {
            // arrived: the emitters stay and die out with the trail
            commands.entity(entity).remove::<Flying>();
            fx.ttl = fx.ttl.min(0.6);
        }
    }
}

/// Keeps effects on their bones, ends the ones that ran out of time or lost their unit.
fn follow_fx(time: Res<Time>, mut commands: Commands, mut fx: Query<(Entity, &mut Transform, &mut Fx)>, xforms: Query<&GlobalTransform, Without<Fx>>) {
    let dt = time.delta_secs();
    for (entity, mut transform, mut fx) in &mut fx {
        fx.ttl -= dt;
        if fx.ttl <= 0.0 {
            commands.entity(entity).try_despawn();
            continue;
        }
        let Some((unit, bone, offset)) = fx.follow else { continue };
        let (Ok(unit_xf), Ok(bone_xf)) = (xforms.get(unit), xforms.get(bone)) else {
            commands.entity(entity).try_despawn();
            continue;
        };
        let (scale, rotation, _) = unit_xf.to_scale_rotation_translation();
        transform.translation = bone_xf.translation() + rotation * offset * scale.y;
        if !fx.fixed_rotation {
            transform.rotation = rotation;
        }
    }
}

/// The local player turns to its target (smoothly) for a moment while an action runs.
fn turn_to_target(
    time: Res<Time>,
    mut commands: Commands,
    mut player: Query<(Entity, &mut Mover, &FaceTarget, &GlobalTransform), With<LocalPlayer>>,
    xforms: Query<&GlobalTransform, Without<LocalPlayer>>,
) {
    let now = time.elapsed_secs_f64();
    for (entity, mut mover, face, me) in &mut player {
        if now >= face.until {
            commands.entity(entity).remove::<FaceTarget>();
            continue;
        }
        let at = face.entity.and_then(|e| xforms.get(e).ok()).map(|g| g.translation()).or(face.pos);
        let Some(at) = at else { continue };
        let d = at - me.translation();
        if Vec2::new(d.x, d.z).length() < 0.2 {
            continue;
        }
        // game yaw of a Bevy direction: x east, y north = -z
        let yaw = (-d.x).atan2(-d.z);
        mover.facing = approach_angle(mover.facing, yaw, 16.0 * time.delta_secs());
    }
}

/// Asks the server again for a skill it refused for the player's facing, now that the player faces the target, and tells it where
/// the player looks first (a standing movement packet carries the facing).
fn retry_cast(time: Res<Time>, mut combat: ResMut<Combat>, player: Query<&Mover, With<LocalPlayer>>, mut out: MessageWriter<NetCommand>) {
    let now = time.elapsed_secs_f64();
    let Some(retry) = combat.retry.as_ref().filter(|r| now >= r.at) else { return };
    let (skill, target) = (retry.skill, retry.target);
    combat.retry = None;
    let Ok(me) = player.single() else { return };
    out.write(NetCommand(Command::Move(MoveCommand::stopped(Pos::new(me.pos.x, me.pos.y, me.pos.z), me.facing))));
    out.write(NetCommand(Command::UseSkill { skill_id: skill, target_obj_id: target }));
    if verbose() {
        info!("combat: skill {skill} was out of angle: turned to face {target} and asked again");
    }
}

/// Units that were announced dead (a corpse in view when it appeared) lie down at once.
#[allow(clippy::type_complexity)]
fn mark_corpses(mut commands: Commands, units: Res<Units>, new: Query<(Entity, &RemoteUnit), (Added<RemoteUnit>, Without<Dead>)>) {
    for (entity, unit) in &new {
        if units.get(unit.obj_id).is_some_and(|u| u.dead) {
            commands.entity(entity).insert(Dead { at: f64::MIN / 4.0 });
        }
    }
}

/// Starts the death clip of units that just died (at the end of it when the death was long ago or unseen).
fn ensure_corpses(
    time: Res<Time>,
    mut commands: Commands,
    clips: Res<ClipLibrary>,
    mut dead: Query<(Entity, &Dead, &mut Animator, Option<&Locomotion>), Without<Corpse>>,
) {
    let now = time.elapsed_secs_f64();
    for (entity, d, mut animator, loco) in &mut dead {
        commands.entity(entity).insert(Corpse);
        let Some(loco) = loco else { continue };
        let Some(clip) = ["dead_01", "dead_02"].iter().find_map(|n| bind_clip(&animator.rig, &loco.clips.cal, &clips, n)) else { continue };
        let duration = clip.clip.duration;
        animator.stop_layer(0.0);
        animator.play(clip, false, 0.1);
        animator.speed = 1.0;
        animator.random_idle = None;
        let age = now - d.at;
        if age >= f64::from(duration) - 0.05
            && let Some(cur) = animator.current.as_mut()
        {
            cur.time = duration;
        }
    }
}

/// A unit whose action ended lets go of the upper body layer that played it (a one-shot ends by itself; a cast loop does not).
fn release_layers(mut ended: RemovedComponents<Acting>, mut units: Query<&mut Animator, Without<Acting>>) {
    for entity in ended.read() {
        if units.get(entity).is_ok_and(|a| a.has_layer())
            && let Ok(mut animator) = units.get_mut(entity)
        {
            animator.stop_layer(0.12);
            if verbose() {
                info!("combat: upper body layer of {entity:?} released");
            }
        }
    }
}

/// Ends the actions that ran their course; locomotion picks its clip again.
fn end_actions(time: Res<Time>, mut commands: Commands, mut acting: Query<(Entity, &Acting, Option<&mut Locomotion>)>) {
    let now = time.elapsed_secs_f64();
    for (entity, a, loco) in &mut acting {
        if now >= a.until {
            commands.entity(entity).remove::<Acting>();
            // the legs never stopped running under an upper body action
            if !a.layered
                && let Some(mut loco) = loco
            {
                loco.invalidate();
            }
        }
    }
}
