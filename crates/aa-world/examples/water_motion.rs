//! Original WaterSurface UV/motion GPU controls and original river/pond streaming fixture.
//! Default: numeric oracle. --world [--pond] [--legacy] [--unload-check]: original volumes.
//! --normal-stages / --legacy-normals freeze original wave time at zero and isolate normal-stage changes.

use aa_cry::material::{Material as CryMaterial, TextureMap};
use aa_cry::texture::{Dds, DdsOptions, dds_to_image};
use aa_world::water_material::{WaterMaterial, WaterMaterialPlugin, WaterParams};
use bevy::asset::AssetId;
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::prelude::*;
use bevy::render::view::screenshot::ScreenshotCaptured;
use std::collections::{HashMap, HashSet};

#[derive(Resource)]
struct Controls([[u8; 3]; 4]);

#[derive(Resource, Default)]
struct Fixture {
    reference: u8,
    pond: bool,
    legacy: bool,
    endpoint: bool,
    opaque: bool,
    shore: bool,
    soft_control: bool,
    normals: bool,
    old_normals: bool,
    cubes: bool,
    old_cube: bool,
    refraction: bool,
    old_refraction: bool,
    features: bool,
    old_features: bool,
    light_shading: bool,
    old_light_shading: bool,
    foam_uv: bool,
    old_foam_uv: bool,
    environment_2d: bool,
    old_environment_2d: bool,
    farm: bool,
    solzreed: bool,
    grazing: bool,
    unload: bool,
    moved: bool,
    done: bool,
    meshes: HashSet<AssetId<Mesh>>,
    controls: HashMap<AssetId<WaterMaterial>, Handle<WaterMaterial>>,
    last: f32,
}

fn main() {
    let flag = |s: &str| std::env::args().any(|a| a == s);
    let mut app = App::new();
    app.add_plugins((
        aa_dev::default_plugins("Native water authored UV / wave motion"),
        aa_dev::DevPlugin,
    ));
    if flag("--world") {
        let mut config = aa_world::WorldConfig::new(if flag("--farm") {"instance_hadir_farm"} else {"main_world"});
        config.load_radius = 180.0;
        config.unload_radius = 300.0;
        config.water_distance = 300.0;
        config.brush_distance = 180.0;
        config.tree_distance = 180.0;
        config.vegetation = false;
        app.add_plugins(aa_world::AaWorldPlugin { config })
            .insert_resource(aa_world::WorldClock {
                hour: if flag("--reference05") {
                    2.80975842
                } else if flag("--reference10") {
                    3.76261711
                } else if flag("--night") {
                    22.0
                } else {
                    10.0
                },
                ..default()
            })
            .insert_resource(Fixture {
                reference: if flag("--reference05") {
                    5
                } else if flag("--reference10") {
                    10
                } else {
                    0
                },
                pond: flag("--pond") || flag("--shore"),
                legacy: flag("--legacy"),
                endpoint: flag("--endpoint"),
                opaque: flag("--opaque"),
                shore: flag("--shore"),
                soft_control: flag("--legacy-soft"),
                normals: flag("--normal-stages") || flag("--legacy-normals"),
                old_normals: flag("--legacy-normals"),
                cubes: flag("--cube") || flag("--legacy-cube"),
                old_cube: flag("--legacy-cube"),
                refraction: flag("--refraction") || flag("--legacy-refraction"),
                old_refraction: flag("--legacy-refraction"),
                features: flag("--features") || flag("--legacy-features"),
                old_features: flag("--legacy-features"),
                light_shading: flag("--light-shading") || flag("--legacy-light-shading"),
                old_light_shading: flag("--legacy-light-shading"),
                foam_uv: flag("--foam-uv") || flag("--legacy-foam-uv"),
                old_foam_uv: flag("--legacy-foam-uv"),
                environment_2d: flag("--environment-2d") || flag("--legacy-environment-2d"),
                old_environment_2d: flag("--legacy-environment-2d"),
                farm: flag("--farm"),
                solzreed: flag("--solzreed"),
                grazing: flag("--grazing"),
                unload: flag("--unload-check"),
                ..default()
            })
            .add_systems(Startup, world_camera)
            .add_systems(Update, world_check);
    } else {
        app.add_plugins(WaterMaterialPlugin)
            .insert_resource(ClearColor(Color::BLACK))
            .add_systems(
                Startup,
                numeric.run_if(|| !std::env::args().any(|a| a == "--endpoints")),
            )
            .add_systems(
                Startup,
                endpoint_numeric.run_if(|| std::env::args().any(|a| a == "--endpoints")),
            )
            .add_observer(pixels);
    }
    app.run();
}

