//! Original ordinary 2D RGB projector cookies through Bevy's red-channel light textures.

use std::collections::HashMap;
use std::sync::{
    Mutex,
    mpsc::{Receiver, channel},
};

use aa_cry::texture::{Dds, DdsFormat, rgba8_image};
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;

use crate::coords::cry_to_bevy;
use crate::entities::PlacedLight;

// Bevy's conservative binding-array path permits eight textures. Two RGB
// cookies use six; do not depend on partially-bound arrays being available.
const MAX_COOKIES: usize = 2;
const MAX_JOBS: usize = 2;

#[derive(Clone, Copy, Debug)]
pub(super) struct Projection {
    pub rotation: Quat,
    pub xy_scale: f32,
    pub inner_angle: f32,
    pub outer_angle: f32,
}

/// Native 381A8AD0 normalizes matrix columns and projects along local +X.
/// Its cookie basis has opposite handedness to a Bevy camera; mirror the image U.
pub(super) fn projection(p: &PlacedLight) -> Option<Projection> {
    if p.projector_texture.is_empty() || p.project_all_dirs || p.ambient || p.negative || p.fake
        || p.clip_bounds || p.cast_shadow || p.style != 0 || p.hdr_dynamic != 0.0
        // Positive authored near-plane clipping needs a custom light receiver.
        || p.projector_near != 0.0 || p.diffuse.min_element() < 0.0
        || !(0.0..180.0).contains(&p.projector_fov) || p.projector_fov == 0.0
    {
        return None;
    }
    let x = p.matrix.x_axis.truncate().try_normalize()?;
    let y = p.matrix.y_axis.truncate().try_normalize()?;
    let z = p.matrix.z_axis.truncate().try_normalize()?;
    if x.dot(y).abs().max(x.dot(z).abs()).max(y.dot(z).abs()) > 1e-4 || x.cross(y).dot(z) < 0.9999 {
        return None; // A sheared/reflected native basis cannot be represented by a quaternion.
    }
    let tan_half = (p.projector_fov.to_radians() * 0.5).tan();
    // Circumscribe the native square. Keep the complete square inside Bevy's
    // constant inner cone; its 1e-4 denominator guard supplies the outer margin.
    let inner_angle = (std::f32::consts::SQRT_2 * tan_half).atan();
    let cos_outer = inner_angle.cos() - 1e-4;
    if cos_outer <= 0.0 {
        return None;
    }
    let outer_angle = cos_outer.acos();
    Some(Projection {
        rotation: Quat::from_mat3(&Mat3::from_cols(
            cry_to_bevy(z),
            cry_to_bevy(y),
            -cry_to_bevy(x),
        )),
        xy_scale: tan_half / outer_angle.tan(),
        inner_angle,
        outer_angle,
    })
}

/// Native projector attenuation is 1-d²/r². Bevy remains inverse-square; match
/// at d=r/2, where (3/4) / ((15/16)²/(r²/4)) = (16/75)*r².
pub(super) fn lumens(radius: f32, lux_per_unit: f32) -> f32 {
    4.0 * std::f32::consts::PI * (16.0 / 75.0) * radius * radius * lux_per_unit
}

fn channels(width: u32, height: u32, rgba: &[u8], srgb: bool) -> Option<[Image; 3]> {
    if width < 2
        || height < 2
        || width > 1024
        || height > 1024
        || width as usize * height as usize > 512 * 512
        || rgba.len() != width as usize * height as usize * 4
    {
        return None;
    }
    // Clamp-to-edge then produces exactly zero outside the native square.
    let border = |x: u32, y: u32| {
        let i = ((y * width + x) * 4) as usize;
        rgba[i..i + 3] == [0, 0, 0]
    };
    if !(0..width).all(|x| border(x, 0) && border(x, height - 1))
        || !(0..height).all(|y| border(0, y) && border(width - 1, y))
    {
        return None;
    }
    Some(std::array::from_fn(|channel| {
        let mut pixels = vec![0; rgba.len()];
        for y in 0..height {
            for x in 0..width {
                let source = ((y * width + (width - 1 - x)) * 4) as usize;
                let target = ((y * width + x) * 4) as usize;
                let value = rgba[source + channel];
                pixels[target..target + 4].copy_from_slice(&[value, value, value, 255]);
            }
        }
        // Preserve encoded bytes and the native CRYF sRGB-read flag. Hardware
        // decodes tagged channels BEFORE bilinear interpolation.
        let mut image = rgba8_image(width, height, &[pixels], srgb);
        image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::ClampToEdge,
            address_mode_v: ImageAddressMode::ClampToEdge,
            address_mode_w: ImageAddressMode::ClampToEdge,
            mag_filter: ImageFilterMode::Linear,
            min_filter: ImageFilterMode::Linear,
            mipmap_filter: ImageFilterMode::Linear,
            ..default()
        });
        image
    }))
}

