//! Distant cloud sprites of `zone/<id>/clouds.xml`.
//!
//! ```xml
//! <Cloud id="2" bearing="180" distance="2000" height="400" size="700" speed="0.1" material="materials/clouds/distantclouds02_solzreed"/>
//! ```
//!
//! The material is a `DistanceClouds` shader material (`distanceclouds.cfx`): a grey scale texture whose red channel is the
//! cloud density, `StepSize`, `Attenuation`, `AlphaSaturation`, `SkyColorMultiplier`, `SunColorMultiplier`, `Opacity` and the
//! diffuse colour. Each cloud is one camera facing quad `distance` metres away on the compass `bearing`, `height` metres above the
//! camera, `size` metres wide, drawn with alpha blending behind the terrain (`clouds.wesl`). Four sprites per zone: free.

use std::collections::HashMap;

use bevy::asset::embedded_asset;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError};
use bevy::shader::ShaderRef;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::camera::Exposure;

use crate::coords::cry_to_bevy;
use crate::post::PostUniform;
use crate::sky::{AtmosphereTuning, Environment, view_exposure};
use crate::stream::{WorldConfig, WorldFocus};

/// One `<Cloud>` of a `clouds.xml`.
#[derive(Clone, Debug, PartialEq)]
pub struct CloudDef {
    /// Compass bearing in degrees (0 = north, 90 = east); the data has values above 360.
    pub bearing: f32,
    /// Horizontal distance in metres.
    pub distance: f32,
    /// Height above the camera in metres.
    pub height: f32,
    /// Width of the sprite in metres.
    pub size: f32,
    pub speed: f32,
    pub material: String,
}

/// Parses a `clouds.xml`.
pub fn parse_clouds(xml: &str) -> Vec<CloudDef> {
    let Ok(doc) = roxmltree::Document::parse(xml) else { return Vec::new() };
    let num = |n: roxmltree::Node, name: &str| n.attribute(name).and_then(|v| v.trim().parse::<f32>().ok()).unwrap_or(0.0);
    doc.descendants()
        .filter(|n| n.has_tag_name("Cloud"))
        .filter_map(|n| {
            let material = n.attribute("material")?.trim().to_string();
            (!material.is_empty() && num(n, "size") > 0.0).then(|| CloudDef {
                bearing: num(n, "bearing"),
                distance: num(n, "distance"),
                height: num(n, "height"),
                size: num(n, "size"),
                speed: num(n, "speed"),
                material,
            })
        })
        .collect()
}

