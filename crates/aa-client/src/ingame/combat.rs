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
//! * **Tools.** A skill that uses a tool (`skills.mainhand_tool_id` / `offhand_tool_id`: the uproot shovel, the watering pot)
//!   holds its model in that hand while the skill's start or fire clip plays ([`SkillTool`], [`super::tools`]).
//! * **Reactions.** A unit that takes damage plays its hit clip; a unit that dies plays `dead_01` and stays down ([`Dead`]) until
//!   the server removes it. Units that are announced dead (a corpse in view) lie down at once.
//!
//! Everything is looked up once per skill and cached (`SkillBook`); a frame costs a few comparisons plus the effects in flight.

mod buffs;
mod clips;
mod preload;
pub(crate) mod skills;
mod sweep;

use std::collections::HashMap;
use std::sync::Arc;

use aa_anim::{Animator, ClipLibrary};
use aa_fx::{ChrEffect, Effect, FxSettings, Ground};
use aa_net::buff::BuffCaster;
use aa_net::skill::{CastAction, PlotEvent, SkillFired, SkillStarted, SkillTarget, UnitDamage, hit_type, target_kind};
use aa_net::state::{Skills, Units};
use aa_net::{Command, Event, MoveCommand, NetCommand, NetEvent, Pos};
use aa_world::sky::SunLight;
use aa_world::{TerrainQuery, WaterQuery, cry_to_bevy};
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use super::anim::{Locomotion, Motion, bind_action_clip, bind_clip, clip_file};
use super::player::{LocalPlayer, Mover, PlayerInput, approach_angle};
use super::units::RemoteUnit;
use super::camera::CameraShake;
use super::sounds::{AnimSound, Audio, Class, LayerSounds};
use super::tools::SkillTool;
use super::weapons::{Drawn, Out, StaffSide, Wield, holster_clip, hold_weapons, weapon_for_clip};
use crate::Screen;
pub(super) use skills::Sfx;
use skills::{CancelRules, FxItem, Projectile, ShakeItem, SkillBook, bone_names, ev, src_bone_names};

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

/// A unit killed by a blow flies away from the attacker, lands (in water: on the surface), floats for a moment and sinks to the
/// ground (the real client's `combat_death_2` capture: knocked up and back by about half a metre in a third of a second, a
/// moment on the surface, gone under it a second later and faintly seen on the floor).
#[derive(Component)]
struct Thrown {
    /// Horizontal direction (Bevy world) away from the attacker.
    dir: Vec3,
    /// When it was hit (seconds on the app clock).
    at: f64,
}

/// Seconds of the flight, metres it covers and how high it goes.
const THROW_TIME: f64 = 0.35;
const THROW_DISTANCE: f32 = 0.7;
const THROW_HEIGHT: f32 = 0.35;
/// Seconds a corpse stays on the water surface, and the time it needs to sink to the ground.
const FLOAT_TIME: f64 = 0.2;
const SINK_TIME: f64 = 0.45;

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
    /// The items of the projectile's group that play where it arrives (its hit and end moments: `fireball_hit`).
    arrival: Arc<Vec<FxItem>>,
}

/// A damage packet at the moment its blow lands (a swing at the clip's sync point, a shot after its flight, a plot step as it
/// comes; after the draw of the weapon): what the HUD's floating numbers and combat log show, as in the real client. `received` is the
/// time the combat code took the packet up (the draw may have held it), not the time the blow lands.
#[derive(Message, Clone, Debug)]
pub(crate) struct DamageLanded {
    pub damage: UnitDamage,
    pub received: f64,
}

/// The skill uses in progress (by timeline id) and what is waiting for its time.
#[derive(Resource, Default)]
struct Combat {
    /// Landed blows waiting for [`flush_landed`].
    landed: Vec<DamageLanded>,
    casts: HashMap<u16, Cast>,
    timers: Vec<Timer>,
    /// When the last scheduled hit on a unit lands (a death waits for it).
    last_hit: HashMap<u32, f64>,
    /// A skill the server refused because the player did not face the target: when to ask again (once the player turned).
    retry: Option<Retry>,
    /// The skill the player just asked for: it replaces a cast in progress.
    preempt: Option<Preempt>,
    /// The cast (timeline id) whose clip a unit (object id) is playing: only that cast may end it.
    owner: HashMap<u32, u16>,
    /// Who hit a unit (object id) last: the blow that kills throws it away from there.
    last_attacker: HashMap<u32, Entity>,
    /// The player has drawn its weapon for a skill and its swing waits for the draw (see [`DRAW_LEAD`]).
    lead: Option<Lead>,
    /// Server events of the skill that wait for the draw, with the moment they are let through; in the order they came.
    held: Vec<(f64, Event)>,
    /// The timelines of the held events (their later events wait too).
    held_tls: Vec<u16>,
}

/// Seconds from the key to the first swing of a character that has its weapon sheathed: the weapon comes out and the shield onto the
/// arm first (the real client's `combat_melee` and `combat_triple_slash` captures: the damage lands 0.85 s after the key, 0.25 s and
/// 0.19 s into the clips: they start at about 0.6 s). The server answers after 0.12 s; its events wait for the rest.
const DRAW_LEAD: f64 = 0.6;

/// The player asked for a weapon skill with the weapon away.
#[derive(Clone, Copy)]
struct Lead {
    /// When the draw is over (seconds on the app clock).
    ready: f64,
    skill: u32,
    caster: u32,
}

struct Retry {
    at: f64,
    skill: u32,
    target: u32,
}

/// The player asked for a skill (see [`preempt_cast`]).
struct Preempt {
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
    /// The blow of a damage packet lands: the HUD may show its number.
    Landed(DamageLanded),
    /// A skill effect of a cast (held ones are tracked in the cast).
    /// `detach`: seconds after it starts that the effect stops following its bone (a blade trail ends at the swing's throw moment).
    Fx { item: FxItem, tl: u16, on: Entity, toward: Option<Vec3>, once: bool, detach: Option<f64> },
    /// An effect on the target of a skill that has no unit but a point (a meteor, rain, a mire): it starts on the ground there.
    GroundFx { item: FxItem, tl: u16, at: Vec3, toward: Option<Vec3> },
    /// A blade trail ends: it stays where it is and fades out.
    Detach(Entity),
    /// An effect the clip starts itself (`effect` anim event).
    ClipFx { unit: Entity, effect: clips::ClipEffect },
    /// A projectile leaves.
    Launch { projectile: Projectile, tl: u16, caster: Entity, target: Option<Entity>, target_pos: Option<Vec3>, duration: f32, speed: f32, hold: bool },
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

/// Dash's effects (`system_skill.speed_full_pc`, `speed_full_pc_end`): the real client's start of a sprint shows no distortion of the screen,
/// so the refraction spheres (`speed_full_pc1`, `speed_full_pc_end4`) are left out ([`aa_fx::Effect::no_refraction`]).
fn no_refraction(effect: &str) -> bool {
    effect.to_ascii_lowercase().contains("system_skill.speed_full_pc")
}

/// The emitters of Dash's effect that give their first cycle only. `speed_full_pc` holds the big streaked shell twice: `speed_full_pc3`
/// (a burst of 0.4 s at the start) and `speed_full_pc_meshe_high` / `_medium` (continuous, no `EmitterLifeTime`: two shells of size 4 alive
/// at all times for as long as the buff holds the effect, a "bubble" in front of the runner). 1.2 shows the shell at the start
/// of Dash only and gone within a moment, so the continuous ones emit their first cycle (0.4 s) and stop; the thin streaks
/// (`speed_full_pc2`) stay for the run ([`aa_fx::Effect::once`]).
const DASH_ONCE: &[&str] = &["speed_full_pc_meshe"];

fn once_emitters(effect: &str) -> &'static [&'static str] {
    if no_refraction(effect) { DASH_ONCE } else { &[] }
}

