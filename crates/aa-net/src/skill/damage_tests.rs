//! Original suffix goldens plus full executable lane writers; no full native tail claim.
use super::*;
use crate::{game::ServerMsg, opcodes::sc};

#[path = "../../tests/fixtures/unit-damage/lane.rs"]
mod lane;
use lane::Marshal;

pub(crate) struct Fixture {
    pub body: Vec<u8>,
    pub expected: UnitDamage,
}

pub(crate) fn lane_fixtures() -> Vec<Fixture> {
    use lane::{CastAction as A, SkillCaster as C, SkillHitType as H};
    let actions = [
        (
            A::Skill {
                skill_id: 18132,
                tl_id: 3,
            },
            CastAction {
                kind: 0,
                skill_id: 18132,
                tl: 3,
                buff_id: 0,
            },
        ),
        (
            A::Plot {
                plot_id: 77,
                tl_id: 9,
                event_id: 88,
                skill_id: 16064,
            },
            CastAction {
                kind: 1,
                skill_id: 16064,
                tl: 9,
                buff_id: 0,
            },
        ),
        (
            A::Buff {
                buff_id: 555,
                owner_obj_id: 0xffffff,
                index: 4,
            },
            CastAction {
                kind: 2,
                skill_id: 0,
                tl: 0,
                buff_id: 555,
            },
        ),
        (
            A::BuffTarget,
            CastAction {
                kind: 3,
                ..Default::default()
            },
        ),
        (
            A::DestroyTarget,
            CastAction {
                kind: 4,
                ..Default::default()
            },
        ),
    ];
    let casters = [
        C::Unit(0x010203),
        C::Unk1(0xffffff),
        C::Item {
            obj_id: 0x123456,
            item_id: u64::MAX,
            item_template_id: 5660,
            type1: 255,
            type2: u32::MAX,
        },
        C::Mount {
            obj_id: 0xfedcba,
            mount_skill_template_id: u32::MAX,
        },
        C::Doodad(0xabcdef),
    ];
    let values = [0, 255, 256, 65535, 65536, 0xffffff, 0x1000000, i32::MAX];
    let hits = [
        0, 1, 3, 4, 5, 6, 7, 9, 10, 11, 13, 14, 15, 16, 17, 18, 19, 20,
    ];
    let mut out = Vec::new();
    for (ai, (action, expected_action)) in actions.iter().enumerate() {
        for (ci, caster) in casters.iter().enumerate() {
            for vi in 0..values.len() {
                let mut packet = lane::SCUnitDamagedPacket::new(
                    *action,
                    *caster,
                    0x102030,
                    0xffffff,
                    values[vi],
                    values[(vi + 3) % values.len()],
                );
                packet.mana_burn = values[(vi + 6) % values.len()];
                packet.holdable_id = [0, 3, 255][(ai + ci + vi) % 3];
                let hit =
                    hits[(ai * casters.len() * values.len() + ci * values.len() + vi) % hits.len()];
                packet.hit_type = H::from_raw(i64::from(hit)).unwrap();
                let mut stream = lane::PacketStream::new();
                packet.write(&mut stream);
                out.push(Fixture {
                    body: stream.into_bytes(),
                    expected: UnitDamage {
                        action: *expected_action,
                        caster: BuffCaster {
                            kind: caster.kind(),
                            obj_id: caster.obj_id(),
                        },
                        caster_obj_id: packet.caster_id,
                        target_obj_id: packet.target_id,
                        damage: packet.damage,
                        absorbed: packet.absorbed,
                        mana_burn: packet.mana_burn,
                        holdable_id: packet.holdable_id,
                        raw_hit: 0x0120 | u16::from(hit),
                        hit_type: hit,
                    },
                });
            }
        }
    }
    out
}

pub(crate) fn plain(body: &[u8]) -> Vec<u8> {
    let mut w = Writer::new();
    w.raw(&[0xdd, 1, 0, 0])
        .u16(sc::SCUnitDamagedPacket)
        .raw(body);
    w.finish()
}

