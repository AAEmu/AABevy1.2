//! The compiled visarea container, recovered from the 1.2 engine's 3017A020/30170860.
//! Only brush placement is imported: portal visibility and area lighting remain separate work.

use super::{CellObjects, PlacementError, read_brush};
use crate::reader::{Reader, cstr, u32_at};
use bevy::math::Vec3;

fn table(r: &mut Reader) -> Option<Vec<String>> {
    let count = r.u32().ok()? as usize;
    let records = r.bytes(count.checked_mul(260)?).ok()?;
    Some(
        records
            .chunks_exact(260)
            .map(|b| cstr(&b[4..]).trim().to_string())
            .collect(),
    )
}

/// Interior brush placements in cell-local Cry coordinates. The two path tables belong to
/// this file, independently of `statobjs.dat` and `object.dat`. Native 301F2CA0 adds the cell
/// origin to the serialized matrix/bounds; our ordinary brush renderer/collision does that.
///
/// Area/portal/occlusion records are consumed in their serialized order. Other object kinds
/// are skipped, with an unknown-sized kind ending its object block, never its child trees.
pub fn parse_visareas_dat(data: &[u8]) -> Result<CellObjects, PlacementError> {
    parse(data).ok_or(PlacementError::Malformed("visareas.dat"))
}

fn parse(data: &[u8]) -> Option<CellObjects> {
    let mut r = Reader::new(data);
    let mut out = CellObjects {
        models: table(&mut r)?,
        materials: table(&mut r)?,
        ..Default::default()
    };
    let compiled_bytes = r.remaining();
    let header = r.bytes(20).ok()?;
    if u32_at(header, 0) != 4 || u32_at(header, 4) as usize != compiled_bytes {
        return None;
    }
    let records = (u32_at(header, 8) as usize)
        .checked_add(u32_at(header, 12) as usize)?
        .checked_add(u32_at(header, 16) as usize)?;
    // Every record needs its fixed header and at least the polygon point count.
    if records > r.remaining() / 268 {
        return None;
    }
    for _ in 0..records {
        let head = r.bytes(264).ok()?;
        match u32_at(head, 0) {
            2 => (),
            3 => r.skip(12).ok()?,
            _ => return None,
        }
        let points = r.u32().ok()? as usize;
        r.skip(points.checked_mul(12)?).ok()?;
        let bytes = r.bytes(u32_at(head, 84) as usize).ok()?;
        if bytes.len() > 4 {
            let mut tree = Reader::new(bytes);
            walk(&mut tree, &mut out, 0)?;
            if tree.remaining() != 0 {
                return None;
            }
        }
    }
    if r.remaining() != 0
        || out.brushes.iter().any(|b| {
            ((b.model as i32) >= 0 && b.model as usize >= out.models.len())
                || (b.material >= 0 && b.material as usize >= out.materials.len())
        })
    {
        return None;
    }
    Some(out)
}

fn walk(r: &mut Reader, out: &mut CellObjects, depth: usize) -> Option<()> {
    // A malformed mask chain cannot exhaust the worker's stack. Real cell trees are shallow;
    // this is a resource guard, not an authored visibility or geometry limit.
    if depth >= 64 || r.u32().ok()? != 2 {
        return None;
    }
    r.skip(24).ok()?;
    let block_bytes = r.u32().ok()? as usize;
    let mask = r.u8().ok()?;
    let mut block = Reader::new(r.bytes(block_bytes).ok()?);
    while block.remaining() != 0 {
        match block.u32().ok()? {
            1 => out
                .brushes
                .push(read_brush(block.bytes(128).ok()?, Vec3::ZERO)),
            2 => block.skip(64).ok()?,
            9 => block.skip(111).ok()?,
            13 => {
                let road = block.bytes(63).ok()?;
                block
                    .skip((u32_at(road, 39) as usize).checked_mul(12)?)
                    .ok()?;
            }
            // These records have no recovered common byte-size field. In particular water
            // is variable-sized. Keep the next octree child independent of this block.
            _ => break,
        }
    }
    for bit in 0..8 {
        if mask >> bit & 1 != 0 {
            walk(r, out, depth + 1)?;
        }
    }
    Some(())
}

