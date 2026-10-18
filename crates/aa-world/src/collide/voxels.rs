//! Native first-LOD voxel proxies and their local 32-entry terrain-surface palette.

use std::collections::HashMap;

use aa_cry::material::Material;
use bevy::math::{Mat4, Vec3};

use super::{CellStats, Inner, geom::Tri, surface};
use crate::placement::PlacedVoxel;

const MAX_CELL_DECODED_BYTES: usize = 64 * 1024 * 1024;
const MAX_CELL_TRIANGLES: usize = 1_000_000;
// Conservative XY AABB estimate at the configured grid size; the soup can only coarsen it.
// This bounds added grid-index storage even for corrupt meshes made of enormous triangles.
const MAX_CELL_GRID_REFERENCES: usize = 8 * 1024 * 1024;

fn grid_references(triangle: &Tri, grid: f32) -> Option<usize> {
    let lo = triangle[0].min(triangle[1]).min(triangle[2]);
    let hi = triangle[0].max(triangle[1]).max(triangle[2]);
    let span = hi - lo;
    if !span.is_finite() {
        return None;
    }
    let grid = grid.max(f32::MIN_POSITIVE);
    let x = ((span.x / grid).ceil() + 2.0) as usize;
    let y = ((span.y / grid).ceil() + 2.0) as usize;
    x.checked_mul(y)
}

struct Entry {
    name: String,
    material: Option<Material>,
    axis: u8,
}

fn axis(name: &str) -> u8 {
    match name.to_ascii_uppercase().as_str() {
        "X" | "XP" => 1,
        "Y" | "YP" => 2,
        "Z" | "ZP" => 3,
        "XN" => 4,
        "YN" => 5,
        "ZN" => 6,
        _ => 0,
    }
}

// Native 300C66F0 maps ProjAxis X/Y/Z -> 1/2/3 (otherwise 0).
// 301227A0 assigns sub-material projection IDs by X/XP/Y/YP/Z/ZP/XN/YN/ZN name;
// 300C9120 chooses the matching material, or the root when fewer than three children.
fn physical_surface(material: &Material, projection: u8) -> Option<u16> {
    let chosen = if material.sub_materials.len() < 3 {
        Some(material)
    } else {
        material
            .sub_materials
            .iter()
            .take(6)
            .find(|m| axis(&m.name) == projection)
    };
    let Some(chosen) = chosen else {
        return Some(surface::DEFAULT);
    };
    if !surface::collides(&chosen.surface_type) {
        return None;
    }
    Some(surface::id_of(&chosen.surface_type))
}

fn entries(world: &str, cell: (i32, i32)) -> Vec<Entry> {
    let path = format!(
        "{}/client/leveldata.xml",
        crate::world::cell_dir(world, cell.0, cell.1)
    );
    let Ok(xml) = aa_pak::Pak::global().read_string(&path) else {
        return Vec::new();
    };
    let Ok(doc) = roxmltree::Document::parse(&xml) else {
        return Vec::new();
    };
    let mut cache = HashMap::<String, Option<Material>>::new();
    doc.descendants()
        .filter(|n| n.has_tag_name("SurfaceType"))
        .take(32)
        .map(|n| {
            let name = n.attribute("Name").unwrap_or("").to_string();
            let path = n.attribute("DetailMaterial").unwrap_or("").trim();
            let material = (!path.is_empty())
                .then(|| {
                    let path = if path.to_ascii_lowercase().ends_with(".mtl") {
                        path.to_string()
                    } else {
                        format!("{path}.mtl")
                    };
                    cache
                        .entry(path.clone())
                        .or_insert_with(|| Material::load(&path).ok())
                        .clone()
                })
                .flatten();
            let axis = match n.attribute("ProjAxis").unwrap_or("").as_bytes().first() {
                Some(b'X') => 1,
                Some(b'Y') => 2,
                Some(b'Z') => 3,
                _ => 0,
            };
            Entry {
                name,
                material,
                axis,
            }
        })
        .collect()
}

