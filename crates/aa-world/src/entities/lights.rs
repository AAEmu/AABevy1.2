//! The original Light.lua and SimpleLight.lua slot properties, without renderer defaults.

use std::collections::{HashMap, HashSet};

use bevy::math::{Mat4, Vec3};

/// A native local-light slot. Radius is in metres and is independent of entity Scale.
#[derive(Clone, Debug)]
pub struct PlacedLight {
    pub name: String,
    pub guid: u64,
    pub matrix: Mat4,
    pub active: bool,
    pub hidden: bool,
    pub radius: f32,
    /// `clrDiffuse * fDiffuseMultiplier`, or SimpleLight's `clrColor * fColorMultiplier`.
    pub diffuse: Vec3,
    pub specular_multiplier: f32,
    pub hdr_dynamic: f32,
    pub ambient: bool,
    pub cast_shadow: bool,
    pub shadow_bias: f32,
    pub shadow_slope_bias: f32,
    pub projector_texture: String,
    pub projector_fov: f32,
    pub projector_near: f32,
    pub project_all_dirs: bool,
    pub negative: bool,
    pub fake: bool,
    pub area_only: bool,
    pub clip_bounds: bool,
    pub style: u16,
}

impl PlacedLight {
    /// Supports the point-light path. RGB projectors require the renderer's separate
    /// cookie/GPU validation; cubemap, negative and fake lights need other paths.
    pub fn supported(&self) -> bool {
        self.projector_texture.is_empty() && !self.negative && !self.fake && !self.clip_bounds
    }
}

