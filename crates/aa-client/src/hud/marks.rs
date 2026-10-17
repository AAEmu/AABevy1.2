//! Quest markers over NPCs: the yellow "!" of a quest to take, the yellow "?" of one to hand in and
//! the white "?" of one in progress, floating above the name plate and turning slowly, like the
//! original's `objects/ui/quest_mark/*` models (skinned `.chr` files; the `.cdf` picks the material and
//! the light beam particle effect `X2_EFFECTS.doodad_effect.quest_mark_addon_*_loop`, drawn by `aa-fx`).
//!
//! Which NPC gets which marker is [`super::quests`]; this module spawns a model per marked NPC in
//! range, keeps it above the NPC's head and takes it away when the quest state changes.

use std::collections::{HashMap, HashSet};

use aa_cry::{CryAssets, MaterialHook, ModelOptions};
use aa_net::model::UnitKind;
use aa_net::state::{Buffs, Inventory, Player, PlayerStats, Quests, Skills, Units};
use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use super::api::HudBridge;
use super::plates::PlateState;
use super::quests::{Mark, PlayerFacts};

/// How far from the player the markers show, metres.
const RANGE: f32 = 45.0;
/// Frames between two looks at who has a marker.
const REFRESH_FRAMES: u32 = 30;
/// Metres between the top of the name plate's anchor and the foot of the marker.
const GAP: f32 = 0.72;
/// The bobbing is `BOB` metres high; the marker turns `SPIN` radians per second.
const BOB: f32 = 0.05;
const SPIN: f32 = 1.2;
/// The model's size to the world's: the model is 0.6 m high, the original's mark about 1.2 m.
const SCALE: f32 = 2.0;

/// A marker entity, see [`update_marks`].
#[derive(Component)]
pub struct QuestMark;

struct Placed {
    entity: Entity,
    mark: Mark,
}

#[derive(Resource, Default)]
pub struct MarkState {
    placed: HashMap<u32, Placed>,
    refresh_in: u32,
    /// The gradient image shared by the yellow marks (kept when the state is reset).
    ramp: Option<Handle<Image>>,
}

/// The model and material of a marker.
fn variant(mark: Mark) -> (&'static str, &'static str) {
    match mark {
        Mark::Available => ("objects/ui/quest_mark/exclamation_mark_red/exclamation_mark_red.chr", "objects/ui/quest_mark/exclamation_mark_yellow/exclamation_mark_yellow_mul.mtl"),
        Mark::Ready => ("objects/ui/quest_mark/question_mark_red/question_mark_red.chr", "objects/ui/quest_mark/question_mark_yellow/question_mark_yellow_mul.mtl"),
        Mark::InProgress => ("objects/ui/quest_mark/question_mark_red/question_mark_red.chr", "objects/ui/quest_mark/question_mark_ing/question_mark_ing_mul.mtl"),
    }
}

/// The colours the yellow marks are drawn with, as scene-linear values before the world's post pass
/// (fog, grade and film curve): the top and the bottom of the model's texture gradient. The post pass
/// pulls a bright colour towards grey and the original's mark is not washed out, so these are worked out
/// to come out of it as the capture's (217, 214, 70) at the top and (211, 164, 54) below: the blue is
/// negative, which a float texture holds and the unlit shader passes on.
const RAMP_TOP: [f32; 3] = [0.71, 0.685, -0.106];
const RAMP_BOTTOM: [f32; 3] = [0.658, 0.24, -0.038];

/// `f32` to half float bits (the range here is small: no infinities, subnormals flush to zero).
fn half_bits(x: f32) -> u16 {
    let b = x.to_bits();
    let sign = ((b >> 16) & 0x8000) as u16;
    let exp = ((b >> 23) & 0xff) as i32 - 127 + 15;
    if exp <= 0 {
        return sign;
    }
    if exp >= 31 {
        return sign | 0x7bff;
    }
    sign | ((exp as u16) << 10) | ((b & 0x7f_ffff) >> 13) as u16
}