fn prepare(path: &str) -> Option<[Image; 3]> {
    let bytes = aa_pak::Pak::global().read(path).ok()?;
    let dds = Dds::parse(&bytes).ok()?;
    // These colour formats support the native CRYF-controlled sRGB view.
    if dds.faces != 1
        || dds.depth != 1
        || !matches!(dds.format, DdsFormat::Bc1 | DdsFormat::Bc2 | DdsFormat::Bc3)
        || dds.width > 1024
        || dds.height > 1024
        || u64::from(dds.width) * u64::from(dds.height) > 512 * 512
    {
        return None;
    }
    let u32_at = |offset| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
    let flags = u32_at(36);
    if flags & 0x1000000 != 0 || path.contains("_ddn") {
        return None; // The native normal-map image path has different color semantics.
    }
    let srgb = u32_at(4) >= 124 && u32_at(124) == 0x43525946 && flags & 0x800 != 0;
    channels(dds.width, dds.height, &dds.decode_rgba8(0, 0), srgb)
}

enum Cookie {
    Loading,
    Ready([Handle<Image>; 3]),
    Unsupported,
}

struct Pending {
    epoch: u64,
    path: String,
    rx: Mutex<Receiver<Option<[Image; 3]>>>,
}

#[derive(Resource, Default)]
pub(super) struct Cookies {
    world: String,
    epoch: u64,
    entries: HashMap<String, Cookie>,
    pending: Vec<Pending>,
}

impl Cookies {
    /// Called at the light selector's 5 Hz cadence. Retain at most two nearby
    /// assets, submit at most two jobs, and drain old-world jobs before reusing slots.
    pub fn update<'a>(
        &mut self,
        world: &str,
        paths: impl Iterator<Item = &'a str>,
        images: &mut Assets<Image>,
    ) {
        if self.world != world {
            self.world = world.to_string();
            self.epoch = self.epoch.wrapping_add(1);
            self.entries.clear();
        }
        let mut wanted = Vec::new();
        for path in paths {
            if !wanted.contains(&path) {
                wanted.push(path);
                if wanted.len() == MAX_COOKIES {
                    break;
                }
            }
        }
        self.entries
            .retain(|path, _| wanted.contains(&path.as_str()));
        self.pending.retain(|job| {
            let result = job.rx.lock().unwrap().try_recv();
            match result {
                Err(std::sync::mpsc::TryRecvError::Empty) => true,
                result => {
                    if job.epoch == self.epoch && wanted.contains(&job.path.as_str()) {
                        let ready = match result.ok().flatten() {
                            Some(channels) => {
                                Cookie::Ready(channels.map(|image| images.add(image)))
                            }
                            None => Cookie::Unsupported,
                        };
                        self.entries.insert(job.path.clone(), ready);
                    }
                    false
                }
            }
        });
        for path in wanted {
            if self.entries.contains_key(path) {
                continue;
            }
            if self
                .pending
                .iter()
                .any(|job| job.epoch == self.epoch && job.path == path)
            {
                self.entries.insert(path.to_string(), Cookie::Loading);
                continue;
            }
            if self.pending.len() >= MAX_JOBS {
                break;
            }
            let (tx, rx) = channel();
            let path = path.to_string();
            let task_path = path.clone();
            aa_cry::workers::Workers::global().spawn(0.0, move || {
                let _ = tx.send(prepare(&task_path));
            });
            self.pending.push(Pending {
                epoch: self.epoch,
                path: path.clone(),
                rx: Mutex::new(rx),
            });
            self.entries.insert(path, Cookie::Loading);
        }
    }

    pub fn get(&self, path: &str) -> Option<&[Handle<Image>; 3]> {
        match self.entries.get(path)? {
            Cookie::Ready(images) => Some(images),
            _ => None,
        }
    }

    pub fn unsupported(&self, path: &str) -> bool {
        matches!(self.entries.get(path), Some(Cookie::Unsupported))
    }

    pub fn counts(&self) -> (usize, usize) {
        (
            self.entries
                .values()
                .filter(|v| matches!(v, Cookie::Ready(_)))
                .count(),
            self.pending.len(),
        )
    }
}