fn sample(rgba: &[u8], width: u32, height: u32, uv: Vec2) -> Vec2 {
    let p = uv * Vec2::new(width as f32, height as f32) - Vec2::splat(0.5);
    let lo = p.floor();
    let f = p - lo;
    let texel = |x: i32, y: i32| {
        let i =
            (y.rem_euclid(height as i32) * width as i32 + x.rem_euclid(width as i32)) as usize * 4;
        Vec2::new(rgba[i] as f32, rgba[i + 1] as f32) / 255.0
    };
    texel(lo.x as i32, lo.y as i32)
        .lerp(texel(lo.x as i32 + 1, lo.y as i32), f.x)
        .lerp(
            texel(lo.x as i32, lo.y as i32 + 1).lerp(texel(lo.x as i32 + 1, lo.y as i32 + 1), f.x),
            f.y,
        )
}

fn numeric(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<WaterMaterial>>,
    mut standard: ResMut<Assets<StandardMaterial>>,
) {
    let mut expected = [[0; 3]; 4];
    for i in 0..4 {
        let no_flow = i >= 2;
        let time = if i % 2 == 0 { 0.0 } else { 4.0 };
        let mtl = CryMaterial::load(if no_flow {
            "materials/ocean/lake_tank.mtl"
        } else {
            "materials/ocean/valley_river_watervolume1.mtl"
        })
        .unwrap();
        assert_eq!(mtl.gen_mask & 0x200 != 0, no_flow);
        let path = mtl.texture(&TextureMap::Bump).unwrap().pak_path();
        let bytes = aa_cry::read(&path).unwrap();
        let mut dds = Dds::parse(&bytes).unwrap();
        dds.swapped_rg = false;
        let rgba = dds.decode_rgba8(0, 0);
        let uv = Vec2::new(0.173, 0.283);
        let param = |s: &str| mtl.public_param(s).unwrap().parse::<f32>().unwrap();
        let speed = param("WavesSpeed");
        let drift = Vec2::new(time * speed * 0.0125, 0.0);
        let taps = if no_flow {
            [
                uv * 2.0 + drift * 2.0,
                uv * 3.0 - drift * 2.0,
                uv * 4.0 + drift * 2.0,
                uv * 5.0 - drift * 2.0,
            ]
        } else {
            [
                uv * 4.0 + drift * 2.0,
                uv * 2.0 + drift * 3.0,
                uv * 8.0 + drift * 2.0,
                uv * 16.0 + drift * 3.0,
            ]
        };
        let samples = taps.map(|uv| sample(&rgba, dds.width, dds.height, uv));
        let big = param("BigWavesScale");
        let small = param("SmallWavesScale");
        let bump = param("BumpScale");
        let xy = ((samples[0] + samples[1] - Vec2::ONE) * big
            + (samples[2] + samples[3] - Vec2::ONE) * small)
            * bump;
        let normal = Vec3::new(xy.x, 1.0, -xy.y).normalize();
        let colour = normal * 0.5 + Vec3::splat(0.5);
        expected[i] = colour.to_array().map(|v| {
            ((if v <= 0.0031308 {
                v * 12.92
            } else {
                1.055 * v.powf(1.0 / 2.4) - 0.055
            }) * 255.0)
                .round() as u8
        });
        let image = images.add(dds_to_image(&dds, DdsOptions::linear()).unwrap());
        let mut mesh = Mesh::from(Rectangle::new(1.5, 1.5));
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![uv.to_array(); 4]);
        let params = WaterParams {
            waves: Vec4::new(big, small, speed, 0.2),
            refl: Vec4::new(0.0, 0.0, bump, 1.0),
            motion: Vec4::new(1.0, if no_flow { 1.0 } else { 0.0 }, time, 1.0),
            ..default()
        };
        commands.spawn((
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(materials.add(WaterMaterial {
                params,
                normal_map: Some(image),
                foam_map: None,
                env_map: None,
                environment_2d: None,
                gloss_map: None,
                caustics_map: None,
            })),
            Transform::from_xyz(i as f32 * 2.0 - 3.0, 0.0, -1.0)
                .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
        ));
        commands.spawn((
            Mesh3d(meshes.add(Rectangle::new(1.5, 1.5))),
            MeshMaterial3d(standard.add(StandardMaterial {
                base_color: Color::linear_rgb(colour.x, colour.y, colour.z),
                unlit: true,
                ..default()
            })),
            Transform::from_xyz(i as f32 * 2.0 - 3.0, 0.0, 1.0)
                .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
        ));
        info!(
            "native water GPU case{i}: NO_FLOW={no_flow} time={time} uv={uv:?} taps={taps:?} raw_samples={samples:?} expected_srgb={:?}",
            expected[i]
        );
    }
    commands.insert_resource(Controls(expected));
    commands.spawn((
        Camera3d::default(),
        bevy::pbr::ScreenSpaceTransmission::default(),
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: bevy::camera::ScalingMode::FixedVertical {
                viewport_height: 6.0,
            },
            ..OrthographicProjection::default_3d()
        }),
        Tonemapping::None,
        DebandDither::Disabled,
        Msaa::Off,
        Transform::from_xyz(0.0, 6.0, 0.0).looking_at(Vec3::ZERO, Vec3::NEG_Z),
    ));
    commands.spawn((Text::new("Original river flow t=0 / t=4 / pond NO_FLOW t=0 / t=4\nTop: production wave shader; bottom: independent original DDS bilinear controls\nNative mesh UVs + PB_time 0.0125 + original material scales; raw texture XY"),TextFont {font_size:FontSize::Px(20.0),..default()},TextColor(Color::WHITE),Node {position_type:PositionType::Absolute,top:px(12.0),left:px(24.0),..default()}));
}

