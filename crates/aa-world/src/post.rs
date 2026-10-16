//! Camera post processing: an HDR camera, atmosphere (height) fog and the engine's film curve.
//!
//! * [`Post`] on a `Camera3d` turns the camera into an HDR one with a depth prepass, bloom and our own tone mapping
//!   pass. The world's focus camera gets it automatically ([`crate::WorldConfig::post`]); an application that
//!   spawns other cameras can add `Post::default()` itself.
//! * [`PostUniform`] carries the time of day parameters of the frame (fog, film curve, grading); `sky.rs` fills
//!   it each frame.
//! * The pass (`post.wesl`) runs after Bevy's bloom and before its tone mapping (which is switched off): it
//!   reads the HDR colour and the main pass depth, applies the height fog and maps the result with the
//!   `Filmic` curve of `postprocess.cfx` (`Film curve *`, `Saturation`, `Contrast`, `Levels` of the zone's
//!   time of day), writing display linear colour.

use bevy::asset::{embedded_asset, load_embedded_asset};
use bevy::camera::{Camera3d, Camera3dDepthTextureUsage, Hdr};
use bevy::anti_alias::smaa::{Smaa, SmaaPreset};
use bevy::core_pipeline::prepass::DepthPrepass;
use bevy::core_pipeline::schedule::Core3d;
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping, tonemapping};
use bevy::core_pipeline::{Core3dSystems, FullscreenShader};
use bevy::math::{Vec3, Vec4};
use bevy::post_process::bloom::{Bloom, bloom};
use bevy::pbr::{ScreenSpaceAmbientOcclusion, ScreenSpaceAmbientOcclusionQualityLevel};
use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::diagnostic::RecordDiagnostics;
use bevy::render::extract_component::{
    ComponentUniforms, DynamicUniformIndex, ExtractComponent, ExtractComponentPlugin, UniformComponentPlugin,
};
use bevy::render::render_resource::binding_types::{texture_2d, texture_depth_2d, texture_depth_2d_multisampled, uniform_buffer};
use bevy::render::render_resource::{
    BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries, CachedRenderPipelineId, ColorTargetState, ColorWrites,
    FragmentState, LoadOp, Operations, PipelineCache, RenderPassColorAttachment, RenderPassDescriptor,
    RenderPipelineDescriptor, ShaderStages, ShaderType, SpecializedRenderPipeline,
    SpecializedRenderPipelines, StoreOp, TextureFormat, TextureSampleType, TextureUsages,
};
use bevy::render::renderer::{RenderContext, ViewQuery};
use bevy::render::view::{ExtractedView, Msaa, ViewDepthStencilTexture, ViewTarget, ViewUniform, ViewUniformOffset, ViewUniforms};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::Shader;

/// Sample radius of the ambient occlusion in metres: deeper shade under bushes needs more than Bevy's 0.73.
const SSAO_RADIUS: f32 = 1.2;

/// Marks a camera for the world's post processing (see the module docs). Adding it sets up the camera.
#[derive(Component, Clone, Debug)]
pub struct Post {
    /// Bloom intensity (0 turns it off).
    pub bloom: f32,
    /// Apply the atmosphere fog in the post pass (the camera's `DistanceFog` is removed).
    pub fog: bool,
    /// Exposure compensation in stops on top of the time of day's.
    pub exposure_ev: f32,
    /// Screen space ambient occlusion (Bevy's GTAO: needs the normal prepass and switches MSAA off for SMAA). Off by default:
    /// the normal prepass without MSAA turns the anisotropic hair material white (`AA_TUNE=ssao=1` also enables it).
    pub ssao: bool,
}

impl Default for Post {
    fn default() -> Self {
        Post { bloom: 0.1, fog: true, exposure_ev: 0.0, ssao: false }
    }
}

/// Camera components the post pass needs, applied once per `Post` camera.
#[derive(Component)]
struct PostReady;

/// Per frame parameters of the pass (the time of day, see `post.wesl`); a component of the camera.
#[derive(Component, Clone, Copy, Debug, ShaderType, ExtractComponent)]
#[extract_app(RenderApp)]
pub struct PostUniform {
    pub fog_color: Vec4,
    pub fog_top: Vec4,
    pub fog_layer: Vec4,
    pub fog_params: Vec4,
    pub sun_dir: Vec4,
    pub sun_glow: Vec4,
    pub film: Vec4,
    pub grade: Vec4,
    pub balance: Vec4,
}

