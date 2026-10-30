//! The use-target decal of a doodad: the ground ring the 1.2 client draws under the doodad whose use the action bar is
//! about (`doodad_almighties.use_target_decal`, native `TargetDecalManager` layer "doodad").
//!
//! The material is `materials/decals/target/doodad.mtl` (Illum: Diffuse 0.96862751, 0.19215688, 0, Emissive 1, 1, 1,
//! Opacity 0.99, GlowAmount 0.1) over `textures/decal/target/target_enemy04.dds`; `target_decal_size` is the ring's
//! radius in metres (0 in every 1.2 row, so the default below is used). The decal is a card of the 3D world like the
//! ground-target reticle (`aa_fx::decal::drape_quad`): it follows the floor under it, is lifted a few centimetres and
//! is depth tested, so it needs no HUD camera. The exact ring size and the projection (a real CryEngine decal) are
//! still capture-pending: the 2026-10-30 capture stood 5.5 m from portal 4608 (beyond its 4 m reach, so the original
//! bar offered nothing) and kept screenshots only, no radius, tint or projection reading. See
//! `tools/evidence/doodad_affordance/capture-results.txt` and `capture-plan.txt`.
use aa_net::state::{Doodads, Units};
use aa_world::{TerrainQuery, WorldCollision, cry_to_bevy};
use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use super::units::Views;

/// The `doodad.mtl` texture.
const TEXTURE: &str = "textures/decal/target/target_enemy04.dds";
/// Fraction of the card the ring's outer edge spans (as the ground-target ring; unverified for this texture).
const INK: f32 = 0.74;
/// The ring radius when `target_decal_size` is 0, metres (capture-pending: the ring was not measured).
const DEFAULT_RADIUS: f32 = 1.5;
/// Metres over the floor, and how far above the doodad the floor lookup starts.
const LIFT: f32 = 0.08;
const REACH_UP: f32 = 3.0;
/// A missing texture is retried at most this often (the pak can become available after the app starts).
const RETRY_SECS: f32 = 5.0;

/// The doodad the use bar is about, when its template draws a decal (`hud::doodad` writes it every frame).
#[derive(Resource, Default)]
pub struct DoodadTargetDecal(pub Option<DoodadDecal>);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DoodadDecal {
    pub obj_id: u32,
    /// `doodad_almighties.target_decal_size`, 0 = [`DEFAULT_RADIUS`].
    pub size: f32,
    /// The doodad's spawn scale (`SCDoodadCreated`), the ring grows with the model.
    pub scale: f32,
}

/// The one decal entity (unlike the skill reticle a single doodad is focused at a time).
#[derive(Component)]
struct TargetDecal;

#[derive(Default, Resource)]
struct DecalAssets {
    mesh: Option<Handle<Mesh>>,
    material: Option<Handle<StandardMaterial>>,
    /// When the texture last failed to read (no pak or bad file): retried after [`RETRY_SECS`].
    failed_at: Option<f32>,
}

pub fn plugin(app: &mut App) {
    app.init_resource::<DoodadTargetDecal>()
        .init_resource::<DecalAssets>()
        .add_systems(
            Update,
            decal
                .after(super::units::sync_views)
                .after(crate::hud::doodad::TrackDoodads)
                .run_if(super::world_screens),
        )
        .add_systems(OnEnter(crate::Screen::InGame), reset_assets)
        .add_systems(OnExit(crate::Screen::InGame), leave);
}

/// The pak is definitely needed in game: let a decal whose texture was unreadable so far try again.
fn reset_assets(mut assets: ResMut<DecalAssets>) {
    *assets = DecalAssets::default();
}

fn leave(
    mut commands: Commands,
    decals: Query<Entity, With<TargetDecal>>,
    mut target: ResMut<DoodadTargetDecal>,
) {
    target.0 = None;
    for entity in &decals {
        commands.entity(entity).despawn();
    }
}

