//! Distant cloud sprites of `zone/<id>/clouds.xml`.
//!
//! ```xml
//! <Cloud id="2" bearing="180" distance="2000" height="400" size="700" speed="0.1" material="materials/clouds/distantclouds02_solzreed"/>
//! ```
//!
//! The material is a `DistanceClouds` shader material (`distanceclouds.cfx`): a grey scale texture whose red channel is the
//! cloud density, `StepSize`, `Attenuation`, `AlphaSaturation`, `SkyColorMultiplier`, `SunColorMultiplier`, `Opacity` and the
//! diffuse colour. Original client 3012E8A0 / 300B42A0 / 300B4410 define camera-relative angular motion,
//! a radial-facing square with `size` half extents, and far-plane depth (`clouds.wesl`).

use std::collections::{HashMap, HashSet};

use bevy::asset::embedded_asset;
use bevy::camera::Exposure;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, CompareFunction, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;

use crate::coords::cry_to_bevy;
use crate::post::PostUniform;
use crate::sky::{AtmosphereTuning, Environment, view_exposure};
use crate::sky_transition::SkySelection;
use crate::stream::{WorldConfig, WorldFocus};

/// One `<Cloud>` of a `clouds.xml`.
#[derive(Clone, Debug, PartialEq)]
pub struct CloudDef {
    /// Native degrees about Cry +Z, rotating (0,distance,0); the data has values above 360.
    pub bearing: f32,
    /// Horizontal distance in metres.
    pub distance: f32,
    /// Height above the camera in metres.
    pub height: f32,
    /// Native square half extents in metres.
    pub size: f32,
    /// Angular speed in degrees per frame-second, independent of the game time of day.
    pub speed: f32,
    pub material: String,
}