impl Default for PostUniform {
    fn default() -> Self {
        PostUniform {
            fog_color: Vec4::new(0.5, 0.5, 0.5, 0.0),
            fog_top: Vec4::new(0.5, 0.5, 0.5, 0.0),
            fog_layer: Vec4::new(100.0, 1500.0, 0.9, 0.03),
            fog_params: Vec4::new(60.0, 0.0008, 1.0, 9000.0),
            sun_dir: Vec4::new(0.0, 1.0, 0.0, 8.0),
            sun_glow: Vec4::ZERO,
            film: Vec4::new(0.9, 1.0, 0.8, 1.8),
            grade: Vec4::new(1.0, 1.0, 0.0, 1.0),
            balance: Vec4::new(1.0, 1.0, 1.0, 0.0),
        }
    }
}

impl PostUniform {
    /// Tone mapping parameters of a time of day sample.
    pub fn film_from(sample: &crate::env::EnvSample, exposure: f32) -> (Vec4, Vec4, Vec4) {
        let film = if sample.film.w > 0.1 { sample.film } else { Vec4::new(0.9, 1.0, 0.8, 1.8) };
        let saturation = if sample.saturation > 0.01 { sample.saturation } else { 1.0 };
        let contrast = if sample.contrast > 0.01 { sample.contrast } else { 1.0 };
        let balance = sample.color_balance;
        (film, Vec4::new(saturation, contrast, sample.levels_min.clamp(0.0, 0.5), exposure), balance.extend(0.0))
    }
}

pub(crate) fn build(app: &mut App) {
    embedded_asset!(app, "post.wesl");
    app.add_plugins((ExtractComponentPlugin::<PostUniform>::default(), UniformComponentPlugin::<PostUniform>::default()))
        .add_systems(Update, setup_post_cameras);
    let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
    render_app
        .init_resource::<SpecializedRenderPipelines<PostPipeline>>()
        .add_systems(RenderStartup, init_post_pipeline)
        .add_systems(Render, prepare_post_pipelines.in_set(RenderSystems::Prepare))
        .add_systems(Core3d, post_pass.after(bloom).before(tonemapping).in_set(Core3dSystems::PostProcess));
}

/// Gives every camera with a [`Post`] the HDR / depth / bloom setup (once), and the world's own camera (the focus
/// entity, else the first `Camera3d`) a default `Post`.
fn setup_post_cameras(
    mut commands: Commands,
    cfg: Res<crate::stream::WorldConfig>,
    tuning: Res<crate::sky::AtmosphereTuning>,
    focus: Query<Entity, With<crate::stream::WorldFocus>>,
    mut cameras: Query<(Entity, &mut Camera3d, Option<&Post>, Option<&PostReady>)>,
) {
    if !cfg.environment || !cfg.post {
        return;
    }
    let focus = focus.iter().next();
    let main = cameras.iter().find(|c| Some(c.0) == focus).or_else(|| cameras.iter().next()).map(|c| c.0);
    for (entity, mut cam, post, ready) in &mut cameras {
        if ready.is_some() || (post.is_none() && Some(entity) != main) {
            continue;
        }
        let post = post.cloned().unwrap_or_default();
        let usages = TextureUsages::from(cam.depth_texture_usages) | TextureUsages::TEXTURE_BINDING;
        cam.depth_texture_usages = Camera3dDepthTextureUsage::from(usages);
        commands.entity(entity).insert((
            post.clone(),
            PostReady,
            PostUniform::default(),
            Hdr,
            Tonemapping::None,
            DebandDither::Disabled,
            DepthPrepass,
            Bloom { intensity: post.bloom, ..Bloom::NATURAL },
        ));
        if post.ssao || tuning.ssao > 0.5 {
            // SSAO cannot run on a multisampled depth buffer: SMAA takes over the edges
            commands.entity(entity).insert((
                Msaa::Off,
                Smaa { preset: SmaaPreset::High },
                ScreenSpaceAmbientOcclusion {
                    quality_level: ScreenSpaceAmbientOcclusionQualityLevel::Low,
                    radius: SSAO_RADIUS,
                    constant_object_thickness: 0.25,
                },
            ));
        }
    }
}

#[derive(Resource)]
struct PostPipelineData {
    layout: BindGroupLayoutDescriptor,
    fullscreen: FullscreenShader,
    shader: Handle<Shader>,
}

fn init_post_pipeline(mut commands: Commands, fullscreen: Res<FullscreenShader>, assets: Res<AssetServer>) {
    let entries = |multisampled: bool| {
        BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                uniform_buffer::<ViewUniform>(true),
                uniform_buffer::<PostUniform>(true),
                if multisampled { texture_depth_2d_multisampled() } else { texture_depth_2d() },
                texture_2d(TextureSampleType::Float { filterable: false }),
            ),
        )
    };
    commands.insert_resource(PostPipelineData {
        layout: BindGroupLayoutDescriptor::new("aa post layout", &entries(false)),
        fullscreen: fullscreen.clone(),
        shader: load_embedded_asset!(assets.as_ref(), "post.wesl"),
    });
    // the multisampled layout is built on demand in `prepare_post_pipelines`
    commands.insert_resource(PostLayouts {
        multisampled: BindGroupLayoutDescriptor::new("aa post layout (msaa)", &entries(true)),
    });
}