fn dds_image(path: &str) -> Option<Image> {
    let bytes = aa_cry::read(path).ok()?;
    let dds = aa_cry::texture::Dds::parse(&bytes).ok()?;
    let (w, h) = (dds.width, dds.height);
    let mut image = Image::new(
        Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        dds.decode_rgba8(0, 0),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = bevy::image::ImageSampler::linear();
    Some(image)
}

/// Unlit, blended, both sides, pulled a little towards the camera (the ground-target decal material).
fn material(image: &Handle<Image>) -> StandardMaterial {
    StandardMaterial {
        // doodad.mtl Diffuse, with a HDR boost like the reticle tints so the ring survives the tonemapping
        base_color: Color::linear_rgba(3.6, 0.72, 0.0, 1.0),
        base_color_texture: Some(image.clone()),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        cull_mode: None,
        depth_bias: 12.0,
        ..default()
    }
}

/// Draws the ring under the focused doodad (`DoodadTargetDecal`), draped on the floor below it.
#[allow(clippy::too_many_arguments)]
fn decal(
    mut commands: Commands,
    target: Res<DoodadTargetDecal>,
    doodads: Res<Doodads>,
    units: Res<Units>,
    views: Res<Views>,
    houses: Option<Res<super::housing::render::DrawnHouses>>,
    mut assets: ResMut<DecalAssets>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    time: Res<Time>,
    collision: Option<Res<WorldCollision>>,
    terrain: Option<Res<TerrainQuery>>,
    mut decals: Query<
        (
            Entity,
            &mut Visibility,
            &mut MeshMaterial3d<StandardMaterial>,
        ),
        With<TargetDecal>,
    >,
) {
    let shown = target.0.and_then(|t| {
        let doodad = doodads.by_id.get(&t.obj_id)?;
        let world = super::units::doodad_world_in(doodad, &units, houses.as_deref())?;
        // not drawn yet (too far, no model): a decal would point at nothing
        views.doodad_entity(t.obj_id)?;
        Some((t, world))
    });
    let Some((target, world)) = shown else {
        for (_, mut visibility, _) in &mut decals {
            if *visibility != Visibility::Hidden {
                *visibility = Visibility::Hidden;
            }
        }
        return;
    };
    if assets.mesh.is_none() {
        let now = time.elapsed_secs();
        if let Some(at) = assets.failed_at {
            if now < at + RETRY_SECS {
                return;
            }
            debug!("doodad: retrying the use-target decal texture");
        }
        let Some(image) = dds_image(TEXTURE) else {
            if assets.failed_at.is_none() {
                warn!("doodad: cannot read {TEXTURE}: no use-target decal");
            }
            assets.failed_at = Some(now);
            return;
        };
        let image = images.add(image);
        assets.mesh = Some(meshes.add(Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
        )));
        assets.material = Some(materials.add(material(&image)));
    }
    let scale = if target.scale > 0.0 {
        target.scale
    } else {
        1.0
    };
    let radius = (if target.size > 0.0 {
        target.size
    } else {
        DEFAULT_RADIUS
    }) * scale;
    let half = radius.max(0.75) / INK;
    let centre = cry_to_bevy(world);
    let floor = |p: Vec3| -> Option<f32> {
        let (x, y) = (p.x, -p.z);
        collision
            .as_ref()
            .and_then(|c| c.ground_below(x, y, centre.y + REACH_UP, 0.0))
            .map(|g| g.height)
            .or_else(|| terrain.as_ref().and_then(|t| t.height_at(x, y)))
            .map(|h| cry_to_bevy(Vec3::new(world.x, world.y, h)).y)
            .or(Some(centre.y))
    };
    // texture up is game north (-z in Bevy)
    let card = aa_fx::decal::drape_quad(
        centre,
        Vec3::X * half,
        Vec3::NEG_Z * half,
        &floor,
        LIFT,
        aa_fx::decal::cells_for(half, half),
    );
    let mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_NORMAL,
        vec![[0.0f32, 1.0, 0.0]; card.positions.len()],
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, card.positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, card.uvs)
    .with_inserted_indices(Indices::U32(card.indices));
    if let Some(mut m) = meshes.get_mut(assets.mesh.as_ref().unwrap()) {
        *m = mesh;
    }
    let mesh = assets.mesh.clone().unwrap();
    let material = assets.material.clone().unwrap();
    match decals.iter_mut().next() {
        Some((_, mut visibility, mut m)) => {
            if *visibility != Visibility::Inherited {
                *visibility = Visibility::Inherited;
            }
            if m.0 != material {
                m.0 = material;
            }
        }
        None => {
            commands.spawn((
                Name::new("doodad use-target decal"),
                TargetDecal,
                Mesh3d(mesh),
                MeshMaterial3d(material),
                Transform::IDENTITY,
                Visibility::Inherited,
                // the card follows the doodad: its bounds change every frame
                NoFrustumCulling,
                NotShadowCaster,
                NotShadowReceiver,
            ));
        }
    }
}