fn stored_batch(entries: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let mut raw = Writer::new();
    for (opcode, body) in entries {
        raw.u16(0).u16(*opcode).raw(body);
    }
    let raw = raw.finish();
    let len = u16::try_from(raw.len()).unwrap();
    let mut w = Writer::new();
    w.raw(&[0xdd, 4])
        .u16(u16::try_from(entries.len()).unwrap())
        .u8(1)
        .u16(len)
        .u16(!len)
        .raw(&raw);
    w.finish()
}

pub(crate) fn mixed_batch() -> (Vec<u8>, Vec<UnitDamage>) {
    let mut entries = vec![(sc::SCOnOffSnowPacket, vec![0])];
    let mut expected = Vec::new();
    for (i, mut fixture) in lane_fixtures().into_iter().enumerate() {
        let packed: u16 = [0, 0x0123, 0x112b, 0x25a3, 0xffff, 0xa700][i % 6];
        let at = fixture.body.len() - 4;
        fixture.body[at..at + 2].copy_from_slice(&packed.to_le_bytes());
        fixture.expected.raw_hit = packed;
        fixture.expected.hit_type = (packed & 31) as u8;
        entries.push((sc::SCUnitDamagedPacket, fixture.body));
        expected.push(fixture.expected);
    }
    entries.push((sc::SCSkillEndedPacket, 0x9876u16.to_le_bytes().to_vec()));
    entries.push((sc::SCOnOffSnowPacket, vec![1]));
    (stored_batch(&entries), expected)
}

