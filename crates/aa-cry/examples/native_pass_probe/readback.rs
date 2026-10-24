//! Copies actual renderer-owned attachments, not a screenshot proxy.
use bevy::{
    core_pipeline::prepass::ViewPrepassTextures,
    prelude::*,
    render::{
        render_resource::*,
        renderer::{RenderContext, RenderDevice},
        view::ViewDepthStencilTexture,
    },
};
use std::{path::PathBuf, sync::Mutex};

struct Pending {
    buffer: Buffer,
    path: PathBuf,
}
#[derive(Resource, Default)]
pub(crate) struct Readbacks(Mutex<Vec<Pending>>);

pub(crate) fn directory(frame: u64) -> Option<PathBuf> {
    if !super::flag("--capture") {
        return None;
    }
    let wanted = super::arg("--capture-frame")
        .and_then(|s| s.parse().ok())
        .unwrap_or(360);
    if frame != wanted && frame != wanted + 1 {
        return None;
    }
    Some(PathBuf::from(super::arg("--out").unwrap_or_else(|| {
        "data/extract/aa-cry/native-pass-probe-review01-fix-20261024/default".into()
    })))
}
pub(crate) fn capture(
    world: &World,
    ctx: &mut RenderContext,
    view: (Entity, u32),
    frame: u64,
    stage: &str,
    depth: &ViewDepthStencilTexture,
    textures: Option<&ViewPrepassTextures>,
) {
    let Some(dir) = directory(frame) else { return };
    std::fs::create_dir_all(&dir).unwrap();
    let device = world.resource::<RenderDevice>();
    let mut attachments = vec![("depth", depth.texture())];
    if let Some(a) = textures.and_then(|t| t.normal.as_ref()) {
        attachments.push(("normal", &a.texture.texture));
    }
    if let Some(a) = textures.and_then(|t| t.motion_vectors.as_ref()) {
        attachments.push(("motion", &a.texture.texture));
    }
    if stage != "z"
        && let Some(target) = world.get::<bevy::render::view::ViewTarget>(view.0)
    {
        attachments.push(("colour", target.main_texture()));
    }
    for (kind, texture) in attachments {
        let size = texture.size();
        let pixel_size = if texture.format() == TextureFormat::Rgba16Float {
            8
        } else {
            4
        };
        let row_pitch = (size.width * pixel_size).div_ceil(256) * 256;
        let len = row_pitch as u64 * size.height as u64;
        let buffer = device.create_buffer(&BufferDescriptor {
            label: Some("probe attachment readback"),
            size: len,
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut source = texture.as_image_copy();
        if kind == "depth" {
            source.aspect = TextureAspect::DepthOnly;
        }
        ctx.command_encoder().copy_texture_to_buffer(
            source,
            TexelCopyBufferInfo {
                buffer: &buffer,
                layout: TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row_pitch),
                    rows_per_image: Some(size.height),
                },
            },
            size,
        );
        let path = dir.join(format!("v{}-f{frame}-{stage}-{kind}.bin", view.1));
        std::fs::write(
            path.with_extension("txt"),
            format!(
                "width={}\nheight={}\nrow_pitch={}\nformat={:?}\n",
                size.width,
                size.height,
                row_pitch,
                texture.format()
            ),
        )
        .unwrap();
        world
            .resource::<Readbacks>()
            .0
            .lock()
            .unwrap()
            .push(Pending { buffer, path });
    }
}
pub(crate) fn map(readbacks: Res<Readbacks>) {
    for pending in readbacks.0.lock().unwrap().drain(..) {
        let buffer = pending.buffer.clone();
        pending
            .buffer
            .slice(..)
            .map_async(MapMode::Read, move |result| {
                result.expect("probe attachment map");
                {
                    let data = buffer.slice(..).get_mapped_range().unwrap();
                    std::fs::write(&pending.path, &data).unwrap();
                }
                buffer.unmap();
                info!("native-pass-probe: attachment {}", pending.path.display());
            });
    }
}
