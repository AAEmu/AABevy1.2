//! Character customisation data: what the character-create screen changes (and what the server
//! stores in `UnitCustomModelParams`): face morph weights, face texture set, colours, decals.
//!
//! The screen drives the native `X2Customizer` API (`x2ui/customizing/*.lua`); how each call maps
//! onto [`CharacterDesc`](crate::CharacterDesc) / [`Customization`]:
//!
//! | client call | field |
//! |---|---|
//! | `SetCustomizingHair(index)` | `CharacterDesc::hair` (`item_body_parts.item_id`) |
//! | `SetCustomizingHairColor(index)` | `CharacterDesc::hair_color` (`hair_colors.id` of that style) |
//! | `SetCustomizingSkinColor(index)` | `CharacterDesc::skin` (`skin_colors.id`) |
//! | `SetCustomizingFaceNormal(index, weight)` | `normal_map`, `normal_map_weight` (`face_normal_maps.id`) |
//! | `SetCustomizingFaceDiffuse(index)` | `face_diffuse_map_id` (the table is empty in 1.2; unused) |
//! | `SetCustomizingTattoo(index)` / `SetCustomizingMakeUp(index, weight)` | decal, category 2 / 3 |
//! | `SetCustomizingEyebrow(index)`, `...EyebrowColor(r,g,b)` | decal category 4, `eyebrow_color` |
//! | `SetCustomizingDeco(index, weight)`, `...DecoColor(r,g,b)` | decal category 5, `deco_color` |
//! | `SetCustomizingScar(index, x, y, scale, rotate)` | the movable decal (category 1) |
//! | `SetCustomizingPupilColor` / `SetCustomizingLipColor` | `left/right_pupil_color`, `lip_color` |
//! | `ModifyFaceParamValue(index, value)` | `modifier[index]` (signed byte, range from `<race>_targets.xml`) |

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

    /// Sets face slider `idx` (`Idx` of `<race>_targets.xml`; -100..100 style values, see the
    /// slider's `Min`/`Max`); grows the modifier to its 128 bytes.
    pub fn set_slider(&mut self, idx: usize, value: i8) {
        if self.modifier.len() < 128 {
            self.modifier.resize(128, 0);
        }
        if let Some(m) = self.modifier.get_mut(idx) {
            *m = value as u8;
        }
    }

    /// The signed value (-128..127) of morph target `idx` (0 when absent).
    pub fn value(&self, idx: usize) -> i8 {
        self.modifier.get(idx).map(|&b| b as i8).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sliders_are_signed_bytes() {
        let mut c = Customization::default();
        c.set_slider(25, -80);
        c.set_slider(4, 35);
        assert_eq!(c.modifier.len(), 128);
        assert_eq!(c.value(25), -80);
        assert_eq!(c.value(4), 35);
        assert_eq!(c.value(500), 0);
    }
}
