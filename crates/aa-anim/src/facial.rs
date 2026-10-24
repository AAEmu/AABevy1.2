//! Authored facial sequences (`.fsq`) and expression libraries (`.fxl`).
//!
//! Sequence channels contain Hermite splines in seconds. FXL controllers recursively map
//! expression values to morphs through Catmull–Rom curves (or a constant multiplier).
//! Neither channel folders nor the editor's selected/expanded flags affect playback.

mod bone;
mod curve;
mod remap;
mod runtime;
mod sound;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use glam::Vec3;

use crate::{Error, Result};
pub use curve::{Curve, Key};
pub(crate) use runtime::prepare_face;
pub(crate) use runtime::{character_face_enabled, prepare_character_face, validate_replacement_face, rebind_character_face};
pub use runtime::{FacialAnimator, FacialPlugin, prepare_loaded_face, enable_character_face, disable_character_face};
pub use sound::{FacialSentence, SentenceWord, SoundEntry};

#[derive(Clone, Debug, Default)]
pub struct FacialSequence {
    pub start: f32,
    pub end: f32,
    pub channels: Vec<Channel>,
    /// Retained for callers playing the matching body animation.
    pub skeleton_animation: Option<String>,
    /// Number of sound entries, including the legacy root-level entry.
    pub sound_entries: usize,
    /// Authored audio references and sentence data. No audio or phonemes are synthesized.
    pub sounds: Vec<SoundEntry>,
}

#[derive(Clone, Debug)]
pub struct Channel {
    pub name: String,
    pub flags: u32,
    pub splines: Vec<Curve>,
}

impl Channel {
    pub fn sample(&self, time: f32) -> f32 {
        self.splines.iter().map(|s| s.sample(time)).sum()
    }
}

impl FacialSequence {
    pub fn parse(xml: &str) -> Result<Self> {
        let doc = roxmltree::Document::parse(xml).map_err(|e| Error::Xml(e.to_string()))?;
        let root = doc.root_element();
        if !root.has_tag_name("FacialSequence") {
            return Err(Error::Format("expected FacialSequence".into()));
        }
        let start = number(root, "StartTime", 0.0)?;
        let end = number(root, "EndTime", start)?;
        if end < start {
            return Err(Error::Format("facial sequence ends before its start".into()));
        }
        let mut channels = Vec::new();
        for node in root.descendants().filter(|n| n.has_tag_name("Channel")) {
            let flags = integer(node, "Flags", 0)?;
            // A group is a folder; the other low flags denote procedural/balance controls.
            if flags & 0xff != 0 {
                continue;
            }
            let splines = node
                .children()
                .filter(|n| n.has_tag_name("Spline"))
                .map(|n| Curve::parse(n.attribute("Keys").unwrap_or("")))
                .collect::<Result<Vec<_>>>()?;
            if splines.iter().any(|s| !s.keys.is_empty()) {
                channels.push(Channel {
                    name: name(node, "Name"),
                    flags,
                    splines,
                });
            }
        }
        let sounds = sound::parse(root)?;
        Ok(Self {
            start,
            end,
            channels,
            skeleton_animation: root
                .children()
                .find(|n| n.has_tag_name("SkeletonAnimationEntry"))
                .and_then(|n| n.attribute("AnimationName"))
                .map(str::to_owned),
            sound_entries: sounds.len(),
            sounds,
        })
    }

    pub fn load(path: &str) -> Result<Arc<Self>> {
        static CACHE: OnceLock<Mutex<HashMap<String, Arc<FacialSequence>>>> = OnceLock::new();
        cached(path, &CACHE, Self::parse)
    }

    pub fn duration(&self) -> f32 {
        self.end - self.start
    }
}

#[derive(Clone, Debug)]
pub enum EffectorKind {
    Morph,
    Bone {
        attachment: String,
        position: Vec3,
        rotation: Vec3,
    },
    Expression,
    Group,
}

#[derive(Clone, Debug)]
pub struct Controller {
    pub effector: String,
    pub weight: f32,
    /// `None` means `input * weight`; spline controllers do not multiply by Weight.
    pub curve: Option<Curve>,
}

#[derive(Clone, Debug)]
pub struct Effector {
    pub name: String,
    pub kind: EffectorKind,
    pub controllers: Vec<Controller>,
}

