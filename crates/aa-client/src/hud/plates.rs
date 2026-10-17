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
const HEAD_GAP: f32 = 0.3;
/// Name sizes: NPCs, other characters (the client draws player names much larger).
const NPC_FONT_PX: f32 = 11.0;
const PLAYER_FONT_PX: f32 = 26.0;
/// Show the player's own plate (off in the client's default settings, as in the capture).
const SHOW_OWN: bool = false;
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

pub fn despawn(mut commands: Commands, cameras: Query<Entity, With<PlateCamera>>, plates: Query<Entity, With<PlateRoot>>) {
    for e in cameras.iter().chain(plates.iter()) {
        commands.entity(e).despawn();
    }
}

/// The plate camera takes the world camera's HDR and MSAA settings (see `cameras.rs`).
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
    has_bar: bool,
    name: String,
    fraction: f32,
    color: (u8, u8, u8),
}

#[derive(Default)]
pub struct PlateState {
    plates: HashMap<u32, Plate>,
    /// Unit object id -> its entity in the 3D scene, found by name every half second or so.
    entities: HashMap<u32, Entity>,
    scan_in: u32,
    /// Unit object id -> height of its model (metres) and the attempts spent measuring it.
    heights: HashMap<u32, (f32, u8)>,
}

fn srgb(c: (u8, u8, u8)) -> Color {
    Color::srgb_u8(c.0, c.1, c.2)
}

/// Bar and name colours by how the unit stands to the player.
fn colors(relation: &str, own: bool) -> ((u8, u8, u8), (u8, u8, u8)) {
    if own {
        return ((255, 255, 255), (255, 255, 255));
    }
    match relation {
        "hostile" => ((160, 32, 26), (230, 70, 50)),
        "friendly" => ((70, 126, 31), (115, 210, 51)),
        _ => ((155, 131, 0), (255, 216, 0)),
    }
}

/// The height of a unit's model above its root, from the bounds of its meshes.
fn measure(root: Entity, children: &Query<&Children>, bounds: &Query<(&Aabb, &GlobalTransform)>, base_y: f32) -> Option<f32> {
    let mut top: Option<f32> = None;
    let mut stack = vec![root];
    while let Some(e) = stack.pop() {
        if let Ok((aabb, global)) = bounds.get(e) {
            let (c, h) = (Vec3::from(aabb.center), Vec3::from(aabb.half_extents));
            for sx in [-1.0, 1.0] {
                for sy in [-1.0, 1.0] {
                    for sz in [-1.0, 1.0] {
                        let p = global.transform_point(c + h * Vec3::new(sx, sy, sz));
                        top = Some(top.map_or(p.y, |t| t.max(p.y)));
                    }
                }
            }
        }
        if let Ok(kids) = children.get(e) {
            stack.extend(kids.iter());
        }
    }
    top.map(|t| (t - base_y).max(0.3))
}

