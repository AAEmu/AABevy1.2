//! Face morph targets: the shape sliders of the character-create screen.
//!
//! The customisable head mesh (`nuian/*/face/nu_*_face01.chr`, shared by every race) carries a
//! **CompiledMorphTargets** chunk (`0xACDC0002`, v0x801): `u32 count`, then per target
//!
//! ```text
//! u32 mesh_chunk_id, u32 name_len, u32 num_int, u32 num_ext
//! char name[name_len]                      NUL terminated, e.g. "#eye_Size_max"
//! { u32 vertex, f32 dx, dy, dz } x num_int     deltas on the engine's internal vertices (unused here)
//! { u32 vertex, f32 dx, dy, dz } x num_ext     deltas on the render mesh vertices (metres)
//! ```
//!
//! The 91 targets are the sliders (`eye_*`, `FS_*` face shape, `mouth_*`, `nose_*`), the race
//! shapes (`elf_Morph`, `hariharan_Morph`, `elf_EarRotate`, `elf_ElfEar`, `elf_ElfEarSize`) and
//! nothing else (the lip-sync visemes `#m_a`, `#lip_*` live in `*_face00.chr`).
//!
//! `<race>/<gender>/face/*_targets.xml` ("FaceMaker") maps the slider index to targets:
//!
//! ```xml
//! <Target Idx="4" Name="eye_Size" MinMorph="eye_Size_min" MaxMorph="eye_Size_max" Category="eyes"
//!         Apply="minmax" Scale="1" Min="-30" Max="30"/>
//! <Target Idx="2" Name="eye_Yposition" Morph="eye_Yposition" Apply="normal" Scale="1" .../>
//! ```
//!
//! and the 128-byte `modifier` blob of a character (`total_character_customs.modifier`,
//! `FaceModel.modifier`) holds one **signed byte per `Idx`** (`modifier[Idx]`, in
//! `Min..=Max`, 100 = full effect). `normal` targets blend with weight `value/100 * Scale`
//! (negative values push the other way); `minmax` targets use `MinMorph` for negative and
//! `MaxMorph` for positive values.

use aa_cry::chunk::{ChunkFile, ty};
use glam::Vec3;

use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq)]
pub struct MorphTarget {
    /// Name without the leading `#`.
    pub name: String,
    pub mesh_chunk: u32,
    /// Deltas on the render mesh's vertices.
    pub deltas: Vec<(u32, Vec3)>,
}

/// All morph targets of a model file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MorphSet {
    pub targets: Vec<MorphTarget>,
}

impl MorphSet {
    /// Reads the `CompiledMorphTargets` chunk; `None` if the file has none.
    pub fn from_chunks(file: &ChunkFile<'_>) -> Result<Option<MorphSet>> {
        let Some(c) = file.first(ty::COMPILED_MORPH_TARGETS) else { return Ok(None) };
        let mut r = c.reader();
        let count = r.u32()? as usize;
        if count > 4096 {
            return Err(Error::Format(format!("{count} morph targets")));
        }
        let mut targets = Vec::with_capacity(count);
        for _ in 0..count {
            let mesh_chunk = r.u32()?;
            let name_len = r.u32()? as usize;
            let num_int = r.u32()? as usize;
            let num_ext = r.u32()? as usize;
            let name = aa_cry::chunk::cstr(r.bytes(name_len)?);
            r.skip(num_int * 16)?;
            let raw = r.bytes(num_ext * 16)?;
            let deltas = raw
                .chunks_exact(16)
                .map(|k| {
                    let f = |i: usize| f32::from_le_bytes(k[i..i + 4].try_into().unwrap());
                    (u32::from_le_bytes(k[0..4].try_into().unwrap()), Vec3::new(f(4), f(8), f(12)))
                })
                .collect();
            targets.push(MorphTarget { name: name.trim_start_matches('#').to_string(), mesh_chunk, deltas });
        }
        Ok(Some(MorphSet { targets }))
    }

    pub fn load(path: &str) -> Result<Option<MorphSet>> {
        let bytes = aa_cry::read(path)?;
        MorphSet::from_chunks(&ChunkFile::parse(&bytes)?)
    }

    pub fn find(&self, name: &str) -> Option<&MorphTarget> {
        self.targets.iter().find(|t| t.name.eq_ignore_ascii_case(name))
    }

