//! Mesh particles: the `.cgf` an emitter with `Geometry` emits, and the `.mtl` it is drawn with.
//!
//! [`Geom::load`] reads the model's LOD 0 nodes into one vertex / index list in Bevy axes (the pivot is the model's origin; the
//! emitter centres it on the bounding box unless `NoOffset`), picks the sub-material most of the triangles use and turns the
//! `.mtl` into what the unlit particle shader needs: the diffuse and opacity textures, the blend mode, the colour factors and
//! the texture coordinate animation (`TexMod`: tiling, scrolling and flip book frames).

use std::collections::HashMap;
use std::sync::Arc;

use bevy::math::{Affine3A, Mat3, Quat, Vec2, Vec3};

use crate::params::Blend;
use crate::sim::c2b;

/// The triangles of a model, vertices in Bevy axes.
#[derive(Clone, Debug, Default)]
pub struct GeomTemplate {
    pub pos: Vec<Vec3>,
    pub nrm: Vec<Vec3>,
    pub uv: Vec<[f32; 2]>,
    pub idx: Vec<u32>,
    /// Centre of the bounding box.
    pub center: Vec3,
    /// Largest extent of the bounding box.
    pub extent: f32,
}

/// One texture coordinate axis of a `TexMod`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Osc {
    /// `TexMod_?OscillatorType`: 0 none, 1 fixed, 2 constant scroll, 3 jitter, 4 pan (sine), 5, 6 stretch.
    pub kind: u8,
    pub rate: f32,
    pub phase: f32,
    pub amp: f32,
}

impl Osc {
    fn at(&self, t: f32) -> f32 {
        match self.kind {
            1 => self.rate,
            2 => self.rate * t,
            3..=6 => self.amp * (std::f32::consts::TAU * (self.rate * t + self.phase)).sin(),
            _ => 0.0,
        }
    }
}

/// The animation of a material's diffuse texture coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UvMod {
    pub tile: Vec2,
    pub offset: Vec2,
    pub osc: [Osc; 2],
    /// Static rotation in degrees about the texture centre.
    pub rotate: f32,
    /// Flip book: tiles across / down, frames, frames per second, cycle, first tile.
    pub flip: Option<Flip>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flip {
    pub x: u32,
    pub y: u32,
    pub frames: u32,
    pub fps: f32,
    pub cycle: bool,
    pub first: u32,
}

impl Default for UvMod {
    fn default() -> Self {
        UvMod { tile: Vec2::ONE, offset: Vec2::ZERO, osc: [Osc::default(); 2], rotate: 0.0, flip: None }
    }
}

/// `uv * scale + shift`, after the rotation `rot` (sin, cos) about the centre: what one particle at one time needs.
#[derive(Clone, Copy, Debug)]
pub struct UvFrame {
    pub scale: Vec2,
    pub shift: Vec2,
    pub rot: Option<(f32, f32)>,
}

impl UvFrame {
    pub const IDENTITY: UvFrame = UvFrame { scale: Vec2::ONE, shift: Vec2::ZERO, rot: None };

    #[inline]
    pub fn apply(&self, uv: [f32; 2]) -> [f32; 2] {
        let (mut u, mut v) = (uv[0], uv[1]);
        if let Some((s, c)) = self.rot {
            let (x, y) = (u - 0.5, v - 0.5);
            u = x * c - y * s + 0.5;
            v = x * s + y * c + 0.5;
        }
        [u * self.scale.x + self.shift.x, v * self.scale.y + self.shift.y]
    }
}

impl UvMod {
    /// Whether the coordinates stay as the mesh has them.
    pub fn is_static_identity(&self) -> bool {
        self.tile == Vec2::ONE && self.offset == Vec2::ZERO && self.osc.iter().all(|o| o.kind == 0) && self.rotate == 0.0 && self.flip.is_none()
    }