/// `AA_COMBAT_NOFX=a,b`: effects whose name contains one of the parts are not played (to find which effect makes a look).
fn muted(effect: &str) -> bool {
    static M: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    M.get_or_init(|| std::env::var("AA_COMBAT_NOFX").map(|v| v.split(',').filter(|p| !p.is_empty()).map(str::to_owned).collect()).unwrap_or_default())
        .iter()
        .any(|p| effect.contains(p.as_str()))
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
/// * an op may end in `@self` (`T:fire@self:...`): that event aims at the player even when `AA_COMBAT_FAKE_TARGET` names a unit;
///   `@ground`: it aims at the spot where that unit stands (a ground skill: Meteor Strike, Missile Rain)
/// * `T:tp:X:Y[:YAW]`: the player is put there (whole metres and degrees)
/// * `T:buff:INDEX:BUFF_ID[:MS]`, `T:unbuff:INDEX`: a buff arrives on / leaves the target (its aura, pose and freeze: `combat/buffs.rs`)
/// * `T:use:SKILL[:TARGET]`: the player asks the server for a skill (a `UseSkill` command, as the action bar sends it)
///
/// The target is the player itself, or the unit `AA_COMBAT_FAKE_TARGET` names (an object id, `model=ID`, `npc=TEMPLATE` (the nearest) or a part of the unit's name:
/// `model=1290` is the training dummy; the events wait up to 20 s for it to be in view). The first step opens `aa_dev::RecordGate` (`--record-gate`).
#[derive(Resource)]
struct Fake {
    /// (time, arguments, op, aim: 0 the target unit, 1 the player itself, 2 the ground under the target unit)
    steps: Vec<(f64, Vec<u32>, &'static str, u8)>,
    started: Option<f64>,
    /// `AA_COMBAT_FAKE_TARGET`.
    target: Option<String>,
}

impl Fake {
    fn from_env() -> Option<Fake> {
        let text = std::env::var("AA_COMBAT_FAKE").ok()?;
        let mut steps: Vec<(f64, Vec<u32>, &'static str, u8)> = Vec::new();
        for step in text.split(';').filter(|s| !s.trim().is_empty()) {
            let mut parts = step.split(':');
            let at: f64 = parts.next()?.trim().parse().ok()?;
            let raw = parts.next()?.trim();
            let (raw, own) = match (raw.strip_suffix("@self"), raw.strip_suffix("@ground")) {
                (Some(op), _) => (op, 1),
                (_, Some(op)) => (op, 2),
                _ => (raw, 0),
            };
            let op = match raw {
                "start" => "start",
                "fire" => "fire",
                "plot" => "plot",
                "hit" => "hit",
                "end" => "end",
                "stop" => "stop",
                "plotend" => "plotend",
                "plotstop" => "plotstop",
                "buff" => "buff",
                "unbuff" => "unbuff",
                "use" => "use",
                "tp" => "tp",
                other => {
                    warn!("combat: AA_COMBAT_FAKE: unknown step {other}");
                    continue;
                }
            };
            steps.push((at, parts.filter_map(|p| p.trim().parse().ok()).collect(), op, own));
        }
        steps.sort_by(|a, b| b.0.total_cmp(&a.0));
        Some(Fake { steps, started: None, target: std::env::var("AA_COMBAT_FAKE_TARGET").ok().filter(|t| !t.is_empty()) })
    }

    fn event(op: &str, a: &[u32], me: u32, other: u32, ground: Option<Pos>) -> Option<Event> {
        let arg = |i: usize| a.get(i).copied().unwrap_or(0);
        let caster = BuffCaster { kind: 0, obj_id: me };
        let target = match ground {
            Some(pos) => SkillTarget { kind: aa_net::skill::target_kind::POSITION, obj_id: 0, pos: Some(pos) },
            None => SkillTarget { kind: 0, obj_id: other, pos: None },
        };
        let raw_caster = aa_net::SkillCasterData::Unit(me);
        let raw_target = aa_net::SkillTargetData::Unit(other);
        let tl = arg(0) as u16;
        Some(match op {
            "start" => Event::SkillStarted(SkillStarted { skill_id: arg(1), tl, caster, target, cast_ms: arg(2), base_cast_ms: arg(2), synergy: 0, result: 0, result_value: 0, raw_result: Default::default(), raw_caster, raw_target }),
            "fire" => Event::SkillFired(SkillFired { skill_id: arg(1), tl, caster, target, delay_ms: arg(2) as i32, channeling_ms: arg(3) as i32, fire_anim_id: arg(4), raw_final_flag: 0, result: Default::default(), raw_caster, raw_target }),
            "plot" => Event::PlotEvent(PlotEvent { tl, event_id: arg(1), skill_id: arg(2), caster_obj_id: me, target_obj_id: if ground.is_some() { 0 } else { other }, caster_pos: None, target_pos: ground, casting_ms: arg(3) }),
            "hit" => Event::UnitDamaged(UnitDamage {
                action: CastAction { kind: 0, skill_id: arg(1), tl, buff_id: 0 },
                caster,
                caster_obj_id: me,
                target_obj_id: other,
                damage: arg(2) as i32,
                absorbed: 0,
                mana_burn: 0,
                hit_type: 0,
            }),
            "end" => Event::SkillEnded { tl },
            "stop" => Event::CastingStopped { tl, duration_ms: 0 },
            "plotend" => Event::PlotEnded { tl },
            "plotstop" => Event::PlotStopped { tl, duration_ms: 0, last_event: 0, channeling: true },
            // `buff:INDEX:BUFF_ID[:DURATION_MS]`, `unbuff:INDEX`: a buff on the target (the unit's list in `Buffs` follows)
            "buff" => Event::BuffCreated { obj_id: other, buff: aa_net::buff::Buff { index: arg(0), buff_id: arg(1), duration_ms: arg(2) as i32, caster, stacks: 1, ..Default::default() } },
            "unbuff" => Event::BuffRemoved { obj_id: other, index: arg(0) },
            _ => return None,
        })
    }
}

fn fake_events(
    time: Res<Time>,
    mut fake: ResMut<Fake>,
    local: Query<&LocalPlayer>,
    mut movers: Query<&mut Mover, With<LocalPlayer>>,
    mut units: ResMut<Units>,
    mut alone: Local<bool>,
    mut gate: ResMut<aa_dev::RecordGate>,
    mut buffs: Option<ResMut<aa_net::state::Buffs>>,
    mut shot: Option<ResMut<DevShot>>,
    mut out: MessageWriter<NetEvent>,
    mut commands: MessageWriter<NetCommand>,
) {
    let Ok(me) = local.single() else { return };
    let now = time.elapsed_secs_f64();
    let started = *fake.started.get_or_insert(now);
    gate.open(started);
    // the unit the events aim at: AA_COMBAT_FAKE_TARGET (an object id or a part of the name), else the player itself
    let other = match &fake.target {
        None => me.obj_id,
        Some(t) => {
            let wanted = t.to_lowercase();
            // `npc=TEMPLATE` / `model=ID`: the nearest live unit of that npc template or model (the rats of the pond are npc 10004)
            let here = movers.single().map(|m| m.pos).unwrap_or_default();
            let nearest = |keep: &dyn Fn(&aa_net::model::Unit) -> bool| {
                units
                    .by_id
                    .values()
                    .filter(|u| !u.dead && u.obj_id != me.obj_id && keep(u))
                    .min_by(|a, b| {
                        let d = |u: &aa_net::model::Unit| Vec3::new(u.pos.x, u.pos.y, u.pos.z).distance(here);
                        d(a).total_cmp(&d(b))
                    })
                    .map(|u| u.obj_id)
            };
            let found = if let Some(template) = wanted.strip_prefix("npc=").and_then(|n| n.parse::<u32>().ok()) {
                nearest(&|u| matches!(u.kind, aa_net::model::UnitKind::Npc { template_id, .. } if template_id == template))
            } else if let Some(model) = wanted.strip_prefix("model=").and_then(|n| n.parse::<u32>().ok()) {
                nearest(&|u| u.model_id == model)
            } else {
                units.by_id.values().find(|u| u.obj_id != me.obj_id && (u.obj_id.to_string() == wanted || u.name.to_lowercase().contains(&wanted))).map(|u| u.obj_id)
            };
            match found {
                Some(id) => id,
                None if now - started < 20.0 => return,
                None => {
                    warn_once!("combat: AA_COMBAT_FAKE_TARGET {t} not in view: the player is the target (units: {:?})", units.by_id.values().filter(|u| u.faction_id == 101).take(12).map(|u| (u.obj_id, u.model_id, u.kind.clone(), u.faction_id)).collect::<Vec<_>>());
                    me.obj_id
                }
            }
        }
    };
    // `AA_COMBAT_FAKE_ALONE=1`: the other units of the target's model within 15 m are dropped once (every `/spawn npc` leaves a doll on the
    // stage: they stand stacked and come apart when a skill pulls or pushes one)
    if !*alone && other != me.obj_id && std::env::var_os("AA_COMBAT_FAKE_ALONE").is_some() {
        *alone = true;
        if let Some((model, pos)) = units.by_id.get(&other).map(|u| (u.model_id, u.pos)) {
            let extra: Vec<u32> = units.by_id.values().filter(|u| u.obj_id != other && u.model_id == model && (u.pos.x - pos.x).hypot(u.pos.y - pos.y) < 15.0).map(|u| u.obj_id).collect();
            for id in extra {
                units.by_id.remove(&id);
            }
        }
    }
    while fake.steps.last().is_some_and(|s| now - started >= s.0) {
        let (_, args, op, aim) = fake.steps.pop().unwrap();
        let ground = (aim == 2).then(|| units.by_id.get(&other).map(|u| u.pos)).flatten();
        if op == "tp" {
            // `T:tp:X:Y[:YAW_DEGREES]`: the player stands there (whole metres, facing in whole degrees counter-clockwise from
            // north), for reels of skills that move the caster
            if let Ok(mut mover) = movers.single_mut() {
                mover.pos.x = args.first().copied().unwrap_or(0) as f32;
                mover.pos.y = args.get(1).copied().unwrap_or(0) as f32;
                // wrapped into (-pi, pi] like every other facing (turning code compares facings without wrapping)
                let yaw = (args.get(2).copied().unwrap_or(0) as f32).to_radians();
                mover.facing = if yaw > std::f32::consts::PI { yaw - std::f32::consts::TAU } else { yaw };
                mover.velocity = Vec3::ZERO;
            }
        } else if op == "use" {
            // the player asks for a skill without the action bar (which refuses a press while a cast runs); target 0: the fake target
            commands.write(NetCommand(Command::UseSkill { skill_id: args.first().copied().unwrap_or(0), target_obj_id: args.get(1).copied().filter(|&t| t != 0).unwrap_or(other) }));
        } else if let Some(event) = Fake::event(op, &args, me.obj_id, if aim == 1 { me.obj_id } else { other }, ground) {
            // the buff list is the net state's: events written here do not reach it
            if let (Some(buffs), Event::BuffCreated { .. } | Event::BuffRemoved { .. }) = (buffs.as_mut(), &event) {
                buffs.apply(&event);
            }
            if let (Some(shot), Event::BuffCreated { .. }) = (shot.as_mut(), &event) {
                shot.event("buff", now);
            }
            out.write(NetEvent(event));
        }
    }
}

/// `AA_COMBAT_SHOT=<png>@<ask|start|fire|fire2|plot|hit|death|buff>+<seconds>[,<seconds>...]`: screenshots that many game seconds after the first
/// such event (the local player asked for a skill (`ask`: the key), the local player's skill started, the local player fired (`fire2`: for the second time; `plot`: a step of the player's
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
    if verbose() {
        app.add_systems(Update, (log_skill_requests, log_hitches));
    }
    if std::env::var_os("AA_COMBAT_CHASE").is_some() {
        app.init_resource::<super::camera::Chase>();
    }
    if let Some(degrees) = std::env::var("AA_COMBAT_CAM_REL").ok().and_then(|v| v.parse::<f32>().ok()) {
        app.insert_resource(super::camera::CamRel(degrees));
    }
    if let Some(sweep) = sweep::Sweep::from_env() {
        app.insert_resource(sweep).add_systems(Update, sweep::run_sweep.before(handle_events).run_if(resource_exists::<SkillBook>));
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
    app.add_message::<DamageLanded>()
        .add_systems(Update, pass_damage.run_if(not(resource_exists::<SkillBook>)).run_if(super::world_screens))
        .init_resource::<Combat>()
        .init_resource::<preload::Preload>()
        .init_resource::<CameraShake>()
        .init_resource::<PlayerInput>()
        .add_systems(OnExit(Screen::InGame), reset)
        .add_systems(
            Update,
            (
                (request_skill, handle_events, cancel_on_move, cancel_in_water, preempt_cast, retry_cast, run_timers, flush_landed, mark_corpses, ensure_corpses, buffs::reconcile_buffs, end_actions, sheathe, release_layers, fly_projectiles, update_beams, hold_weapons)
                    .chain()
                    .before(super::anim::drive_locomotion)
                    .run_if(resource_exists::<SkillBook>),
                turn_to_target.before(super::player::control_player),
                preload::run_preload.run_if(resource_exists::<SkillBook>).run_if(|| !preload::disabled()),
                throw_corpses.after(super::units::move_units),
            )
                .run_if(super::world_screens),
        )
        .add_systems(PostUpdate, follow_fx.before(TransformSystems::Propagate));
}

/// Hands the landed blows to the HUD.
fn flush_landed(mut combat: ResMut<Combat>, mut out: MessageWriter<DamageLanded>) {
    for landed in combat.landed.drain(..) {
        out.write(landed);
    }
}

/// Without the game database there are no blows to wait for: the HUD gets the packets as they come.
fn pass_damage(time: Res<Time>, mut events: MessageReader<NetEvent>, mut out: MessageWriter<DamageLanded>) {
    for NetEvent(event) in events.read() {
        if let Event::UnitDamaged(d) = event {
            out.write(DamageLanded { damage: d.clone(), received: time.elapsed_secs_f64() });
        }
    }
}

/// `AA_COMBAT_LOG=1`: frames that took longer than 40 ms, to see what a skill's first use costs (clips decoded, libraries and
/// textures loaded, pipelines compiled).
fn log_hitches(real: Res<Time<Real>>, time: Res<Time>, mut worst: Local<f32>) {
    // `AA_COMBAT_HITCH_MS` moves the threshold (default 40 ms)
    static LIMIT: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    let limit = *LIMIT.get_or_init(|| std::env::var("AA_COMBAT_HITCH_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(40.0));
    let ms = real.delta_secs() * 1000.0;
    if ms > limit && time.elapsed_secs() > 5.0 {
        info!("combat: [{:.3}] hitch: a frame took {ms:.0} ms", time.elapsed_secs_f64());
    }
    *worst = worst.max(ms);
}

/// `AA_COMBAT_LOG=1`: the skills the player asks for, to read the delay of the server's answer off the log's time stamps.
fn log_skill_requests(time: Res<Time>, mut commands: MessageReader<NetCommand>) {
    for NetCommand(cmd) in commands.read() {
        if let Command::UseSkill { skill_id, target_obj_id } = cmd {
            info!("combat: [{:.3}] asked for skill {skill_id} on {target_obj_id}", time.elapsed_secs_f64());
        }
    }
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
    emotes: Option<Res<'w, super::emote::Emotes>>,
    net_units: Res<'w, Units>,
    /// A skill aimed at a doodad (a Memory Tome ...) is aimed at its position (object ids are shared by units and doodads).
    net_doodads: Res<'w, aa_net::state::Doodads>,
    skills: Res<'w, Skills>,
    wield: Query<'w, 's, &'static Wield>,
    drawn: Query<'w, 's, &'static mut Drawn>,
    remote: Query<'w, 's, (Entity, &'static RemoteUnit)>,
    local: Query<'w, 's, (Entity, &'static LocalPlayer)>,
    #[allow(clippy::type_complexity)]
    actors: Query<'w, 's, (&'static mut Animator, Option<&'static Locomotion>, Option<&'static Acting>, Option<&'static Motion>, Has<Dead>)>,
    xforms: Query<'w, 's, &'static GlobalTransform>,
    audio: Audio<'w, 's>,
    motion: ResMut<'w, super::skill_motion::Inbox>,
}

/// Picks the entity of an object id from the local player and the remote units: the local one wins when both have it.
fn unit_entity(obj: u32, local: impl Iterator<Item = (Entity, u32)>, remote: impl Iterator<Item = (Entity, u32)>) -> Option<Entity> {
    if obj == 0 {
        return None;
    }
    local.into_iter().find(|(_, id)| *id == obj).map(|(e, _)| e).or_else(|| remote.into_iter().find(|(_, id)| *id == obj).map(|(e, _)| e))
}

impl Stage<'_, '_> {
    /// The object id of a unit's entity.
    fn obj_of(&self, e: Entity) -> Option<u32> {
        self.local.get(e).map(|(_, l)| l.obj_id).or_else(|_| self.remote.get(e).map(|(_, r)| r.obj_id)).ok()
    }

    /// The entity of an object id: **the local player first**. The server is not supposed to send a second copy of it, but a
    /// stale remote entity with the same id (a spawn race, a zone change) must never take the local player's skill events: it
    /// is not the model the player's camera and rig animate, so the local client would show nothing while remote observers
    /// (which only have the remote unit) see the clip and the tool.
    fn entity(&self, obj: u32) -> Option<Entity> {
        unit_entity(obj, self.local.iter().map(|(e, l)| (e, l.obj_id)), self.remote.iter().map(|(e, r)| (e, r.obj_id)))
    }

    fn is_local(&self, e: Entity) -> bool {
        self.local.contains(e)
    }

    /// The class of weapon a unit holds.
    fn wield(&self, unit: Entity) -> Wield {
        self.wield.get(unit).copied().unwrap_or_default()
    }

    /// Puts the tools of the skill whose clip just started on the unit (an empty set takes an older skill's tools away).
    fn set_tools(&mut self, unit: Entity, visual: &skills::SkillVisual) {
        if visual.tools.iter().any(Option::is_some) {
            self.commands.entity(unit).try_insert(SkillTool { items: visual.tools.clone() });
        } else {
            self.commands.entity(unit).try_remove::<SkillTool>();
        }
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
            None => self
                .net_units
                .get(obj)
                .map(|u| u.pos)
                .or_else(|| self.net_doodads.by_id.get(&obj).map(|d| d.pos))
                .map(|p| cry_to_bevy(Vec3::new(p.x, p.y, p.z))),
        }
    }

    /// The bone entity of an effect bone id (`None` for the origin and for rigs without such a bone).
    fn bone(&self, unit: Entity, id: u8) -> Option<Entity> {
        let (animator, ..) = self.actors.get(unit).ok()?;
        find_bone(animator, bone_names(id))
    }

    /// World position of an effect bone of a unit: the bone, else the unit's origin (id 0) or the origin raised to `fallback_height`.
    fn bone_point(&self, unit: Entity, id: u8, fallback_height: f32) -> Option<Vec3> {
        match self.bone(unit, id).and_then(|b| self.xforms.get(b).ok()) {
            Some(g) => Some(g.translation()),
            None => self.xforms.get(unit).ok().map(|g| g.translation() + if id == 0 { Vec3::ZERO } else { Vec3::Y * fallback_height * g.scale().y }),
        }
    }

    /// World position of a projectile's source bone (`src_bone_id`, the table of `src_bone_names`) on its caster.
    fn source_point(&self, unit: Entity, id: u8) -> Option<Vec3> {
        let found = self.actors.get(unit).ok().and_then(|(animator, ..)| source_bone(animator, id));
        match found.and_then(|b| self.xforms.get(b).ok()) {
            Some(g) => Some(g.translation()),
            None => self.xforms.get(unit).ok().map(|g| g.translation() + if (25..=30).contains(&id) { Vec3::Y * g.scale().y } else { Vec3::ZERO }),
        }
    }

    /// The hit reaction clip of a unit struck from `from`: `base` (`all_re_combat_hit` / `all_re_combat_critical`) in the variant the
    /// unit's rig has: the `twohand_` clips of a unit with a two handed weapon and, for a blow from the side, `_l` / `_r`. The `_l`
    /// clips throw the head to the unit's left (`hit_reaction_directions`), so they answer a blow from its right and the `_r` ones a
    /// blow from its left. A blow from the front or the back plays the plain clip (the real client has `additive_*_f` / `_b` clips
    /// for those; they are layers and not played here).
    fn hit_reaction(&self, unit: Entity, from: Option<Vec3>, base: &'static str) -> String {
        let two = self.wield(unit).two_handed;
        let critical = base.ends_with("critical");
        let side = from.zip(self.xforms.get(unit).ok()).and_then(|(from, g)| {
            let local = g.affine().inverse().transform_point3(from);
            let flat = Vec2::new(local.x, local.z);
            // more than 40 degrees off the front / back axis
            (flat.length() > 0.2 && flat.x.abs() > flat.length() * 0.64).then_some(if flat.x > 0.0 { "_l" } else { "_r" })
        });
        let prefixed = |name: &str| if two { name.replacen("all_", "twohand_", 1) } else { name.to_string() };
        let mut candidates = Vec::new();
        if let Some(side) = side.filter(|_| !critical) {
            candidates.push(prefixed(&format!("{base}{side}")));
            candidates.push(format!("{base}{side}"));
        }
        candidates.push(prefixed(base));
        candidates.push(base.to_string());
        candidates.into_iter().find(|n| self.clip(unit, n).is_some()).unwrap_or_else(|| base.to_string())
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
                if verbose() {
                    info!("combat: {unit:?} does not play {name}: dead {dead}, locomotion {}, acting {}", loco.is_some(), acting.is_some());
                }
                return None;
            }
            let moving = kind != Act::Hurt && !full_body_only() && motion.is_some_and(|m| m.speed > 0.3);
            loco.filter(|l| moving && l.clips.can_move()).and_then(|l| l.clips.upper.clone())
        };
        // a stop marker (`all_ac_stop`, the clip of a skill that only ends what the unit is doing) releases the unit
        if skills::is_stop_clip(name) {
            self.commands.entity(unit).try_remove::<Acting>();
            self.commands.queue(move |w: &mut World| {
                if let Some(mut l) = w.get_mut::<Locomotion>(unit) {
                    l.invalidate();
                }
            });
            return None;
        }
        let layered = mask.is_some();
        let clip = if mask.is_some() { self.upper_clip(unit, name) } else { None };
        let Some(clip) = clip.or_else(|| self.clip(unit, name).map(|(clip, _)| clip)) else {
            if verbose() {
                info!("combat: unit has no clip {name}");
            }
            return None;
        };
        let duration = clip.clip.duration;
        // another action replaces the clip (and the tool it held); the skill's own caller puts its tool back after
        self.commands.entity(unit).try_remove::<SkillTool>();
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
            && !name.contains("_ac_holster")
            && let Some(out) = weapon_for_clip(name)
        {
            // the weapon the clip is for comes out of its sheath and stays out for a while
            self.commands.entity(unit).insert(Drawn::after_action(until, out));
        }
        if verbose() {
            info!("combat: [{now:.3}] {name} on {unit:?} ({kind:?}, {duration:.2} s{})", if layered { ", upper body" } else { "" });
        }
        Some(duration)
    }

    /// Starts one item of an effect group on a unit: a particle effect (with the sound it carries), or a sound (a voice is the unit's
    /// own). `toward` is the other party (a hit effect turns to it). Returns the entity that plays it.
    fn spawn_item(&mut self, item: &FxItem, on: Entity, toward: Option<Vec3>, hold: bool) -> Option<Entity> {
        if item.effect.is_empty() {
            return self.sound_item(item, on);
        }
        if muted(&item.effect) {
            return None;
        }
        if verbose() {
            info!("combat: effect {} (start {}, end {}) on {on:?} bone {}{}", item.effect, item.start, item.end, item.bone, if hold { " held" } else { "" });
        }
        let effect = self.spawn_effect(item, on, toward, hold, if hold { 12.0 } else { 6.0 })?;
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

    /// Starts the effect of a clip's anim event on the bone it names (the unit's origin without one): once, at the offset in the
    /// unit's frame.
    fn spawn_clip_effect(&mut self, unit: Entity, e: &clips::ClipEffect) {
        let bone = if e.bone.is_empty() {
            None
        } else {
            self.actors.get(unit).ok().and_then(|(animator, ..)| {
                let i = animator.rig.skeleton.bones.iter().position(|b| b.name == e.bone)?;
                animator.bones.get(i).copied()
            })
        };
        if verbose() {
            info!("combat: clip effect {} on {:?} bone '{}'", e.effect, unit, e.bone);
        }
        let Ok(unit_xf) = self.xforms.get(unit).copied() else { return };
        let (scale, rotation, _) = unit_xf.to_scale_rotation_translation();
        let parent = bone.unwrap_or(unit);
        let Ok(parent_xf) = self.xforms.get(parent).copied() else { return };
        let world = parent_xf.translation() + rotation * cry_to_bevy(Vec3::from(e.offset)) * scale.y;
        let (parent_scale, parent_rotation, _) = parent_xf.to_scale_rotation_translation();
        let local = Transform {
            translation: parent_xf.affine().inverse().transform_point3(world),
            rotation: parent_rotation.inverse() * rotation,
            scale: Vec3::splat(scale.y / ((parent_scale.x + parent_scale.y + parent_scale.z) / 3.0).max(1e-4)),
        };
        let effect = Effect { prime: false, despawn_when_finished: true, ..Effect::new(e.effect.clone()) };
        self.commands.spawn((Name::new(format!("fx {}", e.effect)), effect, local, ChildOf(parent), Fx { follow: None, fixed_rotation: false, ttl: 6.0 }));
    }

    /// Starts an effect of a skill that was aimed at a point, not a unit, on the ground there (`hold`: it lives until released).
    fn spawn_ground_item(&mut self, item: &FxItem, at: Vec3, toward: Option<Vec3>, hold: bool) -> Option<Entity> {
        if item.effect.is_empty() {
            return None;
        }
        Some(ground_effect(&mut self.commands, item, at, toward, hold))
    }

    /// Starts an effect that lives until the caller despawns it (the aura of a buff): no safety timer.
    fn spawn_held(&mut self, item: &FxItem, on: Entity) -> Option<Entity> {
        if item.effect.is_empty() {
            // a sound item that lasts as long as the buff: its entity is the one the caller despawns when the buff goes
            return self.sound_item(item, on);
        }
        let effect = self.spawn_effect(item, on, None, true, f32::MAX)?;
        let at = self.bone_point(on, item.bone, 1.0).unwrap_or(Vec3::ZERO);
        self.effect_sounds(item, on, effect, at);
        Some(effect)
    }

    fn spawn_effect(&mut self, item: &FxItem, on: Entity, toward: Option<Vec3>, hold: bool, ttl: f32) -> Option<Entity> {
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
        if item.flat() {
            // the ring lies flat: the emitters' axis (the effect's forward, -Z) points up
            transform.rotation *= Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
        }
        if item.axis == 4
            && let Some(from) = toward
        {
            let dir = Vec3::new(from.x - anchor.x, 0.0, from.z - anchor.z).normalize_or_zero();
            if dir != Vec3::ZERO {
                transform.rotation = Transform::IDENTITY.looking_to(dir, Vec3::Y).rotation;
                fixed_rotation = true;
            }
        }
        let effect = Effect { scale: item.scale * std::env::var("AA_FX_SCALE").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(1.0), prime: false, item: true, body: Some(on), despawn_when_finished: !hold, no_refraction: no_refraction(&item.effect), once: once_emitters(&item.effect), ..Effect::new(item.effect.clone()) };
        // a character effect (the crows of Summon Crows) plays its start and loop clips, and its end clip when the time is up
        let fx_ttl = if item.chr { ttl + 2.0 } else { ttl };
        let chr = item.chr.then(|| ChrEffect { scale: item.scale, life: Some(ttl), ..ChrEffect::new(item.effect.clone()) });
        if fixed_rotation {
            // a hit effect turned towards the attacker keeps that orientation: it only follows the bone's position (`follow_fx`)
            let follow = bone.map(|b| (on, b, offset));
            let mut e = self.commands.spawn((Name::new(format!("fx {}", item.effect)), transform, Fx { follow, fixed_rotation, ttl: fx_ttl }));
            match chr {
                Some(chr) => e.insert(chr),
                None => e.insert(effect),
            };
            return Some(e.id());
        }
        // the effect is a child of its bone (of the unit, for an effect on the origin), so that its tails and `InheritVelocity` see
        // the motion of the bone; the local pose gives it the world pose it would have had: the unit's orientation and scale
        let parent = bone.unwrap_or(on);
        let parent_xf = *self.xforms.get(parent).ok()?;
        let (parent_scale, parent_rotation, _) = parent_xf.to_scale_rotation_translation();
        let mut local = Transform {
            translation: parent_xf.affine().inverse().transform_point3(transform.translation),
            rotation: parent_rotation.inverse() * transform.rotation,
            scale: Vec3::splat(transform.scale.y / ((parent_scale.x + parent_scale.y + parent_scale.z) / 3.0).max(1e-4)),
        };
        if item.axis == ev::BONE_AXIS && bone.is_some() {
            // a blade trail has the bone's orientation (the weapon's +Y is the blade, which aa-fx takes as the ribbon's width axis)
            local.rotation = Quat::IDENTITY;
        }
        let mut e = self.commands.spawn((Name::new(format!("fx {}", item.effect)), local, ChildOf(parent), Fx { follow: None, fixed_rotation: false, ttl: fx_ttl }));
        match chr {
            Some(chr) => e.insert(chr),
            None => e.insert(effect),
        };
        Some(e.id())
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
fn start_items(st: &mut Stage, combat: &mut Combat, tl: u16, items: &[FxItem], moments: &[u8], caster: Entity, target: Option<Entity>, ground: Option<Vec3>, now: f64, delay: f64, skip_short: bool) {
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
        // the target of the skill is a point: what happens there starts on the ground
        if item.on_target
            && target.is_none()
            && let Some(at) = ground
        {
            combat.after(now + delay, Later::GroundFx { item: item.clone(), tl, at, toward: toward_caster });
            continue;
        }
        let (on, toward) = if item.on_target { (target, toward_caster) } else { (Some(caster), toward_target) };
        let Some(on) = on else { continue };
        combat.after(now + delay, Later::Fx { item: item.clone(), tl, on, toward, once: false, detach: None });
    }
}

/// The blade trails of a swing: the effects that follow the orientation of the weapon bone (`offset_axis_id` 3) and live from
/// the swing's form moment to its throw moment (`sword_trail_test1`, the slash trails of the fighter skills). An instant skill skips
/// what lives until it fires (`start_items`), but these belong to the clip: they start where its anim events say (`form` /
/// `throw`) and run once.
#[allow(clippy::too_many_arguments)]
fn start_swings(st: &Stage, combat: &mut Combat, tl: u16, items: &[FxItem], caster: Entity, target: Option<Entity>, now: f64, events: &clips::ClipEvents) {
    let toward_caster = st.xforms.get(caster).ok().map(|g| g.translation());
    let toward_target = target.and_then(|t| st.xforms.get(t).ok()).map(|g| g.translation());
    for item in items.iter().filter(|i| i.axis == ev::BONE_AXIS && i.end > i.start && i.end <= ev::LAUNCH) {
        let at = match item.start {
            ev::FORM => events.form,
            ev::THROW => events.throw,
            _ => None,
        };
        let Some(at) = at else { continue };
        let (on, toward) = if item.on_target { (target, toward_caster) } else { (Some(caster), toward_target) };
        let Some(on) = on else { continue };
        // the trail ends at the throw moment of the swing (`fx_event_end_id` 2), the data's end of the effect
        let detach = match item.end {
            ev::THROW => events.throw.map(|t| f64::from((t - at).max(0.05))),
            _ => None,
        };
        combat.after(now + f64::from(at), Later::Fx { item: item.clone(), tl, on, toward, once: true, detach });
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
    if played.is_some() && name.starts_with("all_") {
        // a spell puts the staff back on the back (the real client, after a weapon skill had sent it to the hip)
        st.commands.entity(caster).try_insert(StaffSide(false));
    }
    if played.is_some()
        && let Some(cast) = combat.casts.get_mut(&tl)
    {
        cast.casting = looping;
        cast.played = true;
        combat.owner.insert(cast.caster, tl);
    }
    if played.is_some() && !looping {
        clip_effects(st, combat, caster, name, now);
    }
    played
}

/// The effects the clip starts itself (`effect` anim events: the flowers of `/flowers`, the tears of `/sad`), at their times.
fn clip_effects(st: &Stage, combat: &mut Combat, unit: Entity, name: &str, now: f64) {
    let Some(cal) = st.actors.get(unit).ok().and_then(|(_, loco, ..)| loco.map(|l| l.clips.cal.clone())) else { return };
    for effect in clips::clip_effects(&cal, name).iter() {
        combat.after(now + f64::from(effect.time), Later::ClipFx { unit, effect: effect.clone() });
    }
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

/// How long an emote whose clip loops (sitting on the ground ...) is held.
const EMOTE_LOOP_SECS: f64 = 8.0;

/// Server events to clips and effects.
fn handle_events(
    mut events: MessageReader<NetEvent>,
    time: Res<Time>,
    mut st: Stage,
    mut combat: ResMut<Combat>,
    mut clock: ResMut<Time<Virtual>>,
    mut shot: Option<ResMut<DevShot>>,
) {
    let now = time.elapsed_secs_f64();
    // what waited for the draw of the weapon goes first
    if combat.held.first().is_some_and(|(at, _)| *at <= now) {
        let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut combat.held).into_iter().partition(|(at, _)| *at <= now);
        combat.held = later;
        for (_, event) in &due {
            apply(event, now, &mut st, &mut combat, &mut clock, &mut shot);
        }
    }
    if combat.held.is_empty() {
        combat.held_tls.clear();
    }
    if combat.lead.is_some_and(|l| l.ready <= now) {
        combat.lead = None;
    }
    for NetEvent(event) in events.read() {
        // retail: a tool skill puts the drawn weapon away at once, with no holster clip and no delay
        if let Some((caster, skill)) = tool_event(event)
            && let Some(unit) = st.entity(caster)
        {
            sheathe_for_tool(&mut st, unit, skill, now);
        }
        if let Some(at) = hold_until(&combat, event, now) {
            if let Some(tl) = tl_of(event)
                && !combat.held_tls.contains(&tl)
            {
                combat.held_tls.push(tl);
            }
            combat.held.push((at, event.clone()));
            continue;
        }
        apply(event, now, &mut st, &mut combat, &mut clock, &mut shot);
    }
}

/// When the event of a skill the player drew its weapon for may go on: after the draw (`None`: now).
fn hold_until(combat: &Combat, event: &Event, now: f64) -> Option<f64> {
    let lead = combat.lead.filter(|l| l.ready > now)?;
    let mine = |obj: u32| obj == lead.caster;
    let held = |tl: &u16| combat.held_tls.contains(tl);
    let hold = match event {
        Event::SkillStarted(s) => !s.failed() && mine(s.caster.obj_id),
        Event::SkillFired(f) => mine(f.caster.obj_id),
        Event::PlotEvent(p) => mine(p.caster_obj_id),
        Event::UnitDamaged(d) => mine(d.caster_obj_id) || held(&d.action.tl),
        Event::UnitDeath { .. } => !combat.held.is_empty(),
        Event::SkillEnded { tl } | Event::CastingStopped { tl, .. } | Event::PlotEnded { tl } | Event::PlotStopped { tl, .. } => held(tl),
        Event::SkillStopped { obj_id, .. } => mine(*obj_id),
        _ => false,
    };
    hold.then_some(lead.ready)
}

/// The timeline an event of a skill belongs to.
fn tl_of(event: &Event) -> Option<u16> {
    match event {
        Event::SkillStarted(s) => Some(s.tl),
        Event::SkillFired(f) => Some(f.tl),
        Event::PlotEvent(p) => Some(p.tl),
        Event::UnitDamaged(d) => Some(d.action.tl),
        _ => None,
    }
}

/// The caster and skill of an event that may put a tool in a hand (a refused start uses none).
fn tool_event(event: &Event) -> Option<(u32, u32)> {
    match event {
        Event::SkillStarted(s) if !s.failed() => Some((s.caster.obj_id, s.skill_id)),
        Event::SkillFired(f) => Some((f.caster.obj_id, f.skill_id)),
        Event::PlotEvent(p) => Some((p.caster_obj_id, p.skill_id)),
        _ => None,
    }
}

/// One server event, at `now`.
#[allow(clippy::too_many_arguments)]
fn apply(event: &Event, now: f64, st: &mut Stage, combat: &mut Combat, clock: &mut Time<Virtual>, shot: &mut Option<ResMut<DevShot>>) {
    // skills that move units start with their clip (`skill_motion`)
    super::skill_motion::observe(&mut st.motion, event, now);
    if verbose()
        && matches!(
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
        )
    {
        info!("combat: [{now:.3}] {event:?}");
    }
    match event {
        Event::SkillStarted(s) if s.failed() => {
            // not facing the target: turn, tell the server, and ask once more (the real client turns on its own)
            let own = st.local.iter().any(|(_, l)| l.obj_id == s.caster.obj_id);
            // a skill that does not start (not ready, too far ...) puts the weapon it drew away again; a missing turn only waits
            if own && s.result != aa_net::skill::result::OUTOF_ANGLE && combat.lead.is_some_and(|l| l.skill == s.skill_id) {
                cancel_draw(st, combat, now);
            }
            if own && s.result == aa_net::skill::result::OUTOF_ANGLE && s.target.obj_id != 0 && combat.retry.is_none() {
                let target = st.entity(s.target.obj_id);
                if let Some(me) = st.entity(s.caster.obj_id) {
                    face(st, me, target, None, now);
                }
                combat.retry = Some(Retry { at: now + 0.2, skill: s.skill_id, target: s.target.obj_id });
            }
        }
        Event::SkillStarted(s) => {
            let Some(caster) = st.entity(s.caster.obj_id) else { return };
            if let Some(shot) = shot.as_mut()
                && st.is_local(caster)
            {
                shot.event("start", now);
            }
            let Some(visual) = st.book.skill(s.skill_id) else { return };
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
                if let Some(name) = visual.instrument_start_clip(wield.instrument).or(visual.start_anim.as_ref()).cloned()
                    && play_clip(st, combat, s.tl, caster, &name, Act::Cast, hold, now).is_some()
                {
                    st.set_tools(caster, &visual);
                }
                face(st, caster, target, target_pos, now);
                if !visual.plot {
                    start_items(st, combat, s.tl, &look.fx, &[ev::START, ev::FORM], caster, target, target_pos, now, 0.0, false);
                    start_shakes(&st, combat, &look.shakes, &[ev::START, ev::FORM], caster, target, now, 0.0);
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
            let Some(caster) = st.entity(f.caster.obj_id) else { return };
            let Some(visual) = st.book.skill(f.skill_id) else { return };
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
            let mut clip_events = clips::ClipEvents::default();
            if let Some(name) = &name {
                let events = st.events(caster, name);
                clip_events = events;
                release = f64::from(events.release().unwrap_or(0.0));
                voice = f64::from(events.voice_time().unwrap_or(0.0));
                combat.casts.get_mut(&f.tl).unwrap().sync = f64::from(events.combat_sync.unwrap_or(0.0));
                // a channel pose loops for the channeling time
                let hold = if f.channeling_ms > 0 { f64::from(f.channeling_ms) / 1000.0 + 2.0 } else { 20.0 };
                played = play_clip(st, combat, f.tl, caster, name, Act::Attack, hold, now).is_some();
            }
            if played {
                st.set_tools(caster, &visual);
            } else {
                // casting ends with nothing to play: let locomotion resume
                end_loop(st, combat, f.tl);
            }
            if release == 0.0 {
                release = f64::from(visual.effect_delay_ms) / 1000.0;
            }
            face(st, caster, target, target_pos, now);
            combat.release(&mut st.commands, f.tl, ev::LAUNCH);
            if !visual.plot {
                // an instant skill has not shown its start and form effects yet
                if fresh {
                    start_items(st, combat, f.tl, &look.fx, &[ev::START, ev::FORM], caster, target, target_pos, now, 0.0, true);
                    start_shakes(&st, combat, &look.shakes, &[ev::START, ev::FORM], caster, target, now, 0.0);
                }
                start_swings(st, combat, f.tl, &look.fx, caster, target, now, &clip_events);
                start_items(st, combat, f.tl, &look.fx, &[ev::THROW], caster, target, target_pos, now, 0.0, false);
                start_items(st, combat, f.tl, &look.fx, &[ev::LAUNCH], caster, target, target_pos, now, release, false);
                // the voice line at the moment the clip calls it
                start_items(st, combat, f.tl, &look.fx, &[ev::VOICE], caster, target, target_pos, now, if voice > 0.0 { voice } else { release }, false);
                start_shakes(&st, combat, &look.shakes, &[ev::THROW], caster, target, now, 0.0);
                start_shakes(&st, combat, &look.shakes, &[ev::LAUNCH], caster, target, now, release);
                if let Some(projectile) = &look.projectile {
                    start_shakes(&st, combat, &projectile.shakes, &[ev::START, ev::FORM, ev::THROW, ev::LAUNCH], caster, target, now, release);
                    let flight = (f64::from(f.delay_ms.max(0)) / 1000.0 - release).max(0.06);
                    combat.after(now + release, Later::Launch { projectile: projectile.clone(), tl: f.tl, caster, target, target_pos, duration: if projectile.is_beam() && f.channeling_ms > 0 { f.channeling_ms as f32 / 1000.0 } else { flight as f32 }, speed: 0.0, hold: f.channeling_ms > 0 });
                }
                // the doodad the channel spawns (Protective Wings: doodad 879 at the caster's feet) has clouts with a projectile: an effect
                // area on the ground under the caster that stands as long as the channel does
                if verbose() && !visual.clout_projectiles.is_empty() {
                    info!("combat: skill {} channels a doodad with {} clout projectile(s)", f.skill_id, visual.clout_projectiles.len());
                }
                for projectile in &visual.clout_projectiles {
                    let secs = if f.channeling_ms > 0 { f.channeling_ms as f32 / 1000.0 } else { 2.0 };
                    if let Some(e) = launch(st, projectile, caster, Some(caster), None, secs, 0.0)
                        && f.channeling_ms > 0
                        && let Some(cast) = combat.casts.get_mut(&f.tl)
                    {
                        cast.held.push((e, ev::END));
                    }
                }
            }
        }
        Event::PlotEvent(p) => {
            let Some(caster) = st.entity(p.caster_obj_id) else { return };
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
                && let Some(v) = visual.as_ref()
                && let Some(name) = v.instrument_start_clip(wield.instrument).or(v.start_anim.as_ref()).cloned()
                && play_clip(st, combat, p.tl, caster, &name, Act::Cast, f64::from(p.casting_ms) / 1000.0 + 1.0, now).is_some()
            {
                st.set_tools(caster, v);
            }
            let step = st.book.plot_event(p.event_id);
            // `all_co_sk_stop`: ends the clip that loops before this step (alone, it is the whole step)
            if step.as_ref().is_some_and(|s| s.stops()) {
                end_loop(st, combat, p.tl);
            }
            // the clip of the step; a skill that moves its caster (a dash) has only its controller's clip until a step has one
            let idle = !combat.casts.get(&p.tl).is_some_and(|c| c.played);
            let clip = step.as_ref().and_then(|s| s.clip().cloned()).or_else(|| if idle { visual.as_ref().and_then(|v| v.controller_anim.clone()) } else { None });
            let mut release = 0.0;
            let mut voice = 0.0;
            let mut clip_events = clips::ClipEvents::default();
            if let Some(name) = &clip {
                let events = st.events(caster, name);
                clip_events = events;
                release = f64::from(events.release().unwrap_or(0.0));
                voice = f64::from(events.voice_time().unwrap_or(0.0));
                let hold = if p.casting_ms > 0 { f64::from(p.casting_ms) / 1000.0 + 1.0 } else { 20.0 };
                if play_clip(st, combat, p.tl, caster, name, Act::Attack, hold, now).is_some()
                    && let Some(v) = &visual
                {
                    st.set_tools(caster, v);
                }
                combat.casts.get_mut(&p.tl).unwrap().sync = f64::from(events.combat_sync.unwrap_or(0.0));
            }
            let Some(step) = step else { return };
            face(st, caster, target, target_pos, now);
            if !step.fx.is_empty() {
                let cast = combat.casts.get_mut(&p.tl).unwrap();
                cast.hit_fx = step.fx.clone();
                cast.hit_shakes = step.shakes.clone();
            }
            start_swings(st, combat, p.tl, &step.fx, caster, target, now, &clip_events);
            start_items(st, combat, p.tl, &step.fx, &[ev::START, ev::FORM, ev::THROW], caster, target, target_pos, now, 0.0, true);
            start_items(st, combat, p.tl, &step.fx, &[ev::LAUNCH], caster, target, target_pos, now, release, false);
            start_items(st, combat, p.tl, &step.fx, &[ev::VOICE], caster, target, target_pos, now, if voice > 0.0 { voice } else { release }, false);
            start_shakes(&st, combat, &step.shakes, &[ev::START, ev::FORM, ev::THROW], caster, target, now, 0.0);
            start_shakes(&st, combat, &step.shakes, &[ev::LAUNCH], caster, target, now, release);
            if let Some(projectile) = &step.projectile {
                start_shakes(&st, combat, &projectile.shakes, &[ev::START, ev::FORM, ev::THROW, ev::LAUNCH], caster, target, now, release);
                // a beam lives as long as the step's `ProjectileAnim` says (the channel); a flight takes the time its speed needs
                let (duration, hold) = if projectile.is_beam() {
                    (step.projectile_ms as f32 / 1000.0, step.projectile_ms > 0)
                } else {
                    (0.25, visual.as_ref().is_some_and(|v| v.channel_ms > 0 || v.channeling_anim.is_some()))
                };
                combat.after(now + release, Later::Launch { projectile: projectile.clone(), tl: p.tl, caster, target, target_pos, duration, speed: step.projectile_speed, hold });
            }
        }
        Event::UnitDamaged(d) => {
            // buff ticks (kind 2) have no blow to show, and a unit that is not drawn has no blow to wait for: the numbers go on at once
            let now_landed = DamageLanded { damage: d.clone(), received: now };
            if d.action.kind > 1 {
                combat.landed.push(now_landed);
                return;
            }
            let Some(target) = st.entity(d.target_obj_id) else {
                combat.landed.push(now_landed);
                return;
            };
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
            if let Some(attacker) = caster.filter(|c| *c != target) {
                combat.last_attacker.insert(d.target_obj_id, attacker);
            }
            let last = combat.last_hit.entry(d.target_obj_id).or_default();
            *last = last.max(at);
            combat.after(at, Later::Landed(now_landed));
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
                st.commands.entity(e).remove::<(Dead, Corpse, Acting, Thrown)>();
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
            end_loop(st, combat, tl);
            if let Some(c) = combat.casts.get_mut(&tl) {
                c.cancel = None;
            }
            // the damage of this timeline may still be on its way: keep the cast until the timers ran out
            if let Some(c) = combat.casts.get_mut(&tl) {
                c.started = now - 15.0;
            }
        }
        Event::Emote(aa_net::EmoteEvent::Expressed { obj_id, emotion_id, .. }) => {
            // `SCEmotionExpressed` reaches everything in range, the sender included: the clip of the `express_texts` row
            let Some(unit) = st.entity(*obj_id) else { return };
            let Some(anim) = st.emotes.as_ref().and_then(|e| e.anim_id(*emotion_id)) else { return };
            let Some(name) = st.book.anim(anim) else { return };
            let looping = st.book.clip_loops(&name);
            if st.play(unit, &name, Act::Attack, looping, EMOTE_LOOP_SECS, now).is_some() {
                clip_effects(st, combat, unit, &name, now);
            }
        }
        Event::SkillStopped { obj_id, skill_id } => {
            let tls: Vec<u16> = combat.casts.iter().filter(|(_, c)| c.caster == *obj_id && c.skill == *skill_id).map(|(tl, _)| *tl).collect();
            for tl in tls {
                combat.release(&mut st.commands, tl, ev::END);
                end_loop(st, combat, tl);
                if let Some(c) = combat.casts.get_mut(&tl) {
                    c.cancel = None;
                }
            }
        }
        _ => {}
    }
}

/// Seconds a holster clip goes on after the weapon left the hand (the unit goes back to its idle then).
const HOLSTER_TAIL: f64 = 0.6;

/// `skills.id` of the basic melee attack.
const MELEE_ATTACK: u32 = 2;

/// The weapon a skill is played with: that of the first of its clips that is for one (the song's clip of the instrument held, the
/// fire and start clips, the first step of its plot). `None` for the skills that need none (spells, recall).
fn skill_weapon(book: &SkillBook, id: u32, wield: Wield) -> Option<Out> {
    // the basic melee attack has no clip in the data: the server names the weapon's when it fires
    if id == MELEE_ATTACK {
        return wield.melee.then_some(Out::Blade { dual: wield.dual });
    }
    let visual = book.skill(id)?;
    let clips = [
        visual.instrument_fire_clip(wield.instrument).cloned(),
        visual.instrument_start_clip(wield.instrument).cloned(),
        visual.fire_clip(wield.two_handed, wield.dual).cloned(),
        visual.start_anim.clone(),
    ];
    clips.into_iter().flatten().find_map(|c| weapon_for_clip(&c)).or_else(|| {
        if !visual.plot {
            return None;
        }
        book.plot_steps(id).into_iter().find_map(|step| book.plot_event(step).and_then(|p| p.clip().and_then(|c| weapon_for_clip(c))))
    })
}

/// The player presses a skill that needs its weapon while it is away: the weapon comes out and the shield comes onto the arm now, the
/// character takes its combat stance and turns to the target, and the server's answer to the skill waits for the draw
/// ([`DRAW_LEAD`]): the swing starts when the real client's does.
fn request_skill(time: Res<Time>, mut requests: MessageReader<NetCommand>, mut st: Stage, mut combat: ResMut<Combat>, mut shot: Option<ResMut<DevShot>>) {
    let now = time.elapsed_secs_f64();
    for NetCommand(cmd) in requests.read() {
        let Command::UseSkill { skill_id, target_obj_id } = cmd else { continue };
        let Ok((me, local)) = st.local.single() else { continue };
        let obj = local.obj_id;
        if let Some(shot) = shot.as_mut() {
            shot.event("ask", now);
        }
        if verbose() {
            // the units near the player, to put it in reach of a target in a test
            let here = st.xforms.get(me).map(|g| g.translation()).unwrap_or_default();
            let mut near: Vec<(f32, u32, &str)> = st.net_units.iter().filter(|u| !u.dead).map(|u| (st.point(u.obj_id).map_or(f32::MAX, |p| p.distance(here)), u.obj_id, u.name.as_str())).collect();
            near.sort_by(|a, b| a.0.total_cmp(&b.0));
            info!("combat: units near: {:?}", near.iter().take(5).map(|(d, id, n)| format!("{n} {id} {d:.1} m")).collect::<Vec<_>>());
        }
        // not the retry of a cancelled cast itself (the cast is gone by then)
        combat.preempt = Some(Preempt { skill: *skill_id, target: *target_obj_id });
        if st.actors.get(me).is_ok_and(|a| a.4) {
            continue;
        }
        let Some(out) = skill_weapon(&st.book, *skill_id, st.wield(me)) else { continue };
        if st.drawn.get(me).is_ok_and(|d| d.stance(now) == Some(out)) {
            continue;
        }
        st.commands.entity(me).insert(Drawn::after_action(now + DRAW_LEAD, out));
        if let Some(target) = st.entity(*target_obj_id).filter(|t| *t != me) {
            st.commands.entity(me).insert(FaceTarget { entity: Some(target), pos: None, until: now + 0.8 });
        }
        combat.lead = Some(Lead { ready: now + DRAW_LEAD, skill: *skill_id, caster: obj });
        if verbose() {
            info!("combat: [{now:.3}] the weapon comes out for skill {skill_id}, the swing waits {DRAW_LEAD} s");
        }
    }
}

/// The skill did not start: the events held for the draw go on at once and the weapon goes back.
fn cancel_draw(st: &mut Stage, combat: &mut Combat, now: f64) {
    combat.lead = None;
    for held in &mut combat.held {
        held.0 = now;
    }
    if let Ok((me, _)) = st.local.single() {
        st.commands.queue(move |w: &mut World| {
            if let Some(mut d) = w.get_mut::<Drawn>(me) {
                d.until = d.until.min(now);
            }
        });
    }
}

/// The state change of putting a weapon away: the stance ends now and the weapon is in its sheath at `leave` (seconds until it
/// leaves the hand; `None` for no holster clip, an instant sheathe). Returns when it is sheathed.
fn force_sheathe(drawn: &mut Drawn, now: f64, leave: Option<f32>) -> f64 {
    drawn.sheathing = true;
    drawn.until = leave.map_or(now, |at| now + f64::from(at.max(0.0)));
    drawn.until
}

/// Retail: a tool skill (planting, uprooting, watering, gathering) is worked with the weapon away. Every weapon, a sword with a
/// holster clip included, is put away at once (the owner's observation, 2026-10-21: no holster animation, no delay); the
/// skill's clip and its tool are shown immediately and the weapon stays away afterwards, drawn again only by the normal draw
/// triggers (a weapon skill).
fn tool_skill_sheathes(drawn: Option<&Drawn>, tools: &[Option<skills::Tool>; 2], now: f64) -> bool {
    tools.iter().any(Option::is_some) && drawn.is_some_and(|d| d.stance(now).is_some())
}

/// Sheathes a unit's drawn weapon through the same state change and holster clip [`sheathe`] uses when a fight is over.
/// Returns when the weapon is in its sheath (seconds on the app clock), `None` when nothing is drawn.
fn begin_sheathe(st: &mut Stage, unit: Entity, now: f64) -> Option<f64> {
    let (moving, dead) = st.actors.get(unit).ok().map(|(_, _, _, motion, dead)| (motion.is_some_and(|m| m.speed > 0.3), dead))?;
    if dead {
        return None;
    }
    let (out, until, sheathing) = st.drawn.get(unit).ok().map(|d| (d.out, d.until, d.sheathing))?;
    if sheathing {
        return Some(until.max(now));
    }
    let name = holster_clip(out, st.wield(unit)).filter(|_| !moving);
    let leave = name.and_then(|n| {
        let at = st.events(unit, n).aw_none.map(f64::from);
        st.play(unit, n, Act::Attack, false, 0.0, now).map(|duration| {
            // the clip goes on long after the sword is away (2.7 s): the unit is free soon after
            let leave = at.unwrap_or(f64::from(duration) * 0.45);
            let free = f64::from(duration).min(leave + HOLSTER_TAIL);
            st.commands.entity(unit).insert(Acting { until: now + free, layered: false });
            leave as f32
        })
    });
    let ready = st.drawn.get_mut(unit).ok().map(|mut drawn| force_sheathe(&mut drawn, now, leave))?;
    if verbose() {
        info!("combat: [{now:.3}] {unit:?} puts its weapon away ({})", name.unwrap_or("no clip"));
    }
    Some(ready)
}

/// A tool skill's caster puts its drawn weapon away at once: no holster clip, no hold of the skill's events. The normal sheathe
/// path ([`begin_sheathe`]) is left to fights that end and to the player's own sheathe.
fn sheathe_for_tool(st: &mut Stage, unit: Entity, skill: u32, now: f64) {
    let Some(visual) = st.book.skill(skill) else { return };
    let Some(drawn) = st.drawn.get(unit).ok() else { return };
    if !tool_skill_sheathes(Some(drawn), &visual.tools, now) {
        return;
    }
    if let Ok(mut drawn) = st.drawn.get_mut(unit) {
        force_sheathe(&mut drawn, now, None);
        if verbose() {
            info!("combat: [{now:.3}] {unit:?} puts its weapon away at once for a tool");
        }
    }
}

/// A weapon that has been out long enough goes back: the holster clip plays (a unit on the move has none), the weapon leaves the hand
/// when the clip says (`aw_none`) and the stance is over.
fn sheathe(time: Res<Time>, mut st: Stage, units: Query<Entity, With<Drawn>>) {
    let now = time.elapsed_secs_f64();
    for unit in &units {
        let Ok((_, _, acting, motion, dead)) = st.actors.get(unit) else { continue };
        let moving = motion.is_some_and(|m| m.speed > 0.3);
        let Ok((sheathing, until)) = st.drawn.get(unit).map(|d| (d.sheathing, d.until)) else { continue };
        if sheathing {
            // running away from the holster clip ends it: the legs have to run
            if moving && now < until && acting.is_some_and(|a| !a.layered) {
                st.commands.entity(unit).remove::<Acting>();
                st.commands.queue(move |w: &mut World| {
                    if let Some(mut l) = w.get_mut::<Locomotion>(unit) {
                        l.invalidate();
                    }
                });
            }
            continue;
        }
        // an action that is still going on keeps the weapon out
        if now < until || acting.is_some() || dead {
            continue;
        }
        begin_sheathe(&mut st, unit, now);
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
            Later::Fx { item, tl, on, toward, once, detach } => {
                let hold = item.start < ev::LAUNCH && item.end > item.start && !once && !item.flat();
                let spawned = st.spawn_item(&item, on, toward, hold);
                if let (Some(e), Some(after)) = (spawned, detach) {
                    combat.after(now + after, Later::Detach(e));
                }
                if let Some(e) = spawned
                    && item.end > item.start
                    && let Some(cast) = combat.casts.get_mut(&tl)
                {
                    cast.held.push((e, item.end));
                }
            }
            Later::ClipFx { unit, effect } => {
                st.spawn_clip_effect(unit, &effect);
            }
            Later::GroundFx { item, tl, at, toward } => {
                let hold = item.start < ev::LAUNCH && item.end > item.start && !item.flat();
                if let Some(e) = st.spawn_ground_item(&item, at, toward, hold)
                    && item.end > item.start
                    && let Some(cast) = combat.casts.get_mut(&tl)
                {
                    cast.held.push((e, item.end));
                }
            }
            Later::Detach(e) => {
                if let Ok(g) = st.xforms.get(e) {
                    let at = g.compute_transform();
                    st.commands.entity(e).try_remove::<ChildOf>().try_insert(at);
                }
            }
            Later::Launch { projectile, tl, caster, target, target_pos, duration, speed, hold } => {
                // a link that lasts as long as the channel ends with it
                if let Some(e) = launch(&mut st, &projectile, caster, target, target_pos, duration, speed)
                    && hold
                    && projectile.is_beam()
                    && let Some(cast) = combat.casts.get_mut(&tl)
                {
                    cast.held.push((e, ev::END));
                }
            }
            Later::Landed(landed) => combat.landed.push(landed),
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
                if let Some(base) = hurt {
                    let clip = st.hit_reaction(target, toward, base);
                    st.play(target, &clip, Act::Hurt, false, 0.0, now);
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
                    // the blow that killed it throws it away from the attacker (the other units; the player stays where it is)
                    if st.remote.contains(e)
                        && let Ok(at) = st.xforms.get(e).map(|g| g.translation())
                    {
                        let from = combat.last_attacker.get(&obj).and_then(|a| st.xforms.get(*a).ok()).map(|g| g.translation());
                        let away = from.map(|f| Vec3::new(at.x - f.x, 0.0, at.z - f.z)).unwrap_or_default();
                        let back = st.xforms.get(e).map(|g| Vec3::new(g.back().x, 0.0, g.back().z)).unwrap_or(Vec3::Z);
                        let dir = if away.length_squared() > 1e-4 { away.normalize() } else { back.normalize_or_zero() };
                        st.commands.entity(e).insert(Thrown { dir, at: now });
                    }
                    if verbose() {
                        info!("combat: unit {obj} died");
                    }
                }
                combat.last_hit.remove(&obj);
                combat.last_attacker.remove(&obj);
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
        fx.ground = Some(Ground(Arc::new(move |p: Vec3| terrain.ground_at(p.x, -p.z).map(|ground| ground.height))));
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
fn launch(st: &mut Stage, projectile: &Projectile, caster: Entity, target: Option<Entity>, target_pos: Option<Vec3>, duration: f32, speed: f32) -> Option<Entity> {
    let beam = projectile.is_beam();
    // the launch bone is the projectile's own source bone, items of bone 0, 8 and 9 included (`src_bone_names`: id 0 is the chest,
    // 25 the left hand of a bow); there is no fallback to the first item's bone
    let src_bone = projectile.src_bone;
    let from = st.source_point(caster, src_bone)?;
    let dest_bone = if projectile.dest_bone != 0 { projectile.dest_bone } else { 19 };
    let to = match target {
        Some(t) => st.bone_point(t, dest_bone, 1.0),
        None => None,
    }
    .or(target_pos.map(|p| p + Vec3::Y));
    // an effect area stands on the ground at the target unit or point, whatever bone it has
    let to = to.or(projectile.is_zone().then_some(from))?;
    // `proj_physic_id` 3 is an effect area (all 257 of them: Meteor Strike's stardust, Sunder Earth's rocks, the magic circles): the
    // effect stands on the ground at the target and does its own falling, spreading and fading
    let (from, to) = if projectile.is_zone() {
        let ground = match target.and_then(|t| st.xforms.get(t).ok()) {
            Some(g) => g.translation(),
            None => target_pos.unwrap_or(to - Vec3::Y),
        };
        (ground, ground)
    } else {
        (from, to)
    };
    let rotation = Transform::from_translation(from).looking_at(to, Vec3::Y).rotation;
    // a plot step's projectile has its speed (m/s) in the data, the flight takes as long as the distance needs
    let duration = if speed > 0.0 && !beam { (from.distance(to) / speed).clamp(0.08, 4.0) } else { duration };
    let anchor = st
        .commands
        .spawn((
            Name::new(format!("projectile {}", projectile.id)),
            Transform { translation: from, rotation, scale: Vec3::ONE },
            Visibility::default(),
            Fx { follow: None, fixed_rotation: true, ttl: duration + if beam { 0.6 } else { 3.0 } },
        ))
        .id();
    if beam {
        // the link: the mesh of its effect is stretched from the launch bone to the target (`update_beams`)
        st.commands.entity(anchor).insert(Beam { caster, src_bone, target: target.map(|t| (t, dest_bone)), to_pos: to });
    } else {
        let arrival: Vec<FxItem> = projectile.items.iter().filter(|i| (i.start == ev::HIT || i.start == ev::END) && !i.effect.is_empty() && !i.is_dot()).cloned().collect();
        st.commands.entity(anchor).insert(Flying { from, to_unit: target.map(|t| (t, dest_bone)), to_pos: to, elapsed: 0.0, duration: duration.max(0.05), arrival: Arc::new(arrival) });
    }
    for item in projectile.items.iter().filter(|i| i.start <= ev::LAUNCH && !i.is_dot()) {
        // the client moves the items of bone 0, 8 and 9 (the origin and the hands: the launch flash) onto the projectile's source
        // bone, the anchor here; an item of another bone stays on its own bone of the caster (`docs/research/re-skills.md` 4.4)
        if !matches!(item.bone, 0 | 8 | 9) {
            st.spawn_item(item, caster, None, false);
            continue;
        }
        // the sounds of the projectile fly with it
        if item.effect.is_empty() {
            if let Some(sfx) = &item.sound {
                let class = if matches!(sfx, Sfx::Voice(_)) { Class::Voice } else { Class::Skill };
                let obj = st.obj_of(caster);
                st.audio.sfx(&mut st.commands, sfx, class, caster, obj, anchor, from);
            }
            continue;
        }
        if muted(&item.effect) {
            continue;
        }
        st.effect_sounds(item, caster, anchor, from);
        // the client never starts a finished effect again: only an effect whose own params are continuous loops (aa-fx round 13)
        let effect = Effect { scale: item.scale, prime: false, item: true, body: Some(caster), despawn_when_finished: false, ..Effect::new(item.effect.clone()) };
        // the effect flies with the anchor: its forward (-Z) is the flight direction, the mesh emitters (the arrow) draw their model
        st.commands.spawn((Name::new(format!("fx {}", item.effect)), effect, Transform::IDENTITY, ChildOf(anchor)));
    }
    if verbose() {
        info!("combat: projectile {} {} {:.2} s over {:.1} m (from {:.2} m above the caster)", projectile.id, if beam { "links" } else if projectile.is_zone() { "stands" } else { "flies" }, duration, from.distance(to), from.y - st.xforms.get(caster).map_or(0.0, |g| g.translation().y));
    }
    Some(anchor)
}

/// An effect that stands in the world at `at` (`toward`: a hit effect turns to the attacker), not on a unit.
fn ground_effect(commands: &mut Commands, item: &FxItem, at: Vec3, toward: Option<Vec3>, hold: bool) -> Entity {
    // horizontal, yaw only (the emitters rise along the up axis of the world, as in the real client): towards the attacker for a hit
    // effect (`offset_axis_id` 4), else along the way the blow went
    let mut rotation = Quat::IDENTITY;
    if let Some(from) = toward {
        let dir = Vec3::new(from.x - at.x, 0.0, from.z - at.z).normalize_or_zero() * if item.axis == 4 { 1.0 } else { -1.0 };
        if dir != Vec3::ZERO {
            rotation = Transform::IDENTITY.looking_to(dir, Vec3::Y).rotation;
        }
    }
    let transform = Transform { translation: at + rotation * cry_to_bevy(item.offset), rotation, scale: Vec3::ONE };
    let effect = Effect { scale: item.scale, prime: false, item: true, despawn_when_finished: !hold, ..Effect::new(item.effect.clone()) };
    if verbose() {
        info!("combat: {} on the ground at {:.1} {:.1} {:.1}", item.effect, at.x, at.y, at.z);
    }
    let ttl = if hold { 12.0 } else { 6.0 };
    let mut e = commands.spawn((Name::new(format!("fx {}", item.effect)), transform, Visibility::default(), Fx { follow: None, fixed_rotation: true, ttl: if item.chr { ttl + 2.0 } else { ttl } }));
    if item.chr {
        e.insert(ChrEffect { scale: item.scale, life: Some(ttl), ..ChrEffect::new(item.effect.clone()) });
    } else {
        e.insert(effect);
    }
    e.id()
}

/// A link between the caster's launch bone and the target (`proj_physic_id` 2).
#[derive(Component)]
struct Beam {
    caster: Entity,
    src_bone: u8,
    target: Option<(Entity, u8)>,
    to_pos: Vec3,
}

/// The first of `names` the rig has, as the entity of that bone (names match case-insensitively, as the client's do).
fn find_bone(animator: &Animator, names: &[&str]) -> Option<Entity> {
    names.iter().find_map(|name| {
        let i = animator.rig.skeleton.bones.iter().position(|b| b.name.eq_ignore_ascii_case(name))?;
        animator.bones.get(i).copied()
    })
}

/// The bone of a projectile's source id: the table of `src_bone_names`; the weapon joints `effect1` / `effect2` (25 to 28) are not in
/// the rigs, the hand that holds the weapon (a bow is in the left one) stands for them.
fn source_bone(animator: &Animator, id: u8) -> Option<Entity> {
    find_bone(animator, src_bone_names(id)).or_else(|| (25..=28).contains(&id).then(|| find_bone(animator, bone_names(8))).flatten())
}

/// The world position of an effect bone of a unit (a projectile's source bone when `source`).
fn bone_world(xforms: &Query<&GlobalTransform>, actors: &Query<&Animator>, unit: Entity, id: u8, source: bool) -> Option<Vec3> {
    let animator = actors.get(unit).ok()?;
    let bone = if source { source_bone(animator, id) } else { find_bone(animator, bone_names(id)) }?;
    xforms.get(bone).ok().map(|g| g.translation())
}

/// Stretches the links between their two ends: the anchor sits on the launch bone, faces the target's bone and is as long as the
/// distance (the effect's mesh is one unit long along its forward axis).
fn update_beams(mut beams: Query<(&mut Transform, &mut Beam)>, xforms: Query<&GlobalTransform>, actors: Query<&Animator>) {
    for (mut transform, mut beam) in &mut beams {
        let Some(from) = bone_world(&xforms, &actors, beam.caster, beam.src_bone, true).or_else(|| xforms.get(beam.caster).ok().map(|g| g.translation() + Vec3::Y)) else { continue };
        if let Some((unit, bone)) = beam.target
            && let Some(point) = bone_world(&xforms, &actors, unit, bone, false).or_else(|| xforms.get(unit).ok().map(|g| g.translation() + Vec3::Y))
        {
            beam.to_pos = point;
        }
        let d = beam.to_pos - from;
        let length = d.length();
        if length < 0.05 {
            continue;
        }
        transform.translation = from;
        transform.rotation = Transform::IDENTITY.looking_to(d / length, Vec3::Y).rotation;
        transform.scale = Vec3::new(1.0, 1.0, length);
    }
}

/// Moves the projectiles to their targets; on arrival they stop and their trail fades.
fn fly_projectiles(
    time: Res<Time>,
    mut commands: Commands,
    mut flying: Query<(Entity, &mut Transform, &mut Flying, &mut Fx)>,
    xforms: Query<&GlobalTransform>,
    actors: Query<&Animator>,
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
    for (entity, mut transform, mut f, mut fx) in &mut flying {
        f.elapsed += dt;
        // homing: the target's hit bone as it moves
        if let Some((unit, bone_id)) = f.to_unit
            && let Some(point) = bone_world(&xforms, &actors, unit, bone_id, false)
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
            // arrived: the emitters stay and die out with the trail, but nothing starts again at the target; what the group plays on
            // arrival (the impact) starts where it landed
            for item in f.arrival.iter() {
                ground_effect(&mut commands, item, f.to_pos, Some(f.from), false);
            }
            commands.entity(entity).remove::<Flying>();
            fx.ttl = fx.ttl.min(0.6);
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

/// Cancels the cast `tl` of the player: `CSStopCasting` (the server answers `CastingStopped` / `PlotStopped`), and the clip and the
/// held effects end at once, without waiting for the answer.
fn cancel_cast(st: &mut Stage, combat: &mut Combat, out: &mut MessageWriter<NetCommand>, tl: u16, plot: bool) {
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
}

/// The casts of the player that `rule` cancels now, as (timeline, plot).
fn casts_to_cancel(st: &Stage, combat: &Combat, rule: impl Fn(&Cancel) -> bool) -> Vec<(u16, bool)> {
    let Some(me) = st.local.iter().next().map(|(_, l)| l.obj_id) else { return Vec::new() };
    combat.casts.iter().filter(|(_, c)| c.caster == me && !c.cancel_sent).filter(|(_, c)| c.cancel.as_ref().is_some_and(&rule)).map(|(tl, c)| (*tl, c.plot)).collect()
}

/// The player moves or jumps during a cast time (or a channel that moving ends), or turns during a cast that turning ends: the cast
/// is cancelled.
fn cancel_on_move(input: Res<PlayerInput>, mut st: Stage, mut combat: ResMut<Combat>, mut out: MessageWriter<NetCommand>) {
    if !(input.moving || input.jumped || input.turned) || no_cancel() {
        return;
    }
    for (tl, plot) in casts_to_cancel(&st, &combat, |r| (r.on_move && (input.moving || input.jumped)) || (r.on_turn && input.turned)) {
        cancel_cast(&mut st, &mut combat, &mut out, tl, plot);
        if verbose() {
            info!("combat: the player moved: cast {tl} cancelled ({}, moving {}, jump {}, turn {})", if plot { "plot" } else { "skill" }, input.moving, input.jumped, input.turned);
        }
    }
}

/// The player presses another skill during a cast time: the server answers `ON_CASTING` to it, so the cast is cancelled first and
/// the skill asked for again a moment later ([`retry_cast`]), the way the real client lets a new skill replace a cast.
fn preempt_cast(time: Res<Time>, mut st: Stage, mut combat: ResMut<Combat>, mut out: MessageWriter<NetCommand>) {
    let Some(Preempt { skill, target }) = combat.preempt.take() else { return };
    let now = time.elapsed_secs_f64();
    // the same skill again waits for the cast it is (the server refuses it meanwhile)
    let casts: Vec<(u16, bool)> = casts_to_cancel(&st, &combat, |r| r.on_move).into_iter().filter(|(tl, _)| combat.casts.get(tl).is_some_and(|c| c.skill != skill)).collect();
    if casts.is_empty() {
        return;
    }
    for (tl, plot) in casts {
        cancel_cast(&mut st, &mut combat, &mut out, tl, plot);
        if verbose() {
            info!("combat: the player asked for skill {skill}: cast {tl} cancelled");
        }
    }
    if combat.retry.is_none() {
        combat.retry = Some(Retry { at: now + 0.25, skill, target });
    }
}

/// The player started to swim in this step (`was` remembers the step before).
fn entered_water(was: &mut bool, swimming: bool) -> bool {
    let entered = swimming && !*was;
    *was = swimming;
    entered
}

/// Entering the water ends the casts of the player (nothing is cast while swimming).
fn cancel_in_water(player: Query<&Mover, With<LocalPlayer>>, mut st: Stage, mut combat: ResMut<Combat>, mut out: MessageWriter<NetCommand>, mut was_swimming: Local<bool>) {
    if !entered_water(&mut was_swimming, player.single().is_ok_and(|m| m.swim.is_some())) || no_cancel() {
        return;
    }
    for (tl, plot) in casts_to_cancel(&st, &combat, |_| true) {
        cancel_cast(&mut st, &mut combat, &mut out, tl, plot);
        if verbose() {
            info!("combat: the player entered the water: cast {tl} cancelled");
        }
    }
}

/// Moves the corpses of [`Thrown`] units: on top of the place the unit sync put them at this frame (it rewrites their transform, and
/// pauses with the clock, so a frame that does not advance adds nothing).
fn throw_corpses(time: Res<Time>, terrain: Option<Res<TerrainQuery>>, water: Option<Res<WaterQuery>>, mut corpses: Query<(&Thrown, &mut Transform)>) {
    if time.delta_secs() <= 0.0 {
        return;
    }
    let now = time.elapsed_secs_f64();
    // the game position (x east, y north, z up) under a Bevy one: the water surface over the ground
    let wet = |p: Vec3| Some((water.as_ref()?.water_at(Vec3::new(p.x, -p.z, p.y))?.0, terrain.as_ref()?.height_at(p.x, -p.z)?));
    for (thrown, mut transform) in &mut corpses {
        transform.translation = thrown_pose(now - thrown.at, transform.translation, thrown.dir, wet);
    }
}

/// Where a unit thrown `age` seconds ago is, from the place `base` its sync puts it at: pushed along `dir`, up over an arc, and (when
/// `wet` says there is water over the ground there: its surface and the ground heights) down on the surface, floating, sinking.
fn thrown_pose(age: f64, base: Vec3, dir: Vec3, wet: impl Fn(Vec3) -> Option<(f32, f32)>) -> Vec3 {
    let age = age.max(0.0);
    let s = (age / THROW_TIME).min(1.0) as f32;
    let mut pos = base + dir * THROW_DISTANCE * (1.0 - (1.0 - s) * (1.0 - s));
    let (rest, floor) = match wet(pos) {
        Some((surface, ground)) if surface > ground + 0.1 => (surface - 0.03, ground.min(pos.y)),
        _ => (pos.y, pos.y),
    };
    let eased = s * s * (3.0 - 2.0 * s);
    pos.y += (rest - pos.y) * eased + THROW_HEIGHT * 4.0 * s * (1.0 - s);
    let sink = ((age - THROW_TIME - FLOAT_TIME) / SINK_TIME).clamp(0.0, 1.0) as f32;
    pos.y -= (rest - floor) * sink * sink * (3.0 - 2.0 * sink);
    pos
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
    use crate::ingame::anim::ClipSet;
    use crate::ingame::tools::{Hand, HeldTool};
    use crate::ingame::weapons::{Grip, HeldWeapon, hold_weapons};
    use aa_anim::Cal;
    use aa_anim::pose::shared_rig;

    const SKELETON: &str = "objects/characters/nuian/male/nude/nu_m_base.chr";
    const CAL: &str = "objects/characters/nuian/male/nude/nu_m_base.cal";

    fn local_tool_start(mut out: MessageWriter<NetEvent>, mut frames: Local<u32>, mut once: Local<bool>) {
        *frames += 1;
        // one frame first, so a drawn weapon has put the character in its combat stance (as it would after a fight)
        if *frames < 2 || *once {
            return;
        }
        *once = true;
        let event = Fake::event("start", &[7, 13789, 5000], 7, 0, None).expect("a SkillStarted event");
        out.write(NetEvent(event));
    }

    /// The local player is not a remote unit: its own client still plays the tool clip and puts the tool on the local model,
    /// with a weapon drawn (put away at once) and without.
    fn assert_local_tool_skill(weapon: Option<Out>) {
        let Some(book) = SkillBook::open() else { return };
        let Ok(rig) = shared_rig(SKELETON) else { return };
        let Ok(cal) = Cal::shared(CAL) else { return };
        let clips = ClipLibrary::default();
        let Some(set) = ClipSet::load(&rig, cal, &clips) else { return };
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(clips)
            .insert_resource(book)
            .insert_resource(Units::default())
            .insert_resource(aa_net::state::Doodads::default())
            .insert_resource(Skills::default())
            .init_resource::<crate::ingame::skill_motion::Inbox>()
            .insert_resource(Combat::default())
            .add_message::<NetEvent>()
            .add_systems(
                Update,
                (local_tool_start, handle_events, end_actions, hold_weapons, crate::ingame::anim::drive_locomotion, crate::ingame::tools::end_skill_tools, crate::ingame::tools::hold_tools)
                    .chain(),
            );
        let bones: Vec<Entity> = rig.skeleton.bones.iter().map(|_| app.world_mut().spawn((Transform::default(), GlobalTransform::default())).id()).collect();
        let local = app
            .world_mut()
            .spawn((
                Animator::new(rig.clone(), bones),
                Locomotion::new(set),
                Motion::default(),
                LocalPlayer { obj_id: 7 },
                Transform::default(),
                GlobalTransform::default(),
                Visibility::default(),
            ))
            .id();
        let sheath = app.world_mut().spawn(()).id();
        let held_weapon = app
            .world_mut()
            .spawn(HeldWeapon {
                owner: local,
                grip: if weapon == Some(Out::Bow) { Grip::Bow } else { Grip::Blade },
                offhand: false,
                sheath,
                hand: Some(local),
                sheath_transform: Transform::IDENTITY,
                hand_transform: Transform::IDENTITY,
                in_hand: true,
                hide_sheathed: false,
                side: None,
                on_side: false,
            })
            .id();
        if let Some(out) = weapon {
            app.world_mut().entity_mut(local).insert(Drawn::after_action(10.0, out));
        }
        // the stance frame, then the tool skill
        app.update();
        app.update();
        let now = app.world().resource::<Time>().elapsed_secs_f64();
        let animator = app.world().get::<Animator>(local).expect("the local animator");
        let current = animator.current.as_ref().expect("a clip plays on the local player");
        assert!(current.clip.source.as_deref().is_some_and(|s| s.contains("fist_ac_shoveling")), "{:?}", current.clip.source);
        let acting = *app.world().get::<Acting>(local).expect("the local player owns the action");
        assert!(acting.until >= now + 5.0, "the action is the 5 s cast, not a holster clip ({acting:?})");
        assert!(app.world().get::<SkillTool>(local).is_some(), "the local player holds the tool");
        if weapon.is_some() {
            let d = app.world().get::<Drawn>(local).expect("the weapon state");
            assert!(d.sheathing, "a drawn weapon is put away");
            assert!(d.until <= now, "the weapon is away at once, with no holster delay ({d:?})");
            assert_eq!(d.stance(now), None, "the weapon is in its sheath while the tool clip plays");
            assert_eq!(
                app.world().get::<ChildOf>(held_weapon).map(ChildOf::parent),
                Some(sheath),
                "the weapon moved to its sheath bone in the same frame the tool appeared"
            );
            assert!(app.world().resource::<Combat>().held.is_empty(), "the tool skill's events are not held");
        }
        // the tool prop is a child of the local player's off hand bone (the shovel of uproot)
        app.update();
        let held: Vec<(Entity, &HeldTool)> = app.world_mut().query::<(Entity, &HeldTool)>().iter(app.world()).collect();
        assert_eq!(held.len(), 1, "one tool anchor on the local player");
        assert_eq!((held[0].1.owner, held[0].1.hand), (local, Hand::Off));
        // the action ends: the clip and the tool go, and the weapon stays sheathed
        app.world_mut().get_mut::<Acting>(local).unwrap().until = 0.0;
        app.update();
        app.update();
        assert!(app.world().get::<Acting>(local).is_none());
        assert!(app.world().get::<SkillTool>(local).is_none());
        assert_eq!(app.world_mut().query::<&HeldTool>().iter(app.world()).count(), 0, "no tool entity is left behind");
        if weapon.is_some() {
            let d = app.world().get::<Drawn>(local).expect("the weapon state stays");
            assert!(d.sheathing && d.stance(8.0).is_none(), "the weapon is not drawn again by the end of the action");
        }
    }

    #[test]
    fn the_local_player_plays_a_tool_skill_with_and_without_a_drawn_weapon() {
        assert_local_tool_skill(None);
        assert_local_tool_skill(Some(Out::Bow));
        // a sword has a holster clip, but a tool skill must not play it
        assert_local_tool_skill(Some(Out::Blade { dual: false }));
    }

    /// A remote entity with the local player's object id (a stale copy) must not take its skill events: remote observers only
    /// have that copy, but the local client's visible rig is the local entity.
    #[test]
    fn the_local_player_wins_over_a_remote_copy_of_its_object_id() {
        let mut world = World::new();
        let local = world.spawn_empty().id();
        let remote = world.spawn_empty().id();
        assert_eq!(unit_entity(7, [(local, 7)].into_iter(), [(remote, 7)].into_iter()), Some(local));
        assert_eq!(unit_entity(8, [(local, 7)].into_iter(), [(remote, 8)].into_iter()), Some(remote));
        assert_eq!(unit_entity(0, [(local, 0)].into_iter(), [(remote, 0)].into_iter()), None);
    }

    #[test]
    fn dash_draws_no_refraction() {
        assert!(no_refraction("abillity_skill_table_m.system_skill.speed_full_pc"));
        assert!(no_refraction("ABILLITY_SKILL_TABLE_M.system_skill.speed_full_pc_end"));
        assert!(!no_refraction("abillity_skill_table_m.magic.ice_piece_launch"));
    }

    /// With the rule, Dash's held effect has one continuous drawn emitter left: the streaks (`speed_full_pc2`); the shells are bursts
    /// (needs the pak).
    #[test]
    fn dash_keeps_only_its_streaks_running_for_the_whole_buff() {
        let name = "abillity_skill_table_m.system_skill.speed_full_pc";
        assert_eq!(once_emitters(name), DASH_ONCE);
        assert!(once_emitters("abillity_skill_table_m.magic.ice_piece_launch").is_empty());
        let Ok(lib) = aa_fx::library::Library::load("abillity_skill_table_m") else {
            eprintln!("no pak: skipped");
            return;
        };
        let effect = Effect { once: once_emitters(name), ..Effect::new(name) };
        let def = lib.get("system_skill.speed_full_pc").expect("Dash's effect");
        let mut endless = Vec::new();
        for fe in def.emitters.iter().filter(|e| e.def.draws() && e.def.continuous) {
            let d = if effect.plays_once(&fe.def.name) { fe.def.first_cycle().unwrap_or_else(|| (*fe.def).clone()) } else { (*fe.def).clone() };
            if d.emitter_life.base <= 0.0 {
                endless.push(d.name.clone());
            }
        }
        assert_eq!(endless, ["speed_full_pc2"], "{endless:?}");
        // both shells are among the ones turned into a first cycle
        for shell in ["speed_full_pc_meshe_high", "speed_full_pc_meshe_medium"] {
            let fe = def.emitters.iter().find(|e| e.def.name == shell).expect(shell);
            assert!(effect.plays_once(shell) && fe.def.first_cycle().is_some_and(|d| d.emitter_life.base == 0.4), "{shell}");
        }
    }

    #[test]
    fn the_water_cancels_once_when_it_is_entered() {
        let mut was = false;
        let steps: Vec<bool> = [false, true, true, false, true].into_iter().map(|swimming| entered_water(&mut was, swimming)).collect();
        assert_eq!(steps, [false, true, false, false, true]);
    }

    #[test]
    fn a_throw_goes_up_lands_and_sinks() {
        let (base, dir) = (Vec3::new(10.0, 126.0, -20.0), Vec3::X);
        // on dry land: up and over, back at the ground 0.7 m away
        let dry = |_: Vec3| None;
        assert_eq!(thrown_pose(0.0, base, dir, dry), base);
        let peak = thrown_pose(THROW_TIME / 2.0, base, dir, dry);
        assert!((peak.y - base.y - THROW_HEIGHT).abs() < 1e-4 && peak.x > base.x + 0.3, "{peak:?}");
        let landed = thrown_pose(THROW_TIME + 5.0, base, dir, dry);
        assert!((landed - (base + dir * THROW_DISTANCE)).length() < 1e-4, "{landed:?}");
        // over a pond 0.6 m deep (surface 126.6 over the ground at 126.0): on the surface, then on the floor
        let pond = |_: Vec3| Some((126.6, 126.0));
        let floating = thrown_pose(THROW_TIME + FLOAT_TIME * 0.5, base, dir, pond);
        assert!((floating.y - 126.57).abs() < 1e-4, "{floating:?}");
        let sunk = thrown_pose(THROW_TIME + FLOAT_TIME + SINK_TIME + 1.0, base, dir, pond);
        assert!((sunk.y - 126.0).abs() < 1e-4, "{sunk:?}");
        // water under the feet that is not deeper than 0.1 m changes nothing
        let wet_feet = |_: Vec3| Some((126.05, 126.0));
        assert!((thrown_pose(THROW_TIME + 5.0, base, dir, wet_feet).y - base.y).abs() < 1e-4);
    }

    /// Retail: a tool skill is worked with the weapon away. Every weapon is put away at once (no holster clip, no delay) and
    /// stays sheathed after the action; a skill without a tool leaves it out. A manual sheathe still uses the clip's moment.
    #[test]
    fn a_tool_skill_takes_the_weapon_away_at_once() {
        let now = 10.0;
        let shovel = skills::Tool { path: "objects/item/weapon/etc/shovel_general/shovel_general.cgf".into(), offset: [-0.01, 0.16, 0.0], rotation: [10.0, 88.88, 0.0] };
        let mut drawn = Drawn::after_action(now + 5.0, Out::Blade { dual: false });
        assert_eq!(drawn.stance(now), Some(Out::Blade { dual: false }), "the weapon is out");
        assert!(tool_skill_sheathes(Some(&drawn), &[None, Some(shovel.clone())], now));
        assert!(!tool_skill_sheathes(Some(&drawn), &[None, None], now), "a non-tool skill does not sheathe");
        // even a sword, which has a holster clip, is away in the same moment
        assert_eq!(force_sheathe(&mut drawn, now, None), now);
        assert!(drawn.sheathing);
        assert_eq!(drawn.stance(now), None, "the weapon is in its sheath when the tool clip starts");
        assert_eq!(drawn.stance(now + 100.0), None, "and stays sheathed after the action");
        // a unit without a weapon is not touched
        let mut no_weapon = Drawn::after_action(now + 5.0, Out::Blade { dual: false });
        assert!(!tool_skill_sheathes(None, &[Some(shovel), None], now));
        assert_eq!(force_sheathe(&mut no_weapon, now, None), now);
        assert_eq!(no_weapon.stance(now), None, "the weapon is away at once");
        // the manual sheathe is unchanged: it takes the holster clip's `aw_none`
        let mut manual = Drawn::after_action(now + 5.0, Out::Blade { dual: false });
        assert!((force_sheathe(&mut manual, now, Some(0.428)) - (now + 0.428)).abs() < 1e-6);
        assert_eq!(manual.stance(now + 0.1), None, "sheathing, the stance is over");
        assert_eq!(manual.stance(now + 0.5), None, "the weapon is in its sheath after the clip's moment");
    }

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

    /// Which way the head moves in the player's hit reactions (`--ignored --nocapture`): the sign tells which side `_l` / `_r` are for.
    #[test]
    #[ignore]
    fn hit_reaction_directions() {
        use aa_anim::pose::SampleOptions;
        let Ok(rig) = aa_anim::pose::shared_rig("objects/characters/nuian/male/nude/nu_m_base.chr") else { return };
        let cal = aa_anim::Cal::shared("objects/characters/nuian/male/nude/nu_m_base.cal").unwrap();
        let clips = ClipLibrary::default();
        let head = rig.skeleton.bones.iter().position(|b| b.name == "Bip01 Head").unwrap();
        let rest = rig.bind.clone().model_space(&rig)[head].1;
        for name in ["all_re_combat_hit", "all_re_combat_hit_l", "all_re_combat_hit_r", "twohand_re_combat_hit_l", "twohand_re_combat_hit_r", "all_re_combat_critical", "all_re_relaxed_hit"] {
            let Some(clip) = bind_action_clip(&rig, &cal, &clips, name) else { println!("{name}: missing"); continue };
            let mut worst = Vec3::ZERO;
            for i in 0..=20 {
                let mut pose = rig.bind.clone();
                pose.sample(&rig, &clip.clip, &clip.binding, clip.clip.duration * i as f32 / 20.0, SampleOptions { lock_root_motion: true, ..clip.options });
                let d = pose.model_space(&rig)[head].1 - rest;
                if d.length() > worst.length() { worst = d; }
            }
            println!("{name:26} {:.2} s head moves x {:+.2} y {:+.2} z {:+.2} (cry: x right, y forward)", clip.clip.duration, worst.x, worst.y, worst.z);
        }
    }
}
