//! DDS textures as the 1.2 pak stores them, to Bevy [`Image`]s.
//!
//! Formats in the pak (36,817 files): DXT1 27,613 (33 cubemaps), DXT5 5,595, DXT3 59,
//! ATI2/3Dc 1,125 (normal maps), A8R8G8B8 2,374 (1 cube, 1 volume), X8R8G8B8 2 (1 cube),
//! R8G8B8 38, L8 6 (1 volume), A8 3, G16R16F 2, R32F 1. Block-compressed data stays compressed
//! on the GPU (BC1/2/3/5); the rest is uploaded as is or expanded to RGBA8.
//!
//! CryEngine appends an extension block after the image data: `"CExt"`, then tagged chunks
//! (`"AvgC"` average colour, `"AttC"` attached image = a second complete DDS, used for the
//! gloss of 62 `_ddn` normal maps and the puddle masks), then `"CEnd"`. [`Dds::attached`]
//! exposes the attached image.

use bevy::asset::RenderAssetUsages;
use bevy::image::{Image, ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};

mod opacity;
pub(crate) use opacity::OpacitySampler;
use bevy::render::render_resource::{
    Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension,
};

use crate::{Error, Result};

/// Pixel formats found in the pak.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DdsFormat {
    /// DXT1
    Bc1,
    /// DXT3
    Bc2,
    /// DXT5
    Bc3,
    /// ATI1 / BC4U
    Bc4,
    /// ATI2 (3Dc) / BC5U: two-channel normal maps.
    Bc5,
    /// 32-bit A8R8G8B8 (bytes B, G, R, A).
    Bgra8,
    /// 32-bit X8R8G8B8 (alpha byte unused).
    Bgrx8,
    /// 24-bit R8G8B8 (bytes B, G, R).
    Bgr8,
    /// 8-bit luminance.
    L8,
    /// 8-bit alpha.
    A8,
    /// D3DFMT_G16R16F (fourcc 112).
    Rg16Float,
    /// D3DFMT_R32F (fourcc 114).
    R32Float,
}

impl DdsFormat {
    /// Bytes per 4x4 block for block formats, `None` otherwise.
    pub fn block_bytes(self) -> Option<usize> {
        match self {
            DdsFormat::Bc1 | DdsFormat::Bc4 => Some(8),
            DdsFormat::Bc2 | DdsFormat::Bc3 | DdsFormat::Bc5 => Some(16),
            _ => None,
        }
    }

    /// Bytes per pixel for uncompressed formats.
    pub fn pixel_bytes(self) -> usize {
        match self {
            DdsFormat::Bgra8 | DdsFormat::Bgrx8 | DdsFormat::Rg16Float | DdsFormat::R32Float => 4,
            DdsFormat::Bgr8 => 3,
            DdsFormat::L8 | DdsFormat::A8 => 1,
            _ => 0,
        }
    }

    /// Bytes of one `w x h` surface.
    pub fn surface_bytes(self, w: u32, h: u32) -> usize {
        let (w, h) = (w.max(1) as usize, h.max(1) as usize);
        match self.block_bytes() {
            Some(b) => w.div_ceil(4) * h.div_ceil(4) * b,
            None => w * h * self.pixel_bytes(),
        }
    }
}

/// A parsed DDS file borrowing the bytes.
#[derive(Clone, Debug)]
pub struct Dds<'a> {
    pub width: u32,
    pub height: u32,
    /// Volume depth (1 for 2D and cube textures).
    pub depth: u32,
    /// Mip levels actually present in `data` (>= 1).
    pub mips: u32,
    /// The header requested mip levels that were invalid or did not fit the file.
    pub mips_truncated: bool,
    /// 6 for cubemaps, else 1.
    pub faces: u32,
    pub format: DdsFormat,
    /// Image data in DDS order: face -> mip -> (depth slice) -> rows.
    pub data: &'a [u8],
    /// CryEngine `AttC` attached image (e.g. gloss for `_ddn` normal maps).
    pub attached: Option<Box<Dds<'a>>>,
    /// CryEngine `AvgC` average colour, as stored (u32).
    pub average_color: Option<u32>,
    /// ATI2 (3Dc): the first BC4 block holds Y, the second X (BC5 is X then Y). The loaders
    /// swap them so the GPU / decoded image has X in red and Y in green.
    pub swapped_rg: bool,
}