    /// The transform at time `t` (seconds on the particle's or the effect's clock).
    pub fn frame(&self, t: f32) -> UvFrame {
        let mut scale = self.tile;
        let mut shift = self.offset + Vec2::new(self.osc[0].at(t), self.osc[1].at(t));
        if let Some(f) = self.flip {
            let n = f.frames.max(1);
            let k = (t * f.fps).floor() as i64;
            let k = if f.cycle { k.rem_euclid(n as i64) } else { k.clamp(0, n as i64 - 1) } as u32;
            let cell = (f.first + k) % (f.x * f.y).max(1);
            scale /= Vec2::new(f.x as f32, f.y as f32);
            shift = shift / Vec2::new(f.x as f32, f.y as f32) + Vec2::new((cell % f.x) as f32, (cell / f.x) as f32) / Vec2::new(f.x as f32, f.y as f32);
        }
        let rot = (self.rotate != 0.0).then(|| self.rotate.to_radians().sin_cos());
        UvFrame { scale, shift, rot }
    }
}

/// What the material of a mesh particle asks of the shader.
#[derive(Clone, Debug)]
pub struct GeomMtl {
    /// Pak path of the diffuse texture ("" = none, white).
    pub texture: String,
    /// Pak path of the separate opacity map ("" = none).
    pub opacity_map: String,
    /// Channels of the opacity map that make the opacity.
    pub opacity_mask: [f32; 4],
    pub blend: Blend,
    /// Alpha test threshold, 0 = none.
    pub cutoff: f32,
    pub tint: Vec3,
    pub emissive: f32,
    pub opacity: f32,
    pub uv: UvMod,
    /// A lit surface (`Illum` shader): its diffuse colour is a reflectance, lit by the sun and sky, not an emitted colour.
    pub lit: bool,
}

impl Default for GeomMtl {
    fn default() -> Self {
        GeomMtl {
            texture: String::new(),
            opacity_map: String::new(),
            opacity_mask: [0.0; 4],
            blend: Blend::Opaque,
            cutoff: 0.0,
            tint: Vec3::ONE,
            emissive: 0.0,
            opacity: 1.0,
            uv: UvMod::default(),
            lit: false,
        }
    }
}

/// A model with its material, ready to be instanced.
#[derive(Clone, Debug, Default)]
pub struct Geom {
    pub tmpl: GeomTemplate,
    pub mtl: GeomMtl,
}

/// Native mesh subobjects retain local vertices and their own source frame.
/// Kept outside the public `Geom`/`GeomKey` data contract.
#[derive(Clone, Debug)]
pub(crate) struct Piece {
    pub tmpl: GeomTemplate,
    pub translation: Vec3,
    pub rotation: Mat3,
    pub scale: f32,
}

pub(crate) struct LoadedGeom {
    pub geom: Arc<Geom>,
    pub pieces: Arc<[Piece]>,
}

/// What an emitter asks for.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct GeomKey {
    /// Pak path of the `.cgf`.
    pub path: String,
    /// The emitter's `Material` (empty = the model's own).
    pub material: String,
    /// The emitter's `Texture` (empty = the material's).
    pub texture: String,
    /// A `BlendType` the emitter names (otherwise the material decides).
    pub blend: Option<Blend>,
}

fn attr_f(el: roxmltree::Node, name: &str) -> Option<f32> {
    el.attribute(name).and_then(|v| v.trim().parse().ok())
}

