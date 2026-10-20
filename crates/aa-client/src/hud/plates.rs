//! Name plates over the units in the 3D view: the name in the client's UI font and, for NPCs, the
//! health bar, coloured by how the unit stands to the player (yellow neutral, green friendly,
//! red hostile; the colours of `reference/ingame/spawn/real.png`).
//!
//! The plates are Bevy `Text2d` / `Sprite` entities seen by a camera of their own (order between the
//! world's and the UI's), positioned every frame by projecting the point above each unit's head to
//! the screen: moving them does not touch the UI toolkit's meshes. Only the nearest few dozen units
//! within the culling distance get one.

use std::collections::HashMap;

use aa_net::model::UnitKind;
use aa_net::state::{Player, Units};
use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::RenderLayers;
use bevy::camera::{CompositingSpace, Hdr};
use bevy::ecs::system::SystemParam;
use bevy::mesh::skinning::SkinnedMesh;
use bevy::prelude::*;
use bevy::render::view::Msaa;
use bevy::sprite::{Anchor, Text2dShadow};
use bevy::text::{Font, FontSize, FontSource, TextColor, TextFont};

use super::api::HudBridge;
use crate::Screen;

/// The render layer of the plates.
const LAYER: usize = 30;
/// Culling distance for NPCs / other characters, metres.
const NPC_RANGE: f32 = 45.0;
const PLAYER_RANGE: f32 = 70.0;
/// At most this many plates.
const MAX_PLATES: usize = 40;
/// Bar size in pixels (the client's: 69 x 6 with a dark rim).
const BAR_W: f32 = 69.0;
const BAR_H: f32 = 6.0;
/// Metres between the head and the bar.
pub(super) const HEAD_GAP: f32 = 0.3;
/// A character's height for the plate, metres: the plate sits over the head, not over the bow.
const CHARACTER_HEIGHT: f32 = 1.85;
/// Name sizes: NPCs, other characters (the client draws player names much larger).
const NPC_FONT_PX: f32 = 11.0;
const PLAYER_FONT_PX: f32 = 26.0;
/// Show the player's own plate (the capture has it: "Refone" over the character).
const SHOW_OWN: bool = true;
/// The font the client draws English UI text with.
const FONT_PATH: &str = "ui/font/flareserif_821_roman.ttf";

#[derive(Component)]
pub struct PlateCamera;

#[derive(Component)]
pub struct PlateRoot;

#[derive(Resource, Default)]
pub struct PlateFont(pub Handle<Font>);

pub fn load_font(mut fonts: ResMut<Assets<Font>>, mut font: ResMut<PlateFont>) {
    match aa_pak::Pak::global().read(FONT_PATH) {
        Ok(bytes) => font.0 = fonts.add(Font::from_bytes(bytes)),
        Err(e) => warn!("hud: plate font {FONT_PATH}: {e}"),
    }
}

/// The camera that draws the plates, in game only. It is spawned once the world camera has its
/// final HDR / MSAA settings (aa-world's post processing adds them a frame after the camera exists):
/// a frame in which the two disagree is a validation error of the shared main texture.
#[allow(clippy::type_complexity)]
pub fn ensure_camera(
    mut commands: Commands,
    world: Query<(Has<Hdr>, Option<&Msaa>), (With<Camera3d>, Without<PlateCamera>)>,
    plate: Query<Entity, With<PlateCamera>>,
) {
    if !plate.is_empty() {
        return;
    }
    let Some((true, msaa)) = world.iter().next() else { return };
    commands.spawn((
        Name::new("name plate camera"),
        PlateCamera,
        Camera2d,
        Camera { order: 5, clear_color: ClearColorConfig::None, ..default() },
        RenderLayers::layer(LAYER),
        CompositingSpace::Linear,
        Hdr,
        msaa.copied().unwrap_or_default(),
    ));
}

pub fn despawn(mut commands: Commands, mut state: ResMut<PlateState>, cameras: Query<Entity, With<PlateCamera>>, plates: Query<Entity, With<PlateRoot>>) {
    for e in cameras.iter().chain(plates.iter()) {
        commands.entity(e).despawn();
    }
    *state = PlateState::default();
}

