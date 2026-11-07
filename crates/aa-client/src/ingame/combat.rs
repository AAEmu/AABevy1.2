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
mod clouts;
mod framehold;
mod clips;
mod flash;
mod graph;
mod preload;
pub(crate) mod skills;
mod sweep;

use std::collections::HashMap;
use std::sync::Arc;

use aa_anim::{Animator, ClipLibrary};
use aa_fx::{ChrEffect, Effect, FxSettings, Ground};
use aa_net::buff::BuffCaster;
use aa_net::skill::{CastAction, PlotEvent, SkillFired, SkillStarted, SkillTarget, UnitDamage, hit_type, target_kind};
use aa_net::model::UnitKind;
use aa_net::state::{Player, Skills, Units};
use aa_net::{Command, Event, MoveCommand, NetCommand, NetEvent, Pos};
use aa_world::sky::SunLight;
use aa_world::{TerrainQuery, WaterQuery, cry_to_bevy};
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use super::anim::{Locomotion, Motion, RunningAction, bind_action_clip, bind_clip, clip_file};
use super::player::{LocalPlayer, Mover, PlayerInput, approach_angle};
use super::units::{RemoteDoodad, RemoteUnit};
use super::camera::CameraShake;
use super::sounds::{AnimSound, Audio, Class, LayerSounds};
use super::tools::SkillTool;
use super::weapons::{Drawn, Out, StaffSide, Wield, holster_clip, hold_weapons, weapon_for_clip};
use crate::Screen;
pub(super) use skills::Sfx;

/// The blend of the idle or run that follows an action whose graph exit link is `exit` (see `graph`).
pub(super) fn idle_blend(exit: f32) -> f32 {
    graph::ActionGraph::idle_blend(exit)
}
use skills::{CancelRules, FxItem, Projectile, ShakeItem, SkillBook, bone_names, ev, src_bone_names};

/// Eligible movement/yaw cast stops have been submitted before a seat exit.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct InputCancellation;

/// What a unit is busy with, so locomotion does not take its clip away.
#[derive(Component, Clone, Copy, Debug)]
pub struct Acting {
    /// When it is over (seconds on the app clock).
    pub until: f64,
    /// The clip plays on the upper body (an animator layer) while locomotion keeps the legs: the unit was moving.
    pub layered: bool,
}

/// The client's battle flag (`X2Unit + 0x3bba`): set by `SCCombatEngaged` (`OnCombatEngaged`, `FUN_391d69a0` -> `FUN_390b36f0`)
/// and by a real blow dealt or taken (the damage handler's `FUN_39391670`: a caster, a hit, both units alive), and cleared only
/// by `SCCombatCleared` (`OnCombatCleared`, `FUN_391d6a60` -> the unit's vtable `+0x48` = `FUN_39355ec0` -> `FUN_390b3710`). It
/// has no timer of its own.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct CombatStance;

/// Seconds the model keeps its combat idle after the last combat activity: `FUN_39355ee0` compares the current time with
/// `+0x5fc0` against `DAT_39ac7424` (10000 ms, read at `x2game_unpacked.dll` file offset `0xad5a24`).
const COMBAT_IDLE_SECS: f64 = 10.0;
/// The skill-use pulse of `FUN_39355ee0`: `+0x5fc5` refreshes `+0x5fc0` for 1000 ms after the local player's auto attack
/// request (the `+0x5fc6` branch for another skill is unreachable in 1.2; see `request_skill`).
const SKILL_PULSE_SECS: f64 = 1.0;

/// The model's "recent combat" flag (`X2Unit + 0x5fc4`, derived from the `+0x5fc0` deadline in `FUN_39355ee0`): it drives the
/// combat idle (`[model + 0x14] + 0x428`). It is refreshed every update while [`CombatStance`] is set (the deadline never clears
/// the stance byte), for 1000 ms after the local player's auto attack request (`+0x5fc5`), and when a skill is used while it is
/// already set (`FUN_393545e0`). It falls 10 s after the last refresh; `FUN_39355ee0` then writes `false` to the model
/// and zeroes `+0x5fc0`. Putting the weapon away zeroes `+0x5fc0` too (`FUN_39360920` with weapon 0, `FUN_39362690` on the
/// resurrect reset, `FUN_39362c70`), so the idle ends with the weapon; the stance byte is separate.
#[derive(Component, Clone, Copy, Debug)]
pub struct CombatIdle {
    /// `+0x5fc0`: the last activity time the 10 s is counted from.
    pub last: f64,
    /// `+0x5fc5`: the auto attack request refreshes `last` until this moment.
    pub pulse_until: f64,
}

impl CombatIdle {
    pub fn at(now: f64) -> CombatIdle {
        CombatIdle { last: now, pulse_until: 0.0 }
    }

