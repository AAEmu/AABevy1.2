//! Object placement files of a cell (`cells/XXX_YYY/client/`): `statobjs.dat`, `material_list.dat`,
//! `brush.dat`, `vegetation.dat` and the octree `object.dat`.
//!
//! All positions returned are **cell-local, cry space** (metres, 0..1024, Z up).
//!
//! ```text
//! statobjs.dat       u32 n; n x char[256]            model (.cgf) paths, the brush "type" table
//! material_list.dat  u32 n; n x { u32; char[256] }   material (.mtl, no extension) names
//! brush.dat          u32 version(1); u32 offset[256]; u32 size[256]; 128 B records; sector = sx*16+sy (64 m)
//! vegetation.dat     u32 version(1); u32 offset[16];  u32 size[16];  64 B records;  sector = sx*4+sy (256 m)
//! object.dat         u32 n; n x { u32; char[256] } model table, u32 m; m x { u32; char[256] } materials,
//!                    then an octree: node { u32 version, f32 box[6], u32 objectsBlockSize, u8 childMask },
//!                    objects block = repeated { u32 type, payload }, then one child node per mask bit.
//!                    types: 1 brush (128 B), 2 vegetation (64 B), 9 decal (111 B), 13 road (63 B + 12 n),
//!                    11 water volume (the rest of the block, see [`Water`]), 6 and 14 (rest of block, ignored).
//! ```
//!
//! Brush record (128 B, the engine's `SBrushChunk`): `f32 bbox[6]` @0, `f32 maxViewDist` @28, `u32 rndFlags` @32,
//! `u8 viewDistRatio, lodRatio, internalFlags` @36, **`f32 m[12]` @67 row-major 3 x 4 (rotation*scale | translation)**,
//! `i32 materialId` @115, `i32 editorId` @119, `i32 statObjId` @123, `u8 materialLayers` @127. In
//! brush.dat the matrix and box are sector-local; in object.dat they are cell-local.
//!
//! Vegetation record (64 B, `SVegetationChunk`): same node header (bbox @0, maxViewDist @28, rndFlags @32,
//! ratios @36), `f32 pos[3]` @39, `f32 scale` @51, `u16 group` @55 (a `vegetation.xml` `<group id>`),
//! `u8 angleX, angleY` @57, 58, `u8 bright` @59, `u8 angleZ` @60 (256 steps per turn), 61..63 colour/pad.
//! brush.dat and object.dat carry the same brushes; object.dat additionally holds decals and roads
//! and (in worlds without the dedicated files, e.g. login2) everything.

use bevy::math::{Mat4, Quat, Vec3};

use crate::coords::SECTOR_SIZE;
use crate::reader::{Reader, cstr, f32_at, i32_at, u16_at, u32_at};

#[derive(Clone, Debug)]
pub struct Brush {
    /// Row-major 3 x 4: `[r00 r01 r02 tx, r10 r11 r12 ty, r20 r21 r22 tz]`, cell-local cry space.
    pub matrix: [f32; 12],
    /// Index into [`CellObjects::models`].
    pub model: u32,
    /// Index into [`CellObjects::materials`], or -1.
    pub material: i32,
    pub flags: u32,
    pub max_view_dist: f32,
    /// Cell-local bounding box `[min xyz, max xyz]`.
    pub bbox: [f32; 6],
}

impl Brush {
    pub fn translation(&self) -> Vec3 {
        Vec3::new(self.matrix[3], self.matrix[7], self.matrix[11])
    }

    /// Cry-space object -> cell matrix.
    pub fn mat4(&self) -> Mat4 {
        let m = &self.matrix;
        Mat4::from_cols_array(&[
            m[0], m[4], m[8], 0.0, //
            m[1], m[5], m[9], 0.0, //
            m[2], m[6], m[10], 0.0, //
            m[3], m[7], m[11], 1.0,
        ])
    }
}

#[derive(Clone, Debug)]
pub struct Vegetation {
    /// Cell-local cry position.
    pub pos: Vec3,
    pub scale: f32,
    /// `vegetation.xml` group id.
    pub group: u16,
    pub angle_x: u8,
    pub angle_y: u8,
    pub bright: u8,
    pub angle_z: u8,
    pub flags: u32,
    pub max_view_dist: f32,
}

