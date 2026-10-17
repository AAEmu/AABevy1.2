//! Combat in the HUD: the combat events of the server as the UI events the original scripts listen
//! for, and the floating combat text those scripts drive.
//!
//! ## Events
//!
//! [`route_combat`] turns aa-net's combat events into what the client's C++ raises:
//!
//! * `COMBAT_TEXT(sourceId, targetId, amount, skillType, hitOrMissType, weaponDamage, isSynergy, distance)`
//!   for damage and heals of units other than the player (`amount` is negative for damage: the script
//!   prints `-amount`), `skillType` `SWING` for the auto attack (white numbers), `SKILL` (yellow),
//!   `DOT`, `HEAL`, `MANA`;
//! * `COMBAT_MSG(targetId, "MELEE_DAMAGE" | "SPELL_DAMAGE" | ..., source, target, ...)` for what happens
//!   to the player (`x2ui/chat/chat_msg_event.lua` parses the arguments, `combattext/combat_text.lua`
//!   prints the red "-12" over the player);
//! * `EXP_CHANGED(unitId, delta, deltaText)` (the "Gained 14 XP." chat line and the pink "XP +14" text).
//!
//! `UNIT_COMBAT_STATE_CHANGED` ("Battle Start!" / "Battle End!") comes from the snapshot's `in_combat`
//! (`api::push_state`).
//!
//! ## Floating text
//!
//! The scripts put their text on 30 `damagedisplay` widgets; the client's native widget anchors the text
//! over a unit and plays the frame table (`COMBAT_TEXT_ANIMATION[CTA_*]`) of the animation type. aa-ui
//! has no such widget, so `combat.lua` replaces the widgets' methods (`SetUnitId`, `SetText`,
//! `SetInitPos`, `SetAnimFrameInfo`, `Animation`, `IsVisible`, the styles' `SetColor`) after the stage is
//! built and hands each started text to [`Texts`] (`HUD.combat_text_start`); [`update_texts`] draws them
//! as Bevy `Text2d` on the name plate camera.
//!
//! Frames (measured on `reference/ingame/combat_*`): the text starts at the first frame's scale and
//! alpha; every frame holds its state for `showTime` ms and then changes to the next frame's scale and
//! alpha over `timeToNextFrame` ms (an unset alpha is 1, `moveX` / `moveY` move the text by that many
//! pixels during the frame's own transition); reaching the last frame ends the text. The font is the
//! client's `ui/font/yoon_firedgothic_b.ttf` which for Latin text is Libre Baskerville Bold (`fonts.g`),
//! 60 px times the scale; no outline or shadow. The offset of `SetInitPos` is in pixels from the point
//! over the unit's head (a little under the name plate), y down.

use aa_net::Event;
use aa_net::model::UnitKind;
use aa_net::skill::hit_type;
use aa_net::state::{Player, Units};
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::sprite::Anchor;
use bevy::text::{Font, FontSize, FontSource, TextColor, TextFont};
use mlua::{Lua, Table, Value};

use super::api::HudBridge;
use super::plates::{PlateRoot, PlateState, head_gap, unit_height};
use crate::Screen;

/// The `damagedisplay` widgets of the combat text frame.
pub const SLOTS: usize = 30;
/// The render layer of the plate camera (`plates::LAYER`).
const LAYER: usize = 30;
/// The font size the scripts give the widgets: the scale of a frame multiplies it.
const BASE_PX: f32 = 60.0;
/// The font of `yoon_firedgothic_b.ttf` in the English client (`game/fonts/fonts.g`).
const FONT_PATH: &str = "ui/font/librebaskerville-bold.ttf";
/// Metres over the head where the text is anchored (the plate's bar is 0.3 m over the head).
const ANCHOR_GAP: f32 = 0.18;
/// Seconds after the end of a fight in which the packets of its last blows do not start a new one.
const OVER_QUIET: f64 = 1.0;

/// One frame of a text's animation table.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub scale: f32,
    pub alpha: f32,
    /// Milliseconds the frame's state is held before the change to the next frame.
    pub show_ms: f32,
    /// Milliseconds of the change to the next frame.
    pub time_ms: f32,
    pub move_x: f32,
    pub move_y: f32,
}

/// What a text looks like at some moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub scale: f32,
    pub alpha: f32,
    /// Pixels moved since the start (x right, y down).
    pub offset: Vec2,
}

