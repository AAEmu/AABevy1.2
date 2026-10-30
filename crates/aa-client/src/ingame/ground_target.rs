//! Ground targeting: the skills that are aimed at a spot of the ground (Meteor Strike, Missile Rain, thrown bombs, shovels).
//!
//! ## How the 1.2 data marks them
//!
//! `skills.target_type_id` is the server's `SkillTargetType` (the same numbers the client reads: `0` self, `4` hostile, `6`
//! position ...), `target_selection_id` its `SkillTargetSelection` (`1` source, `2` target, `4` location):
//!
//! | `target_type_id` | selection | what the client does here |
//! |---|---|---|
//! | `6` Pos, `11` BallisticPos, `15` ArtilleryPos, `13` RelativePos | any but 1 | **ground reticle**: the decal follows the cursor, a left click casts at that spot (`CSStartSkill`, position target) |
//! | `6` Pos | `1` source | **self position**: the area is centred on the caster, no reticle, cast at once with the caster's own position |
//! | `0` self, `4`, `5` ... `12` SummonPos, `14` SourcePos, `7` Line | | not ground targeted: the plain unit/self cast (`Command::UseSkill`); the server's `initial_target` takes the caster's spot for these |
//!
//! The reticle's size is `skills.target_area_radius` (`target_decal_radius` when a skill has one: seven rows do); the range is
//! `min_range ..= max_range` (3D distance from the caster's feet less the body, as the server's `distance_to(.., include_z)`).
//! The server also checks `target_angle` (the point must lie in front of the caster, `skill_outof_angle`), so the caster is turned
//! towards the spot with a standing movement packet right before the cast.
//!
//! The client's own strings (`x2game.dll`): `InitGroundTargetDecal`, the decal materials `materials/Decals/target/target_correct`
//! (green, the spot is good) and `target_null` (red, tinted copy of the same ring texture `target_correct_df.dds`), and the option
//! `OPTION_ITEM_SMART_GROUND_TARGETING` ("Skill Deployment Location": *activates skills at the location of the mouse cursor when
//! their shortcut key is pressed*; clicking the slot with the mouse still asks for a spot). The decal is the red ring while the spot
//! is out of range.
//!
//! ## Flow
//!
//! The HUD's slot use (`hud/panels.rs`) sends a [`SkillPressed`] instead of the command. [`route`] sends the plain command for a
//! skill without a spot, and for a ground skill it
//! * with the smart option on and the key used (the cursor is not over the HUD): casts at the cursor's spot at once (when the cursor
//!   is not over the ground, at the current target's spot),
//! * else starts aiming ([`Aiming`]): [`reticle`] draws the decal under the cursor, [`aim_input`] takes the left click (cast), the
//!   right click or `Escape` (cancel), and swallows the click so it does not also select the unit under the cursor.
//!
//! Dev flags (seconds count from entering the game): `--ground-press SKILL T` (as the key), `--aim X Y T` (the cursor, logical
//! window pixels), `--aim-world X Y T` (the cursor held on a ground point, game coordinates), `--click T`, `--ground-cancel T`, `--ground-smart 0|1` (the option), `--bar-skill SLOT SKILL` (puts a skill on
//! the action bar's 1-based global slot SLOT, for `--hud-key slotN` on page 1).

use std::collections::HashMap;

use aa_net::skill::target_kind;
use aa_net::state::{Target, Units};
use aa_net::{Command, Event, MoveCommand, NetCommand, NetEvent, Pos};
use aa_world::{TerrainQuery, WaterQuery, WorldCollision, bevy_to_cry, cry_to_bevy};
use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::window::PrimaryWindow;

mod plant;

use super::player::{LocalPlayer, Mover};
use crate::Screen;
use crate::hud::{HudPointerOverUi, InFlight};

/// The ring texture of both decal materials.
const TEXTURE: &str = "textures/decal/target/target_correct_df.dds";
/// Fraction of the quad the ring's outer edge spans (the corner crosses reach out to 0.9).
const INK: f32 = 0.74;
/// Metres over the ground, and how far above the spot a vertex still finds the floor it follows.
const LIFT: f32 = 0.25;
const REACH_UP: f32 = 3.0;
/// A click that moves more than this many pixels between press and release is a camera drag.
const CLICK_SLOP: f32 = 6.0;
/// The body of the caster the range is measured from (the server subtracts the model size), and the slack the client allows over
/// the red ring before it refuses to send (`hud::usable::SEND_SLACK` is 2 m for units).
const BODY: f32 = 0.5;
const SEND_SLACK: f32 = 1.0;
/// The longest cursor ray, metres.
const REACH: f32 = 400.0;

/// How a skill uses a spot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The player picks the spot (reticle).
    Ground,
    /// The area is centred on the caster: the caster's own spot, at once.
    SelfPos,
}

/// The `skills` row of a ground skill.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroundSkill {
    pub mode: Mode,
    /// Area radius of the decal (m).
    pub radius: f32,
    pub min_range: f32,
    pub max_range: f32,
}

/// Which mode a `(target_type_id, target_selection_id)` pair is, if any.
pub fn classify(target_type: i64, selection: i64) -> Option<Mode> {
    match (target_type, selection) {
        (6, 1) => Some(Mode::SelfPos),
        (6 | 11 | 13 | 15, _) => Some(Mode::Ground),
        _ => None,
    }
}

/// The ground skills of the database (`skills` rows whose target type takes a position).
#[derive(Resource, Default)]
pub struct GroundSkills(pub HashMap<u32, GroundSkill>);

fn load_skills() -> Option<HashMap<u32, GroundSkill>> {
    use rusqlite::types::ValueRef;
    let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
    let conn = rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI).ok()?;
    let mut q = conn
        .prepare("SELECT id, target_type_id, target_selection_id, target_area_radius, target_decal_radius, min_range, max_range FROM skills WHERE target_type_id IN (6, 11, 13, 15)")
        .ok()?;
    let num = |r: &rusqlite::Row, i: usize| -> f32 {
        match r.get_ref(i) {
            Ok(ValueRef::Integer(v)) => v as f32,
            Ok(ValueRef::Real(v)) => v as f32,
            Ok(ValueRef::Text(t)) => std::str::from_utf8(t).ok().and_then(|s| s.parse().ok()).unwrap_or(0.0),
            _ => 0.0,
        }
    };
    let rows = q
        .query_map([], |r| {
            let id = num(r, 0) as u32;
            let (ty, sel) = (num(r, 1) as i64, num(r, 2) as i64);
            let (area, decal) = (num(r, 3), num(r, 4));
            Ok(classify(ty, sel).map(|mode| (id, GroundSkill { mode, radius: if decal > 0.0 { decal } else { area }, min_range: num(r, 5), max_range: num(r, 6) })))
        })
        .ok()?;
    Some(rows.flatten().flatten().collect())
}

/// The player used a skill (or an item with a skill) from the action bar or the bag. Sent by the HUD instead of the command; [`route`]
/// sends the command itself for what has no spot.
#[derive(Message, Clone, Copy, Debug)]
pub struct SkillPressed {
    pub skill: u32,
    /// `(item id, template id)` when an item's skill is used.
    pub item: Option<(u64, u32)>,
    /// The unit target of the plain command (0: none).
    pub target_obj_id: u32,
    /// The skill waits for the global cooldown: once sent, the next press of such a skill waits for the server's answer
    /// ([`InFlight`]).
    pub gcd: bool,
}
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ItemUseRouting;
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct AimInput;

impl SkillPressed {
    /// The command a skill without a spot sends.
    fn plain(&self) -> Command {
        match self.item {
            Some((item_id, template_id)) => Command::UseItem { item_id, template_id, skill_id: self.skill, target_obj_id: self.target_obj_id },
            None => Command::UseSkill { skill_id: self.skill, target_obj_id: self.target_obj_id },
        }
    }

    fn at(&self, pos: Pos) -> Command {
        self.at_rot(pos, 0.0)
    }

