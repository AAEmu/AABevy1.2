//! Particle libraries (`game/libs/particles/*.xml`): `<ParticleLibrary Name="X2_EFFECTS">` holds `<Particles Name="fire_smoke.torchlight_b">`
//! effects, each a tree of emitters (`<Params/>` plus `<Childs>`). An effect is addressed as `library.name`
//! (`X2_EFFECTS.fire_smoke.torchlight_b`, case does not matter): everything before the first dot is the library.

use std::collections::HashMap;
use std::sync::Arc;

use crate::params::{EmitterDef, Spawn};

/// One emitter and the emitters below it.
#[derive(Clone, Debug)]
pub struct Node {
    pub def: Arc<EmitterDef>,
    pub children: Vec<Node>,
}

/// An emitter of the flattened effect (parents come first).
#[derive(Clone, Debug)]
pub struct FlatEmitter {
    pub def: Arc<EmitterDef>,
    /// Index of the emitter whose particles trigger this one (`SpawnIndirection`).
    pub parent: Option<usize>,
}

/// A named effect of a library.
#[derive(Clone, Debug)]
pub struct EffectDef {
    pub name: String,
    pub root: Node,
    /// The emitters that do something (draw, or start other emitters), parents first.
    pub emitters: Vec<FlatEmitter>,
}

impl EffectDef {
    fn new(name: &str, root: Node) -> EffectDef {
        let mut emitters = Vec::new();
        flatten(&root, None, &mut emitters);
        EffectDef { name: name.to_string(), root, emitters }
    }

    /// Whether the effect loops: the real client takes it from the effect's own params (`FxParticle::Start` reads the byte at +1 of the
    /// root's `ParticleParams`, `bContinuous`; `docs/research/re-skills.md` 4.3), whatever the caller asks. A root without `Continuous`
    /// plays once even when the skill holds it (`psychokinesis_launch_form_end`, the launches of the beams), one with it starts again
    /// when it has finished (`life_absorb_launch`, `lightning_magic_form`).
    pub fn loops(&self) -> bool {
        self.root.def.continuous
    }

    /// Number of emitters that draw.
    pub fn drawn(&self) -> usize {
        self.emitters.iter().filter(|e| e.def.draws()).count()
    }

    /// The first emitter that emits a mesh (`Geometry`): the model path and the `.mtl` it is drawn with (empty: the model's own).
    /// The particle simulation does not draw these; an application that wants the mesh (an arrow in flight) spawns it itself.
    pub fn geometry(&self) -> Option<(&str, &str)> {
        fn find(node: &Node) -> Option<(&str, &str)> {
            if node.def.enabled && !node.def.geometry.is_empty() {
                return Some((node.def.geometry.as_str(), node.def.material.as_str()));
            }
            node.children.iter().find_map(find)
        }
        find(&self.root)
    }
}

fn flatten(node: &Node, parent: Option<usize>, out: &mut Vec<FlatEmitter>) {
    let triggers = node.children.iter().any(|c| matches!(c.def.spawn, Spawn::ParentStart | Spawn::ParentDeath | Spawn::ParentCollide) && c.def.enabled);
    let mut mine = parent;
    if node.def.enabled && (node.def.draws() || triggers) {
        out.push(FlatEmitter { def: node.def.clone(), parent: if matches!(node.def.spawn, Spawn::Direct) { None } else { parent } });
        mine = Some(out.len() - 1);
    }
    for c in &node.children {
        // a direct child (no indirection) starts with the effect, the others hang off this emitter
        let p = if matches!(c.def.spawn, Spawn::Direct) { None } else { mine };
        flatten(c, p, out);
    }
}

fn parse_node(el: roxmltree::Node, targets: &[Arc<EmitterDef>]) -> Node {
    let name = el.attribute("Name").unwrap_or("");
    let mut def = el
        .children()
        .find(|c| c.has_tag_name("Params"))
        .map(|p| EmitterDef::from_attributes(name, p.attributes()))
        .unwrap_or_else(|| EmitterDef::disabled(name));
    def.target_sources = targets.to_vec();
    let def = Arc::new(def);
    let mut targets = targets.to_vec();
    if def.enabled && def.force_target {
        targets.insert(0, def.clone());
    }
    let children = el
        .children()
        .filter(|c| c.has_tag_name("Childs"))
        .flat_map(|c| c.children().filter(|p| p.has_tag_name("Particles")))
        .map(|p| parse_node(p, &targets))
        .collect();
    Node { def, children }
}

/// A parsed library.
#[derive(Debug, Default)]
pub struct Library {
    pub name: String,
    /// Effects by lowercase name (without the library part).
    pub effects: HashMap<String, Arc<EffectDef>>,
}

impl Library {
    pub fn parse(xml: &str) -> Result<Library, String> {
        let xml = xml.trim_start_matches('\u{feff}');
        let doc = roxmltree::Document::parse(xml).map_err(|e| e.to_string())?;
        let root = doc.root_element();
        let mut lib = Library { name: root.attribute("Name").unwrap_or("").to_string(), ..Default::default() };
        for p in root.children().filter(|c| c.has_tag_name("Particles")) {
            let Some(name) = p.attribute("Name") else { continue };
            let node = parse_node(p, &[]);
            lib.effects.insert(name.to_ascii_lowercase(), Arc::new(EffectDef::new(name, node)));
        }
        Ok(lib)
    }

