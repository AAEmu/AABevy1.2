//! Lane world-loading and blink directives, separate from completed movement reports.

use crate::wire::{Pos, Reader, Result, long_to_coord};

/// `SCLoadInstance`'s fixed-size loading directive, without assuming load completion.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InstanceLoad {
    /// Lane senders use either a world template ID or runtime world ID in this field.
    pub raw_id: u32,
    pub zone_id: u32,
    pub pos: Pos,
    /// Lane senders convert their stored angles to radians; all three wire values survive.
    pub angles: [f32; 3],
}

impl InstanceLoad {
    pub fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            raw_id: r.u32()?,
            zone_id: r.u32()?,
            pos: Pos::new(r.f32()?, r.f32()?, r.f32()?),
            angles: [r.f32()?, r.f32()?, r.f32()?],
        })
    }
}

/// `SCBlinkUnit`'s movement directive. Receiving it alone does not settle a unit's position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlinkUnit {
    pub obj_id: u32,
    pub distance: f32,
    /// Original wire value; no direction/rotation conversion is inferred.
    pub degree: f32,
    /// Raw native fixed-point bits, including the lower 32 bits.
    pub x: i64,
    pub y: i64,
    pub z: f32,
}

impl BlinkUnit {
    pub fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            obj_id: r.bc()?,
            distance: r.f32()?,
            degree: r.f32()?,
            x: r.i64()?,
            y: r.i64()?,
            z: r.f32()?,
        })
    }

    pub fn position(&self) -> Pos {
        Pos::new(long_to_coord(self.x), long_to_coord(self.y), self.z)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::ServerMsg;
    use crate::opcodes::sc;
    use crate::wire::Writer;

    fn decode(opcode: u16, raw: &[u8]) -> crate::wire::Result<ServerMsg> {
        let mut frame = Writer::new();
        frame.raw(&[0xdd, 1, 0, 0]).u16(opcode).raw(raw);
        ServerMsg::decode(&frame.finish())
    }

    fn following_snow(opcode: u16, raw: &[u8]) -> ServerMsg {
        let mut inner = Writer::new();
        inner
            .u16(0)
            .u16(opcode)
            .raw(raw)
            .u16(0)
            .u16(sc::SCOnOffSnowPacket)
            .bool(true);
        let inner = inner.finish();
        let len = inner.len() as u16;
        let mut outer = Writer::new();
        outer
            .raw(&[0xdd, 4])
            .u16(2)
            .u8(1)
            .u16(len)
            .u16(!len)
            .raw(&inner);
        ServerMsg::decode(&outer.finish()).unwrap()
    }

    #[test]
    fn load_instance_fixed_body_preserves_raw_identity_angles_and_dd04_tail() {
        // Independent LE bytes for the lane packets_special.rs writer: two u32, six f32.
        let raw = [
            0x98, 0xba, 0xdc, 0xfe, 0x67, 0x45, 0x23, 0x81, 0, 0, 0x80, 0x3f, 0, 0, 0, 0xc0, 0, 0,
            0x60, 0x40, 0, 0, 0x80, 0xbe, 0, 0, 0, 0x3f, 0, 0, 0x40, 0xbf,
        ];
        let expected = InstanceLoad {
            raw_id: 0xfedcba98,
            zone_id: 0x81234567,
            pos: Pos::new(1.0, -2.0, 3.5),
            angles: [-0.25, 0.5, -0.75],
        };
        let mut r = Reader::new(&raw);
        assert_eq!(InstanceLoad::read(&mut r).unwrap(), expected);
        assert!(r.is_empty());
        let message = ServerMsg::LoadInstance(expected);
        assert_eq!(decode(sc::SCLoadInstancePacket, &raw).unwrap(), message);
        assert_eq!(message.name(), "SCLoadInstance");
        assert_eq!(
            following_snow(sc::SCLoadInstancePacket, &raw),
            ServerMsg::Batch(vec![message, ServerMsg::Snow(true)])
        );
        for n in 0..raw.len() {
            assert!(InstanceLoad::read(&mut Reader::new(&raw[..n])).is_err());
            assert!(decode(sc::SCLoadInstancePacket, &raw[..n]).is_err());
        }
    }

    #[test]
    fn blink_fixed_body_preserves_raw_xy_and_following_dd04_entry() {
        // bc object, distance/degree, two raw i64, height; not packed-position or a teleport ACK.
        let raw = [
            0x56, 0x34, 0x12, 0, 0, 0x48, 0x41, 0, 0, 0x34, 0xc2, 0xef, 0xcd, 0xab, 0x89, 0, 0x10,
            0, 0, 0x67, 0x45, 0x23, 1, 0, 0xe0, 0xff, 0xff, 0, 0, 0xe0, 0x40,
        ];
        let expected = BlinkUnit {
            obj_id: 0x123456,
            distance: 12.5,
            degree: -45.0,
            x: 0x0000100089abcdef,
            y: -0x00001ffffedcba99,
            z: 7.0,
        };
        let mut r = Reader::new(&raw);
        assert_eq!(BlinkUnit::read(&mut r).unwrap(), expected);
        assert!(r.is_empty());
        assert_eq!(expected.position(), Pos::new(1.0, -2.0, 7.0));
        let message = ServerMsg::BlinkUnit(expected);
        assert_eq!(decode(sc::SCBlinkUnitPacket, &raw).unwrap(), message);
        assert_eq!(message.name(), "SCBlinkUnit");
        assert_eq!(
            following_snow(sc::SCBlinkUnitPacket, &raw),
            ServerMsg::Batch(vec![message, ServerMsg::Snow(true)])
        );
        for n in 0..raw.len() {
            assert!(BlinkUnit::read(&mut Reader::new(&raw[..n])).is_err());
            assert!(decode(sc::SCBlinkUnitPacket, &raw[..n]).is_err());
        }
    }

    #[test]
    fn receive_float_fields_are_not_silently_sanitized_or_reinterpreted() {
        let bits = [
            0x80000000, 0x7f800000, 0xff800000, 0x7fc01234, 0, 0x3f800000,
        ];
        let mut w = Writer::new();
        w.u32(0).u32(u32::MAX);
        for value in bits {
            w.u32(value);
        }
        let load = InstanceLoad::read(&mut Reader::new(&w.finish())).unwrap();
        assert_eq!(load.pos.x.to_bits(), bits[0]);
        assert_eq!(load.pos.y.to_bits(), bits[1]);
        assert_eq!(load.pos.z.to_bits(), bits[2]);
        assert_eq!(load.angles.map(f32::to_bits), [bits[3], bits[4], bits[5]]);
        let mut w = Writer::new();
        w.bc(0)
            .u32(bits[3])
            .u32(bits[1])
            .i64(i64::MIN)
            .i64(i64::MAX)
            .u32(bits[0]);
        let blink = BlinkUnit::read(&mut Reader::new(&w.finish())).unwrap();
        assert_eq!(blink.distance.to_bits(), bits[3]);
        assert_eq!(blink.degree.to_bits(), bits[1]);
        assert_eq!((blink.x, blink.y), (i64::MIN, i64::MAX));
        assert_eq!(blink.z.to_bits(), bits[0]);
    }
}
