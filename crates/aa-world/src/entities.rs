//! Static render slots authored in a cell's `entities.xml`.
//!
//! `GeomEntity` uses `Geometry`; `BasicEntity` and `AnimObject` use
//! `Properties.object_Model` (the original BasicEntity.lua calls `LoadObject(0, ...)`).
//! Static CGFs / stopped CGAs and playing rigid CGAs have separate parsers and draw bins.
//! Character actors retain their own lifecycle. Hidden slots remain data and never enter bins.

use std::collections::{HashMap, HashSet};

use bevy::math::{Mat4, Quat, Vec3};

mod lights;
pub use lights::{PlacedLight, parse_lights};

/// One static render slot, in cell-local CryEngine space.
#[derive(Clone, Debug)]
pub struct PlacedProp {
    pub name: String,
    pub guid: u64,
    pub model: String,
    pub material: Option<String>,
    /// Includes parent transforms; the streaming cell origin is added exactly once.
    pub matrix: Mat4,
    pub hidden: bool,
    pub cast_shadow: bool,
    pub animation: Option<EntityAnimation>,
}

#[derive(Clone, Debug)]
pub struct EntityAnimation {
    pub name: String,
    pub looped: bool,
    pub speed: f32,
    pub always_update: bool,
    pub physicalize_after: bool,
}

impl PlacedProp {
    pub fn translation(&self) -> Vec3 {
        self.matrix.w_axis.truncate()
    }
}

pub(crate) fn vector<const N: usize>(value: &str) -> Option<[f32; N]> {
    let values: Vec<f32> = value.split(',').map(|p| p.trim().parse()).collect::<Result<_, _>>().ok()?;
    let values: [f32; N] = values.try_into().ok()?;
    values.iter().all(|v| v.is_finite()).then_some(values)
}

pub(crate) fn guid(value: &str) -> Option<u64> {
    u64::from_str_radix(value.trim().trim_start_matches("0x"), 16).ok()
}

pub(crate) fn asset_path(value: &str) -> String {
    aa_pak::normalize(value.trim()).trim_start_matches("game/").to_string()
}

fn local(node: roxmltree::Node<'_, '_>) -> Option<Mat4> {
    let position = node.attribute("Pos").map(vector::<3>).unwrap_or(Some([0.0; 3]))?;
    let [w, x, y, z] = node.attribute("Rotate").map(vector::<4>).unwrap_or(Some([1.0, 0.0, 0.0, 0.0]))?;
    let rotation = Quat::from_xyzw(x, y, z, w);
    if rotation.length_squared() < 1e-8 {
        return None;
    }
    let scale = node.attribute("Scale").map(vector::<3>).unwrap_or(Some([1.0; 3]))?;
    let matrix = Mat4::from_scale_rotation_translation(Vec3::from(scale), rotation.normalize(), Vec3::from(position));
    (matrix.is_finite() && matrix.determinant().abs() > 1e-9).then_some(matrix)
}

// ParentGUID positions are parent-local in the native CEntityLoader::LoadEntity. Parents
// can have a class we do not render, so look them up among all direct mission entities.
pub(crate) fn transform(
    node: roxmltree::Node<'_, '_>,
    nodes: &HashMap<u64, roxmltree::Node<'_, '_>>,
    visiting: &mut HashSet<u64>,
) -> Option<(Mat4, bool)> {
    if visiting.len() >= 64 {
        return None;
    }
    let matrix = local(node)?;
    let hidden = node.attribute("HiddenInGame") == Some("1");
    let Some(parent) = node.attribute("ParentGUID") else {
        return Some((matrix, hidden));
    };
    let parent = guid(parent)?;
    if !visiting.insert(parent) {
        return None;
    }
    let (parent_matrix, parent_hidden) = transform(*nodes.get(&parent)?, nodes, visiting)?;
    visiting.remove(&parent);
    Some((parent_matrix * matrix, hidden || parent_hidden))
}

/// Parses native static slots, without creating editor boxes or loading animation actors.
pub fn parse_static_entities(xml: &str) -> Vec<PlacedProp> {
    parse_entities(xml, false)
}

/// Authored playing rigid CGA slots. Named clips / controller curves are validated by the
/// animation loader, not replaced with frozen render slots when unsupported.
pub fn parse_animated_entities(xml: &str) -> Vec<PlacedProp> {
    parse_entities(xml, true)
}