    /// Reads `libs/particles/<name>.xml` from the pak.
    pub fn load(name: &str) -> Result<Library, String> {
        let path = library_path(name);
        let text = aa_pak::Pak::global().read_string(&path).map_err(|e| format!("{path}: {e}"))?;
        Library::parse(&text)
    }

    pub fn get(&self, effect: &str) -> Option<&Arc<EffectDef>> {
        self.effects.get(&effect.to_ascii_lowercase())
    }

    /// The one effect whose name ends in the same last part as `effect` (`wild.long_distance_shot_launch` finds
    /// `test.long_distance_shot_launch`): the data has the folder part swapped in a few places. `None` when there is none or
    /// several. The real client does not do this (it logs "Particle effect not found"), see `FxSettings::name_fallback`.
    pub fn get_by_last_part(&self, effect: &str) -> Option<&Arc<EffectDef>> {
        let last = effect.rsplit('.').next()?.to_ascii_lowercase();
        let mut found = self.effects.iter().filter(|(k, _)| k.rsplit('.').next() == Some(last.as_str()));
        let first = found.next()?;
        found.next().is_none().then_some(first.1)
    }
}

/// `X2_EFFECTS` -> `libs/particles/x2_effects.xml`.
pub fn library_path(library: &str) -> String {
    format!("libs/particles/{}.xml", library.to_ascii_lowercase())
}

/// `"X2_EFFECTS.fire_smoke.torchlight_b"` -> `("X2_EFFECTS", "fire_smoke.torchlight_b")`.
pub fn split_name(full: &str) -> Option<(&str, &str)> {
    let (lib, rest) = full.trim().split_once('.')?;
    (!lib.is_empty() && !rest.is_empty()).then_some((lib, rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_effect_names() {
        assert_eq!(split_name("X2_EFFECTS.fire_smoke.torchlight_b"), Some(("X2_EFFECTS", "fire_smoke.torchlight_b")));
        assert_eq!(split_name("nodot"), None);
    }

    #[test]
    fn parses_a_tree() {
        let xml = r#"<ParticleLibrary Name="T"><Particles Name="a.b">
            <Params Continuous="true" Speed="0"/>
            <Childs>
              <Particles Name="fire"><Params Texture="textures/x.dds" Continuous="true"/>
                <Childs><Particles Name="spark"><Params Texture="textures/y.dds" SpawnIndirection="ParentStart"/></Particles></Childs>
              </Particles>
              <Particles Name="off"><Params Enabled="false" Texture="textures/z.dds"/></Particles>
            </Childs></Particles></ParticleLibrary>"#;
        let lib = Library::parse(xml).unwrap();
        let e = lib.get("A.B").unwrap();
        assert_eq!(e.emitters.len(), 2);
        assert_eq!(e.emitters[0].def.name, "fire");
        assert_eq!(e.emitters[0].parent, None);
        assert_eq!(e.emitters[1].parent, Some(0));
        assert_eq!(e.drawn(), 2);
    }

    #[test]
    fn last_part_fallback_needs_a_single_match() {
        let xml = r#"<ParticleLibrary Name="T"><Particles Name="test.shot_launch"><Params Texture="textures/g.dds"/></Particles>
            <Particles Name="a.twice"><Params Texture="textures/g.dds"/></Particles>
            <Particles Name="b.twice"><Params Texture="textures/g.dds"/></Particles></ParticleLibrary>"#;
        let lib = Library::parse(xml).unwrap();
        assert!(lib.get("wild.shot_launch").is_none());
        assert!(lib.get_by_last_part("wild.shot_launch").is_some());
        assert!(lib.get_by_last_part("c.twice").is_none());
        assert!(lib.get_by_last_part("c.missing").is_none());
    }

    #[test]
    fn finds_the_geometry_of_an_effect() {
        let xml = r#"<ParticleLibrary Name="T"><Particles Name="arrow">
            <Params Geometry="objects/effects/arrow.cgf" Material="materials/effects/arrow_a1"/>
            <Childs><Particles Name="glow"><Params Texture="textures/g.dds"/></Particles></Childs></Particles>
            <Particles Name="plain"><Params Texture="textures/g.dds"/></Particles></ParticleLibrary>"#;
        let lib = Library::parse(xml).unwrap();
        assert_eq!(lib.get("arrow").unwrap().geometry(), Some(("objects/effects/arrow.cgf", "materials/effects/arrow_a1.mtl")));
        assert_eq!(lib.get("plain").unwrap().geometry(), None);
    }

    #[test]
    fn real_libraries_parse() {
        if !aa_cry::pak_available() {
            return;
        }
        for name in ["smoke_and_fire", "misc", "x2_effects", "abillity_skill_table_m", "x2_effects_p", "wind", "rain"] {
            let lib = Library::load(name).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(!lib.effects.is_empty(), "{name}");
        }
        let lib = Library::load("x2_effects").unwrap();
        let torch = lib.get("fire_smoke.torchlight_b").expect("torch");
        assert!(torch.drawn() >= 2, "{}", torch.drawn());
        let lib = Library::load("abillity_skill_table_m").unwrap();
        assert!(lib.get("login_stage_effect.flow_fog_01").is_some());
    }
}
