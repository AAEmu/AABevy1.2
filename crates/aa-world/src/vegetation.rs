//! `worlds/<world>/vegetation.xml`: the vegetation group table (`<group id=.. modelFileName=..>`).
//!
//! `vegetation.dat` records refer to a group by id; the group names the model and carries the
//! per-species settings. 4,145 groups in main_world (2.3 MB of XML), parsed once in the background.

use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct VegGroup {
    pub id: u16,
    pub name: String,
    pub model: String,
    /// `matName` override, empty when the model's own material is used.
    pub material: String,
    pub use_sprites: bool,
    /// `fSpriteDistRatio` and `fSize`, the group's factors of the sprite switch distance (`objects::sprites`).
    pub sprite_dist_ratio: f32,
    pub size: f32,
    pub max_view_dist_ratio: f32,
    pub cast_shadow: bool,
    /// Native group brightness, before the render-time upper cap of one.
    pub brightness: f32,
    pub use_terrain_color: bool,
    /// Nonzero authored material-layer variants are not handled by the colour-only extension.
    pub material_layers: u32,
    /// Native group bending. Missing/malformed values are kept unsupported.
    pub bending: f32,
    pub complex_bending: bool,
}

impl VegGroup {
    /// Native non-terrain branch: group+36, capped only above one. Negative/nonfinite
    /// input is deliberately unsupported, rather than inventing a lower clamp.
    pub fn supported_brightness(&self) -> Option<f32> {
        (!self.use_terrain_color && self.brightness.is_finite() && self.brightness >= 0.0)
            .then(|| self.brightness.min(1.0))
    }

