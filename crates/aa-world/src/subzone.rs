//! Native Group-18 spatial queries over an explicitly supplied live area set.
//!
//! This module does not discover or load world areas. The caller must supply the
//! engine registration order, live entity/proxy resolution and seamless origin.

use bevy::math::{IVec2, Vec2, Vec3};

/// Polygon form of native CArea, in seamless-local Cry coordinates.
#[derive(Clone, Debug)]
pub struct NativeSubZonePolygon {
    min: Vec3,
    max: Vec3,
    height: f32,
    segments: Vec<Segment>,
}

#[derive(Clone, Debug)]
struct Segment {
    horizontal: bool,
    min_y: f32,
    max_y: f32,
    max_x: f32,
    slope: f32,
    intercept: f32,
}

impl NativeSubZonePolygon {
    /// Prepares already-transformed native-local vertices and the raw authored height.
    /// `32094AD0` stores minimum vertex Z and these per-edge equations; height is not scaled.
    /// Invalid geometry is declined. Box/sphere areas require a separate supported representation.
    pub fn new(points: &[Vec3], height: f32) -> Option<Self> {
        if points.len() < 3 || !height.is_finite() || points.iter().any(|p| !p.is_finite()) {
            return None;
        }
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        let mut segments = Vec::with_capacity(points.len());
        for (a, b) in points.iter().zip(points.iter().cycle().skip(1)) {
            min = min.min(*a);
            max = max.max(*a);
            let horizontal = a.y == b.y;
            let (slope, intercept) = if horizontal {
                (0.0, 0.0)
            } else if a.x == b.x {
                (0.0, a.x)
            } else {
                let slope = (b.y - a.y) / (b.x - a.x);
                (slope, a.y - a.x * slope)
            };
            segments.push(Segment {
                horizontal,
                min_y: a.y.min(b.y),
                max_y: a.y.max(b.y),
                max_x: a.x.max(b.x),
                slope,
                intercept,
            });
        }
        Some(Self {
            min,
            max,
            height,
            segments,
        })
    }

    /// Native `3208F090` polygon predicate with `ignoreHeight=false`.
    /// Height <= 0 is vertically unbounded. Positive height has inclusive floor/ceiling.
    pub fn contains(&self, point: Vec3) -> bool {
        if !(self.min.x <= point.x
            && point.x <= self.max.x
            && self.min.y <= point.y
            && point.y <= self.max.y)
            || (self.height > 0.0
                && !(self.min.z <= point.z && point.z <= self.min.z + self.height))
        {
            return false;
        }
        let p = point.truncate();
        let mut inside = false;
        for edge in &self.segments {
            if !edge.horizontal && edge.min_y < p.y && p.y <= edge.max_y && p.x <= edge.max_x {
                let crossing = if edge.slope == 0.0 {
                    edge.intercept >= p.x
                } else {
                    (p.y - edge.intercept) / edge.slope > p.x
                };
                inside ^= crossing;
            }
        }
        inside
    }
}

/// One entry in native AreaManager registration order, including stale/missing entities.
#[derive(Clone, Copy, Debug)]
pub struct NativeSubZoneRegistration {
    pub group: i32,
    pub entity_id: u32,
}

/// A resolved live IEntity / Area proxy. `enabled` is CArea's active byte, not entity visibility.
#[derive(Clone, Copy, Debug)]
pub struct NativeSubZoneArea<'a> {
    pub id: u32,
    pub enabled: bool,
    pub polygon: &'a NativeSubZonePolygon,
}

/// Native `391EEE00` snapshot: at most the first 1024 registered Group-18 areas,
/// resolved to live proxies at refresh. No automatic world, zone, camera or HUD fallback.
#[derive(Default, Debug)]
pub struct NativeSubZoneSnapshot {
    entries: Vec<(u32, u32)>,
}

