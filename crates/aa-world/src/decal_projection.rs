//! Native 1.2 deferred decal projection: client 30056FD0 / 30058040, renderer 380444C0.

use bevy::prelude::*;
use bevy::render::render_resource::ShaderType;

use crate::coords::{CELL_SIZE, cry_to_bevy};
use crate::placement::Decal;

#[derive(Clone, Copy, Debug, Default, Reflect, ShaderType)]
pub(crate) struct DeferredParams {
    /// World centre; w = 0 disabled, 1 planar deferred, 2 wrapped, 3 fake wrapped.
    pub center: Vec4,
    /// Unit native X / Y axes in Bevy space; w = inverse physical half extent.
    pub right: Vec4,
    pub up: Vec4,
    /// Unit native Z axis in Bevy space; w = source material opacity.
    pub front: Vec4,
    /// Front distance, inverse total depth, mesh surface offset, source alpha reference.
    pub depth: Vec4,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct DeferredProjection {
    pub center: Vec3,
    /// Inverse g_mLightProj unit-box centre / half axes, including asymmetric depth.
    pub volume_center: Vec3,
    pub volume: Mat3,
    axes: Mat3,
    extents: Vec3,
    front: f32,
    back: f32,
    mode: u8,
}

impl DeferredProjection {
    pub fn new(decal: &Decal) -> Option<Self> {
        if !decal.pos.is_finite()
            || !decal.basis.is_finite()
            || !decal.radius.is_finite()
            || !(0.01..=128.0).contains(&decal.radius)
            || decal.basis.determinant().abs() < 1e-6
            || decal.depth.iter().any(|v| !v.is_finite() || *v <= 0.0)
        {
            return None;
        }
        // The producer scales Z by max(depths); the renderer then multiplies both unchanged
        // depth fractions by that Z-column length. Do not remove this second multiplication.
        let producer = decal.basis
            * Mat3::from_diagonal(Vec3::new(
                decal.radius,
                decal.radius,
                decal.radius * decal.depth[0].max(decal.depth[1]),
            ));
        let extents = Vec3::new(
            producer.x_axis.length(),
            producer.y_axis.length(),
            producer.z_axis.length(),
        );
        if !extents.is_finite() || extents.min_element() <= 0.0 {
            return None;
        }
        let axes = Mat3::from_cols(
            producer.x_axis / extents.x,
            producer.y_axis / extents.y,
            producer.z_axis / extents.z,
        );
        if axes.determinant().abs() < 1e-6 {
            return None;
        }
        let front = decal.depth[0] * extents.z;
        let back = decal.depth[1] * extents.z;
        let inverse_distance = axes.transpose().inverse();
        // 38044070 uploads the inverse projection to DecalVolumeVS; box producer 381672A0
        // emits XYZ corners 0..1. Its Z range is [-back, front], not symmetric producer Z.
        let volume_center = decal.pos + inverse_distance * Vec3::new(0.0, 0.0, (front - back) * 0.5);
        let volume = inverse_distance * Mat3::from_diagonal(Vec3::new(extents.x, extents.y, (front + back) * 0.5));
        let bounds = volume.x_axis.abs() + volume.y_axis.abs() + volume.z_axis.abs();
        if !volume_center.is_finite() || !bounds.is_finite() || bounds.max_element() > CELL_SIZE {
            return None;
        }
        Some(Self {
            center: decal.pos,
            volume_center,
            volume,
            axes,
            extents,
            front,
            back,
            mode: match decal.projection {
                4 | 6 => 2,
                5 | 7 => 3,
                _ => 1,
            },
        })
    }

    pub fn params(self, origin: Vec3, surface_offset: f32) -> DeferredParams {
        DeferredParams {
            center: cry_to_bevy(self.center + origin).extend(self.mode as f32),
            right: cry_to_bevy(self.axes.x_axis).extend(self.extents.x.recip()),
            up: cry_to_bevy(self.axes.y_axis).extend(self.extents.y.recip()),
            front: cry_to_bevy(self.axes.z_axis).extend(1.0),
            depth: Vec4::new(self.front, (self.front + self.back).recip(), surface_offset, 0.0),
        }
    }

