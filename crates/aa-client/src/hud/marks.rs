//! Quest markers over NPCs: the yellow "!" of a quest to take, the yellow "?" of one to hand in and
//! the white "?" of one in progress, floating above the name plate and turning slowly, like the
//! original's `objects/ui/quest_mark/*` models (skinned `.chr` files; the `.cdf` only picks the
//! material and a particle effect, which is not drawn here).
//!
//! Which NPC gets which marker is [`super::quests`]; this module spawns a model per marked NPC in
//! range, keeps it above the NPC's head and takes it away when the quest state changes.

use std::collections::{HashMap, HashSet};

use aa_cry::{CryAssets, MaterialHook, ModelOptions};
use aa_net::model::UnitKind;
use aa_net::state::{Buffs, Inventory, Player, PlayerStats, Quests, Skills, Units};
use bevy::prelude::*;

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
}

/// The model and material of a marker.
fn variant(mark: Mark) -> (&'static str, &'static str) {
    match mark {
        Mark::Available => ("objects/ui/quest_mark/exclamation_mark_red/exclamation_mark_red.chr", "objects/ui/quest_mark/exclamation_mark_yellow/exclamation_mark_yellow_mul.mtl"),
        Mark::Ready => ("objects/ui/quest_mark/question_mark_red/question_mark_red.chr", "objects/ui/quest_mark/question_mark_yellow/question_mark_yellow_mul.mtl"),
        Mark::InProgress => ("objects/ui/quest_mark/question_mark_red/question_mark_red.chr", "objects/ui/quest_mark/question_mark_ing/question_mark_ing_mul.mtl"),
    }
}

/// Unlit and bright like the original's glowing marks; the shell sub-material without a texture is invisible.
fn options(material: &str) -> ModelOptions {
    ModelOptions {
        material: Some(material.to_string()),
        material_hook: Some(MaterialHook::new(|_, m| {
            m.unlit = true;
            m.double_sided = true;
            m.cull_mode = None;
            if m.base_color_texture.is_none() {
                m.base_color = Color::NONE;
                m.alpha_mode = AlphaMode::Blend;
            } else {
                m.base_color = Color::linear_rgb(1.5, 1.5, 1.5);
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
            let opts = options(material);
            match cry.load_model(model, &opts) {
                Ok(loaded) => {
                    let root = aa_cry::scene::spawn_loaded(&mut commands, model, &loaded, &opts);
                    let anchor = commands
                        .spawn((Name::new(format!("quest mark {id}")), QuestMark, Transform::from_scale(Vec3::splat(SCALE)), Visibility::Inherited))
                        .add_child(root)
                        .id();
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
        let scale = units.get(*id).map_or(1.0, |u| u.scale.max(0.3));
        let height = plates.heights.get(id).map(|h| h.0).filter(|h| *h > 0.0).unwrap_or(1.8 * scale);
        transform.translation = root + Vec3::Y * (height + super::plates::HEAD_GAP + GAP + BOB * (t * 3.0).sin());
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
    *state = MarkState::default();
}