impl NativeSubZoneSnapshot {
    /// Rebuilds in supplied registration order. The native discovery cap applies before
    /// resolving entity IDs, so stale entries still consume their discovery slot.
    pub fn refresh<'a>(
        &mut self,
        registrations: impl IntoIterator<Item = NativeSubZoneRegistration>,
        mut live: impl FnMut(u32) -> Option<NativeSubZoneArea<'a>>,
    ) {
        self.entries.clear();
        for entry in registrations
            .into_iter()
            .filter(|e| e.group == 18)
            .take(1024)
        {
            if let Some(area) = live(entry.entity_id) {
                self.entries.push((area.id, entry.entity_id));
            }
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// `391EEF00`: convert global XY once using signed seamless origin cells, then
    /// resolve each saved entity again. Every containing live/active polygon overwrites
    /// the result; the last hit wins, returning its snapshotted area ID.
    pub fn query<'a>(
        &self,
        global_point: Vec3,
        origin_cells: IVec2,
        mut live: impl FnMut(u32) -> Option<NativeSubZoneArea<'a>>,
    ) -> Option<u32> {
        // 365EB1C0 shifts signed origin cells before converting to f32; Z is unchanged.
        let origin = Vec2::new(
            origin_cells.x.wrapping_shl(10) as f32,
            origin_cells.y.wrapping_shl(10) as f32,
        );
        let point = Vec3::new(
            global_point.x - origin.x,
            global_point.y - origin.y,
            global_point.z,
        );
        let mut result = None;
        for &(id, entity_id) in &self.entries {
            if let Some(area) = live(entity_id)
                && area.enabled
                && area.polygon.contains(point)
            {
                result = Some(id);
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn polygon(points: &[[f32; 3]], height: f32) -> NativeSubZonePolygon {
        NativeSubZonePolygon::new(
            &points.iter().copied().map(Vec3::from).collect::<Vec<_>>(),
            height,
        )
        .unwrap()
    }

    fn square(height: f32) -> NativeSubZonePolygon {
        polygon(
            &[
                [0.0, 0.0, 1.0],
                [10.0, 0.0, 1.0],
                [10.0, 10.0, 1.0],
                [0.0, 10.0, 1.0],
            ],
            height,
        )
    }

    fn entry(entity_id: u32) -> NativeSubZoneRegistration {
        NativeSubZoneRegistration {
            group: 18,
            entity_id,
        }
    }

    #[test]
    fn original_x86_height_and_edge_controls() {
        let shape = square(4.0);
        for (p, inside) in [
            ([5.0, 5.0, 0.999], false),
            ([5.0, 5.0, 1.0], true),
            ([5.0, 5.0, 5.0], true),
            ([5.0, 5.0, 5.001], false),
            ([0.0, 5.0, 3.0], false),
            ([10.0, 5.0, 3.0], true),
            ([5.0, 0.0, 3.0], false),
            ([5.0, 10.0, 3.0], true),
        ] {
            assert_eq!(shape.contains(Vec3::from(p)), inside, "{p:?}");
        }
        let triangle = polygon(&[[0.0, 0.0, 1.0], [10.0, 0.0, 1.0], [0.0, 10.0, 1.0]], 0.0);
        for (p, inside) in [
            ([5.0, 5.0, 3.0], false),
            ([4.999, 5.0, 3.0], true),
            ([0.0, 5.0, 3.0], false),
            ([5.0, 0.0, 3.0], false),
        ] {
            assert_eq!(triangle.contains(Vec3::from(p)), inside, "{p:?}");
        }
    }

    #[test]
    fn original_x86_registration_order_liveness_and_origin_controls() {
        let shape = square(4.0);
        let mut snapshot = NativeSubZoneSnapshot::default();
        let live = |entity_id| {
            Some(NativeSubZoneArea {
                id: entity_id * 101,
                enabled: true,
                polygon: &shape,
            })
        };
        let point = Vec3::new(5.0, 5.0, 3.0);
        snapshot.refresh(
            [
                entry(1),
                entry(2),
                NativeSubZoneRegistration {
                    group: 14,
                    entity_id: 3,
                },
            ],
            live,
        );
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot.query(point, IVec2::ZERO, live), Some(202));
        snapshot.refresh([entry(2), entry(1)], live);
        assert_eq!(snapshot.query(point, IVec2::ZERO, live), Some(101));
        snapshot.refresh([entry(1), entry(2)], live);
        let missing = |id| if id == 2 { None } else { live(id) };
        assert_eq!(snapshot.query(point, IVec2::ZERO, missing), Some(101));
        snapshot.refresh([entry(1), entry(2)], missing);
        assert_eq!(snapshot.len(), 1);
        snapshot.refresh([entry(1), entry(2)], live);
        assert_eq!(
            snapshot.query(point, IVec2::ZERO, |id| {
                live(id).map(|mut area| {
                    area.enabled = id != 2;
                    area
                })
            }),
            Some(101)
        );
        for origin in [IVec2::ZERO, IVec2::splat(11), IVec2::new(-2, 3)] {
            let shifted = point + (origin.as_vec2() * 1024.0).extend(0.0);
            assert_eq!(snapshot.query(shifted, origin, live), Some(202));
        }
        // Query returns the saved ID, even if the live proxy's ID changed since refresh.
        assert_eq!(
            snapshot.query(point, IVec2::ZERO, |_| {
                Some(NativeSubZoneArea {
                    id: 999,
                    enabled: true,
                    polygon: &shape,
                })
            }),
            Some(202)
        );
        snapshot.refresh([], live);
        assert!(snapshot.is_empty());
        assert_eq!(snapshot.query(point, IVec2::ZERO, live), None);
    }

    #[test]
    fn original_x86_discovery_cap_precedes_live_resolution() {
        let shape = square(0.0);
        let mut snapshot = NativeSubZoneSnapshot::default();
        let live = |id| {
            Some(NativeSubZoneArea {
                id,
                enabled: true,
                polygon: &shape,
            })
        };
        snapshot.refresh((1..=1025).map(entry), live);
        assert_eq!(snapshot.len(), 1024);
        assert_eq!(
            snapshot.query(Vec3::new(5.0, 5.0, 1.0), IVec2::ZERO, live),
            Some(1024)
        );
        snapshot.refresh((1..=1025).map(entry), |id| {
            if id == 1025 { live(id) } else { None }
        });
        assert!(snapshot.is_empty());
    }

    #[test]
    fn invalid_geometry_is_declined() {
        assert!(NativeSubZonePolygon::new(&[Vec3::ZERO; 2], 0.0).is_none());
        assert!(NativeSubZonePolygon::new(&[Vec3::NAN; 3], 0.0).is_none());
        assert!(NativeSubZonePolygon::new(&[Vec3::ZERO; 3], f32::NAN).is_none());
    }

    #[test]
    fn original_mission_polygons_match_native_producer_and_query_goldens() {
        if !crate::pak_available() {
            return;
        }
        let xml = aa_pak::Pak::global()
            .read_string("worlds/main_world/zone/129/mission_mission0.xml")
            .unwrap();
        let doc = roxmltree::Document::parse(&xml).unwrap();
        let objects = doc
            .root_element()
            .children()
            .find(|n| n.has_tag_name("Objects"))
            .unwrap();
        let mut snapshot = NativeSubZoneSnapshot::default();
        let mut hash = 0xcbf29ce484222325_u64;
        let mut count = 0;
        for node in objects.children().filter(|n| n.has_tag_name("Entity")) {
            let Some(area) = node
                .children()
                .find(|n| n.has_tag_name("Area") && n.attribute("Group") == Some("18"))
            else {
                continue;
            };
            // These original numeric fixtures have translation/one scale, no parenting/rotation.
            // This does not establish which mission entities the real client loads at runtime.
            assert!(node.attribute("Rotate").is_none() && node.attribute("ParentGuid").is_none());
            let vector = |s| Vec3::from(crate::entities::vector::<3>(s).unwrap());
            let pos = vector(node.attribute("Pos").unwrap());
            let scale = vector(node.attribute("Scale").unwrap_or("1,1,1"));
            let points: Vec<_> = area
                .children()
                .find(|n| n.has_tag_name("Points"))
                .unwrap()
                .children()
                .filter(|n| n.has_tag_name("Point"))
                .map(|n| vector(n.attribute("Pos").unwrap()) * scale + pos)
                .collect();
            let shape = NativeSubZonePolygon::new(
                &points,
                area.attribute("Height").unwrap().parse().unwrap(),
            )
            .unwrap();
            let id = area.attribute("Id").unwrap().parse().unwrap();
            let live = |_| {
                Some(NativeSubZoneArea {
                    id,
                    enabled: true,
                    polygon: &shape,
                })
            };
            snapshot.refresh([entry(1)], live);
            let lo = points
                .iter()
                .copied()
                .fold(Vec3::splat(f32::INFINITY), Vec3::min);
            let hi = points
                .iter()
                .copied()
                .fold(Vec3::splat(f32::NEG_INFINITY), Vec3::max);
            let mid = (lo + hi) * 0.5;
            for p in [
                mid,
                Vec3::new(lo.x, mid.y, 1000.0),
                Vec3::new(hi.x, mid.y, 1000.0),
                Vec3::new(mid.x, lo.y, -1000.0),
                Vec3::new(mid.x, hi.y, -1000.0),
                Vec3::new(lo.x - 0.25, mid.y, 1000.0),
                Vec3::new(hi.x + 0.25, mid.y, 1000.0),
            ] {
                let result = snapshot.query(p, IVec2::ZERO, live);
                hash = (hash ^ u64::from(result.unwrap_or(0))).wrapping_mul(0x100000001b3);
                count += 1;
            }
        }
        // 45 original shapes, seven probes each. Original SetPoints32094AD0 and
        // snapshot/query391EEE00/391EEF00 executed in x86, including native containment.
        assert_eq!(count, 315);
        assert_eq!(hash, 0xa2ef7777d53f10b8);
    }
}
