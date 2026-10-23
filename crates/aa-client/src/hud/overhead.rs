//! The raid marks over the heads of units: the numbers, heart, star and cross the raid leader puts on
//! members, monsters and anything else (`PartyCommand::SetMarker`).
//!
//! The marks come from the received team (`aa_net::party::Party::marks`, twelve slots, resolved to
//! units by `party::marker_object`). The original draws them as camera-facing world billboards, anchor
//! bottom centre, from cell `index` of the 4 x 3 atlas `ui/hud/overhead_mark.dds` (64 x 75 pixels a cell;
//! native `3940b340` / `3940bcf0`, see `docs/aa-ui.md`), over the unit's name tag and only while the tag is
//! shown. Here the mark is a sprite on the name plate (`plates.rs`), scaled with the distance like a world
//! object. The sizes are the native cvar defaults read from a running client: 48 x 54 units times
//! `name_tag_render_size` (100) x 0.0001 = 0.48 x 0.54 m (the cell is shrunk from 64 x 75), its foot 0.10 m above the
//! top line of the tag. A marked unit's tag is shown whatever the name tag options say.

use std::collections::HashMap;

use aa_net::state::{Player, Units};
use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::sprite::Anchor;

use super::api::HudBridge;
use super::plates::PlateState;

const ATLAS: &str = "ui/hud/overhead_mark.dds";
/// One cell of the atlas, pixels.
const CELL: UVec2 = UVec2::new(64, 75);
/// Cells across and down.
const GRID: UVec2 = UVec2::new(4, 3);
/// The billboard's height in the world, metres.
const WORLD_HEIGHT: f32 = 0.54;
/// Metres between the top of the name and the foot of the mark.
const GAP: f32 = 0.10;
/// The mark is never smaller or bigger than these fractions of its cell.
const MIN_SCALE: f32 = 0.3;
const MAX_SCALE: f32 = 1.5;
/// A text line is this many times its font size high.
const LINE: f32 = 1.15;

/// A mark sprite.
#[derive(Component)]
pub struct OverheadMark;

#[derive(Resource, Default)]
pub struct OverheadState {
    image: Option<Handle<Image>>,
    layout: Option<Handle<TextureAtlasLayout>>,
    /// Marked unit -> (its sprite, the atlas cell shown).
    placed: HashMap<u32, (Entity, usize)>,
}

