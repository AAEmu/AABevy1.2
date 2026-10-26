//! Writers for the cell placement files of [`crate::placement`], for editors.
//!
//! Each file is read into an editable record model that keeps every byte it does not understand, and written back
//! in the original binary layout: parse -> write gives the identical bytes for an unmodified file.
//!
//! * [`SectorFile`]: `brush.dat` (256 sectors of 64 m, 128 B records), `vegetation.dat` (16 sectors of 256 m,
//!   64 B records) and the table-less `vegetation/SX_SY.dat` (one 256 m sector);
//! * [`ObjectTree`]: the `object.dat` octree (tables verbatim, nodes with their typed items);
//! * [`entities_xml`]: span edits of `entities.xml` (`Pos` / `Rotate` / `Scale`, remove or copy an `<Entity>`).
//!
//! Records keep their slot when deleted (`deleted = true`, skipped on write), so indices stay stable while editing.

use bevy::math::{Mat4, Quat, Vec3};

use crate::placement::{Brush, Vegetation};
use crate::reader::{f32_at, u32_at};

/// One record of a [`SectorFile`] or item of an [`ObjectTree`].
#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    /// Sector of the table (`sx * 16 + sy` in `brush.dat`, `sx * 4 + sy` in `vegetation.dat`, 0 without a table).
    pub sector: u16,
    pub bytes: Vec<u8>,
    pub deleted: bool,
}

/// `brush.dat`, `vegetation.dat` or `vegetation/SX_SY.dat`.
#[derive(Clone, Debug, PartialEq)]
pub struct SectorFile {
    pub version: u32,
    /// Sectors per row of the table (16 or 4); 0: no table (`vegetation/SX_SY.dat`).
    pub side: usize,
    /// Sector edge in metres.
    pub sector_size: f32,
    /// Record size in bytes.
    pub record: usize,
    /// In file order (by sector, then order within the sector).
    pub records: Vec<Record>,
    /// Origin of a table-less file (cell-local), e.g. `(256, 512, 0)` for `vegetation/01_02.dat`.
    pub origin: Vec3,
}

impl SectorFile {
    pub fn parse_brushes(data: &[u8]) -> Option<Self> {
        Self::parse_table(data, 16, crate::coords::SECTOR_SIZE, 128)
    }

    pub fn parse_vegetation(data: &[u8]) -> Option<Self> {
        Self::parse_table(data, 4, 256.0, 64)
    }

    /// `vegetation/{sx:02}_{sy:02}.dat`.
    pub fn parse_vegetation_sector(data: &[u8], sx: usize, sy: usize) -> Option<Self> {
        if data.len() < 4 || !(data.len() - 4).is_multiple_of(64) {
            return None;
        }
        let records = data[4..].as_chunks::<64>().0.iter().map(|b| Record { sector: 0, bytes: b.to_vec(), deleted: false }).collect();
        Some(SectorFile {
            version: u32_at(data, 0),
            side: 0,
            sector_size: 256.0,
            record: 64,
            records,
            origin: Vec3::new(sx as f32 * 256.0, sy as f32 * 256.0, 0.0),
        })
    }

    fn parse_table(data: &[u8], side: usize, sector_size: f32, record: usize) -> Option<Self> {
        let n = side * side;
        let head = 4 + n * 8;
        if data.len() < head {
            return None;
        }
        let mut records = Vec::new();
        for i in 0..n {
            let off = u32_at(data, 4 + i * 4) as usize;
            let size = u32_at(data, 4 + n * 4 + i * 4) as usize;
            if size == 0 {
                continue;
            }
            if !size.is_multiple_of(record) {
                return None;
            }
            let block = data.get(off..off.checked_add(size)?)?;
            records.extend(block.chunks_exact(record).map(|b| Record { sector: i as u16, bytes: b.to_vec(), deleted: false }));
        }
        Some(SectorFile { version: u32_at(data, 0), side, sector_size, record, records, origin: Vec3::ZERO })
    }