/// The plate camera takes the world camera's HDR and MSAA settings (see `cameras.rs`).
#[allow(clippy::type_complexity)]
pub fn match_camera(
    mut commands: Commands,
    world: Query<(Has<Hdr>, Option<&Msaa>), (With<Camera3d>, Without<PlateCamera>)>,
    plate: Query<(Entity, Has<Hdr>, Option<&Msaa>), With<PlateCamera>>,
) {
    let Some((hdr, msaa)) = world.iter().next() else { return };
    let msaa = msaa.copied().unwrap_or_default();
    for (entity, plate_hdr, plate_msaa) in &plate {
        if plate_hdr != hdr {
            if hdr {
                commands.entity(entity).insert(Hdr);
            } else {
                commands.entity(entity).remove::<Hdr>();
            }
        }
        if plate_msaa.copied() != Some(msaa) {
            commands.entity(entity).insert(msaa);
        }
    }
}

struct Plate {
    root: Entity,
    text: Entity,
    fill: Entity,
    bar: Entity,
    /// The dark inside of the framed bar of the selected target.
    back: Entity,
    /// Styled as the selected target (big name, framed bar), and as a corpse (grey name, empty bar).
    targeted: bool,
    dead: bool,
    /// The "-Away-" line under the name of the player that is away from the keyboard.
    status: Entity,
    away: bool,
    has_bar: bool,
    name: String,
    fraction: f32,
    color: (u8, u8, u8),
}

/// The plate line: retail prefixes the title to the name on the same line ("Title Name", no brackets).
fn plate_line(title: &str, name: String) -> String {
    if title.is_empty() { name } else { format!("{title} {name}") }
}

/// `line` is [`plate_line`]`(title, name)`, without building it.
fn is_plate_line(line: &str, title: &str, name: &str) -> bool {
    if title.is_empty() {
        line == name
    } else {
        line.strip_prefix(title).and_then(|r| r.strip_prefix(' ')) == Some(name)
    }
}

#[derive(Resource, Default)]
pub struct PlateState {
    plates: HashMap<u32, Plate>,
    /// Unit object id -> its entity in the 3D scene, found by name every half second or so.
    pub(super) entities: HashMap<u32, Entity>,
    scan_in: u32,
    /// Unit object id -> height of its model (metres) and the attempts spent measuring it.
    pub(super) heights: HashMap<u32, (f32, u16)>,
    /// Unit object id -> maximum HP from the formulas and the frames until it is worked out again
    /// (the buffs and level it depends on change rarely).
    max_hp: HashMap<u32, (i32, u8)>,
}

fn srgb(c: (u8, u8, u8)) -> Color {
    Color::srgb_u8(c.0, c.1, c.2)
}

/// Bar and name colours by how the unit stands to the player.
fn colors(relation: &str) -> ((u8, u8, u8), (u8, u8, u8)) {
    match relation {
        // the Angry Rat of reference/ingame/combat_*: name (238, 107, 35), bar fill (146, 66, 21)
        "hostile" => ((146, 66, 21), (238, 107, 35)),
        "friendly" => ((70, 126, 31), (115, 210, 51)),
        _ => ((155, 131, 0), (255, 216, 0)),
    }
}

/// The faction a unit counts as: an NPC's is its template's, the packet's is the nation it was spawned for.
fn unit_faction(game: Option<&super::data::GameData>, u: &aa_net::Unit) -> u32 {
    let template = u.kind.template_id();
    match game.and_then(|g| g.npcs.get(&template)).filter(|_| template != 0) {
        Some(n) if n.faction_id != 0 => n.faction_id as u32,
        _ => u.faction_id,
    }
}

/// How a unit stands to the player (`"friendly"`, `"neutral"`, `"hostile"`): the plate's colours and the target ring's.
pub(super) fn relation_of(game: Option<&super::data::GameData>, own_faction: u32, u: &aa_net::Unit) -> &'static str {
    let faction = unit_faction(game, u);
    let mut relation = game.map_or("neutral", |g| g.relation(own_faction, faction));
    // the capture's villagers of the player's own faction (the newbie towns) are yellow, only players are green
    if !matches!(u.kind, UnitKind::Character { .. }) && relation == "friendly" && faction == own_faction {
        relation = "neutral";
    }
    relation
}