    pub(crate) fn style(&self) -> Style {
        Style {
            brightness: self.supported_brightness().unwrap_or(1.0),
            bending: if self.bending.is_finite() && (0.0..=10.0).contains(&self.bending) && !self.complex_bending {
                self.bending.clamp(0.01, 10.0)
            } else { 0.0 },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Style { pub brightness: f32, pub bending: f32 }

#[derive(Clone, Debug, Default)]
pub struct VegGroups {
    pub groups: HashMap<u16, VegGroup>,
    styles: HashMap<(String, String), Vec<Style>>,
    pub(crate) wind: bevy::math::Vec3,
}

fn normalized_asset(p: &str) -> String {
    aa_pak::normalize(p).trim_start_matches("game/").trim_end_matches(".mtl").to_string()
}

impl VegGroups {
    pub(crate) fn styles(&self, model: &str, material: Option<&str>) -> Vec<Style> {
        self.styles.get(&(normalized_asset(model),normalized_asset(material.unwrap_or(""))))
            .cloned().unwrap_or_else(|| vec![Style { brightness: 1.0, bending: 0.0 }])
    }
    /// Small pre-indexed variant list, queried once per model arrival, not every frame/instance.
    #[cfg(test)]
    fn brightnesses(&self, model: &str, material: Option<&str>) -> Vec<f32> {
        let mut values=Vec::new();
        for style in self.styles(model,material) {
            if !values.contains(&style.brightness) { values.push(style.brightness); }
        }
        values
    }
    pub fn load(world: &str) -> Option<VegGroups> {
        let text = aa_pak::Pak::global().read_string(&format!("worlds/{world}/vegetation.xml")).ok()?;
        let mut groups=VegGroups::parse(&text)?;
        groups.wind=aa_pak::Pak::global().read_string(&format!("worlds/{world}/env.xml"))
            .map(|xml| crate::vegetation_wind::environment(&xml)).unwrap_or(bevy::math::Vec3::X);
        Some(groups)
    }

    pub fn parse(xml: &str) -> Option<VegGroups> {
        let doc = roxmltree::Document::parse(xml).ok()?;
        let mut groups = HashMap::new();
        for g in doc.descendants().filter(|n| n.has_tag_name("group")) {
            let Some(id) = g.attribute("id").and_then(|v| v.parse::<u16>().ok()) else { continue };
            let model = g.attribute("modelFileName").unwrap_or("").trim().to_string();
            if model.is_empty() {
                continue;
            }
            let flag = |name: &str| g.attribute(name).is_some_and(|v| v == "1");
            groups.insert(
                id,
                VegGroup {
                    id,
                    name: g.attribute("name").unwrap_or("").to_string(),
                    model,
                    material: g.attribute("matName").unwrap_or("").trim().to_string(),
                    use_sprites: flag("bUseSprites"),
                    sprite_dist_ratio: g.attribute("fSpriteDistRatio").and_then(|v| v.parse().ok()).unwrap_or(1.0),
                    size: g.attribute("fSize").and_then(|v| v.parse().ok()).unwrap_or(1.0),
                    max_view_dist_ratio: g.attribute("fMaxViewDistRatio").and_then(|v| v.parse().ok()).unwrap_or(1.0),
                    cast_shadow: flag("bCastShadow"),
                    brightness: g.attribute("fBrightness").map(|v| v.parse().unwrap_or(f32::NAN)).unwrap_or(1.0),
                    use_terrain_color: flag("bUseTerrainColor"),
                    material_layers: g.attribute("nMaterialLayers").and_then(|v| v.parse().ok()).unwrap_or(0),
                    bending: g.attribute("fBending").and_then(|v| v.parse().ok()).unwrap_or(f32::NAN),
                    complex_bending: flag("bComplexBending"),
                },
            );
        }
        let mut styles: HashMap<_, Vec<Style>> = HashMap::new();
        for g in groups.values().filter(|g| g.material_layers == 0) {
            let values=styles.entry((normalized_asset(&g.model), normalized_asset(&g.material)))
                .or_insert_with(|| vec![Style { brightness: 1.0, bending: 0.0 }]);
            let style=g.style();
            if !values.contains(&style) { values.push(style); }
        }
        Some(VegGroups { groups, styles, wind: bevy::math::Vec3::X })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_groups() {
        let g = VegGroups::parse(
            r#"<vegetationMgr><groupList><group id="3" name="a" modelFileName="objects/x.cgf" matName="" bUseSprites="1" fMaxViewDistRatio="0.5" bCastShadow="1"/></groupList></vegetationMgr>"#,
        )
        .unwrap();
        let v = &g.groups[&3];
        assert_eq!(v.model, "objects/x.cgf");
        assert!(v.use_sprites && v.cast_shadow);
        assert_eq!(v.max_view_dist_ratio, 0.5);
    }

    #[test]
    fn brightness_uses_native_upper_cap_and_declines_unknown_terrain_path() {
        let xml = r#"<vegetationMgr><groupList>
          <group id="1" modelFileName="Game/Objects/a.cgf"/>
          <group id="2" modelFileName="objects/a.cgf" fBrightness="0.40000001"/>
          <group id="3" modelFileName="objects/a.cgf" fBrightness="10"/>
          <group id="4" modelFileName="objects/a.cgf" fBrightness="-0.2"/>
          <group id="5" modelFileName="objects/a.cgf" fBrightness="NaN"/>
          <group id="6" modelFileName="objects/a.cgf" fBrightness="0.4" bUseTerrainColor="1"/>
          <group id="7" modelFileName="objects/a.cgf" fBrightness="0"/>
          <group id="8" modelFileName="objects/a.cgf" fBrightness="0.7" matName="Game/Materials/b"/>
          <group id="9" modelFileName="objects/a.cgf" fBrightness="0.6" nMaterialLayers="1"/>
        </groupList></vegetationMgr>"#;
        let groups = VegGroups::parse(xml).unwrap();
        assert_eq!(groups.groups[&1].supported_brightness(), Some(1.0));
        assert_eq!(groups.groups[&2].supported_brightness(), Some(0.4));
        assert_eq!(groups.groups[&3].supported_brightness(), Some(1.0));
        for i in [4,5,6] { assert_eq!(groups.groups[&i].supported_brightness(), None); }
        assert_eq!(groups.groups[&7].supported_brightness(), Some(0.0));
        let mut values = groups.brightnesses("objects/a.cgf", None);
        values.sort_by(f32::total_cmp);
        assert_eq!(values, [0.0,0.4,1.0]); // no lower clamp, layer or terrain approximation.
        assert_eq!(groups.brightnesses("GAME/objects/a.cgf", Some("materials/b.mtl")), [1.0,0.7]);
        assert_eq!(groups.brightnesses("objects/other.cgf", None), [1.0]);
    }
    #[test]
    fn main_wind_uses_native_group_limits_and_declines_missing_or_complex_inputs() {
        let groups=VegGroups::parse(r#"<vegetationMgr><groupList>
            <group id="1" modelFileName="a.cgf" fBending="0"/>
            <group id="2" modelFileName="a.cgf" fBending="12"/>
            <group id="3" modelFileName="a.cgf" fBending="2" bComplexBending="1"/>
            <group id="4" modelFileName="a.cgf"/>
            <group id="5" modelFileName="a.cgf" fBending="NaN"/>
            <group id="6" modelFileName="a.cgf" fBending="0.4" nMaterialLayers="1"/>
            <group id="7" modelFileName="a.cgf" fBending="10"/>
        </groupList></vegetationMgr>"#).unwrap();
        assert_eq!(groups.groups[&1].style().bending,0.01);
        assert_eq!(groups.groups[&7].style().bending,10.0);
        for id in [2,3,4,5] {assert_eq!(groups.groups[&id].style().bending,0.0);}
        assert_eq!(groups.styles("a.cgf",None).len(),3);
    }

    #[test]
    fn original_palace_flowers_carry_nonunity_group_brightness() {
        if !crate::pak_available() { return; }
        let g = VegGroups::load("main_world").unwrap();
        assert_eq!(g.groups.len(),2745);
        assert_eq!(g.groups.values().filter(|g| g.brightness != 1.0).count(),8);
        let flowers = &g.groups[&618];
        assert_eq!(flowers.model,"objects/env/05_nature/00_common/flower_a.cgf");
        assert!(!flowers.use_terrain_color);
        assert_eq!(flowers.supported_brightness(),Some(0.4));
        let b = aa_pak::Pak::global().read("worlds/main_world/cells/013_010/client/vegetation.dat").unwrap();
        let plants = crate::placement::parse_vegetation_dat(&b).unwrap();
        assert_eq!(plants.iter().filter(|p| p.group == 618).count(),88);
    }

    #[test]
    fn real_main_world() {
        if !crate::pak_available() {
            return;
        }
        let g = VegGroups::load("main_world").unwrap();
        assert!(g.groups.len() > 2500, "{}", g.groups.len());
        // ids are sparse: the spawn area uses group 3799
        assert!(g.groups.contains_key(&3799));
        assert!(g.groups.values().all(|v| !v.model.is_empty()));
    }
    #[test]
    fn authored_shadow_eligibility_matches_original_compiled_record_flags() {
        if !crate::pak_available() { return; }
        let groups = VegGroups::load("main_world").unwrap();
        for (cell, casters, noncasters) in [("015_015",355,0),("010_010",2529,1275),("021_008",7137,89),("012_014",22467,5901)] {
            let bytes = aa_pak::Pak::global().read(&format!("worlds/main_world/cells/{cell}/client/vegetation.dat")).unwrap();
            let plants = crate::placement::parse_vegetation_dat(&bytes).unwrap();
            let count = plants.iter().filter(|v| v.flags & 0x8 != 0).count();
            assert_eq!((count, plants.len()-count), (casters,noncasters), "{cell}");
            for v in &plants {
                assert_eq!(v.flags & 0x8 != 0, groups.groups[&v.group].cast_shadow, "{cell} group{}",v.group);
            }
            if cell == "012_014" {
                let shine:Vec<_> = plants.iter().filter(|v| v.group == 1084).collect();
                assert_eq!(shine.len(),5792);
                assert!(shine.iter().all(|v| v.flags & 0x8 == 0));
            }
        }
    }

}
