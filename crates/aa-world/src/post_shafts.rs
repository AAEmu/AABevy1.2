//! The 1.2 merged HDR sun shafts: normalized linear depth, three weighted eight-tap
//! radial blurs (0.1, 0.025, 0.01), then a screen blend after film mapping and before
//! color grading. Evidence: renderer 3806CD70/3806E3C0/3815D290 and sunshafts.cfx.
//! `Sun shafts visibility` controls the standalone effect; HDR uses RaysAmount.
//! Cost at 1920 x 1080: four passes over 480 x 270.

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
// The native $BackBufferScaled_d4 and generation temporary use format 2,
// A8R8G8B8 (38266500 / 381C0330). Keep its per-pass saturation.
pub(crate) const SHAFT_FORMAT: TextureFormat = TextureFormat::Rgba8Unorm;

/// Native HDR controls, with the Bevy reversed-Z/camera-distance conversion kept
/// separate from intensity. The game's default RaysSunColInfluence is 1 (no white mix).
pub(crate) fn hdr_controls(rays: f32, attenuation: f32, sun: Vec3, gain: f32, far: f32, near: f32) -> (Vec4, Vec4) {
    let far = far.max(near + 1.0);
    let color = sun / (sun.length_squared() + f32::MIN_POSITIVE).sqrt();
    (Vec4::new(rays * gain.max(0.0), far / (far - near), attenuation.clamp(0.0, 10.0), far), color.extend(0.0))
}

/// The two quarter size textures the shafts ping-pong between (`b` holds the result).
#[derive(Component)]
pub(crate) struct ShaftTextures {
    a: CachedTexture,
    b: CachedTexture,
}

impl ShaftTextures {
    /// The finished shafts.
    pub(crate) fn view(&self) -> TextureView {
        self.b.texture.create_view(&TextureViewDescriptor::default())
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
                    size: Extent3d { width: (size.x / 4).max(1), height: (size.y / 4).max(1), depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: TextureDimension::D2,
                    format: SHAFT_FORMAT,
                    usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
            )
        };
        let (a, b) = (make("aa shafts a"), make("aa shafts b"));
        commands.entity(entity).insert(ShaftTextures { a, b });
    }
}

/// Mask and three blurs. Does nothing while the HDR ray intensity is 0.
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
    let (Some(mask), Some(blur1), Some(blur2), Some(blur3), Some(view_binding), Some(post_binding)) = (
        pipeline_cache.get_render_pipeline(pipelines.shaft_mask),
        pipeline_cache.get_render_pipeline(pipelines.shaft_blur[0]),
        pipeline_cache.get_render_pipeline(pipelines.shaft_blur[1]),
        pipeline_cache.get_render_pipeline(pipelines.shaft_blur[2]),
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
    let a = shafts.a.texture.create_view(&TextureViewDescriptor::default());
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
        ("aa shafts blur 3", blur3, &from_a, &b),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_hdr_color_oracle() {
        // Original x86 GetShaftsParams + HDRFinalPass color slice, executed by
        // tools/native_sun_shafts_oracle.py. Neither Amount nor Active is read.
        for (rays, sun, expected) in [
            (2.5, Vec3::ONE, Vec3::splat(1.443375587)),
            (1.0, Vec3::new(4.0, 2.0, 1.0), Vec3::new(0.872871518, 0.436435759, 0.218217850)),
            (0.1, Vec3::ZERO, Vec3::ZERO),
            (0.21299468, Vec3::new(0.2, 0.15, 0.1), Vec3::new(0.158208475, 0.118656367, 0.079104245)),
        ] {
            let (controls, color) = hdr_controls(rays, 5.0, sun, 1.0, 2000.0, 0.1);
            assert!((color.truncate() * controls.x - expected).abs().max_element() < 3e-7);
        }
    }

    #[test]
    fn attenuation_limits_and_tuning_do_not_change_native_projection() {
        let (a, _) = hdr_controls(2.5, -1.0, Vec3::ONE, 1.0, 2000.0, 0.1);
        let (b, _) = hdr_controls(2.5, 12.0, Vec3::ONE, 0.0, 2000.0, 0.1);
        assert_eq!((a.x, a.z, b.x, b.z), (2.5, 0.0, 0.0, 10.0));
        assert_eq!((a.y, a.w), (b.y, b.w));
        assert_eq!(a.w, 2000.0);
        assert!((a.y - 1.0000500).abs() < 1e-7);
    }
}