    /// The file's bytes: header, offset and size tables, then the live records grouped by sector (file order kept
    /// within a sector).
    pub fn write(&self) -> Vec<u8> {
        let mut out = self.version.to_le_bytes().to_vec();
        if self.side == 0 {
            for r in self.records.iter().filter(|r| !r.deleted) {
                out.extend_from_slice(&r.bytes);
            }
            return out;
        }
        let n = self.side * self.side;
        let head = 4 + n * 8;
        let mut blocks: Vec<Vec<u8>> = vec![Vec::new(); n];
        for r in self.records.iter().filter(|r| !r.deleted) {
            blocks[(r.sector as usize).min(n - 1)].extend_from_slice(&r.bytes);
        }
        let mut offset = head;
        let mut sizes = Vec::with_capacity(n);
        for b in &blocks {
            out.extend_from_slice(&(offset as u32).to_le_bytes());
            sizes.push(b.len() as u32);
            offset += b.len();
        }
        for s in sizes {
            out.extend_from_slice(&s.to_le_bytes());
        }
        for b in blocks {
            out.extend(b);
        }
        out
    }

    /// Cell-local origin of a sector (the records' coordinates are relative to it).
    pub fn sector_origin(&self, sector: u16) -> Vec3 {
        if self.side == 0 {
            return self.origin;
        }
        let s = sector as usize;
        Vec3::new((s / self.side) as f32 * self.sector_size, (s % self.side) as f32 * self.sector_size, 0.0)
    }

    /// The table sector holding a cell-local position (0 for a table-less file).
    pub fn sector_at(&self, p: Vec3) -> u16 {
        if self.side == 0 {
            return 0;
        }
        let max = (self.side - 1) as f32;
        let sx = (p.x / self.sector_size).floor().clamp(0.0, max) as usize;
        let sy = (p.y / self.sector_size).floor().clamp(0.0, max) as usize;
        (sx * self.side + sy) as u16
    }
}

fn put_f32(b: &mut [u8], at: usize, v: f32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

/// The bounding box (`f32[6]` at 0) of a brush / vegetation / decal record, made cell-local.
pub fn record_bbox(rec: &[u8], origin: Vec3) -> [f32; 6] {
    std::array::from_fn(|i| f32_at(rec, i * 4) + origin[i % 3])
}

fn put_bbox(rec: &mut [u8], bbox: &[f32; 6], origin: Vec3) {
    for (i, v) in bbox.iter().enumerate() {
        put_f32(rec, i * 4, v - origin[i % 3]);
    }
}

/// Writes a brush's matrix and bounding box (both cell-local in `b`) into its 128 B record, relative to `origin`
/// (the sector origin in `brush.dat`, zero in `object.dat`). Other fields are kept.
pub fn set_brush(rec: &mut [u8], b: &Brush, origin: Vec3) {
    for (i, v) in b.matrix.iter().enumerate() {
        let o = match i {
            3 => origin.x,
            7 => origin.y,
            11 => origin.z,
            _ => 0.0,
        };
        put_f32(rec, 67 + i * 4, v - o);
    }
    put_bbox(rec, &b.bbox, origin);
}

/// Writes a vegetation instance (position, scale, angles; cell-local) and its cell-local box into its 64 B record.
pub fn set_vegetation(rec: &mut [u8], v: &Vegetation, bbox: &[f32; 6], origin: Vec3) {
    let p = v.pos - origin;
    put_f32(rec, 39, p.x);
    put_f32(rec, 43, p.y);
    put_f32(rec, 47, p.z);
    put_f32(rec, 51, v.scale);
    rec[57] = v.angle_x;
    rec[58] = v.angle_y;
    rec[60] = v.angle_z;
    put_bbox(rec, bbox, origin);
}

/// The axis-aligned box of `bbox` moved by the transform `delta` (cell-local cry space).
pub fn transform_bbox(bbox: &[f32; 6], delta: Mat4) -> [f32; 6] {
    let (lo, hi) = (Vec3::from_slice(&bbox[..3]), Vec3::from_slice(&bbox[3..]));
    let mut a = Vec3::splat(f32::MAX);
    let mut b = Vec3::splat(f32::MIN);
    for i in 0..8 {
        let c = Vec3::new(
            if i & 1 == 0 { lo.x } else { hi.x },
            if i & 2 == 0 { lo.y } else { hi.y },
            if i & 4 == 0 { lo.z } else { hi.z },
        );
        let p = delta.transform_point3(c);
        a = a.min(p);
        b = b.max(p);
    }
    [a.x, a.y, a.z, b.x, b.y, b.z]
}

/// A node of the `object.dat` octree.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    /// `u32 version, f32 box[6]`.
    pub header: Vec<u8>,
    pub mask: u8,
    /// Indices into [`ObjectTree::items`].
    pub items: Vec<usize>,
    /// Bytes of the objects block after an item of unknown size (kept verbatim).
    pub rest: Vec<u8>,
    /// Indices into [`ObjectTree::nodes`], one per mask bit.
    pub children: Vec<usize>,
}

