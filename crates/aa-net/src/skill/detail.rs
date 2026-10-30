//! Complete received skill identities. Legacy projections intentionally keep their old shapes.
use super::SkillTarget;
use crate::{
    buff::BuffCaster,
    wire::{Pos, Reader, Result, WireError, long_to_coord},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkillCasterData {
    Unit(u32),
    Unk1(u32),
    Item {
        obj_id: u32,
        item_id: u64,
        template_id: u32,
        type1: u8,
        type2: u32,
    },
    Mount {
        obj_id: u32,
        mount_skill_id: u32,
    },
    Doodad(u32),
}
impl SkillCasterData {
    pub fn read(r: &mut Reader) -> Result<Self> {
        let kind = r.u8()?;
        if kind > 4 {
            return Err(WireError::Invalid(format!("skill caster kind {kind}")));
        }
        let obj_id = r.bc()?;
        Ok(match kind {
            0 => Self::Unit(obj_id),
            1 => Self::Unk1(obj_id),
            2 => Self::Item {
                obj_id,
                item_id: r.u64()?,
                template_id: r.u32()?,
                type1: r.u8()?,
                type2: r.u32()?,
            },
            3 => Self::Mount {
                obj_id,
                mount_skill_id: r.u32()?,
            },
            4 => Self::Doodad(obj_id),
            _ => return Err(WireError::Invalid(format!("skill caster kind {kind}"))),
        })
    }
    pub fn projection(&self) -> BuffCaster {
        let (kind, obj_id) = match *self {
            Self::Unit(id) => (0, id),
            Self::Unk1(id) => (1, id),
            Self::Item { obj_id, .. } => (2, obj_id),
            Self::Mount { obj_id, .. } => (3, obj_id),
            Self::Doodad(id) => (4, id),
        };
        BuffCaster { kind, obj_id }
    }
    /// Original39391da0: type2 and mount_skill_id do not participate in identity.
    pub fn native_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Unit(a), Self::Unit(b)) | (Self::Unk1(a), Self::Unk1(b)) | (Self::Doodad(a), Self::Doodad(b)) => a == b,
            (Self::Mount { obj_id: a, .. }, Self::Mount { obj_id: b, .. }) => a == b,
            (
                Self::Item {
                    obj_id: a,
                    item_id: ai,
                    template_id: at,
                    type1: af,
                    ..
                },
                Self::Item {
                    obj_id: b,
                    item_id: bi,
                    template_id: bt,
                    type1: bf,
                    ..
                },
            ) => a == b && ai == bi && at == bt && af == bf,
            _ => false,
        }
    }
}