/// How far down the original texture's gradient a row is: its green channel runs from yellow (223) at
/// the top to orange (146) from five eighths down.
fn gradient(v: f32) -> f32 {
    const G: [(f32, f32); 6] = [(0.0, 223.0), (0.125, 219.0), (0.25, 201.0), (0.375, 178.0), (0.5, 156.0), (0.625, 146.0)];
    let g = match G.iter().position(|(at, _)| *at >= v) {
        None => 146.0,
        Some(0) => 223.0,
        Some(i) => {
            let ((a, ga), (b, gb)) = (G[i - 1], G[i]);
            ga + (gb - ga) * (v - a) / (b - a)
        }
    };
    (g - 146.0) / (223.0 - 146.0)
}

/// The gradient image of the yellow marks (two columns, 128 rows: the original texture's layout).
fn ramp_image() -> Image {
    const H: usize = 128;
    let (top, bottom) = (RAMP_TOP, RAMP_BOTTOM);
    let mut data = Vec::with_capacity(2 * H * 8);
    for y in 0..H {
        let t = gradient((y as f32 + 0.5) / H as f32);
        for _ in 0..2 {
            for c in 0..3 {
                data.extend_from_slice(&half_bits(bottom[c] + (top[c] - bottom[c]) * t).to_le_bytes());
            }
            data.extend_from_slice(&half_bits(1.0).to_le_bytes());
        }
    }
    Image::new(Extent3d { width: 2, height: H as u32, depth_or_array_layers: 1 }, TextureDimension::D2, data, TextureFormat::Rgba16Float, RenderAssetUsages::RENDER_WORLD)
}

/// Unlit like the original's glowing marks; the shell sub-material without a texture is invisible.
/// `ramp` replaces the texture of the yellow marks (see [`RAMP_TOP`]).
fn options(material: &str, ramp: Option<Handle<Image>>) -> ModelOptions {
    ModelOptions {
        material: Some(material.to_string()),
        material_hook: Some(MaterialHook::new(move |_, m| {
            m.unlit = true;
            m.double_sided = true;
            m.cull_mode = None;
            if m.base_color_texture.is_none() {
                m.base_color = Color::NONE;
                m.alpha_mode = AlphaMode::Blend;
            } else if let Some(ramp) = &ramp {
                m.base_color = Color::WHITE;
                m.base_color_texture = Some(ramp.clone());
            } else {
                m.base_color = Color::WHITE;
            }
        })),
        ..default()
    }
}