impl Vegetation {
    /// Rotation about Z (up), radians.
    pub fn yaw(&self) -> f32 {
        self.angle_z as f32 * std::f32::consts::TAU / 256.0
    }

    /// Cry-space transform (cell-local): scale, tilt, yaw, translation.
    pub fn mat4(&self) -> Mat4 {
        let tau = std::f32::consts::TAU / 256.0;
        let rot = Quat::from_euler(
            bevy::math::EulerRot::ZYX,
            self.angle_z as f32 * tau,
            self.angle_y as f32 * tau,
            self.angle_x as f32 * tau,
        );
        Mat4::from_scale_rotation_translation(Vec3::splat(self.scale.clamp(0.1, 5.0)), rot, self.pos)
    }
}

/// A terrain decal (`SDecalChunk`, 111 B). Kept raw: position, size and material id.
#[derive(Clone, Debug)]
pub struct Decal {
    pub raw: Vec<u8>,
}

/// A terrain road (`SRoadChunk_NEW`, 63 B followed by paired left/right spline edges).
/// Points and bounding box are cell-local, cry space. Z on the spline is only a bound: the
/// engine clips terrain triangles against each four-point trapezoid, not a floating strip.
#[derive(Clone, Debug)]
pub struct Road {
    pub bbox: [f32; 6],
    pub max_view_dist: f32,
    pub flags: u32,
    pub tex_coords: [f32; 2],
    /// UV bounds of the whole spline (a spline is split across several road objects).
    pub global_tex_coords: [f32; 2],
    pub material: u32,
    pub sort_priority: u8,
    pub points: Vec<Vec3>,
}

fn read_road(head: &[u8], points: &[u8]) -> Road {
    let packed_material = u32_at(head, 59);
    Road {
        bbox: std::array::from_fn(|i| f32_at(head, i * 4)),
        max_view_dist: f32_at(head, 28),
        flags: u32_at(head, 32),
        tex_coords: [f32_at(head, 43), f32_at(head, 47)],
        global_tex_coords: [f32_at(head, 51), f32_at(head, 55)],
        material: packed_material & 0x00ff_ffff,
        sort_priority: (packed_material >> 24) as u8,
        points: points.chunks_exact(12).map(|p| Vec3::new(f32_at(p, 0), f32_at(p, 4), f32_at(p, 8))).collect(),
    }
}

/// A water volume (object type 11, the whole objects block): rivers are chains of 4-point quads, lakes and
/// ponds a ring of 8..30 points. Cell-local, cry space.
///
/// ```text
/// f32 bbox[6] @0 (z range = [surface - depth, surface]), f32 maxViewDist @28, u8 viewDistRatio,lodRatio @36,
/// u32 @39, u64 editor guid @43, ... header up to @71, then 12-byte records:
///   (0,0,1) plane normal | (-surface, ?, ?) | (uv scale x, y, 0) | (depth, 0, 0) | polygon points (x, y, z)...
/// ```
///
/// Lakes list their ring twice and some variants append more data after the points; the points are read while
/// they stay inside the box and stop at the first record that does not (or at a repeat of the first point).
#[derive(Clone, Debug)]
pub struct Water {
    pub bbox: [f32; 6],
    /// Height of the water surface.
    pub surface: f32,
    pub depth: f32,
    pub uv_scale: [f32; 2],
    pub points: Vec<Vec3>,
    /// `u32 @39`: the volume type (low 2 bits: 2 = area, 3 = river; higher bits are flags).
    pub kind: u32,
    /// `u32 @51`: index of the volume's material in the cell's material list (`game/materials/ocean/..`).
    pub material: u32,
    /// `f32 @55` and `f32[3] @59`: density and colour (linear) of the water's own fog.
    pub fog_density: f32,
    pub fog_color: [f32; 3],
}

