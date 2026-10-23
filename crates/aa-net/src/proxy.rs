//! Received CryNetwork proxy messages. Raw timestamps do not imply a shared clock domain.
use crate::wire::{Reader, Result};

/// Proxy `SetGameType` (0x00f), sent during the lobby handshake. `level` is the
/// server's level name, not a character zone or a request to synthesize terrain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameType {
    pub level: String,
    pub checksum: u64,
    pub immersive: u8,
}

impl GameType {
    pub fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self { level: r.string()?, checksum: r.u64()?, immersive: r.u8()? })
    }
}

/// Original proxy opcode 0x13 and active lane: four i64 fields followed by two u32 fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProxyPong {
    pub tm: i64,
    pub when: i64,
    pub elapsed: i64,
    pub remote: i64,
    /// Echoed local send-time milliseconds used by native TimeKeeper::AddSample.
    pub local: u32,
    /// Received remote milliseconds used by native TimeKeeper::AddSample.
    pub world: u32,
}
impl ProxyPong {
    pub fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            tm: r.i64()?,
            when: r.i64()?,
            elapsed: r.i64()?,
            remote: r.i64()?,
            local: r.u32()?,
            world: r.u32()?,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        game::{LEVEL_PROXY, MAGIC, ServerMsg},
        opcodes::pp,
        wire::Writer,
    };
    #[test]
    fn pong_native_lane_golden_retains_all_timestamps_and_rejects_every_truncation() {
        // Matches lane packets/proxy.rs's independent golden, including remote=world*1000.
        let mut bytes = vec![MAGIC, LEVEL_PROXY, 0x13, 0];
        for v in [1i64, 2, 0, 4000] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        bytes.extend_from_slice(&3u32.to_le_bytes());
        bytes.extend_from_slice(&4u32.to_le_bytes());
        let expected = ProxyPong {
            tm: 1,
            when: 2,
            elapsed: 0,
            remote: 4000,
            local: 3,
            world: 4,
        };
        assert_eq!(ServerMsg::decode(&bytes).unwrap(), ServerMsg::Pong(expected));
        for cut in 4..bytes.len() {
            assert!(ServerMsg::decode(&bytes[..cut]).is_err(), "cut {cut}");
        }
        let mut w = Writer::new();
        w.i64(i64::MIN).i64(i64::MAX).i64(-3).i64(-4).u32(u32::MAX).u32(0x80000000).u16(0xcafe);
        let b = w.finish();
        let mut r = Reader::new(&b);
        let p = ProxyPong::read(&mut r).unwrap();
        assert_eq!(
            (p.tm, p.when, p.elapsed, p.remote, p.local, p.world),
            (i64::MIN, i64::MAX, -3, -4, u32::MAX, 0x80000000)
        );
        assert_eq!(r.u16().unwrap(), 0xcafe);
        assert_eq!(pp::PongPacket, 0x13);
    }
}
