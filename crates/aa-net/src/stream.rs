//! Stream server messages (`CT*` / `TC*`): `[u16 len][u16 opcode][body]`, no level byte.
//!
//! The stream server carries bulk data (doodad streams, emblems, user content). The client only
//! has to join it with the token from `X2EnterWorldResponse`; its data is not interpreted.

use crate::opcodes::{ct, tc};
use crate::wire::{Reader, Result, Writer};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientMsg {
    /// `CTJoinPacket`.
    Join { account_id: u32, token: u32 },
}

impl ClientMsg {
    pub fn name(&self) -> &'static str {
        ct::name(ct::CTJoinPacket)
    }

    /// `[u16 opcode][body]`.
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        match self {
            Self::Join { account_id, token } => {
                w.u16(ct::CTJoinPacket).u32(*account_id).u32(*token);
            }
        }
        w.finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerMsg {
    /// `TCJoinResponsePacket`; `code` 0 means accepted.
    JoinResponse { code: u8 },
    Other { opcode: u16, len: usize },
}

impl ServerMsg {
    pub fn decode(payload: &[u8]) -> Result<Self> {
        let mut r = Reader::new(payload);
        Ok(match r.u16()? {
            tc::TCJoinResponsePacket => Self::JoinResponse { code: r.u8()? },
            opcode => Self::Other {
                opcode,
                len: r.remaining(),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_roundtrip() {
        // aaemu-testclient `StreamClient::join`: CT_JOIN 0x001, account id, token.
        assert_eq!(
            ClientMsg::Join { account_id: 1, token: 2 }.encode(),
            [1, 0, 1, 0, 0, 0, 2, 0, 0, 0]
        );
        assert_eq!(
            ServerMsg::decode(&[1, 0, 0]).unwrap(),
            ServerMsg::JoinResponse { code: 0 }
        );
        assert_eq!(
            ServerMsg::decode(&[2, 0, 9, 9]).unwrap(),
            ServerMsg::Other { opcode: 2, len: 2 }
        );
    }
}
