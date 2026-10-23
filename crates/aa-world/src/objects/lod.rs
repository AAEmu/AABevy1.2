//! Ordinary brush requested LOD: compiled byte37 and Cry3DEngine1.2 300FFC60/30112240.
//! Mesh usability/quality overrides remain the existing aa-cry picker policy.

use crate::coords::{bevy_to_cry, cell_origin};
use crate::placement::Brush;
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;

/// Maximum completed-brush checks per frame; loading and model work keep their existing budgets.
pub const MAX_CHECKS: usize = 512;
const MAX_LOD: u8 = 6; // Native constructor e_lod_max; the asset picker supports 0..=6.

/// Requested level of a drawn brush part, before the asset picker's missing-LOD fallback.
#[derive(Component, Clone, Copy, Debug)]
pub struct BrushLodPart {
    pub cell: (i32, i32),
    pub index: u32,
    pub requested: u8,
}

#[derive(Clone, Copy)]
pub(super) struct View {
    position: Vec3,
    scale: f32,
}

pub(super) fn select_view<'a>(
    cameras: impl Iterator<
        Item = (
            &'a Camera,
            &'a GlobalTransform,
            &'a Projection,
            Option<&'a RenderLayers>,
        ),
    >,
) -> Option<View> {
    let mut cameras = cameras.filter(|(c, _, _, layers)| {
        c.is_active && layers.is_none_or(|layers| layers.intersects(&RenderLayers::default()))
    });
    let (_, transform, projection, _) = cameras.next()?;
    if cameras.next().is_some() {
        return None;
    }
    match projection {
        Projection::Perspective(p) => View::new(transform.translation(), p.fov),
        _ => None,
    }
}

impl View {
    pub(super) fn narrow(self) -> bool { self.scale < 0.7 }
    pub(super) fn new(position: Vec3, fov: f32) -> Option<Self> {
        if !position.is_finite()
            || !fov.is_finite()
            || !(0.0..std::f32::consts::PI).contains(&fov)
            || fov == 0.0
        {
            return None;
        }
        // Normal-view upload30151BE0; keep the native radians->degrees and multiply order.
        let scale = ((fov * 57.295776 * 0.016666668) * 0.8) + 0.2;
        Some(Self {
            position: bevy_to_cry(position),
            scale,
        })
    }

    pub(super) fn requested(self, brush: &Brush, cell: (i32, i32)) -> Option<u8> {
        let (distance, radius) = self.distance(brush, cell)?;
        Some(source_lod(true, distance, brush.max_view_dist, brush.lod_ratio, radius))
    }

    pub(super) fn distance(self, brush: &Brush, cell: (i32, i32)) -> Option<(f32, f32)> {
        let origin = cell_origin(cell.0, cell.1);
        let lo = Vec3::from_slice(&brush.bbox[..3]) + origin;
        let hi = Vec3::from_slice(&brush.bbox[3..]) + origin;
        if !lo.is_finite()
            || !hi.is_finite()
            || !lo.cmple(hi).all()
            || !brush.max_view_dist.is_finite()
        {
            return None;
        }
        let delta = (lo - self.position).max(Vec3::ZERO).max(self.position - hi);
        let distance = delta.length() * self.scale;
        let radius = (hi - lo).length() * 0.5;
        if !distance.is_finite()
            || !radius.is_finite()
            || (brush.max_view_dist < 0.001 && radius <= 0.0)
        {
            return None;
        }
        Some((distance, radius))
    }
}

fn source_lod(sync: bool, distance: f32, view: f32, ratio: u8, radius: f32) -> u8 {
    // At q=0 native RSQRTSS produces NaN -> INT_MIN, subsequently clamped to near LOD0.
    if distance <= 0.0 {
        return 0;
    }
    let ratio = f32::from(ratio) * 0.01;
    let value = if !sync || view < 0.001 {
        ((ratio * ratio) * distance) / (6.0 * radius)
    } else {
        let q = distance / view;
        let square = q * q;
        let root = source_sqrt(q);
        (((root - square) * (ratio * 0.39215687)) + square) * 4.0
    };
    value.floor().clamp(0.0, f32::from(MAX_LOD)) as u8
}

