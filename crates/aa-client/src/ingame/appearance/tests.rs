use super::*;
use aa_anim::{Cdf, PartKind, hair::HairVariant};
use aa_net::model::EquipDetail;

fn item(template_id: u32) -> Item {
    Item {
        id: u64::from(template_id),
        template_id,
        count: 1,
        ..default()
    }
}
fn equipment(head: Option<u32>) -> Equipment {
    let mut equipment = Equipment::default();
    equipment.slots[slot::HAIR] = Some(item(24024));
    equipment.slots[slot::HEAD] = head.map(item);
    equipment
}
fn description(db: &GameDb, equipment: &Equipment) -> CharacterDesc {
    describe(db, Race::Nuian, Gender::Female, equipment, &ModelParams::default()).unwrap()
}
fn hair(assets: &CharacterAssets) -> Option<&aa_anim::PartAsset> {
    assets.parts.iter().find(|part| part.kind == PartKind::Hair)
}
fn model(path: &str) -> aa_anim::Result<String> {
    if path.ends_with(".cdf") {
        Ok(Cdf::load(path)?.model)
    } else {
        Ok(path.to_owned())
    }
}

#[test]
fn received_headgear_uses_authored_alternate_or_hidden_hair_without_changing_other_parts() -> Result<(), Box<dyn std::error::Error>> {
    let db = GameDb::open()?;
    let style = db.body_part(24024)?;
    let expected_alternate = model(style.variant_path(HairVariant::Type3).unwrap())?;
    for (head, detail) in [(None, None), (Some(22874), Some(28744)), (Some(18230), Some(25768))] {
        let equipment = equipment(head);
        let desc = description(&db, &equipment);
        let ordinary = desc.resolve(&db)?;
        let applied = resolve(&db, &desc, &equipment)?;
        assert_eq!(headgear_detail(&db, &desc, &equipment), detail);
        match head {
            Some(22874) => assert_eq!(hair(&applied).unwrap().path, expected_alternate),
            Some(18230) => assert!(hair(&applied).is_none()),
            _ => assert_eq!(hair(&applied).unwrap().path, hair(&ordinary).unwrap().path),
        }
        if let Some(selected) = hair(&applied) {
            assert_eq!(
                selected.material,
                hair(&ordinary).unwrap().material,
                "retain actual style/color material"
            );
        }
        let other = |assets: &CharacterAssets| {
            assets
                .parts
                .iter()
                .filter(|p| p.kind != PartKind::Hair)
                .map(|p| (p.kind, p.slot.clone(), p.path.clone(), p.material.clone(), p.dye))
                .collect::<Vec<_>>()
        };
        assert_eq!(other(&applied), other(&ordinary));
    }
    Ok(())
}

#[test]
fn received_image_override_and_fallback_use_the_same_visual_as_the_rendered_helmet() -> Result<(), Box<dyn std::error::Error>> {
    let db = GameDb::open()?;
    let mut equipment = equipment(Some(18230));
    equipment.slots[slot::HEAD].as_mut().unwrap().detail = ItemDetail::Equip(EquipDetail {
        image_item_template_id: 22874,
        ..default()
    });
    let desc = description(&db, &equipment);
    assert!(matches!(&desc.costume, Costume::Items(ids) if ids == &[22874]));
    assert_eq!(headgear_detail(&db, &desc, &equipment), Some(28744));
    assert!(hair(&resolve(&db, &desc, &equipment)?).is_some());
    equipment.slots[slot::HEAD].as_mut().unwrap().detail = ItemDetail::Equip(EquipDetail {
        image_item_template_id: u32::MAX,
        ..default()
    });
    let desc = description(&db, &equipment);
    assert!(matches!(&desc.costume, Costume::Items(ids) if ids == &[18230]));
    assert_eq!(headgear_detail(&db, &desc, &equipment), Some(25768));
    assert!(hair(&resolve(&db, &desc, &equipment)?).is_none());
    Ok(())
}

#[test]
fn rendered_cosplay_default_precedes_hidden_head_and_nonrendered_equipment_does_not_hide_hair() -> Result<(), Box<dyn std::error::Error>> {
    let db = GameDb::open()?;
    let mut equipment = equipment(Some(18230));
    equipment.slots[slot::COSPLAY] = Some(item(14359));
    let mut desc = description(&db, &equipment);
    assert_eq!(headgear_detail(&db, &desc, &equipment), Some(57344));
    let normal = db.body_part(24024)?;
    assert_eq!(hair(&resolve(&db, &desc, &equipment)?).unwrap().path, model(&normal.path)?);
    equipment.slots[slot::COSPLAY] = None;
    desc.costume = Costume::None;
    assert_eq!(headgear_detail(&db, &desc, &equipment), None);
    assert!(hair(&resolve(&db, &desc, &equipment)?).is_some());
    desc.hair = None;
    desc.hair_color = None;
    assert!(hair(&resolve(&db, &desc, &equipment)?).is_none());
    Ok(())
}