fn parse_entities(xml: &str, animated: bool) -> Vec<PlacedProp> {
    let Ok(doc) = roxmltree::Document::parse(xml.trim_start_matches('\u{feff}')) else {
        return Vec::new();
    };
    let Some(objects) = doc.root_element().children().find(|n| n.has_tag_name("Objects")) else {
        return Vec::new();
    };
    let nodes: HashMap<_, _> = objects
        .children()
        .filter(|n| n.has_tag_name("Entity"))
        .filter_map(|n| Some((guid(n.attribute("EntityGuid")?)?, n)))
        .collect();
    let mut out = Vec::new();
    for node in objects.children().filter(|n| n.has_tag_name("Entity")) {
        let class = node.attribute("EntityClass").unwrap_or("");
        let props = node.children().find(|n| n.has_tag_name("Properties"));
        let model = match class {
            "GeomEntity" => node.attribute("Geometry"),
            "BasicEntity" | "AnimObject" => props.and_then(|n| n.attribute("object_Model")),
            _ => continue,
        };
        let Some(model) = model.map(asset_path).filter(|s| !s.is_empty()) else {
            continue;
        };
        let anim = props.and_then(|n| n.children().find(|a| a.has_tag_name("Animation")));
        let playing = class == "AnimObject" && anim.and_then(|a| a.attribute("bPlaying")) == Some("1");
        if playing != animated || !(model.ends_with(".cgf") || model.ends_with(".cga")) || (animated && !model.ends_with(".cga")) {
            continue;
        }
        let animation = if animated {
            let anim = anim.unwrap();
            let speed: f32 = anim.attribute("Speed").and_then(|v| v.parse().ok()).unwrap_or(1.0);
            if !speed.is_finite() || speed < 0.0 { continue; }
            Some(EntityAnimation {
                name: anim.attribute("Animation").unwrap_or("Default").to_string(),
                looped: anim.attribute("bLoop") == Some("1"),
                speed,
                always_update: anim.attribute("bAlwaysUpdate") == Some("1"),
                physicalize_after: anim.attribute("bPhysicalizeAfterAnimation") == Some("1"),
            })
        } else { None };
        let Some((matrix, hidden)) = transform(node, &nodes, &mut HashSet::new()) else {
            continue;
        };
        let material = node
            .attribute("Material")
            .map(asset_path)
            .filter(|m| !m.is_empty())
            .map(|m| if m.ends_with(".mtl") { m } else { format!("{m}.mtl") });
        out.push(PlacedProp {
            name: node.attribute("Name").unwrap_or("").to_string(),
            guid: node.attribute("EntityGuid").and_then(guid).unwrap_or(0),
            model,
            material,
            matrix,
            hidden,
            cast_shadow: node.attribute("CastShadow") != Some("0"),
            animation,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_slots_preserve_parent_scale_material_and_hidden_state() {
        let xml = r#"<Mission><Objects>
          <Entity EntityClass="CameraSource" EntityGuid="A" Pos="10,20,30" Rotate="0.70710678,0,0,0.70710678" Scale="2,3,4"/>
          <Entity Name="lamp" EntityClass="BasicEntity" EntityGuid="B" ParentGUID="A" Pos="1,2,3" Material="Game/Materials/Test" CastShadow="0">
            <Properties object_Model="Objects/Test.cgf"/>
          </Entity>
          <Entity Name="hidden target" EntityClass="AnimObject" EntityGuid="C" HiddenInGame="1"><Properties object_Model="target.cgf"><Animation bPlaying="0"/></Properties></Entity>
          <Entity EntityClass="AnimObject"><Properties object_Model="windmill.cga"><Animation bPlaying="1"/></Properties></Entity>
          <Entity EntityClass="GeomEntity" Geometry="character.cdf"/>
          <Entity EntityClass="BasicEntity" EntityGuid="D" ParentGUID="E"><Properties object_Model="cycle.cgf"/></Entity>
          <Entity EntityClass="BasicEntity" EntityGuid="E" ParentGUID="D"><Properties object_Model="cycle.cgf"/></Entity>
        </Objects></Mission>"#;
        let props = parse_static_entities(xml);
        assert_eq!(props.len(), 2);
        let lamp = &props[0];
        assert!((lamp.translation() - Vec3::new(4.0, 22.0, 42.0)).length() < 1e-4);
        assert_eq!(lamp.model, "objects/test.cgf");
        assert_eq!(lamp.material.as_deref(), Some("materials/test.mtl"));
        assert!(!lamp.cast_shadow);
        assert_eq!(lamp.guid, 0xB);
        assert!(props[1].hidden);
    }

    #[test]
    fn real_entity_slots_include_streetlamp_and_stopped_login_models() {
        if !crate::pak_available() {
            return;
        }
        let lamp = crate::placement::load_cell_objects("main_world", 18, 7);
        assert_eq!(lamp.entity_props.len(), 1);
        assert_eq!(
            crate::objects::CellObjectData::new(lamp.clone(), 220.0)
                .entity_bins
                .iter()
                .map(Vec::len)
                .sum::<usize>(),
            1
        );
        let lamp = &lamp.entity_props[0];
        assert!(lamp.model.ends_with("ncom_city_streetlamp01.cgf"));
        assert!(!lamp.hidden);
        assert!((lamp.translation() - Vec3::new(535.62256, 566.70435, 101.87896)).length() < 0.001);
        let model = aa_cry::Model::load(&lamp.model).expect("original streetlamp CGF");
        assert!(model.meshes.iter().any(|m| !m.indices.is_empty()));
        let login = crate::placement::load_cell_objects("login2", 1, 0);
        let visible: Vec<_> = login.entity_props.iter().filter(|p| !p.hidden).collect();
        assert_eq!(
            visible.len(),
            16,
            "five map CGFs, one miniature boat CGF and ten stopped miniature CGAs"
        );
        assert!(
            visible
                .iter()
                .any(|p| p.name == "wm_origin" && p.matrix.x_axis.truncate().length() > 1.9)
        );
        assert_eq!(
            login
                .entity_props
                .iter()
                .filter(|p| p.hidden && p.name.starts_with("target_"))
                .count(),
            12
        );
        let miniature = visible.iter().find(|p| p.name == "select_objet12").unwrap();
        assert!(
            !aa_pak::Pak::global().exists(&miniature.model),
            "this old login2 miniature references an absent model"
        );
        let present: Vec<_> = visible.iter().filter(|p| aa_pak::Pak::global().exists(&p.model)).collect();
        assert_eq!(present.len(), 1, "fifteen obsolete stage model references are absent from this pak");
        assert_eq!(present[0].name, "select_objet02");
        let model = aa_cry::Model::load(&present[0].model).expect("original miniature boat CGF");
        assert!(model.meshes.iter().any(|m| !m.indices.is_empty()));
        // This CGA is present and authored in the same login2 XML. It plays natively, so it is
        // deliberately absent from the static bins; loading it validates the CGA rest geometry.
        let model = aa_cry::Model::load("objects/env/02_harihara/03_ti/ti_factorygate_a.cga").expect("original CGA rest geometry");
        assert!(model.meshes.iter().any(|m| !m.indices.is_empty()));
    }

    #[test]
    fn entity_bins_exclude_hidden_and_identical_brush_slots() {
        let mut objects = crate::placement::CellObjects::default();
        objects.entity_props = parse_static_entities(
            r#"<Mission><Objects>
          <Entity EntityClass="BasicEntity" Name="visible" Pos="40,90,0"><Properties object_Model="Game/Objects/lamp.cgf"/></Entity>
          <Entity EntityClass="BasicEntity" Name="hidden" HiddenInGame="1"><Properties object_Model="Objects/target.cgf"/></Entity>
        </Objects></Mission>"#,
        );
        let data = crate::objects::CellObjectData::new(objects.clone(), 200.0);
        assert_eq!(data.entity_bins.iter().map(Vec::len).sum::<usize>(), 1);
        assert_eq!(data.entity_bins[1], [0]);
        objects.models.push("game/OBJECTS/lamp.cgf".to_string());
        objects.brushes.push(crate::placement::Brush {
            model: 0,
            material: -1,
            flags: 0,
            max_view_dist: 200.0,
            bbox: [0.0; 6],
            matrix: [1.0, 0.0, 0.0, 40.0, 0.0, 1.0, 0.0, 90.0, 0.0, 0.0, 1.0, 0.0],
        });
        let data = crate::objects::CellObjectData::new(objects, 200.0);
        assert_eq!(data.brush_bins.iter().map(Vec::len).sum::<usize>(), 1);
        assert!(data.entity_bins.iter().all(Vec::is_empty));
    }
}
