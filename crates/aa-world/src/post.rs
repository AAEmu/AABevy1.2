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
//! * Sun shafts (`post_shafts.rs`) and eye adaptation (`post_adapt.rs`) add a few small passes before the final one; the
//!   zone's sun shafts / rays parameters and [`crate::sky::AtmosphereTuning`] (`shafts`, `adapt`) drive them.
//! * The depth of field (`post_dof.rs`, rules in [`crate::depth_of_field`]): a blurred half resolution copy of the fogged scene,
//!   which the final pass blends into each pixel by the pixel's own depth, as 1.2's composite does; the time of day's at rest
//!   ([`PostUniform::dof_rest`]), the application's (a quest talk) over it ([`PostUniform::dof`]).
//! * The pass (`post.wesl`) runs before Bevy's tone mapping (which is switched off): it reads the HDR colour and the main
//!   pass depth, applies the height fog and the bloom and maps the result with the `Filmic` curve of `postprocess.cfx`
//!   (`Film curve *`, `Saturation`, `Contrast`, `Levels` of the zone's time of day), writing display linear colour.

use bevy::asset::{embedded_asset, load_embedded_asset};
use bevy::camera::{Camera3d, Camera3dDepthTextureUsage, Hdr, visibility::RenderLayers};
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
    BindGroup, BindGroupEntries, BindGroupLayout, BindGroupLayoutDescriptor, BindingResource, BindGroupLayoutEntries, BlendComponent, BlendFactor, BlendOperation, BlendState,
    CachedRenderPipelineId, ColorTargetState, ColorWrites, FilterMode, FragmentState, LoadOp, Operations, PipelineCache,
    RenderPassColorAttachment, RenderPassDescriptor, RenderPipelineDescriptor, Sampler, SamplerBindingType, SamplerDescriptor,
    ShaderStages, ShaderType, SpecializedRenderPipeline, SpecializedRenderPipelines, StoreOp, TextureFormat, TextureSampleType,
    TextureUsages, TextureView,
};
use bevy::render::renderer::{RenderContext, RenderDevice, ViewQuery};
use bevy::render::view::{ExtractedView, Msaa, ViewDepthStencilTexture, ViewTarget, ViewUniform, ViewUniformOffset, ViewUniforms};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::Shader;

use crate::post_adapt::{self, ADAPT_FORMAT, AdaptTextures};
use crate::post_bloom::{self, BLOOM_FORMAT, BloomTextures};
use crate::post_dof::{self, DOF_FORMAT, DofTextures};
use crate::post_shafts::{self, ShaftTextures, SHAFT_FORMAT};

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
    /// 8–10% of frame time. On by default since the hair material is prepass-safe (aa-cry `9751975`); off in the
    /// browser, where each extra pipeline variant (the normal prepass doubles most of the prepass ones) costs a
    /// shader compile of ~0.4 s in the GPU process that stalls the page.
    pub ssao: bool,
    /// The depth of field (1.2's `r_DepthOfField` 2, the Options' depth of field box): the time of day's blur at rest and the
    /// application's ([`PostUniform::dof`]). Off, neither is drawn and no pass runs. Off by default in the browser, like
    /// [`Post::ssao`]: its three passes are three more pipelines to compile there.
    pub dof: bool,
}

