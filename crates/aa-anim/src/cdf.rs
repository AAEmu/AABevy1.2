//! `.cdf` character definitions (XML): a base model plus attachments.
//!
//! ```xml
//! <CharacterDefinition>
//!  <Model File="objects/characters/elf/female/nude/el_f_base.chr" Material="....mtl"/>
//!  <AttachmentList customize="6">
//!   <Attachment AName="body" Type="CA_SKIN" Binding="objects/.../el_f_nude.chr" Flags="0"/>
//!   <Attachment AName="mainhand" Type="CA_BONE" BoneName="item_hand_r" Binding=""
//!               Rotation="w,x,y,z" Position="x,y,z" scale="0.9" Flags="1"/>
//!  </AttachmentList>
//!  <ShapeDeformation COL0="0" ... COL7="0"/>
//! </CharacterDefinition>
//! ```
//!
//! Three kinds of `.cdf` exist:
//!
//! * **base** (`nu_m.cdf`, `nu_m_base.cdf`): `Model` = the skeleton `.chr`, an attachment list with
//!   the named slots of a player (`body`, `face`, `hair`, `head`, `chest`, `waist`, `legs`,
//!   `hands`, `feet`, `arms`, `glasses`, `beard`, `cosplay` = `CA_SKIN`; `mainhand`, `offhand`,
//!   `ranged`, `back`, ... = `CA_BONE`). `Flags="1"` marks an empty / hidden slot;
//! * **part** (`el_f_ar_cloth001.cdf`, `el_f_face00.cdf`): only a `Model` (a skin `.chr`) and a
//!   `Material`; the part is attached to a base skeleton by bone name;
//! * `Attachment` elements of **part** files may also carry a `Material`.

use glam::{Affine3A, Quat, Vec3};

use crate::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttachmentKind {
    /// `CA_SKIN`: a skinned mesh sharing the base skeleton.
    Skin,
    /// `CA_BONE`: a rigid object on a bone.
    Bone,
    /// `CA_FACE`, `CA_PROX`, ...
    Other,
}

/// Native `AttachmentType` payload, separate from the CA_BONE/CA_SKIN attachment.
/// Values match the original descriptor at +0x44, not inferred file extensions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum BindingKind {
    #[default]
    Object = 1,
    Entity = 3,
    Effect = 5,
}