/// Uniform of `clouds.wesl`.
#[derive(Clone, Copy, Debug, PartialEq, ShaderType)]
pub struct CloudParams {
    pub shade_sky: Vec4,
    pub shade_sun: Vec4,
    pub material: Vec4,
    pub shape: Vec4,
    pub sun: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct CloudMaterial {
    #[uniform(0)]
    pub params: CloudParams,
    #[texture(1)]
    #[sampler(2)]
    pub texture: Option<Handle<Image>>,
}

impl Material for CloudMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://aa_world/clouds.wesl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
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
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// Constants of a cloud material (`.mtl` public params), the shading colours are added each frame.
#[derive(Clone, Debug)]
struct CloudBase {
    sky_multiplier: f32,
    sun_multiplier: f32,
    diffuse: Vec3,
    opacity: f32,
    attenuation: f32,
    step: f32,
    alpha_saturation: f32,
}

#[derive(Component)]
pub struct CloudSprite {
    def: CloudDef,
    /// Height / width of the texture.
    aspect: f32,
}

/// The clouds of the zone under the camera.
#[derive(Resource, Default)]
pub struct CloudState {
    /// The zone the sprites were built for (`None` = nothing built yet).
    zone: Option<Option<u16>>,
    entities: Vec<Entity>,
    bases: Vec<(Handle<CloudMaterial>, CloudBase)>,
    textures: HashMap<String, Option<(Handle<Image>, f32)>>,
}

pub(crate) fn build(app: &mut App) {
    embedded_asset!(app, "clouds.wesl");
    app.add_plugins(MaterialPlugin::<CloudMaterial>::default())
        .init_resource::<CloudState>()
        .add_systems(PostUpdate, place_clouds.before(bevy::transform::TransformSystems::Propagate));
}

fn load_base(material: &str) -> Option<(CloudBase, String)> {
    let mtl = aa_cry::material::Material::load(&format!("{}.mtl", material.trim_end_matches(".mtl"))).ok()?;
    if !mtl.shader.eq_ignore_ascii_case("distanceclouds") {
        return None;
    }
    let path = mtl.texture(&aa_cry::material::TextureMap::Diffuse)?.pak_path();
    let param = |name: &str, default: f32| mtl.public_param(name).and_then(|v| v.parse().ok()).unwrap_or(default);
    Some((
        CloudBase {
            sky_multiplier: param("SkyColorMultiplier", 1.5),
            sun_multiplier: param("SunColorMultiplier", 4.0),
            diffuse: mtl.diffuse,
            opacity: mtl.opacity,
            attenuation: param("Attenuation", 0.6),
            step: param("StepSize", 0.004),
            alpha_saturation: param("AlphaSaturation", 2.0),
        },
        path,
    ))
}

/// Builds the sprites of the zone under the camera when the zone changes and refreshes their shading from the environment.
#[allow(clippy::too_many_arguments)]
pub fn update_clouds(
    mut commands: Commands,
    cfg: Res<WorldConfig>,
    env: Res<Environment>,
    tuning: Res<AtmosphereTuning>,
    mut state: ResMut<CloudState>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<CloudMaterial>>,
    mut images: ResMut<Assets<Image>>,
    exposure: Query<Option<&Exposure>, With<PostUniform>>,
) {
    if !cfg.environment {
        return;
    }
    let Some(cur) = &env.sample else { return };
    let state = &mut *state;
    if state.zone != Some(env.zone) {
        state.zone = Some(env.zone);
        for e in state.entities.drain(..) {
            commands.entity(e).despawn();
        }
        state.bases.clear();
        let xml = env
            .zone
            .and_then(|z| aa_pak::Pak::global().read_string(&format!("worlds/{}/zone/{z}/clouds.xml", cfg.world)).ok());
        let mut by_material: HashMap<String, Handle<CloudMaterial>> = HashMap::new();
        let quad = meshes.add(Rectangle::new(1.0, 1.0));
        for def in xml.map(|x| parse_clouds(&x)).unwrap_or_default() {
            let key = def.material.to_ascii_lowercase();
            let handle = match by_material.get(&key) {
                Some(h) => h.clone(),
                None => {
                    let Some((base, path)) = load_base(&def.material) else { continue };
                    let texture = state.textures.entry(path.clone()).or_insert_with(|| {
                        let opts = aa_cry::texture::DdsOptions { srgb: false, bc_supported: true, repeat: false };
                        let mut image = aa_cry::texture::load_dds_from_pak(&path, opts).ok()?;
                        let aspect = image.height() as f32 / image.width().max(1) as f32;
                        image.sampler = bevy::image::ImageSampler::Descriptor(bevy::image::ImageSamplerDescriptor {
                            address_mode_u: bevy::image::ImageAddressMode::ClampToEdge,
                            address_mode_v: bevy::image::ImageAddressMode::ClampToEdge,
                            mag_filter: bevy::image::ImageFilterMode::Linear,
                            min_filter: bevy::image::ImageFilterMode::Linear,
                            mipmap_filter: bevy::image::ImageFilterMode::Linear,
                            ..default()
                        });
                        Some((images.add(image), aspect))
                    });
                    let Some((texture, aspect)) = texture.clone() else { continue };
                    let params = CloudParams {
                        shade_sky: Vec4::ZERO,
                        shade_sun: Vec4::ZERO,
                        material: base.diffuse.extend(base.opacity),
                        shape: Vec4::new(base.attenuation, base.step, base.alpha_saturation, aspect),
                        sun: Vec4::new(0.0, 1.0, 0.0, 1.0),
                    };
                    let h = materials.add(CloudMaterial { params, texture: Some(texture) });
                    state.bases.push((h.clone(), base));
                    by_material.insert(key, h.clone());
                    h
                }
            };
            let aspect = materials.get(&handle).map_or(1.0, |m| m.params.shape.w);
            let entity = commands
                .spawn((
                    CloudSprite { def, aspect },
                    Mesh3d(quad.clone()),
                    MeshMaterial3d(handle),
                    Transform::default(),
                    NotShadowCaster,
                    NotShadowReceiver,
                    NoFrustumCulling,
                ))
                .id();
            state.entities.push(entity);
        }
    }

    // shading colours of the time of day, like the sky box
    let rs = tuning.radiance * view_exposure(&exposure) * tuning.sky_gain * tuning.cloud_gain;
    let sun = cry_to_bevy(cur.sun_dir).normalize();
    for (handle, base) in &state.bases {
        let visible = if tuning.cloud_gain > 0.0 { 1.0 } else { 0.0 };
        let shade_sky = (cur.cloud_sky * base.sky_multiplier * rs).extend(visible);
        let shade_sun = (cur.cloud_sun * base.sun_multiplier * rs).extend(1.0);
        let sun_dir = sun.extend(1.0);
        let changed = materials
            .get(handle)
            .is_some_and(|m| m.params.shade_sky != shade_sky || m.params.shade_sun != shade_sun || m.params.sun != sun_dir);
        if changed && let Some(mut m) = materials.get_mut(handle) {
            m.params.shade_sky = shade_sky;
            m.params.shade_sun = shade_sun;
            m.params.sun = sun_dir;
        }
    }
}

/// Puts every sprite at its bearing / distance / height around the camera, facing it.
#[allow(clippy::type_complexity)]
fn place_clouds(
    focus: Query<Entity, With<WorldFocus>>,
    cameras: Query<(Entity, &Transform), (With<Camera3d>, Without<CloudSprite>)>,
    mut sprites: Query<(&CloudSprite, &mut Transform), Without<Camera3d>>,
) {
    let focus = focus.iter().next();
    let Some((_, cam)) = cameras.iter().find(|(e, _)| Some(*e) == focus).or_else(|| cameras.iter().next()) else { return };
    for (sprite, mut t) in &mut sprites {
        let b = sprite.def.bearing.to_radians();
        // compass: 0 = north = bevy -Z, 90 = east = +X
        let dir = Vec3::new(b.sin(), 0.0, -b.cos());
        t.translation = cam.translation + dir * sprite.def.distance + Vec3::Y * sprite.def.height;
        t.rotation = cam.rotation;
        t.scale = Vec3::new(sprite.def.size, sprite.def.size * sprite.aspect, 1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_clouds_xml() {
        let xml = r#"<Clouds>
 <Cloud id="1" bearing="100" distance="1500" height="-50" size="866.9" speed="0.2" material="materials/clouds/distanceclouds"/>
 <Cloud id="4" bearing="500" distance="1000" height="350" size="350" speed="0.4" material="materials/clouds/b"/>
 <Cloud id="5" size="0" material="x"/>
</Clouds>"#;
        let c = parse_clouds(xml);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].material, "materials/clouds/distanceclouds");
        assert!((c[0].size - 866.9).abs() < 1e-3 && c[0].height == -50.0 && c[1].bearing == 500.0);
        assert!(parse_clouds("not xml").is_empty());
    }

    #[test]
    fn solzreed_cloud_material() {
        if !crate::pak_available() {
            return;
        }
        let (base, path) = load_base("materials/clouds/distantclouds02_solzreed").expect("DistanceClouds material");
        assert!((base.opacity - 0.653).abs() < 1e-3 && (base.alpha_saturation - 1.419).abs() < 1e-3);
        assert!(path.ends_with("wz_gar_distantclouds_03.dds"), "{path}");
        assert!(load_base("game/materials/ocean/lake_tank").is_none());
    }
}
