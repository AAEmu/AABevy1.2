//! Original x2moon discs/coronas. Geometry and parameter units are from the 1.2 renderer:
//! 381B8A50 (XML/size), 381B89F0 + 381B8550 (quad/UVs), 380BDAC0 (draw/constants).

use std::sync::{
    Mutex,
    mpsc::{Receiver, TryRecvError, channel},
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
use crate::env::MoonSample;
use crate::post::PostUniform;
use crate::sky::{AtmosphereTuning, Environment, update_environment, view_exposure};
use crate::stream::WorldConfig;

#[derive(Clone, Debug)]
pub struct Moon {
    pub slot: usize,
    pub direction: Vec3,
    /// Engine remap: .04 + .96 * clamp(XML Size, 0, 10), not an angle in degrees.
    pub size: f32,
    pub texture: String,
    pub order: i32,
}

/// Native spelling `SecondayMoons`, secondary Used flag and MoonN time-of-day slot.
pub fn parse_moons(xml: &str) -> Vec<Moon> {
    let Ok(doc) = roxmltree::Document::parse(xml) else {
        return Vec::new();
    };
    let root = doc.root_element();
    let secondary = root.children().find(|n| n.has_tag_name("SecondayMoons"));
    let nodes = root.children().filter(|n| n.has_tag_name("Moon")).chain(
        secondary
            .into_iter()
            .flat_map(|n| n.children().filter(|n| n.is_element())),
    );
    let mut moons = Vec::new();
    for node in nodes {
        let slot = if node.has_tag_name("Moon") {
            0
        } else {
            if node
                .attribute("Used")
                .and_then(|s| s.parse::<i32>().ok())
                .unwrap_or(0)
                == 0
            {
                continue;
            }
            let Some(i) = node
                .tag_name()
                .name()
                .strip_prefix("Moon")
                .and_then(|s| s.parse::<usize>().ok())
            else {
                continue;
            };
            if !(1..4).contains(&i) {
                continue;
            }
            i
        };
        let number = |name, fallback| {
            node.attribute(name)
                .and_then(|s| s.parse::<f32>().ok())
                .unwrap_or(fallback)
        };
        let (latitude, longitude, size) = (
            number("Latitude", 240.0),
            number("Longitude", 90.0),
            number("Size", 0.5),
        );
        if !latitude.is_finite() || !longitude.is_finite() || !size.is_finite() {
            continue;
        }
        let Some(texture) = node.attribute("Texture").filter(|s| !s.is_empty()) else {
            continue;
        };
        let lat = latitude.to_radians() - std::f32::consts::PI;
        let lon = std::f32::consts::FRAC_PI_2 - longitude.to_radians();
        moons.push(Moon {
            slot,
            direction: Vec3::new(lat.cos() * lon.sin(), lat.sin() * lon.sin(), lon.cos()),
            size: 0.04 + 0.96 * size.clamp(0.0, 10.0),
            texture: crate::entities::asset_path(texture),
            order: node
                .attribute("Order")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
        });
    }
    // Native comparator 381B9080 sorts larger Order first; equal-order authored slots stay together.
    moons.sort_by(|a, b| b.order.cmp(&a.order));
    moons
}

fn basis(direction: Vec3) -> (Vec3, Vec3) {
    if (direction.z.abs() - 1.0).abs() <= 2.3841858e-7 {
        (Vec3::X, Vec3::Y)
    } else {
        let right = direction.cross(Vec3::Z).normalize();
        (right, right.cross(direction).normalize())
    }
}

#[derive(Clone, Copy, Debug, Default, ShaderType)]
pub struct MoonParams {
    /// xyz direction, w quad half width in the native unit-direction plane.
    direction: Vec4,
    /// xyz native right, w UV half-extent / corona alpha radius.
    right: Vec4,
    /// xyz native up, w existing sky radiance/exposure conversion.
    up: Vec4,
    color: Vec4,
    inner: Vec4,
    outer: Vec4,
}

impl MoonParams {
    fn new(moon: &Moon, direction: Vec3, sample: &MoonSample, exposure: f32) -> Self {
        let (right, up) = basis(direction);
        let corona = sample.outer_size;
        Self {
            direction: cry_to_bevy(direction).extend((moon.size + corona) * 0.57735026),
            right: cry_to_bevy(right).extend((corona / moon.size + 1.0) * 0.5),
            up: cry_to_bevy(up).extend(exposure),
            color: sample.color.extend(0.0),
            inner: sample.inner.extend(1.0 + sample.inner_scale * 1000.0),
            outer: sample.outer.extend(1.0 + sample.outer_scale * 1000.0),
        }
    }
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct MoonMaterial {
    #[uniform(0)]
    params: MoonParams,
    #[texture(1)]
    #[sampler(2)]
    texture: Handle<Image>,
    /// All sky meshes have the same origin. Preserve native moon order after additive stars.
    rank: u8,
}

impl Material for MoonMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://aa_world/moons.wesl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "embedded://aa_world/moons.wesl".into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }
    fn depth_bias(&self) -> f32 {
        crate::night_sky::SKY_SORT_BIAS + self.rank as f32 + 1.0
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
        descriptor.vertex.buffers = vec![
            layout
                .0
                .get_layout(&[Mesh::ATTRIBUTE_POSITION.at_shader_location(0)])?,
        ];
        descriptor.primitive.cull_mode = None;
        if let Some(depth) = &mut descriptor.depth_stencil {
            depth.depth_write_enabled = Some(false);
            depth.depth_compare = Some(CompareFunction::GreaterEqual);
        }
        Ok(())
    }
}

#[derive(Resource, Clone, Copy, Debug)]
pub struct MoonSettings {
    pub enabled: bool,
}
impl Default for MoonSettings {
    fn default() -> Self {
        Self {
            enabled: std::env::var_os("AA_NO_MOONS").is_none(),
        }
    }
}

#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct MoonStats {
    pub authored: usize,
    pub rendered: usize,
}