impl<'a> Dds<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Dds<'a>> {
        if bytes.len() < 128 || &bytes[..4] != b"DDS " {
            return Err(Error::invalid("not a DDS file"));
        }
        let u = |o: usize| u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap());
        let flags = u(8);
        let height = u(12);
        let width = u(16);
        let depth_field = u(24);
        let mip_field = u(28);
        let pf_flags = u(80);
        let fourcc = &bytes[84..88];
        let bits = u(88);
        let (rmask, gmask, bmask, amask) = (u(92), u(96), u(100), u(104));
        let caps2 = u(112);
        if width == 0 || height == 0 || width > 16384 || height > 16384 {
            return Err(Error::invalid(format!("DDS size {width}x{height}")));
        }

        let swapped_rg = pf_flags & 0x4 != 0 && fourcc == b"ATI2";
        let format = if pf_flags & 0x4 != 0 {
            match fourcc {
                b"DXT1" => DdsFormat::Bc1,
                b"DXT2" | b"DXT3" => DdsFormat::Bc2,
                b"DXT4" | b"DXT5" => DdsFormat::Bc3,
                b"ATI1" | b"BC4U" => DdsFormat::Bc4,
                b"ATI2" | b"BC5U" => DdsFormat::Bc5,
                [112, 0, 0, 0] => DdsFormat::Rg16Float,
                [114, 0, 0, 0] => DdsFormat::R32Float,
                [28, 0, 0, 0] => DdsFormat::A8,
                [50, 0, 0, 0] => DdsFormat::L8,
                [21, 0, 0, 0] => DdsFormat::Bgra8,
                [22, 0, 0, 0] => DdsFormat::Bgrx8,
                [20, 0, 0, 0] => DdsFormat::Bgr8,
                f => {
                    return Err(Error::Unsupported(format!(
                        "DDS fourcc {:?}",
                        String::from_utf8_lossy(f)
                    )));
                }
            }
        } else if pf_flags & 0x40 != 0 {
            // RGB
            match (bits, rmask, gmask, bmask) {
                (32, 0xFF_0000, 0xFF00, 0xFF) => {
                    if pf_flags & 0x1 != 0 && amask == 0xFF00_0000 {
                        DdsFormat::Bgra8
                    } else {
                        DdsFormat::Bgrx8
                    }
                }
                (24, 0xFF_0000, 0xFF00, 0xFF) => DdsFormat::Bgr8,
                _ => {
                    return Err(Error::Unsupported(format!(
                        "DDS RGB {bits} bits masks {rmask:#x} {gmask:#x} {bmask:#x} {amask:#x}"
                    )));
                }
            }
        } else if pf_flags & 0x2_0000 != 0 && bits == 8 {
            DdsFormat::L8
        } else if (pf_flags & 0x2 != 0 || pf_flags & 0x1 != 0) && bits == 8 {
            DdsFormat::A8
        } else {
            return Err(Error::Unsupported(format!("DDS pixel format flags {pf_flags:#x} bits {bits}")));
        };

        let faces = if caps2 & 0x200 != 0 { 6 } else { 1 };
        let depth = if caps2 & 0x20_0000 != 0 && flags & 0x80_0000 != 0 { depth_field.max(1) } else { 1 };
        let max_mips = 32 - width.max(height).max(depth).leading_zeros();
        let declared = if flags & 0x2_0000 != 0 { mip_field.max(1) } else { 1 };
        let requested = declared;
        let declared = declared.min(max_mips);

        let rest = &bytes[128..];
        let face_bytes = |mips: u32| -> usize {
            (0..mips)
                .map(|m| format.surface_bytes(width >> m, height >> m) * (depth >> m).max(1) as usize)
                .sum()
        };
        // Truncated files: keep the mips that fit.
        let mut mips = declared;
        while mips > 1 && face_bytes(mips) * faces as usize > rest.len() {
            mips -= 1;
        }
        let size = face_bytes(mips) * faces as usize;
        if size > rest.len() {
            return Err(Error::invalid(format!("DDS data: need {size} bytes, have {}", rest.len())));
        }
        let data = &rest[..size];

        // CryEngine extension block.
        let mut attached = None;
        let mut average_color = None;
        let mut tail = &rest[size..];
        if tail.starts_with(b"CExt") {
            tail = &tail[4..];
            while tail.len() >= 4 && !tail.starts_with(b"CEnd") {
                let tag = &tail[..4];
                if tail.len() < 8 {
                    break;
                }
                let len = u32::from_le_bytes(tail[4..8].try_into().unwrap()) as usize;
                let body = tail.get(8..8 + len).unwrap_or(&tail[8..]);
                match tag {
                    b"AttC" => attached = Dds::parse(body).ok().map(Box::new),
                    b"AvgC" if body.len() >= 4 => {
                        average_color = Some(u32::from_le_bytes(body[..4].try_into().unwrap()))
                    }
                    _ => {}
                }
                tail = &tail[(8 + len).min(tail.len())..];
            }
        }
        Ok(Dds { width, height, depth, mips, mips_truncated: mips != requested, faces, format, data, attached, average_color, swapped_rg })
    }

    pub fn is_cubemap(&self) -> bool {
        self.faces == 6
    }

    /// Proves alpha is 255 throughout every authored mip of a complete, flat texture.
    /// Scans alpha only, without decoding colour or allocating a full image. Padding texels of
    /// partial BC blocks are included conservatively: transparent padding declines the proof.
    pub fn alpha_is_opaque(&self) -> bool {
        if self.faces != 1 || self.depth != 1 || self.mips_truncated || self.data.is_empty() {
            return false;
        }
        match self.format {
            DdsFormat::Bc1 => self.data.chunks_exact(8).all(|b| {
                let c0 = u16::from_le_bytes([b[0], b[1]]);
                let c1 = u16::from_le_bytes([b[2], b[3]]);
                let indices = u32::from_le_bytes(b[4..8].try_into().unwrap());
                // Selector 3 is transparent only in BC1's three-colour mode.
                c0 > c1 || indices & (indices >> 1) & 0x5555_5555 == 0
            }),
            DdsFormat::Bc2 => self.data.chunks_exact(16).all(|b| b[..8].iter().all(|a| *a == 255)),
            DdsFormat::Bc3 => self.data.chunks_exact(16).all(|b| {
                let mut alpha = [0; 16];
                bc4_block(b, &mut alpha);
                alpha.iter().all(|a| *a == 255)
            }),
            DdsFormat::Bgra8 => self.data.chunks_exact(4).all(|p| p[3] == 255),
            DdsFormat::A8 => self.data.iter().all(|a| *a == 255),
            // These formats provide an implicit alpha of one, including their authored mips.
            DdsFormat::Bc4 | DdsFormat::Bc5 | DdsFormat::Bgrx8 | DdsFormat::Bgr8
            | DdsFormat::L8 | DdsFormat::Rg16Float | DdsFormat::R32Float => true,
        }
    }

    /// Byte range of (face, mip) in `data` (all depth slices of that mip).
    pub fn surface_range(&self, face: u32, mip: u32) -> std::ops::Range<usize> {
        let level = |m: u32| {
            self.format.surface_bytes(self.width >> m, self.height >> m)
                * (self.depth >> m).max(1) as usize
        };
        let per_face: usize = (0..self.mips).map(level).sum();
        let start = face as usize * per_face + (0..mip).map(level).sum::<usize>();
        start..start + level(mip)
    }

    /// Decodes (face, mip, slice 0) to RGBA8 on the CPU. Luminance becomes grey, alpha-only
    /// becomes white with alpha, float formats are clamped to 0..1.
    pub fn decode_rgba8(&self, face: u32, mip: u32) -> Vec<u8> {
        let w = (self.width >> mip).max(1) as usize;
        let h = (self.height >> mip).max(1) as usize;
        let src = &self.data[self.surface_range(face, mip)];
        let mut out = vec![0u8; w * h * 4];
        match self.format {
            DdsFormat::Bc1 | DdsFormat::Bc2 | DdsFormat::Bc3 | DdsFormat::Bc4 | DdsFormat::Bc5 => {
                decode_blocks(self.format, src, w, h, &mut out);
                if self.swapped_rg {
                    for p in out.chunks_exact_mut(4) {
                        p.swap(0, 1);
                    }
                }
            }
            DdsFormat::Bgra8 | DdsFormat::Bgrx8 => {
                for (o, p) in out.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
                    let a = if self.format == DdsFormat::Bgra8 { p[3] } else { 255 };
                    o.copy_from_slice(&[p[2], p[1], p[0], a]);
                }
            }
            DdsFormat::Bgr8 => {
                for (o, p) in out.chunks_exact_mut(4).zip(src.chunks_exact(3)) {
                    o.copy_from_slice(&[p[2], p[1], p[0], 255]);
                }
            }
            DdsFormat::L8 => {
                for (o, &l) in out.chunks_exact_mut(4).zip(src) {
                    o.copy_from_slice(&[l, l, l, 255]);
                }
            }
            DdsFormat::A8 => {
                for (o, &a) in out.chunks_exact_mut(4).zip(src) {
                    o.copy_from_slice(&[255, 255, 255, a]);
                }
            }
            DdsFormat::Rg16Float => {
                for (o, p) in out.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
                    let r = half_to_f32(u16::from_le_bytes([p[0], p[1]]));
                    let g = half_to_f32(u16::from_le_bytes([p[2], p[3]]));
                    o.copy_from_slice(&[unorm8(r), unorm8(g), 0, 255]);
                }
            }
            DdsFormat::R32Float => {
                for (o, p) in out.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
                    let r = f32::from_le_bytes(p.try_into().unwrap());
                    o.copy_from_slice(&[unorm8(r), 0, 0, 255]);
                }
            }
        }
        out
    }
}

