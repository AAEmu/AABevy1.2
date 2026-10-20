//! The bloom of the post pass: the engine's bright pass and a blur chain.
//!
//! The engine (`postprocess.cfx`, 1.2 client with `r_HDRRendering 3`) runs `HDRBrightPassFilterPS` on the half resolution
//! scene (after its fog pass): `v = scene x level / adapted luminance - threshold`, `v / (offset + v)` per channel, so a
//! pixel's contribution saturates below 1; three blurred levels are summed (`HDRFlaresPS`) and `FilmicMapping` adds them to the
//! exposed scene before the grading. Here:
//!
//! 1. `bloom_passes` draws the bright pass (`post.wesl`, `bright_fragment`: it includes the fog and the under water fog of the
//!    final pass) into mip 0 of a texture at half the view's size,
//! 2. downsamples it to mips 1 to 3 (1/4, 1/8, 1/16: 13 tap filter) and adds every mip, tent filtered, back onto the one above,
//! 3. the final pass (`post.rs`) samples mip 0 and adds it to the exposed scene.
//!
//! Cost at 1920 x 1080: one bright pass over 960 x 540 (four fog evaluations per pixel, as a full resolution pass costs),
//! 3 + 3 small blur passes; mip 0 is 4 bytes per pixel (`Rg11b10Ufloat`).

use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::diagnostic::RecordDiagnostics;
use bevy::render::extract_component::{ComponentUniforms, DynamicUniformIndex};
use bevy::render::render_resource::{
    BindGroupEntries, Extent3d, LoadOp, Operations, PipelineCache, RenderPassColorAttachment, RenderPassDescriptor,
    StoreOp, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages, TextureView, TextureViewDescriptor,
};
use bevy::render::renderer::{RenderContext, RenderDevice, ViewQuery};
use bevy::render::texture::{CachedTexture, TextureCache};
use bevy::render::view::{ViewDepthStencilTexture, ViewTarget, ViewUniformOffset, ViewUniforms};

use crate::post_adapt::AdaptTextures;
use crate::post::{PostPipelineData, PostPipelineId, PostUniform};

/// Bloom textures are small and only ever blurred: 11 / 11 / 10 bit floats are enough.
pub(crate) const BLOOM_FORMAT: TextureFormat = TextureFormat::Rg11b10Ufloat;

/// Mips of the chain: 1/2, 1/4, 1/8 and 1/16 of the view.
const MIPS: u32 = 4;

/// The chain's texture of a view (the mips are the levels).
#[derive(Component)]
pub(crate) struct BloomTextures {
    texture: CachedTexture,
}

impl BloomTextures {
    pub(crate) fn view(&self, mip: u32) -> TextureView {
        self.texture.texture.create_view(&TextureViewDescriptor { base_mip_level: mip, mip_level_count: Some(1), ..default() })
    }
}

pub(crate) fn prepare_bloom_textures(
    mut commands: Commands,
    mut texture_cache: ResMut<TextureCache>,
    render_device: Res<RenderDevice>,
    views: Query<(Entity, &ExtractedCamera), With<PostUniform>>,
) {
    for (entity, camera) in &views {
        let Some(size) = camera.physical_viewport_size else { continue };
        let texture = texture_cache.get(
            &render_device,
            TextureDescriptor {
                label: Some("aa bloom chain"),
                size: Extent3d { width: size.x.div_ceil(2).max(1), height: size.y.div_ceil(2).max(1), depth_or_array_layers: 1 },
                mip_level_count: MIPS,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: BLOOM_FORMAT,
                usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
        );
        commands.entity(entity).insert(BloomTextures { texture });
    }
}

/// Bright pass, down chain, up chain (see the module docs).
#[allow(clippy::type_complexity)]
pub(crate) fn bloom_passes(
    view: ViewQuery<(
        &ViewUniformOffset,
        &ViewTarget,
        &ViewDepthStencilTexture,
        &PostPipelineId,
        &DynamicUniformIndex<PostUniform>,
        &BloomTextures,
        &AdaptTextures,
    )>,
    pipeline_cache: Res<PipelineCache>,
    view_uniforms: Res<ViewUniforms>,
    post_uniforms: Res<ComponentUniforms<PostUniform>>,
    data: Option<Res<PostPipelineData>>,
    mut ctx: RenderContext,
) {
    let (view_offset, target, depth, pipelines, post_index, bloom, adapt) = view.into_inner();
    let Some(data) = data else { return };
    let Some(depth_view) = depth.attachment.depth_stencil_views().depth_only_view() else { return };
    let (Some(bright), Some(down), Some(up), Some(view_binding), Some(post_binding)) = (
        pipeline_cache.get_render_pipeline(pipelines.bright),
        pipeline_cache.get_render_pipeline(pipelines.down),
        pipeline_cache.get_render_pipeline(pipelines.up),
        view_uniforms.uniforms.binding(),
        post_uniforms.binding(),
    ) else {
        return;
    };
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let time_span = diagnostics.time_span(ctx.command_encoder(), "aa_bloom");

    let device = ctx.render_device().clone();
    let views: Vec<TextureView> = (0..MIPS).map(|m| bloom.view(m)).collect();
    // the chain's bind groups: mip m as input
    let chain_layout = pipeline_cache.get_bind_group_layout(&data.chain);
    let chain: Vec<_> = views
        .iter()
        .map(|v| device.create_bind_group(Some("aa bloom chain bind group"), &chain_layout, &BindGroupEntries::sequential((v, &data.sampler))))
        .collect();
    let bright_group = device.create_bind_group(
        Some("aa bright bind group"),
        &pipeline_cache.get_bind_group_layout(&data.bright[pipelines.multisample as usize]),
        &BindGroupEntries::sequential((view_binding, post_binding, depth_view, target.main_texture_view(), &adapt.state_view())),
    );

    let encoder = ctx.command_encoder();
    // the bright pass of the fogged scene into mip 0
    {
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("aa bright pass"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: &views[0],
                depth_slice: None,
                resolve_target: None,
                ops: Operations { load: LoadOp::Clear(default()), store: StoreOp::Store },
            })],
            ..default()
        });
        pass.set_pipeline(bright);
        pass.set_bind_group(0, &bright_group, &[view_offset.offset, post_index.index()]);
        pass.draw(0..3, 0..1);
    }
    // down: mip m-1 into mip m
    for m in 1..MIPS as usize {
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("aa bloom down"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: &views[m],
                depth_slice: None,
                resolve_target: None,
                ops: Operations { load: LoadOp::Clear(default()), store: StoreOp::Store },
            })],
            ..default()
        });
        pass.set_pipeline(down);
        pass.set_bind_group(0, &chain[m - 1], &[]);
        pass.draw(0..3, 0..1);
    }
    // up: mip m is added onto mip m-1, from the smallest
    for m in (1..MIPS as usize).rev() {
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("aa bloom up"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: &views[m - 1],
                depth_slice: None,
                resolve_target: None,
                ops: Operations { load: LoadOp::Load, store: StoreOp::Store },
            })],
            ..default()
        });
        pass.set_pipeline(up);
        pass.set_bind_group(0, &chain[m], &[]);
        pass.draw(0..3, 0..1);
    }
    time_span.end(ctx.command_encoder());
}
