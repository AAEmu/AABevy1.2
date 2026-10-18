//! Original bounty controls. The active bevy lane has no CS11f/120 handlers.
use crate::wire::Writer;
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BountyCommand {
    RequestSetMoney { name: Vec<u8> },
    Update { kind: u32, name: Vec<u8>, money: i32 },
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
        let name = match self {
            Self::RequestSetMoney { name } | Self::Update { name, .. } => name,
        };
        if Self::cstring(name).len() > 128 {
            Err("bounty name exceeds the native 128-byte C-string buffer")
        } else {
            Ok(())
        }
    }
    pub fn opcode(&self) -> u16 {
        match self {
            Self::RequestSetMoney { .. } => 0x11f,
            Self::Update { .. } => 0x120,
        }
    }
    pub fn write(&self, w: &mut Writer) {
        match self {
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
}