/// The pose after `age` milliseconds, `None` once the last frame is reached.
pub fn pose(frames: &[Frame], age: f32) -> Option<Pose> {
    let mut t = age;
    let mut moved = Vec2::ZERO;
    for (i, f) in frames.iter().enumerate() {
        let next = frames.get(i + 1)?;
        if t < f.show_ms {
            return Some(Pose { scale: f.scale, alpha: f.alpha, offset: moved });
        }
        t -= f.show_ms;
        if t < f.time_ms {
            let k = t / f.time_ms;
            return Some(Pose {
                scale: f.scale + (next.scale - f.scale) * k,
                alpha: f.alpha + (next.alpha - f.alpha) * k,
                offset: moved + Vec2::new(f.move_x, f.move_y) * k,
            });
        }
        t -= f.time_ms;
        moved += Vec2::new(f.move_x, f.move_y);
    }
    None
}

/// The scale a text rests at (the frame with a `showTime`, else the last): its glyphs are drawn at
/// that size and only scaled for the pop in and out.
fn steady_scale(frames: &[Frame]) -> f32 {
    frames.iter().find(|f| f.show_ms > 0.0).or(frames.last()).map_or(0.5, |f| f.scale)
}

/// A text that is playing.
#[derive(Clone, Debug)]
pub struct Text {
    pub text: String,
    /// Object id of the unit it floats over (`None`: the player).
    pub target: Option<u32>,
    /// `SetInitPos`: pixels from the anchor.
    pub offset: Vec2,
    pub color: [f32; 4],
    pub frames: Vec<Frame>,
    pub age_ms: f32,
    /// The point over the unit (world), found when the text is first drawn.
    pub anchor: Option<Vec3>,
    pub changed: bool,
}

/// The widgets' slots (index 0 is widget 1).
#[derive(Debug)]
pub struct Texts {
    pub slots: Vec<Option<Text>>,
}

impl Default for Texts {
    fn default() -> Self {
        Texts { slots: vec![None; SLOTS] }
    }
}

fn frame_of(t: &Table) -> mlua::Result<Frame> {
    let num = |key: &str, default: f32| t.get::<Option<f32>>(key).map(|v| v.unwrap_or(default));
    Ok(Frame {
        scale: num("scale", 1.0)?,
        alpha: num("alpha", 1.0)?,
        show_ms: num("showTime", 0.0)?.max(0.0),
        time_ms: num("timeToNextFrame", 0.0)?.max(0.0),
        move_x: num("moveX", 0.0)?,
        move_y: num("moveY", 0.0)?,
    })
}

/// The `HUD.combat_text_*` functions `combat.lua` calls.
pub fn install(lua: &Lua, hud: &Table, bridge: &super::api::Shared) -> mlua::Result<()> {
    let b = bridge.clone();
    hud.raw_set(
        "combat_text_start",
        lua.create_function(move |_, (slot, target, text, x, y, color, frames): (usize, Option<String>, String, f32, f32, Table, Table)| {
            let Some(index) = slot.checked_sub(1).filter(|i| *i < SLOTS) else { return Ok(()) };
            let mut list = Vec::new();
            for v in frames.sequence_values::<Table>() {
                list.push(frame_of(&v?)?);
            }
            debug!("hud: combat text {slot} {text:?} at ({x}, {y}) over {target:?}, {} frames", list.len());
            if list.len() < 2 {
                return Ok(());
            }
            let c = |i: i64| color.get::<Option<f32>>(i).map(|v| v.unwrap_or(1.0));
            b.borrow_mut().texts.slots[index] = Some(Text {
                text,
                target: target.and_then(|t| t.parse().ok()),
                offset: Vec2::new(x, y),
                color: [c(1)?, c(2)?, c(3)?, c(4)?],
                frames: list,
                age_ms: 0.0,
                anchor: None,
                changed: true,
            });
            Ok(())
        })?,
    )?;
    let b = bridge.clone();
    hud.raw_set(
        "combat_text_text",
        lua.create_function(move |_, (slot, text): (usize, String)| {
            let mut d = b.borrow_mut();
            if let Some(t) = slot.checked_sub(1).and_then(|i| d.texts.slots.get_mut(i)).and_then(Option::as_mut) {
                t.text = text;
                t.changed = true;
            }
            Ok(())
        })?,
    )?;
    let b = bridge.clone();
    hud.raw_set(
        "combat_text_active",
        lua.create_function(move |_, slot: usize| Ok(slot.checked_sub(1).is_some_and(|i| b.borrow().texts.slots.get(i).is_some_and(Option::is_some))))?,
    )?;
    let b = bridge.clone();
    hud.raw_set(
        "combat_text_stop",
        lua.create_function(move |_, slot: usize| {
            let mut d = b.borrow_mut();
            if let Some(s) = slot.checked_sub(1).and_then(|i| d.texts.slots.get_mut(i)) {
                *s = None;
            }
            Ok(())
        })?,
    )?;
    Ok(())
}

