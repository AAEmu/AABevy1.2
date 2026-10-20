//! Camera post processing: an HDR camera, atmosphere (height) fog, the engine's bloom and film curve.
//!
//! * [`Post`] on a `Camera3d` turns the camera into an HDR one with a depth prepass, our own bloom and our own tone
//!   mapping pass. The world's focus camera gets it automatically ([`crate::WorldConfig::post`]); an application that
//!   spawns other cameras can add `Post::default()` itself.
//! * [`PostUniform`] carries the time of day parameters of the frame (fog, film curve, grading, bloom); `sky.rs`
//!   fills it each frame.
//! * The bloom (`post_bloom.rs`) is the engine's: a half resolution bright pass of the fogged scene (`v = c x level /
//!   key - threshold`, `v / (offset + v)`, so it saturates), a blur chain, and the sum added to the exposed scene before the
//!   grading.
//! * The pass (`post.wesl`) runs before Bevy's tone mapping (which is switched off): it reads the HDR colour and the main
//!   pass depth, applies the height fog and the bloom and maps the result with the `Filmic` curve of `postprocess.cfx`
//!   (`Film curve *`, `Saturation`, `Contrast`, `Levels` of the zone's time of day), writing display linear colour.

use bevy::asset::{embedded_asset, load_embedded_asset};
use bevy::camera::{Camera3d, Camera3dDepthTextureUsage, Hdr};
use bevy::anti_alias::smaa::{Smaa, SmaaPreset};
use bevy::core_pipeline::prepass::DepthPrepass;
use bevy::core_pipeline::schedule::Core3d;
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping, tonemapping};
use bevy::core_pipeline::{Core3dSystems, FullscreenShader};
use bevy::math::{Vec3, Vec4};
use bevy::pbr::{ScreenSpaceAmbientOcclusion, ScreenSpaceAmbientOcclusionQualityLevel};
use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::diagnostic::RecordDiagnostics;
use bevy::render::extract_component::{
    ComponentUniforms, DynamicUniformIndex, ExtractComponent, ExtractComponentPlugin, UniformComponentPlugin,
};
use bevy::render::render_resource::binding_types::{
    sampler, texture_2d, texture_depth_2d, texture_depth_2d_multisampled, uniform_buffer,
};
use bevy::render::render_resource::{
    BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries, BlendComponent, BlendFactor, BlendOperation, BlendState,
    CachedRenderPipelineId, ColorTargetState, ColorWrites, FilterMode, FragmentState, LoadOp, Operations, PipelineCache,
    RenderPassColorAttachment, RenderPassDescriptor, RenderPipelineDescriptor, Sampler, SamplerBindingType, SamplerDescriptor,
    ShaderStages, ShaderType, SpecializedRenderPipeline, SpecializedRenderPipelines, StoreOp, TextureFormat, TextureSampleType,
    TextureUsages,
};
use bevy::render::renderer::{RenderContext, RenderDevice, ViewQuery};
use bevy::render::view::{ExtractedView, Msaa, ViewDepthStencilTexture, ViewTarget, ViewUniform, ViewUniformOffset, ViewUniforms};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::Shader;

use crate::post_bloom::{self, BLOOM_FORMAT, BloomTextures};

/// Sample radius of the ambient occlusion in metres: deeper shade under bushes needs more than Bevy's 0.73.
const SSAO_RADIUS: f32 = 1.2;

/// Marks a camera for the world's post processing (see the module docs). Adding it sets up the camera.
#[derive(Component, Clone, Debug)]
pub struct Post {
    /// Gain of the bloom on top of the engine's (`AtmosphereTuning::bloom_gain`): 1 = as the engine, 0 turns it off.
    pub bloom: f32,
    /// Apply the atmosphere fog in the post pass (the camera's `DistanceFog` is removed).
    pub fog: bool,
    /// Exposure compensation in stops on top of the time of day's.
    pub exposure_ev: f32,
    /// Screen space ambient occlusion (Bevy's GTAO: needs the normal prepass and switches MSAA off for SMAA), about
    /// 8–10% of frame time. On by default since the hair material is prepass-safe (aa-cry `9751975`).
    pub ssao: bool,
}

impl Default for Post {
    fn default() -> Self {
        Post { bloom: 1.0, fog: true, exposure_ev: 0.0, ssao: true }
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
    /// Under water: x water level (world Y, metres), y extinction per metre, z in-scattering gain, w 1 when the camera is below the surface.
    pub under: Vec4,
    /// Under water: rgb colour of the deep water at the surface (pre-exposed), a: unused.
    pub under_color: Vec4,
    /// Bloom: x brightness scale (`r_HDRBrightLevel / 0.18`), y threshold, z offset.
    pub bloom: Vec4,
    /// Bloom: rgb strength per channel added to the exposed scene (the world's bloom colour x multiplier x gain; 0 = off).
    pub bloom_color: Vec4,
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
            under: Vec4::ZERO,
            under_color: Vec4::ZERO,
            bloom: Vec4::new(1.25 / 0.18, 6.0, 5.0, 0.0),
            bloom_color: Vec4::ZERO,
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
        // `Levels: min input` is in 0..255 like the engine's AdjustLevels ((c * 255 - min) / (max - min)).
        (film, Vec4::new(saturation, contrast, (sample.levels_min / 255.0).clamp(0.0, 0.5), exposure), balance.extend(0.0))
    }
}