fn pixels(shot: On<ScreenshotCaptured>, controls: Res<Controls>) {
    let image = shot.image.clone().try_into_dynamic().unwrap().to_rgb8();
    let (w, h) = image.dimensions();
    for i in 0..4 {
        let x = (w as i32 / 2 + (i as i32 * 2 - 3) * h as i32 / 6) as u32;
        let gpu = image.get_pixel(x, h / 3).0;
        let cpu = image.get_pixel(x, 2 * h / 3).0;
        for c in 0..3 {
            assert!(
                gpu[c].abs_diff(cpu[c]) <= 3 && gpu[c].abs_diff(controls.0[i][c]) <= 3,
                "water case{i} GPU={gpu:?} CPU={cpu:?} expected={:?}",
                controls.0[i]
            );
        }
        info!("native water UV/motion GPU case{i} PASSED GPU={gpu:?} CPU={cpu:?}");
    }
}

fn world_camera(
    mut commands: Commands,
    fixture: Res<Fixture>,
    terrain: Res<aa_world::TerrainQuery>,
    water: Res<aa_world::WaterQuery>,
) {
    let (p, t, fov) = if fixture.solzreed {
        // Original cell13,13 lake_solzreed: native polygon + B-D terrain selection.
        let target=Vec3::new(14168.5,14263.5,112.93799);
        let depth=target.z-terrain.height_at(target.x,target.y).unwrap();
        assert!((0.1..0.3).contains(&depth));
        assert!((water.water_at(target+Vec3::Z*0.1).unwrap().0-target.z).abs()<0.001);
        let mut camera=target+Vec3::new(5.0,7.0,4.0);
        camera.z=camera.z.max(terrain.height_at(camera.x,camera.y).unwrap()+2.5);
        info!("native source lake_solzreed target={target:?} true depth={depth} camera={camera:?}");
        (camera,target,55.0_f32.to_radians())
    } else if fixture.farm {
        // Original instance_hadir_farm type11 swamp_rescue six-point outline.
        let level=136.13449;
        let target=Vec3::new(756.0,680.0,level);
        assert!((water.water_at(target+Vec3::Z*0.1).unwrap().0-level).abs()<0.001);
        (target+Vec3::new(0.0,-14.0,10.0),target,55.0_f32.to_radians())
    } else if fixture.reference != 0 {
        // Original reference/ingame/combat_{triple_slash,return}/camera.json before_key_press.
        let (p, yaw) = if fixture.reference == 5 {
            (
                Vec3::new(15573.67773, 15381.66406, 127.979904),
                -115.528885_f32,
            )
        } else {
            (
                Vec3::new(15572.85498, 15379.30811, 127.979904),
                -148.365387_f32,
            )
        };
        let yaw = yaw.to_radians();
        (p, p + Vec3::new(-yaw.sin(), yaw.cos(), 0.0), 1.24354708)
    } else if fixture.grazing {
        // Original pond polygon/UV/terrain audit: 77 wet receiver rays below the horizon.
        let camera = Vec3::new(15549.375,15380.0,127.75);
        let target = Vec3::new(15575.375,15366.0,127.0);
        assert!(terrain.height_at(camera.x,camera.y).unwrap()<127.0);
        assert_eq!(water.water_at(camera).unwrap().0,127.0);
        (camera,target,55.0_f32.to_radians())
    } else if fixture.shore {
        // Pick an actual shallow pond point from original terrain and volume data.
        let anchor = Vec2::new(15564.0, 15346.0);
        let mut best: Option<(f32, Vec3, f32)> = None;
        for y in 0..132 {
            for x in 0..140 {
                let p = Vec2::new(15540.0 + x as f32 * 0.5, 15332.0 + y as f32 * 0.5);
                let Some((surface, _)) = water.water_at(p.extend(128.0)) else {
                    continue;
                };
                let Some(ground) = terrain.height_at(p.x, p.y) else {
                    continue;
                };
                let depth = surface - ground;
                if !(0.1..0.3).contains(&depth) {
                    continue;
                }
                let score = (depth - 0.15).abs() + p.distance_squared(anchor) * 0.0001;
                if best.is_none_or(|b| score < b.0) {
                    best = Some((score, p.extend(surface), depth));
                }
            }
        }
        let (_, target, depth) = best.expect("original shallow pond fixture");
        let mut camera = target + Vec3::new(-5.0, -7.0, 4.0);
        camera.z = camera
            .z
            .max(terrain.height_at(camera.x, camera.y).unwrap() + 2.5);
        info!(
            "native shoreline fixture: target={target:?} true vertical depth={depth} camera={camera:?}"
        );
        (camera, target, 55.0_f32.to_radians())
    } else if fixture.pond {
        (
            Vec3::new(15570.0, 15335.0, 139.0),
            Vec3::new(15580.0, 15375.0, 127.0),
            60.0_f32.to_radians(),
        )
    } else if fixture.endpoint {
        (
            Vec3::new(9268.0, 12755.0, 205.0),
            Vec3::new(9239.0, 12785.0, 189.4),
            45.0_f32.to_radians(),
        )
    } else {
        (
            Vec3::new(9245.0, 12815.0, 211.0),
            Vec3::new(9218.0, 12792.0, 189.4),
            60.0_f32.to_radians(),
        )
    };
    commands.spawn((
        Camera3d::default(),
        bevy::pbr::ScreenSpaceTransmission::default(),
        Projection::Perspective(PerspectiveProjection { fov, ..default() }),
        Transform::from_translation(aa_world::cry_to_bevy(p))
            .looking_at(aa_world::cry_to_bevy(t), Vec3::Y),
    ));
}