/// One authored sky disc, removed when the world changes.
#[derive(Component, Clone, Copy, Debug)]
pub struct MoonDisc {
    pub slot: usize,
}

struct Draw {
    moon: Moon,
    entity: Entity,
    material: Handle<MoonMaterial>,
}
type Loaded = (String, Vec<(Moon, Image)>);
#[derive(Resource, Default)]
struct State {
    world: String,
    loaded: bool,
    pending: Option<Mutex<Receiver<Loaded>>>,
    mesh: Option<Handle<Mesh>>,
    draws: Vec<Draw>,
    visible: bool,
}

pub(crate) fn build(app: &mut App) {
    embedded_asset!(app, "moons.wesl");
    app.add_plugins(MaterialPlugin::<MoonMaterial>::default())
        .init_resource::<State>()
        .init_resource::<MoonSettings>()
        .init_resource::<MoonStats>()
        .add_systems(Update, update.after(update_environment));
}

fn load(world: String) -> Loaded {
    let pak = aa_pak::Pak::global();
    let moons = pak
        .read_string(&format!("worlds/{world}/env.xml"))
        .ok()
        .map_or_else(Vec::new, |s| parse_moons(&s));
    let assets = moons
        .into_iter()
        .filter_map(|moon| {
            let bytes = pak.read(&moon.texture).ok()?;
            let mut image = aa_cry::texture::load_dds(
                &bytes,
                aa_cry::texture::DdsOptions {
                    srgb: true,
                    ..default()
                },
            )
            .ok()?;
            let mut sampler = aa_cry::texture::repeat_sampler();
            sampler.address_mode_u = bevy::image::ImageAddressMode::ClampToEdge;
            sampler.address_mode_v = bevy::image::ImageAddressMode::ClampToEdge;
            image.sampler = bevy::image::ImageSampler::Descriptor(sampler);
            Some((moon, image))
        })
        .collect();
    (world, assets)
}

