//! Eye adaptation of the post pass: the exposure follows the brightness of what the camera looks at.
//!
//! The engine (`r_HDRRendering 3`) measures the scene's luminance, adapts `adapted luminance` towards it over time and divides
//! the scene by it (`HDRBrightPassFilterPS` still shows the `level / adapted luminance` term). Here:
//!
//! 1. `lum_fragment` writes the mean log2 luminance of the exposed, fogged scene in 32 x 18 cells (576 pixels, 36 samples each),
//! 2. `adapt_fragment` averages them (centre weighted) into one state value, `log2(measured / calibrated)`, and eases the previous
//!    state towards it: fast when the scene got brighter, slower when it got darker (the eye dark adapts slowly). The result goes
//!    to a second 1 x 1 texture that is copied over the state,
//! 3. `exposure()` in `post.wesl` multiplies the time of day's exposure by `2^(-state x strength)` clamped to a few stops, in
//!    the bloom's bright pass and the final pass alike.
//!
//! State 0 is the calibrated look (the luminance the world's calibration views have, `AtmosphereTuning::adapt_ref`), so the
//! steady state of those views is unchanged; only a much darker (forest, interior, night) or brighter (sky, snow) view moves it.
//! The time of day has no adaptation range of its own (`HDR dynamic power factor` is the colours' HDR multiplier, already applied).
//! Cost: two tiny passes (576 + 1 pixels).

use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::diagnostic::RecordDiagnostics;
use bevy::render::extract_component::{ComponentUniforms, DynamicUniformIndex};
use bevy::render::render_resource::{
    BindGroupEntries, Extent3d, LoadOp, Operations, Origin3d, PipelineCache, RenderPassColorAttachment, RenderPassDescriptor, StoreOp,
    TexelCopyTextureInfo, TextureAspect, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages, TextureView,
    TextureViewDescriptor,
};
use bevy::render::renderer::{RenderContext, RenderDevice, ViewQuery};
use bevy::render::texture::{CachedTexture, TextureCache};
use bevy::render::view::{ViewDepthStencilTexture, ViewTarget, ViewUniformOffset, ViewUniforms};

use crate::post::{PostPipelineData, PostPipelineId, PostUniform};

/// Cells of the luminance measurement (`LUM_CELLS` in `post.wesl`).
const CELLS: Extent3d = Extent3d { width: 32, height: 18, depth_or_array_layers: 1 };
const ONE: Extent3d = Extent3d { width: 1, height: 1, depth_or_array_layers: 1 };
pub(crate) const ADAPT_FORMAT: TextureFormat = TextureFormat::R16Float;

/// The measurement and the adaptation state of a view. The state texture lives on between frames (zero = calibrated).
#[derive(Component)]
pub(crate) struct AdaptTextures {
    cells: CachedTexture,
    state: CachedTexture,
    next: CachedTexture,
}

impl AdaptTextures {
    /// The state, 1 x 1.
    pub(crate) fn state_view(&self) -> TextureView {
        self.state.texture.create_view(&TextureViewDescriptor::default())
    }
}

pub(crate) fn prepare_adapt_textures(
    mut commands: Commands,
    mut texture_cache: ResMut<TextureCache>,
    render_device: Res<RenderDevice>,
    views: Query<Entity, (With<ExtractedCamera>, With<PostUniform>)>,
) {
    for entity in &views {
        let mut make = |label: &'static str, size: Extent3d, usage: TextureUsages| {
            texture_cache.get(
                &render_device,
                TextureDescriptor {
                    label: Some(label),
                    size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: TextureDimension::D2,
                    format: ADAPT_FORMAT,
                    usage: usage | TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
            )
        };
        // (the descriptors differ in the label: the cache hands the state texture back every frame)
        let cells = make("aa adapt cells", CELLS, TextureUsages::RENDER_ATTACHMENT);
        let state = make("aa adapt state", ONE, TextureUsages::COPY_DST);
        let next = make("aa adapt next", ONE, TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC);
        commands.entity(entity).insert(AdaptTextures { cells, state, next });
    }
}

fn copy_info(texture: &CachedTexture) -> TexelCopyTextureInfo<'_> {
    TexelCopyTextureInfo { texture: &texture.texture, mip_level: 0, origin: Origin3d::ZERO, aspect: TextureAspect::All }
}

/// Measure, adapt, store (see the module docs). Does nothing while the strength is 0.
#[allow(clippy::type_complexity)]
pub(crate) fn adapt_passes(
    view: ViewQuery<(
        &ViewUniformOffset,
        &ViewTarget,
        &ViewDepthStencilTexture,
        &PostPipelineId,
        &DynamicUniformIndex<PostUniform>,
        &PostUniform,
        &AdaptTextures,
    )>,
    pipeline_cache: Res<PipelineCache>,
    view_uniforms: Res<ViewUniforms>,
    post_uniforms: Res<ComponentUniforms<PostUniform>>,
    data: Option<Res<PostPipelineData>>,
    mut ctx: RenderContext,
) {
    let (view_offset, target, depth, pipelines, post_index, post, adapt) = view.into_inner();
    if post.adapt.y <= 0.0 {
        return;
    }
    let Some(data) = data else { return };
    let Some(depth_view) = depth.attachment.depth_stencil_views().depth_only_view() else { return };
    let (Some(measure), Some(update), Some(view_binding), Some(post_binding)) = (
        pipeline_cache.get_render_pipeline(pipelines.lum),
        pipeline_cache.get_render_pipeline(pipelines.adapt),
        view_uniforms.uniforms.binding(),
        post_uniforms.binding(),
    ) else {
        return;
    };
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let time_span = diagnostics.time_span(ctx.command_encoder(), "aa_adapt");

    let device = ctx.render_device().clone();
    let state_view = adapt.state_view();
    let cells_view = adapt.cells.texture.create_view(&TextureViewDescriptor::default());
    let next_view = adapt.next.texture.create_view(&TextureViewDescriptor::default());
    let multisample = pipelines.multisample as usize;
    let measure_group = device.create_bind_group(
        Some("aa lum bind group"),
        &pipeline_cache.get_bind_group_layout(&data.bright[multisample]),
        &BindGroupEntries::sequential((
            view_binding.clone(),
            post_binding.clone(),
            depth_view,
            target.main_texture_view(),
            &state_view,
        )),
    );
    // the cells are read through the bloom slot (a filterable 2D texture; the shafts slot is filled with the same)
    let update_group = crate::post::last_group(
        &device,
        &pipeline_cache.get_bind_group_layout(&data.last[multisample]),
        view_binding,
        post_binding,
        depth_view,
        target.main_texture_view(),
        &state_view,
        &cells_view,
        &data.sampler,
        &cells_view,
    );

    let encoder = ctx.command_encoder();
    for (label, pipeline, group, view, indices) in [
        ("aa lum pass", measure, &measure_group, &cells_view, [view_offset.offset, post_index.index()]),
        ("aa adapt pass", update, &update_group, &next_view, [view_offset.offset, post_index.index()]),
    ] {
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some(label),
            color_attachments: &[Some(RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: Operations { load: LoadOp::Clear(default()), store: StoreOp::Store },
            })],
            ..default()
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, group, &indices);
        pass.draw(0..3, 0..1);
    }
    encoder.copy_texture_to_texture(copy_info(&adapt.next), copy_info(&adapt.state), ONE);
    time_span.end(ctx.command_encoder());
}
