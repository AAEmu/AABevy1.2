//! The ring on the ground under the selected target.
//!
//! The client draws a decal under the unit that is targeted: `materials/decals/target/*.mtl` all use
//! `textures/decal/target/target_enemy04.dds` (an ornate ring in a 512 x 512 texture, the ink fills
//! about nine tenths of it) and differ in their colour: `npc_hostile` / `pc_hostile` red (the capture
//! of `reference/ingame/combat_*`: about (232, 74, 63) at its brightest), `npc_neutral` yellow,
//! `npc_friendly` darker yellow, `pc_friendly` green, `pc_party` blue.
//!
//! The decal is drawn *over* the world, water included (the captures show it through the pond), so it
//! is not a 3D object: a grid of points on the ground around the unit (following the terrain) is
//! projected to the screen every frame and drawn as a textured 2D mesh on the name plate camera. The
//! texture's "up" points away from the camera, as the capture's ring does (the ornaments sit at the
//! near and far ends of the ellipse).

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use aa_net::model::UnitKind;
use aa_net::state::{Player, Target, Units};
use aa_world::{TerrainQuery, bevy_to_cry, cry_to_bevy};

use super::api::HudBridge;
use super::plates::{PlateState, relation_of};
use crate::Screen;

/// Where the decal texture is in the pak.
const TEXTURE: &str = "textures/decal/target/target_enemy04.dds";
/// Fraction of the quad the ring's ink spans.
const INK: f32 = 0.9;
/// Outer diameter of the ink, metres, at least (the Angry Rat of the captures, scale 1.5, is 1.28 m).
const MIN_DIAMETER: f32 = 1.2;
/// Metres over the ground.
const LIFT: f32 = 0.03;
/// Cells per side of the projected grid.
const GRID: usize = 12;
/// The plate camera's render layer (`plates::LAYER`).
const LAYER: usize = 30;

#[derive(Component)]
pub struct TargetRing;

#[derive(Resource, Default)]
pub struct RingAssets {
    mesh: Option<Handle<Mesh>>,
    image: Option<Handle<Image>>,
    /// One material per colour name.
    materials: Vec<(&'static str, Handle<ColorMaterial>)>,
}

/// The ring colour of how the target stands to the player (the decal materials' tints, as bright as the capture).
fn tint(relation: &str, character: bool) -> (&'static str, Color) {
    match (relation, character) {
        ("hostile", _) => ("hostile", Color::srgb_u8(250, 80, 68)),
        ("friendly", true) => ("friendly player", Color::srgb_u8(70, 205, 80)),
        ("friendly", false) => ("friendly", Color::srgb_u8(230, 186, 40)),
        _ => ("neutral", Color::srgb_u8(255, 222, 60)),
    }
}

fn load_image() -> Option<Image> {
    dds_image(TEXTURE)
}

/// A DDS of the pak as a texture (linear filtering).
pub(super) fn dds_image(path: &str) -> Option<Image> {
    let bytes = aa_cry::read(path).ok()?;
    let dds = aa_cry::texture::Dds::parse(&bytes).ok()?;
    let (w, h) = (dds.width, dds.height);
    let mut image = Image::new(Extent3d { width: w, height: h, depth_or_array_layers: 1 }, TextureDimension::D2, dds.decode_rgba8(0, 0), TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::RENDER_WORLD);
    image.sampler = bevy::image::ImageSampler::linear();
    Some(image)
}

/// The grid's mesh: `(GRID + 1)^2` vertices (positions are filled in every frame), texture coordinates over the quad.
fn grid_mesh() -> Mesh {
    let n = GRID + 1;
    let mut uv = Vec::with_capacity(n * n);
    for j in 0..n {
        for i in 0..n {
            uv.push([i as f32 / GRID as f32, j as f32 / GRID as f32]);
        }
    }
    let mut idx = Vec::with_capacity(GRID * GRID * 6);
    for j in 0..GRID {
        for i in 0..GRID {
            let a = (j * n + i) as u32;
            let (b, c, d) = (a + 1, a + n as u32, a + n as u32 + 1);
            idx.extend([a, c, b, b, c, d]);
        }
    }
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]; n * n])
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
        .with_inserted_indices(Indices::U32(idx))
}

