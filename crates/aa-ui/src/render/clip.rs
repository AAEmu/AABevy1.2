//! CPU scissor clipping with interpolated UVs/colours for rotated effect and scroll geometry.

use super::Quad;
use bevy::color::ColorToComponents;
use bevy::prelude::Color;

#[derive(Clone, Copy)]
struct Vertex {
    pos: [f32; 2],
    uv: [f32; 2],
    color: [f32; 4],
}

impl Vertex {
    fn lerp(self, other: Self, t: f32) -> Self {
        let mix = |a: f32, b: f32| a + (b - a) * t;
        Self {
            pos: std::array::from_fn(|i| mix(self.pos[i], other.pos[i])),
            uv: std::array::from_fn(|i| mix(self.uv[i], other.uv[i])),
            color: std::array::from_fn(|i| mix(self.color[i], other.color[i])),
        }
    }
}

pub(super) fn quads(out: &mut Vec<Quad>, first: usize, bounds: [f32; 4]) {
    if out[first..]
        .iter()
        .all(|q| q.corners.is_none() && q.uvs.is_none() && q.vcolors.is_none())
    {
        super::clip_rectangles(&mut out[first..], bounds);
        return;
    }
    let mut clipped = Vec::new();
    for quad in out.drain(first..) {
        let [x0, y0, x1, y1] = quad.pos;
        let [u0, v0, u1, v1] = quad.uv;
        let points = quad
            .corners
            .unwrap_or([[x0, y0], [x1, y0], [x1, y1], [x0, y1]]);
        let uv = quad.uvs.unwrap_or([[u0, v0], [u1, v0], [u1, v1], [u0, v1]]);
        let colors = quad.vcolors.unwrap_or([quad.color; 4]).map(|c| {
            Color::srgba(c[0], c[1], c[2], c[3])
                .to_linear()
                .to_f32_array()
        });
        if points.iter().all(|p| {
            p[0] >= bounds[0] && p[1] >= bounds[1] && p[0] <= bounds[2] && p[1] <= bounds[3]
        }) {
            clipped.push(quad);
            continue;
        }
        let vertices: [Vertex; 4] = std::array::from_fn(|i| Vertex {
            pos: points[i],
            uv: uv[i],
            color: colors[i],
        });
        for indices in [[0, 1, 2], [0, 2, 3]] {
            let mut polygon = indices.map(|i| vertices[i]).to_vec();
            for (axis, edge, lower) in [
                (0, bounds[0], true),
                (1, bounds[1], true),
                (0, bounds[2], false),
                (1, bounds[3], false),
            ] {
                polygon = cut(&polygon, axis, edge, lower);
            }
            for i in 1..polygon.len().saturating_sub(1) {
                let points = [polygon[0], polygon[i], polygon[i + 1], polygon[i + 1]];
                let area = (points[1].pos[0] - points[0].pos[0])
                    * (points[2].pos[1] - points[0].pos[1])
                    - (points[1].pos[1] - points[0].pos[1]) * (points[2].pos[0] - points[0].pos[0]);
                if area.abs() < 0.00001 {
                    continue;
                }
                let mut triangle = quad.clone();
                triangle.corners = Some(points.map(|v| v.pos));
                triangle.uvs = Some(points.map(|v| v.uv));
                triangle.vcolors = Some(points.map(|v| {
                    let c = v.color;
                    Color::linear_rgba(c[0], c[1], c[2], c[3])
                        .to_srgba()
                        .to_f32_array()
                }));
                triangle.pos = [
                    points
                        .iter()
                        .map(|v| v.pos[0])
                        .fold(f32::INFINITY, f32::min),
                    points
                        .iter()
                        .map(|v| v.pos[1])
                        .fold(f32::INFINITY, f32::min),
                    points
                        .iter()
                        .map(|v| v.pos[0])
                        .fold(f32::NEG_INFINITY, f32::max),
                    points
                        .iter()
                        .map(|v| v.pos[1])
                        .fold(f32::NEG_INFINITY, f32::max),
                ];
                clipped.push(triangle);
            }
        }
    }
    out.extend(clipped);
}

fn cut(polygon: &[Vertex], axis: usize, edge: f32, lower: bool) -> Vec<Vertex> {
    let mut out = Vec::with_capacity(polygon.len() + 1);
    let Some(&last) = polygon.last() else {
        return out;
    };
    let inside = |v: Vertex| {
        if lower {
            v.pos[axis] >= edge
        } else {
            v.pos[axis] <= edge
        }
    };
    let mut prev = last;
    for &next in polygon {
        if inside(prev) != inside(next) {
            let t = (edge - prev.pos[axis]) / (next.pos[axis] - prev.pos[axis]);
            out.push(prev.lerp(next, t));
        }
        if inside(next) {
            out.push(next);
        }
        prev = next;
    }
    out
}
