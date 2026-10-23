//! A house is a prefab (`models.sub_type = PrefabModel`): `prefab_elements` names one `<Prefab>` of an editor xml
//! (`prefab://prefabs/housing_big.xml/housing_step_small.step1`), whose `<Objects>` place many models relative to the
//! prefab's origin. What is drawn here:
//!
//! * `Brush`: `Prefab=` (the `.cgf`), `Material=`, `Pos`, `Rotate` (`w,x,y,z`), `Scale`;
//! * `Entity` with an `object_Model` (the `AnimObject` doors, windows and scaffold pieces), and `GeomEntity` (`Geometry=`).
//!
//! Lights, particle effects, decals, vis areas and portals are left out. The `<export>` list after `<Objects>` repeats
//! the same objects flattened; it is not read.

use bevy::math::{Quat, Vec3};

use super::data::attr;

/// One model of a prefab, relative to the prefab's origin (cry space).
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    /// The model's pak path (lower case, `/`, without `game/`).
    pub model: String,
    /// The `.mtl` the editor gave it (pak path), if any.
    pub material: Option<String>,
    pub pos: Vec3,
    pub rot: Quat,
    pub scale: Vec3,
}

fn vec3(text: &str) -> Option<Vec3> {
    let mut it = text.split(',').map(|v| v.trim().parse::<f32>().ok());
    Some(Vec3::new(it.next()??, it.next()??, it.next()??))
}

/// `Rotate="w,x,y,z"`.
fn quat(text: &str) -> Option<Quat> {
    let mut it = text.split(',').map(|v| v.trim().parse::<f32>().ok());
    let (w, x, y, z) = (it.next()??, it.next()??, it.next()??, it.next()??);
    let q = Quat::from_xyzw(x, y, z, w);
    (q.length_squared() > 1e-6).then(|| q.normalize())
}

/// `Scale="s"` or `Scale="x,y,z"`.
fn scale(text: &str) -> Option<Vec3> {
    vec3(text).or_else(|| text.trim().parse::<f32>().ok().map(Vec3::splat))
}

/// A model reference as a pak path.
fn path(raw: &str) -> Option<String> {
    let p = raw.trim().replace('\\', "/").to_ascii_lowercase();
    let p = p.strip_prefix("game/").unwrap_or(&p).to_string();
    p.contains('.').then_some(p)
}

/// The parts of prefab `name` in the text of its xml file.
pub fn parts_of(xml: &str, name: &str) -> Vec<Part> {
    let Some(from) = xml.find(&format!("<Prefab Name=\"{name}\"")) else { return Vec::new() };
    let tail = &xml[from..];
    let prefab = &tail[..tail.find("</Prefab>").unwrap_or(tail.len())];
    let objects = &prefab[..prefab.find("<export>").unwrap_or(prefab.len())];
    let mut out = Vec::new();
    let mut rest = objects;
    while let Some(at) = rest.find("<Object ") {
        rest = &rest[at + 8..];
        // the object's own tag, and the body up to the next object (an entity's `<Properties>`)
        let tag = &rest[..rest.find('>').unwrap_or(rest.len())];
        let body = &rest[..rest.find("<Object ").unwrap_or(rest.len())];
        let model = match attr(tag, "Type") {
            Some("Brush") => attr(tag, "Prefab").and_then(path),
            Some("Entity") => match attr(tag, "EntityClass") {
                Some("GeomEntity") => attr(tag, "Geometry").and_then(path),
                _ => body.find("object_Model=\"").and_then(|s| {
                    let v = &body[s + 14..];
                    path(&v[..v.find('"')?])
                }),
            },
            _ => None,
        };
        let Some(model) = model else { continue };
        out.push(Part {
            model,
            material: attr(tag, "Material").filter(|m| !m.trim().is_empty()).and_then(|m| {
                let m = m.trim().replace('\\', "/").to_ascii_lowercase();
                let m = m.strip_prefix("game/").unwrap_or(&m);
                (!m.is_empty()).then(|| if m.ends_with(".mtl") { m.to_string() } else { format!("{m}.mtl") })
            }),
            pos: attr(tag, "Pos").and_then(vec3).unwrap_or(Vec3::ZERO),
            rot: attr(tag, "Rotate").and_then(quat).unwrap_or(Quat::IDENTITY),
            scale: attr(tag, "Scale").and_then(scale).unwrap_or(Vec3::ONE),
        });
    }
    out
}