#[derive(Component)]
pub(super) struct Channels(pub [Entity; 3]);

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::render::render_resource::TextureFormat;

    fn placed() -> PlacedLight {
        crate::entities::parse_lights(r#"<Mission><Objects><Entity EntityClass="Light"><Properties Radius="15"><Projector fProjectorFov="62.2" texture_Texture="cookie.dds"/></Properties></Entity></Objects></Mission>"#).remove(0)
    }

    #[test]
    fn native_matrix_goldens_match_mirrored_bevy_uv_and_constant_square_cone() {
        let mut p = placed();
        let native_tan = (31.1_f32.to_radians()).tan();
        // Original 381A9CB0 bytes, x86 Unicorn; imported tanf alone used host libm.
        let samples = [
            (Vec3::new(5.0, 0.0, 0.0), Vec2::splat(0.5)),
            (Vec3::new(5.0, 1.0, 0.0), Vec2::new(0.5, 0.3342280984)),
            (Vec3::new(5.0, 0.0, 1.0), Vec2::new(0.3342280984, 0.5)),
            (Vec3::new(5.0, 2.0, 2.0), Vec2::splat(0.1684561968)),
            (Vec3::new(5.0, -2.0, -2.0), Vec2::splat(0.8315438032)),
        ];
        for scale in [Vec3::ONE, Vec3::new(2.0, 3.0, 4.0)] {
            p.matrix = Mat4::from_scale(scale);
            let q = projection(&p).unwrap();
            let transform = Mat4::from_scale_rotation_translation(
                Vec3::new(q.xy_scale, q.xy_scale, 1.0),
                q.rotation,
                Vec3::ZERO,
            );
            for (native, expected) in samples {
                let local = transform.inverse().transform_point3(cry_to_bevy(native));
                let bevy_uv = local.truncate() / (local.z * q.outer_angle.tan())
                    * Vec2::new(-0.5, 0.5)
                    + Vec2::splat(0.5);
                assert!((Vec2::new(1.0 - bevy_uv.x, bevy_uv.y) - expected).length() < 2e-6);
            }
            let corner = Vec3::new(1.0, native_tan, native_tan);
            let cd = corner.x / corner.length();
            let angular = ((cd - q.outer_angle.cos())
                / (q.inner_angle.cos() - q.outer_angle.cos()).max(1e-4))
            .clamp(0.0, 1.0);
            assert!(
                angular > 0.999,
                "the square's corners must not be vignetted"
            );
        }
        p.matrix = Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0));
        assert!(projection(&p).is_none());
        p.matrix = Mat4::IDENTITY;
        p.cast_shadow = true;
        assert!(
            projection(&p).is_none(),
            "never shadow only one RGB component"
        );
        p.cast_shadow = false;
        p.projector_near = 1.0;
        assert!(
            projection(&p).is_none(),
            "positive near clipping needs a custom receiver"
        );
    }

    #[test]
    fn channel_images_preserve_srgb_bytes_mirror_u_and_reject_nonblack_border() {
        let mut rgba = vec![0; 4 * 4 * 4];
        rgba[(1 * 4 + 1) * 4..(1 * 4 + 1) * 4 + 4].copy_from_slice(&[128, 64, 32, 0]);
        let result = channels(4, 4, &rgba, true).unwrap();
        for (index, value) in [128, 64, 32].into_iter().enumerate() {
            assert_eq!(
                result[index].texture_descriptor.format,
                TextureFormat::Rgba8UnormSrgb
            );
            let data = result[index].data.as_ref().unwrap();
            assert_eq!(
                &data[(1 * 4 + 2) * 4..(1 * 4 + 2) * 4 + 4],
                &[value, value, value, 255]
            );
        }
        assert_eq!(
            channels(4, 4, &rgba, false).unwrap()[0]
                .texture_descriptor
                .format,
            TextureFormat::Rgba8Unorm
        );
        rgba[0] = 1;
        assert!(channels(4, 4, &rgba, true).is_none());
    }

    #[test]
    fn rgb_components_sum_without_a_threefold_power_or_normalization_change() {
        let color = Vec3::new(0.0196078, 0.705882, 0.54902) * 31.3;
        let filter = Vec3::new(0.2158605, 0.05126946, 0.014443844); // sRGB bytes 128,64,32
        let native = color * filter;
        let split: Vec3 = (0..3)
            .map(|i| {
                let mut component = Vec3::ZERO;
                component[i] = color[i] * filter[i];
                component
            })
            .sum();
        assert_eq!(split, native);
        let r = 15.0;
        let d = r * 0.5;
        let q: f32 = d * d / (r * r);
        let bevy = lumens(r, 1.0) / (4.0 * std::f32::consts::PI) * (1.0 - q * q).powi(2) / (d * d);
        assert!((bevy - (1.0 - q)).abs() < 1e-6);
    }

    #[test]
    fn original_library_cookie_is_rgb_black_bordered_and_supported() {
        if !crate::pak_available() {
            return;
        }
        let path = "objects/env/08_dungeon/05_libray/light_a_df.dds";
        let images = prepare(path).unwrap();
        assert_eq!(images[0].width(), 256);
        assert_eq!(images[0].height(), 512);
        assert_eq!(
            images[0].texture_descriptor.format,
            TextureFormat::Rgba8UnormSrgb
        );
        assert_ne!(images[0].data, images[1].data);
        let cell = crate::placement::load_cell_objects("instance_library", 0, 4);
        let light = cell.lights.iter().find(|p| p.name == "Light_587").unwrap();
        assert_eq!(light.guid, 0x491a724ff2eb4d01);
        assert_eq!(light.projector_texture, path);
        assert_eq!(light.projector_fov, 62.2);
        assert_eq!(light.projector_near, 0.0);
        let q = projection(light).unwrap();
        let inverse = Mat4::from_scale_rotation_translation(
            Vec3::new(q.xy_scale, q.xy_scale, 1.0),
            q.rotation,
            cry_to_bevy(light.matrix.w_axis.truncate()),
        )
        .inverse();
        // Native matrix oracle with the original authored rotation and position.
        for (point, expected) in [
            (
                Vec3::new(871.999509, 102.795236, 257.967327),
                Vec2::new(0.49999870, 0.50000488),
            ),
            (
                Vec3::new(871.999509, 103.561281, 258.610114),
                Vec2::new(0.49999871, 0.33423306),
            ),
            (
                Vec3::new(872.999509, 102.795236, 257.967327),
                Vec2::new(0.33422687, 0.50000489),
            ),
            (
                Vec3::new(873.999509, 104.327325, 259.252901),
                Vec2::new(0.16845504, 0.16846125),
            ),
        ] {
            let local = inverse.transform_point3(cry_to_bevy(point));
            let bevy_uv = local.truncate() / (local.z * q.outer_angle.tan()) * Vec2::new(-0.5, 0.5)
                + Vec2::splat(0.5);
            assert!((Vec2::new(1.0 - bevy_uv.x, bevy_uv.y) - expected).length() < 3e-5);
        }
    }

    #[test]
    fn world_switch_keeps_inflight_budget_until_drained_and_discards_stale_images() {
        let mut cache = Cookies::default();
        let mut images = Assets::<Image>::default();
        let mut senders = Vec::new();
        for index in 0..MAX_JOBS {
            let (tx, rx) = channel();
            senders.push(tx);
            cache.pending.push(Pending {
                epoch: 0,
                path: format!("old{index}.dds"),
                rx: Mutex::new(rx),
            });
        }
        cache.update("new", ["new.dds"].into_iter(), &mut images);
        assert_eq!(
            cache.pending.len(),
            MAX_JOBS,
            "world switches cannot flood the shared workers"
        );
        let rgba = vec![0; 4 * 4 * 4];
        for tx in senders {
            tx.send(channels(4, 4, &rgba, true)).unwrap();
        }
        cache.update("new", std::iter::empty(), &mut images);
        assert_eq!(cache.pending.len(), 0);
        assert_eq!(cache.entries.len(), 0);
        assert!(
            images.is_empty(),
            "old-world worker results must not be uploaded"
        );
    }
}
