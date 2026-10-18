use aa_anim::cdf::{AttachmentKind, BindingKind, BoneAlignment};
use aa_anim::{Cdf, CharacterDesc, Costume, GameDb, Gender, PartKind, Race};

#[test]
fn original_white_day_part_keeps_its_head_ring_after_character_resolution() {
    if !aa_cry::pak_available() {
        return;
    }
    let db = GameDb::open().unwrap();
    let path = "objects/characters/elf/female/costume/el_f_sk_whiteday001/el_f_sk_whiteday001.cdf";
    let cdf = Cdf::load(path).unwrap();
    let mut desc = CharacterDesc::default_for(&db, Race::Elf, Gender::Female).unwrap();
    desc.hair = None;
    desc.costume = Costume::Meshes(vec![path.into()]);
    let assets = desc.resolve(&db).unwrap();
    let part = assets
        .parts
        .iter()
        .find(|part| part.kind == PartKind::Costume)
        .unwrap();
    assert_eq!(part.path, cdf.model);
    assert_eq!(part.material, cdf.material);
    assert_eq!(part.attachments, cdf.attachments);
    let effect = &part.attachments[0];
    assert_eq!(effect.name, "headring");
    assert_eq!(effect.kind, AttachmentKind::Bone);
    assert_eq!(effect.binding_kind, BindingKind::Effect);
    assert_eq!(
        effect.binding,
        "X2_EFFECTS.doodad_effect.whiteday_custume_head_elf"
    );
    assert_eq!(effect.bone_name, "Bip01 Head");
    assert_eq!(effect.bone_alignment, BoneAlignment::Model);
    let skeleton = aa_cry::Skeleton::load(&assets.skeleton).unwrap();
    let (bone, local) = effect.bone_binding(&skeleton).unwrap();
    let authored = glam::Affine3A::from_rotation_translation(effect.rotation, effect.position);
    assert!((skeleton.bones[bone].bind_model * local).abs_diff_eq(authored, 1e-4));

    // Raw-mesh resolution retains the same model/material semantics without inventing
    // the attachments of a similarly named CDF that the caller did not supply.
    desc.costume = Costume::Meshes(vec![part.path.clone()]);
    let raw = desc.resolve(&db).unwrap();
    let raw = raw
        .parts
        .iter()
        .find(|part| part.kind == PartKind::Costume)
        .unwrap();
    assert_eq!(raw.path, part.path);
    assert!(raw.attachments.is_empty());
    assert!(
        assets
            .parts
            .iter()
            .filter(|part| part.kind == PartKind::Body)
            .all(|part| part.attachments.is_empty())
    );
}