// -- events -------------------------------------------------------------------------------------------

/// `hitOrMissType` of the scripts for a wire hit type: "HIT", "CRITICAL" or the miss kind.
fn hit_name(h: u8) -> &'static str {
    match h {
        hit_type::MELEE_CRITICAL | hit_type::RANGED_CRITICAL | hit_type::SPELL_CRITICAL => "CRITICAL",
        hit_type::MELEE_MISS | hit_type::RANGED_MISS | hit_type::SPELL_MISS => "MISS",
        hit_type::MELEE_DODGE | hit_type::RANGED_DODGE => "DODGE",
        hit_type::MELEE_BLOCK | hit_type::RANGED_BLOCK => "BLOCK",
        hit_type::MELEE_PARRY | hit_type::RANGED_PARRY => "PARRY",
        hit_type::IMMUNE => "IMMUNE",
        hit_type::SPELL_RESIST => "RESIST",
        _ => "HIT",
    }
}

/// A unit's name for the chat lines.
fn unit_name(units: &Units, game: Option<&super::data::GameData>, own: &Player, id: u32) -> String {
    if own.spawn.as_ref().is_some_and(|s| s.obj_id == id) {
        return own.spawn.as_ref().map(|s| s.name().to_string()).unwrap_or_default();
    }
    let Some(u) = units.get(id) else { return String::new() };
    if !u.name.is_empty() {
        return u.name.clone();
    }
    match &u.kind {
        UnitKind::Character { .. } => String::new(),
        kind => game.and_then(|g| g.npcs.get(&kind.template_id())).map(|n| n.name.clone()).unwrap_or_default(),
    }
}