    fn at_rot(&self, pos: Pos, rot: f32) -> Command {
        match self.item {
            Some((item_id, template_id)) => Command::UseItemAt {
                item_id,
                template_id,
                skill_id: self.skill,
                pos,
                rot,
            },
            None => Command::UseSkillAt {
                skill_id: self.skill,
                pos,
            },
        }
    }
}

/// What the player is aiming.
struct Aim {
    press: SkillPressed,
    skill: GroundSkill,
    /// The spot under the cursor (game coordinates) and whether the caster is within range of it.
    spot: Option<(Vec3, bool)>,
    /// The item that places a doodad is aimed, not a skill ([`plant`]).
    plant: Option<plant::Plant>,
}

#[derive(Resource, Default)]
pub struct Aiming(Option<Aim>);
impl Aiming { pub(crate) fn cancel(&mut self) -> bool { self.0.take().is_some() } }

/// `--ground-press` and friends.
#[derive(Resource, Default)]
struct Script {
    press: Vec<(f32, u32)>,
    aim: Vec<(f32, Vec2)>,
    /// `--aim-world X Y T`: from T on the cursor is held on this ground point (game coordinates).
    aim_world: Vec<(f32, Vec2)>,
    held: Option<Vec2>,
    click: Vec<f32>,
    cancel: Vec<f32>,
    bar: Vec<(u8, u32)>,
    smart: Option<bool>,
    /// The scripted cursor, once `--aim` set it.
    cursor: Option<Vec2>,
    /// A scripted click / cancel waiting for [`aim_input`].
    clicked: bool,
    cancelled: bool,
    /// Seconds since the game screen came up.
    started: Option<f32>,
}

impl Script {
    fn from_args() -> Script {
        let args: Vec<String> = aa_dev::args();
        let mut s = Script::default();
        let f = |i: usize| args.get(i).and_then(|v| v.parse::<f32>().ok());
        for (i, a) in args.iter().enumerate() {
            match a.as_str() {
                "--ground-press" => {
                    if let (Some(k), Some(t)) = (f(i + 1), f(i + 2)) {
                        s.press.push((t, k as u32));
                    }
                }
                "--aim" => {
                    if let (Some(x), Some(y), Some(t)) = (f(i + 1), f(i + 2), f(i + 3)) {
                        s.aim.push((t, Vec2::new(x, y)));
                    }
                }
                "--aim-world" => {
                    if let (Some(x), Some(y), Some(t)) = (f(i + 1), f(i + 2), f(i + 3)) {
                        s.aim_world.push((t, Vec2::new(x, y)));
                    }
                }
                "--click" if args.get(i + 1).is_some_and(|v| v.parse::<f32>().is_ok()) && args.get(i + 2).is_none_or(|v| v.starts_with("--")) => {
                    if let Some(t) = f(i + 1) {
                        s.click.push(t);
                    }
                }
                "--ground-cancel" => {
                    if let Some(t) = f(i + 1) {
                        s.cancel.push(t);
                    }
                }
                "--ground-smart" => s.smart = args.get(i + 1).map(|v| v != "0"),
                "--bar-skill" => {
                    if let (Some(slot), Some(k)) = (f(i + 1), f(i + 2)) {
                        s.bar.push((slot as u8, k as u32));
                    }
                }
                _ => {}
            }
        }
        s
    }
}

#[derive(Component)]
struct Reticle;

#[derive(Resource, Default)]
struct ReticleAssets {
    mesh: Option<Handle<Mesh>>,
    good: Option<Handle<StandardMaterial>>,
    bad: Option<Handle<StandardMaterial>>,
}

pub fn plugin(app: &mut App) {
    app.add_message::<SkillPressed>()
        .add_message::<crate::hud::PortalBookUsed>()
        .add_message::<crate::hud::MusicItemUsed>()
        .add_message::<crate::hud::UseFailed>()
        .add_message::<crate::hud::ShowError>()
        .init_resource::<Aiming>()
        .init_resource::<ReticleAssets>()
        .init_resource::<InFlight>()
        .insert_resource(Script::from_args())
        .insert_resource(GroundSkills(load_skills().unwrap_or_default()))
        .add_systems(PreUpdate, aim_input.in_set(AimInput).after(bevy::input::InputSystems).run_if(in_state(Screen::InGame)))
        .add_systems(Update, (dev_script, route.in_set(ItemUseRouting).after(crate::hud::HudItemUse)).chain().run_if(in_state(Screen::InGame)))
        .add_systems(Update, (track_cursor, plant::validate, reticle, plant::ghost, log_server).chain().after(crate::hud::PointerFinished).run_if(in_state(Screen::InGame)))
        .add_systems(OnExit(Screen::InGame), leave);
}

fn leave(mut commands: Commands, mut aiming: ResMut<Aiming>, reticles: Query<Entity, With<Reticle>>, mut script: ResMut<Script>) {
    aiming.0 = None;
    script.started = None;
    for e in &reticles {
        commands.entity(e).despawn();
    }
}

/// The cursor, in the window's logical pixels: the scripted one, else the mouse.
fn cursor_of(window: Option<&Window>, script: &Script) -> Option<Vec2> {
    script.cursor.or_else(|| window.and_then(|w| w.cursor_position()))
}