#[derive(Resource)]
struct PostLayouts {
    multisampled: BindGroupLayoutDescriptor,
}

#[derive(Clone)]
struct PostPipeline {
    layout: BindGroupLayoutDescriptor,
    fullscreen: FullscreenShader,
    shader: Handle<Shader>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct PostPipelineKey {
    target_format: TextureFormat,
    multisample: bool,
}

impl SpecializedRenderPipeline for PostPipeline {
    type Key = PostPipelineKey;

    fn specialize(&self, key: Self::Key) -> RenderPipelineDescriptor {
        let mut shader_defs = Vec::new();
        if key.multisample {
            shader_defs.push("MULTISAMPLED".into());
        }
        RenderPipelineDescriptor {
            label: Some("aa post pipeline".into()),
            layout: vec![self.layout.clone()],
            vertex: self.fullscreen.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: self.shader.clone(),
                shader_defs,
                entry_point: Some("fragment".into()),
                targets: vec![Some(ColorTargetState { format: key.target_format, blend: None, write_mask: ColorWrites::ALL })],
                constants: vec![],
            }),
            ..default()
        }
    }
}

#[derive(Component)]
struct PostPipelineId {
    id: CachedRenderPipelineId,
    multisample: bool,
}

#[allow(clippy::type_complexity)]
fn prepare_post_pipelines(
    mut commands: Commands,
    pipeline_cache: Res<PipelineCache>,
    mut pipelines: ResMut<SpecializedRenderPipelines<PostPipeline>>,
    data: Option<Res<PostPipelineData>>,
    layouts: Option<Res<PostLayouts>>,
    views: Query<(Entity, &ExtractedView, &Msaa), (With<ExtractedCamera>, With<PostUniform>)>,
) {
    let (Some(data), Some(layouts)) = (data, layouts) else { return };
    for (entity, view, msaa) in &views {
        let multisample = *msaa != Msaa::Off;
        let pipeline = PostPipeline {
            layout: if multisample { layouts.multisampled.clone() } else { data.layout.clone() },
            fullscreen: data.fullscreen.clone(),
            shader: data.shader.clone(),
        };
        let id = pipelines.specialize(&pipeline_cache, &pipeline, PostPipelineKey { target_format: view.target_format, multisample });
        commands.entity(entity).insert(PostPipelineId { id, multisample });
    }
}

fn post_pass(
    view: ViewQuery<(&ViewUniformOffset, &ViewTarget, &ViewDepthStencilTexture, &PostPipelineId, &DynamicUniformIndex<PostUniform>)>,
    pipeline_cache: Res<PipelineCache>,
    view_uniforms: Res<ViewUniforms>,
    post_uniforms: Res<ComponentUniforms<PostUniform>>,
    data: Option<Res<PostPipelineData>>,
    layouts: Option<Res<PostLayouts>>,
    mut ctx: RenderContext,
) {
    let (view_offset, target, depth, pipeline_id, post_index) = view.into_inner();
    let (Some(data), Some(layouts)) = (data, layouts) else { return };
    let Some(depth_view) = depth.attachment.depth_stencil_views().depth_only_view() else { return };
    let (Some(pipeline), Some(view_binding), Some(post_binding)) =
        (pipeline_cache.get_render_pipeline(pipeline_id.id), view_uniforms.uniforms.binding(), post_uniforms.binding())
    else {
        return;
    };
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let time_span = diagnostics.time_span(ctx.command_encoder(), "aa_post");
    let write = target.post_process_write();
    let layout = if pipeline_id.multisample { &layouts.multisampled } else { &data.layout };
    let bind_group = ctx.render_device().create_bind_group(
        Some("aa post bind group"),
        &pipeline_cache.get_bind_group_layout(layout),
        &BindGroupEntries::sequential((view_binding, post_binding, depth_view, write.source)),
    );
    let descriptor = RenderPassDescriptor {
        label: Some("aa post pass"),
        color_attachments: &[Some(RenderPassColorAttachment {
            view: write.destination,
            depth_slice: None,
            resolve_target: None,
            ops: Operations { load: LoadOp::Clear(default()), store: StoreOp::Store },
        })],
        ..default()
    };
    {
        let mut pass = ctx.command_encoder().begin_render_pass(&descriptor);
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &bind_group, &[view_offset.offset, post_index.index()]);
        pass.draw(0..3, 0..1);
    }
    time_span.end(ctx.command_encoder());
}

/// Helper for tests / examples: a neutral uniform with the sun direction in Bevy space.
pub fn neutral(sun: Vec3) -> PostUniform {
    PostUniform { sun_dir: sun.normalize_or(Vec3::Y).extend(8.0), ..default() }
}
