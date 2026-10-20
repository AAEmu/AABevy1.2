//! Native 1.2 / bevy-lane expedition requests. These never mutate received caches.
use crate::{ExpeditionRolePolicy, wire::Writer};
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExpeditionCommand {
    Create { name: Vec<u8>, sponsor_id: u32 },
    Rename { expedition_id: u32, name: Vec<u8> },
    Invite { name: Vec<u8> },
    Reply { expedition_id: u32, inviter_id: u32, join: bool },
    Leave,
    Kick { member_id: u32 },
    ChangeMemberRole { role: u8, member_id: u32 },
    ChangeOwner { member_id: u32 },
    ChangePolicy { policy: ExpeditionRolePolicy },
    Dismiss,
}
impl ExpeditionCommand {
    /// App-side identity/capacity checks, not the native Lua permission planner.
    pub fn validate(&self) -> Result<(), &'static str> {
        let id = |id| if id == 0 { Err("expedition identity must be nonzero") } else { Ok(()) };
        match self {
            Self::Invite { name } | Self::Create { name, .. } | Self::Rename { name, .. } => {
                if name.len() <= 128 {
                    Ok(())
                } else {
                    Err("expedition request name exceeds 128 bytes")
                }
            }
            Self::Reply { expedition_id, inviter_id, .. } => {
                id(*expedition_id)?;
                id(*inviter_id)
            }
            Self::Kick { member_id } | Self::ChangeOwner { member_id } | Self::ChangeMemberRole { member_id, .. } => id(*member_id),
            Self::ChangePolicy { policy } => {
                id(policy.expedition_id)?;
                if policy.name.len() > 128 {
                    return Err("expedition policy name exceeds 128 bytes");
                }
                if policy.permissions.iter().any(|v| *v > 1) {
                    return Err("request policy permissions must be boolean bytes");
                }
                Ok(())
            }
            Self::Leave | Self::Dismiss => Ok(()),
        }
    }
    pub(crate) fn opcode(&self) -> u16 {
        match self {
            Self::Create { .. } => 0x004,
            Self::Rename { .. } => 0x009,
            Self::ChangePolicy { .. } => 0x006,
            Self::ChangeMemberRole { .. } => 0x007,
            Self::ChangeOwner { .. } => 0x008,
            Self::Dismiss => 0x00b,
            Self::Invite { .. } => 0x00c,
            Self::Reply { .. } => 0x00d,
            Self::Leave => 0x00e,
            Self::Kick { .. } => 0x00f,
        }
    }
    pub(crate) fn write(&self, w: &mut Writer) {
        match self {
            Self::Create { name, sponsor_id } => { w.blob(name).u32(*sponsor_id); }
            Self::Rename { expedition_id, name } => { w.u32(*expedition_id).blob(name).u8(1); }
            Self::Invite { name } => {
                w.blob(name);
            }
            Self::Reply {
                expedition_id,
                inviter_id,
                join,
            } => {
                w.u32(*expedition_id).u32(*inviter_id).u8(u8::from(*join));
            }
            Self::Kick { member_id } | Self::ChangeOwner { member_id } => {
                w.u32(*member_id);
            }
            Self::ChangeMemberRole { role, member_id } => {
                w.u8(*role).u32(*member_id);
            }
            Self::ChangePolicy { policy } => {
                w.u32(policy.expedition_id).u8(policy.role).blob(&policy.name);
                for value in policy.permissions {
                    w.u8(value);
                }
            }
            Self::Leave | Self::Dismiss => {}
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::ClientMsg;
    #[test]
    fn native_lane_eight_requests_exact_wire() {
        let cases = [
            (ExpeditionCommand::Invite { name: vec![255, 0, 65] }, 12, vec![3, 0, 255, 0, 65]),
            (ExpeditionCommand::Create{name:vec![255,0,65],sponsor_id:0x12345678},4,vec![3,0,255,0,65,120,86,52,18]),
            (ExpeditionCommand::Rename{expedition_id:0x12345678,name:vec![255,0]},9,vec![120,86,52,18,2,0,255,0,1]),
            (
                ExpeditionCommand::Reply {
                    expedition_id: 9,
                    inviter_id: 0x12345678,
                    join: true,
                },
                13,
                vec![9, 0, 0, 0, 120, 86, 52, 18, 1],
            ),
            (
                ExpeditionCommand::Reply {
                    expedition_id: 9,
                    inviter_id: 1,
                    join: false,
                },
                13,
                vec![9, 0, 0, 0, 1, 0, 0, 0, 0],
            ),
            (ExpeditionCommand::Leave, 14, vec![]),
            (ExpeditionCommand::Kick { member_id: 0x12345678 }, 15, vec![120, 86, 52, 18]),
            (
                ExpeditionCommand::ChangeMemberRole {
                    role: 7,
                    member_id: 0x12345678,
                },
                7,
                vec![7, 120, 86, 52, 18],
            ),
            (ExpeditionCommand::ChangeOwner { member_id: 9 }, 8, vec![9, 0, 0, 0]),
            (ExpeditionCommand::Dismiss, 11, vec![]),
            (
                ExpeditionCommand::ChangePolicy {
                    policy: ExpeditionRolePolicy {
                        expedition_id: 9,
                        role: 7,
                        name: vec![255, 0],
                        permissions: [1, 0, 1, 0, 1, 0, 1, 0, 1],
                    },
                },
                6,
                vec![9, 0, 0, 0, 7, 2, 0, 255, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1],
            ),
        ];
        for (command, opcode, bytes) in cases {
            assert_eq!(command.validate(), Ok(()));
            let message = ClientMsg::Expedition(command);
            assert_eq!(message.opcode(), opcode);
            assert_eq!(message.encode_body(), bytes);
            assert_eq!(&message.encode(7)[6..], bytes);
        }
    }
    #[test]
    fn reply_is_expedition_then_inviter_then_join() {
        let message = ClientMsg::Expedition(ExpeditionCommand::Reply {
            expedition_id: 1001,
            inviter_id: 4,
            join: true,
        });
        assert_eq!(message.opcode(), 0x00d);
        assert_eq!(message.encode_body(), [0xe9, 0x03, 0, 0, 4, 0, 0, 0, 1]);
    }
    #[test]
    fn app_bounds_are_explicit_without_normalizing_binary_names() {
        assert!(ExpeditionCommand::Invite { name: vec![255; 128] }.validate().is_ok());
        assert!(ExpeditionCommand::Invite { name: vec![0; 129] }.validate().is_err());
        assert!(ExpeditionCommand::Kick { member_id: 0 }.validate().is_err());
        assert!(
            ExpeditionCommand::Reply {
                expedition_id: 0,
                inviter_id: 1,
                join: false
            }
            .validate()
            .is_err()
        );
        let mut policy = ExpeditionRolePolicy {
            expedition_id: 1,
            ..Default::default()
        };
        policy.permissions[3] = 2;
        assert!(ExpeditionCommand::ChangePolicy { policy }.validate().is_err());
    }
}