/// aa-net's combat events as the scripts' events (after `sync_hud` pushed this frame's state).
#[allow(clippy::too_many_arguments)]
pub fn route_combat(
    mut events: MessageReader<aa_net::NetEvent>,
    rt: Option<NonSend<aa_ui::UiRuntime>>,
    bridge: NonSend<HudBridge>,
    units: Res<Units>,
    player: Res<Player>,
    time: Res<Time>,
    mut over_at: Local<f64>,
) {
    let Some(rt) = rt else {
        events.clear();
        return;
    };
    let now = time.elapsed_secs_f64();
    let Some(own) = player.spawn.as_ref().map(|s| s.obj_id) else {
        events.clear();
        return;
    };
    // no borrow of the bridge while the events run: the scripts call back into `HUD.combat_text_*`
    let (shared_game, own_pos) = {
        let data = bridge.0.borrow();
        (data.game.clone(), data.pos)
    };
    let game = shared_game.get();
    let skill = |id: u32| game.and_then(|g| g.skills.get(&id));
    let id = |obj: u32| obj.to_string();
    for aa_net::NetEvent(event) in events.read() {
        match event {
            Event::UnitDamaged(d) => {
                if d.caster_obj_id != own && d.target_obj_id != own {
                    continue;
                }
                // (the packet of the killing blow comes after the one that ends the fight: it is not a new fight)
                if now - *over_at > OVER_QUIET {
                    bridge.0.borrow_mut().local_combat = true;
                }
                let info = skill(d.action.skill_id);
                let auto = info.is_some_and(|s| s.auto_attack);
                let hit = hit_name(d.hit_type);
                let missed = hit_type::is_miss(d.hit_type);
                if d.target_obj_id == own {
                    let source = unit_name(&units, game, &player, d.caster_obj_id);
                    let target = unit_name(&units, game, &player, own);
                    let spell = info.map_or_else(String::new, |s| s.named.name.clone());
                    let (event, with_spell) = match (missed, d.action.kind == 2, auto) {
                        (true, _, true) => ("MELEE_MISSED", false),
                        (true, _, false) => ("SPELL_MISSED", true),
                        (false, true, _) => ("SPELL_DOT_DAMAGE", true),
                        (false, false, true) => ("MELEE_DAMAGE", false),
                        (false, false, false) => ("SPELL_DAMAGE", true),
                    };
                    // (targetId, event, source, target, [spellId, spellName, school,] then the event's own values)
                    let mut args: Vec<Value> = Vec::new();
                    let s = |text: &str| rt.lua.create_string(text).map(Value::String).unwrap_or(Value::Nil);
                    args.extend([s(&id(own)), s(event), s(&source), s(&target)]);
                    if with_spell {
                        args.extend([Value::Integer(i64::from(d.action.skill_id)), s(&spell), s("physical")]);
                    }
                    if missed {
                        args.push(s(hit));
                    } else {
                        args.extend([Value::Integer(i64::from(d.damage)), s("HEALTH"), s(hit), Value::Integer(i64::from(d.absorbed)), s("slashing"), Value::Integer(0)]);
                    }
                    rt.fire_event("COMBAT_MSG", mlua::MultiValue::from_iter(args));
                } else {
                    let skill_type = if d.action.kind == 2 {
                        "DOT"
                    } else if auto {
                        "SWING"
                    } else {
                        "SKILL"
                    };
                    let distance = units.get(d.target_obj_id).map_or(0.0, |u| own_pos.distance(Vec3::new(u.pos.x, u.pos.y, u.pos.z)) as f64);
                    debug!("hud: COMBAT_TEXT {} -> {} {} {skill_type} {hit}", d.caster_obj_id, d.target_obj_id, d.damage);
                    rt.fire_event(
                        "COMBAT_TEXT",
                        (id(own), id(d.target_obj_id), -i64::from(d.damage), skill_type, hit, 0i64, false, distance),
                    );
                }
            }
            Event::UnitHealed(h) => {
                if h.target_obj_id != own && h.caster.obj_id != own {
                    continue;
                }
                let critical = matches!(h.hit_type, hit_type::RANGED_CRITICAL | hit_type::SPELL_CRITICAL);
                let hit = if critical { "CRITICAL" } else { "HIT" };
                if h.target_obj_id == own {
                    let name = skill(h.action.skill_id).map_or_else(String::new, |s| s.named.name.clone());
                    let (event, power) = if h.heal_type == 1 { ("SPELL_ENERGIZE", "MANA") } else { ("SPELL_HEALED", "HEALTH") };
                    let source = unit_name(&units, game, &player, h.caster.obj_id);
                    let target = unit_name(&units, game, &player, own);
                    if h.heal_type == 1 {
                        rt.fire_event("COMBAT_MSG", (id(own), event, source, target, i64::from(h.action.skill_id), name, "physical", i64::from(h.value), power));
                    } else {
                        rt.fire_event("COMBAT_MSG", (id(own), event, source, target, i64::from(h.action.skill_id), name, "physical", i64::from(h.value), hit));
                    }
                } else {
                    let skill_type = if h.heal_type == 1 { "MANA" } else { "HEAL" };
                    rt.fire_event("COMBAT_TEXT", (id(own), id(h.target_obj_id), i64::from(h.value), skill_type, hit, 0i64, false, 0.0f64));
                }
            }
            // attacking something starts the fight the moment the skill does (the weapon comes out)
            Event::SkillStarted(s) if s.caster.obj_id == own && s.result == 0 && s.target.obj_id != 0 && s.target.obj_id != own => {
                if now - *over_at > OVER_QUIET {
                    bridge.0.borrow_mut().local_combat = true;
                }
            }
            // the server clears the combat state of the player when the fight is over (the kill); a death ends it too
            Event::CombatChanged { obj_id, engaged: false } | Event::UnitDeath { obj_id } | Event::Resurrected { obj_id, .. } if *obj_id == own => {
                debug!("hud: the combat of the player is over ({event:?})");
                bridge.0.borrow_mut().local_combat = false;
                *over_at = now;
            }
            Event::CombatChanged { obj_id, engaged } => debug!("hud: combat state of unit {obj_id}: {engaged}"),
            Event::ExpChanged { obj_id, delta, .. } if *obj_id == own => {
                rt.fire_event("EXP_CHANGED", (id(own), i64::from(*delta), delta.abs().to_string()));
            }
            _ => {}
        }
    }
}

// -- drawing ------------------------------------------------------------------------------------------

#[derive(Component)]
pub struct FloatText;

#[derive(Resource, Default)]
pub struct CombatFont(Handle<Font>);

#[derive(Resource, Default)]
pub struct FloatState {
    entities: Vec<Entity>,
    /// The font size each slot's glyphs were laid out at.
    px: Vec<f32>,
}

pub fn load_font(mut fonts: ResMut<Assets<Font>>, mut font: ResMut<CombatFont>) {
    match aa_pak::Pak::global().read(FONT_PATH) {
        Ok(bytes) => font.0 = fonts.add(Font::from_bytes(bytes)),
        Err(e) => warn!("hud: combat text font {FONT_PATH}: {e}"),
    }
}