/// Original `AlignBoneAttachment` mode. `Bone` ignores XML Rotation/Position;
/// `Model` uses a model-space default transform. `Skin` needs surface projection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BoneAlignment {
    #[default]
    Model,
    Bone,
    Skin,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Attachment {
    pub name: String,
    pub kind: AttachmentKind,
    pub binding_kind: BindingKind,
    /// Object pak path, effect library identifier or entity binding, according to
    /// `binding_kind`. Non-object identifiers retain authored spelling/case.
    pub binding: String,
    pub material: Option<String>,
    /// Bone of a `CA_BONE` attachment.
    pub bone_name: String,
    pub bone_alignment: BoneAlignment,
    pub position: Vec3,
    pub rotation: Quat,
    pub scale: f32,
    pub flags: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cdf {
    /// The base model (`.chr` skeleton or skin part).
    pub model: String,
    pub material: Option<String>,
    pub attachments: Vec<Attachment>,
    /// `AttachmentList@customize` (a bit set of customisable slots; 6 / 7 in player cdfs).
    pub customize: Option<u32>,
    /// `ShapeDeformation@COL0..COL7`.
    pub shape_deformation: [f32; 8],
}

impl Cdf {
    pub fn parse(xml: &str) -> Result<Cdf> {
        let xml = xml.trim_start_matches('\u{feff}');
        let doc = roxmltree::Document::parse(xml).map_err(|e| Error::Xml(e.to_string()))?;
        let root = doc.root_element();
        if !root.has_tag_name("CharacterDefinition") {
            return Err(Error::Format(format!("cdf root is <{}>", root.tag_name().name())));
        }
        let child = |n: &str| root.children().find(|c| c.is_element() && c.has_tag_name(n));
        let model = child("Model").ok_or_else(|| Error::Format("cdf without <Model>".into()))?;
        let mut cdf = Cdf {
            model: norm(model.attribute("File").unwrap_or("")),
            material: model.attribute("Material").map(norm).filter(|m| !m.is_empty()),
            attachments: Vec::new(),
            customize: None,
            shape_deformation: [0.0; 8],
        };
        if let Some(list) = child("AttachmentList") {
            cdf.customize = list.attribute("customize").and_then(|v| v.parse().ok());
            for a in list.children().filter(|c| c.is_element() && c.has_tag_name("Attachment")) {
                let kind = match a.attribute("Type") {
                    Some("CA_SKIN") => AttachmentKind::Skin,
                    Some("CA_BONE") => AttachmentKind::Bone,
                    _ => AttachmentKind::Other,
                };
                let binding_kind = match a.attribute("AttachmentType") {
                    Some(value) if value.eq_ignore_ascii_case("effect") => BindingKind::Effect,
                    Some(value) if value.eq_ignore_ascii_case("entity") => BindingKind::Entity,
                    _ => BindingKind::Object,
                };
                let binding = a.attribute("Binding").unwrap_or("");
                let rotation = a
                    .attribute("Rotation")
                    .and_then(|v| floats::<4>(v))
                    .map(|[w, x, y, z]| Quat::from_xyzw(x, y, z, w).normalize())
                    .unwrap_or(Quat::IDENTITY);
                cdf.attachments.push(Attachment {
                    name: a.attribute("AName").unwrap_or("").to_string(),
                    kind,
                    binding_kind,
                    binding: if binding_kind == BindingKind::Object { norm(binding) } else { binding.to_string() },
                    material: a.attribute("Material").map(norm).filter(|m| !m.is_empty()),
                    bone_name: a.attribute("BoneName").unwrap_or("").to_string(),
                    bone_alignment: match a.attribute("AlignBoneAttachment") {
                        Some(mode) if mode.eq_ignore_ascii_case("Bone") => BoneAlignment::Bone,
                        Some(mode) if mode.eq_ignore_ascii_case("Skin") => BoneAlignment::Skin,
                        _ => BoneAlignment::Model,
                    },
                    position: a.attribute("Position").and_then(floats::<3>).map(Vec3::from).unwrap_or(Vec3::ZERO),
                    rotation,
                    scale: a.attribute("scale").or(a.attribute("Scale")).and_then(|v| v.trim().parse().ok()).unwrap_or(1.0),
                    flags: a.attribute("Flags").and_then(|v| v.parse().ok()).unwrap_or(0),
                });
            }
        }
        if let Some(s) = child("ShapeDeformation") {
            for (i, v) in cdf.shape_deformation.iter_mut().enumerate() {
                *v = s.attribute(format!("COL{i}").as_str()).and_then(|v| v.parse().ok()).unwrap_or(0.0);
            }
        }
        Ok(cdf)
    }

    /// Loads a `.cdf` from the global pak.
    pub fn load(path: &str) -> Result<Cdf> {
        let bytes = aa_cry::read(path)?;
        Cdf::parse(&String::from_utf8_lossy(&bytes))
    }

    /// The attachment slot with this name (case-insensitive).
    pub fn attachment(&self, name: &str) -> Option<&Attachment> {
        self.attachments.iter().find(|a| a.name.eq_ignore_ascii_case(name))
    }

    /// Bind an explicitly supplied body-part item to its already-declared skin slot.
    /// Use server-supplied item IDs, not inferred CDF filename/default-body substitutions.
    /// Returns false for missing/non-skin slots; never invents attachments. The caller
    /// selects a compatible model and handles custom textures/colours separately.
    pub fn bind_body_part(&mut self, part: &crate::db::BodyPart) -> bool {
        let Some(slot) = part.attachment_slot() else { return false };
        let Some(attachment) = self.attachments.iter_mut().find(|a| a.kind == AttachmentKind::Skin && a.name.eq_ignore_ascii_case(slot)) else { return false };
        attachment.binding = part.path.clone();
        attachment.binding_kind = BindingKind::Object;
        attachment.material = None;
        // This is the effective visible binding, replacing an empty/hidden slot.
        attachment.flags &= !1;
        true
    }
}

impl Attachment {
    /// Joint index and Cry-space local transform for parenting a rigid attachment to
    /// an existing animated joint. The model-root already performs Cry-to-Bevy conversion.
    /// Callers retain visibility/empty-slot filtering and load the binding/material normally.
    pub fn bone_binding(&self, skeleton: &aa_cry::Skeleton) -> Result<(usize, Affine3A)> {
        if self.kind != AttachmentKind::Bone {
            return Err(Error::Format("not a CA_BONE attachment".into()));
        }
        let bone = skeleton.bones.iter().position(|bone| bone.name.eq_ignore_ascii_case(&self.bone_name))
            .ok_or_else(|| Error::Missing(format!("attachment bone {}", self.bone_name)))?;
        let local = match self.bone_alignment {
            BoneAlignment::Bone => Affine3A::from_scale(Vec3::splat(self.scale)),
            BoneAlignment::Model => skeleton.bones[bone].bind_model.inverse()
                * Affine3A::from_rotation_translation(self.rotation, self.position),
            BoneAlignment::Skin => return Err(Error::Format("skin-aligned bone attachment projection is unsupported".into())),
        };
        Ok((bone, local))
    }
}

fn floats<const N: usize>(s: &str) -> Option<[f32; N]> {
    let mut out = [0f32; N];
    let mut it = s.split(',');
    for v in &mut out {
        *v = it.next()?.trim().parse().ok()?;
    }
    Some(out)
}

/// Lowercase, `/` separators, without a leading `game/`.
pub fn norm(p: &str) -> String {
    let p = p.trim().replace('\\', "/").to_ascii_lowercase();
    p.trim_start_matches("./").trim_start_matches('/').trim_start_matches("game/")
        .split('/').filter(|part| !part.is_empty()).collect::<Vec<_>>().join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = r#"<CharacterDefinition>
 <Model File="objects/characters/elf/female/nude/el_f_base.chr"/>
 <AttachmentList customize="6">
  <Attachment AName="body" Type="CA_SKIN" Binding="objects/characters/elf/female/nude/el_f_nude.chr" Flags="0"/>
  <Attachment AName="chest" Type="CA_SKIN" Binding="" Flags="1"/>
  <Attachment AName="mainhand" Type="CA_BONE" Rotation="0.98238242,-0.07097321,-0.17119513,-0.024078663" Position="0.4,0.006,1.02" AlignBoneAttachment="Bone" BoneName="item_hand_r" Flags="1" scale="0.9"/>
 </AttachmentList>
 <ShapeDeformation COL0="0" COL1="0.5" COL2="0" COL3="0" COL4="0" COL5="0" COL6="0" COL7="0"/>
</CharacterDefinition>"#;

    #[test]
    fn binding_payload_uses_native_explicit_type_and_preserves_identifiers() {
        let cdf = Cdf::parse(r#"<CharacterDefinition><Model File="body.chr"/><AttachmentList>
            <Attachment AName="object" Type="CA_BONE" Binding="GAME\Objects\Mesh.CGF"/>
            <Attachment AName="unknown" Type="CA_BONE" AttachmentType="other" Binding="GAME\Objects\Mesh.CGF"/>
            <Attachment AName="effect" Type="CA_BONE" AttachmentType="EfFeCt" Binding="Library.Group.MixedCase"/>
            <Attachment AName="entity" Type="CA_BONE" AttachmentType="EnTiTy" Binding="AuthoredEntityName"/>
            </AttachmentList></CharacterDefinition>"#).unwrap();
        for name in ["object", "unknown"] {
            let attachment = cdf.attachment(name).unwrap();
            assert_eq!(attachment.binding_kind as u32, 1);
            assert_eq!(attachment.binding, "objects/mesh.cgf");
        }
        let effect = cdf.attachment("effect").unwrap();
        assert_eq!(effect.binding_kind as u32, 5);
        assert_eq!(effect.binding, "Library.Group.MixedCase");
        let entity = cdf.attachment("entity").unwrap();
        assert_eq!(entity.binding_kind as u32, 3);
        assert_eq!(entity.binding, "AuthoredEntityName");
    }

    #[test]
    fn original_horse_and_costume_effects_keep_native_bone_transforms() {
        if !aa_cry::pak_available() { return; }
        let horse = Cdf::load("objects/characters/animals/horse/horse_lord.cdf").unwrap();
        let skeleton = aa_cry::Skeleton::load(&horse.model).unwrap();
        for (slot, suffix) in [("effect_foot_lf", "LF"), ("effect_foot_lr", "LR"), ("effect_foot_rf", "RF"), ("effect_foot_rr", "RR")] {
            let effect = horse.attachment(slot).unwrap();
            assert_eq!(effect.kind, AttachmentKind::Bone);
            assert_eq!(effect.binding_kind, BindingKind::Effect);
            assert_eq!(effect.binding, format!("abillity_skill_table_m.system_skill.lord_horse_foot_{suffix}"));
            assert_eq!(effect.bone_alignment, BoneAlignment::Model);
            let (bone, local) = effect.bone_binding(&skeleton).unwrap();
            assert_eq!(skeleton.bones[bone].name, effect.bone_name);
            let authored = Affine3A::from_rotation_translation(effect.rotation, effect.position);
            assert!((skeleton.bones[bone].bind_model * local).abs_diff_eq(authored, 1e-4));
        }
        let costume = Cdf::load("objects/characters/elf/female/costume/el_f_sk_whiteday001/el_f_sk_whiteday001.cdf").unwrap();
        let headring = costume.attachment("headring").unwrap();
        assert_eq!(headring.binding_kind, BindingKind::Effect);
        assert_eq!(headring.binding, "X2_EFFECTS.doodad_effect.whiteday_custume_head_elf");
        let skeleton = aa_cry::Skeleton::load("objects/characters/elf/female/nude/el_f_base.chr").unwrap();
        let (bone, local) = headring.bone_binding(&skeleton).unwrap();
        assert_eq!(skeleton.bones[bone].name, "Bip01 Head");
        assert!((skeleton.bones[bone].bind_model * local).abs_diff_eq(Affine3A::from_rotation_translation(headring.rotation, headring.position), 1e-4));
    }

    #[test]
    fn parses_base_cdf() {
        let c = Cdf::parse(BASE).unwrap();
        assert_eq!(c.model, "objects/characters/elf/female/nude/el_f_base.chr");
        assert_eq!(c.customize, Some(6));
        assert_eq!(c.attachments.len(), 3);
        let body = c.attachment("BODY").unwrap();
        assert_eq!(body.kind, AttachmentKind::Skin);
        assert_eq!(body.binding, "objects/characters/elf/female/nude/el_f_nude.chr");
        assert!(c.attachment("chest").unwrap().binding.is_empty());
        let hand = c.attachment("mainhand").unwrap();
        assert_eq!(hand.kind, AttachmentKind::Bone);
        assert_eq!(hand.bone_name, "item_hand_r");
        assert!((hand.rotation.w - 0.98238).abs() < 1e-3);
        assert_eq!(hand.scale, 0.9);
        assert_eq!(c.shape_deformation[1], 0.5);
    }

    #[test]
    fn parses_part_cdf() {
        let c = Cdf::parse(
            "<CharacterDefinition>\n<Model File=\"objects/Characters/elf/female/costume/a.chr\"\nMaterial=\"objects/Characters/elf/female/costume/a.mtl\"/>\n</CharacterDefinition>",
        )
        .unwrap();
        assert_eq!(c.model, "objects/characters/elf/female/costume/a.chr");
        assert_eq!(c.material.as_deref(), Some("objects/characters/elf/female/costume/a.mtl"));
        assert!(c.attachments.is_empty());
    }

    #[test]
    fn rejects_other_xml() {
        assert!(Cdf::parse("<Material/>").is_err());
        assert!(Cdf::parse("not xml").is_err());
    }

    #[test]
    fn original_chef_eyes_bind_to_joints_and_normalize_authored_double_slash() {
        if !aa_cry::pak_available() { return; }
        let cdf = Cdf::load("objects/characters/people/npc_chef01/nude/npc_chef01_base.cdf").unwrap();
        let skeleton = aa_cry::Skeleton::load(&cdf.model).unwrap();
        for name in ["eye_left", "eye_right"] {
            let eye = cdf.attachment(name).unwrap();
            assert_eq!(eye.bone_alignment, BoneAlignment::Bone);
            assert!(aa_pak::Pak::global().exists(&eye.binding), "{}", eye.binding);
            assert!(!eye.binding.contains("//"));
            let (bone, local) = eye.bone_binding(&skeleton).unwrap();
            assert_eq!(skeleton.bones[bone].name, eye.bone_name);
            assert!(local.abs_diff_eq(Affine3A::IDENTITY, 1e-6));
            // Native Bone alignment ignores the serialized model-space offset entirely.
            let mut changed = eye.clone();
            changed.position = Vec3::splat(100.0);
            changed.rotation = Quat::from_rotation_z(1.0);
            assert_eq!(changed.bone_binding(&skeleton).unwrap(), (bone, local));
            changed.scale = 0.5;
            assert!(changed.bone_binding(&skeleton).unwrap().1.abs_diff_eq(Affine3A::from_scale(Vec3::splat(0.5)), 1e-6));
            // Without alignment, the same XML transform is absolute in character space.
            changed.bone_alignment = BoneAlignment::Model;
            let desired = Affine3A::from_rotation_translation(changed.rotation, changed.position);
            let (bone, local) = changed.bone_binding(&skeleton).unwrap();
            assert!((skeleton.bones[bone].bind_model * local).abs_diff_eq(desired, 1e-4));
            changed.bone_alignment = BoneAlignment::Skin;
            assert!(changed.bone_binding(&skeleton).is_err());
            changed.bone_name = "not_a_joint".into();
            assert!(changed.bone_binding(&skeleton).is_err());
        }
    }

    #[test]
    fn original_explicit_dwarf_beard_and_missing_ferre_beard_inventory() {
        if !aa_cry::pak_available() { return; }
        let db = crate::GameDb::open().unwrap();
        let beard = db.body_part(392).unwrap();
        assert_eq!((beard.slot_type, beard.model_id, beard.asset_ids), (29, 14, [180, 0, 0, 181, 0]));
        let original = Cdf::load("objects/characters/dwarf/male/nude/dw_m_base.cdf").unwrap();
        let mut received = original.clone();
        assert!(received.bind_body_part(&beard));
        let bound = received.attachment("beard").unwrap();
        assert_eq!(bound.kind, AttachmentKind::Skin);
        assert_eq!(bound.binding, beard.path);
        assert_eq!(bound.flags & 1, 0);
        assert!(aa_pak::Pak::global().exists(&bound.binding));
        for (before, after) in original.attachments.iter().zip(&received.attachments) {
            if before.name != "beard" { assert_eq!(before, after); }
        }
        let skeleton = aa_cry::Skeleton::load(&original.model).unwrap();
        assert!(!skeleton.bones.is_empty());
        for variant in [crate::hair::HairVariant::Default, crate::hair::HairVariant::Type1,
            crate::hair::HairVariant::Type2, crate::hair::HairVariant::Type4] {
            assert_eq!(beard.variant_path(variant), Some(beard.path.as_str()));
        }
        assert_eq!(beard.variant_path(crate::hair::HairVariant::Hidden), None);
        assert_eq!(beard.variant_path(crate::hair::HairVariant::Type3), Some(db.asset_path(181).unwrap().as_str()));
        assert!(!aa_pak::Pak::global().exists(beard.variant_path(crate::hair::HairVariant::Type3).unwrap()));
        // These DB rows do not establish availability in this original 1.2 client.
        for (item, model, asset) in [(564, 20, 229), (584, 40, 246)] {
            let missing = db.body_part(item).unwrap();
            assert_eq!((missing.slot_type, missing.model_id, missing.asset_ids[0]), (29, model, asset));
            assert!(!aa_pak::Pak::global().exists(&missing.path), "{}", missing.path);
        }
    }

    #[test]
    fn original_blank_actor_cdf_accepts_explicit_packet_body_items_only() {
        if !aa_cry::pak_available() { return; }
        let db = crate::GameDb::open().unwrap();
        let original = Cdf::load("objects/characters/people/npc_chef01/nude/npc_chef01.cdf").unwrap();
        let mut bound = original.clone();
        for (item, slot) in [(14478, "face"), (14475, "hair"), (14479, "body")] {
            let body = db.body_part(item).unwrap();
            assert_eq!(body.model_id, 631);
            assert_eq!(body.attachment_slot(), Some(slot));
            assert!(original.attachment(slot).unwrap().binding.is_empty());
            assert!(bound.bind_body_part(&body));
            let attachment = bound.attachment(slot).unwrap();
            assert_eq!(attachment.binding, body.path);
            assert_eq!(attachment.flags & 1, 0);
            assert!(aa_pak::Pak::global().exists(&attachment.binding));
        }
        assert_eq!(bound.model, original.model);
        assert_eq!(bound.customize, original.customize);
        for name in ["eye_left", "eye_right", "head", "mainhand"] {
            assert_eq!(bound.attachment(name), original.attachment(name));
        }
        let mut invalid = db.body_part(14478).unwrap();
        invalid.slot_type = 0;
        let before = bound.clone();
        assert!(!bound.bind_body_part(&invalid));
        invalid.slot_type = 29; // this actor CDF does not declare a beard slot
        assert!(!bound.bind_body_part(&invalid));
        assert_eq!(bound, before);
        let mut slots = db.body_part(14478).unwrap();
        let names = ["face", "hair", "glasses", "reserved", "tail", "body", "beard"];
        for (i, name) in names.into_iter().enumerate() {
            slots.slot_type = 23 + i as u32;
            assert_eq!(slots.attachment_slot(), Some(name));
        }
    }
}
