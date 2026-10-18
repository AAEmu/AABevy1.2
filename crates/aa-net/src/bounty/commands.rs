//! Original bounty controls. The active bevy lane has no CS11f/120 handlers.
use crate::wire::Writer;
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BountyCommand {
    /// Captured native bulletin interaction. Caller admits the actual doodad interaction first.
    OpenBulletin {
        doodad_obj_id: u32,
    },
    RequestSetMoney {
        name: Vec<u8>,
    },
    Update {
        kind: u32,
        name: Vec<u8>,
        money: i32,
    },
}
impl BountyCommand {
    fn cstring(name: &[u8]) -> &[u8] {
        &name[..name.iter().position(|b| *b == 0).unwrap_or(name.len())]
    }
    fn cvtt(value: f32) -> i32 {
        if !value.is_finite() || value >= 2147483648.0 || value < -2147483648.0 {
            i32::MIN
        } else {
            value as i32
        }
    }
    pub fn request_set_money(name: &[u8]) -> Result<Self, &'static str> {
        let value = Self::RequestSetMoney {
            name: Self::cstring(name).to_vec(),
        };
        value.validate()?;
        Ok(value)
    }
    /// Caller must admit actual Lua numeric/string types first. These are the native numeric conversions.
    pub fn update_from_lua(kind: f32, name: &[u8], money: f32) -> Result<Self, &'static str> {
        let value = Self::Update {
            kind: Self::cvtt(kind) as u32,
            name: Self::cstring(name).to_vec(),
            money: Self::cvtt(money),
        };
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if let Self::OpenBulletin { doodad_obj_id } = self {
            return if *doodad_obj_id == 0 || *doodad_obj_id > 0x00ff_ffff {
                Err("bounty bulletin requires a nonzero 24-bit doodad object identity")
            } else {
                Ok(())
            };
        }
        let name = match self {
            Self::RequestSetMoney { name } | Self::Update { name, .. } => name,
            Self::OpenBulletin { .. } => unreachable!(),
        };
        if Self::cstring(name).len() > 128 {
            Err("bounty name exceeds the native 128-byte C-string buffer")
        } else {
            Ok(())
        }
    }
    pub fn opcode(&self) -> u16 {
        match self {
            Self::OpenBulletin { .. } => 0xea,
            Self::RequestSetMoney { .. } => 0x11f,
            Self::Update { .. } => 0x120,
        }
    }
    pub fn write(&self, w: &mut Writer) {
        match self {
            Self::OpenBulletin { doodad_obj_id } => {
                w.bc(*doodad_obj_id);
            }
            Self::RequestSetMoney { name } => {
                w.blob(Self::cstring(name));
            }
            Self::Update { kind, name, money } => {
                w.u32(*kind).blob(Self::cstring(name)).i32(*money);
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_constructor_and_serializer_goldens() {
        let cases = [
            (BountyCommand::request_set_money(b"N\0tail").unwrap(), 0x11f, vec![1, 0, b'N']),
            (
                BountyCommand::Update {
                    kind: u32::MAX,
                    name: vec![255],
                    money: i32::MIN,
                },
                0x120,
                vec![255, 255, 255, 255, 1, 0, 255, 0, 0, 0, 128],
            ),
        ];
        for (cmd, op, body) in cases {
            let msg = crate::game::ClientMsg::Bounty(cmd);
            assert_eq!(msg.opcode(), op);
            assert_eq!(msg.encode_body(), body);
        }
    }
    #[test]
    fn native_numeric_and_cstring_boundary_not_saturating_rust_cast() {
        assert_eq!(
            BountyCommand::update_from_lua(-1.9, b"X", -3.9).unwrap(),
            BountyCommand::Update {
                kind: u32::MAX,
                name: b"X".to_vec(),
                money: -3
            }
        );
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 2147483648.0] {
            assert!(matches!(
                BountyCommand::update_from_lua(value, b"", value).unwrap(),
                BountyCommand::Update {
                    kind: 0x80000000,
                    money: i32::MIN,
                    ..
                }
            ));
        }
        assert!(BountyCommand::request_set_money(&[255; 128]).is_ok());
        assert!(BountyCommand::request_set_money(&[255; 129]).is_err());
        let mut name = vec![b'X'; 128];
        name.push(0);
        name.extend_from_slice(&[b'Y'; 200]);
        let cmd = BountyCommand::request_set_money(&name).unwrap();
        assert!(matches!(cmd,BountyCommand::RequestSetMoney{name} if name==vec![b'X';128]));
    }
    #[test]
    fn native_bulletin_object_identity_and_three_byte_wire() {
        let command = BountyCommand::OpenBulletin { doodad_obj_id: 0x123456 };
        assert!(command.validate().is_ok());
        let msg = crate::game::ClientMsg::Bounty(command);
        assert_eq!(msg.opcode(), 0xea);
        assert_eq!(msg.encode_body(), vec![0x56, 0x34, 0x12]);
        for id in [0, 0x1000000, u32::MAX] {
            assert!(BountyCommand::OpenBulletin { doodad_obj_id: id }.validate().is_err());
        }
    }
}