pub(crate) fn build(app: &mut App) {
    embedded_asset!(app, "post.wesl");
    embedded_asset!(app, "post_bloom.wesl");
    app.add_plugins((ExtractComponentPlugin::<PostUniform>::default(), UniformComponentPlugin::<PostUniform>::default()))
        .add_systems(Update, setup_post_cameras);
    let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
    render_app
        .init_resource::<SpecializedRenderPipelines<PostPipeline>>()
        .add_systems(RenderStartup, init_post_pipeline)
        .add_systems(
            Render,
            (
                prepare_post_pipelines.in_set(RenderSystems::Prepare),
                post_bloom::prepare_bloom_textures.in_set(RenderSystems::PrepareResources),
            ),
        )
        .add_systems(
            Core3d,
            (post_bloom::bloom_passes.before(post_pass), post_pass.before(tonemapping)).in_set(Core3dSystems::PostProcess),
        );
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
        ));
        // The water refracts the frame's opaque colour (`reads_view_transmission_texture`); aa-client's in-game camera
        // brings the same setting, the examples' cameras did not (their water showed its body colour instead).
        commands.entity(entity).insert_if_new(bevy::pbr::ScreenSpaceTransmission {
            steps: 1,
            quality: bevy::pbr::ScreenSpaceTransmissionQuality::Low,
        });
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

/// The bind group layouts and shaders of the passes (built once at render startup).
#[derive(Resource)]
pub(crate) struct PostPipelineData {
    /// view, post uniform, depth, scene colour (the bright pass), and the same plus the bloom and its sampler (the final pass);
    /// each for a single sampled and for a multisampled depth buffer
    pub(crate) bright: [BindGroupLayoutDescriptor; 2],
    pub(crate) last: [BindGroupLayoutDescriptor; 2],
    /// The blur chain: a texture and a sampler.
    pub(crate) chain: BindGroupLayoutDescriptor,
    pub(crate) sampler: Sampler,
    fullscreen: FullscreenShader,
    shader: Handle<Shader>,
    bloom_shader: Handle<Shader>,
}

fn init_post_pipeline(
    mut commands: Commands,
    fullscreen: Res<FullscreenShader>,
    assets: Res<AssetServer>,
    render_device: Res<RenderDevice>,
) {
    let depth_and_color = |multisampled: bool| {
        (
            uniform_buffer::<ViewUniform>(true),
            uniform_buffer::<PostUniform>(true),
            if multisampled { texture_depth_2d_multisampled() } else { texture_depth_2d() },
            texture_2d(TextureSampleType::Float { filterable: false }),
        )
    };
    let bright = |multisampled: bool| {
        BindGroupLayoutDescriptor::new(
            if multisampled { "aa bright layout (msaa)" } else { "aa bright layout" },
            &BindGroupLayoutEntries::sequential(ShaderStages::FRAGMENT, depth_and_color(multisampled)),
        )
    };
    let last = |multisampled: bool| {
        let (view, post, depth, color) = depth_and_color(multisampled);
        BindGroupLayoutDescriptor::new(
            if multisampled { "aa post layout (msaa)" } else { "aa post layout" },
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (
                    view,
                    post,
                    depth,
                    color,
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
                ),
            ),
        )
    };
    commands.insert_resource(PostPipelineData {
        bright: [bright(false), bright(true)],
        last: [last(false), last(true)],
        chain: BindGroupLayoutDescriptor::new(
            "aa bloom chain layout",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (texture_2d(TextureSampleType::Float { filterable: true }), sampler(SamplerBindingType::Filtering)),
            ),
        ),
        sampler: render_device.create_sampler(&SamplerDescriptor {
            min_filter: FilterMode::Linear,
            mag_filter: FilterMode::Linear,
            ..Default::default()
        }),
        fullscreen: fullscreen.clone(),
        shader: load_embedded_asset!(assets.as_ref(), "post.wesl"),
        bloom_shader: load_embedded_asset!(assets.as_ref(), "post_bloom.wesl"),
    });
}

/// The four kinds of pass of the post processing.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum PassKind {
    /// Fogged scene to half resolution bright pass (`post.wesl`, `bright_fragment`).
    Bright,
    /// Blur chain, down and up (`post_bloom.wesl`).
    Down,
    Up,
    /// Fog, bloom, grade and film curve into the view (`post.wesl`, `fragment`).
    Final,
}