#[allow(clippy::too_many_arguments)]
pub fn update_plates(
    mut commands: Commands,
    mut state: Local<PlateState>,
    font: Res<PlateFont>,
    bridge: NonSend<HudBridge>,
    units: Res<Units>,
    player: Res<Player>,
    world_camera: Query<(&Camera, &Transform), (With<Camera3d>, Without<PlateRoot>)>,
    named: Query<(Entity, &Name)>,
    transforms: Query<&Transform, Without<PlateRoot>>,
    children: Query<&Children>,
    bounds: Query<(&Aabb, &GlobalTransform)>,
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
            } else if let Some(rest) = n.strip_prefix("unit ") {
                if let Some(id) = rest.split(' ').next().and_then(|v| v.parse::<u32>().ok()) {
                    found.insert(id, entity);
                }
            }
        }
        state.entities = found;
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
            (spawn.name().to_string(), 1.0, false, "friendly", 1.0, None)
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
            // an NPC's faction is its template's; the packet's is the nation it was spawned for
            let faction = match npc {
                Some(n) if n.faction_id != 0 => n.faction_id as u32,
                _ => u.faction_id,
            };
            let is_character = matches!(u.kind, UnitKind::Character { .. });
            let mut relation = game.map_or("neutral", |g| g.relation(spawn.character().faction_id, faction));
            // the capture's villagers of the player's own faction (the newbie towns) are yellow, only players are green
            if !is_character && relation == "friendly" && faction == spawn.character().faction_id {
                relation = "neutral";
            }
            if !state.plates.contains_key(&w.id) {
                debug!("hud: plate {} {:?}: faction {} (unit {}) own {} -> {}, entity {:?}", w.id, name, faction, u.faction_id, spawn.character().faction_id, relation, state.entities.get(&w.id));
            }
            let fraction = (u.hp as f32 / u.max_hp.max(u.hp).max(1) as f32).clamp(0.0, 1.0);
            (name, fraction, true, relation, u.scale, Some(Vec3::new(u.pos.x, u.pos.y, u.pos.z)))
        };
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
        let height = {
            let entry = state.heights.entry(w.id).or_insert((0.0, 0));
            if entry.0 <= 0.0 && entry.1 < 60 {
                entry.1 += 1;
                if let Some(h) = entity.and_then(|e| measure(e, &children, &bounds, root_pos.y)) {
                    entry.0 = h;
                    debug!("hud: plate height of unit {} ({}): {:.2} m (scale {:.2})", w.id, name, h, scale);
                }
            }
            if entry.0 > 0.0 { entry.0 } else { 1.8 * scale.max(0.3) }
        };
        let Ok(screen) = camera.world_to_viewport(&camera_global, root_pos + Vec3::Y * (height + HEAD_GAP)) else { continue };
        if screen.x < -100.0 || screen.y < -50.0 || screen.x > viewport.x + 100.0 || screen.y > viewport.y + 50.0 {
            continue;
        }
        // to the plate camera's space: origin in the middle, y up; whole pixels keep the bar crisp
        let x = (screen.x - viewport.x / 2.0).round();
        let y = (viewport.y / 2.0 - screen.y).round();
        let (bar_color, text_color) = colors(relation, own_plate);
        seen.push(w.id);

        // a plate whose entities went away with the last game
        if state.plates.get(&w.id).is_some_and(|p| roots.get(p.root).is_err()) {
            state.plates.remove(&w.id);
        }
        match state.plates.get_mut(&w.id) {
            Some(p) => {
                if let Ok((mut t, mut v)) = roots.get_mut(p.root) {
                    t.translation = Vec3::new(x, y, 0.0);
                    if *v != Visibility::Inherited {
                        *v = Visibility::Inherited;
                    }
                }
                if p.name != name || p.color != text_color {
                    if let Ok((mut text, mut color)) = texts.get_mut(p.text) {
                        if p.name != name {
                            text.0 = name.clone();
                        }
                        color.0 = srgb(text_color);
                    }
                    p.name = name;
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
            }
            None => {
                let visibility = if bar { Visibility::Inherited } else { Visibility::Hidden };
                let mut text = Entity::PLACEHOLDER;
                let mut fill = Entity::PLACEHOLDER;
                let mut rim = Entity::PLACEHOLDER;
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
                        fill = c
                            .spawn((
                                Sprite::from_color(srgb(bar_color), Vec2::new((BAR_W * fraction).round(), BAR_H)),
                                Anchor::BOTTOM_LEFT,
                                Transform::from_xyz(-BAR_W / 2.0, 0.0, 0.2),
                                visibility,
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
                state.plates.insert(w.id, Plate { root, text, fill, bar: rim, has_bar: bar, name, fraction, color: bar_color });
            }
        }
    }
    // the plates of units that are gone, out of range or not on screen
    let gone: Vec<u32> = state.plates.keys().copied().filter(|id| !seen.contains(id)).collect();
    for id in gone {
        let Some(p) = state.plates.get(&id) else { continue };
        if let Ok((_, mut v)) = roots.get_mut(p.root) {
            if *v != Visibility::Hidden {
                *v = Visibility::Hidden;
            }
        }
        // units that left the view for good lose their plate
        if units.get(id).is_none() && id != own {
            commands.entity(p.root).despawn();
            state.plates.remove(&id);
            state.heights.remove(&id);
        }
    }
}

/// Not with `--no-hud`.
fn wanted(options: Option<Res<super::HudOptions>>) -> bool {
    options.is_none_or(|o| o.enabled)
}

pub fn plugin(app: &mut App) {
    app.init_resource::<PlateFont>()
        .add_systems(Startup, load_font)
        .add_systems(OnExit(Screen::InGame), despawn)
        .add_systems(
            PostUpdate,
            update_plates.before(TransformSystems::Propagate).run_if(in_state(Screen::InGame)).run_if(wanted),
        )
        .add_systems(Update, (ensure_camera, match_camera).chain().run_if(in_state(Screen::InGame)).run_if(wanted));
}