/// Native position fields retain all fixed-point bits, rather than round-trip through f32.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkillPosition {
    pub x: i64,
    pub y: i64,
    pub z: f32,
}
impl SkillPosition {
    fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            x: r.i64()?,
            y: r.i64()?,
            z: r.f32()?,
        })
    }
    pub fn projection(&self) -> Pos {
        Pos::new(long_to_coord(self.x), long_to_coord(self.y), self.z)
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SkillTargetData {
    Unit(u32),
    Position {
        position: SkillPosition,
        rotation: f32,
        obj_id1: u32,
        obj_id2: u32,
    },
    Position2 {
        position: SkillPosition,
        end: [f32; 3],
        normal: [f32; 3],
    },
    Item {
        obj_id: u32,
        item_id: u64,
        type1: u32,
        type2: u8,
    },
    Doodad(u32),
    Position3 {
        position: SkillPosition,
        pitch: f32,
    },
}
impl SkillTargetData {
    pub fn read(r: &mut Reader) -> Result<Self> {
        Ok(match r.u8()? {
            0 => Self::Unit(r.bc()?),
            1 => Self::Position {
                position: SkillPosition::read(r)?,
                rotation: r.f32()?,
                obj_id1: r.bc()?,
                obj_id2: r.bc()?,
            },
            2 => Self::Position2 {
                position: SkillPosition::read(r)?,
                end: [r.f32()?, r.f32()?, r.f32()?],
                normal: [r.f32()?, r.f32()?, r.f32()?],
            },
            3 => Self::Item {
                obj_id: r.bc()?,
                item_id: r.u64()?,
                type1: r.u32()?,
                type2: r.u8()?,
            },
            4 => Self::Doodad(r.bc()?),
            5 => Self::Position3 {
                position: SkillPosition::read(r)?,
                pitch: r.f32()?,
            },
            kind => return Err(WireError::Invalid(format!("skill target kind {kind}"))),
        })
    }
    pub fn projection(&self) -> SkillTarget {
        let (kind, obj_id, pos, rot) = match *self {
            Self::Unit(id) => (0, id, None, 0.0),
            Self::Position {
                position, rotation, ..
            } => (1, 0, Some(position.projection()), rotation),
            Self::Position2 { position, .. } => (2, 0, Some(position.projection()), 0.0),
            Self::Item { obj_id, .. } => (3, obj_id, None, 0.0),
            Self::Doodad(id) => (4, id, None, 0.0),
            Self::Position3 { position, .. } => (5, 0, Some(position.projection()), 0.0),
        };
        SkillTarget {
            kind,
            obj_id,
            pos,
            rot,
        }
    }
    /// Original39391e30. Position rotation is ignored; all compared floats reject NaN.
    /// This compares identities only, not retained-timeline ownership or planner admission.
    pub fn native_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Unit(a), Self::Unit(b)) | (Self::Doodad(a), Self::Doodad(b)) => a == b,
            (
                Self::Position {
                    position: a,
                    obj_id1: a1,
                    obj_id2: a2,
                    ..
                },
                Self::Position {
                    position: b,
                    obj_id1: b1,
                    obj_id2: b2,
                    ..
                },
            ) => a == b && a1 == b1 && a2 == b2,
            (
                Self::Position2 {
                    position: a,
                    end: ae,
                    normal: an,
                },
                Self::Position2 {
                    position: b,
                    end: be,
                    normal: bn,
                },
            ) => a == b && ae == be && an == bn,
            (
                Self::Item {
                    obj_id: a,
                    item_id: ai,
                    type1: at,
                    type2: af,
                },
                Self::Item {
                    obj_id: b,
                    item_id: bi,
                    type1: bt,
                    type2: bf,
                },
            ) => a == b && ai == bi && at == bt && af == bf,
            (Self::Position3 { position: a, pitch: ap }, Self::Position3 { position: b, pitch: bp }) => a == b && ap == bp,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::Writer;
    fn caster_cases() -> Vec<(Vec<u8>, SkillCasterData)> {
        let mut out = Vec::new();
        for kind in 0..=4 {
            let mut w = Writer::new();
            w.u8(kind).bc(0x123456);
            let expected = match kind {
                0 => SkillCasterData::Unit(0x123456),
                1 => SkillCasterData::Unk1(0x123456),
                2 => {
                    w.u64(0x0102030405060708).u32(0x11223344).u8(255).u32(0x89abcdef);
                    SkillCasterData::Item {
                        obj_id: 0x123456,
                        item_id: 0x0102030405060708,
                        template_id: 0x11223344,
                        type1: 255,
                        type2: 0x89abcdef,
                    }
                }
                3 => {
                    w.u32(0x11223344);
                    SkillCasterData::Mount {
                        obj_id: 0x123456,
                        mount_skill_id: 0x11223344,
                    }
                }
                _ => SkillCasterData::Doodad(0x123456),
            };
            out.push((w.finish(), expected));
        }
        out
    }
    fn target_cases() -> Vec<(Vec<u8>, SkillTargetData)> {
        let mut out = Vec::new();
        let position = SkillPosition {
            x: i64::MIN + 1,
            y: i64::MAX,
            z: 3.5,
        };
        for kind in 0..=5 {
            let mut w = Writer::new();
            w.u8(kind);
            let expected = match kind {
                0 => {
                    w.bc(0x123456);
                    SkillTargetData::Unit(0x123456)
                }
                4 => {
                    w.bc(0x123456);
                    SkillTargetData::Doodad(0x123456)
                }
                3 => {
                    w.bc(0x123456).u64(0x0102030405060708).u32(0x11223344).u8(255);
                    SkillTargetData::Item {
                        obj_id: 0x123456,
                        item_id: 0x0102030405060708,
                        type1: 0x11223344,
                        type2: 255,
                    }
                }
                _ => {
                    w.i64(position.x).i64(position.y).f32(position.z);
                    match kind {
                        1 => {
                            w.f32(-2.0).bc(0x654321).bc(0x123456);
                            SkillTargetData::Position {
                                position,
                                rotation: -2.0,
                                obj_id1: 0x654321,
                                obj_id2: 0x123456,
                            }
                        }
                        2 => {
                            for f in [1., 2., 3., -1., -2., -3.] {
                                w.f32(f);
                            }
                            SkillTargetData::Position2 {
                                position,
                                end: [1., 2., 3.],
                                normal: [-1., -2., -3.],
                            }
                        }
                        _ => {
                            w.f32(-4.0);
                            SkillTargetData::Position3 { position, pitch: -4.0 }
                        }
                    }
                }
            };
            out.push((w.finish(), expected));
        }
        out
    }
    #[test]
    fn complete_caster_and_target_variants_keep_exact_raw_fields_and_legacy_projections() {
        for (bytes, expected) in caster_cases() {
            let mut r = Reader::new(&bytes);
            let got = SkillCasterData::read(&mut r).unwrap();
            assert_eq!(got, expected);
            assert!(r.is_empty());
            assert_eq!(got.projection(), BuffCaster::read(&mut Reader::new(&bytes)).unwrap());
            for n in 0..bytes.len() {
                assert!(SkillCasterData::read(&mut Reader::new(&bytes[..n])).is_err());
            }
        }
        for (bytes, expected) in target_cases() {
            let mut r = Reader::new(&bytes);
            let got = SkillTargetData::read(&mut r).unwrap();
            assert_eq!(got, expected);
            assert!(r.is_empty());
            assert_eq!(got.projection(), SkillTarget::read(&mut Reader::new(&bytes)).unwrap());
            for n in 0..bytes.len() {
                assert!(SkillTargetData::read(&mut Reader::new(&bytes[..n])).is_err());
            }
        }
        for kind in 5..=255 {
            assert!(SkillCasterData::read(&mut Reader::new(&[kind])).is_err());
        }
        for kind in 6..=255 {
            assert!(SkillTargetData::read(&mut Reader::new(&[kind])).is_err());
        }
    }
    #[test]
    fn every_started_fired_identity_combination_keeps_compressed_tail_and_rejects_truncation() {
        use std::io::Write;
        for (caster_bytes, caster) in caster_cases() {
            for (target_bytes, target) in target_cases() {
                let mut started = Writer::new();
                started.u32(18132).u16(3).raw(&caster_bytes).raw(&target_bytes).u8(0).u16(0).u16(0).u8(0).u8(0);
                let started = started.finish();
                let mut fired = Writer::new();
                fired
                    .u32(18132)
                    .u16(3)
                    .raw(&caster_bytes)
                    .raw(&target_bytes)
                    .u8(0)
                    .i16(10)
                    .i16(10)
                    .u8(7)
                    .u8(73)
                    .u16(0x1234)
                    .u32(0x89abcdef)
                    .u32(296)
                    .u8(0x82);
                let fired = fired.finish();
                let s = crate::skill::SkillStarted::read(&mut Reader::new(&started)).unwrap();
                let f = crate::skill::SkillFired::read(&mut Reader::new(&fired)).unwrap();
                assert_eq!(s.raw_caster, caster);
                assert_eq!(s.raw_target, target);
                assert_eq!(f.raw_caster, caster);
                assert_eq!(f.raw_target, target);
                assert_eq!(s.caster, caster.projection());
                assert_eq!(s.target, target.projection());
                assert_eq!(f.result.extra, 0x1234);
                assert_eq!(f.result.value, 0x89abcdef);
                for n in 0..started.len() {
                    assert!(crate::skill::SkillStarted::read(&mut Reader::new(&started[..n])).is_err());
                }
                for n in 0..fired.len() {
                    assert!(crate::skill::SkillFired::read(&mut Reader::new(&fired[..n])).is_err());
                }
                let mut raw = Writer::new();
                raw.u16(0).u16(0xa1).raw(&started).u16(0).u16(0xa2).raw(&fired).u16(0).u16(0xa3).u16(3);
                let mut compressor = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
                compressor.write_all(&raw.finish()).unwrap();
                let mut packet = Writer::new();
                packet.u8(0xdd).u8(crate::game::LEVEL_COMPRESSED).u16(3).raw(&compressor.finish().unwrap());
                let crate::game::ServerMsg::Batch(messages) = crate::game::ServerMsg::decode(&packet.finish()).unwrap() else {
                    panic!()
                };
                assert_eq!(messages.len(), 3);
                assert!(matches!(&messages[0],crate::game::ServerMsg::SkillStarted(s) if s.raw_caster==caster &&s.raw_target==target));
                assert!(matches!(&messages[1],crate::game::ServerMsg::SkillFired(f) if f.raw_caster==caster &&f.raw_target==target));
                assert!(matches!(messages[2], crate::game::ServerMsg::SkillEnded { tl: 3 }));
            }
        }
    }
    #[test]
    fn original_x86_identity_oracle_all_supported_kinds_and_float_edges() {
        fn u32_at(m: &[u8; 64], o: usize) -> u32 {
            u32::from_le_bytes(m[o..o + 4].try_into().unwrap())
        }
        fn u64_at(m: &[u8; 64], o: usize) -> u64 {
            u64::from_le_bytes(m[o..o + 8].try_into().unwrap())
        }
        fn f(m: &[u8; 64], o: usize) -> f32 {
            f32::from_bits(u32_at(m, o))
        }
        fn caster(m: &[u8; 64]) -> SkillCasterData {
            match u32_at(m, 0) {
                0 => SkillCasterData::Unit(u32_at(m, 8)),
                1 => SkillCasterData::Unk1(u32_at(m, 8)),
                2 => SkillCasterData::Item {
                    obj_id: u32_at(m, 20),
                    item_id: u64_at(m, 8),
                    template_id: u32_at(m, 16),
                    type1: m[24],
                    type2: u32_at(m, 28),
                },
                3 => SkillCasterData::Mount {
                    obj_id: u32_at(m, 8),
                    mount_skill_id: u32_at(m, 12),
                },
                4 => SkillCasterData::Doodad(u32_at(m, 8)),
                _ => panic!(),
            }
        }
        fn target(m: &[u8; 64]) -> SkillTargetData {
            let position = SkillPosition {
                x: u64_at(m, 8) as i64,
                y: u64_at(m, 16) as i64,
                z: f(m, 24),
            };
            match u32_at(m, 0) {
                0 => SkillTargetData::Unit(u32_at(m, 8)),
                1 => SkillTargetData::Position {
                    position,
                    rotation: f(m, 28),
                    obj_id1: u32_at(m, 32),
                    obj_id2: u32_at(m, 36),
                },
                2 => SkillTargetData::Position2 {
                    position,
                    end: [f(m, 32), f(m, 36), f(m, 40)],
                    normal: [f(m, 44), f(m, 48), f(m, 52)],
                },
                3 => SkillTargetData::Item {
                    obj_id: u32_at(m, 8),
                    item_id: u64_at(m, 16),
                    type1: u32_at(m, 24),
                    type2: m[28],
                },
                4 => SkillTargetData::Doodad(u32_at(m, 8)),
                5 => SkillTargetData::Position3 { position, pitch: f(m, 28) },
                _ => panic!(),
            }
        }
        // Original comparators executed without hooks/mocks; native memory offsets are fixture inputs.
        let cases: &[(bool, u8, i32, u32, u8, bool)] = &[
            (true, 0, -1, 0x00000001, 255, true),
            (true, 0, 8, 0x00000001, 255, false),
            (true, 0, -1, 0x00000001, 1, false),
            (true, 1, -1, 0x00000001, 255, true),
            (true, 1, 8, 0x00000001, 255, false),
            (true, 1, -1, 0x00000001, 2, false),
            (true, 2, -1, 0x00000001, 255, true),
            (true, 2, 8, 0x00000001, 255, false),
            (true, 2, 12, 0x00000001, 255, false),
            (true, 2, 16, 0x00000001, 255, false),
            (true, 2, 20, 0x00000001, 255, false),
            (true, 2, 24, 0x00000001, 255, false),
            (true, 2, 28, 0x00000001, 255, true),
            (true, 2, -1, 0x00000001, 3, false),
            (true, 3, -1, 0x00000001, 255, true),
            (true, 3, 8, 0x00000001, 255, false),
            (true, 3, 12, 0x00000001, 255, true),
            (true, 3, -1, 0x00000001, 4, false),
            (true, 4, -1, 0x00000001, 255, true),
            (true, 4, 8, 0x00000001, 255, false),
            (true, 4, -1, 0x00000001, 0, false),
            (false, 0, -1, 0x00000001, 255, true),
            (false, 0, 8, 0x00000001, 255, false),
            (false, 0, -1, 0x00000001, 1, false),
            (false, 1, -1, 0x00000001, 255, true),
            (false, 1, 8, 0x00000001, 255, false),
            (false, 1, 12, 0x00000001, 255, false),
            (false, 1, 16, 0x00000001, 255, false),
            (false, 1, 20, 0x00000001, 255, false),
            (false, 1, 24, 0x00000001, 255, false),
            (false, 1, 28, 0x00000001, 255, true),
            (false, 1, 32, 0x00000001, 255, false),
            (false, 1, 36, 0x00000001, 255, false),
            (false, 1, -1, 0x00000001, 2, false),
            (false, 1, 24, 0x80000000, 255, true),
            (false, 1, 24, 0x7fc00001, 255, false),
            (false, 2, -1, 0x00000001, 255, true),
            (false, 2, 8, 0x00000001, 255, false),
            (false, 2, 12, 0x00000001, 255, false),
            (false, 2, 16, 0x00000001, 255, false),
            (false, 2, 20, 0x00000001, 255, false),
            (false, 2, 24, 0x00000001, 255, false),
            (false, 2, 28, 0x00000001, 255, true),
            (false, 2, 32, 0x00000001, 255, false),
            (false, 2, 36, 0x00000001, 255, false),
            (false, 2, 40, 0x00000001, 255, false),
            (false, 2, 44, 0x00000001, 255, false),
            (false, 2, 48, 0x00000001, 255, false),
            (false, 2, 52, 0x00000001, 255, false),
            (false, 2, -1, 0x00000001, 3, false),
            (false, 2, 24, 0x80000000, 255, true),
            (false, 2, 24, 0x7fc00001, 255, false),
            (false, 3, -1, 0x00000001, 255, true),
            (false, 3, 8, 0x00000001, 255, false),
            (false, 3, 12, 0x00000001, 255, true),
            (false, 3, 16, 0x00000001, 255, false),
            (false, 3, 20, 0x00000001, 255, false),
            (false, 3, 24, 0x00000001, 255, false),
            (false, 3, 28, 0x00000001, 255, false),
            (false, 3, -1, 0x00000001, 4, false),
            (false, 4, -1, 0x00000001, 255, true),
            (false, 4, 8, 0x00000001, 255, false),
            (false, 4, -1, 0x00000001, 5, false),
            (false, 5, -1, 0x00000001, 255, true),
            (false, 5, 8, 0x00000001, 255, false),
            (false, 5, 12, 0x00000001, 255, false),
            (false, 5, 16, 0x00000001, 255, false),
            (false, 5, 20, 0x00000001, 255, false),
            (false, 5, 24, 0x00000001, 255, false),
            (false, 5, 28, 0x00000001, 255, false),
            (false, 5, -1, 0x00000001, 0, false),
            (false, 5, 24, 0x80000000, 255, true),
            (false, 5, 24, 0x7fc00001, 255, false),
            (false, 1, 28, 0x7fc00001, 255, true),
            (false, 5, 28, 0x7fc00001, 255, false),
        ];
        for &(is_caster, kind, offset, bits, other_kind, expected) in cases {
            let mut a = [0u8; 64];
            a[..4].copy_from_slice(&u32::from(kind).to_le_bytes());
            let mut b = a;
            if offset >= 0 {
                let o = offset as usize;
                b[o..o + 4].copy_from_slice(&bits.to_le_bytes());
            }
            if other_kind != 255 {
                b[..4].copy_from_slice(&u32::from(other_kind).to_le_bytes());
            }
            let got = if is_caster {
                caster(&a).native_eq(&caster(&b))
            } else {
                target(&a).native_eq(&target(&b))
            };
            assert_eq!(got, expected, "caster={is_caster} kind={kind} offset={offset} bits={bits:x}");
        }
    }
}
