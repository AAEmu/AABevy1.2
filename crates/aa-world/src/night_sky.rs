//! Original `libs/sky/stars.dat` catalogue and `stars.cfx` sprite shading.
//! One cached mesh, screen-sized quads, no generated star positions or per-frame geometry.

use std::sync::{
    Mutex,
    mpsc::{Receiver, channel},
};

use bevy::asset::{RenderAssetUsages, embedded_asset};
use bevy::camera::{Exposure, visibility::NoFrustumCulling};
use bevy::light::NotShadowCaster;
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, CompareFunction, RenderPipelineDescriptor, ShaderType,
    SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;

use crate::coords::cry_to_bevy;
use crate::post::PostUniform;
use crate::reader::{ReadError, Reader};
use crate::sky::{AtmosphereTuning, Environment, update_environment, view_exposure};
use crate::stream::WorldConfig;

// CPU transparent-sort distance only, beyond the bounds of the authored worlds. Sky shaders
// ignore translation, so their origin's projected distance must not sort them over nearby glass.
pub(crate) const SKY_SORT_BIAS: f32 = -1_000_000.0;

#[derive(Clone, Copy, Debug)]
pub struct Star {
    /// Right ascension in hours; declination in degrees.
    pub right_ascension: f32,
    pub declination: f32,
    pub color_magnitude: [u8; 4],
}

impl Star {
    pub fn direction(&self) -> Vec3 {
        let ra = (self.right_ascension * 15.0).to_radians();
        let dec = self.declination.to_radians();
        Vec3::new(-dec.cos() * ra.sin(), dec.cos() * ra.cos(), dec.sin())
    }
}

pub fn parse_stars(data: &[u8]) -> Result<Vec<Star>, ReadError> {
    let mut r = Reader::new(data);
    if r.bytes(4)? != b"STAR" || r.u32()? != 0x00010001 {
        return Err(ReadError);
    }
    let count = r.u32()? as usize;
    if count.checked_mul(12) != Some(r.remaining()) {
        return Err(ReadError);
    }
    let mut stars = Vec::with_capacity(count);
    for _ in 0..count {
        let ra = r.f32()?;
        let dec = r.f32()?;
        if !ra.is_finite()
            || !dec.is_finite()
            || !(0.0..=24.0).contains(&ra)
            || !(-90.0..=90.0).contains(&dec)
        {
            return Err(ReadError);
        }
        stars.push(Star {
            right_ascension: ra,
            declination: dec,
            color_magnitude: r.bytes(4)?.try_into().unwrap(),
        });
    }
    Ok(stars)
}

fn star_mesh(stars: &[Star]) -> Mesh {
    let mut positions = Vec::with_capacity(stars.len() * 4);
    let mut colors = Vec::with_capacity(stars.len() * 4);
    let mut uvs = Vec::with_capacity(stars.len() * 4);
    let mut indices = Vec::with_capacity(stars.len() * 6);
    for (i, star) in stars.iter().enumerate() {
        let position = cry_to_bevy(star.direction()).to_array();
        let color = star.color_magnitude.map(|v| v as f32 / 255.0);
        for uv in [[0.0f32, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]] {
            positions.push(position);
            colors.push(color);
            uvs.push(uv);
        }
        let base = (i * 4) as u32;
        indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
    .with_inserted_indices(Indices::U32(indices))
}

#[derive(Clone, Copy, Debug, Default, ShaderType)]
pub struct StarParams {
    /// x: exposed intensity, y: native flicker clock (seconds/2), z: native r_stars_size (default 1).
    pub values: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct StarMaterial {
    #[uniform(0)]
    pub params: StarParams,
}

impl Material for StarMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://aa_world/stars.wesl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "embedded://aa_world/stars.wesl".into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
    }
    fn depth_bias(&self) -> f32 {
        SKY_SORT_BIAS
    }
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.vertex.buffers = vec![layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_UV_0.at_shader_location(1),
            Mesh::ATTRIBUTE_COLOR.at_shader_location(2),
        ])?];
        descriptor.primitive.cull_mode = None;
        if let Some(depth) = &mut descriptor.depth_stencil {
            depth.depth_write_enabled = Some(false);
            depth.depth_compare = Some(CompareFunction::GreaterEqual);
        }
        Ok(())
    }
}

#[derive(Resource, Clone, Debug)]
pub struct StarSettings {
    pub enabled: bool,
}
impl Default for StarSettings {
    fn default() -> Self {
        Self {
            enabled: std::env::var_os("AA_NO_STARS").is_none(),
        }
    }
}

#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct StarStats {
    pub catalogue: usize,
    pub intensity: f32,
}