#[derive(Clone, Debug, Default)]
pub struct ExpressionLibrary {
    pub effectors: HashMap<String, Effector>,
    // Native face-state indices are assigned while reading AllEffectors. Rotations on the
    // same joint do not commute; HashMap iteration must never choose their composition order.
    bone_order: Vec<String>,
}

impl ExpressionLibrary {
    pub fn parse(xml: &str) -> Result<Self> {
        let doc = roxmltree::Document::parse(xml).map_err(|e| Error::Xml(e.to_string()))?;
        let root = doc.root_element();
        if !root.has_tag_name("ExpressionLibrary") {
            return Err(Error::Format("expected ExpressionLibrary".into()));
        }
        let mut effectors = HashMap::new();
        let mut bone_order = Vec::new();
        if let Some(all) = root.children().find(|n| n.has_tag_name("AllEffectors")) {
            for node in all.children().filter(|n| n.has_tag_name("Effector")) {
                let name = name(node, "Name");
                let kind = match node.attribute("Type").unwrap_or("group") {
                    "morph" => EffectorKind::Morph,
                    "expression" => EffectorKind::Expression,
                    "group" => EffectorKind::Group,
                    "bone" => EffectorKind::Bone {
                        attachment: node.attribute("Attachment").unwrap_or("").into(),
                        position: vector(node, "PosOffset")?,
                        rotation: vector(node, "RotOffset")?,
                    },
                    other => return Err(Error::Format(format!("facial effector type {other}"))),
                };
                if matches!(kind, EffectorKind::Bone { .. }) && !bone_order.contains(&name) {
                    bone_order.push(name.clone());
                }
                effectors.insert(
                    name.clone(),
                    Effector {
                        name,
                        kind,
                        controllers: Vec::new(),
                    },
                );
            }
        }
        if let Some(all) = root.children().find(|n| n.has_tag_name("Controllers")) {
            for node in all.children().filter(|n| n.has_tag_name("Effector")) {
                let Some(effector) = effectors.get_mut(&name(node, "Name")) else {
                    continue;
                };
                for sub in node.children().filter(|n| n.has_tag_name("SubCtrl")) {
                    let curve = match integer(sub, "Type", 0)? {
                        0 => None,
                        1 => Some(Curve::parse(sub.attribute("Keys").unwrap_or(""))?),
                        other => return Err(Error::Format(format!("facial controller type {other}"))),
                    };
                    effector.controllers.push(Controller {
                        effector: name(sub, "Effector"),
                        weight: number(sub, "Weight", 0.0)?,
                        curve,
                    });
                }
            }
        }
        Ok(Self { effectors, bone_order })
    }

    pub fn load(path: &str) -> Result<Arc<Self>> {
        static CACHE: OnceLock<Mutex<HashMap<String, Arc<ExpressionLibrary>>>> = OnceLock::new();
        cached(path, &CACHE, Self::parse)
    }

    /// Evaluates authored channels into morph weights in `targets` order, without allocations.
    pub fn sample_morphs(&self, sequence: &FacialSequence, time: f32, targets: &[String], out: &mut [f32]) {
        self.sample_weights(sequence, time, targets, out);
    }

    /// Evaluates both morph and bone leaf weights. Contributions to the same bone effector
    /// sum before quaternion weighting, as in the native face state.
    pub fn sample_weights(&self, sequence: &FacialSequence, time: f32, targets: &[String], out: &mut [f32]) {
        out.fill(0.0);
        for channel in &sequence.channels {
            self.apply(&channel.name, channel.sample(time), targets, out, 0);
        }
    }

    fn apply(&self, name: &str, input: f32, targets: &[String], out: &mut [f32], depth: usize) {
        if input.abs() < 0.0001 || !input.is_finite() || depth >= 32 {
            return;
        }
        let Some(effector) = self.effectors.get(name) else { return };
        match effector.kind {
            EffectorKind::Morph | EffectorKind::Bone { .. } => {
                if let Some(i) = targets.iter().position(|s| s.eq_ignore_ascii_case(name)) {
                    if let Some(w) = out.get_mut(i) {
                        *w += input;
                    }
                }
            }
            EffectorKind::Group | EffectorKind::Expression => {
                for controller in &effector.controllers {
                    let value = controller
                        .curve
                        .as_ref()
                        .map_or(input * controller.weight, |c| c.sample_catmull(input));
                    self.apply(&controller.effector, value, targets, out, depth + 1);
                }
            }
        }
    }
}

