use super::*;
use crate::{game::ClientMsg, item_effect::CapturedBagItem, model::Item, wire::Reader};

#[path = "../../tests/fixtures/music/lane-skill-object.rs"]
mod lane_object;

pub(crate) const ACTOR: u32 = 0x010203;
pub(crate) const UID: u64 = 0xfedcba9876543210;

pub(crate) fn source(slot: u8) -> CapturedBagItem {
    CapturedBagItem {
        slot,
        item: Item {
            id: UID,
            template_id: 27902,
            count: 1,
            grade: 6,
            made_unit_id: 0x11223344,
            detail_payload: Some(vec![0, 255, 0]),
            ..Default::default()
        },
    }
}

pub(crate) fn fixture(actor: u32, grade: u8) -> Vec<u8> {
    let line = include_str!("../../tests/fixtures/music/native-compose.tsv")
        .lines()
        .find(|line| {
            let fields: Vec<_> = line.split('\t').collect();
            u32::from_str_radix(fields[0], 16).unwrap() == actor
                && fields[4].parse::<u8>().unwrap() == grade
        })
        .unwrap();
    let hex = line.split('\t').nth(5).unwrap();
    (0..hex.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&hex[at..at + 2], 16).unwrap())
        .collect()
}

#[test]
fn compose_writer_matches_original_full_item_caster_goldens_and_lane_object_reader() {
    for line in include_str!("../../tests/fixtures/music/native-compose.tsv").lines() {
        let fields: Vec<_> = line.split('\t').collect();
        let actor = u32::from_str_radix(fields[0], 16).unwrap();
        let mut captured = source(9);
        captured.item.id = u64::from_str_radix(fields[1], 16).unwrap();
        captured.item.grade = fields[2].parse().unwrap();
        captured.item.made_unit_id = u32::from_str_radix(fields[3], 16).unwrap();
        let grade = fields[4].parse().unwrap();
        let request = MusicComposeRequest::new(captured.clone(), grade).unwrap();
        assert_eq!(request.source(), &captured);
        assert_eq!(request.grade(), grade);
        assert_eq!(request.skill_id(), 22215);
        let mut writer = Writer::new();
        request.write(&mut writer, actor);
        let bytes = writer.finish();
        assert_eq!(bytes, fixture(actor, grade));
        let packet = ClientMsg::ComposeMusic {
            self_obj_id: actor,
            request,
        };
        assert_eq!(packet.opcode(), 0x52);
        assert_eq!(packet.encode_body(), bytes);

        let mut r = Reader::new(&bytes);
        assert_eq!(r.u32().unwrap(), 22215);
        assert_eq!(
            crate::skill::SkillCasterData::read(&mut r).unwrap(),
            crate::skill::SkillCasterData::Item {
                obj_id: actor,
                item_id: captured.item.id,
                template_id: 27902,
                type1: captured.item.grade,
                type2: captured.item.made_unit_id,
            }
        );
        assert_eq!(
            crate::skill::SkillTargetData::read(&mut r).unwrap(),
            crate::skill::SkillTargetData::Unit(actor)
        );
        let mut lane = super::tests::lane::PacketStream::from_bytes(r.rest().to_vec());
        let flag = lane.read_u8().unwrap();
        assert_eq!(flag, 5);
        assert_eq!(
            lane_object::SkillObject::read(flag, &mut lane).unwrap(),
            lane_object::SkillObject::Unk5(i32::from(grade))
        );
        assert_eq!(lane.remaining(), 0);
        for end in 0..4 {
            let mut lane = super::tests::lane::PacketStream::from_bytes(vec![0; end]);
            assert!(lane_object::SkillObject::read(5, &mut lane).is_err());
        }
    }
}

#[test]
fn compose_constructor_bounds_only_the_verified_music_paper_and_limit_steps() {
    for grade in 0..=10 {
        assert!(MusicComposeRequest::new(source(255), grade).is_ok());
    }
    for grade in [11, 127, 255] {
        assert!(MusicComposeRequest::new(source(0), grade).is_err());
    }
    let mut invalid = source(0);
    invalid.item.id = 0;
    assert!(MusicComposeRequest::new(invalid, 0).is_err());
    for template in [0, 28051, 27901] {
        let mut invalid = source(0);
        invalid.item.template_id = template;
        assert!(MusicComposeRequest::new(invalid, 0).is_err());
    }
    // Equality includes the complete captured record and composition grade.
    let original = MusicComposeRequest::new(source(0), 0).unwrap();
    let mut changed = source(0);
    changed.item.detail_payload.as_mut().unwrap().push(1);
    assert_ne!(original, MusicComposeRequest::new(changed, 0).unwrap());
    assert_ne!(original, MusicComposeRequest::new(source(1), 0).unwrap());
    assert_ne!(original, MusicComposeRequest::new(source(0), 1).unwrap());
}
