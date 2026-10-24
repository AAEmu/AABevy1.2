//! Private supplied-coverage renderer mechanism proof, NOT native Illum completion.
//! Run with --shot ... --size 960x720. See README.md for controls and limits.
mod passes;
mod readback;
mod readiness;
mod scene;

use bevy::{
    asset::embedded_asset, pbr::*, prelude::*, render::render_resource::*, shader::ShaderRef,
};

#[derive(Asset, AsBindGroup, Reflect, Clone, Debug, Default)]
#[data(100, MarkerData, binding_array(101))]
#[bindless(index_table(range(100..101), binding(100)))]
struct SuppliedCoverage {
    marker: Vec4,
}
#[derive(ShaderType)]
struct MarkerData {
    marker: Vec4,
}
impl From<&SuppliedCoverage> for MarkerData {
    fn from(value: &SuppliedCoverage) -> Self {
        Self {
            marker: value.marker,
        }
    }
}

impl MaterialExtension for SuppliedCoverage {
    fn vertex_shader() -> ShaderRef {
        shader()
    }
    fn fragment_shader() -> ShaderRef {
        shader()
    }
    fn prepass_vertex_shader() -> ShaderRef {
        shader()
    }
    fn prepass_fragment_shader() -> ShaderRef {
        shader()
    }
    fn specialize(
        _: &MaterialExtensionPipeline,
        d: &mut RenderPipelineDescriptor,
        _: &bevy::mesh::MeshVertexBufferLayoutRef,
        _: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if d.label
            .as_deref()
            .is_some_and(|label| label.contains("prepass"))
        {
            d.vertex.shader_defs.push("SUPPLIED_PREPASS".into());
            if let Some(f) = &mut d.fragment {
                f.shader_defs.push("SUPPLIED_PREPASS".into());
            }
        }
        Ok(())
    }
}
fn shader() -> ShaderRef {
    "embedded://native_pass_probe/coverage.wesl".into()
}
type ProbeMaterial = ExtendedMaterial<StandardMaterial, SuppliedCoverage>;

pub(crate) fn arg(name: &str) -> Option<String> {
    let args: Vec<_> = std::env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}
pub(crate) fn flag(name: &str) -> bool {
    std::env::args().any(|a| a == name)
}

fn main() {
    let mut plugins = aa_dev::default_plugins("aa-cry: SUPPLIED coverage mechanism probe").build();
    let mut settings = bevy::render::settings::WgpuSettings::default();
    settings.features |= bevy::render::settings::WgpuFeatures::TIMESTAMP_QUERY
        | bevy::render::settings::WgpuFeatures::TIMESTAMP_QUERY_INSIDE_ENCODERS;
    if flag("--no-bindless") {
        settings.disabled_features =
            Some(bevy::render::settings::WgpuFeatures::TEXTURE_BINDING_ARRAY);
    }
    plugins = plugins.set(bevy::render::RenderPlugin {
        render_creation: settings.into(),
        ..default()
    });
    let mut app = App::new();
    app.add_plugins((
        plugins,
        aa_dev::DevPlugin,
        aa_cry::CryPlugin,
        bevy::render::diagnostic::RenderDiagnosticsPlugin,
    ));
    embedded_asset!(app, "examples/native_pass_probe", "coverage.wesl");
    app.add_plugins(MaterialPlugin::<ProbeMaterial>::default());
    if !flag("--stock-renderer") {
        app.add_plugins(passes::ProbePasses);
    }
    app.insert_resource(ClearColor(Color::BLACK))
        .add_systems(Startup, scene::setup)
        .add_systems(Update, (scene::animate, scene::timings));
    app.run();
}