#[allow(clippy::too_many_arguments)]
fn update(
    mut commands: Commands,
    cfg: Res<WorldConfig>,
    env: Res<Environment>,
    tuning: Res<AtmosphereTuning>,
    settings: Res<MoonSettings>,
    mut stats: ResMut<MoonStats>,
    mut state: ResMut<State>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<MoonMaterial>>,
    exposure: Query<Option<&Exposure>, With<PostUniform>>,
) {
    if state.world != cfg.world {
        for draw in state.draws.drain(..) {
            commands.entity(draw.entity).despawn();
            materials.remove(draw.material.id());
        }
        state.world = cfg.world.clone();
        state.loaded = false;
        state.visible = false;
    }
    let received = state
        .pending
        .as_ref()
        .and_then(|rx| match rx.lock().unwrap().try_recv() {
            Ok(v) => Some(Some(v)),
            Err(TryRecvError::Disconnected) => Some(None),
            Err(TryRecvError::Empty) => None,
        });
    if let Some(result) = received {
        state.pending = None;
        if let Some((world, assets)) = result {
            if world == state.world {
                state.loaded = true;
                state.visible = false;
                let mesh = state
                    .mesh
                    .get_or_insert_with(|| {
                        meshes.add(
                            Mesh::new(
                                PrimitiveTopology::TriangleList,
                                RenderAssetUsages::default(),
                            )
                            .with_inserted_attribute(
                                Mesh::ATTRIBUTE_POSITION,
                                vec![
                                    [-1.0f32, -1.0, 0.0],
                                    [1.0, -1.0, 0.0],
                                    [1.0, 1.0, 0.0],
                                    [-1.0, 1.0, 0.0],
                                ],
                            )
                            .with_inserted_indices(Indices::U32(vec![0, 1, 2, 0, 2, 3])),
                        )
                    })
                    .clone();
                for (rank, (moon, image)) in assets.into_iter().enumerate() {
                    let material = materials.add(MoonMaterial {
                        params: MoonParams::default(),
                        texture: images.add(image),
                        rank: rank as u8,
                    });
                    let entity = commands
                        .spawn((
                            Name::new(format!("Original moon {}", moon.slot)),
                            MoonDisc { slot: moon.slot },
                            Mesh3d(mesh.clone()),
                            MeshMaterial3d(material.clone()),
                            Transform::IDENTITY,
                            Visibility::Hidden,
                            NoFrustumCulling,
                            NotShadowCaster,
                        ))
                        .id();
                    state.draws.push(Draw {
                        moon,
                        entity,
                        material,
                    });
                }
            }
        } else {
            state.loaded = true;
            warn!("aa-world: moon asset task disconnected");
        }
    }
    // At most one pending task, even if a screen/world switch arrives while decoding.
    if cfg.environment && !state.loaded && state.pending.is_none() {
        let world = state.world.clone();
        let (tx, rx) = channel();
        state.pending = Some(Mutex::new(rx));
        aa_cry::workers::Workers::global().spawn(-60000.0, move || {
            let _ = tx.send(load(world));
        });
    }
    let visible = cfg.environment && settings.enabled && env.sample.is_some();
    if visible != state.visible {
        for draw in &state.draws {
            commands.entity(draw.entity).insert(if visible {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            });
        }
        state.visible = visible;
    }
    // A newly arrived world can inherit an already-visible setting.
    if visible && let Some(sample) = &env.sample {
        let exposure = tuning.radiance * view_exposure(&exposure);
        for (rank, draw) in state.draws.iter().enumerate() {
            if let Some(material) = materials.get_mut(&draw.material) {
                let direction = if rank == 0 {
                    sample.moon_dir
                } else {
                    draw.moon.direction
                };
                material.into_inner().params = MoonParams::new(
                    &draw.moon,
                    direction,
                    &sample.moons[draw.moon.slot],
                    exposure,
                );
            }
        }
    }
    stats.authored = state.draws.len();
    stats.rendered = if visible { state.draws.len() } else { 0 };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::{Lighting, TimeOfDay};

    #[test]
    fn native_xml_order_size_and_polar_basis() {
        let moons = parse_moons(
            r#"<Environment><Moon Size="20" Texture="game/a.dds" Order="1"/>
            <SecondayMoons><Moon1 Used="1" Size="-2" Texture="game/b.dds" Order="3"/>
            <Moon2 Used="0" Texture="game/c.dds"/><Moon3 Used="1" Texture=""/></SecondayMoons></Environment>"#,
        );
        assert_eq!(moons.iter().map(|m| m.slot).collect::<Vec<_>>(), [1, 0]);
        assert!((moons[0].size - 0.04).abs() < 1e-6);
        assert!((moons[1].size - 9.64).abs() < 1e-6);
        assert_eq!(moons[1].texture, "a.dds");
        for direction in [Vec3::Z, -Vec3::Z, Vec3::new(0.2, -0.6, 0.7).normalize()] {
            let (right, up) = basis(direction);
            assert!(right.dot(direction).abs() < 1e-6 && up.dot(direction).abs() < 1e-6);
            assert!((right.length() - 1.0).abs() < 1e-6 && (up.length() - 1.0).abs() < 1e-6);
            assert!(right.dot(up).abs() < 1e-6);
        }
    }

    #[test]
    fn real_moons_use_native_quad_uv_and_tod_units() {
        if !crate::pak_available() {
            return;
        }
        let pak = aa_pak::Pak::global();
        let moons = parse_moons(&pak.read_string("worlds/main_world/env.xml").unwrap());
        assert_eq!(moons.iter().map(|m| m.slot).collect::<Vec<_>>(), [0, 1]);
        assert!((moons[0].size - 0.2512).abs() < 1e-6 && (moons[1].size - 0.064).abs() < 1e-6);
        assert!((moons[1].direction - Vec3::new(0.6275069, -0.5265408, 0.5735765)).length() < 1e-6);
        let (_, assets) = load("main_world".to_string());
        assert_eq!(assets.len(), 2, "both original DDS textures decode");
        assert!(
            assets
                .iter()
                .all(|(_, image)| image.width() >= 64 && image.height() >= 64)
        );
        let tod = TimeOfDay::parse(
            &pak.read_string("worlds/main_world/zone/133/time_of_day.xml")
                .unwrap(),
        )
        .unwrap();
        // Identity clock warp evaluates the midnight spline key itself.
        let sample = tod.sample(
            0.0,
            Lighting {
                dawn_hour: 6.0,
                ..default()
            },
        );
        assert!(
            (sample.moons[0].color - Vec3::new(0.772549, 0.921569, 0.929412) * 0.75).length()
                < 1e-5
        );
        assert!((sample.moons[0].outer - Vec3::splat(0.37352902 * 0.088311)).length() < 1e-5);
        let p = MoonParams::new(&moons[0], sample.moon_dir, &sample.moons[0], 1.0);
        assert!(
            (p.direction.w - 0.6991296).abs() < 1e-5,
            "native total quad half width {:?}",
            p.direction.w
        );
        assert!(
            (p.right.w - 2.4102867).abs() < 1e-5,
            "corona expands UVs outside the disc texture"
        );
        assert!(
            (p.outer.w - 1039.3102).abs() < 0.001,
            "native 1 + 1000 * scale"
        );
        let noon = tod.sample(12.0, Lighting::default());
        assert!(sample.moon_dir.z > 0.7 && noon.moon_dir.z < 0.0);
        assert!(
            (sample.moon_dir + sample.sun_dir).length() > 0.7,
            "planet axis offset survives the half-turn"
        );
        assert!(noon.moons[0].color.length() < 1e-6);
        assert!(
            noon.moons[1].color.length() > 0.65,
            "secondary moon remains authored during daylight"
        );
    }

    #[test]
    fn runtime_disable_and_world_change_release_draws() {
        let mut app = App::new();
        let (tx, rx) = channel();
        let moon = parse_moons(r#"<Environment><Moon Texture="a.dds"/></Environment>"#).remove(0);
        tx.send(("main_world".to_string(), vec![(moon, Image::default())]))
            .unwrap();
        app.insert_resource(WorldConfig::new("main_world"))
            .insert_resource(Environment {
                sample: Some(TimeOfDay::default().sample(0.0, Lighting::default())),
                zone: None,
                hour: 0.0,
                ocean_fog: Vec3::ZERO, ocean_density: 0.0,
            })
            .init_resource::<AtmosphereTuning>()
            .init_resource::<MoonSettings>()
            .init_resource::<MoonStats>()
            .init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<Image>>()
            .init_resource::<Assets<MoonMaterial>>()
            .insert_resource(State {
                world: "main_world".to_string(),
                pending: Some(Mutex::new(rx)),
                ..default()
            })
            .add_systems(Update, update);
        app.update();
        let w = app.world_mut();
        assert_eq!(w.query::<&MoonDisc>().iter(w).count(), 1);
        assert_eq!(w.resource::<MoonStats>().rendered, 1);
        assert_eq!(
            w.query::<&Visibility>().single(w).unwrap(),
            &Visibility::Inherited
        );
        w.resource_mut::<MoonSettings>().enabled = false;
        app.update();
        assert_eq!(app.world().resource::<MoonStats>().rendered, 0);
        let w = app.world_mut();
        assert_eq!(
            w.query::<&Visibility>().single(w).unwrap(),
            &Visibility::Hidden
        );
        w.resource_mut::<WorldConfig>().world = "login2".to_string();
        w.resource_mut::<WorldConfig>().environment = false;
        app.update();
        let w = app.world_mut();
        assert_eq!(w.query::<&MoonDisc>().iter(w).count(), 0);
        assert_eq!(w.resource::<Assets<MoonMaterial>>().len(), 0);
        assert_eq!(w.resource::<MoonStats>().authored, 0);
    }

    #[test]
    fn sky_sort_stays_behind_glass_on_either_side_of_the_origin() {
        use bevy::core_pipeline::core_3d::TransparentSortingInfo3d;
        use bevy::render::render_phase::ViewRangefinder3d;
        let stars = crate::night_sky::StarMaterial { params: default() };
        let moon = |rank| MoonMaterial {
            params: default(),
            texture: default(),
            rank,
        };
        let (az, el) = (150.0_f32.to_radians(), 35.0_f32.to_radians());
        let look = cry_to_bevy(Vec3::new(
            az.sin() * el.cos(),
            az.cos() * el.cos(),
            el.sin(),
        ));
        for position in [
            Vec3::new(15579.5, 15385.85, 137.0),
            Vec3::new(30000.0, 10000.0, 800.0),
        ] {
            let at = cry_to_bevy(position);
            let camera = Transform::from_translation(at).looking_to(look, Vec3::Y);
            let rangefinder = ViewRangefinder3d::from_world_from_view(&camera.compute_affine());
            let key = |bias| {
                TransparentSortingInfo3d::Sorted {
                    mesh_center: Vec3::ZERO,
                    depth_bias: bias,
                }
                .sort_distance(&rangefinder)
            };
            let glass = rangefinder.distance(&(at + look * 50.0));
            assert!(key(stars.depth_bias()) < key(moon(0).depth_bias()));
            assert!(key(moon(0).depth_bias()) < key(moon(1).depth_bias()));
            assert!(
                key(moon(3).depth_bias()) < glass,
                "sky must draw before the nearby alpha pane"
            );
        }
    }
}