/// Parses a `clouds.xml`.
pub fn parse_clouds(xml: &str) -> Vec<CloudDef> {
    let Ok(doc) = roxmltree::Document::parse(xml) else {
        return Vec::new();
    };
    let num = |n: roxmltree::Node, name: &str| {
        n.attribute(name)
            .and_then(|v| v.trim().parse::<f32>().ok())
            .unwrap_or(0.0)
    };
    doc.descendants()
        .filter(|n| n.has_tag_name("Cloud"))
        .filter_map(|n| {
            let material = n.attribute("material")?.trim().to_string();
            let def = CloudDef {
                bearing: num(n, "bearing"),
                distance: num(n, "distance"),
                height: num(n, "height"),
                size: num(n, "size"),
                speed: num(n, "speed"),
                material,
            };
            (!def.material.is_empty()
                && def.size > 0.0
                && def.distance > 0.0
                && [def.bearing, def.distance, def.height, def.size, def.speed]
                    .iter()
                    .all(|x| x.is_finite()))
            .then_some(def)
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
    fn vertex_shader() -> ShaderRef {
        "embedded://aa_world/clouds.wesl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "embedded://aa_world/clouds.wesl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }

    fn depth_bias(&self) -> f32 {
        // CPU ordering only: native far-depth clouds belong behind world translucency and
        // above the opaque sky / catalogue. Preserve distance ordering between cloud sprites.
        crate::night_sky::SKY_SORT_BIAS + 100_000.0
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
        if let Some(depth) = descriptor.depth_stencil.as_mut() {
            depth.depth_write_enabled = Some(false);
            depth.depth_compare = Some(CompareFunction::GreaterEqual);
        }
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
    bearing: f32,
}

impl CloudSprite {
    pub fn definition(&self) -> &CloudDef {
        &self.def
    }
    /// Angle to use for the next frame, after the last rendered pose.
    pub fn bearing(&self) -> f32 {
        self.bearing
    }
}

/// Native `e_clouds` switch. Disabling hides clouds and freezes their current angles.
#[derive(Resource, Clone, Debug)]
pub struct CloudSettings {
    pub enabled: bool,
}

impl Default for CloudSettings {
    fn default() -> Self {
        Self {
            enabled: std::env::var_os("AA_NO_CLOUDS").is_none(),
        }
    }
}

/// Native Render advances after constructing this frame's pose, and wraps once at +/-360.
fn advance_bearing(bearing: f32, speed: f32, seconds: f32) -> f32 {
    let next = bearing + speed * seconds;
    if next > 360.0 {
        next - 360.0
    } else if next < -360.0 {
        next + 360.0
    } else {
        next
    }
}

fn cloud_pose(def: &CloudDef, bearing: f32, camera: Vec3) -> Transform {
    let b = bearing.to_radians();
    let offset = cry_to_bevy(Vec3::new(-b.sin() * def.distance, b.cos() * def.distance, def.height));
    let normal = offset.normalize();
    let right = normal.cross(Vec3::Y).normalize();
    let up = right.cross(normal).normalize();
    Transform {
        translation: camera + offset,
        rotation: Quat::from_mat3(&Mat3::from_cols(right, up, -normal)),
        scale: Vec3::new(2.0 * def.size, 2.0 * def.size, 1.0),
    }
}

/// The clouds of the zone under the camera.
#[derive(Resource, Default)]
pub struct CloudState {
    world: Option<String>,
    /// Loaded zone/layer (`None` = nothing built yet). Native reapplies clouds at a layer switch.
    zone: Option<(Option<u16>, u32)>,
    entities: Vec<Entity>,
    bases: Vec<(Handle<CloudMaterial>, CloudBase)>,
    textures: HashMap<String, Option<Handle<Image>>>,
}

/// Production cloud material/shader without world streaming, for renderer controls.
pub struct CloudMaterialPlugin;
impl Plugin for CloudMaterialPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "clouds.wesl");
        app.add_plugins(MaterialPlugin::<CloudMaterial>::default());
    }
}

pub(crate) fn build(app: &mut App) {
    app.add_plugins(CloudMaterialPlugin)
        .init_resource::<CloudState>()
        .init_resource::<CloudSettings>()
        .add_systems(
            PostUpdate,
            place_clouds.before(bevy::transform::TransformSystems::Propagate),
        );
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
    settings: Res<CloudSettings>,
    env: Res<Environment>,
    selection: Res<SkySelection>,
    tuning: Res<AtmosphereTuning>,
    mut state: ResMut<CloudState>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<CloudMaterial>>,
    mut images: ResMut<Assets<Image>>,
    exposure: Query<Option<&Exposure>, With<PostUniform>>,
) {
    if !cfg.environment || env.sample.is_none() {
        for e in state.entities.drain(..) {
            commands.entity(e).despawn();
        }
        state.bases.clear();
        state.textures.clear();
        state.zone = None;
        state.world = None;
        return;
    }
    let Some(cur) = &env.sample else { return };
    let state = &mut *state;
    let key = (selection.zone, selection.layer);
    if state.zone != Some(key) || state.world.as_deref() != Some(cfg.world.as_str()) {
        state.world = Some(cfg.world.clone());
        state.zone = Some(key);
        for e in state.entities.drain(..) {
            commands.entity(e).despawn();
        }
        state.bases.clear();
        let xml = selection.zone.and_then(|z| {
            aa_pak::Pak::global()
                .read_string(&format!("worlds/{}/zone/{z}/clouds.xml", cfg.world))
                .ok()
        });
        let mut by_material: HashMap<String, Handle<CloudMaterial>> = HashMap::new();
        let mut used_textures = HashSet::new();
        let quad = meshes.add(Rectangle::new(1.0, 1.0));
        for def in xml.map(|x| parse_clouds(&x)).unwrap_or_default().into_iter().take(128) {
            let key = def.material.to_ascii_lowercase();
            let handle = match by_material.get(&key) {
                Some(h) => h.clone(),
                None => {
                    let Some((base, path)) = load_base(&def.material) else {
                        continue;
                    };
                    used_textures.insert(path.clone());
                    let texture = state.textures.entry(path.clone()).or_insert_with(|| {
                        let opts = aa_cry::texture::DdsOptions {
                            srgb: false,
                            bc_supported: true,
                            repeat: false,
                        };
                        let mut image = aa_cry::texture::load_dds_from_pak(&path, opts).ok()?;
                        image.sampler = bevy::image::ImageSampler::Descriptor(bevy::image::ImageSamplerDescriptor {
                            address_mode_u: bevy::image::ImageAddressMode::ClampToEdge,
                            address_mode_v: bevy::image::ImageAddressMode::ClampToEdge,
                            mag_filter: bevy::image::ImageFilterMode::Linear,
                            min_filter: bevy::image::ImageFilterMode::Linear,
                            mipmap_filter: bevy::image::ImageFilterMode::Linear,
                            ..default()
                        });
                        Some(images.add(image))
                    });
                    let Some(texture) = texture.clone() else {
                        continue;
                    };
                    let params = CloudParams {
                        shade_sky: Vec4::ZERO,
                        shade_sun: Vec4::ZERO,
                        material: base.diffuse.extend(base.opacity),
                        shape: Vec4::new(base.attenuation, base.step, base.alpha_saturation, 0.0),
                        sun: Vec4::new(0.0, 1.0, 0.0, 1.0),
                    };
                    let h = materials.add(CloudMaterial {
                        params,
                        texture: Some(texture),
                    });
                    state.bases.push((h.clone(), base));
                    by_material.insert(key, h.clone());
                    h
                }
            };
            let bearing = def.bearing;
            let entity = commands
                .spawn((
                    CloudSprite { def, bearing },
                    Mesh3d(quad.clone()),
                    MeshMaterial3d(handle),
                    Transform::default(),
                    if settings.enabled {
                        Visibility::Inherited
                    } else {
                        Visibility::Hidden
                    },
                    NotShadowCaster,
                    NotShadowReceiver,
                    NoFrustumCulling,
                ))
                .id();
            state.entities.push(entity);
        }
        // Retain assets of this zone only. Revisits share active textures, while world/zone
        // transitions cannot accumulate every DDS encountered on a long session.
        state.textures.retain(|path, _| used_textures.contains(path));
    }

    // shading colours of the time of day, like the sky box
    let rs = tuning.radiance * view_exposure(&exposure) * tuning.sky_gain * tuning.cloud_gain;
    // Same native day/night directional selection as C3DEngine::UpdateSun (300A5DD0).
    let light = if cur.sun_dir.z > 0.0 { cur.sun_dir } else { cur.moon_dir };
    let sun = cry_to_bevy(light).normalize();
    for (handle, base) in &state.bases {
        let visible = if tuning.cloud_gain > 0.0 { selection.opacity } else { 0.0 };
        let shade_sky = (cur.cloud_sky * base.sky_multiplier * rs).extend(visible);
        let shade_sun = (cur.cloud_sun * base.sun_multiplier * rs).extend(1.0);
        let sun_dir = sun.extend(1.0);
        let changed = materials.get(handle).is_some_and(|m| {
            m.params.shade_sky != shade_sky || m.params.shade_sun != shade_sun || m.params.sun != sun_dir
        });
        if changed && let Some(mut m) = materials.get_mut(handle) {
            m.params.shade_sky = shade_sky;
            m.params.shade_sun = shade_sun;
            m.params.sun = sun_dir;
        }
    }
}

/// Native radial-facing placement and real frame-time motion, bounded by the active zone.
#[allow(clippy::type_complexity)]
fn place_clouds(
    time: Res<Time>,
    settings: Res<CloudSettings>,
    focus: Query<Entity, With<WorldFocus>>,
    cameras: Query<(Entity, &Transform), (With<Camera3d>, Without<CloudSprite>)>,
    mut sprites: Query<(&mut CloudSprite, &mut Transform, &mut Visibility), Without<Camera3d>>,
) {
    let focus = focus.iter().next();
    let Some((_, cam)) = cameras
        .iter()
        .find(|(e, _)| Some(*e) == focus)
        .or_else(|| cameras.iter().next())
    else {
        return;
    };
    for (mut sprite, mut t, mut visibility) in &mut sprites {
        *visibility = if settings.enabled {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if !settings.enabled {
            continue;
        }
        *t = cloud_pose(&sprite.def, sprite.bearing, cam.translation);
        sprite.bearing = advance_bearing(sprite.bearing, sprite.def.speed, time.delta_secs());
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

    #[test]
    fn native_motion_units_radial_plane_and_square_half_extents() {
        let def = CloudDef {
            bearing: 90.0,
            distance: 1000.0,
            height: 300.0,
            size: 250.0,
            speed: 0.2,
            material: String::new(),
        };
        let camera = Vec3::new(12000.0, 240.0, -21000.0);
        let pose = cloud_pose(&def, def.bearing, camera);
        assert!((pose.translation - camera - Vec3::new(-1000.0, 300.0, 0.0)).length() < 0.002);
        assert_eq!(pose.scale, Vec3::new(500.0, 500.0, 1.0));
        assert!((pose.rotation * Vec3::X - Vec3::NEG_Z).length() < 1e-5);
        assert!((pose.rotation * Vec3::Z).dot((camera - pose.translation).normalize()) > 0.99999);
        assert!((advance_bearing(180.0, 0.1, 60.0) - 186.0).abs() < 1e-5);
        assert!((advance_bearing(500.0, 0.4, 0.25) - 140.1).abs() < 1e-4);
        assert!((advance_bearing(-359.9, -0.4, 0.5) - (-0.1)).abs() < 1e-4);
        assert_eq!(advance_bearing(90.0, 0.0, 60.0), 90.0);
    }

    #[test]
    fn cloud_sort_between_catalogue_and_near_translucency_on_both_sides_of_origin() {
        use bevy::render::render_phase::ViewRangefinder3d;
        let material = CloudMaterial {
            params: CloudParams {
                shade_sky: Vec4::ZERO,
                shade_sun: Vec4::ZERO,
                material: Vec4::ZERO,
                shape: Vec4::ZERO,
                sun: Vec4::ZERO,
            },
            texture: None,
        };
        let def = CloudDef {
            bearing: 180.0,
            distance: 2000.0,
            height: 400.0,
            size: 700.0,
            speed: 0.1,
            material: String::new(),
        };
        for at in [Vec3::new(12000.0, 200.0, -21000.0), Vec3::new(30000.0, 800.0, -10000.0)] {
            let pose = cloud_pose(&def, def.bearing, at);
            let look = (pose.translation - at).normalize();
            let camera = Transform::from_translation(at).looking_to(look, Vec3::Y);
            let range = ViewRangefinder3d::from_world_from_view(&camera.compute_affine());
            let cloud_key = range.distance(&pose.translation) + material.depth_bias();
            assert!(range.distance(&Vec3::ZERO) + crate::night_sky::SKY_SORT_BIAS + 4.0 < cloud_key);
            assert!(
                cloud_key < range.distance(&(at + look * 10.0)),
                "clouds must stay under near glass"
            );
        }
    }

    #[test]
    fn original_cloud_motion_pause_switch_and_world_lifecycle() {
        if !crate::pak_available() {
            return;
        }
        use bevy::time::TimeUpdateStrategy;
        use std::time::Duration;
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(0.25)))
            .insert_resource(WorldConfig::new("main_world"))
            .insert_resource(Environment {
                sample: Some(crate::env::TimeOfDay::default().sample(10.0, crate::env::Lighting::default())),
                zone: Some(179),
                hour: 10.0,
                ocean_fog: Vec3::ZERO, ocean_density: 0.0,
            })
            .insert_resource(AtmosphereTuning::default())
            .insert_resource(CloudSettings { enabled: true })
            .init_resource::<SkySelection>()
            .init_resource::<CloudState>()
            .init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<Image>>()
            .init_resource::<Assets<CloudMaterial>>()
            .add_systems(Update, update_clouds)
            .add_systems(PostUpdate, place_clouds);
        app.world_mut()
            .spawn((Camera3d::default(), Transform::from_xyz(12000.0, 200.0, -21000.0)));
        app.world_mut().resource_mut::<SkySelection>().update("main_world", &[(179, 1.0)], 0, 0.0);
        app.world_mut().resource_mut::<SkySelection>().update("main_world", &[(179, 1.0)], 0, 2.0);
        app.update();
        let count = |app: &mut App| app.world_mut().query::<&CloudSprite>().iter(app.world()).count();
        assert_eq!(count(&mut app), 4);
        assert!(app.world().resource::<Assets<CloudMaterial>>().iter()
            .all(|(_, m)| m.params.shade_sky.w == 1.0));
        // A same-zone relic switch reapplies the shared authored cloud list at zero,
        // then both skybox and cloud alpha use the native frame-time fade.
        app.world_mut().resource_mut::<SkySelection>().update("main_world", &[(179, 1.0)], 1, 2.0);
        app.update();
        assert_eq!(count(&mut app), 4);
        assert!(app.world().resource::<CloudState>().bases.iter().all(|(handle, _)|
            app.world().resource::<Assets<CloudMaterial>>().get(handle).unwrap().params.shade_sky.w == 0.0));
        app.world_mut().resource_mut::<SkySelection>().update("main_world", &[(179, 1.0)], 1, 1.0);
        app.update();
        assert!(app.world().resource::<CloudState>().bases.iter().all(|(handle, _)|
            app.world().resource::<Assets<CloudMaterial>>().get(handle).unwrap().params.shade_sky.w == 0.5));
        let angle = |app: &mut App| {
            app.world_mut()
                .query::<&CloudSprite>()
                .iter(app.world())
                .find(|c| c.def.bearing == 180.0)
                .unwrap()
                .bearing
        };
        let start = angle(&mut app);
        for _ in 0..240 {
            app.update();
        }
        assert!(
            (angle(&mut app) - start - 6.0).abs() < 0.01,
            "native speed is degrees/frame-second"
        );
        app.world_mut().resource_mut::<Time<Virtual>>().pause();
        let paused = angle(&mut app);
        for _ in 0..4 {
            app.update();
        }
        assert_eq!(angle(&mut app), paused);
        app.world_mut().resource_mut::<Time<Virtual>>().unpause();
        app.world_mut().resource_mut::<CloudSettings>().enabled = false;
        for _ in 0..4 {
            app.update();
        }
        assert_eq!(angle(&mut app), paused);
        assert!(
            app.world_mut()
                .query_filtered::<&Visibility, With<CloudSprite>>()
                .iter(app.world())
                .all(|v| *v == Visibility::Hidden)
        );
        app.world_mut().resource_mut::<CloudSettings>().enabled = true;
        app.update();
        assert!(angle(&mut app) > paused);
        app.world_mut().resource_mut::<Environment>().zone = Some(186);
        app.world_mut().resource_mut::<SkySelection>().update("main_world", &[(186, 1.0)], 0, 0.0);
        app.update();
        assert_eq!(count(&mut app), 7);
        assert!(app.world().resource::<CloudState>().textures.len() <= 7);
        app.world_mut().resource_mut::<WorldConfig>().world = "instance_library".into();
        app.update();
        assert_eq!(
            count(&mut app),
            0,
            "same zone id in another world must not retain old sprites"
        );
        assert!(app.world().resource::<CloudState>().textures.is_empty());
        app.world_mut().resource_mut::<WorldConfig>().world = "main_world".into();
        app.update();
        assert_eq!(count(&mut app), 7);
        app.world_mut().resource_mut::<WorldConfig>().environment = false;
        app.update();
        assert_eq!(count(&mut app), 0);
        assert!(app.world().resource::<CloudState>().bases.is_empty());
        assert!(app.world().resource::<CloudState>().textures.is_empty());
    }
}