/// Parses the payload of a type 11 item.
pub fn parse_water(b: &[u8]) -> Option<Water> {
    if b.len() < 119 {
        return None;
    }
    let mut bbox = [0.0f32; 6];
    for (i, v) in bbox.iter_mut().enumerate() {
        *v = f32_at(b, i * 4);
    }
    if !bbox.iter().all(|v| v.is_finite()) {
        return None;
    }
    let normal = Vec3::new(f32_at(b, 71), f32_at(b, 75), f32_at(b, 79));
    let surface = if normal.z > 0.99 { -f32_at(b, 83) } else { bbox[5] };
    let surface = if (surface - bbox[5]).abs() < 2.0 { surface } else { bbox[5] };
    let uv_scale = [f32_at(b, 95), f32_at(b, 99)];
    let depth = f32_at(b, 107).max(0.0);
    let ok = |p: Vec3| {
        p.x > bbox[0] - 12.0
            && p.x < bbox[3] + 12.0
            && p.y > bbox[1] - 12.0
            && p.y < bbox[4] + 12.0
            && p.z > bbox[2] - 16.0
            && p.z < bbox[5] + 16.0
    };
    let mut points: Vec<Vec3> = Vec::new();
    let mut at = 119;
    while at + 12 <= b.len() {
        let p = Vec3::new(f32_at(b, at), f32_at(b, at + 4), f32_at(b, at + 8));
        if !ok(p) || points.first() == Some(&p) {
            break;
        }
        points.push(p);
        at += 12;
    }
    let kind = u32_at(b, 39);
    let material = u32_at(b, 51);
    let fog_density = f32_at(b, 55);
    let fog_color = [f32_at(b, 59), f32_at(b, 63), f32_at(b, 67)];
    (points.len() >= 3).then_some(Water { bbox, surface, depth, uv_scale, points, kind, material, fog_density, fog_color })
}

#[derive(Clone, Debug, Default)]
pub struct CellObjects {
    pub models: Vec<String>,
    pub materials: Vec<String>,
    pub brushes: Vec<Brush>,
    pub vegetation: Vec<Vegetation>,
    pub decals: Vec<Decal>,
    pub roads: Vec<Road>,
    pub water: Vec<Water>,
    /// The active `ParticleEffect` entities of `entities.xml`.
    pub effects: Vec<PlacedEffect>,
    /// The sound entities of `entities.xml` (ambient, random, spots, reverb) and their areas.
    pub sounds: crate::objects::sounds::SoundEntities,
}

/// A `ParticleEffect` entity of a cell's `entities.xml` (only the active ones, `bActive="1"`).
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedEffect {
    /// `library.group.name`, e.g. `X2_EFFECTS.fire_smoke.torchlight_b`.
    pub effect: String,
    /// Cell-local position, cry space.
    pub pos: Vec3,
    /// Rotation, cry space (`Rotate="w,x,y,z"` in the file).
    pub rot: Quat,
    /// The entity's `Scale` times the `Scale` property of the effect.
    pub scale: f32,
    pub speed_scale: f32,
    pub count_scale: f32,
    pub prime: bool,
}

/// The `ParticleEffect` entities of an `entities.xml` (positions are cell-local).
pub fn parse_effect_entities(xml: &str) -> Vec<PlacedEffect> {
    let Ok(doc) = roxmltree::Document::parse(xml.trim_start_matches('\u{feff}')) else { return Vec::new() };
    let floats = |s: Option<&str>| -> Vec<f32> { s.map(|s| s.split(',').filter_map(|v| v.trim().parse().ok()).collect()).unwrap_or_default() };
    let mut out = Vec::new();
    for e in doc.descendants().filter(|n| n.has_tag_name("Entity") && n.attribute("EntityClass") == Some("ParticleEffect")) {
        let Some(props) = e.children().find(|c| c.has_tag_name("Properties")) else { continue };
        let Some(effect) = props.attribute("ParticleEffect").map(str::trim).filter(|n| !n.is_empty()) else { continue };
        if props.attribute("bActive").map(str::trim) == Some("0") {
            continue;
        }
        let p = floats(e.attribute("Pos"));
        if p.len() < 3 {
            continue;
        }
        let rot = match floats(e.attribute("Rotate"))[..] {
            [w, x, y, z] => Quat::from_xyzw(x, y, z, w).normalize(),
            _ => Quat::IDENTITY,
        };
        let entity_scale = floats(e.attribute("Scale")).first().copied().unwrap_or(1.0);
        let prop = |name: &str| props.attribute(name).and_then(|v| v.trim().parse::<f32>().ok()).unwrap_or(1.0);
        out.push(PlacedEffect {
            effect: effect.to_string(),
            pos: Vec3::new(p[0], p[1], p[2]),
            rot,
            scale: entity_scale * prop("Scale"),
            speed_scale: prop("SpeedScale"),
            count_scale: prop("CountScale"),
            prime: props.attribute("bPrime").map(str::trim) != Some("0"),
        });
    }
    out
}

