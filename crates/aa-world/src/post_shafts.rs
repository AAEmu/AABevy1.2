//! Sun shafts of the post pass: screen space god rays (the engine's `SunShafts` effect).
//!
//! 1. `shafts_mask_fragment` takes the sky around the sun (depth beyond the dome, weighted by the angle to the sun) at a quarter
//!    of the view's size; everything in front of the sky is black,
//! 2. two radial blurs towards the sun's screen position (12 taps each; the first a twelfth of the length, the second all of
//!    it) smear the sky light through the gaps between trees and buildings,
//! 3. the final pass adds the result to the exposed scene, weaker with the distance from the sun
//!    (`Sun rays attenuation`), scaled by the zone's `Sun shafts visibility`; the rays are as long as `Sun rays visibility` says.
//!    The effect fades out when the sun is behind the view and is skipped (no pass at all) while the strength is 0.
//!
//! Cost at 1920 x 1080: three passes over 480 x 270.

use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::diagnostic::RecordDiagnostics;
use bevy::render::extract_component::{ComponentUniforms, DynamicUniformIndex};
use bevy::render::render_resource::{
    BindGroupEntries, Extent3d, LoadOp, Operations, PipelineCache, RenderPassColorAttachment, RenderPassDescriptor, StoreOp,
    TextureDescriptor, TextureDimension, TextureUsages, TextureView, TextureViewDescriptor,
};
use bevy::render::renderer::{RenderContext, RenderDevice, ViewQuery};
use bevy::render::texture::{CachedTexture, TextureCache};
use bevy::render::view::{ViewDepthStencilTexture, ViewTarget, ViewUniformOffset, ViewUniforms};

use crate::post::{PostPipelineData, PostPipelineId, PostUniform};
use crate::post_adapt::AdaptTextures;
use crate::post_bloom::BLOOM_FORMAT;

/// The two quarter size textures the shafts ping-pong between (`a` holds the result).
#[derive(Component)]
pub(crate) struct ShaftTextures {
    a: CachedTexture,
    b: CachedTexture,
}

impl ShaftTextures {
    /// The finished shafts.
    pub(crate) fn view(&self) -> TextureView {
        self.a.texture.create_view(&TextureViewDescriptor::default())
    }
}

pub(crate) fn prepare_shaft_textures(
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
                    size: Extent3d { width: size.x.div_ceil(4).max(1), height: size.y.div_ceil(4).max(1), depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: TextureDimension::D2,
                    format: BLOOM_FORMAT,
                    usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
            )
        };
        let (a, b) = (make("aa shafts a"), make("aa shafts b"));
        commands.entity(entity).insert(ShaftTextures { a, b });
    }
}

/// Mask, two blurs (see the module docs). Does nothing while the strength is 0.
#[allow(clippy::type_complexity)]
pub(crate) fn shaft_passes(
    view: ViewQuery<(
        &ViewUniformOffset,
        &ViewTarget,
        &ViewDepthStencilTexture,
        &PostPipelineId,
        &DynamicUniformIndex<PostUniform>,
        &PostUniform,
        &ShaftTextures,
        &AdaptTextures,
    )>,
    pipeline_cache: Res<PipelineCache>,
    view_uniforms: Res<ViewUniforms>,
    post_uniforms: Res<ComponentUniforms<PostUniform>>,
    data: Option<Res<PostPipelineData>>,
    mut ctx: RenderContext,
) {
    let (view_offset, target, depth, pipelines, post_index, post, shafts, adapt) = view.into_inner();
    if post.shafts.x <= 0.0 {
        return;
    }
    let Some(data) = data else { return };
    let Some(depth_view) = depth.attachment.depth_stencil_views().depth_only_view() else { return };
    let (Some(mask), Some(blur1), Some(blur2), Some(view_binding), Some(post_binding)) = (
        pipeline_cache.get_render_pipeline(pipelines.shaft_mask),
        pipeline_cache.get_render_pipeline(pipelines.shaft_blur[0]),
        pipeline_cache.get_render_pipeline(pipelines.shaft_blur[1]),
        view_uniforms.uniforms.binding(),
        post_uniforms.binding(),
    ) else {
        return;
    };
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let time_span = diagnostics.time_span(ctx.command_encoder(), "aa_shafts");

    let device = ctx.render_device().clone();
    let multisample = pipelines.multisample as usize;
    let adapt_view = adapt.state_view();
    let a = shafts.view();
    let b = shafts.b.texture.create_view(&TextureViewDescriptor::default());
    let mask_group = device.create_bind_group(
        Some("aa shafts mask bind group"),
        &pipeline_cache.get_bind_group_layout(&data.bright[multisample]),
        &BindGroupEntries::sequential((view_binding.clone(), post_binding.clone(), depth_view, target.main_texture_view(), &adapt_view)),
    );
    // a blur reads its input through the bloom slot (and the shafts slot, which it does not use)
    let layout = pipeline_cache.get_bind_group_layout(&data.last[multisample]);
    let blur_group = |source: &TextureView| {
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
        )
    };
    let (from_a, from_b) = (blur_group(&a), blur_group(&b));

    let indices = [view_offset.offset, post_index.index()];
    let encoder = ctx.command_encoder();
    for (label, pipeline, group, view) in [
        ("aa shafts mask", mask, &mask_group, &a),
        ("aa shafts blur 1", blur1, &from_a, &b),
        ("aa shafts blur 2", blur2, &from_b, &a),
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
    time_span.end(ctx.command_encoder());
}