#[test]
fn a_hair_style_without_textures_in_this_client_falls_back_to_the_preset_style() -> Result<(), Box<dyn std::error::Error>> {
    let db = GameDb::open()?;
    let base = CharacterDesc::default_for(&db, Race::Nuian, Gender::Male)?;
    // nu_m_hair029 (npc only, beauty shop): meshes and materials in the 1.2 pak, no textures
    let mut equipment = Equipment::default();
    equipment.slots[slot::HAIR] = Some(item(34107));
    let desc = describe(&db, Race::Nuian, Gender::Male, &equipment, &ModelParams::default()).unwrap();
    assert_eq!(desc.hair, base.hair);
    // a style the client has keeps it
    equipment.slots[slot::HAIR] = Some(item(24129));
    let desc = describe(&db, Race::Nuian, Gender::Male, &equipment, &ModelParams::default()).unwrap();
    assert_eq!(desc.hair, Some(24129));
    Ok(())
}

#[test]
fn a_received_costume_replaces_the_body_and_the_armour_it_covers() -> Result<(), Box<dyn std::error::Error>> {
    let db = GameDb::open()?;
    // the starter set + cap under the butler / maid costume (wire slot 27, tagged like any item)
    let mut equipment = equipment(Some(6252));
    for (s, id) in [(slot::CHEST, 23387), (slot::LEGS, 23388), (slot::FEET, 23390), (slot::COSPLAY, 28439)] {
        equipment.slots[s] = Some(item(id));
    }
    let desc = description(&db, &equipment);
    let assets = resolve(&db, &desc, &equipment)?;
    let costume: Vec<_> = assets.parts.iter().filter(|p| p.kind == PartKind::Costume).collect();
    assert!(assets.parts.iter().all(|p| p.kind != PartKind::Body), "the nude body stays under the costume");
    assert_eq!(costume.len(), 1, "{costume:?}");
    assert_eq!((costume[0].slot.as_str(), costume[0].equip_slot), ("cosplay", Some(27)));
    // without the costume the armour is back, each piece in its slot, on the body
    equipment.slots[slot::COSPLAY] = None;
    let desc = description(&db, &equipment);
    let assets = resolve(&db, &desc, &equipment)?;
    let slots: Vec<_> = assets.parts.iter().filter(|p| p.kind == PartKind::Costume).map(|p| p.equip_slot).collect();
    assert_eq!(slots, [Some(4), Some(2), Some(6), Some(0)]);
    assert!(assets.parts.iter().any(|p| p.kind == PartKind::Body));
    Ok(())
}

#[test]
fn received_dye_overrides_the_default_on_rendered_parts_and_unknown_dye_fails_preparation() -> Result<(), Box<dyn std::error::Error>> {
    let db = GameDb::open()?;
    let mut equipment = equipment(None);
    equipment.slots[slot::COSPLAY] = Some(item(14359));
    let desc = description(&db, &equipment);
    let ordinary = resolve(&db, &desc, &equipment)?;
    let colors = |assets: &CharacterAssets| assets.parts.iter().filter(|p| p.equip_slot == Some(27)).map(|p| p.dye).collect::<Vec<_>>();
    assert_eq!(colors(&ordinary), [db.default_dye(14359)]);
    for (dye, expected) in [(29382, [54,2,1]), (29383, [5,0,6])] {
        equipment.slots[slot::COSPLAY].as_mut().unwrap().detail = ItemDetail::Equip(EquipDetail { dye_item_id: dye, ..default() });
        let dyed = resolve(&db, &desc, &equipment)?;
        assert_eq!(colors(&dyed), [Some(expected)]);
        assert_eq!(dyed.parts.iter().map(|p| &p.path).collect::<Vec<_>>(), ordinary.parts.iter().map(|p| &p.path).collect::<Vec<_>>());
    }
    equipment.slots[slot::COSPLAY].as_mut().unwrap().detail = ItemDetail::Equip(EquipDetail { dye_item_id: u32::MAX, ..default() });
    assert!(resolve(&db, &desc, &equipment).is_err(), "an unknown received color cannot be acknowledged as a default-colored refresh");
    equipment.slots[slot::COSPLAY].as_mut().unwrap().detail = ItemDetail::Equip(EquipDetail::default());
    assert_eq!(colors(&resolve(&db, &desc, &equipment)?), colors(&ordinary));
    Ok(())
}