/// The first spot of the ground (or of an object) under the view ray through `cursor`, in game coordinates.
fn spot_under(camera: &Camera, transform: &GlobalTransform, cursor: Vec2, collision: Option<&WorldCollision>, terrain: Option<&TerrainQuery>) -> Option<Vec3> {
    let ray = camera.viewport_to_world(transform, cursor).ok()?;
    let origin = bevy_to_cry(ray.origin);
    let dir = bevy_to_cry(*ray.direction);
    if let Some(hit) = collision.and_then(|c| c.raycast(origin, dir, REACH)) {
        return Some(hit.point);
    }
    // no brushes loaded: march over the terrain heights
    let terrain = terrain?;
    let (mut prev, mut t) = (0.0f32, 0.0f32);
    while t < REACH {
        t += 2.0;
        let p = origin + dir * t;
        let Some(h) = terrain.height_at(p.x, p.y) else { continue };
        if p.z <= h {
            let (mut lo, mut hi) = (prev, t);
            for _ in 0..12 {
                let mid = 0.5 * (lo + hi);
                let q = origin + dir * mid;
                if terrain.height_at(q.x, q.y).is_some_and(|g| q.z <= g) {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
            let q = origin + dir * hi;
            return Some(Vec3::new(q.x, q.y, terrain.height_at(q.x, q.y).unwrap_or(q.z)));
        }
        prev = t;
    }
    None
}

/// Whether the caster at `from` reaches `spot`: the server's 3D distance less the body must lie within `min..=max` (`slack` over).
fn in_range(skill: &GroundSkill, from: Vec3, spot: Vec3, slack: f32) -> bool {
    let d = (from.distance(spot) - BODY).max(0.0);
    d >= skill.min_range && (skill.max_range <= 0.0 || d <= skill.max_range + slack)
}

/// The skill's yaw to face `spot` from `from` (0 north, counter-clockwise).
fn yaw_to(from: Vec3, spot: Vec3) -> Option<f32> {
    let d = spot - from;
    (d.truncate().length() > 0.05).then(|| (-d.x).atan2(d.y))
}

/// A skill that waits for the global cooldown went out: the HUD holds back the next one until the server has answered.
fn sent(flight: &mut InFlight, press: &SkillPressed) {
    if press.gcd {
        flight.sent(web_time::Instant::now());
    }
}

/// Turns the caster to the spot (a standing movement packet carries the facing: the server checks `target_angle` against it)
/// and asks for the skill there.
fn cast_at(press: &SkillPressed, spot: Vec3, mover: &mut Mover, net: &mut MessageWriter<NetCommand>, flight: &mut InFlight) {
    if let Some(yaw) = yaw_to(mover.pos, spot) {
        mover.facing = yaw;
    }
    let me = mover.pos;
    net.write(NetCommand(Command::Move(MoveCommand::stopped(Pos::new(me.x, me.y, me.z), mover.facing))));
    net.write(NetCommand(press.at(Pos::new(spot.x, spot.y, spot.z))));
    sent(flight, press);
    info!("ground: cast skill {} at ({:.1}, {:.1}, {:.1}), {:.1} m away, facing {:.2}", press.skill, spot.x, spot.y, spot.z, me.distance(spot), mover.facing);
}

/// Dev flags, on the clock of the game screen.
fn dev_script(
    time: Res<Time>,
    mut script: ResMut<Script>,
    mut pressed: MessageWriter<SkillPressed>,
    mut net: MessageWriter<NetCommand>,
    player: Query<(), With<LocalPlayer>>,
    cameras: Query<(&Camera, &GlobalTransform), (With<Camera3d>, Without<crate::hud::ModelViewCamera>)>,
    terrain: Option<Res<TerrainQuery>>,
) {
    if player.is_empty() {
        return;
    }
    let now = time.elapsed_secs();
    let start = *script.started.get_or_insert(now);
    let t = now - start;
    if !script.bar.is_empty() {
        for (slot, skill) in std::mem::take(&mut script.bar) {
            net.write(NetCommand(Command::SetActionSlot { slot, action: aa_net::items::ActionSlot { kind: aa_net::items::ActionKind::Spell, id: u64::from(skill) } }));
        }
    }
    while let Some(i) = script.press.iter().position(|(at, _)| *at <= t) {
        let (_, skill) = script.press.remove(i);
        info!("ground: scripted press of skill {skill} at {t:.1} s");
        pressed.write(SkillPressed { skill, item: None, target_obj_id: 0, gcd: false });
    }
    while let Some(i) = script.aim.iter().position(|(at, _)| *at <= t) {
        let (_, p) = script.aim.remove(i);
        script.cursor = Some(p);
        script.held = None;
    }
    while let Some(i) = script.aim_world.iter().position(|(at, _)| *at <= t) {
        let (_, p) = script.aim_world.remove(i);
        script.held = Some(p);
    }
    // the cursor follows a held ground point through the camera
    if let (Some(p), Some((camera, transform))) = (script.held, cameras.iter().find(|(c, _)| c.is_active)) {
        let z = terrain.as_ref().and_then(|t| t.height_at(p.x, p.y)).unwrap_or(0.0);
        script.cursor = camera.world_to_viewport(transform, cry_to_bevy(Vec3::new(p.x, p.y, z))).ok();
    }
    while let Some(i) = script.click.iter().position(|at| *at <= t) {
        script.click.remove(i);
        script.clicked = true;
    }
    while let Some(i) = script.cancel.iter().position(|at| *at <= t) {
        script.cancel.remove(i);
        script.cancelled = true;
    }
}

/// The smart ground targeting option (`OPTION_ITEM_SMART_GROUND_TARGETING`).
fn smart_option(rt: Option<&aa_ui::UiRuntime>, script: &Script) -> bool {
    script.smart.unwrap_or_else(|| {
        rt.and_then(|rt| rt.lua.load("return X2Option:GetOptionItemValue(OPTION_ITEM_SMART_GROUND_TARGETING)").eval::<Option<f32>>().ok().flatten()).is_some_and(|v| v != 0.0)
    })
}

/// The slot use of the HUD: plain skills go out as before, ground skills aim or cast at the cursor.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn route(
    mut pressed: MessageReader<SkillPressed>,
    skills: Res<GroundSkills>,
    mut aiming: ResMut<Aiming>,
    over_ui: Res<HudPointerOverUi>,
    script: Res<Script>,
    rt: Option<NonSend<aa_ui::UiRuntime>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform), (With<Camera3d>, Without<crate::hud::ModelViewCamera>)>,
    collision: Option<Res<WorldCollision>>,
    terrain: Option<Res<TerrainQuery>>,
    target: Res<Target>,
    units: Res<Units>,
    mut player: Query<&mut Mover, With<LocalPlayer>>,
    mut net: MessageWriter<NetCommand>,
    mut flight: ResMut<InFlight>,
    (
        keys,
        mut book,
        mut music,
        emotes,
        db,
        water,
        slaves,
        summon_world,
        (bridge, native_skills, buffs, inventory, mut failed, mut refused),
    ): (
        Res<ButtonInput<KeyCode>>,
        MessageWriter<crate::hud::PortalBookUsed>,
        MessageWriter<crate::hud::MusicItemUsed>,
        Option<Res<super::emote::Emotes>>,
        Option<Res<super::data::Db>>,
        Option<Res<WaterQuery>>,
        Option<Res<aa_net::Slaves>>,
        Option<Res<crate::hud::summon::SummonWorld>>,
        (
            Option<NonSend<crate::hud::api::HudBridge>>,
            Option<Res<aa_net::state::Skills>>,
            Option<Res<aa_net::state::Buffs>>,
            Option<Res<aa_net::state::Inventory>>,
            MessageWriter<crate::hud::UseFailed>,
            MessageWriter<crate::hud::ShowError>,
        ),
    ),
) {
    for press in pressed.read() {
        if let Some((_, template)) = press.item
            && let Some(slave) = bridge.as_deref().and_then(|bridge| {
                let data = bridge.0.borrow();
                data.game.get()?.summon_slaves.get(&template).cloned()
            })
        {
            let Ok(mover) = player.single() else { continue };
            let world;
            let space: &dyn crate::hud::summon::SummonSpace = match summon_world.as_deref() {
                Some(world) => &*world.0,
                None => {
                    world = crate::hud::summon::WorldSummon {
                        terrain: terrain.as_deref().cloned(),
                        water: water.as_deref().cloned(),
                        collision: collision.as_deref().cloned(),
                    };
                    &world
                }
            };
            match crate::hud::summon::press(
                &slave,
                slaves.as_deref().and_then(|s| s.position.as_ref()),
                mover.pos,
                mover.facing,
                space,
            ) {
                crate::hud::summon::Press::Send(Some(spot)) => {
                    let rot = mover.facing + std::f32::consts::FRAC_PI_2;
                    net.write(NetCommand(
                        press.at_rot(Pos::new(spot.x, spot.y, spot.z), rot),
                    ));
                    sent(&mut flight, press);
                    info!(
                        "summon: item {template} at ({:.1}, {:.1}, {:.1}), rot {rot:.4}",
                        spot.x, spot.y, spot.z
                    );
                    continue;
                }
                // no terrain/collision query: the search cannot judge, so keep the plain item cast
                crate::hud::summon::Press::Send(None) => {}
                crate::hud::summon::Press::Despawn(obj_id) => {
                    net.write(NetCommand(Command::Slave(
                        aa_net::slave_control::SlaveCommand::Despawn { obj_id },
                    )));
                    info!("summon: the item {template} cancels the owned slave {obj_id}");
                    continue;
                }
                crate::hud::summon::Press::Refused(code) => {
                    info!("summon: item {template} refused with error {code}: not sent");
                    refused.write(crate::hud::ShowError(code));
                    continue;
                }
            }
        }
        if let Some((item_id,template))=press.item {
            if matches!((template,press.skill),(29880,23741)|(29704,23457)) {
                if let (Some(rt),Some(bridge),Some(skills),Some(buffs),Some(inventory)) =
                    (rt.as_deref(),bridge.as_deref(),native_skills.as_deref(),buffs.as_deref(),inventory.as_deref()) {
                    if let Some(fail) = crate::hud::use_look_source(rt,&bridge.0,crate::hud::ItemLookSource { item_id,template,skill:press.skill },(skills,buffs,inventory,&flight),Some(&mut aiming)) {
                        failed.write(crate::hud::UseFailed { skill:press.skill,result:fail.result,value:fail.value,server:false });
                    }
                }
                continue;
            }
        }
        // Native394eaac0 opens paper locally or looks up a stored score; neither casts here.
        if let Some((item_id, 27902 | 28051)) = press.item {
            music.write(crate::hud::MusicItemUsed(item_id));
            continue;
        }
        // the emote skills of the skill book (`/sad`, `/clap` ...) are expressed emotions: the server announces them to everyone in range
        if press.item.is_none()
            && let Some(emotion_id) = emotes.as_ref().and_then(|e| e.skill(press.skill))
        {
            net.write(NetCommand(Command::ExpressEmotion { emotion_id, target_obj_id: press.target_obj_id }));
            continue;
        }
        // the Teleport Book opens the book; its skill is cast from there (`X2Warp:OpenPortal`)
        if press.item.is_some() && press.skill == crate::hud::OPEN_PORTAL_SKILL {
            let record = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
            book.write(crate::hud::PortalBookUsed { item: press.item, record });
            continue;
        }
        // a house design enters the builder (`housing::placement`), it is not cast
        if press.item.is_some_and(|(_, template)| super::housing::claims_item(template)) {
            continue;
        }
        // seeds, saplings and the other items that place a doodad are aimed at the ground (`plant.rs`); the same item again lets go
        if let Some((skill, plant)) = press.item.and(db.as_deref()).and_then(|db| plant::begin(press, db)) {
            crate::hud::cancel_look_target(rt.as_deref(),bridge.as_deref());
            let again = aiming.0.as_ref().is_some_and(|a| a.press.item == press.item);
            aiming.0 = (!again).then_some(Aim { press: *press, skill, spot: None, plant: Some(plant) });
            continue;
        }
        let Some(skill) = skills.0.get(&press.skill).copied() else {
            net.write(NetCommand(press.plain()));
            sent(&mut flight, press);
            continue;
        };
        let Ok(mut mover) = player.single_mut() else { continue };
        if skill.mode == Mode::SelfPos {
            let me = mover.pos;
            net.write(NetCommand(press.at(Pos::new(me.x, me.y, me.z))));
            sent(&mut flight, press);
            info!("ground: skill {} is centred on the caster: cast at ({:.1}, {:.1}, {:.1})", press.skill, me.x, me.y, me.z);
            continue;
        }
        let cursor = cursor_of(windows.single().ok(), &script);
        let spot_at_cursor = || {
            let (camera, transform) = cameras.iter().find(|(c, _)| c.is_active)?;
            spot_under(camera, transform, cursor?, collision.as_deref(), terrain.as_deref())
        };
        // the same key again while aiming: cast where the cursor is
        let again = aiming.0.as_ref().is_some_and(|a| a.press.skill == press.skill && a.press.item == press.item);
        let key = !over_ui.0;
        if key && (again || smart_option(rt.as_deref(), &script)) {
            let spot = spot_at_cursor().or_else(|| target.obj_id.and_then(|id| units.get(id)).map(|u| Vec3::new(u.pos.x, u.pos.y, u.pos.z)));
            if let Some(spot) = spot {
                if in_range(&skill, mover.pos, spot, SEND_SLACK) {
                    cast_at(press, spot, &mut mover, &mut net, &mut flight);
                } else {
                    info!("ground: skill {} is out of range of ({:.1}, {:.1}, {:.1}): not sent", press.skill, spot.x, spot.y, spot.z);
                }
                aiming.0 = None;
                continue;
            }
        }
        if again {
            aiming.0 = None;
            continue;
        }
        info!("ground: aiming skill {} (radius {:.1} m, range {:.0}..{:.0} m)", press.skill, skill.radius, skill.min_range, skill.max_range);
        crate::hud::cancel_look_target(rt.as_deref(),bridge.as_deref());
        aiming.0 = Some(Aim { press: *press, skill, spot: None, plant: None });
    }
}