    pub fn from_skill(now: f64) -> CombatIdle {
        CombatIdle { last: now, pulse_until: now + SKILL_PULSE_SECS }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    /// The looping cast clip.
    Cast,
    /// A fire / attack clip.
    Attack,
    /// A hit reaction (never interrupts the others).
    Hurt,
    /// A plot step's clip of a `match_animation` skill that would loop: it plays once and holds its last frame until the cast's
    /// next clip (see [`skills::SkillVisual::match_animation`]).
    Held,
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
pub(crate) struct Fx {
    /// The unit and the bone entity a fixed orientation effect follows, with the offset in the unit's frame (the skill trace
    /// names the bone).
    pub(crate) follow: Option<(Entity, Entity, Vec3)>,
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
    /// An effect area (`proj_physic_id` 3): it stands where it lands, and its effects play out there ([`Settling`]).
    zone: bool,
}

/// An effect area that reached its end (`fx_event_end_id` 5): its continuous effects stop emitting, and the anchor goes once
/// every effect has finished. The bursts play out: the 1.2 client keeps Sunder Earth's rocks (`ground_launch_standalone_stone1`,
/// a 3 s particle) for their whole life and crumbles them at their `SpawnDelay` of 2.9 s, about 2.7 s after the zone ended
/// (skill parity trace t1j, 2026-11-03); aa-client cut the anchor 0.6 s after the end and the rocks went with it. Seconds since
/// the end.
#[derive(Component, Default)]
struct Settling(f32);

/// How long an effect area may settle at most, a safety net for an effect that never finishes (out of range, or one whose
/// library is missing): the longest burst of the player skills' areas, Sunder Earth's crumble, ends 7.1 s after the zone.
const ZONE_SETTLE_SECS: f32 = 10.0;

/// The least an effect area lives after its end, for the sounds that ride its anchor (the 0.6 s the anchor had before).
const ZONE_SETTLE_MIN_SECS: f32 = 0.6;

/// A damage packet at the moment its blow lands (a swing at the clip's sync point, a shot after its flight, a plot step as it
/// comes): what the HUD's floating numbers and combat log show, as in the real client. `received` is the time the combat code took
/// the packet up, not the time the blow lands.
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
    /// The skill the player drew its weapon for, to put it back when the server refuses that skill: a notice only, no delay
    /// (the 1.2 client changes the weapon at the request (`FUN_39360920` from `FUN_39390f80`) and plays the clip on the server's
    /// `SCSkillFired`).
    draw: Option<Draw>,
}

/// The player asked for a weapon skill with the weapon away.
#[derive(Clone, Copy)]
struct Draw {
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
    /// The unit the cast was started on (plot unit 2): `target` follows the steps, this stays.
    plot_target: u32,
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
    /// The last clip this cast started, when, and what [`Stage::play`] said: two steps of one packet batch that name the same clip
    /// start it once (Charge's start step falls back to the skill's controller clip and its launch step's controller has the same
    /// `all_co_sk_dashattack`: one start in the original, two in the same frame here; skill parity trace t1f, 2026-11-03).
    last_clip: Option<(Arc<str>, f64, Option<f32>)>,
    /// What the player's input cancels now (a cast of the local player whose skill is flagged for it).
    cancel: Option<Cancel>,
    /// The cancel went to the server (once per cast).
    cancel_sent: bool,
    /// The skill is a plot: the server names its timeline in `plot_tl_id`.
    plot: bool,
    /// The caster's anim rules turned the fire clip down (an auto attack during a skill clip): its blow shows nothing.
    rejected: bool,
    /// The bound effect groups (`FxGroupAnim`) the plot's steps made so far: a clip that a later step plays on a group's source
    /// fires the events of that clip the group has not had (x2game binds every bound group of the unit when it plays the clip,
    /// `3938BF20`). Lasso's `당기기_dot`, made by the cast step, waits for the launch of the next step's `all_co_sk_pull_launch`:
    /// +1153 ms in the original, 28 ms after that clip (skill parity trace, 2026-11-03).
    bound: Vec<BoundGroup>,
}

/// A bound effect group of a plot, where it was made, and the fx events it has had.
#[derive(Clone)]
struct BoundGroup {
    group: skills::StepFx,
    source: Entity,
    aim: Option<Entity>,
    ground: Option<Vec3>,
    /// A bit per fx event id it has had: an event starts its items once.
    fired: u16,
}

/// What cancels a running cast: moving (or jumping), turning, the water.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Cancel {
    on_move: bool,
    on_turn: bool,
    /// `source_not_swim`: the skill cannot be used swimming (36 skills of the data, none of the player's trees), so a cast that is
    /// running when the caster enters the water ends. The other skills are cast swimming as on land.
    in_water: bool,
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
    let in_water = rules.not_swim;
    (on_move || on_turn || in_water).then_some(Cancel { on_move, on_turn, in_water })
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
    Hit { caster: Option<Entity>, target: Entity, items: Arc<Vec<FxItem>>, shakes: Arc<Vec<ShakeItem>>, hurt: Option<&'static str>, sound: Option<UnitDamage> },
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
            Some(pos) => SkillTarget::position(pos),
            None => SkillTarget::unit(other),
        };
        let raw_caster = aa_net::SkillCasterData::Unit(me);
        let raw_target = aa_net::SkillTargetData::Unit(other);
        let tl = arg(0) as u16;
        Some(match op {
            "start" => Event::SkillStarted(SkillStarted { skill_id: arg(1), tl, caster, target, cast_ms: arg(2), base_cast_ms: arg(2), synergy: 0, result: 0, result_value: 0, raw_result: Default::default(), raw_caster, raw_target }),
            "fire" => Event::SkillFired(SkillFired { skill_id: arg(1), tl, caster, target, delay_ms: arg(2) as i32, channeling_ms: arg(3) as i32, fire_anim_id: arg(4), raw_final_flag: 0, result: Default::default(), raw_caster, raw_target }),
            "plot" => Event::PlotEvent(PlotEvent { tl, event_id: arg(1), skill_id: arg(2), caster_obj_id: me, target_obj_id: if ground.is_some() { 0 } else { other }, caster_pos: None, target_pos: ground, casting_ms: arg(3), targets: if ground.is_some() || other == 0 { Vec::new() } else { vec![other] } }),
            "hit" => Event::UnitDamaged(UnitDamage {
                action: CastAction { kind: 0, skill_id: arg(1), tl, buff_id: 0 },
                caster,
                caster_obj_id: me,
                target_obj_id: other,
                damage: arg(2) as i32,
                absorbed: 0,
                mana_burn: 0,
                hit_type: 0,
                holdable_id: 0,
                raw_hit: 0x0120,
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
        .init_resource::<clouts::CloutFx>()
        .init_resource::<framehold::Frameholds>()
        .init_resource::<preload::Preload>()
        .init_resource::<CameraShake>()
        .init_resource::<PlayerInput>()
        .init_resource::<super::skill_motion::Inbox>()
        .add_plugins(flash::plugin)
        .add_systems(OnExit(Screen::InGame), reset)
        .add_systems(Update, framehold::observe.after(super::world_switch::clear_old_units).before(super::units::receive_moves))
        .add_systems(Update, framehold::sync.after(framehold::observe)
            .after(super::units::cull_units).after(super::gear_refresh::refresh)
            .after(super::player::spawn_player).before(request_skill).before(handle_events)
            .before(super::anim::drive_locomotion).before(super::skill_motion::drive_local)
            .before(super::skill_motion::drive_remote))
        .add_systems(Update, buffs::ensure_visuals.after(framehold::sync).before(handle_events))
        .add_systems(Update, framehold::sync_after_receipts.after(handle_events)
            .after(framehold::sync).before(ensure_corpses).before(super::anim::drive_locomotion)
            .before(super::skill_motion::drive_local).before(super::skill_motion::drive_remote))
        .add_systems(
            Update,
            (
                (request_skill, handle_events, clouts::doodad_clouts, tick_combat_idle, cancel_on_move.after(super::player::control_player).in_set(InputCancellation), cancel_in_water, preempt_cast, retry_cast, run_timers, flush_landed, mark_corpses, ensure_corpses, buffs::reconcile_buffs, end_actions, sheathe, release_layers, fly_projectiles, settle_zones, update_beams, hold_weapons)
                    .chain()
                    .before(super::anim::drive_locomotion)
                    // the key press of this frame (the HUD's `UseSkill`): the 1.2 client starts the combat idle within the frame of
                    // the press, where reading it before the HUD wrote it started it a frame later (skill parity trace t1i, 14-21 ms)
                    .after(super::ground_target::ItemUseRouting)
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
    player: Option<Res<'w, Player>>,
    net_units: Res<'w, Units>,
    /// A skill aimed at a doodad (a Memory Tome ...) is aimed at its position (object ids are shared by units and doodads).
    net_doodads: Res<'w, aa_net::state::Doodads>,
    skills: Res<'w, Skills>,
    wield: Query<'w, 's, &'static Wield>,
    drawn: Query<'w, 's, &'static mut Drawn>,
    remote: Query<'w, 's, (Entity, &'static RemoteUnit)>,
    /// The doodads that cast (a ship's cannon: its skill's effects start on the doodad).
    doodads: Query<'w, 's, (Entity, &'static RemoteDoodad)>,
    local: Query<'w, 's, (Entity, &'static LocalPlayer)>,
    #[allow(clippy::type_complexity)]
    actors: Query<'w, 's, (&'static mut Animator, Option<&'static mut Locomotion>, Option<&'static Acting>, Option<&'static Motion>, Has<Dead>)>,
    /// The units seated on a mount: `mount::seat` takes their locomotion (and with it the model's `.cal`) away while they ride.
    riders: Query<'w, 's, &'static aa_anim::Character, With<super::mount::Rider>>,
    asleep: Query<'w, 's, &'static super::units::Asleep>,
    dead: Query<'w, 's, (), With<Dead>>,
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

    /// The entity of the caster of a skill event: a unit, else a doodad (the cannon of a ship casts its skill: the effects and the
    /// projectile start on the doodad, which has no rig: its origin stands for every bone).
    fn caster_entity(&self, obj: u32) -> Option<Entity> {
        self.entity(obj).or_else(|| (obj != 0).then(|| self.doodads.iter().find(|(_, d)| d.obj_id == obj).map(|(e, _)| e)).flatten())
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
                .and_then(|u| super::units::unit_world_pose_in(u, &self.net_units))
                .map(|(pos, _)| cry_to_bevy(pos))
                .or_else(|| {
                    self.net_doodads
                        .by_id
                        .get(&obj)
                        .and_then(|d| super::units::doodad_world(d, &self.net_units))
                        .map(cry_to_bevy)
                }),
        }
    }

    /// The bone entity of an effect bone id (`None` for the origin and for rigs without such a bone).
    fn bone(&self, unit: Entity, id: u8) -> Option<Entity> {
        if let Ok((animator, ..)) = self.actors.get(unit) {
            find_bone(animator, bone_names(id))
        } else {
            find_bone(self.asleep.get(unit).ok()?.animator(), bone_names(id))
        }
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

    /// The unit sits on a mount: `mount::seat` took its locomotion, and its base clip is the rider loop of `mount::Rider`, which
    /// only `Rider::step` may change (its actions are upper body layers, see [`Stage::play`]).
    fn seated(&self, unit: Entity) -> bool {
        matches!(self.actors.get(unit), Ok((_, None, ..))) && self.riders.contains(unit)
    }

    /// The unit's animation name table: its locomotion's, else (a seated rider has none) its character's, as `Rider::bind` reads it.
    fn cal(&self, unit: Entity) -> Option<Arc<aa_anim::Cal>> {
        match self.actors.get(unit).ok()? {
            (_, Some(loco), ..) => Some(loco.clips.cal.clone()),
            _ => aa_anim::Cal::shared(&self.riders.get(unit).ok()?.assets.cal).ok(),
        }
    }

    /// The model's clip `name` bound to the unit's animator and its length.
    fn clip(&self, unit: Entity, name: &str) -> Option<(aa_anim::BoundClip, Arc<aa_anim::Cal>)> {
        let (animator, ..) = self.actors.get(unit).ok()?;
        let cal = self.cal(unit)?;
        let clip = bind_action_clip(&animator.rig, &cal, &self.clips, name)?;
        Some((clip, cal))
    }

    /// The upper body variant of clip `name` for the unit's model: the `move_ub` clip of its `anims` row (`bow_co_sk_launch_ub`),
    /// which the `.cal` of most models names `..._mub`, else as given. `None` when the row or the model has none.
    fn upper_clip(&self, unit: Entity, name: &str) -> Option<aa_anim::BoundClip> {
        let upper = self.book.upper_clip(name)?;
        skills::upper_candidates(&upper).find_map(|n| self.clip(unit, &n).map(|(clip, _)| clip))
    }

    /// The same for a seated rider: the `ride_ub` clip of the row (`onehand_co_sk_shielddefense_ub`, the `.cal`'s `_mub`).
    fn ride_clip(&self, unit: Entity, name: &str) -> Option<aa_anim::BoundClip> {
        let ride = self.book.ride_clip(name)?;
        skills::upper_candidates(&ride).find_map(|n| self.clip(unit, &n).map(|(clip, _)| clip))
    }

    /// The anim events of clip `name` of the unit's model, in seconds into the clip (the databases hold phases of it).
    fn events(&self, unit: Entity, name: &str) -> clips::ClipEvents {
        let Some((clip, cal)) = self.clip(unit, name) else { return clips::ClipEvents::default() };
        clips::ClipEvents::of(&cal, name, clip.clip.duration)
    }

    /// Plays a clip on a unit as an action. `hold` is how long a looping clip may last. Returns the clip's length, `None` when the
    /// unit cannot (no such clip, dead, busy with something a hit reaction must not interrupt).
    fn play(&mut self, unit: Entity, name: &str, kind: Act, looping: bool, hold: f64, now: f64) -> Option<f32> {
        // a unit on the move plays the clip on its upper body and keeps running below it; a rider seated on a mount always does
        // (its legs are the rider loop of `mount::Rider`, which keeps playing under the layer)
        let (mask, exit, riding) = {
            let (animator, loco, acting, motion, dead) = self.actors.get(unit).ok()?;
            let riding = self.seated(unit);
            // a hit reaction never interrupts an action or a run (a rider's run is the mount's)
            let running = !riding && motion.is_some_and(|m| m.speed > 0.3);
            if dead || (loco.is_none() && !riding) || (kind == Act::Hurt && (acting.is_some() || running)) {
                if verbose() {
                    info!("combat: {unit:?} does not play {name}: dead {dead}, locomotion {}, acting {}", loco.is_some(), acting.is_some());
                }
                return None;
            }
            let moving = kind != Act::Hurt && !full_body_only() && running;
            // the exit link of the action the whole body is still in (0 from an idle or a run)
            let exit = acting.filter(|a| !a.layered && a.until > now).and(loco.as_ref().and_then(|l| l.action_exit)).unwrap_or(0.0);
            let mask = match loco {
                Some(loco) => (moving && loco.clips.can_move()).then(|| loco.clips.upper.clone()).flatten(),
                // `player_upper_body.xml` has the riding states; a rig without an upper body plays nothing
                None => Some(aa_anim::BoneMask::upper_body(&animator.rig)?),
            };
            (mask, exit, riding)
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
        if !self.actors.get(unit).ok()?.0.starts_enabled() { return None; }
        if self.rejects(unit, name, now) {
            if verbose() {
                info!("combat: [{now:.3}] {unit:?} turns {name} down: a skill clip plays (anim_rules)");
            }
            return None;
        }
        let layered = mask.is_some();
        // a rider's variant is `ride_ub`; without one the clip itself plays on the upper body (Mounted Attack's
        // `onehand_co_horse_attack_r_slash` is a state of the upper body graph only)
        let clip = if riding {
            self.ride_clip(unit, name)
        } else if layered {
            self.upper_clip(unit, name)
        } else {
            None
        };
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
        let (mut animator, mut loco, ..) = self.actors.get_mut(unit).ok()?;
        if let Some(mask) = mask {
            animator.play_layer(mask, clip, looping, graph::UPPER_BODY_BLEND);
        } else {
            // the blend the 1.2 animation graph walks to the clip's state (see `graph`); 0.08 for a clip it does not know
            let graph = graph::ActionGraph::get();
            animator.stop_layer(0.0);
            animator.play(clip, looping, graph.action_blend(exit, name).unwrap_or(0.08));
            animator.speed = 1.0;
            animator.random_idle = None;
            if let Some(loco) = loco.as_mut() {
                loco.action_exit = graph.knows(name).then(|| graph.exit(name));
            }
        }
        if let Some(mut loco) = loco {
            loco.running = (!looping).then(|| self.book.clip_category(name)).flatten().map(|category| RunningAction { category, started: now, length: f64::from(duration) });
        }
        // a hit reaction is a flinch: the start of its clip, then locomotion fades back in
        let until = now + if looping || kind == Act::Held { hold } else if kind == Act::Hurt { f64::from(duration).min(0.5) } else { f64::from(duration).min(4.0) };
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

    /// x2game's `anim_rules` (`39153EF0`, `39157680`) turn clip `name` down on the unit: the one-shot it plays answers op 5 to the
    /// new clip's category. A skill clip (4) rejects an auto attack (1), a hit (8) and a reaction (3); an attack rejects a hit in its
    /// first 30 %. The rejected clip plays nothing, and nothing of its anim events (skill parity trace, 2026-11-03: the auto attack
    /// the server fires at +2253 during Triple Slash's finisher shows no clip, swing, voice, trail or blow; the finisher plays to its
    /// end). The loops (cast 6, crowd control 11) are x2game's other list, which aa-client does not keep apart: not judged here.
    fn rejects(&self, unit: Entity, name: &str, now: f64) -> bool {
        let Ok((_, Some(loco), Some(_), ..)) = self.actors.get(unit) else { return false };
        let Some(run) = loco.running.filter(|r| now < r.started + r.length && !matches!(r.category, 6 | 11)) else { return false };
        let Some(new) = self.book.clip_category(name) else { return false };
        let progress = ((now - run.started) / run.length.max(1e-3) * 100.0) as f32;
        self.book.anim_rule(run.category, new, progress) == 5
    }

    /// Starts one item of an effect group on a unit: a particle effect (with the sound it carries), or a sound (a voice is the unit's
    /// own). `toward` is the other party (a hit effect turns to it). Returns the entity that plays it.
    fn spawn_item(&mut self, item: &FxItem, on: Entity, toward: Option<Vec3>, hold: bool) -> Option<Entity> {
        if let Some(flash) = item.flash {
            // a material flash is not an entity of its own: the unit's meshes are copied for its fade time
            self.commands.entity(on).try_insert(flash::FlashRequest(flash));
            return None;
        }
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
        let cal = self.cal(unit)?;
        let database = cal.sound_events.as_deref()?;
        let events = self.audio.book()?.clip_events(database, &clip_file(clip)?, clip.clip.duration)?;
        (!events.is_empty()).then_some(events)
    }

    /// The sound of a weapon hitting a unit, at its hit bone.
    fn blow_sound(&mut self, caster: Option<Entity>, target: Entity, impact: &UnitDamage) {
        let (Some(caster), Some(target_obj)) = (caster, self.obj_of(target)) else { return };
        let Some(attacker) = self.obj_of(caster) else { return };
        let parent = self.bone(target, 19).unwrap_or(target);
        let Some(at) = self.xforms.get(parent).ok().map(|g| g.translation()) else { return };
        self.audio.blow(&mut self.commands, impact, attacker, (target, target_obj), parent, at);
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
        // a song's sound is the one of the instrument the caster holds
        let sound = item.sound_for(self.wield(unit).instrument);
        let own = match &sound {
            Some(Sfx::Event(p)) => Some(p.clone()),
            _ => None,
        };
        if let Some(sfx) = sound.as_ref() {
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
            // no bone (id 0, or one this rig lacks): at the unit's origin. The training doll has no `bip01 head`, and the 1.2
            // client puts Sunder Earth's `ground_dot` (bone 2) on its origin, where `ground_hit` (bone 0) is too (skill parity
            // trace t1i, 2026-11-03); aa-client had it a metre up
            None => unit_xf.translation(),
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
        self.casts.entry(tl).or_insert_with(|| Cast { skill, caster, target, plot_target: target, started: now, ..Cast::default() })
    }
}

/// When after the clip starts the items of each early moment begin: `START` at once, `FORM` and `THROW` at the clip's own
/// `form` / `throw` anim events (0 when the clip has none). The original fires them there: Battle Focus' `attack_speedup_cast`
/// (THROW) at +253 ms into `all_co_sk_shout` (throw phase 0.255 of 0.98 s), Frenzy's `reckless_cast` (THROW) at +259 ms, where
/// aa-client started both with the skill (skill parity trace, 2026-11-03).
fn early_moments(events: &clips::ClipEvents) -> [(u8, f64); 3] {
    [(ev::START, 0.0), (ev::FORM, f64::from(events.form.unwrap_or(0.0))), (ev::THROW, f64::from(events.throw.unwrap_or(0.0)))]
}

/// The units a plot step's effects can name ([`skills::plot_unit`]).
struct StepUnits {
    /// 1: the plot's caster, 2: the unit the plot was started on (when drawn).
    plot_caster: Option<Entity>,
    plot_target: Option<Entity>,
    /// 3: the step's source, the packet's caster.
    source: Entity,
    /// 4: the units of the packet's list that are drawn.
    listed: Vec<Entity>,
}

impl StepUnits {
    fn of(&self, id: u8, unit: Option<Entity>) -> Option<Entity> {
        match id {
            skills::plot_unit::CASTER => self.plot_caster,
            skills::plot_unit::PLOT_TARGET => self.plot_target,
            skills::plot_unit::SOURCE => Some(self.source),
            skills::plot_unit::EACH => unit,
            // 5 is a spot: the caller takes the step's target position
            skills::plot_unit::SPOT => None,
            _ => None,
        }
    }
}

/// When a group's events fire, in seconds after the step: `(event, at, skip_short)` for [`start_items`].
///
/// A bound group (`FxGroupAnim`) whose source plays the step's clip gets `start` at once and `form`, `throw`, `launch` and
/// `voice_launch` at those anim events of the clip, and only the ones the clip has: x2game fires an fx event from an anim event of
/// the bound clip only (`3938C040`), so a clip without one never starts the items that wait for it (Sunder Earth's
/// `twohand_co_sk_weapon_blunt_3_cast` has no events: its `대지가르기_launch` never shows during the cast, where aa-client started
/// it at once). A bound group on another unit has no clip: `start` only.
///
/// A free group (`FxGroup`) fires `start` and the one event of its `value2` at once (`3938DF10`): Charge's hit (`dash_hit` on the
/// struck unit, event 4) as the step comes. A step with a projectile also sends `launch` to its groups at the release (the
/// projectile's own launch list, `3938BA40`).
fn group_moments(group: &skills::StepFx, clip: Option<&clips::ClipEvents>, launch: Option<f64>) -> Vec<(u8, f64, bool)> {
    let mut out = vec![(ev::START, 0.0, true)];
    if group.bound {
        if let Some(events) = clip {
            for (moment, at, short) in [(ev::FORM, events.form, true), (ev::THROW, events.throw, true), (ev::LAUNCH, events.launch, false), (ev::VOICE, events.voice, false)] {
                if let Some(at) = at {
                    out.push((moment, f64::from(at), short));
                }
            }
        }
    } else if group.fire != ev::START {
        out.push((group.fire, 0.0, false));
    }
    if let Some(at) = launch
        && !out.iter().any(|(m, ..)| *m == ev::LAUNCH)
    {
        out.push((ev::LAUNCH, at, false));
    }
    out
}

/// Starts the effect groups of a plot step where their plot source and target say (x2game `ClientPlotMan::PlayEvent`,
/// `3938F100`): a group naming unit 4 plays once per unit of the packet's list, none when the list is empty (Whirlwind Slash's
/// `회오리베기` swirl and Sunder Earth's launch group with the self-centred steps, which list nobody), on the listed unit when it
/// is the source (Lasso's `염동력_dot` on the pulled target). Items with `on_target` go on the group's target, the rest on its
/// source. Returns what waits for the damage: the hit items of the bound groups that were made (of every group made, when the
/// step has a projectile), which play when the blow lands (`clip_unit` is the unit that plays the step's clip).
#[allow(clippy::too_many_arguments)]
fn start_step_groups(
    st: &mut Stage,
    combat: &mut Combat,
    tl: u16,
    groups: &[skills::StepFx],
    units: &StepUnits,
    clip_unit: Option<Entity>,
    clip_events: &clips::ClipEvents,
    projectile_launch: Option<f64>,
    target_pos: Option<Vec3>,
    now: f64,
) -> (Vec<FxItem>, Vec<ShakeItem>) {
    // the bound groups of earlier steps on the unit that plays this step's clip: the clip's events they have not had
    if let Some(unit) = clip_unit {
        let earlier: Vec<(usize, BoundGroup)> =
            combat.casts.get(&tl).map(|c| c.bound.iter().cloned().enumerate().filter(|(_, b)| b.source == unit).collect()).unwrap_or_default();
        for (index, mut bound) in earlier {
            if bound.fired & (1 << ev::FORM) == 0 {
                start_swings(st, combat, tl, &bound.group.fx, bound.source, bound.aim, now, clip_events);
            }
            for (moment, at, short) in group_moments(&bound.group, Some(clip_events), None) {
                if bound.fired & (1 << moment) != 0 {
                    continue;
                }
                bound.fired |= 1 << moment;
                start_items(st, combat, tl, &bound.group.fx, &[moment], bound.source, bound.aim, bound.ground, now, at, short, true);
                start_shakes(st, combat, &bound.group.shakes, &[moment], bound.source, bound.aim, now, at);
            }
            if let Some(slot) = combat.casts.get_mut(&tl).and_then(|c| c.bound.get_mut(index)) {
                slot.fired = bound.fired;
            }
        }
    }
    let mut hit_fx: Vec<FxItem> = Vec::new();
    let mut hit_shakes: Vec<ShakeItem> = Vec::new();
    for group in groups {
        let each: Vec<Option<Entity>> = if group.per_unit() { units.listed.iter().copied().map(Some).collect() } else { vec![None] };
        let mut made = false;
        for unit in each {
            let Some(source) = units.of(group.source, unit) else { continue };
            let aim = units.of(group.target, unit);
            // the step's spot (unit 5), or the spot of a ground target when the target unit is not drawn
            let ground = if aim.is_none() { target_pos } else { None };
            made = true;
            let clip = (group.bound && clip_unit == Some(source)).then_some(clip_events);
            if clip.is_some() {
                start_swings(st, combat, tl, &group.fx, source, aim, now, clip_events);
            }
            let mut fired = 0u16;
            for (moment, at, short) in group_moments(group, clip, projectile_launch) {
                fired |= 1 << moment;
                start_items(st, combat, tl, &group.fx, &[moment], source, aim, ground, now, at, short, true);
                start_shakes(st, combat, &group.shakes, &[moment], source, aim, now, at);
            }
            if group.bound
                && let Some(cast) = combat.casts.get_mut(&tl)
            {
                cast.bound.push(BoundGroup { group: group.clone(), source, aim, ground, fired });
            }
        }
        if made && (group.bound || projectile_launch.is_some()) {
            hit_fx.extend(group.fx.iter().cloned());
            hit_shakes.extend(group.shakes.iter().copied());
        }
    }
    (hit_fx, hit_shakes)
}

/// The end of the plot is the `END` event of its bound groups (once each): the items that start there play (Lasso's `full_dot_end`
/// as the channel ends, +17 ms after the 1.2 client's cast bar stops; aa-client never showed it, skill parity trace t1h). Called after
/// [`Combat::release`], which takes away what ends at `END`, so these items stay.
fn end_bound_groups(st: &mut Stage, combat: &mut Combat, tl: u16, now: f64) {
    let open: Vec<(usize, BoundGroup)> =
        combat.casts.get(&tl).map(|c| c.bound.iter().cloned().enumerate().filter(|(_, b)| b.fired & (1 << ev::END) == 0).collect()).unwrap_or_default();
    for (index, bound) in open {
        start_items(st, combat, tl, &bound.group.fx, &[ev::END], bound.source, bound.aim, bound.ground, now, 0.0, false, true);
        start_shakes(st, combat, &bound.group.shakes, &[ev::END], bound.source, bound.aim, now, 0.0);
        if let Some(slot) = combat.casts.get_mut(&tl).and_then(|c| c.bound.get_mut(index)) {
            slot.fired |= 1 << ev::END;
        }
    }
}

/// Starts the items of `moments` of a cast now, or `delay` seconds from now. `dots`: the items of a plot step's group play even when
/// they look like a buff's (`_dot`): Lasso's `염동력_dot` (psychokinesis) on the pulled unit and its `당기기_dot` are plot effects
/// (its buff 3687 has no group), and the original shows both (skill parity trace, 2026-11-03).
#[allow(clippy::too_many_arguments)]
fn start_items(st: &mut Stage, combat: &mut Combat, tl: u16, items: &[FxItem], moments: &[u8], caster: Entity, target: Option<Entity>, ground: Option<Vec3>, now: f64, delay: f64, skip_short: bool, dots: bool) {
    let toward_caster = st.xforms.get(caster).ok().map(|g| g.translation());
    let toward_target = target.and_then(|t| st.xforms.get(t).ok()).map(|g| g.translation());
    for item in items.iter().filter(|i| moments.contains(&i.start)) {
        // buff visuals (damage over time, auras) belong to the buff, not to the skill's own group
        if !dots && item.is_dot() {
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
    let looping = kind == Act::Cast || (kind != Act::Held && st.book.clip_loops(name));
    let held = kind == Act::Held;
    if let Some(played) = repeated_start(combat.casts.get(&tl).and_then(|c| c.last_clip.as_ref()), name, now) {
        return played;
    }
    let played = st.play(caster, name, kind, looping, hold, now);
    if let Some(cast) = combat.casts.get_mut(&tl) {
        cast.last_clip = Some((Arc::from(name), now, played));
    }
    if played.is_some() && name.starts_with("all_") {
        // a spell puts the staff back on the back (the real client, after a weapon skill had sent it to the hip)
        st.commands.entity(caster).try_insert(StaffSide(false));
    }
    if played.is_some()
        && let Some(cast) = combat.casts.get_mut(&tl)
    {
        cast.casting = looping || held;
        cast.played = true;
        combat.owner.insert(cast.caster, tl);
    }
    if played.is_some() && !looping {
        clip_effects(st, combat, caster, name, now);
    }
    played
}

/// What the cast's start of `name` at this same instant gave, when it has started that clip at `now` already ([`Cast::last_clip`]);
/// a later start of the same clip (Tiger Strike's second dash) plays it again.
fn repeated_start(last: Option<&(Arc<str>, f64, Option<f32>)>, name: &str, now: f64) -> Option<Option<f32>> {
    last.filter(|(n, at, _)| &**n == name && *at == now).map(|&(_, _, played)| played)
}

/// The effects the clip starts itself (`effect` anim events: the flowers of `/flowers`, the tears of `/sad`), at their times.
fn clip_effects(st: &Stage, combat: &mut Combat, unit: Entity, name: &str, now: f64) {
    let Some((clip, cal)) = st.clip(unit, name) else { return };
    for effect in clips::clip_effects(&cal, name, clip.clip.duration).iter() {
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
#[allow(clippy::too_many_arguments)]
fn handle_events(
    mut events: MessageReader<NetEvent>,
    time: Res<Time>,
    mut st: Stage,
    mut combat: ResMut<Combat>,
    mut clock: ResMut<Time<Virtual>>,
    mut shot: Option<ResMut<DevShot>>,
    mut frameholds: Option<ResMut<framehold::Frameholds>>,
    mut buff_visuals: Query<&mut buffs::BuffVisuals>,
) {
    let now = time.elapsed_secs_f64();
    for NetEvent(event) in events.read() {
        if let Some(holds) = frameholds.as_deref_mut() {
            let change = holds.before_receipt(event, &mut st);
            buffs::at_receipt(change, holds, &mut st, &mut buff_visuals, now);
        }
        // retail: a tool skill puts the drawn weapon away at once, with no holster clip and no delay
        if let Some((caster, skill)) = tool_event(event)
            && let Some(unit) = st.entity(caster)
        {
            sheathe_for_tool(&mut st, unit, skill, now);
        }
        apply(event, now, &mut st, &mut combat, &mut clock, &mut shot);
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

/// The `+0x5fc0` deadline of `FUN_39355ee0`: refreshed every update while the battle flag is set (`+0x3bba`) and for the 1000 ms
/// after a skill request (`+0x5fc5` / `+0x5fc6`). Ten seconds after the last refresh the derived flag `+0x5fc4` falls and the
/// model leaves its combat idle; the stance byte is not touched. A dead unit has no idle (the reset path zeroes `+0x5fc0`).
#[allow(clippy::type_complexity)]
fn tick_combat_idle(time: Res<Time>, mut commands: Commands, mut units: Query<(Entity, Has<CombatStance>, Option<&mut CombatIdle>, Has<Dead>), Or<(With<CombatStance>, With<CombatIdle>)>>) {
    let now = time.elapsed_secs_f64();
    for (unit, stance, idle, dead) in &mut units {
        if dead {
            if idle.is_some() {
                commands.entity(unit).remove::<CombatIdle>();
            }
            continue;
        }
        if stance {
            match idle {
                Some(mut idle) => idle.last = now,
                None => {
                    commands.entity(unit).insert(CombatIdle::at(now));
                }
            }
            continue;
        }
        let Some(mut idle) = idle else { continue };
        if now < idle.pulse_until {
            idle.last = now;
        }
        if now - idle.last > COMBAT_IDLE_SECS {
            commands.entity(unit).remove::<CombatIdle>();
        }
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
            if own
                && s.result != aa_net::skill::result::OUTOF_ANGLE
                && combat.draw.is_some_and(|d| d.skill == s.skill_id && d.caster == s.caster.obj_id)
            {
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
            let Some(caster) = st.caster_entity(s.caster.obj_id) else { return };
            // `SCSkillStarted` refreshes no combat idle: the 1000 ms `+0x5fc5` / `+0x5fc6` pulses are on the request path
            // (`FUN_39390f80` from `FUN_393989e0`'s skill 2 branch and the click attack, the local player only; see
            // `request_skill`). It does **not** set the battle flag either: that comes only from `SCCombatEngaged` and a
            // blow (`FUN_39391670`).
            if combat.draw.is_some_and(|d| d.caster == s.caster.obj_id) {
                combat.draw = None;
            }
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
                    start_items(st, combat, s.tl, &look.fx, &[ev::START, ev::FORM], caster, target, target_pos, now, 0.0, false, false);
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
            let Some(caster) = st.caster_entity(f.caster.obj_id) else { return };
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
            // a clip the caster's anim rules turn down (an auto attack while a skill clip plays) shows nothing of the attack
            if name.as_ref().is_some_and(|n| st.rejects(caster, n, now)) {
                let cast = combat.casts.get_mut(&f.tl).unwrap();
                cast.rejected = true;
                cast.hit_fx = Arc::default();
                cast.hit_shakes = Arc::default();
                if verbose() {
                    info!("combat: [{now:.3}] skill {} fires during a skill clip: its clip, effects and blow are turned down", f.skill_id);
                }
                return;
            }
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
                let [(_, at_start), (_, at_form), (_, at_throw)] = early_moments(&clip_events);
                if fresh {
                    start_items(st, combat, f.tl, &look.fx, &[ev::START], caster, target, target_pos, now, at_start, true, false);
                    start_items(st, combat, f.tl, &look.fx, &[ev::FORM], caster, target, target_pos, now, at_form, true, false);
                    start_shakes(&st, combat, &look.shakes, &[ev::START], caster, target, now, at_start);
                    start_shakes(&st, combat, &look.shakes, &[ev::FORM], caster, target, now, at_form);
                }
                start_swings(st, combat, f.tl, &look.fx, caster, target, now, &clip_events);
                start_items(st, combat, f.tl, &look.fx, &[ev::THROW], caster, target, target_pos, now, at_throw, false, false);
                start_items(st, combat, f.tl, &look.fx, &[ev::LAUNCH], caster, target, target_pos, now, release, false, false);
                // the voice line at the moment the clip calls it
                start_items(st, combat, f.tl, &look.fx, &[ev::VOICE], caster, target, target_pos, now, if voice > 0.0 { voice } else { release }, false, false);
                start_shakes(&st, combat, &look.shakes, &[ev::THROW], caster, target, now, at_throw);
                start_shakes(&st, combat, &look.shakes, &[ev::LAUNCH], caster, target, now, release);
                if let Some(projectile) = &look.projectile {
                    start_shakes(&st, combat, &projectile.shakes, &[ev::START, ev::FORM, ev::THROW, ev::LAUNCH], caster, target, now, release);
                    let flight = (f64::from(f.delay_ms.max(0)) / 1000.0 - release).max(0.06);
                    combat.after(now + release, Later::Launch { projectile: projectile.clone(), tl: f.tl, caster, target, target_pos, duration: if projectile.is_beam() && f.channeling_ms > 0 { f.channeling_ms as f32 / 1000.0 } else { flight as f32 }, speed: 0.0, hold: f.channeling_ms > 0 });
                }
                // the doodad the channel spawns (Protective Wings: doodad 879 at the caster's feet) shows its clouts' effect area
                // from the server's `SCDoodadCreated` (`clouts`), like every doodad with clouts
            }
        }
        Event::PlotEvent(p) => {
            let Some(caster) = st.caster_entity(p.caster_obj_id) else { return };
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
            let step = st.book.plot_event(p.event_id);
            // a cast step without a clip of its own shows the skill's start clip; one with its own (Sunder Earth's
            // `twohand_co_sk_weapon_blunt_3_cast`, Lasso's `all_co_sk_pull_cast` and its launch step) plays only that: the original
            // starts one clip per step (skill parity trace, 2026-11-03), where both made two or three starts of the same clip
            if p.casting_ms > 0
                && step.as_ref().is_none_or(|s| s.clip().is_none())
                && let Some(v) = visual.as_ref()
                && let Some(name) = v.instrument_start_clip(wield.instrument).or(v.start_anim.as_ref()).cloned()
                && play_clip(st, combat, p.tl, caster, &name, Act::Cast, f64::from(p.casting_ms) / 1000.0 + 1.0, now).is_some()
            {
                st.set_tools(caster, v);
            }
            // `all_co_sk_stop`: ends the clip that loops before this step (alone, it is the whole step)
            if step.as_ref().is_some_and(|s| s.stops()) {
                end_loop(st, combat, p.tl);
            }
            // the clip of the step; a skill that moves its caster (a dash) has only its controller's clip until a step has one
            let idle = !combat.casts.get(&p.tl).is_some_and(|c| c.played);
            let clip = step.as_ref().and_then(|s| s.clip().cloned()).or_else(|| if idle { visual.as_ref().and_then(|v| v.controller_anim.clone()) } else { None });
            let mut release = 0.0;
            let mut clip_events = clips::ClipEvents::default();
            if let Some(name) = &clip {
                let events = st.events(caster, name);
                clip_events = events;
                release = f64::from(events.release().unwrap_or(0.0));
                let hold = if p.casting_ms > 0 { f64::from(p.casting_ms) / 1000.0 + 1.0 } else { 20.0 };
                let held = visual.as_ref().is_some_and(|v| v.match_animation) && st.book.clip_loops(name);
                if play_clip(st, combat, p.tl, caster, name, if held { Act::Held } else { Act::Attack }, hold, now).is_some()
                    && let Some(v) = &visual
                {
                    st.set_tools(caster, v);
                }
                combat.casts.get_mut(&p.tl).unwrap().sync = f64::from(events.combat_sync.unwrap_or(0.0));
            }
            let Some(step) = step else { return };
            face(st, caster, target, target_pos, now);
            // only an anim effect's clip drives the bound groups (not a dash's controller clip, not the skill's own); a step
            // without groups of its own still drives the earlier ones: Lasso's launch step (`all_co_sk_pull_launch`, a clip and a
            // projectile) fires the `full_dot` of the cast step's group at the clip's launch, +31 ms in the original, where
            // aa-client showed nothing (skill parity trace t1h, 2026-11-03)
            let clip_unit = (step.clip().is_some() && !step.controller).then_some(caster);
            let bound_waiting = clip_unit.is_some() && combat.casts.get(&p.tl).is_some_and(|c| !c.bound.is_empty());
            if !step.groups.is_empty() || bound_waiting {
                let (plot_caster, plot_target) = combat.casts.get(&p.tl).map(|c| (c.caster, c.plot_target)).unwrap_or((p.caster_obj_id, p.target_obj_id));
                let units = StepUnits {
                    plot_caster: st.caster_entity(plot_caster),
                    plot_target: st.entity(plot_target),
                    source: caster,
                    listed: p.targets.iter().filter_map(|&id| st.entity(id)).collect(),
                };
                let (hit_fx, hit_shakes) = start_step_groups(st, combat, p.tl, &step.groups, &units, clip_unit, &clip_events, step.projectile.is_some().then_some(release), target_pos, now);
                if !step.groups.is_empty() {
                    let cast = combat.casts.get_mut(&p.tl).unwrap();
                    cast.hit_fx = Arc::new(hit_fx);
                    cast.hit_shakes = Arc::new(hit_shakes);
                }
            }
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
            let now_landed = DamageLanded { damage: d.clone(), received: now };
            let target = st.entity(d.target_obj_id);
            let caster = st.entity(d.caster_obj_id);
            let missed = hit_type::is_miss(d.hit_type) || d.damage <= 0;
            // A real blow puts both sides in the battle stance (`FUN_39391670` from the damage handler), and so keeps the
            // combat idle alive (the next update refreshes its deadline from the stance byte). It needs a caster unit, a hit
            // (not a miss or a zero), and both units alive: the decompile tests `+0x50 != 0`, `+0x6e != 0` and neither unit's
            // `+0x3c26` / `+0x3c27` dead flags. The damage handler calls it after every kind (749000-749128), so buff,
            // passive and doodad damage flags too, not only a skill's blow.
            let dead = |id: u32, entity: Option<Entity>| st.net_units.get(id).is_some_and(|u| u.dead) || entity.is_some_and(|e| st.dead.contains(e));
            if !missed && d.caster_obj_id != 0 && d.target_obj_id != 0 && caster.is_some() && !dead(d.target_obj_id, target) && !dead(d.caster_obj_id, caster) {
                if let Some(target) = target {
                    st.commands.entity(target).insert((CombatStance, CombatIdle::at(now)));
                }
                if let Some(attacker) = caster {
                    st.commands.entity(attacker).insert((CombatStance, CombatIdle::at(now)));
                }
                // The third `FUN_390b36f0` of the damage handler: a blow on the local player's own mate flags the owner
                // too. The decompile tests the TARGET (`+0x410 == 5` mate, `+0x420` owner character id equal to the local
                // character id), looks the owner up by character id (`FUN_3935e370`) and needs it alive (`+0x3c26` /
                // `+0x3c27` zero); the caster is not tested, so a blow the mate deals flags the mate and its victim only.
                let own = st
                    .player
                    .as_ref()
                    .and_then(|p| p.spawn.as_ref())
                    .map(|spawn| (spawn.character().id, spawn.obj_id))
                    .filter(|(char_id, _)| *char_id != 0);
                if let Some((char_id, obj_id)) = own
                    && st.net_units.get(d.target_obj_id).is_some_and(|u| matches!(&u.kind, UnitKind::Mate { owner_id, .. } if *owner_id == char_id))
                    && let Some(owner) = st.local.iter().find(|(_, l)| l.obj_id == obj_id).map(|(e, _)| e)
                    && !dead(obj_id, Some(owner))
                {
                    st.commands.entity(owner).insert((CombatStance, CombatIdle::at(now)));
                }
            }
            // buff ticks (kind 2) have no blow to show, and a unit that is not drawn has no blow to wait for: the numbers go on at once
            if d.action.kind > 1 {
                combat.landed.push(now_landed);
                return;
            }
            let Some(target) = target else {
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
            // Native weapon audio requires a known cast. Preserve the received attack
            // holdable / packed hit until impact; hurt FX and audio admission are separate.
            let sound = cast.filter(|c| !c.rejected).map(|_| d.clone());
            let hurt = (!missed).then_some(if hit_type::is_critical(d.hit_type) { "all_re_combat_critical" } else { "all_re_combat_hit" });
            if let Some(attacker) = caster.filter(|c| *c != target) {
                combat.last_attacker.insert(d.target_obj_id, attacker);
            }
            let last = combat.last_hit.entry(d.target_obj_id).or_default();
            *last = last.max(at);
            combat.after(at, Later::Landed(now_landed));
            combat.after(at, Later::Hit { caster, target, items: if missed { Arc::default() } else { items }, shakes: if missed { Arc::default() } else { shakes }, hurt, sound });
        }
        // `SCCombatEngaged` / `SCCombatCleared` (`OnCombatEngaged` sets `+0x3bba`; `OnCombatCleared` clears it at once through
        // `FUN_39355ec0`). The clear ends the flag only; it touches neither the combat idle (`+0x5fc4`, `FUN_39355ee0`) nor its
        // `+0x5fc0` deadline, which is why the model stays in `onehand_combat_idle` after the K2 capture's clear. A clear on a
        // unit that is not flagged (and has no idle) changes nothing.
        Event::CombatChanged { obj_id, engaged } => {
            if let Some(unit) = st.entity(*obj_id) {
                if *engaged {
                    st.commands.entity(unit).insert((CombatStance, CombatIdle::at(now)));
                } else {
                    st.commands.entity(unit).remove::<CombatStance>();
                }
                if verbose() {
                    info!("combat: [{now:.3}] the battle stance of {unit:?} is {engaged}");
                }
            }
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
                // `OnCharacterResurrected` resets the unit (`FUN_39362690`): weapon away and `+0x5fc0` zeroed, so the combat
                // idle is over; the stance byte is left to `SCCombatCleared`
                st.commands.entity(e).remove::<(Dead, Corpse, Acting, Thrown, CombatIdle)>();
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
            end_bound_groups(st, combat, tl, now);
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
    clips
        .into_iter()
        .flatten()
        .find_map(|c| weapon_for_clip(&c))
        .or_else(|| {
            if !visual.plot {
                return None;
            }
            book.plot_steps(id).into_iter().find_map(|step| book.plot_event(step).and_then(|p| p.clip().and_then(|c| weapon_for_clip(c))))
        })
        .or_else(|| weapon_of_skill(visual.active_weapon, wield))
}

/// The weapon a skill whose clips name none (`all_co_sk_dashattack`) takes out: its `skills.active_weapon_id`. The 1.2 client draws
/// the greatsword for Charge and Tiger Strike (1) and stands in `twohand_ba_combat_idle`, where an `all_` clip alone left aa-client
/// in the fist stance (skill parity trace t1h, 2026-11-03).
fn weapon_of_skill(active_weapon: u8, wield: Wield) -> Option<Out> {
    match active_weapon {
        1 => wield.melee.then_some(Out::Blade { dual: wield.dual }),
        _ => None,
    }
}

/// The player presses a skill that needs its weapon while it is away: Bevy draws the weapon and the shield now and turns the
/// character to the target. The decompile draws at the request only for the auto attack: `FUN_39390f80` (called for skill 2 only)
/// -> `FUN_39360920`, with the 1000 ms `+0x5fc5` pulse (`FUN_39354610`). Both of its callers pass skill 2 (`FUN_393989e0`'s
/// `param_2 == DAT_39ac769c` branch, 748866-748885, and `FUN_393923c0`, 743728, called with `DAT_39ac3874` = 2 at 140928), so
/// its `+0x158` / `+0x5fc6` branch for another skill is unreachable in 1.2. For every other skill the request-time draw is
/// **inferred**: the decompile's start-skill path draws only when `skills.can_active_weapon_without_anim` (`+0x38`) is set, when
/// the server's `SCSkillStarted` arrives (`FUN_3939a810`, 750400-750401). The clip is not timed here: it plays when the server's
/// `SCSkillFired` arrives.
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
        if st.actors.get(me).is_ok_and(|a| a.4 || !a.0.starts_enabled()) {
            continue;
        }
        // `FUN_39390f80` is the local player's request path, and both callers pass skill 2 (`DAT_39ac769c`): it draws the
        // weapon and sets `+0x5fc5` (`FUN_39354610`). Its `+0x158` / `+0x5fc6` branch for another skill is unreachable in 1.2
        // (the click attack `FUN_393923c0` passes `DAT_39ac3874` = 2 too). If the derived idle is already set
        // (`FUN_39354600`), the deadline is only refreshed (`FUN_393545e0`) and no pulse starts. Every skill that turns the auto
        // attack on (`skills.start_autoattack`) takes that path too, whatever it aims at: from the relaxed stance the 1.2 client
        // starts the combat idle at the key press of Triple Slash, Charge, Precision Strike and Lasso (another unit) and of
        // Whirlwind Slash, Sunder Earth and Frenzy (itself), before the server answers; the four skills without the flag
        // (Terrifying Roar, Bondbreaker, Battle Focus, Tiger Strike) do not (skill parity trace t1h, 2026-11-03).
        if *skill_id == MELEE_ATTACK || st.book.skill(*skill_id).is_some_and(|v| v.starts_autoattack) {
            st.commands.queue(move |w: &mut World| {
                if let Some(mut idle) = w.get_mut::<CombatIdle>(me) {
                    idle.last = now;
                } else if let Ok(mut unit) = w.get_entity_mut(me) {
                    unit.insert(CombatIdle::from_skill(now));
                }
            });
        }
        let Some(out) = skill_weapon(&st.book, *skill_id, st.wield(me)) else { continue };
        if st.drawn.get(me).is_ok_and(|d| d.stance(now) == Some(out)) {
            continue;
        }
        // the weapon is drawn at once; the server's events of this skill are not held back (the clip and the damage come when
        // `SCSkillFired` / `SCUnitDamaged` arrive)
        st.commands.entity(me).insert(Drawn::after_action(now, out));
        if let Some(target) = st.entity(*target_obj_id).filter(|t| *t != me) {
            st.commands.entity(me).insert(FaceTarget { entity: Some(target), pos: None, until: now + 0.8 });
        }
        combat.draw = Some(Draw { skill: *skill_id, caster: obj });
        if verbose() {
            info!("combat: [{now:.3}] the weapon comes out for skill {skill_id}");
        }
    }
}

/// The skill did not start: the weapon it drew goes back.
fn cancel_draw(st: &mut Stage, combat: &mut Combat, now: f64) {
    combat.draw = None;
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
    if !st.actors.get(unit).ok()?.0.starts_enabled() { return None; }
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
    // `FUN_39360920` zeroes `+0x5fc0` when the weapon state becomes 0: putting the weapon away ends the combat idle
    st.commands.entity(unit).remove::<CombatIdle>();
    if verbose() {
        info!("combat: [{now:.3}] {unit:?} puts its weapon away ({})", name.unwrap_or("no clip"));
    }
    Some(ready)
}

/// A tool skill's caster puts its drawn weapon away at once: no holster clip, no hold of the skill's events. The normal sheathe
/// path ([`begin_sheathe`]) is left to fights that end and to the player's own sheathe.
fn sheathe_for_tool(st: &mut Stage, unit: Entity, skill: u32, now: f64) {
    if !st.actors.get(unit).is_ok_and(|actor| actor.0.starts_enabled()) { return; }
    let Some(visual) = st.book.skill(skill) else { return };
    let Some(drawn) = st.drawn.get(unit).ok() else { return };
    if !tool_skill_sheathes(Some(drawn), &visual.tools, now) {
        return;
    }
    if let Ok(mut drawn) = st.drawn.get_mut(unit) {
        force_sheathe(&mut drawn, now, None);
        // the weapon away zeroes `+0x5fc0` (`FUN_39360920`), so the combat idle ends with it
        st.commands.entity(unit).remove::<CombatIdle>();
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
            Later::Hit { caster, target, items, shakes, hurt, sound } => {
                let toward = caster.and_then(|c| st.xforms.get(c).ok()).map(|g| g.translation());
                if let Some(impact) = sound {
                    st.blow_sound(caster, target, &impact);
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
                    // a dead unit has no combat idle (`FUN_39362690` / `FUN_39362c70` zero `+0x5fc0`); the stance byte is not
                    // cleared here, the server's `SCCombatCleared` owns it
                    st.commands.entity(e).remove::<(Acting, FaceTarget, CombatIdle)>().insert(Dead { at: now });
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

/// What the sun adds to a lit particle at noon, facing it (`FxSettings::sun_light`). The engine's own figure is 7.6 times the picture's
/// exposure of about 1.6 (12, `aa_fx::sim` `LIT_SUN`); fitted on Summon Wraith's red cloud against the real client in daylight (median
/// red 150, `shots/aa-fx/r16/`) it takes 30: the game's sun and tone mapping are not the real ones.
const FX_SUN_LIGHT: f32 = 30.0;

/// Tells aa-fx where the sun is (the shading of mesh particles) and how high the ground is (particles that bounce, effects that start
/// where a particle lands). The sun is the world's directional light; the ground is the terrain under a world position.
fn feed_fx(terrain: Res<TerrainQuery>, sun: Query<(&GlobalTransform, &DirectionalLight), With<SunLight>>, fx: Option<ResMut<FxSettings>>, mut wired: Local<bool>) {
    let Some(mut fx) = fx else { return };
    if !*wired {
        *wired = true;
        let terrain = terrain.clone();
        // Bevy position -> game position (x east, y north = -z); the height is Bevy's y
        fx.ground = Some(Ground(Arc::new(move |p: Vec3| terrain.ground_at(p.x, -p.z).map(|ground| ground.height))));
    }
    if let Ok((light, sun_light)) = sun.single() {
        // the sun's own light on the lit sprites follows its strength (the world's noon: 1000 x 7.6 lux is `FX_SUN_LIGHT`)
        // (`AA_FX_SUN_LIGHT` sets it by hand)
        let strength = FX_SUN_LIGHT * sun_light.illuminance / 7600.0;
        if (fx.sun_light - strength).abs() > 0.01 && std::env::var_os("AA_FX_SUN_LIGHT").is_none() {
            if (fx.sun_light - strength).abs() > 0.1 * strength.max(1.0) && verbose() {
                info!("combat: the sun lights particles with {strength:.1} ({} lux)", sun_light.illuminance);
            }
            fx.sun_light = strength;
        }
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
        st.commands.entity(anchor).insert(Flying { from, to_unit: target.map(|t| (t, dest_bone)), to_pos: to, elapsed: 0.0, duration: duration.max(0.05), arrival: Arc::new(arrival), zone: projectile.is_zone() });
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
            if f.zone {
                commands.entity(entity).insert(Settling::default());
                fx.ttl = ZONE_SETTLE_SECS;
            } else {
                fx.ttl = fx.ttl.min(0.6);
            }
        }
    }
}

/// Lets the effect areas that reached their end play out ([`Settling`]): stops their continuous effects and removes the anchor
/// when nothing of it is left to show.
fn settle_zones(time: Res<Time>, mut commands: Commands, mut zones: Query<(Entity, &mut Settling, Option<&Children>)>, mut runs: Query<&mut aa_fx::FxRun>) {
    let dt = time.delta_secs();
    for (anchor, mut settling, children) in &mut zones {
        settling.0 += dt;
        let mut live = false;
        for child in children.into_iter().flat_map(|c| c.iter()) {
            let Ok(mut run) = runs.get_mut(child) else { continue };
            if run.is_missing() || run.is_finished() {
                continue;
            }
            // the end event stops what keeps emitting; a burst still to come (a `SpawnDelay`) comes
            if run.one_shot() == Some(false) && !run.is_stopped() {
                run.stop_emitting();
            }
            live = true;
        }
        if !live && settling.0 >= ZONE_SETTLE_MIN_SECS {
            commands.entity(anchor).try_despawn();
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
    let mut casts: Vec<_> = combat.casts.iter().filter(|(_, c)| c.caster == me && !c.cancel_sent).filter(|(_, c)| c.cancel.as_ref().is_some_and(&rule)).map(|(tl, c)| (*tl, c.plot)).collect();
    casts.sort_by_key(|(tl, plot)| (*plot, *tl));
    casts
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

/// Entering the water ends the casts of the player whose skill cannot be used swimming (`source_not_swim`; the server refuses those
/// when they are asked for in the water, `CANNOT_CAST_IN_SWIMMING`).
fn cancel_in_water(player: Query<&Mover, With<LocalPlayer>>, mut st: Stage, mut combat: ResMut<Combat>, mut out: MessageWriter<NetCommand>, mut was_swimming: Local<bool>) {
    if !entered_water(&mut was_swimming, player.single().is_ok_and(|m| m.swim.is_some())) || no_cancel() {
        return;
    }
    for (tl, plot) in casts_to_cancel(&st, &combat, |r| r.in_water) {
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
        if !animator.starts_enabled() { continue; }
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
pub(super) fn end_actions(time: Res<Time>, mut commands: Commands, mut acting: Query<(Entity, &Acting, Option<&mut Locomotion>)>) {
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
pub(super) fn fixture_cancel_casts(app: &mut App, actor: u32) {
    let mut combat = Combat::default();
    for (tl, plot) in [(11, false), (12, true)] {
        combat.casts.insert(tl, Cast { caster: actor, plot, cancel: Some(Cancel { on_move: true, on_turn: true, in_water: false }), ..default() });
    }
    app.insert_resource(combat).insert_resource(SkillBook::open().unwrap())
        .init_resource::<Skills>().init_resource::<super::skill_motion::Inbox>()
        .add_systems(Update, cancel_on_move.after(super::player::control_player).in_set(InputCancellation));
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

    fn group(bound: bool, fire: u8) -> skills::StepFx {
        skills::StepFx { bound, source: skills::plot_unit::CASTER, target: skills::plot_unit::EACH, fire, fx: Arc::default(), shakes: Arc::default() }
    }

    /// A bound group gets `start` at once and only the anim events its clip has (seconds into the clip); without a clip, `start`
    /// only. A free group fires `start` and its one event at once. A projectile's release sends `launch` to both.
    #[test]
    fn group_moments_follow_the_clip_or_the_free_event() {
        let triple = clips::ClipEvents { form: Some(0.24), throw: Some(0.73), launch: Some(0.30), combat_sync: Some(0.30), voice: Some(0.19), aw_none: None };
        let moments = |g: &skills::StepFx, clip: Option<&clips::ClipEvents>, launch| -> Vec<(u8, f64)> {
            group_moments(g, clip, launch).into_iter().map(|(m, at, _)| (m, (at * 100.0).round() / 100.0)).collect()
        };
        assert_eq!(moments(&group(true, 0), Some(&triple), None), [(ev::START, 0.0), (ev::FORM, 0.24), (ev::THROW, 0.73), (ev::LAUNCH, 0.3), (ev::VOICE, 0.19)]);
        // Sunder Earth's cast clip has no events: its launch group shows only its start items
        assert_eq!(moments(&group(true, 0), Some(&clips::ClipEvents::default()), None), [(ev::START, 0.0)]);
        assert_eq!(moments(&group(true, 0), None, None), [(ev::START, 0.0)]);
        // Charge's blow: hit at once
        assert_eq!(moments(&group(false, ev::HIT), Some(&triple), None), [(ev::START, 0.0), (ev::HIT, 0.0)]);
        assert_eq!(moments(&group(false, 0), None, Some(0.4)), [(ev::START, 0.0), (ev::LAUNCH, 0.4)]);
        assert_eq!(moments(&group(true, 0), Some(&triple), Some(0.4)).iter().filter(|(m, _)| *m == ev::LAUNCH).count(), 1, "one launch");
    }

    /// Charge and Tiger Strike have only `all_` clips: their `active_weapon_id` 1 takes the greatsword out; Lasso (0) takes none.
    #[test]
    fn a_skill_whose_clips_name_no_weapon_takes_its_active_weapon() {
        let Some(book) = SkillBook::open() else { return };
        let greatsword = Wield { two_handed: true, melee: true, ..Default::default() };
        assert_eq!(skill_weapon(&book, 11918, greatsword), Some(Out::Blade { dual: false }), "Charge");
        assert_eq!(skill_weapon(&book, 13315, greatsword), Some(Out::Blade { dual: false }), "Tiger Strike");
        assert_eq!(skill_weapon(&book, 12039, greatsword), None, "Lasso");
        assert_eq!(skill_weapon(&book, 11918, Wield::default()), None, "no weapon to take out");
        assert_eq!(weapon_of_skill(1, Wield { melee: true, dual: true, ..Default::default() }), Some(Out::Blade { dual: true }));
        assert_eq!(weapon_of_skill(3, greatsword), None, "3 leaves the weapon as it is");
    }

    #[test]
    fn a_clip_two_steps_start_in_one_instant_starts_once() {
        let last = (Arc::<str>::from("all_co_sk_dashattack"), 4.25, Some(0.8));
        assert_eq!(repeated_start(Some(&last), "all_co_sk_dashattack", 4.25), Some(Some(0.8)), "Charge's start and launch steps");
        assert_eq!(repeated_start(Some(&last), "all_co_sk_dashattack", 4.6), None, "a later step plays it again (Tiger Strike's dashes)");
        assert_eq!(repeated_start(Some(&last), "all_co_sk_dashattack_2", 4.25), None, "another clip starts");
        assert_eq!(repeated_start(None, "all_co_sk_dashattack", 4.25), None);
    }

    /// Unit 4 is each listed unit (none without a list); 1 and 2 are the plot's caster and target, 3 the step's source, 5 none.
    #[test]
    fn step_units_name_the_plot_and_the_listed_units() {
        let mut world = World::new();
        let [me, plot_target, listed] = [(); 3].map(|_| world.spawn_empty().id());
        let units = StepUnits { plot_caster: Some(me), plot_target: Some(plot_target), source: me, listed: vec![listed] };
        assert_eq!(units.of(skills::plot_unit::CASTER, None), Some(me));
        assert_eq!(units.of(skills::plot_unit::PLOT_TARGET, None), Some(plot_target));
        assert_eq!(units.of(skills::plot_unit::SOURCE, None), Some(me));
        assert_eq!(units.of(skills::plot_unit::EACH, Some(listed)), Some(listed));
        assert_eq!(units.of(skills::plot_unit::EACH, None), None);
        assert_eq!(units.of(skills::plot_unit::SPOT, Some(listed)), None);
    }

    #[test]
    fn scheduled_weapon_sound_keeps_each_received_context() {
        let book = SkillBook::open().expect("read-only game catalog");
        let mut world = World::new();
        world.insert_resource(book);
        world.init_resource::<ClipLibrary>();
        world.init_resource::<Time>();
        world.init_resource::<Time<Virtual>>();
        world.init_resource::<Units>();
        world.init_resource::<aa_net::state::Doodads>();
        world.init_resource::<Skills>();
        world.init_resource::<super::super::skill_motion::Inbox>();
        world.init_resource::<Combat>();
        world.init_resource::<Messages<NetEvent>>();
        world.spawn((LocalPlayer { obj_id: 1 }, GlobalTransform::default()));
        world.spawn((LocalPlayer { obj_id: 2 }, GlobalTransform::default()));
        world.resource_mut::<Combat>().casts.insert(7, Cast { skill: 3, caster: 1, target: 2, fired: Some(1.0), delay: 0.5, ..default() });
        let damage = |tl, hit, holdable, raw| {
            let Event::UnitDamaged(mut damage) = Fake::event("hit", &[tl,3,0], 1, 2, None).unwrap() else { unreachable!() };
            damage.hit_type = hit;
            damage.holdable_id = holdable;
            damage.raw_hit = raw;
            damage
        };
        let first = damage(7,3,3,0x2123);
        let second = damage(7,4,24,0x1124);
        for damage in [first.clone(),second.clone(),damage(8,3,3,0x0123)] {
            world.resource_mut::<Messages<NetEvent>>().write(NetEvent(Event::UnitDamaged(damage)));
        }
        let mut schedule = Schedule::default();
        schedule.add_systems(handle_events);
        schedule.run(&mut world);
        let hits: Vec<_> = world.resource::<Combat>().timers.iter().filter_map(|timer| match &timer.what {
            Later::Hit { sound, hurt, .. } => Some((timer.at,sound,hurt)),
            _ => None,
        }).collect();
        assert_eq!(hits.len(),3);
        assert_eq!(hits[0].0,1.5);
        assert_eq!(hits[1].0,1.5);
        assert_eq!(hits[0].1.as_ref(),Some(&first));
        assert_eq!(hits[1].1.as_ref(),Some(&second));
        assert!(hits[0].2.is_none() && hits[1].2.is_none(), "zero damage/miss suppresses hurt FX, not the sound context");
        assert!(hits[2].1.is_none(), "an unknown cast cannot introduce a native weapon sound");
    }

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

    /// `SCSkillFired` of a skill; the caster is `raw` as aa-net projects it (a mate: kind 3).
    fn fired(tl: u16, skill: u32, anim: u32, raw: aa_net::SkillCasterData) -> Event {
        Event::SkillFired(SkillFired { skill_id: skill, tl, caster: raw.projection(), target: SkillTarget::unit(0), delay_ms: 0, channeling_ms: 0, fire_anim_id: anim, raw_final_flag: 0, result: Default::default(), raw_caster: raw, raw_target: aa_net::SkillTargetData::Unit(0) })
    }

    fn started(tl: u16, skill: u32, cast_ms: u32, raw: aa_net::SkillCasterData) -> Event {
        Event::SkillStarted(SkillStarted { skill_id: skill, tl, caster: raw.projection(), target: SkillTarget::unit(0), cast_ms, base_cast_ms: cast_ms, synergy: 0, result: 0, result_value: 0, raw_result: Default::default(), raw_caster: raw, raw_target: aa_net::SkillTargetData::Unit(0) })
    }

    /// An app that turns skill events into clips, on a hand-advanced clock.
    fn clip_app(book: SkillBook, clips: ClipLibrary, units: Units) -> App {
        let mut app = App::new();
        app.init_resource::<Time>()
            .init_resource::<Time<Virtual>>()
            .insert_resource(clips)
            .insert_resource(book)
            .insert_resource(units)
            .insert_resource(aa_net::state::Doodads::default())
            .insert_resource(Skills::default())
            .init_resource::<crate::ingame::skill_motion::Inbox>()
            .insert_resource(Combat::default())
            .add_message::<NetEvent>();
        app
    }

    fn tick(app: &mut App, dt: f64, event: Option<Event>) -> f64 {
        app.world_mut().resource_mut::<Time>().advance_by(std::time::Duration::from_secs_f64(dt));
        if let Some(event) = event {
            app.world_mut().write_message(NetEvent(event));
        }
        app.update();
        app.world().resource::<Time>().elapsed_secs_f64()
    }

    /// The files of the clips started on the upper body layer of `unit` since the last call (needs `aa_anim::trace`).
    fn layer_starts(app: &mut App, unit: Entity) -> Vec<String> {
        let mut animator = app.world_mut().get_mut::<Animator>(unit).unwrap();
        let starts = animator.take_starts();
        starts.iter().filter(|s| s.layer == aa_anim::trace::UPPER).filter_map(|s| s.file.as_deref()?.rsplit('/').next().map(str::to_string)).collect()
    }

    /// A rider seated by `mount::start_stop_riding` (which takes its locomotion away) plays its skills on the upper body:
    /// Mounted Attack (Rider) 11013's `onehand_co_horse_attack_r_slash` (84), which has no `ride_ub` and is a state of the upper body
    /// graph only, and Mounted Defense 11036's 394 through its `ride_ub` (`onehand_co_sk_shielddefense_mub`). The rider loop stays the
    /// base clip; the action and the layer end with the clip (needs `AA_PAK` and `AA_DB`).
    #[test]
    fn a_seated_rider_plays_its_skills_on_the_upper_body() {
        let (Some(book), Some(db), Ok(game)) = (SkillBook::open(), crate::ingame::data::Db::open(), aa_anim::GameDb::open()) else {
            eprintln!("no database: skipped");
            return;
        };
        if !aa_cry::pak_available() {
            eprintln!("no pak: skipped");
            return;
        }
        let assets = aa_anim::CharacterDesc::default_for(&game, aa_anim::Race::Nuian, aa_anim::Gender::Male).and_then(|d| d.resolve(&game)).expect("a Nuian man");
        let rig = shared_rig(&assets.skeleton).expect("his rig");
        let cal = Cal::shared(&assets.cal).expect("his .cal");
        let clips = ClipLibrary::default();
        let set = ClipSet::load(&rig, cal.clone(), &clips).expect("his clips");
        let ride_loop = bind_clip(&rig, &cal, &clips, "horse_ba_relaxed_idle").expect("the rider idle");
        let length = |name: &str| bind_action_clip(&rig, &cal, &clips, name).map(|c| c.clip.duration).unwrap_or_else(|| panic!("{name}"));
        let (attack, defense) = (length("onehand_co_horse_attack_r_slash"), length("onehand_co_sk_shielddefense_mub"));
        let mut rider = crate::ingame::seating::fixture_unit(7, 42, None);
        rider.attached_to = Some(aa_net::model::Attachment { obj_id: 9, point: 1, reason: 1 });
        let mut horse = crate::ingame::seating::fixture_unit(9, 43, None);
        horse.model_id = 139;
        let mut units = Units::default();
        units.by_id.insert(7, rider.clone());
        units.by_id.insert(9, horse);
        let mut app = clip_app(book, clips, units);
        app.insert_resource(db).add_systems(Update, (crate::ingame::mount::start_stop_riding, handle_events, end_actions, release_layers).chain());
        let character = aa_anim::Character { assets: Arc::new(assets), model_root: Entity::PLACEHOLDER, parts: Vec::new() };
        let entity = app
            .world_mut()
            .spawn((RemoteUnit::announced(&rider), Animator::new(rig, Vec::new()), character, Locomotion::new(set), Motion::default(), Transform::default(), GlobalTransform::default()))
            .id();
        tick(&mut app, 0.1, None);
        assert!(app.world().get::<crate::ingame::mount::Rider>(entity).is_some() && app.world().get::<Locomotion>(entity).is_none(), "seated without locomotion");
        app.world_mut().get_mut::<Animator>(entity).unwrap().play(ride_loop, true, 0.0);
        let base = |app: &App| app.world().get::<Animator>(entity).unwrap().current.as_ref().map(|p| Arc::as_ptr(&p.clip.clip));
        let riding = base(&app);
        let unit = aa_net::SkillCasterData::Unit(7);
        // the layer's clip is private to the animator: the clip-start record names it (the skill trace's, on for every animator)
        aa_anim::trace::enable();
        app.world_mut().get_mut::<Animator>(entity).unwrap().take_starts();
        for (tl, skill, anim, clip, length) in [(1, 11013, 84, "onehand_co_horse_attack_r_slash", attack), (2, 11036, 394, "onehand_co_sk_shielddefense_mub", defense)] {
            let now = tick(&mut app, 0.1, Some(fired(tl, skill, anim, unit)));
            let acting = *app.world().get::<Acting>(entity).unwrap_or_else(|| panic!("{skill} plays on the rider"));
            assert!(acting.layered, "{skill} is a layer");
            let duration = f64::from(length.min(4.0));
            assert!((acting.until - now - duration).abs() < 1e-3, "{skill} plays {clip} ({duration:.3} s), not a {:.3} s clip", acting.until - now);
            assert!(app.world().get::<Animator>(entity).unwrap().has_layer());
            assert_eq!(layer_starts(&mut app, entity), [format!("{clip}.caf")]);
            assert_eq!(base(&app), riding, "the rider loop keeps playing under {clip}");
            // the action is over with its clip; the one-shot layer has ended by then too
            app.world_mut().get_mut::<Animator>(entity).unwrap().update(length + 0.2);
            tick(&mut app, duration + 0.01, None);
            assert!(app.world().get::<Acting>(entity).is_none(), "no action is left on the rider");
            assert!(!app.world().get::<Animator>(entity).unwrap().has_layer(), "the layer ended with {clip}");
            assert_eq!(base(&app), riding);
        }
    }

    /// A buff pose on a seated rider: the stun 63 (`all_re_combat_stun`, whose row's `ride_ub` is `all_re_combat_stun_ub`) and
    /// the knockdown 1172 (`knock_down`, thrown with `all_re_combat_knockback_start`). The pose plays on the upper body like any
    /// other action of the rider; the whole body start-and-loop chain a standing unit gets (`play_once_then`) must not replace
    /// the rider loop, which is still the base clip when the buff is gone (needs `AA_PAK` and `AA_DB`).
    #[test]
    fn a_buff_pose_leaves_the_rider_loop_alone() {
        let (Some(book), Some(db), Ok(game)) = (SkillBook::open(), crate::ingame::data::Db::open(), aa_anim::GameDb::open()) else {
            eprintln!("no database: skipped");
            return;
        };
        if !aa_cry::pak_available() {
            eprintln!("no pak: skipped");
            return;
        }
        let assets = aa_anim::CharacterDesc::default_for(&game, aa_anim::Race::Nuian, aa_anim::Gender::Male).and_then(|d| d.resolve(&game)).expect("a Nuian man");
        let rig = shared_rig(&assets.skeleton).expect("his rig");
        let cal = Cal::shared(&assets.cal).expect("his .cal");
        let clips = ClipLibrary::default();
        let set = ClipSet::load(&rig, cal.clone(), &clips).expect("his clips");
        let ride_loop = bind_clip(&rig, &cal, &clips, "horse_ba_relaxed_idle").expect("the rider idle");
        let mut rider = crate::ingame::seating::fixture_unit(7, 42, None);
        rider.attached_to = Some(aa_net::model::Attachment { obj_id: 9, point: 1, reason: 1 });
        rider.buffs.clear();
        let mut horse = crate::ingame::seating::fixture_unit(9, 43, None);
        horse.model_id = 139;
        let mut units = Units::default();
        units.by_id.insert(7, rider.clone());
        units.by_id.insert(9, horse);
        let mut app = clip_app(book, clips, units);
        app.insert_resource(db).insert_resource(State::new(crate::Screen::InGame)).init_resource::<framehold::Frameholds>().add_systems(
            Update,
            (
                crate::ingame::mount::start_stop_riding,
                framehold::observe,
                framehold::sync,
                buffs::ensure_visuals,
                handle_events,
                framehold::sync_after_receipts,
                buffs::reconcile_buffs,
                end_actions,
                release_layers,
            )
                .chain(),
        );
        app.world_mut().spawn((LocalPlayer { obj_id: 100 }, GlobalTransform::default()));
        let character = aa_anim::Character { assets: Arc::new(assets), model_root: Entity::PLACEHOLDER, parts: Vec::new() };
        let entity = app
            .world_mut()
            .spawn((RemoteUnit::announced(&rider), Animator::new(rig, Vec::new()), character, Locomotion::new(set), Motion::default(), Transform::default(), GlobalTransform::default()))
            .id();
        tick(&mut app, 0.1, Some(Event::UnitSpawned(Box::new(rider.clone()))));
        assert!(app.world().get::<crate::ingame::mount::Rider>(entity).is_some() && app.world().get::<Locomotion>(entity).is_none(), "seated without locomotion");
        app.world_mut().get_mut::<Animator>(entity).unwrap().play(ride_loop, true, 0.0);
        let base = |app: &App| app.world().get::<Animator>(entity).unwrap().current.as_ref().map(|p| Arc::as_ptr(&p.clip.clip));
        let riding = base(&app);
        for (index, id) in [(1, 63), (2, 1172)] {
            let buff = aa_net::buff::Buff { buff_id: id, index, kind: aa_net::buff::BuffKind::Unknown, ..default() };
            tick(&mut app, 0.1, Some(Event::BuffCreated { obj_id: 7, buff }));
            assert!(app.world().get::<Acting>(entity).is_some(), "buff {id} poses the rider");
            assert!(app.world().get::<Animator>(entity).unwrap().has_layer(), "buff {id} is a layer");
            assert_eq!(base(&app), riding, "buff {id} keeps the rider loop under its pose");
            tick(&mut app, 0.1, Some(Event::BuffRemoved { obj_id: 7, index }));
            // the knockdown's stand-up plays out before the rider is free again
            for _ in 0..40 {
                app.world_mut().get_mut::<Animator>(entity).unwrap().update(0.1);
                tick(&mut app, 0.1, None);
            }
            assert!(app.world().get::<Acting>(entity).is_none(), "buff {id}: no action is left on the rider");
            assert!(!app.world().get::<Animator>(entity).unwrap().has_layer(), "buff {id}: its layer is gone");
            assert_eq!(base(&app), riding, "buff {id}: the rider loop is the base clip after the buff");
        }
    }

    /// The mount's own skill: Back Kick 11369 (start 18 `all_co_sk_shout`, fire 20 `all_co_sk_lowkick`, 300 ms cast) cast by the
    /// driven horse (`SCSkillStarted` / `SCSkillFired` with a mount caster). Standing it plays both clips on the whole body; galloping
    /// it plays them on the upper body (the horse rig has a `Bip01 Spine`) and keeps its run (needs `AA_PAK` and `AA_DB`).
    #[test]
    fn the_driven_horse_plays_its_own_skill_and_keeps_galloping() {
        let Some(book) = SkillBook::open() else { return };
        if !aa_cry::pak_available() {
            eprintln!("no pak: skipped");
            return;
        }
        const HORSE: &str = "objects/characters/animals/horse/horse.chr";
        let clips = ClipLibrary::default();
        let set = crate::ingame::anim::shared_clip_set(HORSE, "objects/characters/animals/horse/horse.cal", &clips).expect("the horse clips");
        assert!(set.upper.is_some(), "the horse rig has an upper body");
        let rig = shared_rig(HORSE).expect("the horse rig");
        let kick = bind_action_clip(&rig, &set.cal, &clips, "all_co_sk_lowkick").expect("the horse's kick").clip.duration;
        let mut horse = crate::ingame::seating::fixture_unit(9, 43, None);
        horse.model_id = 139;
        let mut units = Units::default();
        units.by_id.insert(9, horse.clone());
        let mut app = clip_app(book, clips, units);
        app.add_systems(Update, (handle_events, end_actions, release_layers, crate::ingame::anim::drive_locomotion).chain());
        let mut loco = Locomotion::new((*set).clone());
        loco.invalidate();
        let mate = app
            .world_mut()
            .spawn((RemoteUnit::announced(&horse), Animator::new(rig, Vec::new()), loco, Motion::default(), crate::ingame::mount::Driven, Transform::default(), GlobalTransform::default()))
            .id();
        let playing = |app: &App| {
            let animator = app.world().get::<Animator>(mate).unwrap();
            animator.current.as_ref().and_then(|p| clip_file(&p.clip)).map_or(String::new(), |f| f.to_string())
        };
        let acting = |app: &App| app.world().get::<Acting>(mate).copied();
        let mount = aa_net::SkillCasterData::Mount { obj_id: 9, mount_skill_id: 11369 };
        tick(&mut app, 0.1, None);
        let idle = playing(&app);

        // standing: the whole body
        tick(&mut app, 0.1, Some(started(1, 11369, 300, mount)));
        assert_eq!((playing(&app).as_str(), acting(&app).map(|a| a.layered)), ("all_co_sk_shout.caf", Some(false)));
        let now = tick(&mut app, 0.3, Some(fired(1, 11369, 20, mount)));
        assert_eq!(playing(&app), "all_co_sk_lowkick.caf");
        let until = acting(&app).expect("kicking").until;
        assert!((until - now - f64::from(kick)).abs() < 1e-3, "the kick lasts its clip: {}", until - now);
        tick(&mut app, f64::from(kick) + 0.01, None);
        assert!(acting(&app).is_none());
        assert_eq!(playing(&app), idle, "back to the idle");

        // galloping: the upper body, with the run kept below
        {
            let mut motion = app.world_mut().get_mut::<Motion>(mate).unwrap();
            motion.speed = 9.0;
            motion.local_dir = Vec2::Y;
        }
        tick(&mut app, 0.1, None);
        let run = playing(&app);
        assert!(run.contains("run"), "{run}");
        aa_anim::trace::enable();
        layer_starts(&mut app, mate);
        tick(&mut app, 0.1, Some(started(2, 11369, 300, mount)));
        assert_eq!(acting(&app).map(|a| a.layered), Some(true), "the shout is a layer");
        // the horse's `.cal` has no `all_co_sk_shout_ub` (`move_ub`): the clip itself plays through the mask
        assert_eq!(layer_starts(&mut app, mate), ["all_co_sk_shout.caf"]);
        assert!(app.world().get::<Animator>(mate).unwrap().has_layer());
        assert_eq!(playing(&app), run);
        let now = tick(&mut app, 0.3, Some(fired(2, 11369, 20, mount)));
        let kicking = acting(&app).expect("kicking");
        assert!(kicking.layered && (kicking.until - now - f64::from(kick)).abs() < 1e-3, "{kicking:?}");
        assert_eq!(layer_starts(&mut app, mate), ["all_co_sk_lowkick.caf"]);
        assert_eq!(playing(&app), run, "the gallop goes on under the kick");
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

    /// The battle flag is the `+0x3bba` byte: a real blow sets it on both units (`FUN_39391670`), `SCCombatCleared` clears it at
    /// once (`FUN_391d6a60` -> `FUN_39355ec0` -> `FUN_390b3710`) and nothing else does. The separate combat idle (`+0x5fc4`,
    /// `FUN_39355ee0`) is refreshed while the flag is set and keeps its 10 s deadline from the last such update
    /// (`DAT_39ac7424` = 10000 ms); the clear does not touch it. Its expiry ends the idle, not the flag and not the weapon.
    #[test]
    fn the_combat_flag_clears_on_the_packet_and_the_idle_keeps_its_ten_second_deadline() {
        if !aa_cry::pak_available() {
            return;
        }
        let Some(book) = SkillBook::open() else { return };
        let mut world = World::new();
        world.insert_resource(book);
        world.init_resource::<ClipLibrary>();
        world.init_resource::<Time>();
        world.init_resource::<Time<Virtual>>();
        world.init_resource::<Units>();
        world.init_resource::<aa_net::state::Doodads>();
        world.init_resource::<Skills>();
        world.init_resource::<super::super::skill_motion::Inbox>();
        world.init_resource::<Combat>();
        world.init_resource::<Messages<NetEvent>>();
        let local = world.spawn((LocalPlayer { obj_id: 1 }, GlobalTransform::default(), Drawn::after_action(10.0, Out::Blade { dual: false }))).id();
        let rat = world.spawn((LocalPlayer { obj_id: 2 }, GlobalTransform::default())).id();
        let mut schedule = Schedule::default();
        schedule.add_systems((handle_events, bevy::ecs::schedule::ApplyDeferred, tick_combat_idle).chain());
        // the blow puts both units in the battle stance (`FUN_39391670`) and refreshes the combat idle
        world.resource_mut::<Messages<NetEvent>>().write(NetEvent(Fake::event("hit", &[1, 2, 100], 1, 2, None).expect("a hit event")));
        schedule.run(&mut world);
        for unit in [local, rat] {
            assert!(world.get::<CombatStance>(unit).is_some(), "a blow starts the battle flag");
            assert!(world.get::<CombatIdle>(unit).is_some(), "and the combat idle");
        }
        // the target is lost 0.7 s later: neither the flag nor the idle ends (only the server's clear ends the flag)
        world.resource_mut::<Time>().advance_by(std::time::Duration::from_millis(700));
        schedule.run(&mut world);
        assert!(world.get::<CombatStance>(local).is_some(), "losing the target alone ends no flag");
        assert!(world.get::<CombatIdle>(local).is_some());
        // the server's clear 0.3 s later: the flag ends at once; the idle keeps the deadline of the last update while flagged
        world.resource_mut::<Time>().advance_by(std::time::Duration::from_millis(300));
        world.resource_mut::<Messages<NetEvent>>().write(NetEvent(Event::CombatChanged { obj_id: 1, engaged: false }));
        schedule.run(&mut world);
        assert!(world.get::<CombatStance>(local).is_none(), "OnCombatCleared clears the flag");
        let idle = *world.get::<CombatIdle>(local).expect("the combat idle stays after the clear");
        assert!((idle.last - 0.7).abs() < 1e-6, "the clear does not refresh the deadline ({idle:?})");
        let drawn = world.get::<Drawn>(local).expect("the weapon stays");
        assert_eq!(drawn.stance(1.0), Some(Out::Blade { dual: false }), "the weapon byte is separate");
        assert!(world.get::<CombatStance>(rat).is_some(), "another unit's flag is its own packet");
        // 9 s after the clear: the idle is still there; the rat's flag keeps refreshing its own deadline
        world.resource_mut::<Time>().advance_by(std::time::Duration::from_millis(9000));
        schedule.run(&mut world);
        assert!(world.get::<CombatIdle>(local).is_some(), "the clear + 9 s is inside the 10 s deadline");
        let rat_idle = *world.get::<CombatIdle>(rat).expect("the flag refreshes the idle every update");
        assert!((rat_idle.last - 10.0).abs() < 1e-6, "the deadline follows the update while engaged ({rat_idle:?})");
        // 1.1 s later (10.4 s after the last flagged update): the idle falls (FUN_39355ee0 writes false to the model)
        world.resource_mut::<Time>().advance_by(std::time::Duration::from_millis(1100));
        schedule.run(&mut world);
        assert!(world.get::<CombatIdle>(local).is_none(), "the 10 s deadline ended the idle");
        assert!(world.get::<CombatStance>(local).is_none(), "the deadline does not touch the flag");
        assert!(world.get::<CombatIdle>(rat).is_some(), "the rat's flag is still set, so its idle never expires");
        assert!(world.get::<Drawn>(local).is_some(), "the weapon is not put away by the idle deadline");
        // the next fight engages it again (OnCombatEngaged)
        world.resource_mut::<Messages<NetEvent>>().write(NetEvent(Event::CombatChanged { obj_id: 1, engaged: true }));
        schedule.run(&mut world);
        assert!(world.get::<CombatStance>(local).is_some(), "re-engaging restores the flag");
        assert!(world.get::<CombatIdle>(local).is_some());
    }

    /// A skill start refreshes no combat idle: the `+0x5fc5` / `+0x5fc6` pulses are on the local player's request path
    /// (`FUN_39390f80`), not on `SCSkillStarted`. A clear on a unit that is not flagged changes nothing: it does not create an
    /// idle, and it does not touch an existing one (`FUN_39355ec0` clears only `+0x3bba`).
    #[test]
    fn a_skill_start_pulses_no_combat_idle_and_a_clear_without_engagement_does_nothing() {
        let Some(book) = SkillBook::open() else { return };
        let mut world = World::new();
        world.insert_resource(book);
        world.init_resource::<ClipLibrary>();
        world.init_resource::<Time>();
        world.init_resource::<Time<Virtual>>();
        world.init_resource::<Units>();
        world.init_resource::<aa_net::state::Doodads>();
        world.init_resource::<Skills>();
        world.init_resource::<super::super::skill_motion::Inbox>();
        world.init_resource::<Combat>();
        world.init_resource::<Messages<NetEvent>>();
        let local = world.spawn((LocalPlayer { obj_id: 1 }, GlobalTransform::default())).id();
        let remote = world.spawn((LocalPlayer { obj_id: 2 }, GlobalTransform::default())).id();
        let mut schedule = Schedule::default();
        schedule.add_systems((handle_events, bevy::ecs::schedule::ApplyDeferred).chain());
        // a remote caster's attack and the local player's gathering skill: neither enters the combat idle
        world.resource_mut::<Messages<NetEvent>>().write(NetEvent(Fake::event("start", &[1, 18132, 0], 2, 1, None).expect("a start event")));
        world.resource_mut::<Messages<NetEvent>>().write(NetEvent(Fake::event("start", &[2, 13789, 0], 1, 0, None).expect("a start event")));
        schedule.run(&mut world);
        for unit in [local, remote] {
            assert!(world.get::<CombatIdle>(unit).is_none(), "a skill start pulses no idle");
            assert!(world.get::<CombatStance>(unit).is_none(), "and sets no battle flag");
        }
        // a clear on the unflagged local player creates no idle
        world.resource_mut::<Messages<NetEvent>>().write(NetEvent(Event::CombatChanged { obj_id: 1, engaged: false }));
        schedule.run(&mut world);
        assert!(world.get::<CombatIdle>(local).is_none(), "a clear on an unflagged unit creates no idle");
        // a clear leaves an existing idle alone: `FUN_39355ec0` does not touch `+0x5fc0`
        world.entity_mut(local).insert(CombatIdle::at(5.0));
        world.resource_mut::<Time>().advance_by(std::time::Duration::from_secs(2));
        world.resource_mut::<Messages<NetEvent>>().write(NetEvent(Event::CombatChanged { obj_id: 1, engaged: false }));
        schedule.run(&mut world);
        let idle = *world.get::<CombatIdle>(local).expect("the idle stays");
        assert!((idle.last - 5.0).abs() < 1e-6, "the clear does not refresh the deadline ({idle:?})");
    }

    /// The request path pulses the combat idle for the local player's auto attack: `FUN_39390f80`'s callers pass skill 2
    /// (`DAT_39ac769c`; the click attack passes `DAT_39ac3874` = 2), and so does a `start_autoattack` skill aimed at another unit
    /// (the 1.2 client's combat idle at the press of Triple Slash). A `start_autoattack` self-buff (Frenzy 10455) starts none.
    #[test]
    fn only_the_auto_attack_request_pulses_the_combat_idle() {
        let Some(book) = SkillBook::open() else { return };
        let mut world = World::new();
        world.insert_resource(book);
        world.init_resource::<ClipLibrary>();
        world.init_resource::<Time>();
        world.init_resource::<Time<Virtual>>();
        world.init_resource::<Units>();
        world.init_resource::<aa_net::state::Doodads>();
        world.init_resource::<Skills>();
        world.init_resource::<super::super::skill_motion::Inbox>();
        world.init_resource::<Combat>();
        world.init_resource::<Messages<NetCommand>>();
        let me = world.spawn((LocalPlayer { obj_id: 7 }, GlobalTransform::default())).id();
        let mut schedule = Schedule::default();
        schedule.add_systems((request_skill, bevy::ecs::schedule::ApplyDeferred).chain());
        // the basic attack: a request creates the 1 s pulse (`FUN_39354610`)
        world.resource_mut::<Messages<NetCommand>>().write(NetCommand(Command::UseSkill { skill_id: 2, target_obj_id: 0 }));
        schedule.run(&mut world);
        let idle = *world.get::<CombatIdle>(me).expect("the auto attack request pulses the idle");
        assert!(idle.pulse_until > idle.last, "it is the +0x5fc5 pulse ({idle:?})");
        // Frenzy 10455, a self buff with start_autoattack 't', enters it too (`fist_ba_combat_idle` at +15 ms in the 1.2 client)
        world.entity_mut(me).remove::<CombatIdle>();
        world.resource_mut::<Messages<NetCommand>>().write(NetCommand(Command::UseSkill { skill_id: 10455, target_obj_id: 0 }));
        schedule.run(&mut world);
        assert!(world.get::<CombatIdle>(me).is_some(), "a start_autoattack self-buff enters the combat idle at the press");
        // Terrifying Roar 18308, a self skill without the flag, does not
        world.entity_mut(me).remove::<CombatIdle>();
        world.resource_mut::<Messages<NetCommand>>().write(NetCommand(Command::UseSkill { skill_id: 18308, target_obj_id: 0 }));
        schedule.run(&mut world);
        assert!(world.get::<CombatIdle>(me).is_none(), "a skill without start_autoattack does not");
        // Triple Slash 18132 (start_autoattack) on another unit enters it at the press, as the 1.2 client does
        let mut doll = crate::hud::fixture_spawn().unit;
        doll.obj_id = 40;
        world.spawn((super::super::units::RemoteUnit::announced(&doll), GlobalTransform::default()));
        world.resource_mut::<Messages<NetCommand>>().write(NetCommand(Command::UseSkill { skill_id: 18132, target_obj_id: 40 }));
        schedule.run(&mut world);
        assert!(world.get::<CombatIdle>(me).is_some(), "a start_autoattack skill on another unit enters the combat idle at the press");
        // a skill without the flag does not (Tiger Strike 13315)
        world.entity_mut(me).remove::<CombatIdle>();
        world.resource_mut::<Messages<NetCommand>>().write(NetCommand(Command::UseSkill { skill_id: 13315, target_obj_id: 40 }));
        schedule.run(&mut world);
        assert!(world.get::<CombatIdle>(me).is_none(), "Tiger Strike does not turn the auto attack on");
    }

    /// `FUN_39391670` needs a real blow: a miss or a zero sets no battle flag, and a dead unit is left out.
    #[test]
    fn a_miss_or_a_zero_damage_sets_no_battle_flag() {
        let Some(book) = SkillBook::open() else { return };
        let mut world = World::new();
        world.insert_resource(book);
        world.init_resource::<ClipLibrary>();
        world.init_resource::<Time>();
        world.init_resource::<Time<Virtual>>();
        world.init_resource::<Units>();
        world.init_resource::<aa_net::state::Doodads>();
        world.init_resource::<Skills>();
        world.init_resource::<super::super::skill_motion::Inbox>();
        world.init_resource::<Combat>();
        world.init_resource::<Messages<NetEvent>>();
        let local = world.spawn((LocalPlayer { obj_id: 1 }, GlobalTransform::default())).id();
        let rat = world.spawn((LocalPlayer { obj_id: 2 }, GlobalTransform::default())).id();
        let mut schedule = Schedule::default();
        schedule.add_systems((handle_events, bevy::ecs::schedule::ApplyDeferred).chain());
        let damage = |damage: i32, hit: u8| {
            let Event::UnitDamaged(mut d) = Fake::event("hit", &[1, 2, damage.max(0) as u32], 1, 2, None).expect("a hit event") else { unreachable!() };
            d.damage = damage;
            d.hit_type = hit;
            d
        };
        // zero damage and a miss: no flag on either side
        for d in [damage(0, 1), damage(100, hit_type::MELEE_MISS), damage(100, hit_type::MELEE_DODGE)] {
            world.resource_mut::<Messages<NetEvent>>().write(NetEvent(Event::UnitDamaged(d)));
            schedule.run(&mut world);
            for unit in [local, rat] {
                assert!(world.get::<CombatStance>(unit).is_none(), "no real blow, no battle flag");
            }
        }
        // a real blow sets both
        world.resource_mut::<Messages<NetEvent>>().write(NetEvent(Event::UnitDamaged(damage(100, hit_type::MELEE_HIT))));
        schedule.run(&mut world);
        for unit in [local, rat] {
            assert!(world.get::<CombatStance>(unit).is_some(), "a real blow starts the battle flag");
        }
        // a blow on a dead unit is left out (the decompile's `+0x3c26` / `+0x3c27`)
        world.entity_mut(rat).insert(Dead { at: 0.0 });
        world.entity_mut(local).remove::<CombatStance>();
        world.entity_mut(rat).remove::<CombatStance>();
        world.resource_mut::<Messages<NetEvent>>().write(NetEvent(Event::UnitDamaged(damage(100, hit_type::MELEE_HIT))));
        schedule.run(&mut world);
        assert!(world.get::<CombatStance>(local).is_none() && world.get::<CombatStance>(rat).is_none(), "a dead target is no blow");
    }

    /// `FUN_39398c80` calls `FUN_39391670` after every damage kind (`749000-749128`): a buff tick is a
    /// real blow for the battle flag too, while the visuals still skip it (the number lands at once).
    #[test]
    fn buff_damage_also_flags_both_units() {
        let Some(book) = SkillBook::open() else { return };
        let mut world = World::new();
        world.insert_resource(book);
        world.init_resource::<ClipLibrary>();
        world.init_resource::<Time>();
        world.init_resource::<Time<Virtual>>();
        world.init_resource::<Units>();
        world.init_resource::<aa_net::state::Doodads>();
        world.init_resource::<Skills>();
        world.init_resource::<super::super::skill_motion::Inbox>();
        world.init_resource::<Combat>();
        world.init_resource::<Messages<NetEvent>>();
        let local = world.spawn((LocalPlayer { obj_id: 1 }, GlobalTransform::default())).id();
        let rat = world.spawn((LocalPlayer { obj_id: 2 }, GlobalTransform::default())).id();
        let mut schedule = Schedule::default();
        schedule.add_systems((handle_events, bevy::ecs::schedule::ApplyDeferred).chain());
        let Event::UnitDamaged(mut d) = Fake::event("hit", &[1, 2, 100], 1, 2, None).expect("a hit event") else { unreachable!() };
        d.action.kind = 2;
        d.action.buff_id = 1234;
        world.resource_mut::<Messages<NetEvent>>().write(NetEvent(Event::UnitDamaged(d)));
        schedule.run(&mut world);
        for unit in [local, rat] {
            assert!(world.get::<CombatStance>(unit).is_some(), "a buff tick flags both sides");
        }
        assert_eq!(world.resource::<Combat>().landed.len(), 1, "the tick still lands as a number at once");
    }

    /// `FUN_39391670`'s third `FUN_390b36f0`: a blow on the local player's own mate (`+0x410 == 5`, `+0x420` the local
    /// character id) also sets the owner's battle flag when the owner (`FUN_3935e370`) is alive. The check is on the
    /// target only: a blow the mate deals flags the mate and its victim, not the owner; another owner's mate and a dead
    /// owner are left out.
    #[test]
    fn a_blow_on_the_local_players_mate_flags_the_owner() {
        let Some(book) = SkillBook::open() else { return };
        let spawn = crate::hud::fixture_spawn();
        let (owner_obj, owner_char) = (spawn.obj_id, spawn.character().id);
        let mut world = World::new();
        world.insert_resource(book);
        world.init_resource::<ClipLibrary>();
        world.init_resource::<Time>();
        world.init_resource::<Time<Virtual>>();
        world.init_resource::<Units>();
        world.init_resource::<aa_net::state::Doodads>();
        world.init_resource::<Skills>();
        world.init_resource::<super::super::skill_motion::Inbox>();
        world.init_resource::<Combat>();
        world.init_resource::<Messages<NetEvent>>();
        world.insert_resource(Player { spawn: Some(spawn.clone()) });
        let owner = world.spawn((LocalPlayer { obj_id: owner_obj }, GlobalTransform::default())).id();
        let mate = world.spawn((LocalPlayer { obj_id: 200 }, GlobalTransform::default())).id();
        let attacker = world.spawn((LocalPlayer { obj_id: 2 }, GlobalTransform::default())).id();
        let mut mate_unit = spawn.unit.clone();
        mate_unit.obj_id = 200;
        mate_unit.kind = UnitKind::Mate { tl_id: 9, template_id: 1, owner_id: owner_char };
        world.resource_mut::<Units>().by_id.insert(200, mate_unit);
        let mut schedule = Schedule::default();
        schedule.add_systems((handle_events, bevy::ecs::schedule::ApplyDeferred).chain());
        let hit = |from: u32, on: u32, tl: u32| Fake::event("hit", &[tl, 2, 100], from, on, None).expect("a hit event");
        // the attacker strikes the mate: all three flags
        world.resource_mut::<Messages<NetEvent>>().write(NetEvent(hit(2, 200, 1)));
        schedule.run(&mut world);
        for unit in [owner, mate, attacker] {
            assert!(world.get::<CombatStance>(unit).is_some(), "the mate's owner is flagged too");
        }
        // the mate strikes the attacker: the owner's flag is not the mate's to set
        world.entity_mut(owner).remove::<(CombatStance, CombatIdle)>();
        world.resource_mut::<Messages<NetEvent>>().write(NetEvent(hit(200, 2, 2)));
        schedule.run(&mut world);
        assert!(world.get::<CombatStance>(owner).is_none(), "a blow dealt by the mate leaves the owner alone");
        assert!(world.get::<CombatStance>(mate).is_some() && world.get::<CombatStance>(attacker).is_some());
        // the mate belongs to another character: no owner flag
        world.resource_mut::<Units>().by_id.get_mut(&200).unwrap().kind = UnitKind::Mate { tl_id: 9, template_id: 1, owner_id: owner_char + 1 };
        world.resource_mut::<Messages<NetEvent>>().write(NetEvent(hit(2, 200, 3)));
        schedule.run(&mut world);
        assert!(world.get::<CombatStance>(owner).is_none(), "another character's mate is not the player's");
        // the owner is dead: `FUN_39391670` needs the owner alive (`+0x3c26` / `+0x3c27` zero)
        world.resource_mut::<Units>().by_id.get_mut(&200).unwrap().kind = UnitKind::Mate { tl_id: 9, template_id: 1, owner_id: owner_char };
        world.entity_mut(owner).insert(Dead { at: 0.0 });
        world.resource_mut::<Messages<NetEvent>>().write(NetEvent(hit(2, 200, 4)));
        schedule.run(&mut world);
        assert!(world.get::<CombatStance>(owner).is_none(), "a dead owner is not flagged");
    }

    /// Putting the weapon away zeroes `+0x5fc0` (`FUN_39360920`), so the combat idle ends with it; the stance byte is separate.
    #[test]
    fn sheathing_the_weapon_ends_the_combat_idle() {
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
            .add_systems(Update, (sheathe, tick_combat_idle, bevy::ecs::schedule::ApplyDeferred).chain());
        let bones: Vec<Entity> = rig.skeleton.bones.iter().map(|_| app.world_mut().spawn((Transform::default(), GlobalTransform::default())).id()).collect();
        let local = app
            .world_mut()
            .spawn((
                Animator::new(rig.clone(), bones),
                Locomotion::new(set),
                Motion::default(),
                LocalPlayer { obj_id: 7 },
                Drawn { until: 0.0, out: Out::Blade { dual: false }, sheathing: false },
                CombatIdle::at(0.0),
                Visibility::default(),
            ))
            .id();
        app.update();
        let drawn = app.world().get::<Drawn>(local).expect("the weapon state");
        assert!(drawn.sheathing, "the weapon is put away");
        assert!(app.world().get::<CombatIdle>(local).is_none(), "the weapon away ends the combat idle");
        // a dead unit's idle goes too (`FUN_39362690` / `FUN_39362c70` zero `+0x5fc0`)
        app.world_mut().entity_mut(local).insert((CombatIdle::at(0.0), Dead { at: 0.0 }));
        app.update();
        assert!(app.world().get::<CombatIdle>(local).is_none(), "a dead unit has no combat idle");
    }

    /// A unit in the combat idle with no weapon in the hand stands in its fist combat idle (`fist_ba_combat_idle`, the K2 state
    /// before the first press); without the idle it stands in the peaceful one. A weapon in the hand keeps its own stance.
    #[test]
    fn a_fighting_unit_without_a_weapon_stands_in_the_fist_combat_idle() {
        let Ok(rig) = shared_rig(SKELETON) else { return };
        let Ok(cal) = Cal::shared(CAL) else { return };
        let clips = ClipLibrary::default();
        let Some(set) = ClipSet::load(&rig, cal, &clips) else { return };
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).insert_resource(clips).add_systems(Update, crate::ingame::anim::drive_locomotion);
        let bones: Vec<Entity> = rig.skeleton.bones.iter().map(|_| app.world_mut().spawn((Transform::default(), GlobalTransform::default())).id()).collect();
        let local = app
            .world_mut()
            .spawn((Animator::new(rig.clone(), bones), Locomotion::new(set), Motion::default(), LocalPlayer { obj_id: 7 }, Visibility::default()))
            .id();
        let idle = |app: &App| app.world().get::<Animator>(local).and_then(|a| a.current.as_ref()).map_or_else(String::new, |c| format!("{} {}", c.clip.clip.name, c.clip.source.as_deref().unwrap_or("")));
        // the spawn puts the animator in its first idle; `Locomotion` starts at `Idle`, so ask it to pick the clip
        app.world_mut().get_mut::<Locomotion>(local).unwrap().invalidate();
        app.update();
        assert!(idle(&app).contains("fist_ba_relaxed_idle"), "at peace: {}", idle(&app));
        assert_eq!(app.world().get::<Locomotion>(local).unwrap().stance(), None);
        app.world_mut().entity_mut(local).insert(CombatIdle::at(0.0));
        app.world_mut().get_mut::<Locomotion>(local).unwrap().invalidate();
        app.update();
        // the model's `.cal` names the fist combat idle after the one handed clip on this rig; the idle is what the client picks
        assert_eq!(app.world().get::<Locomotion>(local).unwrap().stance(), Some(crate::ingame::anim::Stance::Fist));
        assert!(idle(&app).contains("_ba_combat_idle"), "in the combat idle: {}", idle(&app));
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
        assert_eq!(cancel_of(none, Phase::Cast), Some(Cancel { on_move: true, on_turn: false, in_water: false }));
        assert_eq!(cancel_of(none, Phase::Channel), None);
        // Telekinesis: moving ends the cast and the channel
        let channel = CancelRules { channel_on_move: true, ..none };
        assert_eq!(cancel_of(channel, Phase::Cast), Some(Cancel { on_move: true, on_turn: false, in_water: false }));
        assert_eq!(cancel_of(channel, Phase::Channel), Some(Cancel { on_move: true, on_turn: false, in_water: false }));
        let turn = CancelRules { on_turn: true, ..none };
        assert_eq!(cancel_of(turn, Phase::Cast), Some(Cancel { on_move: true, on_turn: true, in_water: false }));
        assert_eq!(cancel_of(turn, Phase::Channel), None);
        // a skill that cannot be used swimming ends in the water, even a channel that moving leaves alone
        let land = CancelRules { not_swim: true, ..none };
        assert_eq!(cancel_of(land, Phase::Channel), Some(Cancel { on_move: false, on_turn: false, in_water: true }));
        assert_eq!(cancel_of(land, Phase::Cast), Some(Cancel { on_move: true, on_turn: false, in_water: true }));
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