    /// Vertex UVs / tangent construction. The shader evaluates the same native projection per
    /// fragment, including the receiver normal, so wrapping and depth fade are not interpolated.
    pub fn uv(self, position: Vec3, normal: Vec3) -> Vec2 {
        let local = self.axes.transpose() * (position - self.center);
        if self.mode == 1 {
            return Vec2::new(local.x / self.extents.x, -local.y / self.extents.y) * 0.5 + Vec2::splat(0.5);
        }
        let n = self.axes.transpose() * normal.normalize_or_zero();
        let mut offset = n.truncate() * local.z;
        if self.mode == 3 {
            // Preserve the authored shader's exact axis/local-normal dot product.
            let cos = self.axes.z_axis.dot(n);
            offset *= (1.0 - cos * cos).sqrt().powi(8);
        }
        ((local.truncate() - offset) * Vec2::new(self.extents.y.recip(), -self.extents.x.recip()) * 0.5
            + Vec2::splat(0.5))
        .clamp(Vec2::ZERO, Vec2::ONE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decal() -> Decal {
        Decal {
            bbox: [0.0; 6],
            max_view_dist: 100.0,
            projection: 0,
            deferred: true,
            depth: [0.25, 0.5],
            pos: Vec3::new(10.0, 20.0, 30.0),
            normal: Vec3::Z,
            basis: Mat3::from_rotation_z(0.7),
            radius: 4.0,
            material: 0,
            sort_priority: 0,
            raw: Vec::new(),
        }
    }

    #[test]
    fn native_depth_units_and_rotated_uvs() {
        let d = decal();
        let p = DeferredProjection::new(&d).unwrap();
        assert!((p.extents - Vec3::new(4.0, 4.0, 2.0)).length() < 1e-5);
        assert!((p.volume_center - (d.pos - Vec3::Z * 0.25)).length() < 1e-5);
        assert!((p.volume.z_axis - Vec3::Z * 0.75).length() < 1e-5);
        let params = p.params(Vec3::new(1024.0, 2048.0, 0.0), 0.005);
        assert!((params.depth.x - 0.5).abs() < 1e-6 && (params.depth.y - 2.0 / 3.0).abs() < 1e-6);
        assert_eq!(
            params.center.truncate(),
            cry_to_bevy(d.pos + Vec3::new(1024.0, 2048.0, 0.0))
        );
        assert!((p.uv(d.pos + p.axes.x_axis * 4.0, Vec3::Z) - Vec2::new(1.0, 0.5)).length() < 1e-5);
        assert!((p.uv(d.pos + p.axes.y_axis * 4.0, Vec3::Z) - Vec2::new(0.5, 0.0)).length() < 1e-5);
    }

    #[test]
    fn native_wrapped_and_fake_wrapped_normal_offsets() {
        let mut d = decal();
        let normal = (d.basis.x_axis + Vec3::Z).normalize();
        let point = d.pos + Vec3::Z;
        d.projection = 4;
        let wrapped = DeferredProjection::new(&d).unwrap().uv(point, normal);
        assert!((wrapped.x - (0.5 - std::f32::consts::FRAC_1_SQRT_2 / 8.0)).abs() < 1e-6);
        d.projection = 5;
        let fake = DeferredProjection::new(&d).unwrap().uv(point, normal);
        assert!((fake.x - (0.5 - std::f32::consts::FRAC_1_SQRT_2 / 128.0)).abs() < 1e-6);
    }

    #[test]
    fn asymmetric_native_volume_matches_fragment_projection_with_scaled_axes() {
        let mut d = decal();
        d.basis = Mat3::from_rotation_z(0.7)
            * Mat3::from_cols(Vec3::X * 2.0, Vec3::new(0.3, 1.0, 0.0), Vec3::new(0.2, 0.1, 1.0));
        let p = DeferredProjection::new(&d).unwrap();
        let params = p.params(Vec3::ZERO, 0.0);
        // Original 381672A0 unit-box corners transformed by inverse g_mLightProj must
        // return exactly the shader's XYZ 0..1, including non-unit/non-orthogonal axes.
        for x in [-1.0, 1.0] {
            for y in [-1.0, 1.0] {
                for z in [-1.0, 1.0] {
                    let point = cry_to_bevy(p.volume_center + p.volume * Vec3::new(x, y, z));
                    let relative = point - params.center.truncate();
                    let projected = Vec3::new(
                        0.5 + relative.dot(params.right.truncate()) * params.right.w * 0.5,
                        0.5 - relative.dot(params.up.truncate()) * params.up.w * 0.5,
                        (params.depth.x - relative.dot(params.front.truncate())) * params.depth.y,
                    );
                    let expected = Vec3::new((x + 1.0) * 0.5, (1.0 - y) * 0.5, (1.0 - z) * 0.5);
                    assert!((projected - expected).length() < 1e-5, "{projected} != {expected}");
                }
            }
        }
    }
}