#[allow(clippy::too_many_arguments)]
pub fn update_marks(
    mut commands: Commands,
    mut cry: CryAssets,
    mut state: ResMut<MarkState>,
    time: Res<Time>,
    plates: Res<PlateState>,
    bridge: NonSend<HudBridge>,
    units: Res<Units>,
    player: Res<Player>,
    quests: Res<Quests>,
    inventory: Res<Inventory>,
    skills: Res<Skills>,
    stats: Res<PlayerStats>,
    buffs: Res<Buffs>,
    transforms: Query<&Transform, Without<QuestMark>>,
    mut marks: Query<(&mut Transform, &mut Visibility), With<QuestMark>>,
) {
    let Some(spawn) = player.spawn.as_ref() else { return };
    let data = bridge.0.borrow();
    let Some(game) = data.game.get() else { return };
    let me = data.pos;
    if state.refresh_in == 0 {
        state.refresh_in = REFRESH_FRAMES;
        let character = spawn.character();
        let facts = PlayerFacts {
            level: stats.level.max(character.level),
            race: character.race,
            gender: character.gender,
            faction: character.faction_id,
            honor: stats.honor,
            vocation: stats.vocation,
            items: inventory.bag_items().map(|(_, it)| it.template_id).chain(inventory.equipment.slots.iter().flatten().map(|it| it.template_id)).collect(),
            skills: skills.known.iter().map(|k| k.id).collect(),
            buffs: buffs.of(spawn.obj_id).iter().map(|b| b.buff.buff_id).collect(),
            equipped: inventory.equipment.slots.iter().flatten().map(|it| it.template_id).collect(),
            quests: Some(&quests),
        };
        debug!("hud: quests: {} active, {} completed blocks, level {}", quests.active.len(), quests.completed.len(), facts.level);
        let mut wanted: HashMap<u32, Mark> = HashMap::new();
        for u in units.iter() {
            let UnitKind::Npc { template_id, .. } = u.kind else { continue };
            if me.distance(Vec3::new(u.pos.x, u.pos.y, u.pos.z)) > RANGE {
                continue;
            }
            if let Some(mark) = game.quests.mark(template_id, &facts) {
                if !state.placed.contains_key(&u.obj_id) {
                    debug!("hud: quest mark {mark:?} on {} (npc {template_id})", u.name);
                }
                wanted.insert(u.obj_id, mark);
            }
        }
        // what changed
        let stale: Vec<u32> = state.placed.iter().filter(|(id, p)| wanted.get(id) != Some(&p.mark)).map(|(id, _)| *id).collect();
        for id in stale {
            if let Some(p) = state.placed.remove(&id) {
                commands.entity(p.entity).try_despawn();
            }
        }
        for (id, mark) in wanted {
            if state.placed.contains_key(&id) {
                continue;
            }
            let (model, material) = variant(mark);
            let yellow = mark != Mark::InProgress;
            let ramp = yellow.then(|| state.ramp.get_or_insert_with(|| cry.images.add(ramp_image())).clone());
            let opts = options(material, ramp);
            match cry.load_model(model, &opts) {
                Ok(loaded) => {
                    let root = aa_cry::scene::spawn_loaded(&mut commands, model, &loaded, &opts);
                    let anchor = commands
                        .spawn((Name::new(format!("quest mark {id}")), QuestMark, Transform::from_scale(Vec3::splat(SCALE)), Visibility::Inherited))
                        .add_child(root)
                        .id();
                    // the original's `.cdf` attaches a light beam effect to the model's root bone (white for "in progress")
                    let fx = if mark == Mark::InProgress { "doodad_effect.quest_mark_addon_white_loop" } else { "doodad_effect.quest_mark_addon_yellow_loop" };
                    commands.spawn((aa_fx::Effect::new(format!("X2_EFFECTS.{fx}")), ChildOf(anchor)));
                    state.placed.insert(id, Placed { entity: anchor, mark });
                }
                Err(e) => warn!("hud: quest mark {model}: {e}"),
            }
        }
    }
    state.refresh_in -= 1;

    // above each marked NPC's head, turning and bobbing
    let t = time.elapsed_secs();
    let in_view: HashSet<u32> = units.iter().map(|u| u.obj_id).collect();
    let gone: Vec<u32> = state.placed.keys().copied().filter(|id| !in_view.contains(id)).collect();
    for id in gone {
        if let Some(p) = state.placed.remove(&id) {
            commands.entity(p.entity).try_despawn();
        }
    }
    for (id, placed) in &state.placed {
        let Ok((mut transform, mut visibility)) = marks.get_mut(placed.entity) else { continue };
        let root = plates.entities.get(id).and_then(|e| transforms.get(*e).ok()).map(|t| t.translation).or_else(|| units.get(*id).map(|u| aa_world::cry_to_bevy(Vec3::new(u.pos.x, u.pos.y, u.pos.z))));
        let Some(root) = root else { continue };
        let scale = units.get(*id).map_or(1.0, |u| u.scale);
        let height = super::plates::unit_height(plates.heights.get(id).map(|h| h.0), scale, false);
        transform.translation = root + Vec3::Y * (height + super::plates::head_gap(height, super::plates::HEAD_GAP) + GAP + BOB * (t * 3.0).sin());
        transform.rotation = Quat::from_rotation_y(t * SPIN);
        if *visibility != Visibility::Inherited {
            *visibility = Visibility::Inherited;
        }
    }
}

/// The markers go with the game.
pub fn despawn(mut commands: Commands, mut state: ResMut<MarkState>, marks: Query<Entity, With<QuestMark>>) {
    for e in &marks {
        commands.entity(e).try_despawn();
    }
    let ramp = state.ramp.take();
    *state = MarkState { ramp, ..MarkState::default() };
}
