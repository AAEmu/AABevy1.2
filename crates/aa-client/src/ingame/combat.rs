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
//! * **Loops.** A clip with `anims.loop` (the cast and channel poses of the mage and healer skills) loops until the next clip of
//!   the skill, a plot step that only stops (`all_co_sk_stop`) or the end of the skill; a skill that only has a controller (a dash,
//!   a leap) plays the controller's clip; songs play the clips of the instrument the caster holds; the one handed, two handed or
//!   dual wield clip of a skill follows the weapon the caster holds ([`Wield`]); skills that change their effects with the skill
//!   level use the group of the caster's level; blows shake the camera of the player (`fx_shake_cameras`).
//! * **Reactions.** A unit that takes damage plays its hit clip; a unit that dies plays `dead_01` and stays down ([`Dead`]) until
//!   the server removes it. Units that are announced dead (a corpse in view) lie down at once.
//!
//! Everything is looked up once per skill and cached (`SkillBook`); a frame costs a few comparisons plus the effects in flight.

mod clips;
mod skills;

use std::collections::HashMap;
use std::sync::Arc;

use aa_anim::{Animator, ClipLibrary};
use aa_fx::{Effect, FxSettings, Ground};
use aa_net::buff::BuffCaster;
use aa_net::skill::{CastAction, PlotEvent, SkillFired, SkillStarted, SkillTarget, UnitDamage, hit_type, target_kind};
use aa_net::state::{Skills, Units};
use aa_net::{Command, Event, MoveCommand, NetCommand, NetEvent, Pos};
use aa_world::sky::SunLight;
use aa_world::{TerrainQuery, cry_to_bevy};
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use super::anim::{Locomotion, Motion, bind_action_clip, bind_clip, clip_file};
use super::player::{LocalPlayer, Mover, PlayerInput, approach_angle};
use super::units::RemoteUnit;
use super::camera::CameraShake;
use super::sounds::{AnimSound, Audio, Class, LayerSounds};
use super::weapons::{Drawn, Wield, hold_weapons, weapon_for_clip};
use crate::Screen;
pub(super) use skills::Sfx;
use skills::{CancelRules, FxItem, Projectile, ShakeItem, SkillBook, bone_names, ev};

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

/// A particle effect of a skill in the world. It rides its bone as a child of it; only a hit effect with a fixed orientation is
/// moved by hand (`follow`), and every effect has a time to live.
#[derive(Component)]
struct Fx {
    /// The unit and the bone entity a fixed orientation effect follows, with the offset in the unit's frame.
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
    /// The cast (timeline id) whose clip a unit (object id) is playing: only that cast may end it.
    owner: HashMap<u32, u16>,
}

struct Retry {
    at: f64,
    skill: u32,
    target: u32,
}

#[derive(Default)]
struct Cast {
    skill: u32,
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
    /// The camera shakes that start when the damage lands.
    hit_shakes: Arc<Vec<ShakeItem>>,
    /// A looping clip (the cast, a channel pose) plays for this cast and has not ended yet.
    casting: bool,
    /// A clip of this cast has played on the caster.
    played: bool,
    /// What the player's input cancels now (a cast of the local player whose skill is flagged for it).
    cancel: Option<Cancel>,
    /// The cancel went to the server (once per cast).
    cancel_sent: bool,
    /// The skill is a plot: the server names its timeline in `plot_tl_id`.
    plot: bool,
}

/// What cancels a running cast: moving (or jumping), turning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Cancel {
    on_move: bool,
    on_turn: bool,
}

/// Which part of a skill is running, for its cancel rules.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// The cast time before the skill fires (for a plot: a step that casts before the next one).
    Cast,
    /// The channel after it fired.
    Channel,
}

/// `AA_COMBAT_NO_CANCEL=1`: moving cancels nothing (to look at casts while the player runs, the upper body clips).
fn no_cancel() -> bool {
    static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OFF.get_or_init(|| std::env::var_os("AA_COMBAT_NO_CANCEL").is_some())
}