fn unorm8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// How a DDS becomes an [`Image`].
#[derive(Clone, Copy, Debug)]
pub struct DdsOptions {
    /// Colour data (diffuse, specular colour, UI): sample with sRGB decoding. Normal maps,
    /// gloss and masks must be linear.
    pub srgb: bool,
    /// The GPU supports BC1-5 (`CompressedImageFormats::BC`); otherwise everything is decoded
    /// to RGBA8 on the CPU (mip 0 only for cubemaps/volumes).
    pub bc_supported: bool,
    /// Wrap (repeat) addressing with trilinear + anisotropic filtering, as CryEngine samples
    /// material textures. `false` keeps Bevy's default sampler (clamp; good for UI).
    pub repeat: bool,
}

impl Default for DdsOptions {
    fn default() -> Self {
        DdsOptions { srgb: true, bc_supported: true, repeat: true }
    }
}

impl DdsOptions {
    pub fn linear() -> Self {
        DdsOptions { srgb: false, ..Default::default() }
    }
}

/// DDS bytes -> Bevy [`Image`] with all mips (and cube faces / volume slices).
pub fn load_dds(bytes: &[u8], options: DdsOptions) -> Result<Image> {
    dds_to_image(&Dds::parse(bytes)?, options)
}

/// Reads a DDS from the pak and converts it ([`load_dds`]).
pub fn load_dds_from_pak(path: &str, options: DdsOptions) -> Result<Image> {
    load_dds(&crate::read(path)?, options)
}

