//! Character customisation data: what the character-create screen changes (and what the server
//! stores in `UnitCustomModelParams`): face morph weights, face texture set, colours, decals.

use crate::db::{Decal, Preset};

/// Per-character appearance parameters beyond race / gender / hair / skin.
#[derive(Clone, Debug, PartialEq)]
pub struct Customization {
    /// 128 signed morph weights, indexed by `<race>_targets.xml` `Idx` (value 100 = full; the
    /// range of each target is its `Min`..`Max`). Empty = the unmodified base face.
    pub modifier: Vec<u8>,
    /// `face_normal_maps.id` (0 = the material's own normal map).
    pub normal_map: u32,
    pub normal_map_weight: f32,
    /// `0xAARRGGBB`
    pub lip_color: u32,
    pub left_pupil_color: u32,
    pub right_pupil_color: u32,
    pub eyebrow_color: u32,
    pub deco_color: u32,
    /// `face_decal_assets.id` makeup / tattoo layers, drawn over the face texture in order.
    pub decals: Vec<Decal>,
}

impl Default for Customization {
    fn default() -> Self {
        Customization {
            modifier: Vec::new(),
            normal_map: 0,
            normal_map_weight: 1.0,
            lip_color: 0,
            left_pupil_color: 0,
            right_pupil_color: 0,
            eyebrow_color: 0,
            deco_color: 0,
            decals: Vec::new(),
        }
    }
}

impl Customization {
    pub fn from_preset(p: &Preset) -> Customization {
        let mut decals: Vec<Decal> = Vec::new();
        if p.movable_decal.asset_id != 0 {
            decals.push(p.movable_decal);
        }
        decals.extend(p.fixed_decals.iter().filter(|d| d.asset_id != 0));
        Customization {
            modifier: p.modifier.clone(),
            normal_map: p.face_normal_map_id,
            normal_map_weight: p.face_normal_map_weight,
            lip_color: p.lip_color,
            left_pupil_color: p.left_pupil_color,
            right_pupil_color: p.right_pupil_color,
            eyebrow_color: p.eyebrow_color,
            deco_color: p.deco_color,
            decals,
        }
    }

    /// The signed value (-128..127) of morph target `idx` (0 when absent).
    pub fn value(&self, idx: usize) -> i8 {
        self.modifier.get(idx).map(|&b| b as i8).unwrap_or(0)
    }
}