/// Keeps the spot under the cursor and whether it is in range.
fn track_cursor(
    mut aiming: ResMut<Aiming>,
    over_ui: Res<HudPointerOverUi>,
    script: Res<Script>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform), (With<Camera3d>, Without<crate::hud::ModelViewCamera>)>,
    collision: Option<Res<WorldCollision>>,
    terrain: Option<Res<TerrainQuery>>,
    player: Query<&Mover, With<LocalPlayer>>,
    mut last: Local<Option<Vec3>>,
) {
    let Some(aim) = aiming.0.as_mut() else {
        *last = None;
        return;
    };
    let (Ok(mover), Some((camera, transform))) = (player.single(), cameras.iter().find(|(c, _)| c.is_active)) else { return };
    let cursor = if over_ui.0 && script.cursor.is_none() { None } else { cursor_of(windows.single().ok(), &script) };
    aim.spot = cursor
        .and_then(|c| spot_under(camera, transform, c, collision.as_deref(), terrain.as_deref()))
        .map(|spot| (spot, in_range(&aim.skill, mover.pos, spot, 0.0)));
    if let Some((spot, ok)) = aim.spot
        && last.is_none_or(|l| l.distance(spot) > 0.5)
    {
        *last = Some(spot);
        info!(
            "ground: cursor {:?}: spot ({:.1}, {:.1}, {:.1}), {:.1} m from the caster at ({:.1}, {:.1}, {:.1}): {}",
            cursor,
            spot.x,
            spot.y,
            spot.z,
            mover.pos.distance(spot),
            mover.pos.x,
            mover.pos.y,
            mover.pos.z,
            if ok { "in range" } else { "out of range" }
        );
    }
}

/// The left click casts at the spot; a right click or explicit dev-script cancel stops aiming.
/// Configured open_config/Escape cancellation belongs to the shared HUD modal route.
#[allow(clippy::too_many_arguments)]
fn aim_input(
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    over_ui: Res<HudPointerOverUi>,
    mut script: ResMut<Script>,
    mut aiming: ResMut<Aiming>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut player: Query<&mut Mover, With<LocalPlayer>>,
    mut net: MessageWriter<NetCommand>,
    mut flight: ResMut<InFlight>,
    mut pressed_at: Local<[Option<Vec2>; 2]>,
) {
    let (clicked, cancelled) = (std::mem::take(&mut script.clicked), std::mem::take(&mut script.cancelled));
    let Some(aim) = aiming.0.as_ref() else {
        *pressed_at = [None; 2];
        return;
    };
    let cursor = cursor_of(windows.single().ok(), &script);
    let mut cast = clicked;
    let mut cancel = cancelled;
    for (i, button) in [MouseButton::Left, MouseButton::Right].into_iter().enumerate() {
        if mouse.just_pressed(button) {
            pressed_at[i] = if over_ui.0 { None } else { cursor };
            if i == 0 && !over_ui.0 {
                mouse.clear_just_pressed(button);
            }
        }
        if mouse.just_released(button) {
            let start = pressed_at[i].take();
            if let (Some(start), Some(end)) = (start, cursor)
                && start.distance(end) <= CLICK_SLOP
                && !over_ui.0
            {
                if i == 0 { cast = true } else { cancel = true }
            }
            if i == 0 && start.is_some() {
                mouse.clear_just_released(button);
            }
        }
    }
    if cancel {
        info!("ground: aiming skill {} cancelled", aim.press.skill);
        aiming.0 = None;
        return;
    }
    if !cast {
        return;
    }
    let Ok(mut mover) = player.single_mut() else { return };
    if let Some(plant) = aim.plant.as_ref() {
        // an item that places a doodad: the packet instead of a cast, the aiming goes on while the spot is not free
        match aim.spot {
            Some((spot, true)) if plant::place(plant, spot, &mover, &mut net) => aiming.0 = None,
            Some(_) => info!("plant: that spot cannot be used"),
            None => info!("plant: no ground under the cursor"),
        }
        return;
    }
    match aim.spot {
        Some((spot, ok)) if ok || in_range(&aim.skill, mover.pos, spot, SEND_SLACK) => {
            let press = aim.press;
            cast_at(&press, spot, &mut mover, &mut net, &mut flight);
            aiming.0 = None;
        }
        Some((spot, _)) => info!("ground: ({:.1}, {:.1}, {:.1}) is out of range of skill {}: not cast, still aiming", spot.x, spot.y, spot.z, aim.press.skill),
        None => info!("ground: no ground under the cursor: not cast"),
    }
}