/// A parsed [`Dds`] -> Bevy [`Image`].
pub fn dds_to_image(dds: &Dds<'_>, options: DdsOptions) -> Result<Image> {
    let srgb = options.srgb;
    let block_aligned = dds.width % 4 == 0 && dds.height % 4 == 0;
    let (format, data): (TextureFormat, Vec<u8>) = match dds.format {
        DdsFormat::Bc1 | DdsFormat::Bc2 | DdsFormat::Bc3 | DdsFormat::Bc4 | DdsFormat::Bc5
            if options.bc_supported && block_aligned =>
        {
            let f = match (dds.format, srgb) {
                (DdsFormat::Bc1, false) => TextureFormat::Bc1RgbaUnorm,
                (DdsFormat::Bc1, true) => TextureFormat::Bc1RgbaUnormSrgb,
                (DdsFormat::Bc2, false) => TextureFormat::Bc2RgbaUnorm,
                (DdsFormat::Bc2, true) => TextureFormat::Bc2RgbaUnormSrgb,
                (DdsFormat::Bc3, false) => TextureFormat::Bc3RgbaUnorm,
                (DdsFormat::Bc3, true) => TextureFormat::Bc3RgbaUnormSrgb,
                (DdsFormat::Bc4, _) => TextureFormat::Bc4RUnorm,
                _ => TextureFormat::Bc5RgUnorm,
            };
            let mut data = dds.data.to_vec();
            if dds.swapped_rg {
                for block in data.chunks_exact_mut(16) {
                    let (a, b) = block.split_at_mut(8);
                    a.swap_with_slice(b);
                }
            }
            (f, data)
        }
        DdsFormat::Bgra8 => {
            (if srgb { TextureFormat::Bgra8UnormSrgb } else { TextureFormat::Bgra8Unorm }, dds.data.to_vec())
        }
        DdsFormat::Rg16Float => (TextureFormat::Rg16Float, dds.data.to_vec()),
        DdsFormat::R32Float => (TextureFormat::R32Float, dds.data.to_vec()),
        DdsFormat::Bgrx8 | DdsFormat::Bgr8 | DdsFormat::L8 | DdsFormat::A8 => {
            // Expand every surface to RGBA8.
            let mut out = Vec::with_capacity(dds.data.len() * 4);
            for face in 0..dds.faces {
                for mip in 0..dds.mips {
                    let r = dds.surface_range(face, mip);
                    let bpp = dds.format.pixel_bytes();
                    for p in dds.data[r].chunks_exact(bpp) {
                        out.extend_from_slice(&match dds.format {
                            DdsFormat::Bgrx8 => [p[2], p[1], p[0], 255],
                            DdsFormat::Bgr8 => [p[2], p[1], p[0], 255],
                            DdsFormat::L8 => [p[0], p[0], p[0], 255],
                            _ => [255, 255, 255, p[0]],
                        });
                    }
                }
            }
            (if srgb { TextureFormat::Rgba8UnormSrgb } else { TextureFormat::Rgba8Unorm }, out)
        }
        // Block formats the GPU cannot take: decode every face/mip on the CPU.
        _ => {
            let mut out = Vec::new();
            for face in 0..dds.faces {
                for mip in 0..dds.mips {
                    out.extend(dds.decode_rgba8(face, mip));
                }
            }
            (if srgb { TextureFormat::Rgba8UnormSrgb } else { TextureFormat::Rgba8Unorm }, out)
        }
    };
    if dds.depth > 1 && dds.format.block_bytes().is_some() && !format.is_compressed() {
        // The CPU decoder handles slice 0 only (no such volume exists in the pak).
        return Err(Error::Unsupported("block-compressed volume texture without BC support".into()));
    }

    let layers = if dds.depth > 1 { dds.depth } else { dds.faces };
    let size = Extent3d { width: dds.width, height: dds.height, depth_or_array_layers: layers };
    let mut image = Image::new_uninit(
        size,
        if dds.depth > 1 { TextureDimension::D3 } else { TextureDimension::D2 },
        format,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.size = size.physical_size(format);
    image.texture_descriptor.mip_level_count = dds.mips;
    image.data = Some(data);
    if dds.is_cubemap() {
        image.texture_view_descriptor = Some(TextureViewDescriptor {
            dimension: Some(TextureViewDimension::Cube),
            ..Default::default()
        });
    }
    if options.repeat {
        image.sampler = ImageSampler::Descriptor(repeat_sampler());
    }
    Ok(image)
}

/// An RGBA8 image from decoded mips (mip 0 first), sampled like the material textures.
pub fn rgba8_image(width: u32, height: u32, mips: &[Vec<u8>], srgb: bool) -> Image {
    let format = if srgb { TextureFormat::Rgba8UnormSrgb } else { TextureFormat::Rgba8Unorm };
    let size = Extent3d { width, height, depth_or_array_layers: 1 };
    let mut image = Image::new_uninit(size, TextureDimension::D2, format, RenderAssetUsages::RENDER_WORLD);
    image.texture_descriptor.mip_level_count = mips.len().max(1) as u32;
    image.data = Some(mips.concat());
    image.sampler = ImageSampler::Descriptor(repeat_sampler());
    image
}

/// Wrap addressing, trilinear, 4x anisotropic: how CryEngine samples material textures.
pub fn repeat_sampler() -> ImageSamplerDescriptor {
    ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        address_mode_w: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 4,
        ..Default::default()
    }
}