pub fn despawn(mut commands: Commands, mut state: ResMut<FloatState>, texts: Query<Entity, With<FloatText>>, bridge: NonSend<HudBridge>) {
    for e in &texts {
        commands.entity(e).despawn();
    }
    *state = FloatState::default();
    bridge.0.borrow_mut().texts = Texts::default();
}

/// The world point over a unit's head, as the plates measure the unit.
#[allow(clippy::too_many_arguments)]
fn overhead(
    id: u32,
    own: u32,
    plates: &PlateState,
    units: &Units,
    transforms: &Query<&Transform, (Without<FloatText>, Without<PlateRoot>)>,
) -> Option<Vec3> {
    let entity = plates.entities.get(&id).copied();
    let unit = units.get(id);
    let root = match entity.and_then(|e| transforms.get(e).ok()) {
        Some(t) => t.translation,
        None => aa_world::cry_to_bevy(unit.map(|u| Vec3::new(u.pos.x, u.pos.y, u.pos.z))?),
    };
    let scale = unit.map_or(1.0, |u| u.scale);
    let is_character = id == own || unit.is_some_and(|u| matches!(u.kind, UnitKind::Character { .. }));
    let height = unit_height(plates.heights.get(&id).map(|h| h.0), scale, is_character);
    Some(root + Vec3::Y * (height + head_gap(height, ANCHOR_GAP)))
}

/// Draws the playing texts: one `Text2d` per widget slot on the plate camera.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn update_texts(
    mut commands: Commands,
    time: Res<Time>,
    bridge: NonSend<HudBridge>,
    font: Res<CombatFont>,
    mut state: ResMut<FloatState>,
    plates: Res<PlateState>,
    units: Res<Units>,
    player: Res<Player>,
    world_camera: Query<(&Camera, &Transform), (With<Camera3d>, Without<FloatText>, Without<PlateRoot>)>,
    transforms: Query<&Transform, (Without<FloatText>, Without<PlateRoot>)>,
    mut floats: Query<(&mut Transform, &mut Visibility, &mut Text2d, &mut TextColor, &mut TextFont), With<FloatText>>,
) {
    let Some(own) = player.spawn.as_ref().map(|s| s.obj_id) else { return };
    let mut data = bridge.0.borrow_mut();
    if state.entities.is_empty() {
        // the pool: spawned once, hidden until a slot plays
        let layers = RenderLayers::layer(LAYER);
        for i in 0..SLOTS {
            let e = commands
                .spawn((
                    Name::new(format!("combat text {}", i + 1)),
                    FloatText,
                    Text2d::new(""),
                    TextFont { font: FontSource::Handle(font.0.clone()), font_size: FontSize::Px(BASE_PX * 0.5), ..default() },
                    TextColor(Color::WHITE),
                    Anchor::CENTER,
                    Transform::from_xyz(0.0, 0.0, 1.0),
                    Visibility::Hidden,
                    layers.clone(),
                ))
                .id();
            state.entities.push(e);
        }
        state.px = vec![BASE_PX * 0.5; SLOTS];
        return;
    }
    let Some((camera, camera_transform)) = world_camera.iter().find(|(c, _)| c.is_active) else { return };
    let Some(viewport) = camera.logical_viewport_size() else { return };
    let camera_global = GlobalTransform::from(*camera_transform);
    let dt = time.delta_secs() * 1000.0;
    for i in 0..SLOTS {
        let entity = state.entities[i];
        let Ok((mut transform, mut visibility, mut text2d, mut color, mut text_font)) = floats.get_mut(entity) else { continue };
        let Some(t) = data.texts.slots[i].as_mut() else {
            if *visibility != Visibility::Hidden {
                *visibility = Visibility::Hidden;
            }
            continue;
        };
        if t.anchor.is_none() {
            t.anchor = overhead(t.target.unwrap_or(own), own, &plates, &units, &transforms).or_else(|| overhead(own, own, &plates, &units, &transforms));
        }
        let first = t.age_ms == 0.0;
        // the first frame is drawn at age 0, then time passes
        let pose_now = pose(&t.frames, t.age_ms);
        if !first {
            t.age_ms += dt;
        } else {
            t.age_ms = 0.001;
        }
        let (Some(p), Some(anchor)) = (pose_now, t.anchor) else {
            debug!("hud: combat text {} ended (anchor {:?}, age {:.0})", i + 1, t.anchor, t.age_ms);
            data.texts.slots[i] = None;
            *visibility = Visibility::Hidden;
            continue;
        };
        let Ok(screen) = camera.world_to_viewport(&camera_global, anchor) else {
            debug!("hud: combat text {} is off screen", i + 1);
            *visibility = Visibility::Hidden;
            continue;
        };
        let steady = steady_scale(&t.frames);
        let px = (BASE_PX * steady).round().max(8.0);
        if t.changed || (state.px[i] - px).abs() > 0.5 {
            text2d.0 = t.text.clone();
            text_font.font_size = FontSize::Px(px);
            state.px[i] = px;
            t.changed = false;
        }
        let x = screen.x - viewport.x / 2.0 + t.offset.x + p.offset.x;
        let y = viewport.y / 2.0 - screen.y - t.offset.y - p.offset.y;
        if first {
            debug!("hud: combat text {} drawn at ({x:.0}, {y:.0}) scale {:.2} alpha {:.2} px {px}", i + 1, p.scale, p.alpha);
        }
        transform.translation = Vec3::new(x.round(), y.round(), 1.0);
        transform.scale = Vec3::splat(p.scale / steady);
        color.0 = Color::srgba(t.color[0], t.color[1], t.color[2], (t.color[3] * p.alpha).clamp(0.0, 1.0));
        if *visibility != Visibility::Inherited {
            *visibility = Visibility::Inherited;
        }
    }
}