fn source_sqrt(q: f32) -> f32 {
    // The original uses RSQRTSS plus this Newton step, rather than SQRTSS. At exact
    // integer thresholds its processor approximation can select the adjacent level.
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::{_mm_cvtss_f32, _mm_rsqrt_ss, _mm_set_ss};
        // SSE is guaranteed on x86_64; this instruction has no memory operand.
        let r = unsafe { _mm_cvtss_f32(_mm_rsqrt_ss(_mm_set_ss(q))) };
        ((1.5 - (((r * q) * r) * 0.5)) * r) * q
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        q.sqrt()
    }
}

/// Frozen build snapshot plus the next bounded scan. Vectors grow only as the
/// existing time-budgeted spawner admits instances, never for distant cells.
#[derive(Default)]
pub(super) struct BrushLods {
    pub current: Vec<u8>,
    requested: Vec<u8>,
    cursor: usize,
    dirty: usize,
}

impl BrushLods {
    pub(super) fn retain(&mut self, index: usize, level: u8) {
        self.dirty -= usize::from(self.requested[index] != self.current[index]);
        self.current[index] = level;
        self.requested[index] = level;
    }
    pub fn initial(&mut self, index: usize, requested: u8) -> u8 {
        if index == self.current.len() {
            self.current.push(requested);
            self.requested.push(requested);
        }
        self.current[index]
    }

    /// Returns (number checked, completed scan with changed levels).
    pub fn scan(&mut self, limit: usize, mut level: impl FnMut(usize) -> u8) -> (usize, bool) {
        let count = limit.min(self.current.len());
        let mut wrapped = false;
        for _ in 0..count {
            let i = self.cursor;
            self.dirty -= usize::from(self.requested[i] != self.current[i]);
            self.requested[i] = level(i);
            self.dirty += usize::from(self.requested[i] != self.current[i]);
            self.cursor += 1;
            if self.cursor == self.current.len() {
                self.cursor = 0;
                wrapped = true;
            }
        }
        (count, wrapped && self.dirty != 0)
    }