/// An object of the octree: `u32 type` and its payload.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub kind: u32,
    pub bytes: Vec<u8>,
    pub deleted: bool,
}

/// `object.dat`: the model and material tables verbatim, then the octree.
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectTree {
    pub head: Vec<u8>,
    /// `nodes[0]` is the root.
    pub nodes: Vec<Node>,
    pub items: Vec<Item>,
    /// Bytes after the root node.
    pub tail: Vec<u8>,
}

/// Payload size of an octree item at the start of `b`, by type (the sizes [`crate::placement::parse_object_dat`]
/// reads); `None` for an unknown type or a truncated item.
fn item_size(kind: u32, b: &[u8]) -> Option<usize> {
    let size = match kind {
        1 => 128,
        2 => 64,
        9 => 111,
        14 => 79,
        13 => {
            b.get(..63)?;
            63usize.checked_add((u32_at(b, 39) as usize).checked_mul(12)?)?
        }
        11 => {
            b.get(..119)?;
            let count = (u32_at(b, 103) as usize).checked_add(u32_at(b, 115) as usize)?;
            119usize.checked_add(count.checked_mul(12)?)?
        }
        6 => {
            let mut at = 198_775usize;
            b.get(..at)?;
            if u32_at(b, 67) & 0x20 != 0 {
                let count = u32_at(b.get(at..at + 4)?, 0) as usize;
                at += 4;
                for _ in 0..count {
                    let size = u32_at(b.get(at..at + 4)?, 0) as usize;
                    at = at.checked_add(4)?.checked_add(size)?;
                }
            }
            at
        }
        _ => return None,
    };
    (size <= b.len()).then_some(size)
}

impl ObjectTree {
    pub fn parse(data: &[u8]) -> Option<Self> {
        let n = u32_at(data.get(..4)?, 0) as usize;
        let mut pos = 4usize.checked_add(n.checked_mul(260)?)?;
        let m = u32_at(data.get(pos..pos + 4)?, 0) as usize;
        pos = pos.checked_add(4)?.checked_add(m.checked_mul(260)?)?;
        let head = data.get(..pos)?.to_vec();
        let mut tree = ObjectTree { head, nodes: Vec::new(), items: Vec::new(), tail: Vec::new() };
        let end = tree.read_node(data, pos, 0)?;
        tree.tail = data[end..].to_vec();
        Some(tree)
    }

