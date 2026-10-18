//! Native 1.2 CWaterVolumeRenderNode UV producers: 300E85A0 /300E9320.

use crate::placement::Water;
use bevy::prelude::*;

/// Four river banks followed by the native centre. UVs are ratios of signed distances
/// to opposite vertical bank/end planes, not a guessed rectangular interpolation.
pub(super) fn river(w: &Water) -> Option<([Vec3; 5], [Vec2; 5])> {
    if w.points.len() != 4 || !w.uv_scale.iter().chain(&w.river_u).all(|v| v.is_finite()) {
        return None;
    }
    let p: [Vec3; 4] = w.points[..4].try_into().ok()?;
    if p.iter().any(|p| !p.is_finite()) {
        return None;
    }
    let plane = |a: Vec3, b: Vec3, sign: f32| {
        let n = (b - a).cross(b + Vec3::Z * sign - a).normalize_or_zero();
        n.extend(-n.dot(a))
    };
    let start = plane(p[0], p[1], 1.0);
    let end = plane(p[2], p[3], -1.0);
    let left = plane(p[0], p[2], -1.0);
    let right = plane(p[1], p[3], 1.0);
    if [start, end, left, right]
        .iter()
        .any(|p| p.truncate().length_squared() < 0.5)
    {
        return None;
    }
    let ratio = |a: f32, b: f32| {
        if (a + b).abs() >= f32::EPSILON {
            a / (a + b)
        } else {
            0.0
        }
    };
    let points = [p[0], p[1], p[2], p[3], (p[0] + p[1] + p[2] + p[3]) * 0.25];
    let mut uvs = [Vec2::ZERO; 5];
    for (point, uv) in points.iter().zip(&mut uvs) {
        let q = point.extend(1.0);
        let u = ratio(start.dot(q), end.dot(q));
        let v = ratio(left.dot(q), right.dot(q));
        *uv = Vec2::new(
            w.uv_scale[0] * (w.river_u[0].abs() * (1.0 - u) + w.river_u[1].abs() * u),
            w.uv_scale[1] * v,
        );
    }
    uvs.iter().all(|uv| uv.is_finite()).then_some((points, uvs))
}

/// _LoadWaterVolume byte40 bits2/4 -> CreateRiver begin/end alpha booleans.
/// Both bank vertices fade together; the native centre remains fully opaque.
pub(super) fn river_alpha(kind: u32) -> [f32; 5] {
    let begin = if kind & 0x200 != 0 { 0.0 } else { 1.0 };
    let end = if kind & 0x400 != 0 { 0.0 } else { 1.0 };
    [begin, begin, end, end, 1.0]
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::mesh::VertexAttributeValues;

    #[test]
    fn native_endpoint_flags_preserve_pair_order_and_centre() {
        assert_eq!(river_alpha(3), [1.0; 5]);
        assert_eq!(river_alpha(0x203), [0.0, 0.0, 1.0, 1.0, 1.0]);
        assert_eq!(river_alpha(0x403), [1.0, 1.0, 0.0, 0.0, 1.0]);
        assert_eq!(river_alpha(0x603), [0.0, 0.0, 0.0, 0.0, 1.0]);
        if !crate::pak_available() {
            return;
        }
        let objects = crate::placement::load_cell_objects("main_world", 9, 12);
        for (flag, expected) in [
            (0x203, [0.0, 0.0, 1.0, 1.0, 1.0]),
            (0x403, [1.0, 1.0, 0.0, 0.0, 1.0]),
        ] {
            let w = objects.water.iter().find(|w| w.kind == flag).unwrap();
            let mesh = super::super::water_mesh(std::slice::from_ref(w)).unwrap();
            let Some(VertexAttributeValues::Float32x4(colors)) =
                mesh.attribute(Mesh::ATTRIBUTE_COLOR)
            else {
                panic!()
            };
            assert_eq!(colors.iter().map(|c| c[3]).collect::<Vec<_>>(), expected);
            assert!(colors.iter().all(|c| c[..3] == [1.0; 3]));
        }
    }

    #[test]
    fn original_river_ranges_five_vertex_fan_and_independent_area_scales() {
        if !crate::pak_available() {
            return;
        }
        let objects = crate::placement::load_cell_objects("main_world", 9, 12);
        let rivers: Vec<_> = objects.water.iter().filter(|w| w.kind & 3 == 3).collect();
        assert!(rivers.len() > 100);
        let w = rivers[0];
        assert_eq!(w.uv_scale, [0.2, 1.6]);
        assert!((w.river_u[0] - 3.063283).abs() < 1e-6);
        let (p, uv) = river(w).unwrap();
        for i in 0..4 {
            assert!((uv[i].x - w.river_u[i / 2].abs() * w.uv_scale[0]).abs() < 1e-4);
            assert!((uv[i].y - if i % 2 == 0 { 0.0 } else { w.uv_scale[1] }).abs() < 1e-4);
        }
        assert_eq!(p[4], w.points.iter().copied().sum::<Vec3>() * 0.25);
        let mesh = super::super::water_mesh(std::slice::from_ref(w)).unwrap();
        assert_eq!(mesh.count_vertices(), 5);
        assert_eq!(
            mesh.indices().unwrap().iter().collect::<Vec<_>>(),
            [0, 1, 4, 1, 3, 4, 3, 2, 4, 0, 4, 2]
        );
        let mut bad = w.clone();
        bad.points[1] = bad.points[0];
        assert!(river(&bad).is_none());
        let pond = crate::placement::load_cell_objects("main_world", 15, 15)
            .water
            .remove(0);
        let mesh = super::super::water_mesh(std::slice::from_ref(&pond)).unwrap();
        let Some(VertexAttributeValues::Float32x2(uv)) = mesh.attribute(Mesh::ATTRIBUTE_UV_0)
        else {
            panic!()
        };
        assert_eq!(uv[0], [0.0, 0.0]);
        for (p, uv) in pond.points.iter().zip(uv) {
            let expected = (p.truncate() - pond.points[0].truncate()) * Vec2::from(pond.uv_scale);
            assert!(Vec2::from(*uv).abs_diff_eq(expected, 1e-6));
        }
    }
}