/// What cancels `phase` of a skill with these flags (`None`: nothing). Moving or jumping always ends the cast time (live game
/// behaviour: "don't move while recalling"); a channel only ends by moving when the skill says so (`channeling_cancelable`);
/// turning only ends the cast when the skill says so (`stop_casting_by_turn`).
fn cancel_of(rules: CancelRules, phase: Phase) -> Option<Cancel> {
    let (on_move, on_turn) = match phase {
        Phase::Cast => (true, rules.on_turn),
        Phase::Channel => (rules.channel_on_move, false),
    };
    (on_move || on_turn).then_some(Cancel { on_move, on_turn })
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
    Hit { caster: Option<Entity>, target: Entity, items: Arc<Vec<FxItem>>, shakes: Arc<Vec<ShakeItem>>, hurt: Option<&'static str>, melee: bool },
    /// A unit dies.
    Die { obj: u32 },
    /// The camera shakes.
    Shake(skills::Shake),
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

/// Dev: `AA_COMBAT_FAKE="T:op:tl:args;..."` plays server events for the local player `T` seconds after the first frame, for the
/// skills the test character does not have (the lane's server only sends what a cast of its own skills does):
///
/// * `T:start:TL:SKILL:CAST_MS`, `T:fire:TL:SKILL:DELAY_MS:CHANNEL_MS[:FIRE_ANIM]`, `T:plot:TL:EVENT:SKILL[:CASTING_MS]`
/// * `T:hit:TL:SKILL:DAMAGE` (a blow of the player lands on the player), `T:end:TL`, `T:stop:TL` (casting stopped),
///   `T:plotend:TL`, `T:plotstop:TL` (channeling stopped)
///
/// The target is the player itself.
#[derive(Resource)]
struct Fake {
    steps: Vec<(f64, Vec<u32>, &'static str)>,
    started: Option<f64>,
}

impl Fake {
    fn from_env() -> Option<Fake> {
        let text = std::env::var("AA_COMBAT_FAKE").ok()?;
        let mut steps: Vec<(f64, Vec<u32>, &'static str)> = Vec::new();
        for step in text.split(';').filter(|s| !s.trim().is_empty()) {
            let mut parts = step.split(':');
            let at: f64 = parts.next()?.trim().parse().ok()?;
            let op = match parts.next()?.trim() {
                "start" => "start",
                "fire" => "fire",
                "plot" => "plot",
                "hit" => "hit",
                "end" => "end",
                "stop" => "stop",
                "plotend" => "plotend",
                "plotstop" => "plotstop",
                other => {
                    warn!("combat: AA_COMBAT_FAKE: unknown step {other}");
                    continue;
                }
            };
            steps.push((at, parts.filter_map(|p| p.trim().parse().ok()).collect(), op));
        }
        steps.sort_by(|a, b| b.0.total_cmp(&a.0));
        Some(Fake { steps, started: None })
    }

    fn event(op: &str, a: &[u32], me: u32) -> Option<Event> {
        let arg = |i: usize| a.get(i).copied().unwrap_or(0);
        let caster = BuffCaster { kind: 0, obj_id: me };
        let target = SkillTarget { kind: 0, obj_id: me, pos: None };
        let tl = arg(0) as u16;
        Some(match op {
            "start" => Event::SkillStarted(SkillStarted { skill_id: arg(1), tl, caster, target, cast_ms: arg(2), base_cast_ms: arg(2), synergy: 0, result: 0, result_value: 0 }),
            "fire" => Event::SkillFired(SkillFired { skill_id: arg(1), tl, caster, target, delay_ms: arg(2) as i32, channeling_ms: arg(3) as i32, fire_anim_id: arg(4) }),
            "plot" => Event::PlotEvent(PlotEvent { tl, event_id: arg(1), skill_id: arg(2), caster_obj_id: me, target_obj_id: me, caster_pos: None, target_pos: None, casting_ms: arg(3) }),
            "hit" => Event::UnitDamaged(UnitDamage {
                action: CastAction { kind: 0, skill_id: arg(1), tl, buff_id: 0 },
                caster,
                caster_obj_id: me,
                target_obj_id: me,
                damage: arg(2) as i32,
                absorbed: 0,
                mana_burn: 0,
                hit_type: 0,
            }),
            "end" => Event::SkillEnded { tl },
            "stop" => Event::CastingStopped { tl, duration_ms: 0 },
            "plotend" => Event::PlotEnded { tl },
            "plotstop" => Event::PlotStopped { tl, duration_ms: 0, last_event: 0, channeling: true },
            _ => return None,
        })
    }
}

fn fake_events(time: Res<Time>, mut fake: ResMut<Fake>, local: Query<&LocalPlayer>, mut out: MessageWriter<NetEvent>) {
    let Ok(me) = local.single() else { return };
    let now = time.elapsed_secs_f64();
    let started = *fake.started.get_or_insert(now);
    while fake.steps.last().is_some_and(|s| now - started >= s.0) {
        let (_, args, op) = fake.steps.pop().unwrap();
        if let Some(event) = Fake::event(op, &args, me.obj_id) {
            out.write(NetEvent(event));
        }
    }
}

/// `AA_COMBAT_SHOT=<png>@<start|fire|fire2|plot|hit|death>+<seconds>[,<seconds>...]`: screenshots that many game seconds after the first
/// such event (the local player's skill started, the local player fired (`fire2`: for the second time; `plot`: a step of the player's
/// plot skill), a unit other than the player was hit, a unit died), then exit. Catches a flying arrow without guessing the network
/// latency. With several times the files are `<name>_01.png`, `<name>_02.png`, ... (a series to line up with a capture of the real
/// client).
#[derive(Resource)]
struct DevShot {
    path: String,
    on: String,
    /// Seconds after the event, in order.
    after: Vec<f64>,
    at: Option<f64>,
    /// How many have been taken, and when (real time) the last one was.
    taken: usize,
    taken_at: f64,
    /// How many times the player fired (`fire2` is the second time).
    fires: u32,
}

impl DevShot {
    fn from_env() -> Option<DevShot> {
        let spec = std::env::var("AA_COMBAT_SHOT").ok()?;
        let (path, rest) = spec.split_once('@')?;
        let (on, after) = rest.split_once('+')?;
        let mut after: Vec<f64> = after.split(',').filter_map(|t| t.trim().parse().ok()).collect();
        after.sort_by(f64::total_cmp);
        (!after.is_empty()).then(|| DevShot { path: path.to_string(), on: on.to_string(), after, at: None, taken: 0, taken_at: 0.0, fires: 0 })
    }

    /// The file of the n-th shot (0-based): the path itself for a single shot, else numbered.
    fn file(&self, n: usize) -> String {
        if self.after.len() == 1 {
            return self.path.clone();
        }
        match self.path.rsplit_once('.') {
            Some((stem, ext)) => format!("{stem}_{:02}.{ext}", n + 1),
            None => format!("{}_{:02}", self.path, n + 1),
        }
    }

    fn event(&mut self, on: &str, now: f64) {
        let on = if on == "fire" {
            self.fires += 1;
            if self.fires == 2 { "fire2" } else { on }
        } else {
            on
        };
        if self.on == on && self.at.is_none() {
            self.at = Some(now);
        }
    }
}

fn dev_shot(mut commands: Commands, time: Res<Time>, real: Res<Time<Real>>, mut shot: ResMut<DevShot>, mut exit: MessageWriter<AppExit>) {
    let now = time.elapsed_secs_f64();
    let Some(start) = shot.at else { return };
    if shot.taken < shot.after.len() {
        if now >= start + shot.after[shot.taken] {
            let file = shot.file(shot.taken);
            if let Some(dir) = std::path::Path::new(&file).parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            commands.spawn(bevy::render::view::screenshot::Screenshot::primary_window()).observe(bevy::render::view::screenshot::save_to_disk(file.clone()));
            shot.taken += 1;
            shot.taken_at = real.elapsed_secs_f64();
            info!("combat: screenshot {} -> {file}", shot.taken);
        }
    } else if real.elapsed_secs_f64() - shot.taken_at > 1.5 && std::path::Path::new(&shot.file(shot.after.len() - 1)).exists() {
        exit.write(AppExit::Success);
    }
}

pub fn plugin(app: &mut App) {
    if let Some(shot) = DevShot::from_env() {
        app.insert_resource(shot).add_systems(Update, dev_shot);
    }
    app.add_systems(Update, feed_fx.run_if(resource_exists::<TerrainQuery>));
    if std::env::var_os("AA_COMBAT_CHASE").is_some() {
        app.init_resource::<super::camera::Chase>();
    }
    if let Some(degrees) = std::env::var("AA_COMBAT_CAM_REL").ok().and_then(|v| v.parse::<f32>().ok()) {
        app.insert_resource(super::camera::CamRel(degrees));
    }
    if let Some(fake) = Fake::from_env() {
        app.insert_resource(fake).add_systems(Update, fake_events.before(handle_events));
    }
    match SkillBook::open() {
        Some(book) => {
            app.insert_resource(book);
        }
        None => warn!("combat: the game database is missing: no combat visuals"),
    }
    app.init_resource::<Combat>()
        .init_resource::<CameraShake>()
        .init_resource::<PlayerInput>()
        .add_systems(OnExit(Screen::InGame), reset)
        .add_systems(
            Update,
            (
                (handle_events, cancel_on_move, retry_cast, run_timers, mark_corpses, ensure_corpses, end_actions, release_layers, fly_projectiles, hold_weapons)
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
    skills: Res<'w, Skills>,
    wield: Query<'w, 's, &'static Wield>,
    remote: Query<'w, 's, (Entity, &'static RemoteUnit)>,
    local: Query<'w, 's, (Entity, &'static LocalPlayer)>,
    #[allow(clippy::type_complexity)]
    actors: Query<'w, 's, (&'static mut Animator, Option<&'static Locomotion>, Option<&'static Acting>, Option<&'static Motion>, Has<Dead>)>,
    xforms: Query<'w, 's, &'static GlobalTransform>,
    audio: Audio<'w, 's>,
}

impl Stage<'_, '_> {
    /// The object id of a unit's entity.
    fn obj_of(&self, e: Entity) -> Option<u32> {
        self.remote.get(e).map(|(_, r)| r.obj_id).or_else(|_| self.local.get(e).map(|(_, l)| l.obj_id)).ok()
    }

    fn entity(&self, obj: u32) -> Option<Entity> {
        if obj == 0 {
            return None;
        }
        self.remote.iter().find(|(_, r)| r.obj_id == obj).map(|(e, _)| e).or_else(|| self.local.iter().find(|(_, l)| l.obj_id == obj).map(|(e, _)| e))
    }

    fn is_local(&self, e: Entity) -> bool {
        self.local.contains(e)
    }

    /// The class of weapon a unit holds.
    fn wield(&self, unit: Entity) -> Wield {
        self.wield.get(unit).copied().unwrap_or_default()
    }

    /// The level a unit has a skill at: the player's from its skill list, the other units' from the server's, else 1.
    fn skill_level(&self, obj: u32, skill: u32) -> u8 {
        if self.local.iter().any(|(_, l)| l.obj_id == obj) {
            return self.skills.level_of(skill).unwrap_or(1);
        }
        self.net_units.get(obj).and_then(|u| u.skills.iter().find(|(id, _)| *id == skill).map(|(_, level)| *level)).unwrap_or(1)
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
        let clip = bind_action_clip(&animator.rig, &cal, &self.clips, name)?;
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
        // the sounds of a clip on the upper body are the combat code's: the animator has it as a layer
        let layer_sounds = if layered { self.layer_sounds(unit, &clip) } else { None };
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
        if let Some(events) = layer_sounds {
            self.commands.entity(unit).insert(LayerSounds::new(events, now));
        }
        if kind != Act::Hurt
            && let Some(out) = weapon_for_clip(name)
        {
            // the weapon the clip is for comes out of its sheath and stays out for a while
            self.commands.entity(unit).insert(Drawn { until: until + 4.0, out });
        }
        if verbose() {
            info!("combat: {name} on {unit:?} ({kind:?}, {duration:.2} s{})", if layered { ", upper body" } else { "" });
        }
        Some(duration)
    }

    /// Starts one item of an effect group on a unit: a particle effect (with the sound it carries), or a sound (a voice is the unit's
    /// own). `toward` is the other party (a hit effect turns to it). Returns the entity that plays it.
    fn spawn_item(&mut self, item: &FxItem, on: Entity, toward: Option<Vec3>, hold: bool) -> Option<Entity> {
        if item.effect.is_empty() {
            return self.sound_item(item, on);
        }
        let effect = self.spawn_effect(item, on, toward, hold)?;
        // the effect entity has no place in the world yet: its sounds start where its bone is
        let bone = self.bone(on, item.bone).unwrap_or(on);
        let at = self.bone_point(on, item.bone, 1.0).unwrap_or(Vec3::ZERO);
        self.effect_sounds(item, on, if hold { effect } else { bone }, at);
        Some(effect)
    }

    /// The sound events of a clip that plays on the unit's upper body.
    fn layer_sounds(&self, unit: Entity, clip: &aa_anim::BoundClip) -> Option<Arc<[AnimSound]>> {
        let (_, loco, ..) = self.actors.get(unit).ok()?;
        let database = loco?.clips.cal.sound_events.as_deref()?;
        let events = self.audio.book()?.clip_sounds(database, &clip_file(clip)?)?;
        (!events.is_empty()).then_some(events)
    }

    /// The sound of a weapon hitting a unit, at its hit bone.
    fn blow_sound(&mut self, caster: Option<Entity>, target: Entity) {
        let (Some(caster), Some(target_obj)) = (caster, self.obj_of(target)) else { return };
        let Some(attacker) = self.obj_of(caster) else { return };
        let parent = self.bone(target, 19).unwrap_or(target);
        let Some(at) = self.xforms.get(parent).ok().map(|g| g.translation()) else { return };
        self.audio.blow(&mut self.commands, attacker, (target, target_obj), parent, at);
    }

    /// A sound item of a group: an event at its bone, or the voice of the unit at its head.
    fn sound_item(&mut self, item: &FxItem, on: Entity) -> Option<Entity> {
        let sfx = item.sound.as_ref()?;
        let (class, bone) = match sfx {
            Sfx::Event(_) => (Class::Skill, self.bone(on, item.bone)),
            Sfx::Voice(_) => (Class::Voice, self.bone(on, 2)),
        };
        let parent = bone.unwrap_or(on);
        let at = self.xforms.get(parent).ok()?.translation();
        let obj = self.obj_of(on);
        self.audio.sfx(&mut self.commands, sfx, class, on, obj, parent, at)
    }

    /// The sounds a particle effect carries: the one of its database row and the `Sound=""` of its emitters (once each). They start
    /// where the effect starts, under `parent`.
    fn effect_sounds(&mut self, item: &FxItem, unit: Entity, parent: Entity, at: Vec3) {
        if !self.audio.active() {
            return;
        }
        let own = match &item.sound {
            Some(Sfx::Event(p)) => Some(p.clone()),
            _ => None,
        };
        if let Some(sfx) = item.sound.as_ref() {
            let obj = self.obj_of(unit);
            self.audio.sfx(&mut self.commands, sfx, Class::Skill, unit, obj, parent, at);
        }
        let Some(book) = self.audio.book() else { return };
        for path in book.particle_sounds(&item.effect).iter() {
            if own.as_ref().is_some_and(|o| o.eq_ignore_ascii_case(path)) {
                continue;
            }
            self.audio.start(&mut self.commands, &super::sounds::Play { path, class: Class::Skill, unit, parent, at });
        }
    }

    /// Starts the particle effect of an item on a unit.
    fn spawn_effect(&mut self, item: &FxItem, on: Entity, toward: Option<Vec3>, hold: bool) -> Option<Entity> {
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
        let effect = Effect { scale: item.scale * std::env::var("AA_FX_SCALE").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(1.0), prime: false, looping: hold, despawn_when_finished: !hold, ..Effect::new(item.effect.clone()) };
        let ttl = if hold { 12.0 } else { 6.0 };
        if fixed_rotation {
            // a hit effect turned towards the attacker keeps that orientation: it only follows the bone's position (`follow_fx`)
            let follow = bone.map(|b| (on, b, offset));
            return Some(self.commands.spawn((Name::new(format!("fx {}", item.effect)), effect, transform, Fx { follow, fixed_rotation, ttl })).id());
        }
        // the effect is a child of its bone (of the unit, for an effect on the origin), so that its tails and `InheritVelocity` see
        // the motion of the bone; the local pose gives it the world pose it would have had: the unit's orientation and scale
        let parent = bone.unwrap_or(on);
        let parent_xf = *self.xforms.get(parent).ok()?;
        let (parent_scale, parent_rotation, _) = parent_xf.to_scale_rotation_translation();
        let local = Transform {
            translation: parent_xf.affine().inverse().transform_point3(transform.translation),
            rotation: parent_rotation.inverse() * transform.rotation,
            scale: Vec3::splat(transform.scale.y / ((parent_scale.x + parent_scale.y + parent_scale.z) / 3.0).max(1e-4)),
        };
        Some(self.commands.spawn((Name::new(format!("fx {}", item.effect)), effect, local, ChildOf(parent), Fx { follow: None, fixed_rotation: false, ttl })).id())
    }
}

impl Combat {
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

    fn cast_for(&mut self, tl: u16, skill: u32, caster: u32, target: u32, now: f64) -> &mut Cast {
        if self.casts.len() > 256 {
            self.casts.retain(|_, c| now - c.started < 20.0);
        }
        self.casts.entry(tl).or_insert_with(|| Cast { skill, caster, target, started: now, ..Cast::default() })
    }
}

/// Starts the items of `moments` of a cast now, or `delay` seconds from now.
#[allow(clippy::too_many_arguments)]
fn start_items(st: &mut Stage, combat: &mut Combat, tl: u16, items: &[FxItem], moments: &[u8], caster: Entity, target: Option<Entity>, now: f64, delay: f64, skip_short: bool) {
    let toward_caster = st.xforms.get(caster).ok().map(|g| g.translation());
    let toward_target = target.and_then(|t| st.xforms.get(t).ok()).map(|g| g.translation());
    for item in items.iter().filter(|i| moments.contains(&i.start)) {
        // buff visuals (damage over time, auras) belong to the buff, not to the skill
        if item.is_dot() {
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

/// Starts the camera shakes of `moments` now, or `delay` seconds from now: on the camera of the player when it is the caster (or
/// the target, for a shake that says so).
#[allow(clippy::too_many_arguments)]
fn start_shakes(st: &Stage, combat: &mut Combat, shakes: &[ShakeItem], moments: &[u8], caster: Entity, target: Option<Entity>, now: f64, delay: f64) {
    for item in shakes.iter().filter(|i| moments.contains(&i.start)) {
        let who = if item.on_target { target } else { Some(caster) };
        if who.is_some_and(|e| st.is_local(e)) {
            combat.after(now + delay, Later::Shake(item.shake));
        }
    }
}

/// Plays `name` as an action of the cast `tl`. A clip with `anims.loop` loops until the next clip of the cast, a stop step or the
/// end of the skill; `hold` is only the safety net for an end that never comes.
#[allow(clippy::too_many_arguments)]
fn play_clip(st: &mut Stage, combat: &mut Combat, tl: u16, caster: Entity, name: &str, kind: Act, hold: f64, now: f64) -> Option<f32> {
    let looping = kind == Act::Cast || st.book.clip_loops(name);
    let played = st.play(caster, name, kind, looping, hold, now);
    if played.is_some()
        && let Some(cast) = combat.casts.get_mut(&tl)
    {
        cast.casting = looping;
        cast.played = true;
        combat.owner.insert(cast.caster, tl);
    }
    played
}

/// Ends the looping clip of the cast `tl` (unless the unit has gone on to a clip of another cast): the unit goes back to
/// locomotion.
fn end_loop(st: &mut Stage, combat: &mut Combat, tl: u16) {
    let Some(cast) = combat.casts.get_mut(&tl) else { return };
    if !std::mem::take(&mut cast.casting) || combat.owner.get(&cast.caster) != Some(&tl) {
        return;
    }
    if let Some(e) = st.entity(cast.caster) {
        st.commands.entity(e).remove::<Acting>();
        if verbose() {
            info!("combat: loop of timeline {tl} ended on {e:?}");
        }
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
        if verbose() && matches!(
                event,
                Event::SkillStarted(_)
                    | Event::SkillFired(_)
                    | Event::PlotEvent(_)
                    | Event::UnitDamaged(_)
                    | Event::UnitDeath { .. }
                    | Event::SkillEnded { .. }
                    | Event::CastingStopped { .. }
                    | Event::SkillStopped { .. }
                    | Event::PlotEnded { .. }
                    | Event::PlotStopped { .. }
            ) {
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
                let look = visual.look(st.skill_level(s.caster.obj_id, s.skill_id));
                let wield = st.wield(caster);
                {
                    let cast = combat.cast_for(s.tl, s.skill_id, s.caster.obj_id, target_obj, now);
                    cast.target_pos = target_pos;
                    cast.hit_fx = look.fx.clone();
                    cast.hit_shakes = look.shakes.clone();
                    cast.plot = visual.plot;
                    cast.cancel = if s.cast_ms > 0 { cancel_of(visual.cancel, Phase::Cast) } else { None };
                }
                if s.cast_ms > 0 {
                    // the cast: its clip loops until the skill fires; the charge-up effects play meanwhile
                    let hold = f64::from(s.cast_ms) / 1000.0 + 1.0;
                    if let Some(name) = visual.instrument_start_clip(wield.instrument).or(visual.start_anim.as_ref()) {
                        play_clip(&mut st, &mut combat, s.tl, caster, name, Act::Cast, hold, now);
                    }
                    face(&mut st, caster, target, target_pos, now);
                    if !visual.plot {
                        start_items(&mut st, &mut combat, s.tl, &look.fx, &[ev::START, ev::FORM], caster, target, now, 0.0, false);
                        start_shakes(&st, &mut combat, &look.shakes, &[ev::START, ev::FORM], caster, target, now, 0.0);
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
                let look = visual.look(st.skill_level(f.caster.obj_id, f.skill_id));
                let wield = st.wield(caster);
                let fresh = !combat.casts.contains_key(&f.tl);
                let idle = !combat.casts.get(&f.tl).is_some_and(|c| c.played);
                {
                    let cast = combat.cast_for(f.tl, f.skill_id, f.caster.obj_id, target_obj, now);
                    cast.target_pos = target_pos;
                    cast.fired = Some(now);
                    cast.plot = visual.plot;
                    // the channel after the fire may be cancelled; a plain cast is over
                    cast.cancel = if f.channeling_ms > 0 { cancel_of(visual.cancel, Phase::Channel) } else { None };
                    cast.delay = f64::from(f.delay_ms.max(0)) / 1000.0;
                    if cast.hit_fx.is_empty() {
                        cast.hit_fx = look.fx.clone();
                        cast.hit_shakes = look.shakes.clone();
                    }
                }
                // the clip: a song's for the instrument held; else the server's choice, which is the skill's one handed clip: the
                // two handed or dual wield one for the weapon held replaces it (the attacks the server picks per weapon stay)
                let server = (f.fire_anim_id != 0).then(|| st.book.anim(f.fire_anim_id)).flatten();
                let own = visual.fire_clip(wield.two_handed, wield.dual).cloned();
                let name = visual
                    .instrument_fire_clip(wield.instrument)
                    .cloned()
                    .or_else(|| match server {
                        Some(n) if visual.fire_anim.as_deref() != Some(&*n) => Some(n),
                        _ => own,
                    })
                    // a dash or a leap has nothing but its controller's clip
                    .or_else(|| if idle { visual.controller_anim.clone() } else { None });
                let mut release = 0.0;
                let mut voice = 0.0;
                let mut played = false;
                if let Some(name) = &name {
                    let events = st.events(caster, name);
                    release = f64::from(events.release().unwrap_or(0.0));
                    voice = f64::from(events.voice_time().unwrap_or(0.0));
                    combat.casts.get_mut(&f.tl).unwrap().sync = f64::from(events.combat_sync.unwrap_or(0.0));
                    // a channel pose loops for the channeling time
                    let hold = if f.channeling_ms > 0 { f64::from(f.channeling_ms) / 1000.0 + 2.0 } else { 20.0 };
                    played = play_clip(&mut st, &mut combat, f.tl, caster, name, Act::Attack, hold, now).is_some();
                }
                if !played {
                    // casting ends with nothing to play: let locomotion resume
                    end_loop(&mut st, &mut combat, f.tl);
                }
                if release == 0.0 {
                    release = f64::from(visual.effect_delay_ms) / 1000.0;
                }
                face(&mut st, caster, target, target_pos, now);
                combat.release(&mut st.commands, f.tl, ev::LAUNCH);
                if !visual.plot {
                    // an instant skill has not shown its start and form effects yet
                    if fresh {
                        start_items(&mut st, &mut combat, f.tl, &look.fx, &[ev::START, ev::FORM], caster, target, now, 0.0, true);
                        start_shakes(&st, &mut combat, &look.shakes, &[ev::START, ev::FORM], caster, target, now, 0.0);
                    }
                    start_items(&mut st, &mut combat, f.tl, &look.fx, &[ev::THROW], caster, target, now, 0.0, false);
                    start_items(&mut st, &mut combat, f.tl, &look.fx, &[ev::LAUNCH], caster, target, now, release, false);
                    // the voice line at the moment the clip calls it
                    start_items(&mut st, &mut combat, f.tl, &look.fx, &[ev::VOICE], caster, target, now, if voice > 0.0 { voice } else { release }, false);
                    start_shakes(&st, &mut combat, &look.shakes, &[ev::THROW], caster, target, now, 0.0);
                    start_shakes(&st, &mut combat, &look.shakes, &[ev::LAUNCH], caster, target, now, release);
                    if let Some(projectile) = &look.projectile {
                        start_shakes(&st, &mut combat, &projectile.shakes, &[ev::START, ev::FORM, ev::THROW, ev::LAUNCH], caster, target, now, release);
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
                let wield = st.wield(caster);
                {
                    let cast = combat.cast_for(p.tl, p.skill_id, p.caster_obj_id, p.target_obj_id, now);
                    cast.target_pos = target_pos;
                    if p.target_obj_id != 0 {
                        cast.target = p.target_obj_id;
                    }
                    // a step with a cast time before the next one is casting; the others channel or just run
                    cast.plot = true;
                    let rules = visual.as_ref().map(|v| v.cancel).unwrap_or_default();
                    cast.cancel = cancel_of(rules, if p.casting_ms > 0 { Phase::Cast } else { Phase::Channel });
                }
                if p.casting_ms > 0
                    && let Some(name) = visual.as_ref().and_then(|v| v.instrument_start_clip(wield.instrument).or(v.start_anim.as_ref()).cloned())
                {
                    play_clip(&mut st, &mut combat, p.tl, caster, &name, Act::Cast, f64::from(p.casting_ms) / 1000.0 + 1.0, now);
                }
                let step = st.book.plot_event(p.event_id);
                // `all_co_sk_stop`: ends the clip that loops before this step (alone, it is the whole step)
                if step.as_ref().is_some_and(|s| s.stops()) {
                    end_loop(&mut st, &mut combat, p.tl);
                }
                // the clip of the step; a skill that moves its caster (a dash) has only its controller's clip until a step has one
                let idle = !combat.casts.get(&p.tl).is_some_and(|c| c.played);
                let clip = step.as_ref().and_then(|s| s.clip().cloned()).or_else(|| if idle { visual.as_ref().and_then(|v| v.controller_anim.clone()) } else { None });
                let mut release = 0.0;
                let mut voice = 0.0;
                if let Some(name) = &clip {
                    let events = st.events(caster, name);
                    release = f64::from(events.release().unwrap_or(0.0));
                    voice = f64::from(events.voice_time().unwrap_or(0.0));
                    let hold = if p.casting_ms > 0 { f64::from(p.casting_ms) / 1000.0 + 1.0 } else { 20.0 };
                    play_clip(&mut st, &mut combat, p.tl, caster, name, Act::Attack, hold, now);
                    combat.casts.get_mut(&p.tl).unwrap().sync = f64::from(events.combat_sync.unwrap_or(0.0));
                }
                let Some(step) = step else { continue };
                face(&mut st, caster, target, target_pos, now);
                if !step.fx.is_empty() {
                    let cast = combat.casts.get_mut(&p.tl).unwrap();
                    cast.hit_fx = step.fx.clone();
                    cast.hit_shakes = step.shakes.clone();
                }
                start_items(&mut st, &mut combat, p.tl, &step.fx, &[ev::START, ev::FORM, ev::THROW], caster, target, now, 0.0, true);
                start_items(&mut st, &mut combat, p.tl, &step.fx, &[ev::LAUNCH], caster, target, now, release, false);
                start_items(&mut st, &mut combat, p.tl, &step.fx, &[ev::VOICE], caster, target, now, if voice > 0.0 { voice } else { release }, false);
                start_shakes(&st, &mut combat, &step.shakes, &[ev::START, ev::FORM, ev::THROW], caster, target, now, 0.0);
                start_shakes(&st, &mut combat, &step.shakes, &[ev::LAUNCH], caster, target, now, release);
                if let Some(projectile) = &step.projectile {
                    start_shakes(&st, &mut combat, &projectile.shakes, &[ev::START, ev::FORM, ev::THROW, ev::LAUNCH], caster, target, now, release);
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
                let (items, shakes) = match cast {
                    Some(c) => (c.hit_fx.clone(), c.hit_shakes.clone()),
                    None => st
                        .book
                        .skill(d.action.skill_id)
                        .map(|v| v.look(st.skill_level(d.caster_obj_id, d.action.skill_id)))
                        .map(|look| (look.fx, look.shakes))
                        .unwrap_or_default(),
                };
                // when the blow lands: a flying shot after its flight, a swing at the clip's sync point, a plot step as it comes
                let at = match cast {
                    Some(c) if c.delay > 0.0 => c.fired.unwrap_or(now) + c.delay,
                    Some(c) if d.action.kind == 0 && c.fired.is_some() => c.fired.unwrap_or(now) + c.sync,
                    _ => now,
                }
                .max(now);
                // a blow (not a shot that flew; buff ticks were left out above) has the sound of the attacker's weapon on the target
                let melee = !cast.is_some_and(|c| c.delay > 0.0);
                let missed = hit_type::is_miss(d.hit_type) || d.damage <= 0;
                let hurt = (!missed).then_some(if hit_type::is_critical(d.hit_type) { "all_re_combat_critical" } else { "all_re_combat_hit" });
                let caster = st.entity(d.caster_obj_id);
                let last = combat.last_hit.entry(d.target_obj_id).or_default();
                *last = last.max(at);
                combat.after(at, Later::Hit { caster, target, items: if missed { Arc::default() } else { items }, shakes: if missed { Arc::default() } else { shakes }, hurt, melee });
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
                let tl = *tl;
                combat.release(&mut st.commands, tl, ev::END);
                // a looping clip ends with the skill, or with the cast that was interrupted
                end_loop(&mut st, &mut combat, tl);
                if let Some(c) = combat.casts.get_mut(&tl) {
                    c.cancel = None;
                }
                // the damage of this timeline may still be on its way: keep the cast until the timers ran out
                if let Some(c) = combat.casts.get_mut(&tl) {
                    c.started = now - 15.0;
                }
            }
            Event::SkillStopped { obj_id, skill_id } => {
                let tls: Vec<u16> = combat.casts.iter().filter(|(_, c)| c.caster == *obj_id && c.skill == *skill_id).map(|(tl, _)| *tl).collect();
                for tl in tls {
                    combat.release(&mut st.commands, tl, ev::END);
                    end_loop(&mut st, &mut combat, tl);
                    if let Some(c) = combat.casts.get_mut(&tl) {
                        c.cancel = None;
                    }
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
fn run_timers(time: Res<Time>, mut st: Stage, mut combat: ResMut<Combat>, mut camera: ResMut<CameraShake>) {
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
            Later::Launch { projectile, caster, target, target_pos, duration } => launch(&mut st, &projectile, caster, target, target_pos, duration),
            Later::Hit { caster, target, items, shakes, hurt, melee } => {
                let toward = caster.and_then(|c| st.xforms.get(c).ok()).map(|g| g.translation());
                if melee && hurt.is_some() {
                    st.blow_sound(caster, target);
                }
                for item in items.iter().filter(|i| i.start == ev::HIT && !i.is_dot()) {
                    let on = if item.on_target { Some(target) } else { caster };
                    if let Some(on) = on {
                        let toward = if item.on_target { toward } else { st.xforms.get(target).ok().map(|g| g.translation()) };
                        st.spawn_item(item, on, toward, false);
                    }
                }
                if let Some(clip) = hurt {
                    st.play(target, clip, Act::Hurt, false, 0.0, now);
                }
                // the camera of the player kicks when its blow lands, or when it is the one that was hit by a heavy one
                for item in shakes.iter().filter(|i| i.start == ev::HIT) {
                    let who = if item.on_target { Some(target) } else { caster };
                    if who.is_some_and(|e| st.is_local(e)) {
                        add_shake(&mut camera, &item.shake);
                    }
                }
            }
            Later::Shake(shake) => add_shake(&mut camera, &shake),
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

fn add_shake(camera: &mut CameraShake, shake: &skills::Shake) {
    if verbose() {
        info!("combat: camera shake ang {:?} shift {:?} every {} s for {} s", shake.ang, shake.shift, shake.frequency, shake.duration);
    }
    camera.add(shake.ang, shake.shift, shake.frequency, shake.randomness, shake.duration);
}

/// Tells aa-fx where the sun is (the shading of mesh particles) and how high the ground is (particles that bounce, effects that start
/// where a particle lands). The sun is the world's directional light; the ground is the terrain under a world position.
fn feed_fx(terrain: Res<TerrainQuery>, sun: Query<&GlobalTransform, With<SunLight>>, fx: Option<ResMut<FxSettings>>, mut wired: Local<bool>) {
    let Some(mut fx) = fx else { return };
    if !*wired {
        *wired = true;
        let terrain = terrain.clone();
        // Bevy position -> game position (x east, y north = -z); the height is Bevy's y
        fx.ground = Some(Ground(Arc::new(move |p: Vec3| terrain.height_at(p.x, -p.z))));
    }
    if let Ok(light) = sun.single() {
        let to_sun = (-light.forward().as_vec3()).normalize_or_zero();
        if to_sun != Vec3::ZERO && fx.sun.distance_squared(to_sun) > 1e-6 {
            fx.sun = to_sun;
        }
    }
}

/// Sends a projectile from the caster's launch bone to the target's hit point.
#[allow(clippy::too_many_arguments)]
fn launch(st: &mut Stage, projectile: &Projectile, caster: Entity, target: Option<Entity>, target_pos: Option<Vec3>, duration: f32) {
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
    for item in projectile.items.iter().filter(|i| i.start <= ev::LAUNCH && !i.is_dot()) {
        // the sounds of the projectile fly with it
        if item.effect.is_empty() {
            if let Some(sfx) = &item.sound {
                let class = if matches!(sfx, Sfx::Voice(_)) { Class::Voice } else { Class::Skill };
                let obj = st.obj_of(caster);
                st.audio.sfx(&mut st.commands, sfx, class, caster, obj, anchor, from);
            }
            continue;
        }
        st.effect_sounds(item, caster, anchor, from);
        let effect = Effect { scale: item.scale, prime: false, looping: true, despawn_when_finished: false, ..Effect::new(item.effect.clone()) };
        // the effect flies with the anchor: its forward (-Z) is the flight direction, the mesh emitters (the arrow) draw their model
        st.commands.spawn((Name::new(format!("fx {}", item.effect)), effect, Transform::IDENTITY, ChildOf(anchor)));
    }
    if verbose() {
        info!("combat: projectile {} flies {:.2} s over {:.1} m", projectile.id, duration, from.distance(to));
    }
}

/// Moves the projectiles to their targets; on arrival they stop and their trail fades.
fn fly_projectiles(
    time: Res<Time>,
    mut commands: Commands,
    mut flying: Query<(Entity, &mut Transform, &mut Flying, &mut Fx, Option<&Children>)>,
    xforms: Query<&GlobalTransform>,
    actors: Query<&Animator>,
    mut effects: Query<&mut Effect>,
    mut chase: Option<ResMut<super::camera::Chase>>,
) {
    let dt = time.delta_secs();
    // dev: the camera follows the projectile (see `Chase`); it goes back to the player when the last one landed
    if let Some(chase) = chase.as_mut() {
        chase.0 = flying.iter().next().map(|(_, t, ..)| {
            let (forward, up) = (t.rotation * Vec3::NEG_Z, Vec3::Y);
            let eye = t.translation - forward * 2.6 + t.rotation * Vec3::X * 0.9 + up * 0.5;
            Transform::from_translation(eye).looking_at(t.translation + forward * 3.0, up)
        });
    }
    for (entity, mut transform, mut f, mut fx, children) in &mut flying {
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
            // arrived: the emitters stay and die out with the trail, but nothing starts again at the target
            commands.entity(entity).remove::<Flying>();
            fx.ttl = fx.ttl.min(0.6);
            for child in children.into_iter().flatten() {
                if let Ok(mut effect) = effects.get_mut(*child) {
                    effect.looping = false;
                }
            }
        }
    }
}

/// Ends the effects that ran out of time, and moves the ones that follow a bone with a fixed orientation (the others are children
/// of their bone) or lost their unit.
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

/// The player moves or jumps during a cast time (or a channel that moving ends), or turns during a cast that turning ends: the cast is cancelled (`CSStopCasting`, the server
/// answers `CastingStopped` / `PlotStopped`) and the clip and the held effects end at once, without waiting for the answer.
fn cancel_on_move(input: Res<PlayerInput>, mut st: Stage, mut combat: ResMut<Combat>, mut out: MessageWriter<NetCommand>) {
    if !(input.moving || input.jumped || input.turned) || no_cancel() {
        return;
    }
    let Some(me) = st.local.iter().next().map(|(_, l)| l.obj_id) else { return };
    let tls: Vec<(u16, bool)> = combat
        .casts
        .iter()
        .filter(|(_, c)| c.caster == me && !c.cancel_sent)
        .filter(|(_, c)| c.cancel.is_some_and(|r| (r.on_move && (input.moving || input.jumped)) || (r.on_turn && input.turned)))
        .map(|(tl, c)| (*tl, c.plot))
        .collect();
    for (tl, plot) in tls {
        out.write(NetCommand(Command::StopCasting { tl: if plot { 0 } else { tl }, plot_tl: if plot { tl } else { 0 } }));
        combat.release(&mut st.commands, tl, ev::END);
        let caster = combat.casts.get_mut(&tl).map(|c| {
            c.cancel_sent = true;
            c.cancel = None;
            c.casting = false;
            c.caster
        });
        // the unit goes back to locomotion, unless a newer cast has its clip
        if let Some(e) = caster.filter(|obj| combat.owner.get(obj) == Some(&tl)).and_then(|obj| st.entity(obj)) {
            st.commands.entity(e).remove::<Acting>();
        }
        if verbose() {
            info!("combat: the player moved: cast {tl} cancelled ({}, moving {}, jump {}, turn {})", if plot { "plot" } else { "skill" }, input.moving, input.jumped, input.turned);
        }
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

/// A unit whose action ended lets go of the upper body layer that played it (a one-shot ends by itself; a cast loop does not), or,
/// when the action had the whole body, starts its locomotion clip again (it may have ended by a stop, not by running out).
fn release_layers(mut ended: RemovedComponents<Acting>, mut units: Query<(&mut Animator, Option<&mut Locomotion>), Without<Acting>>) {
    for entity in ended.read() {
        let Ok((mut animator, loco)) = units.get_mut(entity) else { continue };
        if animator.has_layer() {
            animator.stop_layer(0.12);
            if verbose() {
                info!("combat: upper body layer of {entity:?} released");
            }
        } else if let Some(mut loco) = loco {
            loco.invalidate();
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Moving always ends a cast time; a channel only when flagged; turning only when flagged, and only before the fire.
    #[test]
    fn what_cancels_which_phase() {
        let none = CancelRules::default();
        assert_eq!(cancel_of(none, Phase::Cast), Some(Cancel { on_move: true, on_turn: false }));
        assert_eq!(cancel_of(none, Phase::Channel), None);
        // Telekinesis: moving ends the cast and the channel
        let channel = CancelRules { channel_on_move: true, ..none };
        assert_eq!(cancel_of(channel, Phase::Cast), Some(Cancel { on_move: true, on_turn: false }));
        assert_eq!(cancel_of(channel, Phase::Channel), Some(Cancel { on_move: true, on_turn: false }));
        let turn = CancelRules { on_turn: true, ..none };
        assert_eq!(cancel_of(turn, Phase::Cast), Some(Cancel { on_move: true, on_turn: true }));
        assert_eq!(cancel_of(turn, Phase::Channel), None);
    }
}