fn hex(s: &str) -> Vec<u8> {
    let (pairs, rest) = s.as_bytes().as_chunks::<2>();
    assert!(rest.is_empty());
    pairs
        .iter()
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

#[test]
fn unit_damage_actual_lane_writer_matches_fixed_full_body_golden() {
    let mut packet = lane::SCUnitDamagedPacket::new(
        lane::CastAction::Skill {
            skill_id: 2,
            tl_id: 10,
        },
        lane::SkillCaster::Unit(5),
        5,
        6,
        300,
        40,
    );
    packet.hit_type = lane::SkillHitType::MeleeCritical;
    let mut stream = lane::PacketStream::new();
    packet.write(&mut stream);
    // Same inputs as the lane's unit_damaged_layout, independently fixed bytes.
    let golden = hex("00020000000a000005000005000006000000012c012800000000000023010101");
    assert_eq!(stream.into_bytes(), golden);
    let mut r = Reader::new(&golden);
    let d = UnitDamage::read(&mut r).unwrap();
    assert!(r.is_empty());
    assert_eq!((d.damage, d.absorbed, d.mana_burn), (300, 40, 0));
    assert_eq!(
        (d.holdable_id, d.raw_hit, d.hit_type, d.pierce_selector()),
        (0, 0x0123, 3, 0)
    );
}

#[test]
fn unit_damage_lane_actions_casters_pisc_widths_and_all_prefixes() {
    let fixtures = lane_fixtures();
    assert_eq!(fixtures.len(), 200);
    for fixture in fixtures {
        let mut bytes = fixture.body.clone();
        let sentinel = [0, 0, 0xa7, 0, 0xfe, 0xff];
        bytes.extend(sentinel);
        let mut r = Reader::new(&bytes);
        assert_eq!(UnitDamage::read(&mut r).unwrap(), fixture.expected);
        assert_eq!(r.pos(), fixture.body.len());
        assert_eq!(r.rest(), sentinel);
        assert_eq!(
            ServerMsg::decode(&plain(&fixture.body)).unwrap(),
            ServerMsg::UnitDamaged(fixture.expected)
        );
        for end in 0..fixture.body.len() {
            assert!(
                UnitDamage::read(&mut Reader::new(&fixture.body[..end])).is_err(),
                "action {} prefix {end}",
                fixture.body[0]
            );
            assert!(
                ServerMsg::decode(&plain(&fixture.body[..end])).is_err(),
                "DD01 prefix {end}"
            );
        }
    }
}

#[test]
fn unit_damage_original_suffix_fields_survive_the_unchanged_lane_tail() {
    let template = &lane_fixtures()[0].body;
    let mut cases = 0;
    for line in include_str!("../../tests/fixtures/unit-damage/native-suffix.tsv")
        .lines()
        .filter(|line| !line.starts_with('#'))
    {
        let cells: Vec<_> = line.split('\t').collect();
        let suffix = hex(cells[7]);
        let mut body = template.clone();
        let at = body.len() - 5;
        // Only hol/de are inserted: lane flag1/result1 stay unchanged. Native
        // flag15 goldens do not establish parsing of its later optional tail.
        body[at..at + 3].copy_from_slice(&suffix[..3]);
        let mut r = Reader::new(&body);
        let d = UnitDamage::read(&mut r).unwrap();
        assert!(r.is_empty());
        assert_eq!(d.holdable_id, cells[0].parse::<u8>().unwrap());
        assert_eq!(d.raw_hit, u16::from_str_radix(cells[1], 16).unwrap());
        assert_eq!(d.hit_type, cells[3].parse::<u8>().unwrap());
        assert_eq!((d.raw_hit >> 5) & 7, cells[4].parse::<u16>().unwrap());
        assert_eq!((d.raw_hit >> 8) & 15, cells[5].parse::<u16>().unwrap());
        assert_eq!(d.pierce_selector(), cells[6].parse::<u8>().unwrap());
        cases += 1;
    }
    assert_eq!(cases, 63);
}

#[test]
fn unit_damage_preserves_every_packed_word_and_every_holdable_byte() {
    let mut body = lane_fixtures().remove(0).body;
    let at = body.len() - 5;
    body[at] = 255;
    for packed in u16::MIN..=u16::MAX {
        body[at + 1..at + 3].copy_from_slice(&packed.to_le_bytes());
        let mut r = Reader::new(&body);
        let d = UnitDamage::read(&mut r).unwrap();
        assert!(r.is_empty());
        assert_eq!(
            (d.holdable_id, d.raw_hit, d.hit_type, d.pierce_selector()),
            (255, packed, (packed & 31) as u8, (packed >> 12) as u8)
        );
    }
    for holdable in u8::MIN..=u8::MAX {
        body[at] = holdable;
        let mut r = Reader::new(&body);
        let d = UnitDamage::read(&mut r).unwrap();
        assert!(r.is_empty());
        assert_eq!(
            (d.holdable_id, d.raw_hit, d.hit_type, d.pierce_selector()),
            (holdable, 0xffff, 31, 15)
        );
    }
}

#[test]
fn unit_damage_keeps_generic_hit_helpers_independent_of_raw_upper_bits() {
    let mut body = lane_fixtures().remove(0).body;
    let at = body.len() - 4;
    for hit in 0..32u8 {
        for upper in [0, 0x0120, 0x2120, 0xffe0] {
            body[at..at + 2].copy_from_slice(&(upper | u16::from(hit)).to_le_bytes());
            let d = UnitDamage::read(&mut Reader::new(&body)).unwrap();
            assert_eq!(
                hit_type::is_critical(d.hit_type),
                matches!(hit, 3 | 11 | 15)
            );
            assert_eq!(
                hit_type::is_miss(d.hit_type),
                matches!(hit, 4 | 5 | 6 | 7 | 10 | 14 | 16 | 17 | 18 | 19 | 20)
            );
        }
    }
}

#[test]
fn unit_damage_dd04_keeps_all_contexts_and_following_packets_in_order() {
    let (frame, expected) = mixed_batch();
    let ServerMsg::Batch(messages) = ServerMsg::decode(&frame).unwrap() else {
        panic!("not DD04")
    };
    assert_eq!(messages.len(), expected.len() + 3);
    assert_eq!(messages[0], ServerMsg::Snow(false));
    for (message, damage) in messages[1..=expected.len()].iter().zip(&expected) {
        assert_eq!(message, &ServerMsg::UnitDamaged(damage.clone()));
    }
    assert_eq!(
        messages[expected.len() + 1],
        ServerMsg::SkillEnded { tl: 0x9876 }
    );
    assert_eq!(messages[expected.len() + 2], ServerMsg::Snow(true));
}
