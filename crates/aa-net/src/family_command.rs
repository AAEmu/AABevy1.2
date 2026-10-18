//! Source-backed Family request payloads; roster authority remains received-only.
use crate::wire::Writer;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FamilyCommand {
    Invite { name: Vec<u8>, title: Vec<u8> },
    Reply { inviter_id: u32, join: bool, role: Vec<u8> },
    Leave,
    Kick { member_id: u32 },
    ChangeTitle { member_id: u32, title: Vec<u8> },
    ChangeOwner { member_id: u32 },
}
impl FamilyCommand {
    /// Native constructor capacities exclude the terminal C-string byte. This typed wire API
    /// preserves binary bytes; native Lua name lookup/copy has additional C-string behavior.
    pub fn validate(&self) -> Result<(), &'static str> {
        let title = |bytes: &[u8]| {
            if bytes.len() <= 104 {
                Ok(())
            } else {
                Err("family title/role exceeds 104 bytes")
            }
        };
        let id = |value| if value != 0 { Ok(()) } else { Err("family character ID must be nonzero") };
        match self {
            Self::Invite { name, title: value } => {
                if name.len() > 128 {
                    return Err("family invite name exceeds 128 bytes");
                }
                title(value)
            }
            Self::Reply { inviter_id, role, .. } => {
                id(*inviter_id)?;
                title(role)
            }
            Self::Leave => Ok(()),
            Self::Kick { member_id } | Self::ChangeOwner { member_id } => id(*member_id),
            Self::ChangeTitle { member_id, title: value } => {
                id(*member_id)?;
                title(value)
            }
        }
    }
    pub(crate) fn opcode(&self) -> u16 {
        match self {
            Self::Invite { .. } => crate::opcodes::cs::CSFamilyInviteMemberPacket,
            Self::Reply { .. } => crate::opcodes::cs::CSFamilyReplyInvitationPacket,
            Self::Leave => crate::opcodes::cs::CSFamilyLeavePacket,
            Self::Kick { .. } => crate::opcodes::cs::CSFamilyKickPacket,
            Self::ChangeTitle { .. } => crate::opcodes::cs::CSFamilyChangeTitlePacket,
            Self::ChangeOwner { .. } => crate::opcodes::cs::CSFamilyChangeOwnerPacket,
        }
    }
    pub(crate) fn write(&self, w: &mut Writer) {
        match self {
            Self::Invite { name, title } => {
                w.blob(name).blob(title);
            }
            Self::Reply { inviter_id, join, role } => {
                w.u32(*inviter_id).u8(u8::from(*join)).blob(role);
            }
            Self::Leave => {}
            Self::Kick { member_id } | Self::ChangeOwner { member_id } => {
                w.u32(*member_id);
            }
            Self::ChangeTitle { member_id, title } => {
                w.u32(*member_id).blob(title);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::ClientMsg;
    #[test]
    fn native_and_lane_six_family_payloads_are_exact_and_binary_strings_survive() {
        let cases = [
            (
                FamilyCommand::Invite {
                    name: vec![0xff, 0, b'A'],
                    title: vec![0x80],
                },
                0x1a,
                vec![3, 0, 0xff, 0, b'A', 1, 0, 0x80],
            ),
            (
                FamilyCommand::Reply {
                    inviter_id: 0x12345678,
                    join: true,
                    role: vec![0, 0xfe],
                },
                0x1b,
                vec![0x78, 0x56, 0x34, 0x12, 1, 2, 0, 0, 0xfe],
            ),
            (
                FamilyCommand::Reply {
                    inviter_id: 1,
                    join: false,
                    role: vec![],
                },
                0x1b,
                vec![1, 0, 0, 0, 0, 0, 0],
            ),
            (FamilyCommand::Leave, 0x1c, vec![]),
            (FamilyCommand::Kick { member_id: 0x12345678 }, 0x1d, vec![0x78, 0x56, 0x34, 0x12]),
            (
                FamilyCommand::ChangeTitle {
                    member_id: 9,
                    title: vec![0xff, 0],
                },
                0x1e,
                vec![9, 0, 0, 0, 2, 0, 0xff, 0],
            ),
            (FamilyCommand::ChangeOwner { member_id: 0x12345678 }, 0x1f, vec![0x78, 0x56, 0x34, 0x12]),
        ];
        for (command, opcode, bytes) in cases {
            assert_eq!(command.validate(), Ok(()));
            let message = ClientMsg::Family(command);
            assert_eq!(message.opcode(), opcode);
            assert_eq!(message.encode_body(), bytes);
            assert_eq!(&message.encode(7)[6..], bytes);
        }
    }
    #[test]
    fn explicit_app_bounds_reject_oversize_and_zero_identity_without_normalizing_bytes() {
        assert!(
            FamilyCommand::Invite {
                name: vec![255; 128],
                title: vec![0; 104]
            }
            .validate()
            .is_ok()
        );
        assert!(
            FamilyCommand::Invite {
                name: vec![255; 129],
                title: vec![]
            }
            .validate()
            .is_err()
        );
        assert!(
            FamilyCommand::Invite {
                name: vec![],
                title: vec![1; 105]
            }
            .validate()
            .is_err()
        );
        assert!(
            FamilyCommand::Reply {
                inviter_id: 1,
                join: false,
                role: vec![1; 105]
            }
            .validate()
            .is_err()
        );
        for command in [
            FamilyCommand::Reply {
                inviter_id: 0,
                join: true,
                role: vec![],
            },
            FamilyCommand::Kick { member_id: 0 },
            FamilyCommand::ChangeTitle { member_id: 0, title: vec![] },
            FamilyCommand::ChangeOwner { member_id: 0 },
        ] {
            assert!(command.validate().is_err());
        }
    }
}