/// The sampler of a bump (normal) map: [`repeat_sampler`] without anisotropy, trilinear, as 1.2's `bumpMapSampler`.
pub fn bump_sampler() -> ImageSamplerDescriptor {
    ImageSamplerDescriptor { anisotropy_clamp: 1, ..repeat_sampler() }
}

// ---- BC decoding (CPU fallback) ----

fn rgb565(c: u16) -> [u8; 3] {
    let r = ((c >> 11) & 31) as u32;
    let g = ((c >> 5) & 63) as u32;
    let b = (c & 31) as u32;
    [((r * 527 + 23) >> 6) as u8, ((g * 259 + 33) >> 6) as u8, ((b * 527 + 23) >> 6) as u8]
}

/// Decodes a BC1 colour block into 16 RGBA texels. `four_color` forces the 4-colour mode
/// (BC2/BC3 colour blocks).
fn bc1_block(b: &[u8], four_color: bool, out: &mut [[u8; 4]; 16]) {
    let c0 = u16::from_le_bytes([b[0], b[1]]);
    let c1 = u16::from_le_bytes([b[2], b[3]]);
    let (p0, p1) = (rgb565(c0), rgb565(c1));
    let mut pal = [[0u8; 4]; 4];
    pal[0] = [p0[0], p0[1], p0[2], 255];
    pal[1] = [p1[0], p1[1], p1[2], 255];
    let mix = |a: u8, b: u8, wa: u32, wb: u32, d: u32| ((a as u32 * wa + b as u32 * wb) / d) as u8;
    if c0 > c1 || four_color {
        for k in 0..3 {
            pal[2][k] = mix(p0[k], p1[k], 2, 1, 3);
            pal[3][k] = mix(p0[k], p1[k], 1, 2, 3);
        }
        pal[2][3] = 255;
        pal[3][3] = 255;
    } else {
        for k in 0..3 {
            pal[2][k] = mix(p0[k], p1[k], 1, 1, 2);
        }
        pal[2][3] = 255;
        pal[3] = [0, 0, 0, 0];
    }
    let idx = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
    for (i, t) in out.iter_mut().enumerate() {
        *t = pal[((idx >> (2 * i)) & 3) as usize];
    }
}