/// The parts of a `prefab://file/name` element, read from the pak.
pub fn load(element: &str) -> Vec<Part> {
    let Some((file, name)) = element.strip_prefix("prefab://").and_then(|r| r.rsplit_once('/')) else { return Vec::new() };
    let file = file.replace('\\', "/").to_ascii_lowercase();
    match aa_pak::Pak::global().read_string(&file) {
        Ok(xml) => parts_of(&xml, name),
        Err(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &str = r#"<PrefabsLibrary>
 <Prefab Name="housing_new_g.step4_g" Id="{E31B}">
  <Objects>
   <Object Type="VisArea" Name="VisArea" Pos="3.9,-2.1,0.1"><Points><Point Pos="0,0,0"/></Points></Object>
   <Object Type="Comment" Name="origin" Pos="0,0,0" Comment=""/>
   <Object Type="Brush" Name="step4_g" Pos="0,0,0" Material="game/objects/env/01_nuia/001_housing/housing_house/nuia_h_house_g" Prefab="game/Objects/env/01_nuia/001_housing/housing_house/nuia_h_house_60_4a_out.cgf" RndFlags="2056"/>
   <Object Type="Brush" Name="barrel" Pos="1.5,-2,0.25" Rotate="0.70710671,0,0,0.70710683" Scale="0.88,0.88,0.88" Prefab="Objects\env\barrel.cgf"/>
   <Object Type="Entity" Name="interaction_11" Pos="-4.1,0.2,2.3" Rotate="0.7132,0,0,0.7009" EntityClass="AnimObject">
    <Properties bCanShatter="1" object_Model="game/Objects/env/01_nuia/001_housing/housing_house/nuia_housing_window_a.cga" object_ModelFrozen="">
     <Physics bPhysicalize="1"/>
    </Properties>
   </Object>
   <Object Type="Entity" Name="Housing_Light" Pos="0,0,2" EntityClass="Light"><Properties Radius="5"/></Object>
   <Object Type="Entity" Name="GeomEntity1" Pos="1,2,3" Geometry="Objects\housing_step1.cga" EntityClass="GeomEntity"/>
  </Objects>
  <export>
   <Entity Name="interaction_11" object_Model="ignored.cga"/>
  </export>
 </Prefab>
 <Prefab Name="other" Id="{1}"><Objects><Object Type="Brush" Prefab="x.cgf"/></Objects></Prefab>
</PrefabsLibrary>"#;

    #[test]
    fn brushes_anim_objects_and_geom_entities_are_the_parts() {
        let parts = parts_of(XML, "housing_new_g.step4_g");
        let models: Vec<&str> = parts.iter().map(|p| p.model.as_str()).collect();
        assert_eq!(
            models,
            [
                "objects/env/01_nuia/001_housing/housing_house/nuia_h_house_60_4a_out.cgf",
                "objects/env/barrel.cgf",
                "objects/env/01_nuia/001_housing/housing_house/nuia_housing_window_a.cga",
                "objects/housing_step1.cga",
            ]
        );
        assert_eq!(parts[0].material.as_deref(), Some("objects/env/01_nuia/001_housing/housing_house/nuia_h_house_g.mtl"));
        assert_eq!(parts[1].pos, Vec3::new(1.5, -2.0, 0.25));
        assert!((parts[1].scale - Vec3::splat(0.88)).length() < 1e-6);
        // w first: a quarter turn about Z
        let turned = parts[1].rot * Vec3::X;
        assert!((turned - Vec3::Y).length() < 1e-4, "{turned}");
        assert_eq!(parts[3].pos, Vec3::new(1.0, 2.0, 3.0));
        assert!(parts_of(XML, "missing").is_empty());
    }

    #[test]
    fn a_real_house_prefab_has_its_shell() {
        let parts = load("prefab://prefabs/housing_new_g.xml/housing_new_g.step4_g");
        if parts.is_empty() {
            return; // no client pak
        }
        assert!(parts.iter().any(|p| p.model.ends_with("nuia_h_house_60_4a_out.cgf")));
        assert!(parts.iter().any(|p| p.model.ends_with("nuia_housing_window_a.cga")));
    }
}