/// Not with `--no-hud`.
pub fn plugin(app: &mut App) {
    app.init_resource::<CombatFont>()
        .init_resource::<FloatState>()
        .add_systems(Startup, load_font)
        .add_systems(OnExit(Screen::InGame), despawn);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(scale: f32, alpha: f32, show: f32, time: f32) -> Frame {
        Frame { scale, alpha, show_ms: show, time_ms: time, move_x: 0.0, move_y: 0.0 }
    }

    /// `CTA_BASICATTACK`: pops in at 1.5 and shrinks to 0.55, holds 400 ms, fades.
    fn basic() -> Vec<Frame> {
        vec![f(1.5, 0.2, 0.0, 70.0), f(0.5, 1.0, 0.0, 30.0), f(0.62, 1.0, 0.0, 30.0), f(0.55, 1.0, 400.0, 120.0), f(0.55, 0.1, 0.0, 0.0)]
    }

    #[test]
    fn frames_play_in_order() {
        let frames = basic();
        let p = pose(&frames, 0.0).unwrap();
        assert_eq!((p.scale, p.alpha), (1.5, 0.2));
        // half way through the first change
        let p = pose(&frames, 35.0).unwrap();
        assert!((p.scale - 1.0).abs() < 1e-4 && (p.alpha - 0.6).abs() < 1e-4);
        // held at the steady scale
        let p = pose(&frames, 300.0).unwrap();
        assert_eq!((p.scale, p.alpha), (0.55, 1.0));
        // fading
        let p = pose(&frames, 70.0 + 30.0 + 30.0 + 400.0 + 60.0).unwrap();
        assert!((p.alpha - 0.55).abs() < 1e-3);
        // over after 650 ms
        assert!(pose(&frames, 651.0).is_none());
        assert_eq!(steady_scale(&frames), 0.55);
    }

    #[test]
    fn moves_add_up() {
        let mut frames = vec![f(0.45, 1.0, 600.0, 300.0), f(0.45, 1.0, 0.0, 900.0), f(0.45, 0.2, 0.0, 0.0)];
        frames[1].move_y = -55.0;
        // nothing moves while the first frame is held and changes (900 ms), the second frame's move runs over its own change
        assert_eq!(pose(&frames, 800.0).unwrap().offset, Vec2::ZERO);
        assert_eq!(pose(&frames, 900.0).unwrap().offset, Vec2::ZERO);
        let half = pose(&frames, 900.0 + 450.0).unwrap().offset;
        assert!((half.y + 27.5).abs() < 1e-3);
        assert!(pose(&frames, 1800.0).is_none());
    }

    #[test]
    fn wire_hit_types() {
        assert_eq!(hit_name(hit_type::MELEE_HIT), "HIT");
        assert_eq!(hit_name(hit_type::SPELL_CRITICAL), "CRITICAL");
        assert_eq!(hit_name(hit_type::RANGED_DODGE), "DODGE");
        assert_eq!(hit_name(hit_type::SPELL_RESIST), "RESIST");
    }
}