/// The `TexMod` of the diffuse texture of sub-material `sub` (of the `.mtl` text).
fn read_uv_mod(text: &str, sub: usize) -> UvMod {
    let mut out = UvMod::default();
    let Ok(doc) = roxmltree::Document::parse(text.trim_start_matches('\u{feff}').trim_start()) else { return out };
    let root = doc.root_element();
    let material = root
        .children()
        .find(|c| c.has_tag_name("SubMaterials"))
        .and_then(|s| s.children().filter(|c| c.has_tag_name("Material")).nth(sub))
        .unwrap_or(root);
    let Some(tex) = material.children().find(|c| c.has_tag_name("Textures")).and_then(|t| {
        t.children().find(|c| c.has_tag_name("Texture") && c.attribute("Map").is_some_and(|m| m.eq_ignore_ascii_case("Diffuse")))
    }) else {
        return out;
    };
    let Some(m) = tex.children().find(|c| c.has_tag_name("TexMod")) else { return out };
    out.tile = Vec2::new(attr_f(m, "TileU").unwrap_or(1.0), attr_f(m, "TileV").unwrap_or(1.0));
    out.offset = Vec2::new(attr_f(m, "OffsetU").unwrap_or(0.0), attr_f(m, "OffsetV").unwrap_or(0.0));
    out.rotate = attr_f(m, "RotateW").unwrap_or(0.0);
    if attr_f(m, "TexMod_RotateType").unwrap_or(0.0) != 0.0 {
        // a rotating texture: the angle is its rate; not animated here, it would need a rotation per frame
        out.rotate = 0.0;
    }
    for (i, axis) in ["U", "V"].iter().enumerate() {
        let f = |what: &str| attr_f(m, &format!("TexMod_{axis}Oscillator{what}"));
        out.osc[i] = Osc { kind: f("Type").unwrap_or(0.0) as u8, rate: f("Rate").unwrap_or(0.0), phase: f("Phase").unwrap_or(0.0), amp: f("Amplitude").unwrap_or(0.0) };
    }
    let (ax, ay) = (attr_f(m, "TexMod_AnimTileX").unwrap_or(0.0) as u32, attr_f(m, "TexMod_AnimTileY").unwrap_or(0.0) as u32);
    if ax > 0 || ay > 0 {
        let (x, y) = (ax.max(1), ay.max(1));
        let frames = attr_f(m, "TexMod_AnimFrameCount").map_or(x * y, |f| (f as u32).max(1));
        out.flip = Some(Flip {
            x,
            y,
            frames,
            fps: attr_f(m, "TexMod_AnimFramerate").filter(|f| *f > 0.0).unwrap_or(10.0),
            cycle: attr_f(m, "TexMod_AnimCycle").unwrap_or(1.0) != 0.0,
            first: attr_f(m, "TexMod_AnimFirstTile").unwrap_or(0.0) as u32,
        });
    }
    out
}

/// The vertices of the LOD 0 nodes with the material id of every triangle run.
struct Raw {
    pos: Vec<Vec3>,
    nrm: Vec<Vec3>,
    uv: Vec<[f32; 2]>,
    /// (sub-material id, indices)
    runs: Vec<(u32, Vec<u32>)>,
}

fn raw_model(model: &aa_cry::mesh::Model) -> Raw {
    raw_nodes(model, &model.lod_nodes(0), true)
}

fn raw_nodes(model: &aa_cry::mesh::Model, nodes: &[usize], transform: bool) -> Raw {
    let mut raw = Raw { pos: Vec::new(), nrm: Vec::new(), uv: Vec::new(), runs: Vec::new() };
    for &ni in nodes {
        let Some(mi) = model.nodes[ni].mesh else { continue };
        let m = &model.meshes[mi];
        if m.skin.is_some() || m.positions.is_empty() {
            continue;
        }
        let tm = if transform { aa_cry::scene::render_transform(model, ni) } else { Affine3A::IDENTITY };
        let base = raw.pos.len() as u32;
        raw.pos.extend(m.positions.iter().map(|p| c2b(tm.transform_point3(*p))));
        if m.normals.len() == m.positions.len() {
            raw.nrm.extend(m.normals.iter().map(|n| c2b(tm.transform_vector3(*n)).normalize_or_zero()));
        } else {
            raw.nrm.extend(std::iter::repeat_n(Vec3::Y, m.positions.len()));
        }
        if m.uvs.len() == m.positions.len() {
            raw.uv.extend(m.uvs.iter().map(|u| u.to_array()));
        } else {
            raw.uv.extend(std::iter::repeat_n([0.0, 0.0], m.positions.len()));
        }
        let mut run = |id: u32, idx: &[u32]| {
            if !idx.is_empty() {
                raw.runs.push((id, idx.iter().map(|i| i + base).collect()));
            }
        };
        if m.subsets.is_empty() {
            run(0, &m.indices);
        } else {
            for s in &m.subsets {
                let (a, b) = (s.first_index as usize, (s.first_index + s.index_count) as usize);
                if let Some(idx) = m.indices.get(a..b) {
                    run(s.material_id, idx);
                }
            }
        }
    }
    raw
}