#[derive(Resource, Default)]
struct State {
    requested: bool,
    rx: Option<Mutex<Receiver<Option<(Mesh, usize)>>>>,
    material: Option<Handle<StarMaterial>>,
    entity: Option<Entity>,
    visible: bool,
}

pub(crate) fn build(app: &mut App) {
    embedded_asset!(app, "stars.wesl");
    app.add_plugins(MaterialPlugin::<StarMaterial>::default())
        .init_resource::<State>()
        .init_resource::<StarSettings>()
        .init_resource::<StarStats>()
        .add_systems(Update, update.after(update_environment));
}

#[allow(clippy::too_many_arguments)]
fn update(
    mut commands: Commands,
    cfg: Res<WorldConfig>,
    env: Res<Environment>,
    time: Res<Time>,
    tuning: Res<AtmosphereTuning>,
    settings: Res<StarSettings>,
    mut stats: ResMut<StarStats>,
    mut state: ResMut<State>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StarMaterial>>,
    exposure: Query<Option<&Exposure>, With<PostUniform>>,
) {
    if !state.requested && cfg.environment {
        state.requested = true;
        let (tx, rx) = channel();
        state.rx = Some(Mutex::new(rx));
        aa_cry::workers::Workers::global().spawn(-60000.0, move || {
            let result = aa_pak::Pak::global()
                .read("libs/sky/stars.dat")
                .ok()
                .and_then(|b| parse_stars(&b).ok())
                .map(|stars| (star_mesh(&stars), stars.len()));
            let _ = tx.send(result);
        });
    }
    let received = state
        .rx
        .as_ref()
        .and_then(|rx| rx.lock().unwrap().try_recv().ok());
    if let Some(result) = received {
        state.rx = None;
        if let Some((mesh, count)) = result {
            let material = materials.add(StarMaterial {
                params: StarParams::default(),
            });
            let entity = commands
                .spawn((
                    Name::new("Original star catalogue"),
                    Mesh3d(meshes.add(mesh)),
                    MeshMaterial3d(material.clone()),
                    Transform::default(),
                    NoFrustumCulling,
                    NotShadowCaster,
                    Visibility::Hidden,
                ))
                .id();
            state.entity = Some(entity);
            state.material = Some(material);
            stats.catalogue = count;
        } else {
            warn!("aa-world: original star catalogue missing or malformed");
        }
    }
    stats.intensity = if cfg.environment && settings.enabled {
        env.sample
            .as_ref()
            .map_or(0.0, |s| s.star_intensity.max(0.0))
    } else {
        0.0
    };
    let visible = stats.intensity > 1e-3;
    if visible != state.visible
        && let Some(entity) = state.entity
    {
        commands.entity(entity).insert(if visible {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        state.visible = visible;
    }
    if visible && let Some(material) = state.material.as_ref().and_then(|h| materials.get_mut(h)) {
        material.into_inner().params.values = Vec4::new(
            stats.intensity * tuning.radiance * view_exposure(&exposure),
            time.elapsed_secs() * 0.5,
            1.0,
            0.0,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_star_catalogue_retains_sirius_arcturus_and_native_magnitudes() {
        if !crate::pak_available() {
            return;
        }
        let data = aa_pak::Pak::global().read("libs/sky/stars.dat").unwrap();
        let stars = parse_stars(&data).unwrap();
        assert_eq!(stars.len(), 8912);
        assert!((stars[0].right_ascension - 6.75257).abs() < 0.0001);
        assert!((stars[0].declination + 16.71314).abs() < 0.0001);
        assert_eq!(stars[0].color_magnitude, [132, 160, 255, 255]);
        assert_eq!(stars[2].color_magnitude, [255, 162, 89, 210]);
        assert!(stars[0].direction().z < -0.28 && stars[0].direction().x < -0.9);
        assert!(
            stars
                .iter()
                .all(|s| (s.direction().length() - 1.0).abs() < 1e-5)
        );
        let mesh = star_mesh(&stars);
        assert_eq!(mesh.count_vertices(), 8912 * 4);
        assert_eq!(mesh.indices().unwrap().len(), 8912 * 6);
        assert!(parse_stars(&data[..data.len() - 1]).is_err());
        let xml = aa_pak::Pak::global()
            .read_string("worlds/main_world/zone/133/time_of_day.xml")
            .unwrap();
        let tod = crate::env::TimeOfDay::parse(&xml).unwrap();
        let night = tod.sample(0.0, crate::env::Lighting::default());
        let noon = tod.sample(12.0, crate::env::Lighting::default());
        assert!(night.star_intensity > 2.0 && noon.star_intensity < 0.1);
        assert!(
            (night.lerp(&noon, 0.5).star_intensity
                - (night.star_intensity + noon.star_intensity) * 0.5)
                .abs()
                < 1e-5
        );
    }
}