fn dds_image(path: &str) -> Option<Image> {
    let bytes = aa_cry::read(path).ok()?;
    let dds = aa_cry::texture::Dds::parse(&bytes).ok()?;
    let (w, h) = (dds.width, dds.height);
    let mut image = Image::new(Extent3d { width: w, height: h, depth_or_array_layers: 1 }, TextureDimension::D2, dds.decode_rgba8(0, 0), TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::RENDER_WORLD);
    image.sampler = bevy::image::ImageSampler::linear();
    Some(image)
}

/// A decal material: the ring texture tinted, unlit, blended, drawn from both sides and pulled towards the camera a little so the
/// terrain under it (a lift of centimetres) cannot cut it.
fn decal_material(image: &Handle<Image>, color: Color) -> StandardMaterial {
    StandardMaterial {
        base_color: color,
        base_color_texture: Some(image.clone()),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        cull_mode: None,
        depth_bias: 12.0,
        ..default()
    }
}

/// The decal under the cursor: the ring of `target_correct` while the spot is in range, of `target_null` out of it. A card of the
/// 3D world (`aa_fx::decal::drape_quad`, like the area skills' ground decals): cells whose corners lie on the floor under them (brushes and
/// terrain, [`WorldCollision::ground_below`]) and on the water surface where there is water, lifted a few centimetres, depth tested
/// and drawn by the world camera, so it needs no HUD camera and hides behind what stands in front of it.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn reticle(
    mut commands: Commands,
    aiming: Res<Aiming>,
    mut assets: ResMut<ReticleAssets>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    collision: Option<Res<WorldCollision>>,
    terrain: Option<Res<TerrainQuery>>,
    water: Option<Res<WaterQuery>>,
    mut reticles: Query<(Entity, &mut Visibility, &mut MeshMaterial3d<StandardMaterial>), With<Reticle>>,
) {
    let shown = aiming.0.as_ref().and_then(|a| a.spot.map(|(spot, ok)| (a.skill, spot, ok)));
    let Some((skill, spot, ok)) = shown else {
        for (_, mut v, _) in &mut reticles {
            if *v != Visibility::Hidden {
                *v = Visibility::Hidden;
            }
        }
        return;
    };
    if assets.good.is_none() {
        let Some(image) = dds_image(TEXTURE) else {
            warn!("ground: cannot read {TEXTURE}: no reticle");
            return;
        };
        let image = images.add(image);
        assets.mesh = Some(meshes.add(Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD)));
        // the decal materials' tints: target_correct Diffuse (0.18, 1, 0.44), target_null (0.74, 0.03, 0.01); the grey ring is lifted
        assets.good = Some(materials.add(decal_material(&image, Color::linear_rgba(0.4, 3.6, 1.4, 1.0))));
        assets.bad = Some(materials.add(decal_material(&image, Color::linear_rgba(3.6, 0.15, 0.05, 1.0))));
    }
    let material = if ok { assets.good.clone().unwrap() } else { assets.bad.clone().unwrap() };
    // the ring's outer edge is the area radius: the card is wider by the ink fraction
    let half = skill.radius.max(0.75) / INK;
    // the floor under a Bevy position: a brush or the terrain, the water surface over it
    let floor = |p: Vec3| -> Option<f32> {
        let (x, y) = (p.x, -p.z);
        let ground = collision
            .as_ref()
            .and_then(|c| c.ground_below(x, y, spot.z + REACH_UP, 0.0))
            .map(|g| g.height)
            .or_else(|| terrain.as_ref().and_then(|t| t.height_at(x, y)))
            .unwrap_or(spot.z)
            .clamp(spot.z - 2.0 * REACH_UP, spot.z + REACH_UP);
        let wet = water.as_ref().and_then(|w| w.water_at(Vec3::new(x, y, ground)).map(|(h, _)| h)).filter(|h| *h > ground);
        Some(wet.unwrap_or(ground))
    };
    // texture up is game north (-z in Bevy)
    let card = aa_fx::decal::drape_quad(cry_to_bevy(spot), Vec3::X * half, Vec3::NEG_Z * half, &floor, LIFT, aa_fx::decal::cells_for(half, half));
    let mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; card.positions.len()])
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, card.positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, card.uvs)
        .with_inserted_indices(Indices::U32(card.indices));
    if let Some(mut m) = meshes.get_mut(assets.mesh.as_ref().unwrap()) {
        *m = mesh;
    }
    match reticles.iter_mut().next() {
        Some((_, mut v, mut m)) => {
            if *v != Visibility::Inherited {
                *v = Visibility::Inherited;
            }
            if m.0 != material {
                m.0 = material;
            }
        }
        None => {
            commands.spawn((
                Name::new("ground target reticle"),
                Reticle,
                Mesh3d(assets.mesh.clone().unwrap()),
                MeshMaterial3d(material),
                Transform::IDENTITY,
                Visibility::Inherited,
                // the card follows the cursor: its bounds change every frame
                NoFrustumCulling,
                NotShadowCaster,
                NotShadowReceiver,
            ));
        }
    }
}