fn template(raw: &Raw, skip: impl Fn(u32) -> bool) -> Option<GeomTemplate> {
    let mut remap = vec![u32::MAX; raw.pos.len()];
    let mut tmpl = GeomTemplate::default();
    for (id, idx) in &raw.runs {
        if skip(*id) {
            continue;
        }
        for &i in idx {
            let r = &mut remap[i as usize];
            if *r == u32::MAX {
                *r = tmpl.pos.len() as u32;
                tmpl.pos.push(raw.pos[i as usize]);
                tmpl.nrm.push(raw.nrm[i as usize]);
                tmpl.uv.push(raw.uv[i as usize]);
            }
            tmpl.idx.push(*r);
        }
    }
    if tmpl.idx.len() < 3 {
        return None;
    }
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for p in &tmpl.pos {
        lo = lo.min(*p);
        hi = hi.max(*p);
    }
    tmpl.center = (lo + hi) * 0.5;
    tmpl.extent = (hi - lo).max_element();
    Some(tmpl)
}

/// sub_3008AB30: cube-root determinant scale, orthonormalized matrix rows.
fn piece_frame(tm: Affine3A) -> Option<(Vec3, Mat3, f32)> {
    let m = Mat3::from(tm.matrix3);
    let scale = m.determinant().powf(1.0 / 3.0);
    let rows = m.transpose();
    let x = rows.x_axis.normalize_or_zero();
    let y = rows.z_axis.cross(x).normalize_or_zero();
    let z = x.cross(y);
    if !scale.is_finite() || scale <= 0.0 || x.length_squared() < 0.5 || y.length_squared() < 0.5 {
        return None;
    }
    let q = Quat::from_mat3(&Mat3::from_cols(x, y, z).transpose());
    let axes = aa_cry::CRY_TO_BEVY;
    Some((c2b(tm.translation.into()), Mat3::from_quat(axes * q * axes.inverse()), scale))
}

/// The material a `Liquid` material names as its `DualMaterial` (the one of sub-material `sub` when the file has sub-materials), loaded.
fn dual_material(mtl_path: &str, sub: Option<usize>) -> Option<(aa_cry::material::Material, String)> {
    let text = String::from_utf8_lossy(&aa_cry::read(mtl_path).ok()?).into_owned();
    let doc = roxmltree::Document::parse(&text).ok()?;
    let mut node = doc.root_element();
    if let Some(i) = sub {
        node = node.children().find(|c| c.has_tag_name("SubMaterials"))?.children().filter(|c| c.is_element()).nth(i)?;
    }
    let name = node.children().find(|c| c.has_tag_name("DualMaterial"))?.attribute("Name")?;
    let path = format!("{}.mtl", name.to_ascii_lowercase().replace(char::from(92), "/"));
    Some((aa_cry::material::Material::load(&path).ok()?, path))
}

impl Geom {
    /// Loads the model and its material; `None` when the model has no triangles to draw.
    pub fn load(key: &GeomKey) -> Option<Geom> {
        Self::load_with_pieces(key).and_then(|g| Arc::into_inner(g.geom))
    }