pub(super) fn append_brushes(out: &mut CellObjects, mut interior: CellObjects) {
    let model_base = out.models.len() as u32;
    let material_base = out.materials.len() as i32;
    for brush in &mut interior.brushes {
        // Native 301F2B00/301F2B40 return null for negative serialized indices.
        if (brush.model as i32) >= 0 {
            brush.model += model_base;
        }
        if brush.material >= 0 {
            brush.material += material_base;
        }
    }
    // Preserve authored instances, including byte-identical repeats within one area. Native
    // 301F8420 constructs each brush; neither an exterior fallback nor a second file copy
    // is loaded here. Model/texture caches still share identical paths after this remap.
    out.models.extend(interior.models);
    out.materials.extend(interior.materials);
    out.brushes.extend(interior.brushes);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tables() -> Vec<u8> {
        let mut bytes = Vec::new();
        for names in [&["room.cgf"][..], &["room_floor"][..]] {
            bytes.extend((names.len() as u32).to_le_bytes());
            for name in names {
                let mut entry = [0; 260];
                entry[4..4 + name.len()].copy_from_slice(name.as_bytes());
                bytes.extend(entry);
            }
        }
        bytes
    }

    fn brush(material: i32) -> Vec<u8> {
        let mut bytes = vec![0; 132];
        bytes[..4].copy_from_slice(&1u32.to_le_bytes());
        bytes[119..123].copy_from_slice(&material.to_le_bytes());
        for (i, value) in [
            1.0f32, 0.0, 0.0, 872.0, 0.0, 1.0, 0.0, 104.0, 0.0, 0.0, 1.0, 256.0,
        ]
        .into_iter()
        .enumerate()
        {
            bytes[71 + i * 4..75 + i * 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }

    fn node(block: &[u8], child: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0; 33];
        bytes[..4].copy_from_slice(&2u32.to_le_bytes());
        bytes[28..32].copy_from_slice(&(block.len() as u32).to_le_bytes());
        bytes[32] = u8::from(!child.is_empty());
        bytes.extend(block);
        bytes.extend(child);
        bytes
    }

    fn record(version: u32, tree: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0; 264];
        bytes[..4].copy_from_slice(&version.to_le_bytes());
        bytes[84..88].copy_from_slice(&(tree.len() as u32).to_le_bytes());
        if version == 3 {
            bytes.extend([0; 12]);
        }
        bytes.extend(1u32.to_le_bytes());
        bytes.extend([0; 12]);
        bytes.extend(tree);
        bytes
    }

    fn file(records: &[Vec<u8>]) -> Vec<u8> {
        let mut bytes = tables();
        let size = 20 + records.iter().map(Vec::len).sum::<usize>();
        for value in [4, size as u32, records.len() as u32, 0, 0] {
            bytes.extend(value.to_le_bytes());
        }
        for record in records {
            bytes.extend(record);
        }
        bytes
    }

    #[test]
    fn separate_tables_remap_without_touching_exterior_or_double_cell_offset() {
        let source = file(&[
            record(2, &node(&brush(0), &[])),
            record(3, &node(&brush(-1), &[])),
        ]);
        let interior = parse_visareas_dat(&source).unwrap();
        let mut out = CellObjects {
            models: vec!["outside.cgf".into()],
            materials: vec!["outside".into(), "other".into()],
            ..Default::default()
        };
        append_brushes(&mut out, interior);
        assert_eq!(out.models, ["outside.cgf", "room.cgf"]);
        assert_eq!(out.materials, ["outside", "other", "room_floor"]);
        assert_eq!(out.brushes[0].model, 1);
        assert_eq!(out.brushes[0].material, 2);
        assert_eq!(out.brushes[1].material, -1);
        assert_eq!(out.brushes[0].translation(), Vec3::new(872.0, 104.0, 256.0));
        let global = crate::coords::cell_origin(0, 4) + out.brushes[0].translation();
        assert_eq!(global, Vec3::new(872.0, 4200.0, 256.0));
    }

    #[test]
    fn object_blocks_cannot_consume_children_and_unknown_kinds_keep_children() {
        let child = node(&brush(0), &[]);
        let unknown = node(&[6, 0, 0, 0, 1, 2, 3], &child);
        assert_eq!(
            parse_visareas_dat(&file(&[record(2, &unknown)]))
                .unwrap()
                .brushes
                .len(),
            1
        );
        let short_brush = node(&brush(0)[..128], &child);
        assert!(parse_visareas_dat(&file(&[record(2, &short_brush)])).is_err());
        let mut runaway = child;
        for _ in 0..64 {
            runaway = node(&[], &runaway);
        }
        assert!(parse_visareas_dat(&file(&[record(2, &runaway)])).is_err());
    }

    #[test]
    fn malformed_counts_versions_tree_lengths_and_indices_fail_atomically() {
        let valid = file(&[record(2, &node(&brush(0), &[]))]);
        for cut in [0, 3, 528, 547, valid.len() - 1] {
            assert!(parse_visareas_dat(&valid[..cut]).is_err());
        }
        let table_bytes = tables().len();
        for (at, value) in [
            (0, u32::MAX),
            (table_bytes, 5),
            (table_bytes + 4, 0),
            (table_bytes + 8, u32::MAX),
            (table_bytes + 20, 1),
            (table_bytes + 104, u32::MAX),
            (table_bytes + 284, u32::MAX),
        ] {
            let mut broken = valid.clone();
            broken[at..at + 4].copy_from_slice(&value.to_le_bytes());
            assert!(parse_visareas_dat(&broken).is_err(), "offset {at}");
        }
        assert!(parse_visareas_dat(&file(&[record(2, &node(&brush(1), &[]))])).is_err());
        let mut invalid_model = brush(0);
        invalid_model[127..131].copy_from_slice(&1u32.to_le_bytes());
        assert!(parse_visareas_dat(&file(&[record(2, &node(&invalid_model, &[]))])).is_err());
        invalid_model[127..131].copy_from_slice(&u32::MAX.to_le_bytes());
        let null_model =
            parse_visareas_dat(&file(&[record(2, &node(&invalid_model, &[]))])).unwrap();
        let mut out = CellObjects {
            models: vec!["outside.cgf".into()],
            ..Default::default()
        };
        append_brushes(&mut out, null_model);
        assert_eq!(out.brushes[0].model, u32::MAX);
    }

    #[test]
    fn original_library_interior_floor_and_authored_repeats_survive_cell_merge() {
        if !crate::pak_available() {
            return;
        }
        let bytes = aa_pak::Pak::global()
            .read("game/worlds/instance_library/cells/000_004/client/visareas.dat")
            .unwrap();
        let interior = parse_visareas_dat(&bytes).unwrap();
        assert_eq!(
            (
                interior.models.len(),
                interior.materials.len(),
                interior.brushes.len()
            ),
            (116, 75, 30286)
        );
        let keys: std::collections::HashSet<_> = interior
            .brushes
            .iter()
            .map(|b| (b.model, b.material, b.matrix.map(f32::to_bits)))
            .collect();
        assert_eq!(interior.brushes.len() - keys.len(), 153);
        let floor = interior
            .brushes
            .iter()
            .find(|b| {
                interior.models[b.model as usize].ends_with("library_floor_a.cgf")
                    && (b.translation() - Vec3::new(896.0, 128.0, 255.9970245)).length() < 0.001
            })
            .unwrap();
        assert!(interior.materials[floor.material as usize].ends_with("library3_floor_a"));
        assert!((floor.matrix[1] + 1.0).abs() < 1e-6 && (floor.matrix[4] - 1.0).abs() < 1e-6);
        assert!((floor.bbox[2] - 256.54242).abs() < 1e-4);
        let cell = super::super::load_cell_objects("instance_library", 0, 4);
        assert_eq!(cell.brushes.len(), 30287); // One exterior book, then every interior record once.
        assert!(cell.models[cell.brushes[0].model as usize].ends_with("library3_books_c.cgf"));
        for (loaded, original) in cell.brushes[1..].iter().zip(&interior.brushes) {
            assert_eq!(loaded.matrix, original.matrix);
            assert_eq!(loaded.bbox, original.bbox);
            assert_eq!(
                cell.models[loaded.model as usize],
                interior.models[original.model as usize]
            );
            assert_eq!(
                cell.materials[loaded.material as usize],
                interior.materials[original.material as usize]
            );
        }
        let bins = crate::objects::CellObjectData::new(cell, 120.0);
        assert_eq!(bins.brush_bins.iter().map(Vec::len).sum::<usize>(), 30287);
        assert!(bins.brush_bins.iter().filter(|b| !b.is_empty()).count() < 256);
    }

    #[test]
    fn original_outdoor_and_login_containers_have_valid_independent_tables() {
        if !crate::pak_available() {
            return;
        }
        for (world, x, y, count) in [
            ("main_world", 10, 14, 0),
            ("main_world", 18, 7, 234),
            ("login2", 0, 1, 0),
        ] {
            let path = format!(
                "{}/client/visareas.dat",
                crate::world::cell_dir(world, x, y)
            );
            let bytes = aa_pak::Pak::global().read(&path).unwrap();
            let interior = parse_visareas_dat(&bytes).unwrap_or_else(|e| panic!("{path}: {e}"));
            println!("{path}: {} interior brushes", interior.brushes.len());
            assert_eq!(interior.brushes.len(), count, "{path}");
        }
    }
}