    /// Reads the node at `pos`, returns the position after it and its subtree.
    fn read_node(&mut self, data: &[u8], pos: usize, depth: usize) -> Option<usize> {
        if depth >= 64 || u32_at(data.get(pos..pos + 4)?, 0) != 2 {
            return None;
        }
        let header = data.get(pos..pos + 28)?.to_vec();
        let bytes = u32_at(data.get(pos + 28..pos + 32)?, 0) as usize;
        let mask = *data.get(pos + 32)?;
        let start = pos + 33;
        let block = data.get(start..start.checked_add(bytes)?)?;
        let id = self.nodes.len();
        self.nodes.push(Node { header, mask, items: Vec::new(), rest: Vec::new(), children: Vec::new() });
        let mut at = 0;
        while at < block.len() {
            let Some(kind) = block.get(at..at + 4).map(|b| u32_at(b, 0)) else { break };
            let Some(size) = item_size(kind, &block[at + 4..]) else { break };
            self.nodes[id].items.push(self.items.len());
            self.items.push(Item { kind, bytes: block[at + 4..at + 4 + size].to_vec(), deleted: false });
            at += 4 + size;
        }
        self.nodes[id].rest = block[at..].to_vec();
        let mut next = start + bytes;
        for bit in 0..8 {
            if mask >> bit & 1 != 0 {
                let child = self.nodes.len();
                self.nodes[id].children.push(child);
                next = self.read_node(data, next, depth + 1)?;
            }
        }
        Some(next)
    }

    pub fn write(&self) -> Vec<u8> {
        let mut out = self.head.clone();
        if !self.nodes.is_empty() {
            self.write_node(0, &mut out);
        }
        out.extend_from_slice(&self.tail);
        out
    }

    fn write_node(&self, id: usize, out: &mut Vec<u8>) {
        let node = &self.nodes[id];
        out.extend_from_slice(&node.header);
        let mut block = Vec::new();
        for item in node.items.iter().map(|&i| &self.items[i]).filter(|i| !i.deleted) {
            block.extend_from_slice(&item.kind.to_le_bytes());
            block.extend_from_slice(&item.bytes);
        }
        block.extend_from_slice(&node.rest);
        out.extend_from_slice(&(block.len() as u32).to_le_bytes());
        out.push(node.mask);
        out.extend(block);
        for &c in &node.children {
            self.write_node(c, out);
        }
    }

    /// Item indices of one type in walk order (node items, then the children), the order
    /// [`crate::placement::parse_object_dat`] lists them in.
    pub fn items_of(&self, kind: u32) -> Vec<usize> {
        let mut out = Vec::new();
        if !self.nodes.is_empty() {
            self.collect(0, kind, &mut out);
        }
        out
    }

    fn collect(&self, id: usize, kind: u32, out: &mut Vec<usize>) {
        let node = &self.nodes[id];
        out.extend(node.items.iter().copied().filter(|&i| self.items[i].kind == kind && !self.items[i].deleted));
        for &c in &node.children {
            self.collect(c, kind, out);
        }
    }

    /// Adds an item to the root node (its box covers the cell); returns its index.
    pub fn push(&mut self, kind: u32, bytes: Vec<u8>) -> usize {
        let i = self.items.len();
        self.items.push(Item { kind, bytes, deleted: false });
        if let Some(root) = self.nodes.first_mut() {
            // before the unreadable rest of the block, so readers still see it
            root.items.push(i);
        }
        i
    }
}

/// Span edits of a cell's `entities.xml`. Entities are found by `EntityGuid` among the direct `<Objects>` children;
/// everything else of the text is kept as it is.
pub mod entities_xml {
    use super::*;

    fn bom(xml: &str) -> usize {
        if xml.starts_with('\u{feff}') { '\u{feff}'.len_utf8() } else { 0 }
    }

    fn find<'a, 'input>(doc: &'a roxmltree::Document<'input>, guid: u64) -> Option<roxmltree::Node<'a, 'input>> {
        let objects = doc.root_element().children().find(|n| n.has_tag_name("Objects"))?;
        let mut found = objects
            .children()
            .filter(|n| n.has_tag_name("Entity") && n.attribute("EntityGuid").and_then(crate::entities::guid) == Some(guid));
        let node = found.next()?;
        // an ambiguous guid is not edited
        found.next().is_none().then_some(node)
    }