    pub(crate) fn load_with_pieces(key: &GeomKey) -> Option<LoadedGeom> {
        let model = aa_cry::mesh::Model::load(&key.path).ok()?;
        let raw = raw_model(&model);
        if raw.runs.is_empty() {
            return None;
        }
        let mtl_path = if key.material.is_empty() { aa_cry::scene::find_mtl(&key.path, &model).unwrap_or_default() } else { key.material.clone() };
        let mtl = (!mtl_path.is_empty()).then(|| aa_cry::material::Material::load(&mtl_path).ok()).flatten();

        // the sub-material most triangles use draws; the ones that draw nothing are left out
        let mut tris: HashMap<u32, usize> = HashMap::new();
        for (id, idx) in &raw.runs {
            *tris.entry(*id).or_default() += idx.len();
        }
        let main = tris.iter().max_by_key(|(id, n)| (**n, std::cmp::Reverse(**id))).map(|(id, _)| *id).unwrap_or(0);
        let sub = mtl.as_ref().and_then(|m| m.sub(main as usize).or(Some(m)));
        // a `Liquid` surface (water bubbles, the shells of buff auras) is clear water on its own: the engine's refraction shows the scene
        // through it, and what is seen of it is the dual material's foam layer. Without refraction here that layer stands for it.
        let dual = sub
            .filter(|m| m.shader.eq_ignore_ascii_case("Liquid"))
            .and_then(|_| dual_material(&mtl_path, mtl.as_ref().is_some_and(|m| !m.sub_materials.is_empty()).then_some(main as usize)));
        let (sub, mtl_path) = match &dual {
            Some((m, path)) => (Some(m), path.clone()),
            None => (sub, mtl_path),
        };
        let has_subs = dual.is_none() && mtl.as_ref().is_some_and(|m| !m.sub_materials.is_empty());
        let mut out = GeomMtl::default();
        if let Some(m) = sub {
            use aa_cry::material::{TextureMap, flags};
            out.texture = m.texture(&TextureMap::Diffuse).map(|t| t.pak_path()).unwrap_or_default();
            out.opacity_map = m.texture(&TextureMap::Opacity).map(|t| t.pak_path()).unwrap_or_default();
            out.tint = m.diffuse;
            out.emissive = (m.emissive.x + m.emissive.y + m.emissive.z) / 3.0;
            out.opacity = m.opacity.clamp(0.0, 1.0);
            out.cutoff = m.alpha_test.clamp(0.0, 1.0);
            let additive = m.flags & flags::ADDITIVE != 0;
            out.lit = !additive && m.shader.eq_ignore_ascii_case("Illum");
            out.blend = if additive {
                Blend::Additive
            } else if out.opacity < 1.0 || !out.opacity_map.is_empty() || m.shader.eq_ignore_ascii_case("Glass") {
                Blend::Alpha
            } else {
                Blend::Opaque
            };
            if dual.is_none() && out.texture.is_empty() && m.shader.eq_ignore_ascii_case("Liquid") {
                // a Liquid surface with no dual material and no texture (the centre of `world_crack`, the portal gates): clear water, which the engine draws as
                // the scene seen through it, tinted by `LiquidColor`. Opaque white was wrong; the real gate shows the scene darker and green
                let c: Vec<f32> = m.public_param("LiquidColor").map(|v| v.split(',').filter_map(|x| x.trim().parse().ok()).collect()).unwrap_or_default();
                out.tint = if c.len() == 3 { Vec3::new(c[0], c[1], c[2]) } else { Vec3::new(0.5, 0.6, 0.6) };
                out.blend = Blend::Multiplicative;
                out.lit = false;
                out.opacity = 1.0;
                out.emissive = 0.0;
            }
            if !out.opacity_map.is_empty() {
                // `opacityChannelMask="0,0,0,1"` names the channel; a grey map has the same value in all of them
                out.opacity_mask = match m.public_param("opacityChannelMask") {
                    Some(v) => {
                        let p: Vec<f32> = v.split(',').filter_map(|s| s.trim().parse().ok()).collect();
                        if p.len() == 4 && p.iter().any(|x| *x > 0.0) { [p[0], p[1], p[2], p[3]] } else { [1.0, 0.0, 0.0, 0.0] }
                    }
                    None => [1.0, 0.0, 0.0, 0.0],
                };
            }
            if !mtl_path.is_empty()
                && let Ok(bytes) = aa_cry::read(&mtl_path)
            {
                out.uv = read_uv_mod(&String::from_utf8_lossy(&bytes), if has_subs { main as usize } else { 0 });
            }
        }
        if !key.texture.is_empty() {
            out.texture = key.texture.clone();
        }
        if let Some(b) = key.blend {
            out.blend = b;
        }
        // an emitter that names a texture but whose material says nothing: an alpha blended sprite-like mesh
        if out.texture.is_empty() && out.opacity_map.is_empty() && out.blend == Blend::Opaque && mtl.is_none() {
            out.blend = Blend::Alpha;
        }

        let skip = |id: u32| {
            mtl.as_ref().and_then(|m| m.sub(id as usize)).is_some_and(|m| m.no_draw())
        };
        let tmpl = template(&raw, skip)?;
        let nodes = model.lod_nodes(0);
        let mut pieces = Vec::new();
        // A single mesh (including a compiled Merged mesh) is never fabricated
        // into pieces. Childless geometry follows the existing parent path.
        // All four authored compound assets in the inventory use root mesh
        // nodes. Nested subobject ownership is outside this bounded path.
        if nodes.len() > 1 && nodes.iter().all(|&i| model.nodes[i].parent.is_none()) {
            for ni in nodes {
                // Native subobject +64 is the node's local matrix; +16 holds
                // its world matrix and is not the emission source matrix.
                let tm = model.nodes[ni].local;
                let local = raw_nodes(&model, &[ni], false);
                let (Some(mut tmpl), Some((translation, rotation, scale))) = (template(&local, skip), piece_frame(tm)) else {
                    // Unsupported or undrawable subobjects keep the existing
                    // whole-mesh path; never emit a substitute mesh.
                    pieces.clear();
                    break;
                };
                let mesh = &model.meshes[model.nodes[ni].mesh?];
                // Native GetAABB supplies the child's authored bounds, not the
                // compacted subset's bounds (which can exclude no-draw faces).
                tmpl.center = c2b((mesh.bbox_min + mesh.bbox_max) * 0.5);
                tmpl.extent = (mesh.bbox_max - mesh.bbox_min).max_element();
                pieces.push(Piece { tmpl, translation, rotation, scale });
            }
        }
        Some(LoadedGeom { geom: Arc::new(Geom { tmpl, mtl: out }), pieces: pieces.into() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn piece_frames_match_executed_original_12_transforms() {
        let axes = aa_cry::CRY_TO_BEVY;
        for line in include_str!("../evidence/geometry_pieces_transforms.tsv").lines().filter(|l| !l.starts_with('#')) {
            let (name, numbers) = line.split_once('\t').unwrap();
            let v: Vec<f32> = numbers.split('\t').map(|n| n.parse().unwrap()).collect();
            let vector = |a, b, c| Vec3::new(v[a], v[b], v[c]);
            let tm = Affine3A::from_cols(vector(0, 4, 8).into(), vector(1, 5, 9).into(), vector(2, 6, 10).into(), vector(3, 7, 11).into());
            let (translation, rotation, scale) = piece_frame(tm).unwrap();
            let parent = Mat3::from_quat(axes * Quat::from_xyzw(v[12], v[13], v[14], v[15]) * axes.inverse());
            let position = c2b(vector(16, 17, 18)) + parent * translation * v[19];
            let expected = Mat3::from_quat(axes * Quat::from_xyzw(v[20], v[21], v[22], v[23]).normalize() * axes.inverse());
            assert!(position.distance(c2b(vector(24, 25, 26))) < 1e-5, "{name}: {position}");
            assert!((scale * v[19] - v[27]).abs() < 1e-5, "{name}: {scale}");
            for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
                assert!((parent * rotation * axis).distance(expected * axis) < 1e-5, "{name}: {rotation}");
            }
        }
        assert!(piece_frame(Affine3A::from_scale(Vec3::new(-1.0, 1.0, 1.0))).is_none());
        assert!(piece_frame(Affine3A::from_scale(Vec3::ZERO)).is_none());
    }

    #[test]
    fn original_compound_models_retain_nodes_while_single_meshes_stay_whole() {
        if !aa_cry::pak_available() {
            return;
        }
        for (path, count) in [
            ("objects/effects/oeffect/candy_b.cgf", 3),
            ("objects/effects/oeffect/hexagon_sphere_c.cgf", 49),
            ("objects/env/05_nature/03_temp/temp_tree_anbig03.cgf", 2),
            ("objects/effects/particle_effects/broken_wheel.cgf", 4),
            ("objects/effects/chain03.cgf", 0),
            ("objects/env/01_nuia/01_sol/stone_solzreed_a_fl.cgf", 0),
        ] {
            let key = GeomKey { path: path.into(), material: String::new(), texture: String::new(), blend: None };
            let loaded = Geom::load_with_pieces(&key).unwrap();
            assert_eq!(loaded.pieces.len(), count, "{path}");
            if count > 0 {
                assert_eq!(loaded.pieces.iter().map(|p| p.tmpl.idx.len()).sum::<usize>(), loaded.geom.tmpl.idx.len(), "{path}: every original triangle once");
                for p in loaded.pieces.iter() {
                    assert!(!p.tmpl.idx.is_empty());
                    assert!(p.tmpl.idx.iter().all(|&i| i < p.tmpl.pos.len() as u32));
                    assert_eq!(p.tmpl.pos.len(), p.tmpl.nrm.len());
                    assert_eq!(p.tmpl.pos.len(), p.tmpl.uv.len());
                }
            }
        }
    }

    #[test]
    fn scroll_and_flip_book() {
        let m = UvMod { osc: [Osc { kind: 2, rate: 0.5, ..Default::default() }, Osc::default()], ..Default::default() };
        let f = m.frame(2.0);
        assert!((f.apply([0.0, 0.25])[0] - 1.0).abs() < 1e-6);
        assert!(!m.is_static_identity());
        let m = UvMod { flip: Some(Flip { x: 4, y: 1, frames: 4, fps: 10.0, cycle: true, first: 0 }), ..Default::default() };
        let f = m.frame(0.25);
        assert!((f.apply([0.0, 0.0])[0] - 0.5).abs() < 1e-6, "{:?}", f);
        assert!((f.apply([1.0, 0.0])[0] - 0.75).abs() < 1e-6);
    }

    #[test]
    fn reads_a_texmod() {
        let text = r#"<Material><Textures><Texture Map="Diffuse" File="a.dds"><TexMod TileU="2" TexMod_UOscillatorType="2" TexMod_UOscillatorRate="0.5" TexMod_VOscillatorType="2" TexMod_VOscillatorRate="-0.5" TexMod_AnimTileX="4" TexMod_AnimTileY="2" TexMod_AnimFramerate="8"/></Texture></Textures></Material>"#;
        let m = read_uv_mod(text, 0);
        assert_eq!(m.tile, Vec2::new(2.0, 1.0));
        assert_eq!(m.osc[0].kind, 2);
        assert_eq!(m.osc[1].rate, -0.5);
        let f = m.flip.unwrap();
        assert_eq!((f.x, f.y, f.frames), (4, 2, 8));
    }

    #[test]
    fn a_liquid_bubble_draws_its_dual_materials_translucent_foam() {
        if !aa_cry::pak_available() {
            return;
        }
        // Bubble Trap's aura: a Liquid sphere (opacity 1, no textures) whose dual material is the foam layer at opacity 0.3
        let key = GeomKey { path: "game/objects/effects/high_sphere2.cgf".into(), material: "objects/effects/water_buble_dot.mtl".into(), texture: String::new(), blend: None };
        let g = Geom::load(&key).expect("bubble");
        assert_eq!(g.mtl.blend, Blend::Alpha);
        assert!(g.mtl.opacity < 0.5, "{:?}", g.mtl);
        assert!(g.mtl.texture.contains("water_fall_foamy"), "{:?}", g.mtl);
    }

    #[test]
    fn the_liquid_centre_of_the_portal_gate_is_tinted_water_not_a_white_slab() {
        if !aa_cry::pak_available() {
            return;
        }
        // `world_crack_center.mtl`: a Liquid with no textures and no dual material, LiquidColor 0.45 0.62 0.54 (it was drawn opaque white)
        let key = GeomKey { path: "game/objects/effects/world_crack_center.cgf".into(), material: "game/objects/effects/world_crack_center.mtl".into(), texture: String::new(), blend: None };
        let g = Geom::load(&key).expect("crack centre");
        assert_eq!(g.mtl.blend, Blend::Multiplicative, "{:?}", g.mtl);
        assert!((g.mtl.tint.y - 0.62).abs() < 0.01 && g.mtl.tint.x < 0.5, "{:?}", g.mtl);
    }

    #[test]
    fn loads_effect_meshes() {
        if !aa_cry::pak_available() {
            return;
        }
        let key = GeomKey { path: "objects/effects/lightning.cgf".into(), material: "objects/effects/lightning_magic1b.mtl".into(), texture: String::new(), blend: None };
        let g = Geom::load(&key).expect("lightning");
        assert!(g.tmpl.idx.len() >= 3 && g.tmpl.idx.len().is_multiple_of(3));
        assert_eq!(g.tmpl.pos.len(), g.tmpl.nrm.len());
        assert!(g.mtl.texture.contains("lightnig_flash"), "{:?}", g.mtl);
        assert!(!g.mtl.opacity_map.is_empty());
        println!("lightning: {} verts {} tris extent {} center {:?} mtl {:?}", g.tmpl.pos.len(), g.tmpl.idx.len() / 3, g.tmpl.extent, g.tmpl.center, g.mtl);
    }
}
