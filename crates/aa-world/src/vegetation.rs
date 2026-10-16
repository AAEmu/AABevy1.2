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
    pub max_view_dist_ratio: f32,
    pub cast_shadow: bool,
}

#[derive(Clone, Debug, Default)]
pub struct VegGroups {
    pub groups: HashMap<u16, VegGroup>,
}

impl VegGroups {
    pub fn load(world: &str) -> Option<VegGroups> {
        let text = aa_pak::Pak::global().read_string(&format!("worlds/{world}/vegetation.xml")).ok()?;
        VegGroups::parse(&text)
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
                    max_view_dist_ratio: g.attribute("fMaxViewDistRatio").and_then(|v| v.parse().ok()).unwrap_or(1.0),
                    cast_shadow: flag("bCastShadow"),
                },
            );
        }
        Some(VegGroups { groups })
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
    fn real_main_world() {
        if !crate::pak_available() {
            return;
        }
        let g = VegGroups::load("main_world").unwrap();
        assert!(g.groups.len() > 4000);
        assert!(g.groups.values().all(|v| !v.model.is_empty()));
    }
}