fn atlas_image() -> Option<Image> {
    let bytes = aa_cry::read(ATLAS).ok()?;
    let dds = aa_cry::texture::Dds::parse(&bytes).ok()?;
    let mut image = Image::new(
        Extent3d { width: dds.width, height: dds.height, depth_or_array_layers: 1 },
        TextureDimension::D2,
        dds.decode_rgba8(0, 0),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = bevy::image::ImageSampler::linear();
    Some(image)
}

/// Pixels a metre is high at `distance` metres from the camera.
fn pixels_per_metre(viewport_height: f32, fov_y: f32, distance: f32) -> f32 {
    viewport_height / (2.0 * distance.max(0.5) * (fov_y / 2.0).tan())
}

/// The sprite scale (1: the cell's own pixels) of a mark at that many pixels per metre.
fn mark_scale(pixels_per_metre: f32) -> f32 {
    (WORLD_HEIGHT * pixels_per_metre / CELL.y as f32).clamp(MIN_SCALE, MAX_SCALE)
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn update_overhead(
    mut commands: Commands,
    mut state: ResMut<OverheadState>,
    mut images: ResMut<Assets<Image>>,
    mut layouts: ResMut<Assets<TextureAtlasLayout>>,
    plates: Res<PlateState>,
    bridge: NonSend<HudBridge>,
    units: Res<Units>,
    player: Res<Player>,
    world_camera: Query<(&Camera, &Transform, &Projection), (With<Camera3d>, Without<OverheadMark>)>,
    texts: Query<(&Transform, &TextFont, &Text2d), Without<OverheadMark>>,
    mut marks: Query<&mut Transform, With<OverheadMark>>,
) {
    let Some(spawn) = player.spawn.as_ref() else { return };
    let data = bridge.0.borrow();
    let party = &data.party;
    let own_obj = Some(spawn.obj_id);
    let target = data.panels.target.as_ref();
    // the marked units and their cells
    let mut wanted: HashMap<u32, usize> = HashMap::new();
    if party.id != 0 {
        for (cell, mark) in party.marks.iter().enumerate() {
            if let Some(obj) = super::party::marker_object(*mark, party, data.character_id, own_obj, target, Some(&units)) {
                wanted.entry(obj).or_insert(cell);
            }
        }
    }
    // whatever is not marked any more, or lost its sprite with the game
    let stale: Vec<u32> = state.placed.iter().filter(|(id, (entity, cell))| wanted.get(id) != Some(cell) || marks.get(*entity).is_err()).map(|(id, _)| *id).collect();
    for id in stale {
        if let Some((entity, _)) = state.placed.remove(&id) {
            commands.entity(entity).try_despawn();
        }
    }
    if wanted.is_empty() {
        return;
    }
    let Some((camera, camera_transform, projection)) = world_camera.iter().find(|(c, _, _)| c.is_active) else { return };
    let Some(viewport) = camera.logical_viewport_size() else { return };
    let fov = match projection {
        Projection::Perspective(p) => p.fov,
        _ => return,
    };
    if state.image.is_none() {
        let Some(image) = atlas_image() else {
            warn!("hud: {ATLAS} unreadable, no raid marks");
            return;
        };
        state.image = Some(images.add(image));
        state.layout = Some(layouts.add(TextureAtlasLayout::from_grid(CELL, GRID.x, GRID.y, None, None)));
    }
    let (Some(image), Some(layout)) = (state.image.clone(), state.layout.clone()) else { return };
    for (obj, cell) in wanted {
        // on the plate of the unit, over its name; a unit without a plate (no name tag, out of range) shows no mark
        let Some((root, text)) = plates.parts(obj) else { continue };
        let Ok((text_at, font, content)) = texts.get(text) else { continue };
        let lines = content.0.lines().count().max(1) as f32;
        let size = match font.font_size {
            bevy::text::FontSize::Px(px) => px,
            _ => 26.0,
        };
        let distance = units
            .get(obj)
            .map(|u| aa_world::cry_to_bevy(Vec3::new(u.pos.x, u.pos.y, u.pos.z)))
            .or_else(|| (obj == spawn.obj_id).then(|| aa_world::cry_to_bevy(data.pos)))
            .map_or(20.0, |p| p.distance(camera_transform.translation));
        let metre = pixels_per_metre(viewport.y, fov, distance);
        let scale = mark_scale(metre);
        let at = Transform::from_xyz(0.0, text_at.translation.y + size * lines * LINE + GAP * metre, 0.4).with_scale(Vec3::splat(scale));
        match state.placed.get(&obj) {
            Some((entity, _)) => {
                if let Ok(mut transform) = marks.get_mut(*entity) {
                    *transform = at;
                }
            }
            None => {
                let mut sprite = Sprite::from_atlas_image(image.clone(), TextureAtlas { layout: layout.clone(), index: cell });
                sprite.custom_size = Some(CELL.as_vec2());
                let entity = commands
                    .spawn((Name::new(format!("overhead mark {obj}")), OverheadMark, sprite, Anchor::BOTTOM_CENTER, at, RenderLayers::layer(super::plates::LAYER), ChildOf(root)))
                    .id();
                state.placed.insert(obj, (entity, cell));
            }
        }
    }
}

/// The marks go with the game.
pub fn despawn(mut commands: Commands, mut state: ResMut<OverheadState>, marks: Query<Entity, With<OverheadMark>>) {
    for e in &marks {
        commands.entity(e).try_despawn();
    }
    let (image, layout) = (state.image.take(), state.layout.take());
    *state = OverheadState { image, layout, ..default() };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_shrink_with_the_distance_and_stay_readable() {
        let near = mark_scale(pixels_per_metre(1080.0, 1.0, 5.0));
        let far = mark_scale(pixels_per_metre(1080.0, 1.0, 40.0));
        assert!(near > far);
        assert!((MIN_SCALE..=MAX_SCALE).contains(&near) && (MIN_SCALE..=MAX_SCALE).contains(&far));
        assert_eq!(mark_scale(0.0), MIN_SCALE);
        assert_eq!(mark_scale(1.0e6), MAX_SCALE);
    }

    #[test]
    fn the_atlas_is_twelve_cells() {
        assert_eq!(GRID.x * GRID.y, 12);
        assert_eq!(GRID.x * CELL.x, 256);
    }
}
