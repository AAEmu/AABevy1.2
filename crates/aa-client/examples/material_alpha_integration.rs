//! Original Illum material through the production nested LOD wrapper and prepasses.
//! Compare default with --direct for --view color|depth|normal; no network/world stream.
//! --lod-half checks the production half-dissolve tag against Bevy's same-distance control.
//! Coverage retains the current composed-alpha policy; native successful-Z,
//! depth-equal forward and independent motion coverage remain separate work.
use aa_cry::{CryAssets, CryPlugin, ModelOptions, Shading};
use aa_world::objects::{
    lod::BrushLodPart,
    lod_transition::{LodIllumMaterial, LodTransitionPlugin, MeshLodDither},
};
use bevy::{
    asset::embedded_asset,
    core_pipeline::{
        prepass::{DepthPrepass, NormalPrepass},
        tonemapping::{DebandDither, Tonemapping},
    },
    prelude::*,
    render::render_resource::AsBindGroup,
    shader::ShaderRef,
};

fn flag(name: &str) -> bool {
    std::env::args().any(|arg| arg == name)
}
fn view() -> String {
    let args: Vec<_> = std::env::args().collect();
    args.windows(2)
        .find(|a| a[0] == "--view")
        .map_or("color".into(), |a| a[1].clone())
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
struct PrepassView {
    #[uniform(0)]
    mode: u32,
}
impl Material for PrepassView {
    fn fragment_shader() -> ShaderRef {
        "embedded://material_alpha_integration/prepass_view.wesl".into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }
}

fn main() {
    let mut plugins = aa_dev::default_plugins("aa-client: nested native alpha prepass").build();
    if flag("--no-bindless") {
        plugins = plugins.set(bevy::render::RenderPlugin {
            render_creation: bevy::render::settings::WgpuSettings {
                disabled_features: Some(
                    bevy::render::settings::WgpuFeatures::TEXTURE_BINDING_ARRAY,
                ),
                ..default()
            }
            .into(),
            ..default()
        });
    }
    let mut app = App::new();
    app.add_plugins((plugins, aa_dev::DevPlugin, CryPlugin, LodTransitionPlugin));
    embedded_asset!(app, "examples", "prepass_view.wesl");
    app.add_plugins(MaterialPlugin::<PrepassView>::default())
        .insert_resource(ClearColor(Color::BLACK))
        .insert_resource(GlobalAmbientLight {
            brightness: 0.,
            ..default()
        })
        .add_systems(Startup, setup)
        .add_systems(Last, report);
    app.run();
}

fn setup(mut commands: Commands, mut cry: CryAssets, mut debug: ResMut<Assets<PrepassView>>) {
    let loaded = cry
        .load_model(
            "objects/effects/haje_hammer.cgf",
            &ModelOptions {
                material: Some("objects/effects/haje_hammer_second1.mtl".into()),
                shading: Shading::Full,
                material_hook: Some(aa_cry::MaterialHook::new(|_, m| m.unlit = true)),
                ..default()
            },
        )
        .unwrap();
    let handle = loaded.parts[0].1[0].material.clone();
    let mut rectangle = Mesh::from(Rectangle::new(2., 2.));
    let bevy::mesh::VertexAttributeValues::Float32x3(positions) =
        rectangle.attribute(Mesh::ATTRIBUTE_POSITION).unwrap()
    else {
        unreachable!()
    };
    let colors: Vec<_> = positions
        .iter()
        .map(|p| {
            [
                1.,
                1.,
                1.,
                if flag("--low-alpha") {
                    0.02
                } else {
                    (p[0] + 1.) * 0.5
                },
            ]
        })
        .collect();
    rectangle.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    let mut card = commands.spawn((
        Mesh3d(cry.meshes.add(rectangle)),
        MeshMaterial3d(handle),
        Transform::default(),
    ));
    if flag("--lod-half") {
        card.insert(bevy::camera::visibility::VisibilityRange {
            start_margin: 0.0..0.0,
            end_margin: 2.0..4.0,
            use_aabb: false,
        });
    }
    if !flag("--direct") {
        card.insert((
            BrushLodPart {
                cell: (0, 0),
                index: 0,
                requested: 0,
            },
            MeshLodDither {
                level: 8,
                active: flag("--lod-half"),
                ..default()
            },
        ));
    }
    commands.spawn((
        Camera3d::default(),
        DepthPrepass,
        NormalPrepass,
        Msaa::Off,
        Tonemapping::None,
        DebandDither::Disabled,
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: bevy::camera::ScalingMode::FixedVertical {
                viewport_height: 2.5,
            },
            ..OrthographicProjection::default_3d()
        }),
        Transform::from_xyz(0., 0., 3.).looking_at(Vec3::ZERO, Vec3::Y),
        bevy::camera::Exposure {
            ev100: (998.0_f32 / 1.2).log2(),
        },
    ));
    let mode = view();
    if mode != "color" {
        commands.spawn((
            Mesh3d(cry.meshes.add(Rectangle::new(16., 16.))),
            MeshMaterial3d(debug.add(PrepassView {
                mode: u32::from(mode == "normal"),
            })),
            Transform::from_xyz(0., 0., 1.),
        ));
    }
}

fn report(
    time: Res<Time>,
    mut done: Local<bool>,
    nested: Query<&MeshMaterial3d<LodIllumMaterial>>,
    plain: Query<&MeshMaterial3d<aa_cry::shade::illum::IllumMaterial>>,
    materials: Res<Assets<LodIllumMaterial>>,
    dithers: Query<&MeshLodDither>,
) {
    if *done || time.elapsed_secs() < 3. {
        return;
    }
    if !flag("--direct") {
        assert_eq!(
            nested.iter().count(),
            1,
            "production wrapper must be active"
        );
        let material = materials.get(&nested.single().unwrap().0).unwrap();
        assert!(
            material.base.extension.params.vertex.w > 0.,
            "private source token must survive nesting"
        );
        let dither = dithers.single().unwrap();
        assert_eq!(dither.active, flag("--lod-half"));
        assert_eq!(dither.level, 8);
    } else {
        assert_eq!(plain.iter().count(), 1);
    }
    info!(
        "nested alpha: direct={} view={} low={} lod-half={} depth+normal enabled, source token retained",
        flag("--direct"),
        view(),
        flag("--low-alpha"),
        flag("--lod-half")
    );
    *done = true;
}