    fn floats(v: Option<&str>) -> Vec<f32> {
        v.map(|s| s.split(',').filter_map(|p| p.trim().parse().ok()).collect()).unwrap_or_default()
    }

    /// The entity's own (parent-relative) `Pos`, `Rotate` (`w,x,y,z`) and `Scale`, with the file's defaults.
    pub fn local(xml: &str, guid: u64) -> Option<(Vec3, Quat, Vec3)> {
        let doc = roxmltree::Document::parse(&xml[bom(xml)..]).ok()?;
        let e = find(&doc, guid)?;
        let pos = match floats(e.attribute("Pos"))[..] {
            [x, y, z] => Vec3::new(x, y, z),
            _ => Vec3::ZERO,
        };
        let rot = match floats(e.attribute("Rotate"))[..] {
            [w, x, y, z] => Quat::from_xyzw(x, y, z, w),
            _ => Quat::IDENTITY,
        };
        let scale = match floats(e.attribute("Scale"))[..] {
            [x, y, z] => Vec3::new(x, y, z),
            _ => Vec3::ONE,
        };
        Some((pos, rot, scale))
    }

    /// Whether another entity names this one as its `ParentGUID`.
    pub fn has_children(xml: &str, guid: u64) -> bool {
        let Ok(doc) = roxmltree::Document::parse(&xml[bom(xml)..]) else { return false };
        doc.descendants()
            .any(|n| n.has_tag_name("Entity") && n.attribute("ParentGUID").and_then(crate::entities::guid) == Some(guid))
    }

    fn fmt(v: &[f32]) -> String {
        v.iter().map(|f| format!("{f}")).collect::<Vec<_>>().join(",")
    }

    /// Sets `Pos`, `Rotate` and `Scale` of the entity (parent-relative values); attributes it lacks are added.
    pub fn set_transform(xml: &str, guid: u64, pos: Vec3, rot: Quat, scale: Vec3) -> Option<String> {
        let off = bom(xml);
        let doc = roxmltree::Document::parse(&xml[off..]).ok()?;
        let e = find(&doc, guid)?;
        let values = [
            ("Pos", fmt(&pos.to_array())),
            ("Rotate", fmt(&[rot.w, rot.x, rot.y, rot.z])),
            ("Scale", fmt(&scale.to_array())),
        ];
        // (range, replacement), applied back to front
        let mut edits: Vec<(std::ops::Range<usize>, String)> = Vec::new();
        let insert_at = e.range().start + 1 + e.tag_name().name().len();
        for (name, value) in values {
            match e.attributes().find(|a| a.name() == name) {
                Some(a) => edits.push((a.range_value(), value)),
                None => edits.push((insert_at..insert_at, format!(" {name}=\"{value}\""))),
            }
        }
        edits.sort_by_key(|(r, _)| std::cmp::Reverse(r.start));
        let mut out = xml.to_string();
        for (r, v) in edits {
            out.replace_range(r.start + off..r.end + off, &v);
        }
        Some(out)
    }

    /// The text span of the entity's element including the indentation before it and the line break.
    fn element_span(xml: &str, node: roxmltree::Node) -> std::ops::Range<usize> {
        let r = node.range();
        let bytes = xml.as_bytes();
        let mut start = r.start;
        while start > 0 && matches!(bytes[start - 1], b' ' | b'\t') {
            start -= 1;
        }
        if start > 0 && bytes[start - 1] == b'\n' {
            start -= 1;
            if start > 0 && bytes[start - 1] == b'\r' {
                start -= 1;
            }
        }
        start..r.end
    }

    /// Removes the entity's element.
    pub fn remove(xml: &str, guid: u64) -> Option<String> {
        let off = bom(xml);
        let body = &xml[off..];
        let doc = roxmltree::Document::parse(body).ok()?;
        let e = find(&doc, guid)?;
        let span = element_span(body, e);
        let mut out = xml.to_string();
        out.replace_range(span.start + off..span.end + off, "");
        Some(out)
    }