/// Light and SimpleLight entities; inactive/hidden records remain available as placement data.
pub fn parse_lights(xml: &str) -> Vec<PlacedLight> {
    let Ok(doc) = roxmltree::Document::parse(xml.trim_start_matches('\u{feff}')) else {
        return Vec::new();
    };
    let Some(objects) = doc.root_element().children().find(|n| n.has_tag_name("Objects")) else {
        return Vec::new();
    };
    let nodes: HashMap<_, _> = objects
        .children()
        .filter(|n| n.has_tag_name("Entity"))
        .filter_map(|n| Some((super::guid(n.attribute("EntityGuid")?)?, n)))
        .collect();
    let mut out = Vec::new();
    for e in objects.children().filter(|n| n.has_tag_name("Entity")) {
        let simple = match e.attribute("EntityClass") {
            Some("Light") => false,
            Some("SimpleLight") => true,
            _ => continue,
        };
        let Some((matrix, hidden)) = super::transform(e, &nodes, &mut HashSet::new()) else {
            continue;
        };
        let Some(props) = e.children().find(|n| n.has_tag_name("Properties")) else {
            continue;
        };
        let child = |name| props.children().find(|n| n.has_tag_name(name));
        let number = |node: Option<roxmltree::Node<'_, '_>>, name, default| {
            node.and_then(|n| n.attribute(name))
                .and_then(|v| v.parse::<f32>().ok())
                .filter(|v| v.is_finite())
                .unwrap_or(default)
        };
        let flag = |node: Option<roxmltree::Node<'_, '_>>, name, default| {
            node.and_then(|n| n.attribute(name))
                .and_then(|v| v.parse::<u8>().ok())
                .map_or(default, |v| v != 0)
        };
        let color = child("Color");
        let multiplier = number(color, if simple { "fColorMultiplier" } else { "fDiffuseMultiplier" }, 1.0);
        let diffuse = color
            .and_then(|n| n.attribute(if simple { "clrColor" } else { "clrDiffuse" }))
            .and_then(super::vector::<3>)
            .map(Vec3::from)
            .unwrap_or(Vec3::ONE)
            * multiplier;
        let radius = number(Some(props), "Radius", 10.0);
        if radius <= 0.0 || !diffuse.is_finite() {
            continue;
        }
        let options = child("Options");
        let projector = child("Projector");
        let specular = if simple {
            number(color, "fSpecularPercentage", 100.0) * 0.01
        } else if multiplier != 0.0 {
            number(color, "fSpecularMultiplier", 1.0) / multiplier
        } else {
            1.0
        };
        out.push(PlacedLight {
            name: e.attribute("Name").unwrap_or("").to_string(),
            guid: e.attribute("EntityGuid").and_then(super::guid).unwrap_or(0),
            matrix,
            active: flag(Some(props), "bActive", true),
            hidden,
            radius,
            diffuse,
            specular_multiplier: specular,
            hdr_dynamic: number(color, "fHDRDynamic", 0.0),
            ambient: flag(options, "bAmbientLight", false),
            // The top-level CastShadow belongs to the entity render proxy; the light's own
            // shadow request comes from Options, exactly as Light:LoadLightToSlot builds it.
            cast_shadow: flag(options, "bCastShadow", false),
            shadow_bias: number(options, "fShadowBias", 10.0),
            shadow_slope_bias: number(options, "fShadowSlopeBias", 1.0),
            projector_texture: projector
                .and_then(|n| n.attribute("texture_Texture"))
                .map(super::asset_path)
                .unwrap_or_default(),
            projector_fov: number(projector, "fProjectorFov", 90.0),
            projector_near: number(projector, "fProjectorNearPlane", 0.0),
            project_all_dirs: flag(projector, "bProjectInAllDirs", false),
            negative: flag(child("Test"), "bNegativeLight", false),
            fake: flag(options, "bFakeLight", false),
            area_only: flag(options, "bAffectsThisAreaOnly", true),
            clip_bounds: flag(options, "bDeferredClipBounds", false),
            style: number(child("Style"), "nLightStyle", 0.0).clamp(0.0, u16::MAX as f32) as u16,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_schemas_preserve_native_parent_radius_color_and_slot_flags() {
        let xml = r#"<Mission><Objects>
          <Entity EntityClass="BasicEntity" EntityGuid="A" Pos="10,20,30" Scale="2,3,4"/>
          <Entity EntityClass="Light" EntityGuid="B" ParentGUID="A" Pos="1,2,3" CastShadow="0"><Properties bActive="0" Radius="8">
            <Color clrDiffuse="1,0.5,0.25" fDiffuseMultiplier="2" fSpecularMultiplier="3" fHDRDynamic="1"/>
            <Options bCastShadow="1"/><Projector texture_Texture=""/>
          </Properties></Entity>
          <Entity EntityClass="SimpleLight" EntityGuid="C" HiddenInGame="1"><Properties Radius="4">
            <Color clrColor="0.2,0.4,0.8" fColorMultiplier="3" fSpecularPercentage="25"/>
          </Properties></Entity>
        </Objects></Mission>"#;
        let lights = parse_lights(xml);
        assert_eq!(lights.len(), 2);
        assert_eq!(lights[0].matrix.w_axis.truncate(), Vec3::new(12.0, 26.0, 42.0));
        assert_eq!(
            lights[0].radius, 8.0,
            "entity scale transforms the slot position, not its authored radius"
        );
        assert_eq!(lights[0].diffuse, Vec3::new(2.0, 1.0, 0.5));
        assert!(!lights[0].active && lights[0].cast_shadow);
        assert_eq!(lights[0].specular_multiplier, 1.5);
        assert!(lights[1].hidden && lights[1].active);
        assert!((lights[1].diffuse - Vec3::new(0.6, 1.2, 2.4)).length() < 1e-6);
        assert_eq!(lights[1].specular_multiplier, 0.25);
    }

    #[test]
    fn real_streetlamp_light_and_parented_airport_light_match_authored_data() {
        if !crate::pak_available() {
            return;
        }
        let cell = crate::placement::load_cell_objects("main_world", 18, 7);
        assert_eq!(cell.lights.len(), 32);
        let lamp = cell.lights.iter().find(|l| l.name == "Light_116").unwrap();
        assert_eq!(lamp.guid, 0x405c2f3b9f41a176);
        assert_eq!(lamp.radius, 18.0);
        assert!((lamp.matrix.w_axis.truncate() - Vec3::new(533.82776, 566.42969, 106.61426)).length() < 0.001);
        assert!(lamp.active && !lamp.hidden && lamp.cast_shadow && lamp.supported());
        assert!((lamp.diffuse - Vec3::new(1.952942, 1.592158, 1.270588)).length() < 1e-5);
        assert_eq!(lamp.hdr_dynamic, 1.0);
        let cell = crate::placement::load_cell_objects("main_world", 10, 14);
        let lift = cell.lights.iter().find(|l| l.name == "Light129").unwrap();
        assert!((lift.matrix.w_axis.truncate() - Vec3::new(196.51, 760.55, 223.88553)).length() < 0.02);
    }
}
