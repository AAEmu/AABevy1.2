//! The depth of field of the post pass, drawn as 1.2's renderer draws it ([`crate::depth_of_field`] holds the rules and what was read):
//! one blurred half resolution copy of the fogged scene, blended into each pixel by that pixel's own depth in the final pass.
//!
//! 1. `dof_down_fragment`: the fogged scene box-filtered to half the view's size (1.2's `texToTexParams` take the four full resolution
//!    pixels of the block), each pixel weighted by how much of the copy it takes (premultiplied: rgb x w, w), so a sharp speaker adds
//!    nothing to the blur of the wall behind her and draws no halo,
//! 2. `dof_blur_h_fragment`, `dof_blur_v_fragment`: a separable gaussian of the premultiplied copy
//!    ([`crate::depth_of_field::gaussian_taps`]: sigma 0.66 texel at rest, the merged pass's ring; 4 texels at 1080 p behind a quest talk,
//!    a fit to a native screenshot),
//! 3. the final pass (`post.wesl`, `fragment`) blends `copy.rgb / copy.a` in by the composite weight of the full resolution depth.
//!
//! No pass runs while neither the talk's focus ([`PostUniform::dof`]) nor the time of day's ([`PostUniform::dof_rest`]) is on. Cost at
//! 1920 x 1080: three passes over 960 x 540 (four fog evaluations a pixel in the first, as the bright pass; at most 25 bilinear taps in
//! the others) and one bilinear tap a pixel in the final pass.

use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::diagnostic::RecordDiagnostics;
use bevy::render::extract_component::{ComponentUniforms, DynamicUniformIndex};
use bevy::render::render_resource::{
    BindGroupEntries, Extent3d, LoadOp, Operations, PipelineCache, RenderPassColorAttachment, RenderPassDescriptor, StoreOp,
    TextureDescriptor, TextureDimension, TextureFormat, TextureUsages, TextureView, TextureViewDescriptor,
};
use bevy::render::renderer::{RenderContext, RenderDevice, ViewQuery};
use bevy::render::texture::{CachedTexture, TextureCache};
use bevy::render::view::{ViewDepthStencilTexture, ViewTarget, ViewUniformOffset, ViewUniforms};

use crate::post::{PostPipelineData, PostPipelineId, PostUniform};
use crate::post_adapt::AdaptTextures;

/// The copy is HDR and premultiplied (1.2's is `R16G16B16A16_FLOAT` too).
pub(crate) const DOF_FORMAT: TextureFormat = TextureFormat::Rgba16Float;

/// The two half size textures the passes ping-pong between (`a` holds the result).
#[derive(Component)]
pub(crate) struct DofTextures {
    a: CachedTexture,
    b: CachedTexture,
}

impl DofTextures {
    /// The finished copy.
    pub(crate) fn view(&self) -> TextureView {
        self.a.texture.create_view(&TextureViewDescriptor::default())
    }
}

/// Whether the frame draws a depth of field (the talk's or the time of day's).
pub(crate) fn drawn(post: &PostUniform) -> bool {
    post.dof.x > 0.0 || post.dof_rest.x > 0.0
}

pub(crate) fn prepare_dof_textures(
    mut commands: Commands,
    mut texture_cache: ResMut<TextureCache>,
    render_device: Res<RenderDevice>,
    views: Query<(Entity, &ExtractedCamera), With<PostUniform>>,
) {
    for (entity, camera) in &views {
        let Some(size) = camera.physical_viewport_size else { continue };
        let mut make = |label: &'static str| {
            texture_cache.get(
                &render_device,
                TextureDescriptor {
                    label: Some(label),
                    size: Extent3d { width: size.x.div_ceil(2).max(1), height: size.y.div_ceil(2).max(1), depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: TextureDimension::D2,
                    format: DOF_FORMAT,
                    usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
            )
        };
        let (a, b) = (make("aa dof a"), make("aa dof b"));
        commands.entity(entity).insert(DofTextures { a, b });
    }
}

/// The copy and its two blur passes (see the module docs). Does nothing while no depth of field is drawn.
#[allow(clippy::type_complexity)]
pub(crate) fn dof_passes(
    view: ViewQuery<(
        &ViewUniformOffset,
        &ViewTarget,
        &ViewDepthStencilTexture,
        &PostPipelineId,
        &DynamicUniformIndex<PostUniform>,
        &PostUniform,
        &DofTextures,
        &AdaptTextures,
    )>,
    pipeline_cache: Res<PipelineCache>,
    view_uniforms: Res<ViewUniforms>,
    post_uniforms: Res<ComponentUniforms<PostUniform>>,
    data: Option<Res<PostPipelineData>>,
    mut ctx: RenderContext,
) {
    let (view_offset, target, depth, pipelines, post_index, post, dof, adapt) = view.into_inner();
    if !drawn(post) {
        return;
    }
    let Some(data) = data else { return };
    let Some(depth_view) = depth.attachment.depth_stencil_views().depth_only_view() else { return };
    let (Some(down), Some(blur_h), Some(blur_v), Some(view_binding), Some(post_binding)) = (
        pipeline_cache.get_render_pipeline(pipelines.dof_down),
        pipeline_cache.get_render_pipeline(pipelines.dof_blur[0]),
        pipeline_cache.get_render_pipeline(pipelines.dof_blur[1]),
        view_uniforms.uniforms.binding(),
        post_uniforms.binding(),
    ) else {
        return;
    };
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let time_span = diagnostics.time_span(ctx.command_encoder(), "aa_dof");

    let device = ctx.render_device().clone();
    let multisample = pipelines.multisample as usize;
    let adapt_view = adapt.state_view();
    let a = dof.view();
    let b = dof.b.texture.create_view(&TextureViewDescriptor::default());
    let down_group = device.create_bind_group(
        Some("aa dof down bind group"),
        &pipeline_cache.get_bind_group_layout(&data.bright[multisample]),
        &BindGroupEntries::sequential((view_binding.clone(), post_binding.clone(), depth_view, target.main_texture_view(), &adapt_view)),
    );
    // a blur pass reads its input through the bloom slot (the shafts and dof slots hold the same: neither is read there)
    let layout = pipeline_cache.get_bind_group_layout(&data.last[multisample]);
    let pass_group = |source: &TextureView| {
        crate::post::last_group(
            &device,
            &layout,
            view_binding.clone(),
            post_binding.clone(),
            depth_view,
            target.main_texture_view(),
            &adapt_view,
            source,
            &data.sampler,
            source,
            source,
        )
    };
    let (from_a, from_b) = (pass_group(&a), pass_group(&b));

    let indices = [view_offset.offset, post_index.index()];
    let encoder = ctx.command_encoder();
    // the result ends in `a`, which the final pass reads
    let passes = [("aa dof down", down, &down_group, &a), ("aa dof blur h", blur_h, &from_a, &b), ("aa dof blur v", blur_v, &from_b, &a)];
    for &(label, pipeline, group, view) in &passes {
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
    time_span.end(ctx.command_encoder());
}