    /// Inserts a copy of the entity's element right after it, with `EntityGuid` = `new_guid` (and `EntityId`
    /// dropped: the engine assigns one).
    pub fn copy(xml: &str, guid: u64, new_guid: u64) -> Option<String> {
        let off = bom(xml);
        let body = &xml[off..];
        let doc = roxmltree::Document::parse(body).ok()?;
        let e = find(&doc, guid)?;
        let span = element_span(body, e);
        let r = e.range();
        let mut element = body[span.clone()].to_string();
        // attribute edits inside the copy, relative to its start
        let base = span.start;
        let mut edits: Vec<(std::ops::Range<usize>, String)> = Vec::new();
        for a in e.attributes() {
            match a.name() {
                "EntityGuid" => edits.push((a.range_value(), format!("{new_guid:x}"))),
                "EntityId" => {
                    let mut ar = a.range();
                    ar.start -= 1; // the space before it
                    edits.push((ar, String::new()));
                }
                _ => {}
            }
        }
        edits.sort_by_key(|(r, _)| std::cmp::Reverse(r.start));
        for (ar, v) in edits {
            element.replace_range(ar.start - base..ar.end - base, &v);
        }
        if !element.starts_with(['\r', '\n']) {
            element.insert(0, '\n');
        }
        let mut out = xml.to_string();
        out.insert_str(r.end + off, &element);
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::placement::{parse_brush_dat, parse_object_dat, parse_vegetation_dat, parse_vegetation_sector};

    const CELLS: [&str; 8] = ["015_015", "009_012", "010_014", "012_014", "021_008", "013_010", "014_013", "010_011"];

    fn read(world: &str, cell: &str, file: &str) -> Option<Vec<u8>> {
        aa_pak::Pak::global().read(&format!("worlds/{world}/cells/{cell}/client/{file}")).ok()
    }

    #[test]
    fn real_cells_round_trip_bytes_identical() {
        if !crate::pak_available() {
            return;
        }
        let mut files = 0;
        for (world, cell) in CELLS.iter().map(|c| ("main_world", *c)).chain([("login2", "000_001")]) {
            if let Some(d) = read(world, cell, "brush.dat") {
                let f = SectorFile::parse_brushes(&d).expect("brush.dat");
                assert!(f.write() == d, "{world}/{cell} brush.dat");
                files += 1;
            }
            if let Some(d) = read(world, cell, "vegetation.dat") {
                let f = SectorFile::parse_vegetation(&d).expect("vegetation.dat");
                assert!(f.write() == d, "{world}/{cell} vegetation.dat");
                files += 1;
            }
            for sx in 0..4 {
                for sy in 0..4 {
                    if let Some(d) = read(world, cell, &format!("vegetation/{sx:02}_{sy:02}.dat")) {
                        let f = SectorFile::parse_vegetation_sector(&d, sx, sy).expect("vegetation sector");
                        assert!(f.write() == d, "{world}/{cell} vegetation/{sx:02}_{sy:02}.dat");
                        files += 1;
                    }
                }
            }
            if let Some(d) = read(world, cell, "object.dat") {
                let t = ObjectTree::parse(&d).expect("object.dat");
                assert!(t.write() == d, "{world}/{cell} object.dat");
                // the tree lists the same objects as the reader
                if let Ok(o) = parse_object_dat(&d) {
                    assert_eq!(t.items_of(1).len(), o.brushes.len(), "{world}/{cell}");
                    assert_eq!(t.items_of(2).len(), o.vegetation.len(), "{world}/{cell}");
                    assert_eq!(t.items_of(9).len(), o.decals.len(), "{world}/{cell}");
                }
                files += 1;
            }
        }
        assert!(files > 15, "{files}");
    }

    /// Every cell of the main world: brush.dat, vegetation.dat and object.dat write back identical (about 5 s).
    #[test]
    fn all_main_world_cells_round_trip() {
        if !crate::pak_available() {
            return;
        }
        let (mut cells, mut bad) = (0, Vec::new());
        for x in 0..40 {
            for y in 0..40 {
                let cell = format!("{x:03}_{y:03}");
                let mut any = false;
                for file in ["brush.dat", "vegetation.dat", "object.dat"] {
                    let Some(d) = read("main_world", &cell, file) else { continue };
                    any = true;
                    let out = match file {
                        "brush.dat" => SectorFile::parse_brushes(&d).map(|f| f.write()),
                        "vegetation.dat" => SectorFile::parse_vegetation(&d).map(|f| f.write()),
                        _ => ObjectTree::parse(&d).map(|t| t.write()),
                    };
                    if out.as_deref() != Some(&d[..]) {
                        bad.push(format!("{cell}/{file}"));
                    }
                }
                cells += usize::from(any);
            }
        }
        assert!(bad.is_empty(), "{} of {cells} cells differ: {bad:?}", bad.len());
        assert!(cells > 100, "{cells}");
    }

    #[test]
    fn delete_and_move_survive_write_and_parse() {
        if !crate::pak_available() {
            return;
        }
        let d = read("main_world", "015_015", "brush.dat").unwrap();
        let before = parse_brush_dat(&d).unwrap();
        let mut f = SectorFile::parse_brushes(&d).unwrap();
        // delete the first, move the second into another sector
        f.records[0].deleted = true;
        let mut b = before[1].clone();
        let delta = Mat4::from_translation(Vec3::new(130.0, -70.0, 2.5));
        b.matrix[3] += 130.0;
        b.matrix[7] -= 70.0;
        b.matrix[11] += 2.5;
        b.bbox = transform_bbox(&b.bbox, delta);
        let sector = f.sector_at(b.translation());
        let mut rec = f.records[1].bytes.clone();
        set_brush(&mut rec, &b, f.sector_origin(sector));
        f.records[1] = Record { sector, bytes: rec, deleted: false };
        let after = parse_brush_dat(&f.write()).unwrap();
        assert_eq!(after.len(), before.len() - 1);
        let moved = after.iter().find(|x| x.model == b.model && (x.translation() - b.translation()).length() < 1e-3);
        assert!(moved.is_some(), "moved brush not found at {}", b.translation());
        assert!(!after.iter().any(|x| (x.translation() - before[0].translation()).length() < 1e-4 && x.model == before[0].model));

        // vegetation: delete one, move one, re-read
        let d = read("main_world", "015_015", "vegetation.dat").unwrap();
        let before = parse_vegetation_dat(&d).unwrap();
        let mut f = SectorFile::parse_vegetation(&d).unwrap();
        f.records[3].deleted = true;
        let mut v = before[4].clone();
        v.pos += Vec3::new(300.0, 10.0, -1.0);
        v.angle_z = v.angle_z.wrapping_add(32);
        let origin = f.sector_origin(f.records[4].sector);
        let bbox = transform_bbox(&record_bbox(&f.records[4].bytes, origin), Mat4::from_translation(Vec3::new(300.0, 10.0, -1.0)));
        let sector = f.sector_at(v.pos);
        let mut rec = f.records[4].bytes.clone();
        set_vegetation(&mut rec, &v, &bbox, f.sector_origin(sector));
        f.records[4] = Record { sector, bytes: rec, deleted: false };
        let after = parse_vegetation_dat(&f.write()).unwrap();
        assert_eq!(after.len(), before.len() - 1);
        assert!(after.iter().any(|x| (x.pos - v.pos).length() < 1e-3 && x.angle_z == v.angle_z && x.group == v.group));
        assert!(!after.iter().any(|x| (x.pos - before[3].pos).length() < 1e-5));

        // octree: delete a vegetation item and add a brush copy
        for cell in CELLS {
            let Some(d) = read("main_world", cell, "object.dat") else { continue };
            let Ok(o) = parse_object_dat(&d) else { continue };
            let mut t = ObjectTree::parse(&d).unwrap();
            let (veg, brushes) = (t.items_of(2), t.items_of(1));
            if veg.is_empty() || brushes.is_empty() {
                continue;
            }
            t.items[veg[0]].deleted = true;
            let copy = t.items[brushes[0]].bytes.clone();
            t.push(1, copy);
            let after = parse_object_dat(&t.write()).unwrap();
            assert_eq!(after.vegetation.len(), o.vegetation.len() - 1);
            assert_eq!(after.brushes.len(), o.brushes.len() + 1);
            assert_eq!(after.decals.len(), o.decals.len());
            assert_eq!(after.water.len(), o.water.len());
            return;
        }
        panic!("no cell with octree vegetation and brushes");
    }

    #[test]
    fn vegetation_sector_files_round_trip() {
        if !crate::pak_available() {
            return;
        }
        // dense-forest sector files of two cells (one nearly empty, one large)
        let pak = aa_pak::Pak::global();
        let mut edited = 0;
        for cell in ["018_003", "019_002"] {
            for sx in 0..4 {
                for sy in 0..4 {
                    let path = format!("worlds/main_world/cells/{cell}/client/vegetation/{sx:02}_{sy:02}.dat");
                    let Ok(d) = pak.read(&path) else { continue };
                    let mut f = SectorFile::parse_vegetation_sector(&d, sx, sy).unwrap();
                    assert!(f.write() == d, "{path}");
                    let before = parse_vegetation_sector(&d, sx, sy);
                    if before.is_empty() {
                        continue;
                    }
                    f.records[0].deleted = true;
                    assert_eq!(parse_vegetation_sector(&f.write(), sx, sy).len(), before.len() - 1);
                    edited += 1;
                }
            }
        }
        assert!(edited >= 2, "{edited}");
    }

    #[test]
    fn entity_xml_edits() {
        let xml = "\u{feff}<Mission>\n <Objects>\n  <Entity Name=\"a\" Pos=\"1,2,3\" EntityClass=\"BasicEntity\" EntityId=\"7\" EntityGuid=\"ab\">\n   <Properties object_Model=\"x.cgf\"/>\n  </Entity>\n  <Entity Name=\"b\" EntityClass=\"GeomEntity\" Geometry=\"y.cgf\" EntityGuid=\"cd\"/>\n </Objects>\n</Mission>\n";
        let moved = entities_xml::set_transform(xml, 0xab, Vec3::new(4.0, 5.5, 6.0), Quat::IDENTITY, Vec3::ONE).unwrap();
        assert!(moved.contains("Pos=\"4,5.5,6\""), "{moved}");
        assert!(moved.contains("Rotate=\"1,0,0,0\"") && moved.contains("Scale=\"1,1,1\""));
        let props = crate::entities::parse_static_entities(&moved);
        assert_eq!(props.len(), 2);
        assert_eq!(props[0].translation(), Vec3::new(4.0, 5.5, 6.0));
        let gone = entities_xml::remove(&moved, 0xcd).unwrap();
        assert_eq!(crate::entities::parse_static_entities(&gone).len(), 1);
        assert!(gone.starts_with('\u{feff}') && gone.ends_with("</Mission>\n"));
        let copied = entities_xml::copy(&gone, 0xab, 0x1234).unwrap();
        let props = crate::entities::parse_static_entities(&copied);
        assert_eq!(props.len(), 2);
        assert_eq!(props[1].guid, 0x1234);
        assert!(!copied[copied.find("1234").unwrap()..].contains("EntityId"));
        assert_eq!(entities_xml::local(&copied, 0x1234).unwrap().0, Vec3::new(4.0, 5.5, 6.0));
        // untouched text stays identical
        let same = entities_xml::set_transform(xml, 0xab, Vec3::new(1.0, 2.0, 3.0), Quat::IDENTITY, Vec3::ONE).unwrap();
        assert_eq!(same.replace(" Rotate=\"1,0,0,0\"", "").replace(" Scale=\"1,1,1\"", ""), xml);
    }
}
