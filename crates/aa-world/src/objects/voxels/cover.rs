//! Native CTerrainNode cover selection (300D70B0) and half-texel mapping (300D9340).

use bevy::prelude::*;

use crate::cover::{Cover, CoverAtlas, TILE_PX};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct Node {
    pub level: usize,
    pub x: usize,
    pub y: usize,
}

impl Node {
    fn size(self) -> f32 {
        1024.0 / (1usize << self.level) as f32
    }

    fn contains(self, bbox: &[f32; 6]) -> bool {
        let size = self.size();
        bbox[0] >= self.x as f32 * size - 0.01
            && bbox[1] >= self.y as f32 * size - 0.01
            && bbox[3] <= (self.x + 1) as f32 * size + 0.01
            && bbox[4] <= (self.y + 1) as f32 * size + 0.01
    }

    pub fn uv(self, cell_local: Vec3) -> Vec2 {
        let scale = (1.0 - 1.0 / TILE_PX as f32) / self.size();
        Vec2::new(
            cell_local.y - self.y as f32 * self.size(),
            cell_local.x - self.x as f32 * self.size(),
        ) * scale
            + Vec2::splat(0.5 / TILE_PX as f32)
    }
}

pub(super) fn select(cover: &Cover, bbox: &[f32; 6]) -> Option<Node> {
    if bbox.iter().any(|v| !v.is_finite()) || (0..3).any(|i| bbox[i] > bbox[i + 3]) {
        return None;
    }
    let mut node = Node {
        level: 0,
        x: 0,
        y: 0,
    };
    if !node.contains(bbox) {
        return None;
    }
    // Original terrain's minimum sector is 64 m. Cover pixels below that sector are not
    // independently selectable terrain nodes. Stop at the actual file's deepest tile too.
    while node.level < cover.max_level.min(4) {
        let next = (0..4)
            .map(|i| Node {
                level: node.level + 1,
                x: node.x * 2 + i % 2,
                y: node.y * 2 + i / 2,
            })
            .find(|child| child.contains(bbox));
        let Some(next) = next else { break };
        node = next;
    }
    loop {
        if cover.tile(0, node.level, node.x, node.y).is_some() {
            return Some(node);
        }
        if node.level == 0 {
            return None;
        }
        node = Node {
            level: node.level - 1,
            x: node.x / 2,
            y: node.y / 2,
        };
    }
}

pub(super) fn image(cover: &Cover, node: Node) -> Option<Image> {
    Some(crate::terrain::cover_image(CoverAtlas {
        format: cover.layers.first()?.format,
        size: TILE_PX as u32,
        mips: 1,
        data: cover.tile(0, node.level, node.x, node.y)?.to_vec(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_neighbour_cover_node_and_native_half_texel_mapping() {
        if !crate::pak_available() {
            return;
        }
        let bytes = aa_pak::Pak::global()
            .read("game/worlds/main_world/cells/021_008/client/terrain/cover.ctc")
            .unwrap();
        let cover = Cover::parse(&bytes).unwrap();
        let objects = crate::placement::load_cell_objects("main_world", 21, 8);
        assert_eq!(objects.voxels.len(), 2);
        let expected = Node {
            level: 4,
            x: 13,
            y: 0,
        };
        for voxel in &objects.voxels {
            assert_eq!(select(&cover, &voxel.bbox), Some(expected));
        }
        assert_eq!(
            expected.uv(Vec3::new(832.0, 0.0, 448.0)),
            Vec2::splat(0.5 / 128.0)
        );
        assert_eq!(
            expected.uv(Vec3::new(896.0, 64.0, 576.0)),
            Vec2::splat(127.5 / 128.0)
        );
        assert_eq!(
            expected.uv(Vec3::new(864.0, 16.0, -400.0)),
            Vec2::new(0.251953125, 0.5)
        );
        // A crossing box selects its common ancestor, never a tile chosen by its centre.
        assert_eq!(
            select(&cover, &[831.0, 0.0, 0.0, 896.0, 64.0, 1.0])
                .unwrap()
                .level,
            3
        );
        assert!(select(&cover, &[-1.0, 0.0, 0.0, 64.0, 64.0, 1.0]).is_none());
        assert!(select(&cover, &[0.0, f32::NAN, 0.0, 64.0, 64.0, 1.0]).is_none());
    }
}