fn world_check(
    mut fixture: ResMut<Fixture>,
    time: Res<Time>,
    mut surfaces: Query<(
        &aa_world::water::WaterSurface,
        &Mesh3d,
        &mut MeshMaterial3d<WaterMaterial>,
    )>,
    mut materials: ResMut<Assets<WaterMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut camera: Query<&mut Transform, With<Camera3d>>,
    objects: Res<aa_world::objects::ObjectState>,
    styles: Res<aa_world::water_material::WaterStyles>,
    environment: Res<aa_world::sky::Environment>,
    tuning: Res<aa_world::sky::AtmosphereTuning>,
    exposure: Query<Option<&bevy::camera::Exposure>,With<aa_world::post::PostUniform>>,
) {
    for (_, mesh, mut material) in &mut surfaces {
        if fixture.meshes.insert(mesh.id()) && fixture.opaque {
            let mut mesh = meshes.get_mut(mesh.id()).unwrap();
            let count = mesh.count_vertices();
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0; 4]; count]);
        }
        if (fixture.legacy || fixture.shore || fixture.soft_control || fixture.normals || fixture.cubes || fixture.refraction || fixture.features || fixture.environment_2d || fixture.foam_uv || fixture.light_shading)
            && time.elapsed_secs() > 8.0
            && !fixture.controls.values().any(|h| h.id() == material.id())
        {
            if let Some(handle) = fixture.controls.get(&material.id()) {
                material.0 = handle.clone();
            } else if let Some(source) = materials.get(material.id()) {
                let mut control = source.clone();
                if fixture.legacy {
                    control.params.motion.x = 0.0;
                    control.params.motion.y = 0.0;
                }
                if fixture.shore {
                    control.params.motion.z = 0.0;
                }
                if fixture.soft_control {
                    control.params.motion.w = -1.0;
                }
                if fixture.normals {
                    control.params.motion.z = 0.0;
                    control.params.motion.w = if fixture.old_normals { -2.0 } else { 0.0 };
                }
                if fixture.cubes {
                    assert_eq!(control.params.motion.x,aa_world::water_material::AUTHORED_WATER_CUBE);
                    control.params.motion.z = 0.0;
                    if fixture.old_cube {
                        control.params.motion.x = aa_world::water_material::AUTHORED_WATER_UV;
                    }
                }
                if fixture.refraction {
                    assert!(control.params.motion.x>0.5);
                    control.params.motion.z=0.0;
                    control.params.motion.w=if fixture.old_refraction {-3.0} else {0.0};
                }
                if fixture.features {
                    assert!(control.params.motion.x>0.5);
                    control.params.motion.z=0.0;
                    if fixture.old_features {
                        let style=styles.style_for(material.id()).expect("cached source style");
                        let mtl=CryMaterial::load(&format!("{}.mtl",style.material.trim_end_matches(".mtl"))).unwrap();
                        let value=|name:&str,default:f32| mtl.public_param(name).and_then(|v|v.parse().ok()).unwrap_or(default);
                        let cur=environment.sample.as_ref().unwrap();
                        let rs=tuning.radiance*exposure.iter().next().flatten().map_or(bevy::camera::Exposure::BLENDER.exposure(),|e|e.exposure());
                        control.params.sun=(cur.sun*rs*value("SunMultiplier",0.1)*tuning.water_sun).extend(control.params.sun.w);
                        control.params.foam_col=((cur.sun+cur.sky)*rs*value("FoamMultiplier",0.75)*tuning.water_foam).extend(0.0);
                    }
                }
                if fixture.light_shading {
                    control.params.motion.z=0.0;
                    if fixture.old_light_shading && control.params.motion.x>=2.0 {
                        let style=styles.style_for(material.id()).expect("cached source style");
                        let mtl=CryMaterial::load(&format!("{}.mtl",style.material.trim_end_matches(".mtl"))).unwrap();
                        let foam=mtl.public_param("FoamMultiplier").and_then(|v|v.parse::<f32>().ok()).unwrap_or(0.75);
                        let cur=environment.sample.as_ref().unwrap();
                        let rs=tuning.radiance*exposure.iter().next().flatten().map_or(bevy::camera::Exposure::BLENDER.exposure(),|e|e.exposure());
                        control.params.env_lit=((cur.sun+cur.sky)*rs).extend(control.params.env_lit.w);
                        if mtl.gen_mask&0x1000!=0 { control.params.foam_col=((cur.sun+cur.sky)*rs*foam*tuning.water_foam).extend(0.0); }
                    }
                    info!("water light shading: source_mode={} legacy={}",control.params.motion.x,fixture.old_light_shading);
                }
                if fixture.foam_uv {
                    control.params.motion.z=0.0;
                    if fixture.old_foam_uv {control.params.motion.w=-5.0;}
                    info!("water foam UV control: native={} legacy={}",control.params.motion.x>0.5,fixture.old_foam_uv);
                }
                if fixture.environment_2d {
                    control.params.motion.z=0.0;
                    if fixture.old_environment_2d {control.params.motion.w=-4.0;}
                    info!("water environment2d control: source_mode={} bound2d={} legacy={}",control.params.motion.x,control.environment_2d.is_some(),fixture.old_environment_2d);
                }
                let original = material.id();
                let handle = materials.add(control);
                fixture.controls.insert(original, handle.clone());
                material.0 = handle;
            }
        }
    }
    if time.elapsed_secs() > fixture.last + 2.0 {
        fixture.last = time.elapsed_secs();
        info!(
            "native water world: active_styles={} captured_meshes={} vertices={} pending_models={} failed_models={}",
            surfaces.iter().count(),
            fixture.meshes.len(),
            fixture
                .meshes
                .iter()
                .filter_map(|id| meshes.get(*id))
                .map(Mesh::count_vertices)
                .sum::<usize>(),
            objects.stats.models_pending,
            objects.stats.models_failed
        );
    }
    if fixture.unload && !fixture.moved && time.elapsed_secs() > 12.0 {
        assert!(!fixture.meshes.is_empty());
        camera.single_mut().unwrap().translation =
            aa_world::cry_to_bevy(Vec3::new(-100000.0, -100000.0, 1000.0));
        fixture.moved = true;
    }
    if fixture.moved && !fixture.done && time.elapsed_secs() > 16.0 {
        assert_eq!(surfaces.iter().count(), 0);
        assert!(fixture.meshes.iter().all(|id| meshes.get(*id).is_none()));
        info!(
            "native water unload PASSED: all {} cell mesh IDs and water entities released; shared style texture cache retains its existing lifetime",
            fixture.meshes.len()
        );
        fixture.done = true;
    }
}