#[derive(Clone)]
struct PostPipeline {
    layouts: [[BindGroupLayoutDescriptor; 2]; 2],
    chain: BindGroupLayoutDescriptor,
    fullscreen: FullscreenShader,
    shader: Handle<Shader>,
    bloom_shader: Handle<Shader>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct PostPipelineKey {
    kind: PassKind,
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
        let ms = key.multisample as usize;
        let (label, layout, shader, entry, format, blend) = match key.kind {
            PassKind::Bright => ("aa bright pipeline", self.layouts[0][ms].clone(), &self.shader, "bright_fragment", BLOOM_FORMAT, None),
            PassKind::Final => ("aa post pipeline", self.layouts[1][ms].clone(), &self.shader, "fragment", key.target_format, None),
            PassKind::Down => ("aa bloom down pipeline", self.chain.clone(), &self.bloom_shader, "downsample", BLOOM_FORMAT, None),
            PassKind::Up => (
                "aa bloom up pipeline",
                self.chain.clone(),
                &self.bloom_shader,
                "upsample",
                BLOOM_FORMAT,
                Some(BlendState {
                    color: BlendComponent { src_factor: BlendFactor::One, dst_factor: BlendFactor::One, operation: BlendOperation::Add },
                    alpha: BlendComponent { src_factor: BlendFactor::Zero, dst_factor: BlendFactor::One, operation: BlendOperation::Add },
                }),
            ),
        };
        RenderPipelineDescriptor {
            label: Some(label.into()),
            layout: vec![layout],
            vertex: self.fullscreen.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: shader.clone(),
                shader_defs: if matches!(key.kind, PassKind::Down | PassKind::Up) { Vec::new() } else { shader_defs },
                entry_point: Some(entry.into()),
                targets: vec![Some(ColorTargetState { format, blend, write_mask: ColorWrites::ALL })],
                constants: vec![],
            }),
            ..default()
        }
    }
}

/// The pipelines of a view's passes.
#[derive(Component)]
pub(crate) struct PostPipelineId {
    last: CachedRenderPipelineId,
    pub(crate) bright: CachedRenderPipelineId,
    pub(crate) down: CachedRenderPipelineId,
    pub(crate) up: CachedRenderPipelineId,
    pub(crate) multisample: bool,
}

#[allow(clippy::type_complexity)]
fn prepare_post_pipelines(
    mut commands: Commands,
    pipeline_cache: Res<PipelineCache>,
    mut pipelines: ResMut<SpecializedRenderPipelines<PostPipeline>>,
    data: Option<Res<PostPipelineData>>,
    views: Query<(Entity, &ExtractedView, &Msaa), (With<ExtractedCamera>, With<PostUniform>)>,
) {
    let Some(data) = data else { return };
    let pipeline = PostPipeline {
        layouts: [data.bright.clone(), data.last.clone()],
        chain: data.chain.clone(),
        fullscreen: data.fullscreen.clone(),
        shader: data.shader.clone(),
        bloom_shader: data.bloom_shader.clone(),
    };
    for (entity, view, msaa) in &views {
        let multisample = *msaa != Msaa::Off;
        let mut specialize = |kind: PassKind| {
            pipelines.specialize(&pipeline_cache, &pipeline, PostPipelineKey { kind, target_format: view.target_format, multisample })
        };
        let ids = PostPipelineId {
            last: specialize(PassKind::Final),
            bright: specialize(PassKind::Bright),
            down: specialize(PassKind::Down),
            up: specialize(PassKind::Up),
            multisample,
        };
        commands.entity(entity).insert(ids);
    }
}

fn post_pass(
    view: ViewQuery<(
        &ViewUniformOffset,
        &ViewTarget,
        &ViewDepthStencilTexture,
        &PostPipelineId,
        &DynamicUniformIndex<PostUniform>,
        Option<&BloomTextures>,
    )>,
    pipeline_cache: Res<PipelineCache>,
    view_uniforms: Res<ViewUniforms>,
    post_uniforms: Res<ComponentUniforms<PostUniform>>,
    data: Option<Res<PostPipelineData>>,
    mut ctx: RenderContext,
) {
    let (view_offset, target, depth, pipeline_id, post_index, bloom) = view.into_inner();
    let Some(data) = data else { return };
    let Some(depth_view) = depth.attachment.depth_stencil_views().depth_only_view() else { return };
    let (Some(pipeline), Some(view_binding), Some(post_binding)) =
        (pipeline_cache.get_render_pipeline(pipeline_id.last), view_uniforms.uniforms.binding(), post_uniforms.binding())
    else {
        return;
    };
    let Some(bloom) = bloom else { return };
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let time_span = diagnostics.time_span(ctx.command_encoder(), "aa_post");
    let write = target.post_process_write();
    let layout = &data.last[pipeline_id.multisample as usize];
    let bloom_view = bloom.view(0);
    let bind_group = ctx.render_device().create_bind_group(
        Some("aa post bind group"),
        &pipeline_cache.get_bind_group_layout(layout),
        &BindGroupEntries::sequential((view_binding, post_binding, depth_view, write.source, &bloom_view, &data.sampler)),
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