impl Default for Post {
    fn default() -> Self {
        Post { bloom: 1.0, fog: true, exposure_ev: 0.0, ssao: !cfg!(target_arch = "wasm32"), dof: !cfg!(target_arch = "wasm32") }
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
    /// The engine's `vfRampParams` ([`PostUniform::fog_ramp_from`]): x `1 / (end - start)`, y `-start / (end - start)`, z influence,
    /// w `1 - influence`.
    pub fog_ramp: Vec4,
    pub sun_dir: Vec4,
    pub sun_glow: Vec4,
    pub film: Vec4,
    pub grade: Vec4,
    pub balance: Vec4,
    /// Under water: x water level (world Y, metres), y extinction per metre, z in-scattering gain, w 1 when the camera is below the surface.
    pub under: Vec4,
    /// Under water: rgb colour of the deep water at the surface (pre-exposed).
    /// Alpha is an internal shader selector: 1 volume Outof, -1 qualified sea,
    /// 0 the existing ocean fallback. The sea retains calibrated scatter/colour.
    pub under_color: Vec4,
    /// Bloom: x brightness scale (`r_HDRBrightLevel / 0.18`), y threshold, z offset.
    pub bloom: Vec4,
    /// Bloom: rgb strength per channel added to the exposed scene (the world's bloom colour x multiplier x gain; 0 = off).
    pub bloom_color: Vec4,
    /// Radial motion blur (the application's, e.g. a sprint): x how far the samples reach towards the centre (a share of the
    /// way; 0 = off, the pass then takes no extra samples), yz the centre in uv (kept sharp), w unused. Nothing here writes it.
    pub motion: Vec4,
    /// HDR sun shafts: x RaysAmount times the tuning gain (0 = off), y finite-projection
    /// bridge scale far/(far-near), z RaysAttenuation clamped to 0..10, w world view limit for the depth bridge (metres).
    pub shafts: Vec4,
    /// Sun shafts: L2-normalized native sun RGB (default RaysSunColInfluence = 1).
    pub shafts_color: Vec4,
    /// Eye adaptation: x blend per frame towards a brighter scene, y strength (0 = off), z darkest and w brightest exposure in stops.
    pub adapt: Vec4,
    /// Eye adaptation: x log2 luminance of the calibrated look, y luminance cap of the measurement, z blend per frame towards a darker scene.
    pub adapt2: Vec4,
    /// Colour grading, `AdjustLevels`: x 1 / gamma, y max input (0..1); the min input is `grade.z`.
    pub levels: Vec4,
    /// Colour grading, `ApplyPhotoFilter`: rgb filter colour (display values), w density.
    pub photo: Vec4,
    /// Colour grading, `AdjustColor`: x saturation, y contrast, z brightness, w 1 = the colour grading is on.
    pub color: Vec4,
    /// Depth of field the application asks for (a quest talk's, [`crate::depth_of_field::shader_params`]), which wins over
    /// [`PostUniform::dof_rest`] while its x is above 0: x the blur of the half resolution copy, a gaussian's sigma in its texels (0 = off),
    /// y the focus distance (m), z half the focus range, w the blur amount. Nothing in this crate writes it.
    pub dof: Vec4,
    /// The time of day's depth of field at rest (written by `sky.rs` each frame): the same four numbers; x 0 = none (a zone with no
    /// blur, or [`Post::dof`] off).
    pub dof_rest: Vec4,
}

impl Default for PostUniform {
    fn default() -> Self {
        PostUniform {
            fog_color: Vec4::new(0.5, 0.5, 0.5, 0.0),
            fog_top: Vec4::new(0.5, 0.5, 0.5, 0.0),
            fog_layer: Vec4::new(100.0, 1500.0, 0.9, 0.03),
            fog_params: Vec4::new(0.0, 0.0008, 1.0, 9000.0),
            // influence 0: no ramp, the fog starts at the eye
            fog_ramp: Vec4::new(0.0, 0.0, 0.0, 1.0),
            sun_dir: Vec4::new(0.0, 1.0, 0.0, 8.0),
            sun_glow: Vec4::ZERO,
            film: Vec4::new(0.9, 1.0, 0.8, 1.8),
            grade: Vec4::new(1.0, 1.0, 0.0, 1.0),
            balance: Vec4::new(1.0, 1.0, 1.0, 0.0),
            under: Vec4::ZERO,
            under_color: Vec4::ZERO,
            bloom: Vec4::new(1.25 / 0.18, 6.0, 5.0, 0.0),
            bloom_color: Vec4::ZERO,
            motion: Vec4::ZERO,
            shafts: Vec4::ZERO,
            shafts_color: Vec4::ONE,
            adapt: Vec4::new(1.0, 0.0, -1.0, 1.0),
            adapt2: Vec4::new(-2.0, 4.0, 1.0, 0.0),
            levels: Vec4::new(1.0, 1.0, 0.0, 0.0),
            photo: Vec4::ZERO,
            color: Vec4::new(1.0, 1.0, 1.0, 0.0),
            dof: Vec4::ZERO,
            dof_rest: Vec4::ZERO,
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

    /// The fog ramp of a time of day sample's `(Ramp start, Ramp end, Ramp influence)`, as the engine's `vfRampParams`
    /// (`volumetricfog.cfi`: `r = saturate(dist x + y)`, `r = r (2 - r) z + w`, the fog amount times `r`). The fog eases in from the
    /// start to the end; with influence 1 there is none before the start, with 0 the ramp is off. The clamps keep the division finite.
    pub fn fog_ramp_from(ramp: Vec3) -> Vec4 {
        let start = ramp.x.max(0.0);
        let end = ramp.y.max(start + 0.1);
        let influence = ramp.z.clamp(0.0, 1.0);
        let inv = 1.0 / (end - start);
        Vec4::new(inv, -start * inv, influence, 1.0 - influence)
    }
}

impl PostUniform {
    /// The colour grading of a time of day sample (`levels`, `photo`, `color`): the engine's `ColorGrading` effect on the
    /// tone mapped image (`posteffectsgame.cfx`). `Levels: min/max output` are not used (the zones author 255 there at
    /// times, which would paint the screen white; the effect's own default is 0 / 255).
    pub fn grading_from(sample: &crate::env::EnvSample) -> (Vec4, Vec4, Vec4) {
        let max_in = if sample.levels_max > 1.0 { sample.levels_max } else { 255.0 };
        let gamma = if sample.levels_gamma > 0.05 { sample.levels_gamma } else { 1.0 };
        let levels = Vec4::new(1.0 / gamma, (max_in / 255.0).clamp(0.1, 1.0), 0.0, 0.0);
        let [r, g, b] = sample.photofilter.truncate().to_array();
        let photo = Vec4::new(r, g, b, sample.photofilter.w.clamp(0.0, 1.0));
        let c = sample.color_grade;
        (levels, photo, Vec4::new(c.x.max(0.0), c.y.max(0.0), c.z.max(0.0), 1.0))
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
                (
                    post_bloom::prepare_bloom_textures,
                    post_dof::prepare_dof_textures,
                    post_shafts::prepare_shaft_textures,
                    post_adapt::prepare_adapt_textures,
                )
                    .in_set(RenderSystems::PrepareResources),
            ),
        )
        .add_systems(
            Core3d,
            (
                post_adapt::adapt_passes.before(post_bloom::bloom_passes),
                post_shafts::shaft_passes.before(post_pass),
                post_dof::dof_passes.before(post_pass),
                post_bloom::bloom_passes.before(post_pass),
                post_pass.before(tonemapping),
            )
                .in_set(Core3dSystems::PostProcess),
        );
}

/// Gives every camera with a [`Post`] the HDR / depth / bloom setup (once), and the world's own camera (the focus
/// entity, else the first layer-zero `Camera3d`) a default `Post`.
#[allow(clippy::type_complexity)]
fn setup_post_cameras(
    mut commands: Commands,
    cfg: Res<crate::stream::WorldConfig>,
    tuning: Res<crate::sky::AtmosphereTuning>,
    focus: Query<Entity, With<crate::stream::WorldFocus>>,
    mut cameras: Query<(Entity, &mut Camera3d, Option<&Post>, Option<&PostReady>, Option<&RenderLayers>)>,
) {
    if !cfg.environment || !cfg.post {
        return;
    }
    let focus = focus.iter().next();
    let main = cameras.iter().find(|c| Some(c.0) == focus)
        .or_else(|| cameras.iter().find(|c| c.4.is_none_or(|layers| layers.intersects(&RenderLayers::default()))))
        .map(|c| c.0);
    for (entity, mut cam, post, ready, _) in &mut cameras {
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
        } else if cfg!(target_arch = "wasm32") {
            // the browser keeps the same edges without the occlusion (and a cheaper frame than 4x MSAA)
            commands.entity(entity).insert((Msaa::Off, Smaa { preset: SmaaPreset::High }));
        }
    }
}

/// The bind group layouts and shaders of the passes (built once at render startup).
#[derive(Resource)]
pub(crate) struct PostPipelineData {
    /// view, post uniform, depth, scene colour, eye adaptation state (the bright pass and the measurement), and the same plus the
    /// bloom, its sampler, the sun shafts and the depth of field's copy (the final pass and the passes that read a texture); each for a single sampled and
    /// for a multisampled depth buffer
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
        let (view, post, depth, color, adapt) = depth_and_color(multisampled);
        BindGroupLayoutDescriptor::new(
            if multisampled { "aa post layout (msaa)" } else { "aa post layout" },
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (
                    view,
                    post,
                    depth,
                    color,
                    adapt,
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    texture_2d(TextureSampleType::Float { filterable: true }),
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
    /// Sun shafts: the depth mask and three native blurs (`post.wesl`).
    ShaftMask,
    ShaftBlur1,
    ShaftBlur2,
    ShaftBlur3,
    /// Depth of field: the half resolution copy and its two blur passes (`post.wesl`, `post_dof.rs`).
    DofDown,
    DofBlurH,
    DofBlurV,
    /// Eye adaptation: the luminance cells and the state update (`post.wesl`).
    Lum,
    Adapt,
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
            PassKind::ShaftMask => ("aa shafts mask pipeline", self.layouts[0][ms].clone(), &self.shader, "shafts_mask_fragment", SHAFT_FORMAT, None),
            PassKind::ShaftBlur1 => ("aa shafts blur 1 pipeline", self.layouts[1][ms].clone(), &self.shader, "shafts_blur1_fragment", SHAFT_FORMAT, None),
            PassKind::ShaftBlur2 => ("aa shafts blur 2 pipeline", self.layouts[1][ms].clone(), &self.shader, "shafts_blur2_fragment", SHAFT_FORMAT, None),
            PassKind::ShaftBlur3 => ("aa shafts blur 3 pipeline", self.layouts[1][ms].clone(), &self.shader, "shafts_blur3_fragment", SHAFT_FORMAT, None),
            PassKind::DofDown => ("aa dof down pipeline", self.layouts[0][ms].clone(), &self.shader, "dof_down_fragment", DOF_FORMAT, None),
            PassKind::DofBlurH => ("aa dof blur h pipeline", self.layouts[1][ms].clone(), &self.shader, "dof_blur_h_fragment", DOF_FORMAT, None),
            PassKind::DofBlurV => ("aa dof blur v pipeline", self.layouts[1][ms].clone(), &self.shader, "dof_blur_v_fragment", DOF_FORMAT, None),
            PassKind::Lum => ("aa lum pipeline", self.layouts[0][ms].clone(), &self.shader, "lum_fragment", ADAPT_FORMAT, None),
            PassKind::Adapt => ("aa adapt pipeline", self.layouts[1][ms].clone(), &self.shader, "adapt_fragment", ADAPT_FORMAT, None),
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
    pub(crate) shaft_mask: CachedRenderPipelineId,
    pub(crate) shaft_blur: [CachedRenderPipelineId; 3],
    pub(crate) lum: CachedRenderPipelineId,
    pub(crate) adapt: CachedRenderPipelineId,
    pub(crate) dof_down: CachedRenderPipelineId,
    pub(crate) dof_blur: [CachedRenderPipelineId; 2],
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
            shaft_mask: specialize(PassKind::ShaftMask),
            shaft_blur: [specialize(PassKind::ShaftBlur1), specialize(PassKind::ShaftBlur2), specialize(PassKind::ShaftBlur3)],
            lum: specialize(PassKind::Lum),
            adapt: specialize(PassKind::Adapt),
            dof_down: specialize(PassKind::DofDown),
            dof_blur: [specialize(PassKind::DofBlurH), specialize(PassKind::DofBlurV)],
            multisample,
        };
        commands.entity(entity).insert(ids);
    }
}

#[allow(clippy::type_complexity)]
fn post_pass(
    view: ViewQuery<(
        &ViewUniformOffset,
        &ViewTarget,
        &ViewDepthStencilTexture,
        &PostPipelineId,
        &DynamicUniformIndex<PostUniform>,
        Option<&BloomTextures>,
        Option<&ShaftTextures>,
        Option<&AdaptTextures>,
        Option<&DofTextures>,
    )>,
    pipeline_cache: Res<PipelineCache>,
    view_uniforms: Res<ViewUniforms>,
    post_uniforms: Res<ComponentUniforms<PostUniform>>,
    data: Option<Res<PostPipelineData>>,
    mut ctx: RenderContext,
) {
    let (view_offset, target, depth, pipeline_id, post_index, bloom, shafts, adapt, dof) = view.into_inner();
    let Some(data) = data else { return };
    let Some(depth_view) = depth.attachment.depth_stencil_views().depth_only_view() else { return };
    let (Some(pipeline), Some(view_binding), Some(post_binding)) =
        (pipeline_cache.get_render_pipeline(pipeline_id.last), view_uniforms.uniforms.binding(), post_uniforms.binding())
    else {
        return;
    };
    let (Some(bloom), Some(shafts), Some(adapt), Some(dof)) = (bloom, shafts, adapt, dof) else { return };
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let time_span = diagnostics.time_span(ctx.command_encoder(), "aa_post");
    let write = target.post_process_write();
    let layout = &data.last[pipeline_id.multisample as usize];
    let bloom_view = bloom.view(0);
    let bind_group = last_group(
        ctx.render_device(),
        &pipeline_cache.get_bind_group_layout(layout),
        view_binding,
        post_binding,
        depth_view,
        write.source,
        &adapt.state_view(),
        &bloom_view,
        &data.sampler,
        &shafts.view(),
        &dof.view(),
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

/// The bind group of the `last` layout: the final pass, and the passes that read a texture of the chain.
#[allow(clippy::too_many_arguments)]
pub(crate) fn last_group(
    device: &RenderDevice,
    layout: &BindGroupLayout,
    view: BindingResource,
    post: BindingResource,
    depth: &TextureView,
    color: &TextureView,
    adapt: &TextureView,
    bloom: &TextureView,
    sampler: &Sampler,
    shafts: &TextureView,
    dof: &TextureView,
) -> BindGroup {
    device.create_bind_group(
        Some("aa post bind group"),
        layout,
        &BindGroupEntries::sequential((view, post, depth, color, adapt, bloom, sampler, shafts, dof)),
    )
}

/// Helper for tests / examples: a neutral uniform with the sun direction in Bevy space.
pub fn neutral(sun: Vec3) -> PostUniform {
    PostUniform { sun_dir: sun.normalize_or(Vec3::Y).extend(8.0), ..default() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::{Lighting, TimeOfDay};

    fn camera_app() -> App {
        let mut app = App::new();
        app.insert_resource(crate::WorldConfig::new("camera_fixture"))
            .insert_resource(crate::sky::AtmosphereTuning::default())
            .add_systems(Update, setup_post_cameras);
        app
    }

    #[test]
    fn modelview_post_selection_survives_camera_order_and_archetype_changes() {
        for widget_first in [true, false] {
            let mut app = camera_app();
            let spawn_widget = |app: &mut App| app.world_mut().spawn((Camera3d::default(), RenderLayers::from_layers(&[128]))).id();
            let first = widget_first.then(|| spawn_widget(&mut app));
            let camera = app.world_mut().spawn((
                Camera3d::default(), bevy::camera::RenderTarget::Image(Handle::<Image>::default().into()),
            )).id();
            let widget = first.unwrap_or_else(|| spawn_widget(&mut app));
            for layers in [None, Some(RenderLayers::default()), Some(RenderLayers::from_layers(&[0, 9])), None] {
                if let Some(layers) = layers {
                    app.world_mut().entity_mut(camera).insert(layers);
                    app.world_mut().entity_mut(widget).insert(Name::new("widget archetype transition"));
                } else {
                    app.world_mut().entity_mut(camera).remove::<RenderLayers>();
                    app.world_mut().entity_mut(widget).remove::<Name>();
                }
                app.world_mut().entity_mut(camera).remove::<(Post, PostReady, PostUniform)>();
                app.update();
                assert!(app.world().get::<PostReady>(camera).is_some());
                assert!(app.world().get::<PostUniform>(camera).is_some());
                assert!(app.world().get::<DepthPrepass>(camera).is_some());
                assert!(app.world().get::<Post>(widget).is_none());
                assert!(app.world().get::<PostUniform>(widget).is_none());
                assert!(app.world().get::<Hdr>(widget).is_none());
            }
        }
    }

    #[test]
    fn modelview_only_declines_automatic_post_but_preserves_explicit_opt_in_and_inactive_focus() {
        let mut app = camera_app();
        let widget = app.world_mut().spawn((Camera3d::default(), RenderLayers::from_layers(&[128]))).id();
        app.update();
        assert!(app.world().get::<Post>(widget).is_none());
        assert!(app.world().get::<PostReady>(widget).is_none());
        assert!(app.world().get::<bevy::pbr::ScreenSpaceTransmission>(widget).is_none());
        app.world_mut().entity_mut(widget).insert(Post { ssao: false, ..default() });
        app.update();
        assert!(app.world().get::<PostReady>(widget).is_some());
        assert!(app.world().get::<Hdr>(widget).is_some());
        assert!(app.world().get::<DepthPrepass>(widget).is_some());
        assert!(app.world().get::<bevy::pbr::ScreenSpaceTransmission>(widget).is_some());
        let ordinary = app.world_mut().spawn(Camera3d::default()).id();
        let anchor = app.world_mut().spawn((Camera3d::default(), Camera { is_active: false, ..default() }, crate::WorldFocus)).id();
        app.update();
        assert!(app.world().get::<PostReady>(anchor).is_some());
        assert!(app.world().get::<Post>(ordinary).is_none());
    }

    fn sample(vars: &str) -> crate::env::EnvSample {
        let xml = format!("<TimeOfDayList><TimeOfDay Time=\"12\">{vars}</TimeOfDay></TimeOfDayList>");
        TimeOfDay::parse(&xml).unwrap().sample(12.0, Lighting::default())
    }

    #[test]
    fn grading_is_neutral_without_data() {
        let (levels, photo, color) = PostUniform::grading_from(&sample(""));
        assert_eq!(levels, Vec4::new(1.0, 1.0, 0.0, 0.0));
        assert_eq!(photo.w, 0.0);
        assert_eq!(color, Vec4::new(1.0, 1.0, 1.0, 1.0));
    }

    #[test]
    fn grading_reads_levels_photo_filter_and_colour_matrix() {
        // zone 144 / 195 style levels, a zone with a photo filter, a desaturated one
        let s = sample(
            r#"<Variable Name="Levels: gamma" Value="1"><Spline Keys="0:1.05:0,1:1.05:0,"/></Variable>
               <Variable Name="Levels: max input" Value="255"><Spline Keys="0:225:0,1:225:0,"/></Variable>
               <Variable Name="Filters: photofilter density" Value="0"><Spline Keys="0:0.4:0,1:0.4:0,"/></Variable>
               <Variable Name="Filters: photofilter color" Color="1,1,1"><Spline Keys="0:(0.5:0.25:1):0,1:(0.5:0.25:1):0,"/></Variable>
               <Variable Name="Color: saturation" Value="1"><Spline Keys="0:0.6:0,1:0.6:0,"/></Variable>"#,
        );
        let (levels, photo, color) = PostUniform::grading_from(&s);
        assert!((levels.x - 1.0 / 1.05).abs() < 1e-5 && (levels.y - 225.0 / 255.0).abs() < 1e-5);
        assert!((photo - Vec4::new(0.5, 0.25, 1.0, 0.4)).abs().max_element() < 1e-5);
        assert_eq!((color.x, color.y, color.z, color.w), (0.6, 1.0, 1.0, 1.0));
    }

    /// `volumetricfog.cfi`'s ramp factor at a distance, from the uniform as the shader reads it.
    fn ramp_at(p: Vec4, dist: f32) -> f32 {
        let r = (dist * p.x + p.y).clamp(0.0, 1.0);
        r * (2.0 - r) * p.z + p.w
    }

    #[test]
    fn fog_ramp_eases_the_fog_in_between_start_and_end() {
        // Mirage Isle (arche_mall_world zone 260) at noon: start 10, end 500, influence 1
        let p = PostUniform::fog_ramp_from(Vec3::new(10.0, 500.0, 1.0));
        assert_eq!(ramp_at(p, 5.0), 0.0);
        assert!((ramp_at(p, 255.0) - 0.75).abs() < 1e-5);
        assert_eq!(ramp_at(p, 800.0), 1.0);
        // main world zone 179 style: influence 0.7 keeps 0.3 of the fog before the start
        let p = PostUniform::fog_ramp_from(Vec3::new(60.0, 800.0, 0.7));
        assert!((ramp_at(p, 30.0) - 0.3).abs() < 1e-5);
        // influence 0, or no data: no ramp at all; an end before the start stays finite
        assert_eq!(ramp_at(PostUniform::fog_ramp_from(Vec3::new(50.0, 100.0, 0.0)), 60.0), 1.0);
        assert_eq!(ramp_at(PostUniform::default().fog_ramp, 1.0), 1.0);
        assert!(PostUniform::fog_ramp_from(Vec3::new(100.0, 50.0, 1.0)).is_finite());
    }
}