/// Decodes a BC4 (BC3 alpha) block into 16 values.
fn bc4_block(b: &[u8], out: &mut [u8; 16]) {
    let (a0, a1) = (b[0] as u32, b[1] as u32);
    let mut pal = [0u8; 8];
    pal[0] = a0 as u8;
    pal[1] = a1 as u8;
    if a0 > a1 {
        for i in 1..7 {
            pal[i + 1] = (((7 - i as u32) * a0 + i as u32 * a1) / 7) as u8;
        }
    } else {
        for i in 1..5 {
            pal[i + 1] = (((5 - i as u32) * a0 + i as u32 * a1) / 5) as u8;
        }
        pal[6] = 0;
        pal[7] = 255;
    }
    let mut bits = 0u64;
    for k in 0..6 {
        bits |= (b[2 + k] as u64) << (8 * k);
    }
    for (i, t) in out.iter_mut().enumerate() {
        *t = pal[((bits >> (3 * i)) & 7) as usize];
    }
}

fn decode_blocks(format: DdsFormat, src: &[u8], w: usize, h: usize, out: &mut [u8]) {
    let bs = format.block_bytes().unwrap();
    let bw = w.div_ceil(4);
    for (bi, b) in src.chunks_exact(bs).enumerate() {
        let (bx, by) = (bi % bw, bi / bw);
        if by * 4 >= h {
            break;
        }
        let mut texels = [[0u8; 4]; 16];
        match format {
            DdsFormat::Bc1 => bc1_block(b, false, &mut texels),
            DdsFormat::Bc2 => {
                bc1_block(&b[8..], true, &mut texels);
                for (i, t) in texels.iter_mut().enumerate() {
                    let nib = (b[i / 2] >> (4 * (i % 2))) & 15;
                    t[3] = nib * 17;
                }
            }
            DdsFormat::Bc3 => {
                bc1_block(&b[8..], true, &mut texels);
                let mut a = [0u8; 16];
                bc4_block(b, &mut a);
                for (t, a) in texels.iter_mut().zip(a) {
                    t[3] = a;
                }
            }
            DdsFormat::Bc4 => {
                let mut r = [0u8; 16];
                bc4_block(b, &mut r);
                for (t, r) in texels.iter_mut().zip(r) {
                    *t = [r, r, r, 255];
                }
            }
            DdsFormat::Bc5 => {
                let (mut r, mut g) = ([0u8; 16], [0u8; 16]);
                bc4_block(b, &mut r);
                bc4_block(&b[8..], &mut g);
                for i in 0..16 {
                    texels[i] = [r[i], g[i], 0, 255];
                }
            }
            _ => unreachable!(),
        }
        for (i, t) in texels.iter().enumerate() {
            let (x, y) = (bx * 4 + i % 4, by * 4 + i / 4);
            if x < w && y < h {
                let o = (y * w + x) * 4;
                out[o..o + 4].copy_from_slice(t);
            }
        }
    }
}