fn cached<T>(path: &str, cache: &OnceLock<Mutex<HashMap<String, Arc<T>>>>, parse: fn(&str) -> Result<T>) -> Result<Arc<T>> {
    let key = path.to_ascii_lowercase().replace('\\', "/");
    let cache = cache.get_or_init(Default::default);
    if let Some(value) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return Ok(value.clone());
    }
    let bytes = aa_cry::read(&key)?;
    let xml = std::str::from_utf8(&bytes).map_err(|e| Error::Xml(e.to_string()))?;
    let value = Arc::new(parse(xml)?);
    Ok(cache.lock().unwrap_or_else(|e| e.into_inner()).entry(key).or_insert(value).clone())
}

fn name(n: roxmltree::Node<'_, '_>, attr: &str) -> String {
    n.attribute(attr).unwrap_or("").to_ascii_lowercase()
}
fn number(n: roxmltree::Node<'_, '_>, attr: &str, default: f32) -> Result<f32> {
    let Some(raw) = n.attribute(attr) else { return Ok(default) };
    raw.parse::<f32>()
        .ok()
        .filter(|v| v.is_finite())
        .ok_or_else(|| Error::Format(format!("invalid {attr}={raw}")))
}
fn integer(n: roxmltree::Node<'_, '_>, attr: &str, default: u32) -> Result<u32> {
    n.attribute(attr)
        .map_or(Ok(default), |s| s.parse().map_err(|_| Error::Format(format!("invalid {attr}={s}"))))
}
fn vector(n: roxmltree::Node<'_, '_>, attr: &str) -> Result<Vec3> {
    let Some(raw) = n.attribute(attr) else { return Ok(Vec3::ZERO) };
    let v: Vec<f32> = raw
        .split(',')
        .map(str::parse)
        .collect::<std::result::Result<_, _>>()
        .map_err(|_| Error::Format(format!("invalid {attr}={raw}")))?;
    if v.len() != 3 || v.iter().any(|v| !v.is_finite()) {
        return Err(Error::Format(format!("invalid {attr}={raw}")));
    }
    Ok(Vec3::new(v[0], v[1], v[2]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_multisplines_signed_values_and_metadata() {
        let sequence = FacialSequence::parse(
            r#"<FacialSequence StartTime="2" EndTime="4">
          <Channel Name="folder" Flags="8193"><Channel Name="A" Flags="4096">
            <Spline Keys="2:-1:0,4:1:0,"/><Spline Keys="2:0.5:0,"/>
          </Channel></Channel><Channel Name="balance" Flags="8"><Spline Keys="0:1:0,"/></Channel>
          <SkeletonAnimationEntry AnimationName="talk"/><SoundEntry/>
        </FacialSequence>"#,
        )
        .unwrap();
        assert_eq!(sequence.channels.len(), 1);
        assert_eq!(sequence.channels[0].sample(2.0), -0.5);
        assert_eq!(sequence.channels[0].sample(4.0), 1.5);
        assert_eq!(sequence.duration(), 2.0);
        assert_eq!(sequence.skeleton_animation.as_deref(), Some("talk"));
        assert_eq!(sequence.sound_entries, 1);
    }

    #[test]
    fn recursive_expression_controllers_and_cycle_bound() {
        let lib = ExpressionLibrary::parse(
            r#"<ExpressionLibrary><AllEffectors>
          <Effector Name="a" Type="expression"/><Effector Name="middle" Type="expression"/>
          <Effector Name="m_a" Type="morph"/><Effector Name="cycle" Type="group"/>
        </AllEffectors><Controllers>
          <Effector Name="a"><SubCtrl Weight="0.5" Effector="middle"/></Effector>
          <Effector Name="middle"><SubCtrl Type="1" Weight="9" Keys="-1:-1,1:1," Effector="m_a"/></Effector>
          <Effector Name="cycle"><SubCtrl Weight="1" Effector="cycle"/></Effector>
        </Controllers></ExpressionLibrary>"#,
        )
        .unwrap();
        let seq = FacialSequence::parse(
            r#"<FacialSequence><Channel Name="a"><Spline Keys="0:1:0,"/></Channel>
          <Channel Name="cycle"><Spline Keys="0:1:0,"/></Channel></FacialSequence>"#,
        )
        .unwrap();
        let mut weights = [0.0];
        lib.sample_morphs(&seq, 0.0, &["m_a".into()], &mut weights);
        // Catmull-Rom endpoints have half the segment's slope. Weight=9 is ignored for Type=1.
        assert!((weights[0] - 0.59375).abs() < 1e-6);
    }

    #[test]
    fn original_idle_blink_and_speech_keys() {
        if !aa_cry::pak_available() {
            return;
        }
        let library = ExpressionLibrary::load("animations/nuian/male/facial/nuian_m.fxl").unwrap();
        let seq = FacialSequence::load("animations/nuian/male/facial/fist_ac_stage_idle.fsq").unwrap();
        let targets = vec!["eye_close_l".into(), "eye_close_r".into(), "m_a".into()];
        let mut weights = [0.0; 3];
        library.sample_morphs(&seq, 0.7, &targets, &mut weights);
        assert_eq!(weights, [0.0; 3]);
        library.sample_morphs(&seq, 1.08, &targets, &mut weights);
        assert!(weights[0] > 0.99 && weights[1] > 0.99, "{weights:?}");
        assert_eq!(weights[2], 0.0);
        library.sample_morphs(&seq, 1.3, &targets, &mut weights);
        assert_eq!(weights[0], 0.0);
        let speech = FacialSequence::load("animations/nuian/male/facial/default.fsq").unwrap();
        library.sample_morphs(&speech, 3.5, &targets, &mut weights);
        assert!(weights[2] > 0.85, "{weights:?}");
    }

    #[test]
    fn decodes_original_facial_catalog() {
        if !aa_cry::pak_available() {
            return;
        }
        let pak = aa_pak::Pak::global();
        let mut counts = (0, 0);
        let mut sounds = 0;
        for path in pak.names().filter(|p| p.ends_with(".fsq") || p.ends_with(".fxl")) {
            let bytes = pak.read(path).unwrap();
            let xml = std::str::from_utf8(&bytes).unwrap();
            if path.ends_with(".fsq") {
                let sequence = FacialSequence::parse(xml).unwrap_or_else(|e| panic!("{path}: {e}"));
                assert_eq!(sequence.sound_entries, sequence.sounds.len());
                for sound in &sequence.sounds {
                    sounds += 1;
                    assert!(path.contains("nuian/male/machinima_facial/"), "{path}");
                    assert_eq!(sound.start, 0.0, "{path}");
                    let sentence = sound.sentence.as_ref().expect(path);
                    assert_eq!(sentence.text, "", "{path}");
                    assert!(sentence.words.is_empty(), "{path}");
                    assert!(sentence.phonemes.is_empty(), "{path}");
                    assert!(!pak.exists(&sound.sound), "unexpected authored WAV: {}", sound.sound);
                }
                counts.0 += 1;
            } else {
                ExpressionLibrary::parse(xml).unwrap_or_else(|e| panic!("{path}: {e}"));
                counts.1 += 1;
            }
        }
        assert_eq!(counts, (894, 9));
        assert_eq!(sounds, 7);
    }

    #[test]
    fn original_heads_transfer_only_when_geometry_proves_equivalent() {
        if !aa_cry::pak_available() {
            return;
        }
        let db = crate::GameDb::open().unwrap();
        let mut supported = 0;
        for race in [crate::Race::Nuian, crate::Race::Elf, crate::Race::Hariharan, crate::Race::Ferre] {
            for gender in [crate::Gender::Male, crate::Gender::Female] {
                let assets = crate::CharacterDesc::default_for(&db, race, gender).unwrap().resolve(&db).unwrap();
                let face = assets.parts.iter().find(|p| p.kind == crate::PartKind::Face).unwrap();
                let path = face.path.replace("_face01.chr", "_face00.chr");
                let neutral = aa_cry::Model::load(&face.path).unwrap();
                let set = remap::expression_set(&face.path, &neutral).unwrap_or_else(|e| panic!("{race:?} {gender:?}: {e}"));
                supported += 1;
                assert_eq!(set.targets.len(), if race == crate::Race::Ferre { 33 } else { 28 });
                assert!(set.find("eye_close_l").is_some(), "{path}");
                assert!(set.find("eye_close_r").is_some(), "{path}");
                assert!(set.find("m_a").is_some(), "{path}");
            }
        }
        assert_eq!(supported, 8);
    }
}