    /// Start one atomic sector replacement. No allocation after initial admission;
    /// the current geometry remains visible until the frozen replacement is ready.
    pub fn snapshot(&mut self) {
        self.current.copy_from_slice(&self.requested);
        self.dirty = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_scan_finishes_large_sector_and_snapshot_does_not_restart() {
        let mut levels = BrushLods::default();
        for i in 0..2000 {
            levels.initial(i, 0);
        }
        let capacities = (levels.current.capacity(), levels.requested.capacity());
        for _ in 0..3 {
            assert_eq!(levels.scan(MAX_CHECKS, |_| 3), (512, false));
        }
        assert_eq!(levels.scan(MAX_CHECKS, |_| 3), (512, true));
        assert!(
            levels.current.iter().all(|&v| v == 0),
            "visible snapshot stays fixed while scanning"
        );
        levels.snapshot();
        assert!(levels.current.iter().all(|&v| v == 3));
        assert_eq!(
            (levels.current.capacity(), levels.requested.capacity()),
            capacities
        );
        assert_eq!(levels.scan(MAX_CHECKS, |_| 3), (512, false));
        // A changed request that returns to the current level must not force a rebuild.
        levels.cursor = 0;
        levels.scan(1, |_| 4);
        levels.cursor = 0;
        assert_eq!(levels.scan(2000, |_| 3), (2000, false));
    }

    #[test]
    fn native_fov_world_aabb_distance_and_cell_origin_once() {
        let brush = Brush {
            matrix: [0.0; 12],
            model: 0,
            material: -1,
            flags: 0,
            max_view_dist: 1000.0,
            lod_ratio: 255,
            bbox: [10.0, 20.0, 30.0, 30.0, 60.0, 50.0],
        };
        let origin = cell_origin(9, 12);
        let view = |p, fov: f32| {
            View::new(crate::coords::cry_to_bevy(origin + p), fov.to_radians()).unwrap()
        };
        assert_eq!(
            view(Vec3::new(20.0, 40.0, 40.0), 60.0).requested(&brush, (9, 12)),
            Some(0)
        );
        assert_eq!(
            view(Vec3::new(230.0, 40.0, 40.0), 60.0).requested(&brush, (9, 12)),
            Some(1)
        );
        assert_eq!(
            view(Vec3::new(230.0, 40.0, 40.0), 100.0).requested(&brush, (9, 12)),
            Some(2)
        );
        assert!(View::new(Vec3::ZERO, 0.0).is_none());
        assert!(View::new(Vec3::ZERO, f32::NAN).is_none());
        let mut invalid = brush;
        invalid.bbox[0] = f32::NAN;
        assert!(
            view(Vec3::ZERO, 60.0)
                .requested(&invalid, (9, 12))
                .is_none()
        );
    }

    #[test]
    fn exactly_one_active_world_perspective_view_excludes_portraits() {
        let active = Camera::default();
        let inactive = Camera {
            is_active: false,
            ..default()
        };
        let transform = GlobalTransform::IDENTITY;
        let projection = Projection::Perspective(PerspectiveProjection::default());
        let portrait = RenderLayers::layer(5);
        let world = (&active, &transform, &projection, None);
        assert!(select_view([world].into_iter()).is_some());
        assert!(
            select_view(
                [
                    world,
                    (&active, &transform, &projection, Some(&portrait)),
                    (&inactive, &transform, &projection, None)
                ]
                .into_iter()
            )
            .is_some()
        );
        assert!(select_view([world, world].into_iter()).is_none());
        assert!(
            select_view(
                [(
                    &active,
                    &transform,
                    &Projection::Orthographic(OrthographicProjection::default_3d()),
                    None
                )]
                .into_iter()
            )
            .is_none()
        );
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn original_instruction_oracle_with_hardware_rsqrt_all208_controls() {
        // Cry3DEngine30112240 original instruction execution. Unicorn RSQRTSS
        // is replaced with this machine's actual hardware instruction result;
        // every later original SSE instruction/floor/conversion executes unchanged.
        let distances = [
            0.0, 0.1, 10.0, 62.5, 125.0, 200.0, 250.0, 300.0, 500.0, 750.0, 1000.0, 1200.0, 1500.0,
        ];
        let rows: [(u8, u8, [u8; 13]); 16] = [
            (0, 0, [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
            (0, 40, [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1]),
            (0, 50, [0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 2, 2]),
            (0, 100, [0, 0, 0, 0, 0, 1, 1, 2, 3, 5, 6, 6, 6]),
            (0, 120, [0, 0, 0, 0, 1, 1, 2, 2, 4, 6, 6, 6, 6]),
            (0, 150, [0, 0, 0, 0, 1, 3, 3, 4, 6, 6, 6, 6, 6]),
            (0, 200, [0, 0, 0, 1, 3, 5, 6, 6, 6, 6, 6, 6, 6]),
            (0, 255, [0, 0, 0, 2, 5, 6, 6, 6, 6, 6, 6, 6, 6]),
            (1, 0, [0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 4, 5, 6]),
            (1, 40, [0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 4, 5, 6]),
            (1, 50, [0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 4, 5, 6]),
            (1, 100, [0, 0, 0, 0, 0, 0, 0, 1, 1, 2, 4, 5, 6]),
            (1, 120, [0, 0, 0, 0, 0, 0, 1, 1, 1, 2, 4, 5, 6]),
            (1, 150, [0, 0, 0, 0, 0, 1, 1, 1, 2, 2, 4, 4, 6]),
            (1, 200, [0, 0, 0, 0, 1, 1, 1, 1, 2, 3, 4, 4, 5]),
            (1, 255, [0, 0, 0, 1, 1, 1, 2, 2, 2, 3, 4, 4, 4]),
        ];
        for (sync, ratio, expected) in rows {
            for (distance, expected) in distances.into_iter().zip(expected) {
                assert_eq!(
                    source_lod(sync != 0, distance, 1000.0, ratio, 25.0),
                    expected,
                    "sync={sync} ratio={ratio} distance={distance}"
                );
            }
        }
        // Degenerate maxview takes the same source radius branch; no guessed distance bands.
        assert_eq!(source_lod(true, 300.0, 0.0, 100, 25.0), 2);
        assert_eq!(source_lod(true, 300.0, 0.0001, 100, 25.0), 2);
        for ratio in 0..=255 {
            for distance in [0.0, 0.1, 100.0, 10000.0] {
                assert!(source_lod(true, distance, 1000.0, ratio, 25.0) <= MAX_LOD);
            }
        }
    }
}