/// What measures a unit's model: the bounds of its static meshes and the joints of its skinned ones
/// (skinned meshes carry no bounds: aa-cry draws them without frustum culling).
#[derive(SystemParam)]
pub(super) struct Measure<'w, 's> {
    children: Query<'w, 's, &'static Children>,
    bounds: Query<'w, 's, (&'static Aabb, &'static GlobalTransform)>,
    skins: Query<'w, 's, &'static SkinnedMesh>,
    globals: Query<'w, 's, &'static GlobalTransform>,
    names: Query<'w, 's, &'static Name>,
}

/// A skeleton's top joint is in the head, the skin reaches a bit further (fraction of the height).
const SKULL: f32 = 0.04;

/// A unit's height for the plate, the quest mark and the floating texts: what was measured (a unit
/// without a measure is as tall as a person), a character's limited to the height of a head.
pub(super) fn unit_height(measured: Option<f32>, scale: f32, character: bool) -> f32 {
    let scale = scale.max(0.3);
    let height = measured.filter(|h| *h > 0.0).unwrap_or(1.8 * scale);
    // a character's bounds take in the bow and the hair: the client puts the plate over the head
    if character { height.min(CHARACTER_HEIGHT * scale) } else { height }
}

/// The part of a gap over the head a unit of this height gets: the plate of a rat sits close to the
/// rat (the capture's bar is 0.24 m over its feet), the one of a person 0.3 m over the head.
pub(super) fn head_gap(height: f32, gap: f32) -> f32 {
    gap * (height / 1.8).clamp(0.0, 1.0).sqrt()
}