// Isolated controls for the two independently interpolated native alpha channels.
// --composite also exercises original fog + reflection compositing through depth/scene buffers.
fn endpoint_numeric(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<WaterMaterial>>,
    mut standard: ResMut<Assets<StandardMaterial>>,
) {
    let composite = std::env::args().any(|a| a == "--composite");
    let near = std::env::args().any(|a| a == "--near");
    let shallow = std::env::args().any(|a| a == "--shallow");
    let sea_control = std::env::args().any(|a| a == "--sea-control");
    assert!(!sea_control || shallow);
    assert!(!shallow || composite);
    assert!(!composite || !near);
    let blue = Vec3::new(0.05, 0.1, 0.2);
    let fog = Vec3::new(0.6, 0.15, 0.05);
    let reflection = Vec3::new(0.1, 0.7, 0.3);
    let normal = images.add(Image::new_fill(
        bevy::render::render_resource::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        bevy::render::render_resource::TextureDimension::D2,
        &[255, 128, 0, 255],
        bevy::render::render_resource::TextureFormat::Rgba8Unorm,
        bevy::asset::RenderAssetUsages::default(),
    ));
    let mut expected = [[0; 3]; 4];
    for (i, a) in [0.0_f32, 0.25, 0.5, 1.0].into_iter().enumerate() {
        let a = if near || shallow { 1.0 } else { a };
        let clip_w = if near {
            [0.5_f32, 1.0, 2.0, 8.0][i]
        } else {
            8.0
        };
        let depth = if shallow {
            [0.05_f32, 0.1, 0.4, 1.2][i]
        } else {
            4.0
        };
        let point = Vec3::new(i as f32 * 2.0 - 3.0, 0.0, -1.0);
        let view = (Vec3::new(0.0, 8.0, 0.0) - point).normalize();
        let ray_depth = depth / view.y;
        let soft = ray_depth.clamp(0.0, 1.0);
        let surface = if sea_control {
            1.0
        } else {
            a * (a * (1.0 - (1.0 / clip_w).clamp(0.0, 1.0))).sqrt()
        };
        let color = if composite {
            let transmission = 1.0 - a + a * (-4.0_f32.ln() / 4.0 * ray_depth).exp();
            let background = fog.lerp(blue, transmission);
            if shallow {
                // Nonzero authored-style bump crosses the real blue/red depth-matched edge.
                // The old contribution-only soft mask leaves this refraction opaque.
                let red = Vec3::new(0.7, 0.05, 0.02);
                let normal = Vec3::new(0.5, 1.0, -0.5 / 255.0).normalize();
                let fresnel_normal = if sea_control {
                    Vec3::Y.lerp(normal, 0.2).normalize()
                } else {
                    normal * Vec3::new(0.15, 1.0, 0.15)
                };
                let ndot = view.dot(fresnel_normal).abs();
                let fresnel = ((0.2 + 0.8 * (1.0 - ndot).powi(5)) * 0.7).clamp(0.0, 1.0);
                let refracted = fog.lerp(red, transmission);
                if sea_control {
                    background.lerp(refracted.lerp(reflection, fresnel * soft), surface)
                } else {
                    background.lerp(refracted.lerp(reflection, fresnel), surface * soft)
                }
            } else {
                background.lerp(reflection, surface)
            }
        } else {
            Vec3::new(a, surface, 0.0)
        };
        expected[i] = color.to_array().map(|v| {
            ((if v <= 0.0031308 {
                v * 12.92
            } else {
                1.055 * v.powf(1.0 / 2.4) - 0.055
            }) * 255.0)
                .round() as u8
        });
        let mut mesh = Mesh::from(Rectangle::new(1.5, 1.5));
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0, 1.0, 1.0, a]; 4]);
        let params = WaterParams {
            body: fog.extend(4.0_f32.ln() / 4.0),
            refl: if shallow {
                Vec4::new(0.2, 0.7, 0.5, 1.0)
            } else {
                Vec4::new(1.0, 1.0, 0.0, 1.0)
            },
            waves: Vec4::new(1.0, 0.0, 0.0, 0.2),
            sun: Vec4::new(0.0, 0.0, 0.0, 1.0),
            sky_h: reflection.extend(1.0),
            sky_z: reflection.extend(0.0),
            misc: Vec4::new(0.0, if shallow { 0.05 } else { 0.0 }, 0.0, 1.0),
            motion: Vec4::new(
                if sea_control { 0.0 } else { 1.0 },
                0.0,
                0.0,
                if composite { 0.0 } else { 2.0 },
            ),
            ..default()
        };
        commands.spawn((
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(materials.add(WaterMaterial {
                params,
                normal_map: if shallow { Some(normal.clone()) } else { None },
                foam_map: None,
                env_map: None,
                environment_2d: None,
                gloss_map: None,
                caustics_map: None,
            })),
            Transform::from_xyz(
                (i as f32 * 2.0 - 3.0) * clip_w / 8.0,
                8.0 - clip_w,
                -clip_w / 8.0,
            )
            .with_scale(Vec3::splat(clip_w / 8.0))
            .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
        ));
        commands.spawn((
            Mesh3d(meshes.add(Rectangle::new(1.5, 1.5))),
            MeshMaterial3d(standard.add(StandardMaterial {
                base_color: Color::linear_rgb(color.x, color.y, color.z),
                unlit: true,
                ..default()
            })),
            Transform::from_xyz(i as f32 * 2.0 - 3.0, 0.0, 1.0)
                .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
        ));
        if shallow {
            let ratio = (8.0 + depth) / 8.0;
            // Split the background 0.005m right of the sampled surface pixel. Both
            // halves have the same measured depth, so refraction cannot bypass it.
            for (side, background) in [(-1.0, blue), (1.0, Vec3::new(0.7, 0.05, 0.02))] {
                commands.spawn((
                    Mesh3d(meshes.add(Rectangle::new(0.75, 1.5))),
                    MeshMaterial3d(standard.add(StandardMaterial {
                        base_color: Color::linear_rgb(background.x, background.y, background.z),
                        unlit: true,
                        ..default()
                    })),
                    Transform::from_xyz((point.x + 0.005 + side * 0.375) * ratio, -depth, -ratio)
                        .with_scale(Vec3::splat(ratio))
                        .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
                ));
            }
        }
        info!(
            "native endpoint GPU case{i}: authored={a} clipW={clip_w} surface={surface} ray_depth={ray_depth} soft={soft} shallow={shallow} sea_control={sea_control} composite={composite} expected={:?}",
            expected[i]
        );
    }
    commands.insert_resource(Controls(expected));
    if !shallow {
        commands.spawn((
            Mesh3d(meshes.add(Plane3d::default().mesh().size(100.0, 100.0))),
            MeshMaterial3d(standard.add(StandardMaterial {
                base_color: Color::linear_rgb(blue.x, blue.y, blue.z),
                unlit: true,
                ..default()
            })),
            Transform::from_xyz(0.0, -4.0, 0.0),
        ));
    }
    commands.spawn((
        Camera3d::default(),
        bevy::pbr::ScreenSpaceTransmission::default(),
        Projection::Perspective(PerspectiveProjection {
            fov: 2.0 * (3.0_f32 / 8.0).atan(),
            ..default()
        }),
        bevy::core_pipeline::prepass::DepthPrepass,
        Tonemapping::None,
        DebandDither::Disabled,
        Msaa::Off,
        Transform::from_xyz(0.0, 8.0, 0.0).looking_at(Vec3::ZERO, Vec3::NEG_Z),
    ));
    commands.spawn((Text::new(format!("Native river endpoint alpha: 0 / 0.25 / 0.5 / 1; clip W=8 (or --near: .5 / 1 / 2 / 8)\nTop production shader, bottom independent CPU reference; composite={composite}, shallow refraction={shallow}, sea control={sea_control}\nFog: 1-a+T*a; surface: a*sqrt(a*(1-1/W))")),TextFont {font_size:FontSize::Px(20.0),..default()},TextColor(Color::WHITE),Node {position_type:PositionType::Absolute,top:px(12.0),left:px(24.0),..default()}));
}