/// Keeps the ring under the selected target: spawned on selection, moved with the unit, gone with the selection.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn update_ring(
    mut commands: Commands,
    target: Res<Target>,
    units: Res<Units>,
    player: Res<Player>,
    bridge: NonSend<HudBridge>,
    plates: Res<PlateState>,
    terrain: Option<Res<TerrainQuery>>,
    mut assets: ResMut<RingAssets>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    world_camera: Query<(&Camera, &Transform), (With<Camera3d>, Without<TargetRing>)>,
    transforms: Query<&Transform, (Without<TargetRing>, Without<Camera3d>)>,
    mut rings: Query<(Entity, &mut Visibility, &mut MeshMaterial2d<ColorMaterial>), With<TargetRing>>,
) {
    let own = player.spawn.as_ref().map(|s| s.obj_id);
    let unit = target.obj_id.filter(|id| Some(*id) != own).and_then(|id| units.get(id));
    let Some(unit) = unit else {
        for (e, _, _) in &rings {
            commands.entity(e).despawn();
        }
        return;
    };
    let Some((camera, camera_transform)) = world_camera.iter().find(|(c, _)| c.is_active) else { return };
    let Some(viewport) = camera.logical_viewport_size() else { return };
    let data = bridge.0.borrow();
    let game = data.game.get();
    let own_faction = player.spawn.as_ref().map_or(0, |s| s.character().faction_id);
    let relation = relation_of(game, own_faction, unit);
    let character = matches!(unit.kind, UnitKind::Character { .. });
    let (name, color) = tint(relation, character);

    // the assets, made when the first ring is wanted
    if assets.mesh.is_none() {
        assets.mesh = Some(meshes.add(grid_mesh()));
    }
    if assets.image.is_none() {
        match load_image() {
            Some(image) => assets.image = Some(images.add(image)),
            None => {
                warn!("hud: cannot read {TEXTURE}: no target ring");
                return;
            }
        }
    }
    let material = match assets.materials.iter().find(|(n, _)| *n == name) {
        Some((_, m)) => m.clone(),
        None => {
            let m = materials.add(ColorMaterial { color, texture: assets.image.clone(), ..default() });
            assets.materials.push((name, m.clone()));
            m
        }
    };

    // where it stands (the unit's entity once it has one: it is smoothed towards the server's position)
    let game_pos = match plates.entities.get(&unit.obj_id).and_then(|e| transforms.get(*e).ok()) {
        Some(t) => bevy_to_cry(t.translation),
        None => Vec3::new(unit.pos.x, unit.pos.y, unit.pos.z),
    };
    let ground_at = |x: f32, y: f32| terrain.as_ref().and_then(|t| t.ground_at(x, y)).map_or(game_pos.z, |ground| ground.height);
    let diameter = MIN_DIAMETER.max(0.95 * unit.scale.max(0.1));
    let size = diameter / INK;
    // the ground basis: "up" of the texture is away from the camera
    let camera_global = GlobalTransform::from(*camera_transform);
    let center = cry_to_bevy(game_pos);
    let away = {
        let f = center - camera_transform.translation;
        Vec3::new(f.x, 0.0, f.z).normalize_or(Vec3::NEG_Z)
    };
    let right = Vec3::new(-away.z, 0.0, away.x);
    // vertices in plate camera coordinates
    let n = GRID + 1;
    let mut positions: Vec<[f32; 3]> = Vec::with_capacity(n * n);
    for j in 0..n {
        for i in 0..n {
            let (u, v) = (i as f32 / GRID as f32 - 0.5, 0.5 - j as f32 / GRID as f32);
            let flat = center + right * (u * size) + away * (v * size);
            let g = bevy_to_cry(flat);
            let ground = ground_at(g.x, g.y) + LIFT;
            let world = cry_to_bevy(Vec3::new(g.x, g.y, ground));
            match camera.world_to_viewport(&camera_global, world) {
                Ok(s) => positions.push([s.x - viewport.x / 2.0, viewport.y / 2.0 - s.y, 0.0]),
                Err(_) => {
                    // part of the ring is behind the camera: not drawn
                    for (_, mut v, _) in &mut rings {
                        *v = Visibility::Hidden;
                    }
                    return;
                }
            }
        }
    }
    if let Some(mut mesh) = meshes.get_mut(assets.mesh.as_ref().unwrap()) {
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    }
    match rings.iter_mut().next() {
        Some((_, mut v, mut m)) => {
            if *v != Visibility::Inherited {
                *v = Visibility::Inherited;
            }
            if m.0 != material {
                m.0 = material;
            }
        }
        None => {
            debug!("hud: target ring under unit {} ({name}) at {center:?}, quad {size:.2} m, unit z {:.2}, ground {:?}, net z {:.2}", unit.obj_id, game_pos.z, terrain.as_ref().and_then(|t| t.height_at(game_pos.x, game_pos.y)), unit.pos.z);
            commands.spawn((
                Name::new("target ring"),
                TargetRing,
                Mesh2d(assets.mesh.clone().unwrap()),
                MeshMaterial2d(material),
                // under the plates and the floating texts (z 0 and up)
                Transform::from_xyz(0.0, 0.0, -1.0),
                Visibility::Inherited,
                NoFrustumCulling,
                RenderLayers::layer(LAYER),
            ));
        }
    }
}

pub fn despawn(mut commands: Commands, rings: Query<Entity, With<TargetRing>>) {
    for e in &rings {
        commands.entity(e).despawn();
    }
}

pub fn plugin(app: &mut App) {
    // `update_ring` runs with the plates (`plates::plugin`), over the camera's final position
    app.init_resource::<RingAssets>().add_systems(OnExit(Screen::InGame), despawn);
}
