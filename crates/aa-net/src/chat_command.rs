//! The chat commands that are packets of their own: `/roll` (`CSRollDice`), joining, creating and
//! leaving a player channel (`CSJoinUserChatChannel`, `CSLeaveChatChannel`), and the dice answer
//! (`SCDiceValue`).
//!
//! The layouts are the server's (`systems/services/character.rs`, `systems/social/chat.rs`):
//!
//! | packet | body |
//! |---|---|
//! | `CSRollDice` 0x0bd | `i32 max` |
//! | `CSJoinUserChatChannel` 0x061 | `string name, string password, bool create` |
//! | `CSLeaveChatChannel` 0x062 | `i64 identity` (see [`channel_identity`]) |
//! | `SCDiceValue` 0x134 | `string name, i32 max, i32 value` |
//!
//! The server broadcasts the dice answer to everything in range, the roller included. A joined or
//! left channel arrives as the usual `ChatChannelJoined` / `ChatChannelLeft`.
use crate::wire::{Reader, Result, Writer};

use crate::opcodes::{cs, sc};

/// `chat_type` of a player created channel.
const USER_CHANNEL: i16 = 15;

#[derive(Clone, PartialEq, Eq)]
pub enum ChatCommand {
    /// `/roll`: a die with `max` sides.
    RollDice { max: i32 },
    /// `/cjoin name password` (`create` false) or `/ccreate name password` (`create` true).
    JoinChannel {
        name: String,
        password: String,
        create: bool,
    },
    /// `/cleave name`: the channel with this [`channel_identity`].
    LeaveChannel { identity: i64 },
}

impl std::fmt::Debug for ChatCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RollDice { max } => f.debug_struct("RollDice").field("max", max).finish(),
            Self::JoinChannel { name, create, .. } => f
                .debug_struct("JoinChannel")
                .field("name", name)
                .field("password", &"[redacted]")
                .field("create", create)
                .finish(),
            Self::LeaveChannel { identity } => f
                .debug_struct("LeaveChannel")
                .field("identity", identity)
                .finish(),
        }
    }
}

/// The id the server knows a channel by: `faction << 32 | sub_type << 16 | chat_type`, from the
/// values `ChatChannelJoined` reported.
pub fn channel_identity(chat_type: i16, sub_type: i16, faction_id: u32) -> i64 {
    ((u64::from(faction_id) << 32)
        | (u64::from(sub_type as u16) << 16)
        | u64::from(chat_type as u16)) as i64
}

impl ChatCommand {
    pub(crate) fn opcode(&self) -> u16 {
        match self {
            Self::RollDice { .. } => cs::CSRollDicePacket,
            Self::JoinChannel { .. } => cs::CSJoinUserChatChannelPacket,
            Self::LeaveChannel { .. } => cs::CSLeaveChatChannelPacket,
        }
    }

    pub(crate) fn write(&self, w: &mut Writer) {
        match self {
            Self::RollDice { max } => {
                w.i32(*max);
            }
            Self::JoinChannel {
                name,
                password,
                create,
            } => {
                w.string(name).string(password).bool(*create);
            }
            Self::LeaveChannel { identity } => {
                w.i64(*identity);
            }
        }
    }

    /// The request can go out: a die has at least one side, a channel a name.
    pub fn validate(&self) -> std::result::Result<(), &'static str> {
        match self {
            Self::RollDice { max } if *max < 1 => Err("a die needs at least one side"),
            Self::JoinChannel { name, .. } if name.trim().is_empty() => {
                Err("a channel needs a name")
            }
            _ => Ok(()),
        }
    }
}

/// `SCDiceValue`: `name` rolled `value` on a die of `max` sides.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiceRolled {
    pub name: String,
    pub max: i32,
    pub value: i32,
}

impl DiceRolled {
    pub fn handles(opcode: u16) -> bool {
        opcode == sc::SCDiceValuePacket
    }

    pub fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            name: r.string()?,
            max: r.i32()?,
            value: r.i32()?,
        })
    }
}

/// `SCChatSpamDelay` 0x0d3: configuration sent during the proxy handshake. These are
/// received values, not a locally inferred rate limit or a reason to retry a rejected chat.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChatSpamDelay {
    pub version: u8,
    pub chat_type_groups: [u8; 15],
    pub group_delays: [f32; 15],
    pub apply_config: String,
    pub detect_config: String,
}

impl ChatSpamDelay {
    pub fn read(r: &mut Reader) -> Result<Self> {
        let version = r.u8()?;
        let chat_type_groups = r.bytes(15)?.try_into().expect("fifteen bytes");
        let mut group_delays = [0.0; 15];
        for delay in &mut group_delays {
            *delay = r.f32()?;
        }
        Ok(Self {
            version,
            chat_type_groups,
            group_delays,
            apply_config: r.string()?,
            detect_config: r.string()?,
        })
    }
}

/// `chat_type` of the channels [`ChatCommand::LeaveChannel`] can leave.
pub fn is_user_channel(chat_type: i16) -> bool {
    chat_type == USER_CHANNEL
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::ClientMsg;

    #[test]
    fn the_requests_have_the_servers_layouts() {
        let roll = ClientMsg::ChatCommand(ChatCommand::RollDice { max: 100 });
        assert_eq!(roll.name(), "CSRollDicePacket");
        assert_eq!(roll.encode_body(), [100, 0, 0, 0]);
        let join = ClientMsg::ChatCommand(ChatCommand::JoinChannel {
            name: "ab".into(),
            password: String::new(),
            create: true,
        });
        assert_eq!(join.encode_body(), [2, 0, b'a', b'b', 0, 0, 1]);
        let leave = ClientMsg::ChatCommand(ChatCommand::LeaveChannel {
            identity: channel_identity(15, 3, 0),
        });
        assert_eq!(leave.encode_body(), [15, 0, 3, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn the_dice_answer_reads() {
        let mut w = Writer::new();
        w.string("Bob").i32(100).i32(42);
        let bytes = w.finish();
        let mut r = Reader::new(&bytes);
        assert_eq!(
            DiceRolled::read(&mut r).unwrap(),
            DiceRolled {
                name: "Bob".into(),
                max: 100,
                value: 42
            }
        );
    }

    #[test]
    fn invalid_requests_are_refused() {
        assert!(ChatCommand::RollDice { max: 0 }.validate().is_err());
        assert!(ChatCommand::RollDice { max: 6 }.validate().is_ok());
        assert!(
            ChatCommand::JoinChannel {
                name: " ".into(),
                password: String::new(),
                create: false
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn channel_password_is_redacted_at_every_debug_layer() {
        let command = ChatCommand::JoinChannel {
            name: "group".into(),
            password: "secret-channel-password".into(),
            create: true,
        };
        for debug in [
            format!("{command:?}"),
            format!("{:?}", ClientMsg::ChatCommand(command.clone())),
            format!("{:?}", crate::Command::ChatCommand(command)),
        ] {
            assert!(debug.contains("[redacted]"));
            assert!(!debug.contains("secret-channel-password"));
        }
        assert_eq!(
            channel_identity(-1, -2, 0xabcdef01) as u64,
            0xabcdef01_fffe_ffff
        );
    }
}