impl Measure<'_, '_> {
    /// The height of a unit's model above its root, from the bounds of its meshes and joints.
    fn height(&self, root: Entity, base_y: f32) -> Option<f32> {
        // (the joints' global positions are of the last frame's propagation: measure from the root's)
        let base_y = self.globals.get(root).map_or(base_y, |g| g.translation().y);
        let mut top: Option<f32> = None;
        let mut raise = |y: f32| top = Some(top.map_or(y, |t| t.max(y)));
        let mut joint_top: Option<f32> = None;
        let mut stack = vec![root];
        while let Some(e) = stack.pop() {
            if let Ok((aabb, global)) = self.bounds.get(e) {
                let (c, h) = (Vec3::from(aabb.center), Vec3::from(aabb.half_extents));
                for sx in [-1.0, 1.0] {
                    for sy in [-1.0, 1.0] {
                        for sz in [-1.0, 1.0] {
                            raise(global.transform_point(c + h * Vec3::new(sx, sy, sz)).y);
                        }
                    }
                }
            }
            if let Ok(skin) = self.skins.get(e) {
                for j in &skin.joints {
                    // (hair and cloth bones hang loose around the head)
                    if self.names.get(*j).is_ok_and(|n| n.as_str().starts_with("rope")) {
                        continue;
                    }
                    if let Ok(g) = self.globals.get(*j) {
                        let y = g.translation().y;
                        joint_top = Some(joint_top.map_or(y, |t| t.max(y)));
                    }
                }
            }
            if let Ok(kids) = self.children.get(e) {
                stack.extend(kids.iter());
            }
        }
        // (joints that are not posed yet sit at the origin: below the root)
        if let Some(y) = joint_top.filter(|y| *y > base_y) {
            raise(y + SKULL * (y - base_y));
        }
        top.map(|t| (t - base_y).max(0.05))
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn update_plates(
    mut commands: Commands,
    mut state: ResMut<PlateState>,
    font: Res<PlateFont>,
    bridge: NonSend<HudBridge>,
    units: Res<Units>,
    buffs: Res<aa_net::state::Buffs>,
    player: Res<Player>,
    appellations: Res<aa_net::Appellations>,
    target: Res<aa_net::state::Target>,
    world_camera: Query<(&Camera, &Transform), (With<Camera3d>, Without<PlateRoot>)>,
    named: Query<(Entity, &Name)>,
    transforms: Query<&Transform, Without<PlateRoot>>,
    measure: Measure,
    mut roots: Query<(&mut Transform, &mut Visibility), With<PlateRoot>>,
    mut texts: Query<(&mut Text2d, &mut TextColor), Without<Sprite>>,
    mut sprites: Query<&mut Sprite, Without<Text2d>>,
) {
    let Some((camera, camera_transform)) = world_camera.iter().find(|(c, _)| c.is_active) else { return };
    let Some(viewport) = camera.logical_viewport_size() else { return };
    let Some(spawn) = player.spawn.as_ref() else { return };
    let data = bridge.0.borrow();
    let game = data.game.get();
    let me = data.pos;
    let camera_global = GlobalTransform::from(*camera_transform);
    let own = spawn.obj_id;

    // which entity is which unit
    if state.scan_in == 0 {
        state.scan_in = 30;
        let mut found = HashMap::new();
        let own_name = format!("player {}", spawn.name());
        for (entity, name) in &named {
            let n = name.as_str();
            if n == own_name {
                found.insert(own, entity);
            } else if let Some(rest) = n.strip_prefix("unit ")
                && let Some(id) = rest.split(' ').next().and_then(|v| v.parse::<u32>().ok())
            {
                found.insert(id, entity);
            }
        }
        state.entities = found;
        // units that went out of view
        let in_view: std::collections::HashSet<u32> = units.iter().map(|u| u.obj_id).collect();
        state.max_hp.retain(|id, _| in_view.contains(id));
    }
    state.scan_in -= 1;

    // the units that get a plate: the nearest ones in range
    struct Wanted<'a> {
        id: u32,
        distance: f32,
        unit: Option<&'a aa_net::Unit>,
    }
    let mut wanted: Vec<Wanted> = Vec::new();
    if SHOW_OWN {
        wanted.push(Wanted { id: own, distance: 0.0, unit: None });
    }
    for u in units.iter() {
        if u.obj_id == own {
            continue;
        }
        let d = me.distance(Vec3::new(u.pos.x, u.pos.y, u.pos.z));
        let range = if matches!(u.kind, UnitKind::Character { .. }) { PLAYER_RANGE } else { NPC_RANGE };
        if d <= range {
            wanted.push(Wanted { id: u.obj_id, distance: d, unit: Some(u) });
        }
    }
    wanted.sort_by(|a, b| a.distance.total_cmp(&b.distance));
    wanted.truncate(MAX_PLATES);

    let mut seen: Vec<u32> = Vec::with_capacity(wanted.len());
    for w in &wanted {
        let own_plate = w.id == own;
        // name, bar, colours
        let is_character = own_plate || w.unit.is_some_and(|u| matches!(u.kind, UnitKind::Character { .. }));
        let (name, fraction, bar, relation, scale, net_pos) = if own_plate {
            let snap = &data.snapshot;
            (spawn.name().to_string(), (snap.hp as f32 / snap.max_hp.max(1) as f32).clamp(0.0, 1.0), true, "friendly", 1.0, None)
        } else {
            let u = w.unit.unwrap();
            let template = u.kind.template_id();
            let npc = game.and_then(|g| g.npcs.get(&template)).filter(|_| template != 0);
            if npc.is_some_and(|n| !n.show_name_tag) {
                continue;
            }
            let name = if !u.name.is_empty() { u.name.clone() } else { npc.map(|n| n.name.clone()).unwrap_or_default() };
            if name.is_empty() {
                continue;
            }
            let relation = relation_of(game, spawn.character().faction_id, u);
            let faction = unit_faction(game, u);
            if !state.plates.contains_key(&w.id) {
                debug!("hud: plate {} {:?}: faction {} (unit {}) own {} -> {}, entity {:?}", w.id, name, faction, u.faction_id, spawn.character().faction_id, relation, state.entities.get(&w.id));
            }
            let max_hp = {
                let entry = state.max_hp.entry(w.id).or_insert((0, 0));
                if entry.1 == 0 {
                    entry.0 = game.and_then(|g| super::stats::unit_max(g, u, buffs.of(u.obj_id))).map_or(u.max_hp, |m| m.0);
                    entry.1 = 90;
                }
                entry.1 -= 1;
                entry.0
            };
            let fraction = (u.hp as f32 / max_hp.max(u.hp).max(1) as f32).clamp(0.0, 1.0);
            (name, fraction, true, relation, u.scale, Some(Vec3::new(u.pos.x, u.pos.y, u.pos.z)))
        };
        // the selected target has the big name and the framed bar; a corpse a grey name and an empty bar
        let title_id = if own_plate { appellations.active } else { w.unit.map_or(0, |u| u.appellation) };
        let title: &str = if title_id == 0 { "" } else { game.and_then(|g| g.appellations.get(&title_id)).map_or("", |a| a.name.as_str()) };
        let targeted = target.obj_id == Some(w.id) && !own_plate;
        let dead = w.unit.is_some_and(|u| u.dead);
        let away = own_plate && data.afk;
        let fraction = if dead { 0.0 } else { fraction };
        // where its feet are in the 3D scene
        let entity = state.entities.get(&w.id).copied();
        let root_pos = match entity.and_then(|e| transforms.get(e).ok()) {
            Some(t) => t.translation,
            None => match net_pos {
                Some(p) => aa_world::cry_to_bevy(p),
                None => continue,
            },
        };
        // how tall it is
        // measured from the meshes' bounds, which arrive part by part while the model loads: keep the
        // tallest result of the first few seconds
        let height = {
            let frame = state.scan_in;
            let entry = state.heights.entry(w.id).or_insert((0.0, 0));
            // (a model that is still loading has no bounds yet: ask every few frames until it has, a few hundred times)
            let due = if entry.0 <= 0.0 { frame.is_multiple_of(3) } else { frame.is_multiple_of(10) };
            if entry.1 < 400 && due {
                entry.1 += 1;
                if let Some(h) = entity.and_then(|e| measure.height(e, root_pos.y))
                    && h > entry.0
                {
                    // (the first result is the unit at rest: a few more tries take in the late parts, not what the
                    // animations do later, a rat that rears up in the fight is not tall)
                    if entry.0 <= 0.0 {
                        entry.1 = entry.1.max(370);
                    }
                    entry.0 = h;
                    debug!("hud: plate height of unit {} ({}): {:.2} m (scale {:.2})", w.id, name, h, scale);
                }
                if (entry.1 == 20 || entry.1 == 400) && entry.0 <= 0.0 {
                    debug!("hud: no bounds for unit {} ({}), entity {:?}, scale {:.2}", w.id, name, entity, scale);
                }
            }
            unit_height(Some(entry.0), scale, is_character)
        };
        let Ok(screen) = camera.world_to_viewport(&camera_global, root_pos + Vec3::Y * (height + head_gap(height, HEAD_GAP))) else { continue };
        if screen.x < -100.0 || screen.y < -50.0 || screen.x > viewport.x + 100.0 || screen.y > viewport.y + 50.0 {
            continue;
        }
        // to the plate camera's space: origin in the middle, y up; whole pixels keep the bar crisp
        let x = (screen.x - viewport.x / 2.0).round();
        let y = (viewport.y / 2.0 - screen.y).round();
        let (bar_color, text_color) = if dead { ((60, 64, 60), (149, 149, 149)) } else { colors(relation) };
        seen.push(w.id);

        // a plate whose entities went away with the last game
        if state.plates.get(&w.id).is_some_and(|p| roots.get(p.root).is_err()) {
            state.plates.remove(&w.id);
        }
        match state.plates.get_mut(&w.id) {
            Some(p) => {
                if let Ok((mut t, mut v)) = roots.get_mut(p.root) {
                    let at = Vec3::new(x, y, 0.0);
                    if t.translation != at {
                        t.translation = at;
                    }
                    if *v != Visibility::Inherited {
                        *v = Visibility::Inherited;
                    }
                }
                let same_name = is_plate_line(&p.name, title, &name);
                if !same_name || p.color != text_color {
                    if !same_name {
                        p.name = plate_line(title, name);
                    }
                    if let Ok((mut text, mut color)) = texts.get_mut(p.text) {
                        if !same_name {
                            text.0 = p.name.clone();
                        }
                        color.0 = srgb(text_color);
                    }
                }
                if p.color != bar_color || (p.fraction - fraction).abs() > 0.004 {
                    if let Ok(mut s) = sprites.get_mut(p.fill) {
                        s.color = srgb(bar_color);
                        s.custom_size = Some(Vec2::new((BAR_W * fraction).round(), BAR_H));
                    }
                    p.fraction = fraction;
                    p.color = bar_color;
                }
                if p.has_bar != bar {
                    let want = if bar { Visibility::Inherited } else { Visibility::Hidden };
                    commands.entity(p.bar).insert(want);
                    commands.entity(p.fill).insert(want);
                    p.has_bar = bar;
                }
                if p.targeted != targeted || p.dead != dead || p.away != away {
                    restyle(&mut commands, &font, p, targeted, dead, away, is_character);
                }
                if p.color != text_color {
                    commands.entity(p.status).insert(TextColor(srgb(text_color)));
                }
            }
            None => {
                let name = plate_line(title, name);
                let visibility = if bar { Visibility::Inherited } else { Visibility::Hidden };
                let mut text = Entity::PLACEHOLDER;
                let mut fill = Entity::PLACEHOLDER;
                let mut rim = Entity::PLACEHOLDER;
                let mut back = Entity::PLACEHOLDER;
                let mut status = Entity::PLACEHOLDER;
                let layers = RenderLayers::layer(LAYER);
                let root = commands
                    .spawn((
                        Name::new(format!("plate {}", w.id)),
                        PlateRoot,
                        Transform::from_xyz(x, y, 0.0),
                        Visibility::Inherited,
                        layers.clone(),
                    ))
                    .with_children(|c| {
                        // the dark rim (one pixel around the fill)
                        rim = c
                            .spawn((
                                Sprite::from_color(Color::srgba(0.0, 0.0, 0.0, 0.75), Vec2::new(BAR_W + 2.0, BAR_H + 2.0)),
                                Anchor::BOTTOM_CENTER,
                                Transform::from_xyz(0.0, -1.0, 0.1),
                                visibility,
                                layers.clone(),
                            ))
                            .id();
                        back = c
                            .spawn((
                                Sprite::from_color(Color::srgb_u8(20, 24, 22), Vec2::new(BAR_W + 8.0, BAR_H + 4.0)),
                                Anchor::BOTTOM_CENTER,
                                Transform::from_xyz(0.0, -2.0, 0.15),
                                Visibility::Hidden,
                                layers.clone(),
                            ))
                            .id();
                        fill = c
                            .spawn((
                                Sprite::from_color(srgb(bar_color), Vec2::new((BAR_W * fraction).round(), BAR_H)),
                                Anchor::BOTTOM_LEFT,
                                Transform::from_xyz(-BAR_W / 2.0, 0.0, 0.2),
                                visibility,
                                layers.clone(),
                            ))
                            .id();
                        status = c
                            .spawn((
                                Text2d::new("-Away-"),
                                TextFont { font: FontSource::Handle(font.0.clone()), font_size: FontSize::Px(PLAYER_FONT_PX), ..default() },
                                TextColor(srgb(text_color)),
                                Text2dShadow { offset: Vec2::new(1.0, -1.0), color: Color::srgba(0.0, 0.0, 0.0, 0.85) },
                                Anchor::BOTTOM_CENTER,
                                Transform::from_xyz(0.0, BAR_H + 14.0, 0.3),
                                Visibility::Hidden,
                                layers.clone(),
                            ))
                            .id();
                        text = c
                            .spawn((
                                Text2d::new(name.clone()),
                                TextFont { font: FontSource::Handle(font.0.clone()), font_size: FontSize::Px(if is_character { PLAYER_FONT_PX } else { NPC_FONT_PX }), ..default() },
                                TextColor(srgb(text_color)),
                                Text2dShadow { offset: Vec2::new(1.0, -1.0), color: Color::srgba(0.0, 0.0, 0.0, 0.85) },
                                Anchor::BOTTOM_CENTER,
                                // the bar's top is at +6, the client leaves about 8 pixels below the letters
                                Transform::from_xyz(0.0, if !bar { 0.0 } else if is_character { BAR_H + 14.0 } else { BAR_H + 6.0 }, 0.3),
                                layers.clone(),
                            ))
                            .id();
                    })
                    .id();
                state.plates.insert(w.id, Plate { root, text, fill, bar: rim, back, targeted: false, dead: false, status, away: false, has_bar: bar, name, fraction, color: bar_color });
            }
        }
    }
    // the plates of units that are gone, out of range or not on screen
    let gone: Vec<u32> = state.plates.keys().copied().filter(|id| !seen.contains(id)).collect();
    for id in gone {
        let Some(p) = state.plates.get(&id) else { continue };
        if let Ok((_, mut v)) = roots.get_mut(p.root)
            && *v != Visibility::Hidden
        {
            *v = Visibility::Hidden;
        }
        // units that left the view for good lose their plate
        if units.get(id).is_none() && id != own {
            commands.entity(p.root).despawn();
            state.plates.remove(&id);
            state.heights.remove(&id);
        }
    }
}

/// Gives a plate the look of the selected target (the capture's: the name at the player's size, the bar in a yellow
/// frame with a dark inside) or of a corpse (the frame goes, the bar is empty), or back to the plain one.
fn restyle(commands: &mut Commands, font: &PlateFont, p: &mut Plate, targeted: bool, dead: bool, away: bool, is_character: bool) {
    p.targeted = targeted;
    p.dead = dead;
    p.away = away;
    commands.entity(p.status).insert(if away { Visibility::Inherited } else { Visibility::Hidden });
    let framed = targeted && !dead;
    let big = is_character || targeted;
    let (rim_color, rim_size, rim_y) = if framed {
        (Color::srgb_u8(212, 200, 74), Vec2::new(BAR_W + 12.0, BAR_H + 8.0), -4.0)
    } else {
        (Color::srgba(0.0, 0.0, 0.0, 0.75), Vec2::new(BAR_W + 2.0, BAR_H + 2.0), -1.0)
    };
    commands.entity(p.bar).insert((Sprite::from_color(rim_color, rim_size), Transform::from_xyz(0.0, rim_y, 0.1)));
    commands.entity(p.back).insert(if framed { Visibility::Inherited } else { Visibility::Hidden });
    let above_bar = if framed { BAR_H + 8.0 } else { BAR_H + 6.0 };
    commands.entity(p.text).insert((
        TextFont { font: FontSource::Handle(font.0.clone()), font_size: FontSize::Px(if big { PLAYER_FONT_PX } else { NPC_FONT_PX }), ..default() },
        // the status line takes the place over the bar, the name moves up by its height
        Transform::from_xyz(0.0, if big { above_bar + 8.0 } else { above_bar } + if away { AWAY_LINE } else { 0.0 }, 0.3),
    ));
}

/// Pixels the "-Away-" line takes between the name and the bar (the capture: 405 and 428).
const AWAY_LINE: f32 = 23.0;

/// Seconds without a key or mouse input after which the client marks the player away (an assumption: the
/// captures show "-Away-" on a character that had stood idle; the server knows nothing of it).
const AWAY_AFTER_SECS: f32 = 300.0;

/// `--hud-afk`: away at once (for screenshots).
fn forced_away() -> bool {
    std::env::args().any(|a| a == "--hud-afk")
}

/// Tracks the idle time and keeps `HudData::afk`.
#[allow(clippy::too_many_arguments)]
pub fn track_idle(
    time: Res<Time<Real>>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<bevy::input::mouse::AccumulatedMouseMotion>,
    bridge: NonSend<HudBridge>,
    mut idle: Local<f32>,
) {
    let active = keys.get_pressed().next().is_some() || mouse.get_pressed().next().is_some() || motion.delta != Vec2::ZERO;
    if active {
        *idle = 0.0;
    } else {
        *idle += time.delta_secs();
    }
    let away = forced_away() || *idle > AWAY_AFTER_SECS;
    let mut d = bridge.0.borrow_mut();
    if d.afk != away {
        d.afk = away;
    }
}

/// Not with `--no-hud`.
fn wanted(options: Option<Res<super::HudOptions>>) -> bool {
    options.is_none_or(|o| o.enabled)
}

pub fn plugin(app: &mut App) {
    app.init_resource::<PlateFont>()
        .init_resource::<PlateState>()
        .init_resource::<super::marks::MarkState>()
        .add_systems(Startup, load_font)
        .add_systems(OnExit(Screen::InGame), (despawn, super::marks::despawn))
        .add_systems(
            PostUpdate,
            (update_plates, super::marks::update_marks, super::ring::update_ring, super::loot::update_icons).chain().before(TransformSystems::Propagate).run_if(in_state(Screen::InGame)).run_if(wanted),
        )
        .add_systems(Update, (ensure_camera, match_camera, track_idle).chain().run_if(in_state(Screen::InGame)).run_if(wanted));
}

#[cfg(test)]
mod tests {
    use super::{is_plate_line, plate_line};

    #[test]
    fn title_is_a_prefix_on_the_name_line() {
        assert_eq!(plate_line("Autumn's Petal", "Titletest".into()), "Autumn's Petal Titletest");
        assert_eq!(plate_line("", "Titletest".into()), "Titletest");
        assert!(is_plate_line("Autumn's Petal Titletest", "Autumn's Petal", "Titletest"));
        assert!(is_plate_line("Titletest", "", "Titletest"));
        assert!(!is_plate_line("Autumn's Petal Titletest", "", "Titletest"));
        assert!(!is_plate_line("Titletest", "Autumn's Petal", "Titletest"));
    }
}