fn palette(slots: &[String; 32], entries: &[Entry]) -> [Option<u16>; 32] {
    // Native 30196930: case-insensitive name match, otherwise first entry with a material,
    // otherwise entry zero. Empty local slots stay null rather than inventing a surface.
    let fallback = entries
        .iter()
        .find(|e| e.material.is_some())
        .or_else(|| entries.first());
    std::array::from_fn(|i| {
        if slots[i].is_empty() {
            return None;
        }
        let entry = entries
            .iter()
            .find(|e| e.name.eq_ignore_ascii_case(&slots[i]))
            .or(fallback)?;
        entry
            .material
            .as_ref()
            .map_or(Some(surface::DEFAULT), |m| physical_surface(m, entry.axis))
    })
}

pub(super) fn append(
    inner: &Inner,
    slots: &[PlacedVoxel],
    cell: (i32, i32),
    structures: &mut (Vec<Tri>, Vec<u16>),
    stats: &mut CellStats,
) {
    stats.voxels = slots.len();
    if slots.is_empty() {
        return;
    }
    let entries = entries(&inner.world, cell);
    let origin = Mat4::from_translation(crate::coords::cell_origin(cell.0, cell.1));
    let mut decoded_bytes = 0usize;
    let mut grid_references_left = MAX_CELL_GRID_REFERENCES;
    for slot in slots {
        let Some(size) = crate::placement::voxel_decoded_size(&slot.compiled) else {
            stats.voxels_unsupported += 1;
            continue;
        };
        if size > MAX_CELL_DECODED_BYTES.saturating_sub(decoded_bytes) {
            stats.voxels_unsupported += 1;
            continue;
        }
        decoded_bytes += size;
        let Some(mesh) = crate::placement::voxel_collision(slot) else {
            stats.voxels_unsupported += 1;
            continue;
        };
        if mesh.indices.len() / 3 > MAX_CELL_TRIANGLES.saturating_sub(stats.voxel_triangles) {
            stats.voxels_unsupported += 1;
            continue;
        }
        let surfaces = palette(&slot.surfaces, &entries);
        let transform = origin * slot.mat4();
        let mirrored = transform.determinant() < 0.0;
        let world: Vec<Vec3> = mesh
            .positions
            .iter()
            .map(|p| transform.transform_point3(*p))
            .collect();
        let references = mesh
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .zip(&mesh.materials)
            .filter(|(_, material)| surfaces[**material as usize].is_some())
            .try_fold(0usize, |sum, (t, _)| {
                let triangle = t.map(|i| world[i as usize]);
                if !triangle.iter().all(|p| p.is_finite()) {
                    return Some(sum);
                }
                let total = sum.checked_add(grid_references(&triangle, inner.config.grid)?)?;
                (total <= grid_references_left).then_some(total)
            });
        let Some(references) = references else {
            stats.voxels_unsupported += 1;
            continue;
        };
        grid_references_left -= references;
        let before = stats.voxel_triangles;
        for (t, &material) in mesh.indices.as_chunks::<3>().0.iter().zip(&mesh.materials) {
            let Some(surface) = surfaces.get(material as usize).copied().flatten() else {
                continue;
            };
            let [a, b, c] = t.map(|i| world[i as usize]);
            if ![a, b, c].iter().all(|p| p.is_finite()) {
                continue;
            }
            structures
                .0
                .push(if mirrored { [a, c, b] } else { [a, b, c] });
            structures.1.push(surface);
            stats.voxel_triangles += 1;
        }
        stats.voxels_colliding += usize::from(stats.voxel_triangles > before);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collide::Mask;
    use crate::{TerrainQuery, WorldCollision};

    #[test]
    fn native_surface_projection_names_and_missing_palette_fallback() {
        let material = Material::parse(
            r#"<Material SurfaceType="mat_default"><SubMaterials>
          <Material Name="YN" SurfaceType="mat_metal"/><Material Name="ZP" SurfaceType="mat_dirt"/>
          <Material Name="XP" SurfaceType="mat_rock"/></SubMaterials></Material>"#,
        )
        .unwrap();
        assert_eq!(
            surface::name_of(physical_surface(&material, 3).unwrap()),
            "mat_dirt"
        );
        assert_eq!(
            surface::name_of(physical_surface(&material, 1).unwrap()),
            "mat_rock"
        );
        assert_eq!(
            surface::name_of(physical_surface(&material, 5).unwrap()),
            "mat_metal"
        );
        assert_eq!(physical_surface(&material, 2), Some(surface::DEFAULT));
        let entries = vec![
            Entry {
                name: "missing".into(),
                material: None,
                axis: 0,
            },
            Entry {
                name: "fallback".into(),
                material: Some(material),
                axis: 3,
            },
        ];
        let mut names = std::array::from_fn(|_| String::new());
        names[0] = "undefined".into();
        names[1] = "FALLBACK".into();
        let ids = palette(&names, &entries);
        assert_eq!(ids[0], ids[1]);
        assert_eq!(ids[2], None);
    }

    #[test]
    fn malformed_voxel_scale_cannot_expand_the_collision_grid_index() {
        if !crate::pak_available() {
            return;
        }
        let objects = crate::placement::load_cell_objects("main_world", 20, 8);
        let mut voxel = objects.voxels[0].clone();
        for i in [0, 5, 10] {
            voxel.matrix[i] = 1_000_000.0;
        }
        let collision = WorldCollision::new("main_world", TerrainQuery::new("main_world"));
        let mut structures = (Vec::new(), Vec::new());
        let mut stats = CellStats::default();
        append(
            &collision.inner,
            &[voxel],
            (20, 8),
            &mut structures,
            &mut stats,
        );
        assert_eq!(stats.voxels_unsupported, 1);
        assert_eq!(stats.voxel_triangles, 0);
        assert!(structures.0.is_empty() && structures.1.is_empty());
    }

    #[test]
    fn original_cave_floor_wall_rays_and_cell_unload_use_native_voxel_proxies() {
        if !crate::pak_available() {
            return;
        }
        let world = WorldCollision::new("main_world", TerrainQuery::new("main_world"));
        let cell = world.build_blocking(20, 8).unwrap();
        println!("native voxel cell: {:?}", cell.stats);
        assert_eq!(cell.stats.voxels, 7);
        assert!(cell.stats.voxels_colliding > 0 && cell.stats.voxel_triangles > 5642);
        for (point, normal) in [
            (
                Vec3::new(21354.6667, 9055.3333, 454.91738),
                Vec3::new(0.26708772, -0.33918378, 0.90200806),
            ),
            (
                Vec3::new(21322.6667, 9063.3309, 472.96565),
                Vec3::new(0.61825323, 0.7845947, -0.04662632),
            ),
            (
                Vec3::new(21325.3333, 9054.6667, 438.62933),
                Vec3::new(-0.2663303, 0.3331916, 0.904462),
            ),
        ] {
            let hit = world
                .raycast_mask(point + normal * 0.5, -normal, 1.0, Mask::STRUCTURES)
                .unwrap();
            println!("native voxel ray: {hit:?}");
            assert!(
                !hit.terrain && (hit.t - 0.5).abs() < 0.004 && hit.point.distance(point) < 0.008
            );
            assert!(hit.normal.dot(normal) > 0.999);
        }
        let floor = Vec3::new(21354.6667, 9055.3333, 454.91738);
        let ground = world
            .ground_below(floor.x, floor.y, floor.z + 0.3, 0.0)
            .unwrap();
        assert!((ground.height - floor.z).abs() < 0.01);
        let (_, physical) = world
            .raycast_surface(floor + Vec3::Z * 0.3, Vec3::NEG_Z, 1.0, Mask::STRUCTURES)
            .unwrap();
        assert_eq!(physical.name(), "mat_dirt");
        drop(cell);
        world.release_outside(-100000.0, -100000.0, 0.0);
        assert_eq!(world.stats().cells, 0);
        assert!(
            world
                .raycast_mask(floor + Vec3::Z, Vec3::NEG_Z, 2.0, Mask::STRUCTURES)
                .is_none()
        );
    }
}
