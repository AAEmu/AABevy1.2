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

/// Where the target ring's unit stands (game coordinates): the drawn root's current `Transform` chain, else
/// the parent-composed packet pose. An attached child's root `Transform` is parent-local, so composing the
/// chain puts the ring on the ship rather than at its local offset.
fn target_point<F: bevy::ecs::query::QueryFilter>(
    unit: &aa_net::model::Unit,
    plates: &PlateState,
    units: &Units,
    transforms: &Query<&Transform, F>,
    parents: &Query<&ChildOf>,
) -> Vec3 {
    match plates
        .entities
        .get(&unit.obj_id)
        .and_then(|e| crate::ingame::units::current_pose(*e, transforms, parents))
    {
        Some((pos, _)) => pos,
        None => crate::ingame::units::unit_world(unit, units).unwrap_or(Vec3::new(unit.pos.x, unit.pos.y, unit.pos.z)),
    }
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
    world_camera: Query<(&Camera, &Transform), (With<Camera3d>, Without<TargetRing>, Without<crate::hud::ModelViewCamera>)>,
    transforms: Query<&Transform, (Without<TargetRing>, Without<Camera3d>)>,
    parents: Query<&ChildOf>,
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

    let game_pos = target_point(unit, &plates, &units, &transforms, &parents);
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

#[cfg(test)]
mod attached_ring_tests {
    use super::*;
    use aa_net::model::{Unit, UnitKind, UnitStateAttachments};
    use aa_net::state::Units;

    #[derive(Component)]
    struct ShipRoot;
    #[derive(Resource, Default)]
    struct Seen(Option<Vec3>);
    #[derive(Resource, Default)]
    struct MoveShip(bool);

    fn move_ship(mut moved: ResMut<MoveShip>, mut roots: Query<&mut Transform, With<ShipRoot>>) {
        if moved.0
            && let Ok(mut transform) = roots.single_mut()
        {
            transform.translation += Vec3::new(10.0, 0.0, 0.0);
        }
    }

    fn read_ring(
        plates: Res<PlateState>,
        units: Res<Units>,
        transforms: Query<&Transform, (Without<TargetRing>, Without<Camera3d>)>,
        parents: Query<&ChildOf>,
        mut seen: ResMut<Seen>,
    ) {
        seen.0 = units
            .get(7)
            .map(|u| target_point(u, &plates, &units, &transforms, &parents));
    }

    fn cannon(id: u32, parent: u32, local: Vec3) -> Unit {
        let mut unit = crate::hud::fixture_spawn().unit;
        unit.obj_id = id;
        unit.kind = UnitKind::Slave {
            id: 1,
            tl_id: 1,
            template_id: 1,
            summoner_id: 0,
        };
        unit.pos = aa_net::Pos::new(local.x, local.y, local.z);
        unit.state_attachments = Some(UnitStateAttachments {
            first_point: 3,
            first_obj_id: Some(parent),
            bonding_obj_id: None,
        });
        unit
    }

    #[test]
    fn an_attached_ring_follows_its_parent_in_the_same_update() {
        let ship_at = Vec3::new(12000.0, 9000.0, 0.0);
        let local = Vec3::new(2.0, 3.0, 1.0);
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::transform::TransformPlugin))
            .init_resource::<PlateState>()
            .init_resource::<Units>()
            .init_resource::<Seen>()
            .init_resource::<MoveShip>()
            .add_systems(Update, (move_ship, read_ring).chain());
        let parent = Transform::from_translation(cry_to_bevy(ship_at)).with_rotation(
            Quat::from_rotation_y(std::f32::consts::FRAC_PI_2) * Quat::from_rotation_x(-0.2),
        );
        let ship = app.world_mut().spawn((parent, ShipRoot)).id();
        let child = app
            .world_mut()
            .spawn((
                Transform::from_translation(cry_to_bevy(local)),
                ChildOf(ship),
            ))
            .id();
        app.world_mut()
            .resource_mut::<PlateState>()
            .entities
            .insert(7, child);
        app.world_mut()
            .resource_mut::<Units>()
            .by_id
            .insert(7, cannon(7, 60, local));
        app.update();
        assert!(app.world().resource::<Seen>().0.is_some());
        app.world_mut().resource_mut::<MoveShip>().0 = true;
        app.update();
        let moved = Transform::from_translation(cry_to_bevy(ship_at) + Vec3::new(10.0, 0.0, 0.0))
            .with_rotation(
                Quat::from_rotation_y(std::f32::consts::FRAC_PI_2) * Quat::from_rotation_x(-0.2),
            );
        let expected = bevy_to_cry(moved.transform_point(cry_to_bevy(local)));
        let seen = app.world().resource::<Seen>().0.expect("the ring point");
        assert!(
            seen.distance(expected) < 1e-3,
            "the ring lags its parent: {seen:?} != {expected:?}"
        );
        assert!(seen.distance(local) > 1.0, "the ring used the local offset");
    }
}