/// What the server made of a spot: its `SkillStarted` / `SkillFired` carry the position target back.
fn log_server(mut events: MessageReader<NetEvent>, skills: Res<GroundSkills>) {
    for NetEvent(event) in events.read() {
        match event {
            Event::SkillStarted(s) if skills.0.contains_key(&s.skill_id) => {
                info!(
                    "ground: server: skill {} {} (result {}), target kind {} at {:?}, cast {} ms",
                    s.skill_id,
                    if s.failed() { "refused" } else { "started" },
                    s.result,
                    s.target.kind,
                    s.target.pos.map(|p| (p.x, p.y, p.z)),
                    s.cast_ms
                );
            }
            Event::SkillFired(s) if skills.0.contains_key(&s.skill_id) && s.target.kind != target_kind::UNIT => {
                info!("ground: server: skill {} fired at {:?}, delay {} ms", s.skill_id, s.target.pos.map(|p| (p.x, p.y, p.z)), s.delay_ms);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_types_pick_the_mode() {
        assert_eq!(classify(6, 2), Some(Mode::Ground), "Meteor Strike, Missile Rain");
        assert_eq!(classify(6, 4), Some(Mode::Ground), "Plant Seed, Dig");
        assert_eq!(classify(11, 2), Some(Mode::Ground));
        assert_eq!(classify(15, 2), Some(Mode::Ground));
        assert_eq!(classify(6, 1), Some(Mode::SelfPos), "centred on the caster");
        for plain in [(0, 1), (4, 2), (5, 2), (7, 4), (12, 4), (14, 2), (16, 2)] {
            assert_eq!(classify(plain.0, plain.1), None, "{plain:?}");
        }
    }

    #[test]
    fn range_is_measured_in_3d_from_the_body() {
        let s = GroundSkill { mode: Mode::Ground, radius: 5.0, min_range: 0.0, max_range: 25.0 };
        let from = Vec3::new(100.0, 100.0, 10.0);
        assert!(in_range(&s, from, Vec3::new(125.0, 100.0, 10.0), 0.0));
        assert!(!in_range(&s, from, Vec3::new(126.0, 100.0, 10.0), 0.0));
        assert!(in_range(&s, from, Vec3::new(126.0, 100.0, 10.0), SEND_SLACK));
        // a spot 25 m away but 10 m higher is further
        assert!(!in_range(&s, from, Vec3::new(125.0, 100.0, 20.0), 0.0));
        let ring = GroundSkill { min_range: 10.0, max_range: 50.0, ..s };
        assert!(!in_range(&ring, from, Vec3::new(105.0, 100.0, 10.0), 0.0), "too close (Missile Rain)");
    }

    #[test]
    fn yaw_is_north_zero_counter_clockwise() {
        let o = Vec3::ZERO;
        assert!((yaw_to(o, Vec3::new(0.0, 5.0, 0.0)).unwrap()).abs() < 1e-6);
        assert!((yaw_to(o, Vec3::new(-5.0, 0.0, 0.0)).unwrap() - std::f32::consts::FRAC_PI_2).abs() < 1e-5, "west is +90 degrees");
        assert!((yaw_to(o, Vec3::new(5.0, 0.0, 0.0)).unwrap() + std::f32::consts::FRAC_PI_2).abs() < 1e-5);
        assert!(yaw_to(o, Vec3::new(0.0, 0.0, 9.0)).is_none());
    }

    #[derive(Resource, Default)]
    struct Sent(Vec<Command>);

    fn collect(mut commands: MessageReader<NetCommand>, mut sent: ResMut<Sent>) {
        sent.0.extend(commands.read().map(|c| c.0.clone()));
    }

    #[test]
    fn an_emote_skill_expresses_its_emotion_at_the_target() {
        let emotes = super::super::emote::Emotes::from_env();
        if emotes.is_empty_for_tests() {
            return;
        }
        let clap = emotes.command("clap").expect("the database has /clap");
        // /clap is skill 14460 of the emote tab
        assert_eq!(emotes.skill(14460), Some(clap));
        let mut app = App::new();
        app.add_message::<SkillPressed>()
            .add_message::<crate::hud::PortalBookUsed>()
            .add_message::<crate::hud::MusicItemUsed>()
            .add_message::<crate::hud::UseFailed>()
            .add_message::<crate::hud::ShowError>()
            .add_message::<NetCommand>()
            .insert_resource(GroundSkills::default())
            .init_resource::<Aiming>()
            .init_resource::<HudPointerOverUi>()
            .init_resource::<InFlight>()
            .init_resource::<Script>()
            .init_resource::<Target>()
            .init_resource::<Units>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<Sent>()
            .insert_resource(emotes)
            .add_systems(Update, (route, collect).chain());
        app.world_mut().write_message(SkillPressed { skill: 14460, item: None, target_obj_id: 77, gcd: true });
        // a skill that is no emote, and an item whose skill is one: plain uses
        app.world_mut().write_message(SkillPressed { skill: 11_001, item: None, target_obj_id: 5, gcd: true });
        app.world_mut().write_message(SkillPressed { skill: 14460, item: Some((9, 3)), target_obj_id: 0, gcd: true });
        app.update();
        let sent = std::mem::take(&mut app.world_mut().resource_mut::<Sent>().0);
        assert!(matches!(sent[0], Command::ExpressEmotion { emotion_id, target_obj_id: 77 } if emotion_id == clap), "{:?}", sent[0]);
        assert!(matches!(sent[1], Command::UseSkill { skill_id: 11_001, target_obj_id: 5 }), "{:?}", sent[1]);
        assert!(matches!(sent[2], Command::UseItem { skill_id: 14460, .. }), "{:?}", sent[2]);
        assert_eq!(sent.len(), 3);
    }

    #[test]
    fn a_sent_skill_that_waits_for_the_global_cooldown_is_in_flight() {
        let mut app = App::new();
        app.add_message::<SkillPressed>()
            .add_message::<crate::hud::PortalBookUsed>()
            .add_message::<crate::hud::MusicItemUsed>()
            .add_message::<crate::hud::UseFailed>()
            .add_message::<crate::hud::ShowError>()
            .add_message::<NetCommand>()
            .insert_resource(GroundSkills::default())
            .init_resource::<Aiming>()
            .init_resource::<HudPointerOverUi>()
            .init_resource::<InFlight>()
            .init_resource::<Script>()
            .init_resource::<Target>()
            .init_resource::<Units>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<Sent>()
            .add_systems(Update, (route, collect).chain());
        let pending = |app: &App| app.world().resource::<InFlight>().pending(&aa_net::state::Skills::default(), web_time::Instant::now());
        // a skill that ignores the global cooldown does not hold the others back
        app.world_mut().write_message(SkillPressed { skill: 11_001, item: None, target_obj_id: 5, gcd: false });
        app.update();
        assert!(!pending(&app));
        app.world_mut().write_message(SkillPressed { skill: 11_002, item: None, target_obj_id: 5, gcd: true });
        app.update();
        assert!(pending(&app));
        assert_eq!(app.world().resource::<Sent>().0.len(), 2);
    }

    #[test]
    fn music_paper_and_sheet_use_local_controller_before_any_cast() {
        let mut app = App::new();
        app.add_message::<SkillPressed>()
            .add_message::<crate::hud::PortalBookUsed>()
            .add_message::<crate::hud::MusicItemUsed>()
            .add_message::<crate::hud::UseFailed>()
            .add_message::<crate::hud::ShowError>()
            .add_message::<NetCommand>()
            .insert_resource(GroundSkills::default())
            .init_resource::<Aiming>()
            .init_resource::<HudPointerOverUi>()
            .init_resource::<InFlight>()
            .init_resource::<Script>()
            .init_resource::<Target>()
            .init_resource::<Units>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<Sent>()
            .add_systems(Update, (route, collect).chain());
        for template in [27902,28051] {
            app.world_mut().write_message(SkillPressed { skill: 22215, item: Some((9_007_199_254_740_993, template)), target_obj_id: 0, gcd: true });
            app.update();
            assert!(app.world().resource::<Sent>().0.is_empty());
            let opened: Vec<_> = app.world_mut().resource_mut::<Messages<crate::hud::MusicItemUsed>>().drain().collect();
            assert!(matches!(opened.as_slice(), [crate::hud::MusicItemUsed(9_007_199_254_740_993)]));
        }
        assert!(!app.world().resource::<InFlight>().pending(&default(), web_time::Instant::now()));
    }

    #[test]
    fn appearance_powders_route_full_ids_before_any_cast_or_in_flight_state() {
        let (shared, rt) = crate::hud::look_test_runtime();
        let mut app = App::new();
        app.add_message::<SkillPressed>()
            .add_message::<crate::hud::PortalBookUsed>()
            .add_message::<crate::hud::MusicItemUsed>()
            .add_message::<crate::hud::UseFailed>()
            .add_message::<crate::hud::ShowError>()
            .add_message::<NetCommand>()
            .insert_resource(GroundSkills::default())
            .init_resource::<Aiming>()
            .init_resource::<HudPointerOverUi>()
            .init_resource::<InFlight>()
            .insert_resource(Script {
                smart: Some(false),
                ..Default::default()
            })
            .init_resource::<Target>()
            .init_resource::<Units>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<Sent>()
            .insert_resource(aa_net::state::Skills {
                obj_id: 100,
                ..Default::default()
            })
            .init_resource::<aa_net::state::Buffs>()
            .init_resource::<aa_net::state::Inventory>()
            .insert_non_send(crate::hud::api::HudBridge(shared.clone()))
            .insert_non_send(rt)
            .add_systems(Update, (route, collect).chain());
        let id = 9_007_199_254_740_997;
        let powder = SkillPressed {
            skill: 23741,
            item: Some((id, 29880)),
            target_obj_id: 5,
            gcd: true,
        };
        app.world_mut().write_message(powder);
        app.update();
        app.world()
            .get_non_send::<aa_ui::UiRuntime>()
            .unwrap()
            .lua
            .load("assert(X2Item:IsInItemSkinizeMode() and X2Cursor:GetCursorInfo()==nil)")
            .exec()
            .unwrap();
        assert!(app.world().resource::<Sent>().0.is_empty());
        assert!(
            !app.world()
                .resource::<InFlight>()
                .pending(&default(), web_time::Instant::now())
        );
        // An unsupported cleanup record is consumed without replacing the admitted target.
        app.world_mut().write_message(SkillPressed {
            skill: 23457,
            item: Some((id, 29704)),
            ..powder
        });
        app.update();
        app.world()
            .get_non_send::<aa_ui::UiRuntime>()
            .unwrap()
            .lua
            .load("assert(X2Item:IsInItemSkinizeMode())")
            .exec()
            .unwrap();
        app.world_mut().spawn((
            LocalPlayer { obj_id: 100 },
            Mover {
                pos: Vec3::ZERO,
                facing: 0.0,
                vz: 0.0,
                grounded: true,
                velocity: Vec3::ZERO,
                swim: None,
                wading: 0.0,
            },
        ));
        app.world_mut().resource_mut::<GroundSkills>().0.insert(
            10664,
            GroundSkill {
                mode: Mode::Ground,
                radius: 5.0,
                min_range: 0.0,
                max_range: 25.0,
            },
        );
        let ground = SkillPressed {
            skill: 10664,
            item: None,
            target_obj_id: 0,
            gcd: true,
        };
        // Input order is preserved in one iteration; a later reticle wins.
        app.world_mut().write_message(powder);
        app.world_mut().write_message(ground);
        app.update();
        assert!(app.world().resource::<Aiming>().0.is_some());
        app.world()
            .get_non_send::<aa_ui::UiRuntime>()
            .unwrap()
            .lua
            .load("assert(not X2Item:IsInItemSkinizeMode())")
            .exec()
            .unwrap();
        // The inverse order leaves only the item target, never two modal owners.
        app.world_mut().write_message(ground);
        app.world_mut().write_message(powder);
        app.update();
        assert!(app.world().resource::<Aiming>().0.is_none());
        app.world()
            .get_non_send::<aa_ui::UiRuntime>()
            .unwrap()
            .lua
            .load("assert(X2Item:IsInItemSkinizeMode())")
            .exec()
            .unwrap();
        app.world_mut().write_message(ground);
        app.update();
        shared.borrow_mut().snapshot.mp = 0;
        app.world_mut().write_message(powder);
        app.update();
        assert!(
            app.world().resource::<Aiming>().0.is_some(),
            "refused source use preserves the old reticle"
        );
        assert!(app.world().resource::<Sent>().0.is_empty());
        assert!(
            !app.world()
                .resource::<InFlight>()
                .pending(&default(), web_time::Instant::now())
        );
        // A template/skill mismatch stays on the established ordinary route.
        app.world_mut().write_message(SkillPressed {
            skill: 23457,
            item: Some((id, 29880)),
            target_obj_id: 5,
            gcd: true,
        });
        app.update();
        assert!(
            matches!(app.world().resource::<Sent>().0.as_slice(),[Command::UseItem{item_id,template_id:29880,skill_id:23457,target_obj_id:5}] if *item_id==id)
        );
    }

    #[test]
    fn the_database_marks_the_player_skills() {
        let Some(skills) = load_skills() else { return };
        if skills.is_empty() {
            return;
        }
        let meteor = skills[&10664];
        assert_eq!((meteor.mode, meteor.radius, meteor.max_range), (Mode::Ground, 5.0, 25.0));
        let rain = skills[&13281];
        assert_eq!((rain.radius, rain.min_range, rain.max_range), (7.0, 10.0, 50.0));
    }

    #[test]
    fn remapped_open_config_cancels_modal_targets_before_menu_and_preserves_focus_and_held_cursor()
    -> Result<(), Box<dyn std::error::Error>> {
        use bevy::input::{
            ButtonState, InputPlugin,
            keyboard::{Key, KeyboardInput},
        };
        let (shared, rt) = crate::hud::look_test_runtime();
        rt.lua.globals().set("NET", rt.lua.create_table()?)?;
        rt.lua.globals().set("X2Hotkey", rt.lua.create_table()?)?;
        let path = crate::settings::isolated_bindings(&rt)?;
        rt.lua.load(r#"
          X2Hotkey:BindingToOption();X2Hotkey:SetOptionBindingWithIndex('open_config','f12',1,0);X2Hotkey:SaveHotKey()
          assert(X2Hotkey:GetBinding('open_config',1)=='f12')
          OPEN_COUNT=0;local w=UIParent:CreateWidget('emptywidget','escapeObserver','UIParent')
          w:RegisterEvent('OPEN_CONFIG');w:SetHandler('OnEvent',function()OPEN_COUNT=OPEN_COUNT+1 end)
          X2Cursor:SetHeld({kind='bag',slot=7,count=1})
        "#).exec()?;
        let mut target = Target::default();
        target.obj_id = Some(77);
        let mut app = App::new();
        app.add_plugins(InputPlugin)
            .add_message::<SkillPressed>()
            .add_message::<crate::hud::PortalBookUsed>()
            .add_message::<crate::hud::MusicItemUsed>()
            .add_message::<crate::hud::UseFailed>()
            .add_message::<crate::hud::ShowError>()
            .add_message::<NetCommand>()
            .insert_resource(GroundSkills::default())
            .init_resource::<Aiming>()
            .init_resource::<HudPointerOverUi>()
            .init_resource::<InFlight>()
            .init_resource::<Script>()
            .init_resource::<crate::hud::HudKeyboardFocus>()
            .init_resource::<crate::hud::TestScriptedKeys>()
            .init_resource::<crate::settings::Actions>()
            .init_resource::<Units>()
            .init_resource::<Sent>()
            .insert_resource(target)
            .insert_resource(aa_net::state::Skills {
                obj_id: 100,
                ..default()
            })
            .init_resource::<aa_net::state::Buffs>()
            .init_resource::<aa_net::state::Inventory>()
            .insert_non_send(crate::hud::api::HudBridge(shared))
            .insert_non_send(rt)
            .add_systems(PreUpdate, aim_input.after(bevy::input::InputSystems))
            .add_systems(
                Update,
                (
                    crate::settings::update_input,
                    route,
                    crate::hud::test_escape_key,
                    collect,
                )
                    .chain(),
            );
        let send = |app: &mut App, key_code, down| {
            app.world_mut().write_message(KeyboardInput {
                key_code,
                logical_key: Key::Character("x".into()),
                state: if down {
                    ButtonState::Pressed
                } else {
                    ButtonState::Released
                },
                text: None,
                repeat: false,
                window: Entity::PLACEHOLDER,
            });
            app.update();
        };
        app.world_mut().write_message(SkillPressed {
            skill: 23741,
            item: Some((9_007_199_254_740_997, 29880)),
            target_obj_id: 0,
            gcd: true,
        });
        app.update();
        send(&mut app, KeyCode::F12, true);
        app.world().get_non_send::<aa_ui::UiRuntime>().unwrap().lua.load("assert(not X2Item:IsInItemSkinizeMode() and X2Cursor:GetHeld()~=nil and OPEN_COUNT==0)").exec()?;
        assert!(app.world().resource::<Sent>().0.is_empty());
        send(&mut app, KeyCode::F12, false);
        let aim = || Aim {
            press: SkillPressed {
                skill: 10664,
                item: None,
                target_obj_id: 0,
                gcd: true,
            },
            skill: GroundSkill {
                mode: Mode::Ground,
                radius: 5.0,
                min_range: 0.0,
                max_range: 25.0,
            },
            spot: None,
            plant: None,
        };
        app.world_mut().resource_mut::<Aiming>().0 = Some(aim());
        send(&mut app, KeyCode::F12, true);
        assert!(app.world().resource::<Aiming>().0.is_none());
        app.world()
            .get_non_send::<aa_ui::UiRuntime>()
            .unwrap()
            .lua
            .load("assert(OPEN_COUNT==0 and X2Cursor:GetHeld()~=nil)")
            .exec()?;
        send(&mut app, KeyCode::F12, false);
        app.world_mut().resource_mut::<Aiming>().0 = Some(aim());
        app.world_mut()
            .resource_mut::<crate::hud::HudKeyboardFocus>()
            .0 = true;
        send(&mut app, KeyCode::Escape, true);
        assert!(
            app.world().resource::<Aiming>().0.is_some(),
            "pre-update aiming must not bypass focused keyboard capture"
        );
        send(&mut app, KeyCode::Escape, false);
        app.world_mut()
            .resource_mut::<crate::hud::HudKeyboardFocus>()
            .0 = false;
        send(&mut app, KeyCode::Escape, true);
        assert!(app.world().resource::<Aiming>().0.is_none());
        app.world()
            .get_non_send::<aa_ui::UiRuntime>()
            .unwrap()
            .lua
            .load("assert(OPEN_COUNT==0)")
            .exec()?;
        send(&mut app, KeyCode::Escape, false);
        app.world_mut().remove_non_send::<aa_ui::UiRuntime>();
        app.world_mut().resource_mut::<Aiming>().0 = Some(aim());
        send(&mut app, KeyCode::Escape, true);
        assert!(app.world().resource::<Aiming>().0.is_none(), "--no-hud retains physical Escape cancellation");
        std::fs::remove_file(path)?;
        Ok(())
    }

    #[derive(Clone)]
    struct SummonTestWorld {
        terrain: f32,
        water: Option<f32>,
        terrain_box: bool,
    }

    impl crate::hud::summon::SummonSpace for SummonTestWorld {
        fn terrain_height(&self, _x: f32, _y: f32) -> Option<f32> {
            Some(self.terrain)
        }

        fn water_level(&self, _x: f32, _y: f32) -> Option<f32> {
            self.water
        }

        fn ray_blocked(&self, _from: Vec3, _to: Vec3) -> bool {
            false
        }

        fn box_blocked(&self, _center: Vec3, _yaw: f32, _half: Vec3, terrain: bool) -> bool {
            terrain && self.terrain_box
        }
    }

    #[derive(Resource, Default)]
    struct Errors(Vec<i16>);

    fn collect_errors(mut errors: MessageReader<crate::hud::ShowError>, mut out: ResMut<Errors>) {
        out.0.extend(errors.read().map(|e| e.0));
    }

    fn summon_app(world: SummonTestWorld, slaves: aa_net::Slaves) -> App {
        let bridge = crate::hud::api::HudBridge(std::rc::Rc::new(std::cell::RefCell::new(
            crate::hud::api::HudData::default(),
        )));
        let mut game = crate::hud::GameData::default();
        game.summon_slaves.insert(
            5223,
            crate::hud::summon::SummonSlave {
                slave_id: 21,
                spawn_x_offset: 30.0,
                spawn_y_offset: 20.0,
                ship: true,
                model_id: 393,
                half_extents: Vec3::new(3.0, 15.0, 7.0),
                box_center: Vec3::ZERO,
                valid_area_range: 50.0,
            },
        );
        let _ = bridge.0.borrow().game.0.set(game);
        let mut app = App::new();
        app.add_message::<SkillPressed>()
            .add_message::<crate::hud::PortalBookUsed>()
            .add_message::<crate::hud::MusicItemUsed>()
            .add_message::<crate::hud::UseFailed>()
            .add_message::<crate::hud::ShowError>()
            .add_message::<NetCommand>()
            .insert_resource(GroundSkills::default())
            .init_resource::<Aiming>()
            .init_resource::<HudPointerOverUi>()
            .init_resource::<InFlight>()
            .init_resource::<Script>()
            .init_resource::<Target>()
            .init_resource::<Units>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<Sent>()
            .init_resource::<Errors>()
            .insert_resource(slaves)
            .insert_resource(crate::hud::summon::SummonWorld(std::sync::Arc::new(world)))
            .insert_non_send(bridge)
            .add_systems(Update, (route, collect, collect_errors).chain());
        app.world_mut().spawn((
            LocalPlayer { obj_id: 1 },
            Mover {
                pos: Vec3::new(100.0, 100.0, 0.0),
                facing: 0.0,
                vz: 0.0,
                grounded: true,
                velocity: Vec3::ZERO,
                swim: None,
                wading: 0.0,
            },
        ));
        app
    }

    fn summon_press(app: &mut App) {
        app.world_mut().write_message(SkillPressed {
            skill: 15802,
            item: Some((1, 5223)),
            target_obj_id: 0,
            gcd: true,
        });
        app.update();
    }

    #[test]
    fn a_ship_summon_item_is_refused_on_land_and_shallow_water() {
        // dry land: a ship item finds no water
        let mut app = summon_app(
            SummonTestWorld {
                terrain: 0.0,
                water: None,
                terrain_box: false,
            },
            aa_net::Slaves::default(),
        );
        summon_press(&mut app);
        assert!(
            app.world().resource::<Sent>().0.is_empty(),
            "no packet when the search fails"
        );
        assert_eq!(
            app.world().resource::<Errors>().0,
            vec![100],
            "Summoning requires a larger space."
        );

        // shallow water: the water check passes but the hull box overlaps the ground
        let mut app = summon_app(
            SummonTestWorld {
                terrain: 0.0,
                water: Some(1.0),
                terrain_box: true,
            },
            aa_net::Slaves::default(),
        );
        summon_press(&mut app);
        assert!(app.world().resource::<Sent>().0.is_empty());
        assert_eq!(app.world().resource::<Errors>().0, vec![100]);
    }

    #[test]
    fn a_ship_summon_item_is_sent_on_open_water() {
        let mut app = summon_app(
            SummonTestWorld {
                terrain: -20.0,
                water: Some(1.0),
                terrain_box: false,
            },
            aa_net::Slaves::default(),
        );
        summon_press(&mut app);
        assert!(
            matches!(
                app.world().resource::<Sent>().0.as_slice(),
                [Command::UseItemAt { item_id: 1, template_id: 5223, skill_id: 15802, pos, rot }]
                    if (pos.x - 100.0).abs() < 1e-4
                        && (pos.y - 130.0).abs() < 1e-4
                        && (pos.z - 1.0).abs() < 1e-4
                        && (*rot - std::f32::consts::FRAC_PI_2).abs() < 1e-6
            ),
            "{:?}",
            app.world().resource::<Sent>().0
        );
        assert!(app.world().resource::<Errors>().0.is_empty());
    }

    #[test]
    fn an_owned_slave_toggles_the_summon_item_into_a_cancel() {
        use aa_net::slave::SlavePositionInfo;
        let near = SlavePositionInfo {
            obj_id: 9,
            tl_id: 1,
            name: "Eznan Cutter".into(),
            template_id: 21,
            hp: 1,
            max_hp: 1,
            x: aa_net::wire::coord_to_long(110.0),
            y: aa_net::wire::coord_to_long(100.0),
            z: 0.0,
        };
        let mut slaves = aa_net::Slaves::default();
        slaves.position = Some(near);
        let mut app = summon_app(
            SummonTestWorld {
                terrain: -20.0,
                water: Some(1.0),
                terrain_box: false,
            },
            slaves,
        );
        summon_press(&mut app);
        assert!(
            matches!(
                app.world().resource::<Sent>().0.as_slice(),
                [Command::Slave(
                    aa_net::slave_control::SlaveCommand::Despawn { obj_id: 9 }
                )]
            ),
            "{:?}",
            app.world().resource::<Sent>().0
        );
        assert!(app.world().resource::<Errors>().0.is_empty());

        let far = SlavePositionInfo {
            obj_id: 9,
            tl_id: 1,
            name: String::new(),
            template_id: 21,
            hp: 1,
            max_hp: 1,
            x: aa_net::wire::coord_to_long(300.0),
            y: aa_net::wire::coord_to_long(100.0),
            z: 0.0,
        };
        let mut slaves = aa_net::Slaves::default();
        slaves.position = Some(far);
        let mut app = summon_app(
            SummonTestWorld {
                terrain: -20.0,
                water: Some(1.0),
                terrain_box: false,
            },
            slaves,
        );
        summon_press(&mut app);
        assert!(app.world().resource::<Sent>().0.is_empty());
        assert_eq!(
            app.world().resource::<Errors>().0,
            vec![312],
            "Too far away to cancel summoning."
        );
    }
}