/// IEEE half -> f32.
pub fn half_to_f32(h: u16) -> f32 {
    let sign = ((h >> 15) as u32) << 31;
    let exp = ((h >> 10) & 0x1F) as u32;
    let mant = (h & 0x3FF) as u32;
    let bits = if exp == 0 {
        if mant == 0 {
            sign
        } else {
            let mut e = 127 - 15 + 1;
            let mut m = mant;
            while m & 0x400 == 0 {
                m <<= 1;
                e -= 1;
            }
            sign | ((e as u32) << 23) | ((m & 0x3FF) << 13)
        }
    } else if exp == 31 {
        sign | 0x7F80_0000 | (mant << 13)
    } else {
        sign | ((exp + 127 - 15) << 23) | (mant << 13)
    };
    f32::from_bits(bits)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(w: u32, h: u32, mips: u32, pf_flags: u32, fourcc: &[u8; 4], bits: u32, masks: [u32; 4]) -> Vec<u8> {
        let mut b = vec![0u8; 128];
        b[..4].copy_from_slice(b"DDS ");
        let mut put = |o: usize, v: u32| b[o..o + 4].copy_from_slice(&v.to_le_bytes());
        put(4, 124);
        put(8, 0x1007 | if mips > 1 { 0x2_0000 } else { 0 });
        put(12, h);
        put(16, w);
        put(28, mips);
        put(76, 32);
        put(80, pf_flags);
        put(88, bits);
        for (i, m) in masks.iter().enumerate() {
            put(92 + 4 * i, *m);
        }
        b[84..88].copy_from_slice(fourcc);
        b
    }

    #[test]
    fn dxt1_with_cext_tail() {
        let mut f = header(8, 8, 4, 4, b"DXT1", 0, [0; 4]);
        // 8x8: 4 blocks, 4x4: 1, 2x2: 1, 1x1: 1 = 7 blocks
        for _ in 0..7 {
            f.extend_from_slice(&[0xFF, 0xFF, 0, 0, 0, 0, 0, 0]); // white
        }
        f.extend_from_slice(b"CExtAvgC");
        f.extend_from_slice(&4u32.to_le_bytes());
        f.extend_from_slice(&0x1234u32.to_le_bytes());
        f.extend_from_slice(b"CEnd");
        let d = Dds::parse(&f).unwrap();
        assert_eq!((d.format, d.mips, d.data.len()), (DdsFormat::Bc1, 4, 56));
        assert_eq!(d.average_color, Some(0x1234));
        let px = d.decode_rgba8(0, 0);
        assert_eq!(&px[..4], &[255, 255, 255, 255]);
        let img = dds_to_image(&d, DdsOptions::default()).unwrap();
        assert_eq!(img.texture_descriptor.format, TextureFormat::Bc1RgbaUnormSrgb);
        assert_eq!(img.texture_descriptor.mip_level_count, 4);
    }

    #[test]
    fn bgr8_expands() {
        let mut f = header(2, 1, 1, 0x40, b"\0\0\0\0", 24, [0xFF_0000, 0xFF00, 0xFF, 0]);
        f.extend_from_slice(&[1, 2, 3, 4, 5, 6]);
        let d = Dds::parse(&f).unwrap();
        assert_eq!(d.format, DdsFormat::Bgr8);
        let img = dds_to_image(&d, DdsOptions::linear()).unwrap();
        assert_eq!(img.data.as_deref().unwrap(), &[3, 2, 1, 255, 6, 5, 4, 255]);
    }

    #[test]
    fn truncated_mips_are_dropped() {
        let mut f = header(8, 8, 4, 4, b"DXT5", 0, [0; 4]);
        f.extend_from_slice(&[0u8; 64]); // only mip 0
        let d = Dds::parse(&f).unwrap();
        assert_eq!(d.mips, 1);
        assert!(d.mips_truncated);
    }

    #[test]
    fn bc3_alpha() {
        let mut b = [0u8; 16];
        b[0] = 255; // a0
        b[1] = 0; // a1, indices 0 -> all 255
        let mut a = [0u8; 16];
        bc4_block(&b, &mut a);
        assert!(a.iter().all(|&v| v == 255));
    }

    #[test]
    fn opacity_proof_checks_bc1_selectors_and_every_mip() {
        let mut f = header(4, 4, 3, 4, b"DXT1", 0, [0; 4]);
        // Three-colour mode, selector 0 throughout all three authored mips.
        f.extend_from_slice(&[0; 24]);
        assert!(Dds::parse(&f).unwrap().alpha_is_opaque());
        f[132] = 3;
        assert!(!Dds::parse(&f).unwrap().alpha_is_opaque(), "base cutout, opaque smallest mip");
        f[132] = 0;
        f[148] = 3;
        assert!(!Dds::parse(&f).unwrap().alpha_is_opaque(), "lower mip cutout");
        f[148] = 0;
        f[128] = 1; // four-colour mode makes selector 3 opaque
        f[132] = 255;
        assert!(Dds::parse(&f).unwrap().alpha_is_opaque());
    }

    #[test]
    fn opacity_proof_reads_bc2_and_both_bc3_alpha_palettes() {
        let mut f = header(4, 4, 1, 4, b"DXT3", 0, [0; 4]);
        f.extend_from_slice(&[255; 16]);
        assert!(Dds::parse(&f).unwrap().alpha_is_opaque());
        f[135] = 0xfe;
        assert!(!Dds::parse(&f).unwrap().alpha_is_opaque());
        let mut f = header(4, 4, 1, 4, b"DXT5", 0, [0; 4]);
        f.extend_from_slice(&[0; 16]);
        for (a0, a1, selector, opaque) in [
            (255, 0, 0, true), (255, 0, 1, false), (255, 0, 2, false),
            (0, 255, 1, true), (0, 255, 7, true), (0, 255, 6, false),
            (255, 255, 5, true), (255, 255, 6, false),
        ] {
            f[128] = a0;
            f[129] = a1;
            let bits: u64 = (0..16).map(|i| (selector as u64) << (i * 3)).sum();
            f[130..136].copy_from_slice(&bits.to_le_bytes()[..6]);
            assert_eq!(Dds::parse(&f).unwrap().alpha_is_opaque(), opaque, "{a0}/{a1} selector {selector}");
        }
    }

    #[test]
    fn opacity_proof_declines_unproven_padding_and_missing_mips() {
        let mut f = header(1, 1, 1, 4, b"DXT1", 0, [0; 4]);
        f.extend_from_slice(&[0; 8]);
        assert!(Dds::parse(&f).unwrap().alpha_is_opaque());
        f[132] = 3 << 2; // only a padding texel is transparent: decline conservatively
        assert_eq!(Dds::parse(&f).unwrap().decode_rgba8(0, 0)[3], 255);
        assert!(!Dds::parse(&f).unwrap().alpha_is_opaque());
        let mut f = header(4, 4, 3, 4, b"DXT1", 0, [0; 4]);
        f.extend_from_slice(&[0; 8]);
        let d = Dds::parse(&f).unwrap();
        assert_eq!(d.decode_rgba8(0, 0)[3], 255);
        assert!(d.mips_truncated);
        assert!(!d.alpha_is_opaque());
    }

    #[test]
    fn opacity_proof_distinguishes_explicit_and_implicit_uncompressed_alpha() {
        let mut f = header(2, 1, 1, 0x41, &[0; 4], 32, [0xFF_0000, 0xFF00, 0xFF, 0xFF00_0000]);
        f.extend_from_slice(&[0, 0, 0, 255, 0, 0, 0, 255]);
        assert!(Dds::parse(&f).unwrap().alpha_is_opaque());
        f[135] = 254;
        assert!(!Dds::parse(&f).unwrap().alpha_is_opaque());
        f[80] = 0x40; // X8R8G8B8: unused byte is not alpha
        assert!(Dds::parse(&f).unwrap().alpha_is_opaque());
        let mut f = header(2, 1, 1, 2, &[0; 4], 8, [0; 4]);
        f.extend_from_slice(&[255, 255]);
        assert!(Dds::parse(&f).unwrap().alpha_is_opaque());
        f[129] = 0;
        assert!(!Dds::parse(&f).unwrap().alpha_is_opaque());
    }
}