    /// `positions += sum(weight * delta)` for `(target name, weight)` pairs.
    pub fn apply(&self, positions: &mut [Vec3], weights: &[(String, f32)]) {
        for (name, w) in weights {
            if w.abs() < 1e-4 {
                continue;
            }
            let Some(t) = self.find(name) else { continue };
            for &(v, d) in &t.deltas {
                if let Some(p) = positions.get_mut(v as usize) {
                    *p += d * *w;
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Apply {
    Normal,
    MinMax,
}

/// One slider of `*_targets.xml`.
#[derive(Clone, Debug, PartialEq)]
pub struct TargetDef {
    pub idx: usize,
    pub name: String,
    pub category: String,
    pub apply: Apply,
    pub morph: String,
    pub min_morph: String,
    pub max_morph: String,
    pub scale: f32,
    pub min: i32,
    pub max: i32,
}

/// Parsed `*_targets.xml`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FaceMaker {
    /// `Asset0` / `Asset1` / `Asset2`: full, mid and low detail head `.chr` (normalised paths).
    pub assets: Vec<String>,
    pub targets: Vec<TargetDef>,
}

impl FaceMaker {
    pub fn parse(xml: &str) -> Result<FaceMaker> {
        let xml = xml.trim_start_matches('\u{feff}');
        let doc = roxmltree::Document::parse(xml).map_err(|e| Error::Xml(e.to_string()))?;
        let mut fm = FaceMaker::default();
        for n in doc.root_element().descendants().filter(|n| n.is_element()) {
            let name = n.tag_name().name();
            if name.starts_with("Asset") {
                if let Some(p) = n.attribute("Path") {
                    fm.assets.push(crate::cdf::norm(p));
                }
            } else if name == "Target" {
                let s = |a: &str| n.attribute(a).unwrap_or("").to_string();
                let i = |a: &str, d: i32| n.attribute(a).and_then(|v| v.trim().parse().ok()).unwrap_or(d);
                fm.targets.push(TargetDef {
                    idx: i("Idx", -1).max(0) as usize,
                    name: s("Name"),
                    category: s("Category"),
                    apply: if n.attribute("Apply") == Some("minmax") { Apply::MinMax } else { Apply::Normal },
                    morph: s("Morph"),
                    min_morph: s("MinMorph"),
                    max_morph: s("MaxMorph"),
                    scale: n.attribute("Scale").and_then(|v| v.trim().parse().ok()).unwrap_or(1.0),
                    min: i("Min", -100),
                    max: i("Max", 100),
                });
            }
        }
        Ok(fm)
    }

    pub fn load(path: &str) -> Result<FaceMaker> {
        FaceMaker::parse(&String::from_utf8_lossy(&aa_cry::read(path)?))
    }

    /// Morph weights for a 128-byte modifier: `(target name, weight)`.
    pub fn weights(&self, modifier: &[u8]) -> Vec<(String, f32)> {
        let mut out = Vec::new();
        for t in &self.targets {
            let v = modifier.get(t.idx).map(|&b| b as i8 as f32).unwrap_or(0.0);
            if v == 0.0 {
                continue;
            }
            let w = v / 100.0 * t.scale;
            match t.apply {
                Apply::Normal => {
                    if !t.morph.is_empty() {
                        out.push((t.morph.clone(), w));
                    }
                }
                Apply::MinMax => {
                    if w < 0.0 && !t.min_morph.is_empty() {
                        out.push((t.min_morph.clone(), -w));
                    } else if w > 0.0 && !t.max_morph.is_empty() {
                        out.push((t.max_morph.clone(), w));
                    }
                }
            }
        }
        out
    }

    /// Race-shape targets that are always on for a race (they are not sliders).
    pub fn race_weights(race: crate::db::Race) -> Vec<(String, f32)> {
        match race {
            crate::db::Race::Elf => vec![("elf_Morph".to_string(), 1.0)],
            crate::db::Race::Hariharan => vec![("hariharan_Morph".to_string(), 1.0)],
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &str = r#"<FaceMaker>
 <Asset0 Path="objects\characters\nuian\male\face\nu_m_face01.chr"/>
 <Targets>
  <Target Idx="2" Name="eye_Yposition" Morph="eye_Yposition" Category="eyes" Apply="normal" Scale="2" Min="-100" Max="100"/>
  <Target Idx="4" Name="eye_Size" MinMorph="eye_Size_min" MaxMorph="eye_Size_max" Category="eyes" Apply="minmax" Scale="1" Min="-30" Max="30"/>
 </Targets>
</FaceMaker>"#;

    #[test]
    fn weights_from_modifier() {
        let fm = FaceMaker::parse(XML).unwrap();
        assert_eq!(fm.assets, vec!["objects/characters/nuian/male/face/nu_m_face01.chr"]);
        let mut m = vec![0u8; 128];
        m[2] = (-50i8) as u8;
        m[4] = 30;
        let w = fm.weights(&m);
        assert_eq!(w, vec![("eye_Yposition".to_string(), -1.0), ("eye_Size_max".to_string(), 0.3)]);
        m[4] = (-30i8) as u8;
        assert_eq!(fm.weights(&m)[1], ("eye_Size_min".to_string(), 0.3));
        assert!(fm.weights(&[]).is_empty());
    }

    #[test]
    fn apply_moves_vertices() {
        let set = MorphSet {
            targets: vec![MorphTarget { name: "a".into(), mesh_chunk: 1, deltas: vec![(1, Vec3::X), (9, Vec3::Y)] }],
        };
        let mut p = vec![Vec3::ZERO; 3];
        set.apply(&mut p, &[("A".to_string(), 0.5), ("missing".to_string(), 1.0)]);
        assert_eq!(p[1], Vec3::new(0.5, 0.0, 0.0));
        assert_eq!(p[0], Vec3::ZERO);
    }

    #[test]
    fn real_face_has_slider_targets() {
        if !aa_cry::pak_available() {
            return;
        }
        let set = MorphSet::load("objects/characters/nuian/male/face/nu_m_face01.chr").unwrap().unwrap();
        assert_eq!(set.targets.len(), 91);
        assert!(set.find("eye_Size_max").is_some());
        assert!(set.find("hariharan_Morph").is_some());
        let fm = FaceMaker::load("objects/characters/nuian/male/face/nu_m_targets.xml").unwrap();
        assert_eq!(fm.targets.len(), 81);
        // every slider refers to existing targets
        for t in &fm.targets {
            for n in [&t.morph, &t.min_morph, &t.max_morph] {
                assert!(n.is_empty() || set.find(n).is_some(), "{} -> {n}", t.name);
            }
        }
    }
}