#[derive(Debug, thiserror::Error)]
pub enum PlacementError {
    #[error("truncated or malformed {0}")]
    Malformed(&'static str),
}

/// `statobjs.dat`: `u32 n` then `n` x 256 byte names.
pub fn parse_statobjs(data: &[u8]) -> Vec<String> {
    let n = u32_at(data, 0) as usize;
    (0..n)
        .filter_map(|i| data.get(4 + i * 256..4 + (i + 1) * 256))
        .map(|s| cstr(s).trim().to_string())
        .collect()
}

/// `material_list.dat` (and the tables inside `object.dat`): `u32 n`, then `n` x { u32, char[256] }.
pub fn parse_material_list(data: &[u8]) -> Vec<String> {
    let n = u32_at(data, 0) as usize;
    (0..n)
        .filter_map(|i| data.get(4 + i * 260 + 4..4 + (i + 1) * 260))
        .map(|s| cstr(s).trim().to_string())
        .collect()
}

fn read_brush(rec: &[u8], origin: Vec3) -> Brush {
    let mut matrix = [0.0f32; 12];
    for (i, m) in matrix.iter_mut().enumerate() {
        *m = f32_at(rec, 67 + i * 4);
    }
    matrix[3] += origin.x;
    matrix[7] += origin.y;
    matrix[11] += origin.z;
    let mut bbox = [0.0f32; 6];
    for (i, b) in bbox.iter_mut().enumerate() {
        *b = f32_at(rec, i * 4);
    }
    for k in 0..3 {
        bbox[k] += origin[k];
        bbox[k + 3] += origin[k];
    }
    Brush {
        matrix,
        model: u32_at(rec, 123),
        material: i32_at(rec, 115),
        flags: u32_at(rec, 32),
        max_view_dist: f32_at(rec, 28),
        bbox,
    }
}

fn read_vegetation(rec: &[u8], origin: Vec3) -> Vegetation {
    Vegetation {
        pos: Vec3::new(f32_at(rec, 39), f32_at(rec, 43), f32_at(rec, 47)) + origin,
        scale: f32_at(rec, 51),
        group: u16_at(rec, 55),
        angle_x: rec[57],
        angle_y: rec[58],
        bright: rec[59],
        angle_z: rec[60],
        flags: u32_at(rec, 32),
        max_view_dist: f32_at(rec, 28),
    }
}

/// `brush.dat`.
pub fn parse_brush_dat(data: &[u8]) -> Result<Vec<Brush>, PlacementError> {
    if data.len() < 2052 || u32_at(data, 0) != 1 {
        return Err(PlacementError::Malformed("brush.dat"));
    }
    let mut out = Vec::new();
    for i in 0..256 {
        let off = u32_at(data, 4 + i * 4) as usize;
        let size = u32_at(data, 4 + 1024 + i * 4) as usize;
        if size == 0 {
            continue;
        }
        let block = data.get(off..off + size).ok_or(PlacementError::Malformed("brush.dat"))?;
        let origin = Vec3::new((i / 16) as f32 * SECTOR_SIZE, (i % 16) as f32 * SECTOR_SIZE, 0.0);
        out.extend(block.chunks_exact(128).map(|rec| read_brush(rec, origin)));
    }
    Ok(out)
}

/// `vegetation.dat`.
pub fn parse_vegetation_dat(data: &[u8]) -> Result<Vec<Vegetation>, PlacementError> {
    if data.len() < 132 || u32_at(data, 0) != 1 {
        return Err(PlacementError::Malformed("vegetation.dat"));
    }
    let mut out = Vec::new();
    for i in 0..16 {
        let off = u32_at(data, 4 + i * 4) as usize;
        let size = u32_at(data, 4 + 64 + i * 4) as usize;
        if size == 0 {
            continue;
        }
        let block = data.get(off..off + size).ok_or(PlacementError::Malformed("vegetation.dat"))?;
        let origin = Vec3::new((i / 4) as f32 * 256.0, (i % 4) as f32 * 256.0, 0.0);
        out.extend(block.chunks_exact(64).map(|rec| read_vegetation(rec, origin)));
    }
    Ok(out)
}

/// `vegetation/SX_SY.dat` (cells with dense forests): `u32 version(1)` then 64 B records of the 256 m
/// sector `(sx, sy)` (file name = `{sx:02}_{sy:02}`), positions sector-local.
pub fn parse_vegetation_sector(data: &[u8], sx: usize, sy: usize) -> Vec<Vegetation> {
    if data.len() < 4 || u32_at(data, 0) != 1 {
        return Vec::new();
    }
    let origin = Vec3::new(sx as f32 * 256.0, sy as f32 * 256.0, 0.0);
    data[4..].chunks_exact(64).map(|rec| read_vegetation(rec, origin)).collect()
}

/// `object.dat`: models, materials and the octree objects (cell-local already).
pub fn parse_object_dat(data: &[u8]) -> Result<CellObjects, PlacementError> {
    let bad = PlacementError::Malformed("object.dat");
    let mut objects = CellObjects::default();
    let n = u32_at(data, 0) as usize;
    let mut pos = 4;
    for _ in 0..n {
        let rec = data.get(pos..pos + 260).ok_or(PlacementError::Malformed("object.dat"))?;
        objects.models.push(cstr(&rec[4..]).trim().to_string());
        pos += 260;
    }
    let m = u32_at(data, pos) as usize;
    pos += 4;
    for _ in 0..m {
        let rec = data.get(pos..pos + 260).ok_or(PlacementError::Malformed("object.dat"))?;
        objects.materials.push(cstr(&rec[4..]).trim().to_string());
        pos += 260;
    }
    let mut r = Reader::new(data);
    r.seek(pos).map_err(|_| PlacementError::Malformed("object.dat"))?;
    walk_node(&mut r, &mut objects).ok_or(bad)?;
    Ok(objects)
}

fn walk_node(r: &mut Reader, out: &mut CellObjects) -> Option<()> {
    let _version = r.u32().ok()?;
    r.skip(24).ok()?;
    let block = r.u32().ok()? as usize;
    let mask = r.u8().ok()?;
    let end = r.pos().checked_add(block)?;
    while r.pos() < end {
        let t = r.u32().ok()?;
        match t {
            1 => out.brushes.push(read_brush(r.bytes(128).ok()?, Vec3::ZERO)),
            2 => out.vegetation.push(read_vegetation(r.bytes(64).ok()?, Vec3::ZERO)),
            9 => out.decals.push(Decal { raw: r.bytes(111).ok()?.to_vec() }),
            13 => {
                let head = r.bytes(63).ok()?;
                let points = u32_at(head, 39) as usize;
                let bytes = points.checked_mul(12)?;
                if bytes > end.checked_sub(r.pos())? {
                    return None;
                }
                out.roads.push(read_road(head, r.bytes(bytes).ok()?));
            }
            11 => {
                let rest = end.checked_sub(r.pos())?;
                out.water.extend(parse_water(r.bytes(rest).ok()?));
            }
            // unknown payload size: the rest of this block is unreadable, the tree continues
            _ => r.seek(end).ok()?,
        }
    }
    r.seek(end).ok()?;
    for bit in 0..8 {
        if mask >> bit & 1 != 0 {
            walk_node(r, out)?;
        }
    }
    Some(())
}

/// Everything placed in a cell: brushes from `brush.dat` (else the `object.dat` octree), vegetation from
/// `vegetation.dat`, the per-sector `vegetation/` files and the octree.
pub fn load_cell_objects(world: &str, x: i32, y: i32) -> CellObjects {
    let dir = crate::world::cell_dir(world, x, y);
    let pak = aa_pak::Pak::global();
    let read = |f: &str| pak.read(&format!("{dir}/client/{f}")).ok();
    let mut objects = CellObjects::default();
    let from_octree = read("object.dat").and_then(|d| parse_object_dat(&d).ok());
    match (read("brush.dat").and_then(|d| parse_brush_dat(&d).ok()), read("statobjs.dat")) {
        (Some(brushes), Some(statobjs)) => {
            objects.brushes = brushes;
            objects.models = parse_statobjs(&statobjs);
            // `materialId` of `brush.dat` is an index into the material table of the cell's own `object.dat` (the same
            // index as the octree's copy of the brush: the model gw_memo_gate_watch01 gets gw_memo_gate there). The
            // separate `material_list.dat` (a longer list, in another order in 1808 of 2110 brushes of cell 10 / 14) is only
            // the fallback when the cell has no `object.dat`.
            objects.materials = match &from_octree {
                Some(o) => o.materials.clone(),
                None => read("material_list.dat").map(|d| parse_material_list(&d)).unwrap_or_default(),
            };
        }
        _ => {
            if let Some(o) = &from_octree {
                objects.brushes = o.brushes.clone();
                objects.models = o.models.clone();
                objects.materials = o.materials.clone();
            }
        }
    }
    // vegetation: vegetation.dat and the octree's own vegetation are different instances (no overlap)
    if let Some(v) = read("vegetation.dat").and_then(|d| parse_vegetation_dat(&d).ok()) {
        objects.vegetation = v;
    }
    if let Some(o) = &from_octree {
        objects.vegetation.extend(o.vegetation.iter().cloned());
    }
    // dense forests: one file per 256 m sector
    for sx in 0..4 {
        for sy in 0..4 {
            if let Some(d) = read(&format!("vegetation/{sx:02}_{sy:02}.dat")) {
                objects.vegetation.extend(parse_vegetation_sector(&d, sx, sy));
            }
        }
    }
    if let Some(o) = from_octree {
        objects.decals = o.decals;
        objects.roads = o.roads;
        objects.water = o.water;
    }
    if let Some(xml) = read("entities.xml") {
        let xml = String::from_utf8_lossy(&xml);
        objects.effects = parse_effect_entities(&xml);
        objects.sounds = crate::objects::sounds::parse(&xml);
    }
    objects
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn road_count_is_bounded_by_its_octree_block() {
        // Empty tables, one root node, one road claiming a vertex outside its object block.
        let mut bytes = vec![0; 8 + 33 + 4 + 63];
        bytes[8..12].copy_from_slice(&1u32.to_le_bytes());
        bytes[36..40].copy_from_slice(&67u32.to_le_bytes());
        bytes[41..45].copy_from_slice(&13u32.to_le_bytes());
        bytes[84..88].copy_from_slice(&1u32.to_le_bytes());
        assert!(parse_object_dat(&bytes).is_err());
    }

    #[test]
    fn effect_entities_are_parsed() {
        let xml = r#"<Mission><Objects>
          <Entity Name="a" Pos="1,2,3" Rotate="0.5,0.5,0.5,0.5" Scale="0.5,0.5,0.5" EntityClass="ParticleEffect"><Properties bActive="1" ParticleEffect="X2_EFFECTS.fire_smoke.torchlight_b" bPrime="1" Scale="2" SpeedScale="0.5"/></Entity>
          <Entity Name="b" Pos="4,5,6" EntityClass="ParticleEffect"><Properties bActive="0" ParticleEffect="x.y.z"/></Entity>
          <Entity Name="c" Pos="7,8,9" EntityClass="Light"><Properties/></Entity>
        </Objects></Mission>"#;
        let fx = parse_effect_entities(xml);
        assert_eq!(fx.len(), 1);
        assert_eq!(fx[0].effect, "X2_EFFECTS.fire_smoke.torchlight_b");
        assert_eq!(fx[0].pos, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(fx[0].scale, 1.0);
        assert_eq!(fx[0].speed_scale, 0.5);
        assert_eq!(fx[0].count_scale, 1.0);
        assert!(fx[0].prime);
    }

    #[test]
    fn spawn_cell_effects_are_read() {
        if !crate::pak_available() {
            return;
        }
        // the login stage's training field: ground mist and leaves
        let objects = load_cell_objects("login2", 0, 1);
        assert!(objects.effects.iter().any(|e| e.effect.ends_with("flow_fog_01")), "{:?}", objects.effects.len());
        assert!(objects.effects.iter().all(|e| e.pos.x > 0.0 && e.scale > 0.0));
        // a town cell of the main world
        let town = load_cell_objects("main_world", 10, 11);
        assert!(town.effects.len() > 100, "{}", town.effects.len());
    }

    #[test]
    fn login_cell_brushes_match_object_dat() {
        if !crate::pak_available() {
            return;
        }
        let pak = aa_pak::Pak::global();
        let dir = "worlds/login2/cells/000_001/client";
        let brush = parse_brush_dat(&pak.read(&format!("{dir}/brush.dat")).unwrap()).unwrap();
        let object = parse_object_dat(&pak.read(&format!("{dir}/object.dat")).unwrap()).unwrap();
        assert_eq!(brush.len(), 33);
        assert_eq!(object.brushes.len(), 33);
        let key = |b: &Brush| ((b.matrix[3] * 100.0) as i64, (b.matrix[7] * 100.0) as i64, b.model);
        let mut a: Vec<_> = brush.iter().map(key).collect();
        let mut b: Vec<_> = object.brushes.iter().map(key).collect();
        a.sort();
        b.sort();
        assert_eq!(a, b);
        let models = parse_statobjs(&pak.read(&format!("{dir}/statobjs.dat")).unwrap());
        assert_eq!(models.len(), 11);
        assert!(models.iter().all(|m| m.ends_with(".cgf")), "{models:?}");
        assert!(brush.iter().all(|b| (b.model as usize) < models.len()));
    }

    #[test]
    fn brush_materials_come_from_the_object_dat_table() {
        if !crate::pak_available() {
            return;
        }
        // Memoria (cell 10 / 14): `material_list.dat` is another table, with it the vines (gw_memo_etc_f02) got the road material whose
        // physics `proxy` sub-material is a pale green sheet, the canal tiles the stair material's white proxy slab, and gate
        // watch towers an elf tent
        let o = load_cell_objects("main_world", 10, 14);
        let material_of = |model: &str| {
            o.brushes
                .iter()
                .find(|b| o.models[b.model as usize].ends_with(model))
                .and_then(|b| usize::try_from(b.material).ok())
                .and_then(|i| o.materials.get(i))
                .cloned()
                .unwrap_or_default()
        };
        assert!(material_of("gw_memo_gate_watch01.cgf").ends_with("gw_memo_gate"), "{}", material_of("gw_memo_gate_watch01.cgf"));
        assert!(material_of("campfire_set_royster.cgf").ends_with("campfire_set_royster"));
        assert!(!material_of("gw_memo_etc_f02.cgf").contains("remain_divided_road"), "{}", material_of("gw_memo_etc_f02.cgf"));
        // every brush index is inside the table it is read from
        let table = o.materials.len() as i32;
        assert!(o.brushes.iter().all(|b| b.material < table), "table {table}");
    }

    #[test]
    fn main_world_cell() {
        if !crate::pak_available() {
            return;
        }
        let o = load_cell_objects("main_world", 9, 14);
        assert!(o.brushes.len() > 100 && o.vegetation.len() > 1000);
        assert!(o.brushes.iter().all(|b| (b.model as usize) < o.models.len()));
        assert!(o.vegetation.iter().all(|v| v.pos.x >= 0.0 && v.pos.x <= 1024.0 && v.pos.y >= 0.0 && v.pos.y <= 1024.0));
    }

    #[test]
    fn spawn_pond_water_volume() {
        if !crate::pak_available() {
            return;
        }
        let pak = aa_pak::Pak::global();
        let data = pak.read("worlds/main_world/cells/015_015/client/object.dat").unwrap();
        let o = parse_object_dat(&data).unwrap();
        assert_eq!(o.water.len(), 1);
        let w = &o.water[0];
        // the pond next to the Nuian spawn (218, 22): surface 127, ring of 8 points inside its box
        assert!((w.surface - 127.0).abs() < 0.01, "{}", w.surface);
        assert_eq!(w.points.len(), 8);
        assert!(w.points.iter().all(|p| p.x > 170.0 && p.x < 260.0));
        // its material and fog (header @39 volume type, @51 material index, @55 density, @59 colour)
        assert_eq!(w.kind & 3, 2);
        assert_eq!(o.materials[w.material as usize], "game/materials/ocean/lake_tank");
        assert!((w.fog_density - 0.6).abs() < 1e-4 && w.fog_color.iter().all(|c| *c > 0.0 && *c < 0.01), "{:?}", w);
    }
}
