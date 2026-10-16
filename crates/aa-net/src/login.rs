//! Login server messages (`CA*` from the client, `AC*` from the server).
//!
//! Frames are `[u16 len][u16 opcode][body]`. The 1.2 launcher authenticates with a Trion ticket: an
//! XML document with the user name and the hex SHA-256 of the password
//! (`CARequestAuthTrionPacket`). That is the flow used here; the server's `AutoAccount` setting
//! creates unknown accounts on their first login.

use std::fmt;
use std::net::{Ipv4Addr, SocketAddrV4};

use sha2::{Digest, Sha256};

use crate::opcodes::{ac, ca};
use crate::wire::{Reader, Result, Writer};

/// Congestion levels of a world and of a race on a world (`RACE_CONGESTION`).
pub mod congestion {
    pub const LOW: u8 = 0;
    pub const MIDDLE: u8 = 1;
    pub const HIGH: u8 = 2;
    pub const FULL: u8 = 3;
    pub const PRE_SELECT_RACE_FULL: u8 = 9;
    pub const CHECK: u8 = 10;
}

/// Index of a race in [`World::race_congestion`] is the race id (`model::race`), 0 unused.
pub const RACE_SLOTS: usize = 9;

/// A world (game server) of `ACWorldListPacket`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct World {
    pub id: u8,
    pub name: String,
    /// Registered and accepting players; offline worlds carry no congestion data.
    pub online: bool,
    /// World congestion ([`congestion::LOW`] / `MIDDLE` / `HIGH`).
    pub congestion: u8,
    /// Congestion per race, indexed by race id.
    pub race_congestion: [u8; RACE_SLOTS],
}

/// A character of the account on some world, as listed by the login server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoginCharacter {
    pub account_id: u32,
    pub world_id: u8,
    pub id: u32,
    pub name: String,
    pub race: u8,
    pub gender: u8,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorldList {
    pub worlds: Vec<World>,
    pub characters: Vec<LoginCharacter>,
}

impl WorldList {
    pub fn world(&self, id: u8) -> Option<&World> {
        self.worlds.iter().find(|w| w.id == id)
    }

    /// Characters the account has on world `id`.
    pub fn character_count(&self, id: u8) -> usize {
        self.characters.iter().filter(|c| c.world_id == id).count()
    }

    pub fn characters_of(&self, id: u8) -> impl Iterator<Item = &LoginCharacter> {
        self.characters.iter().filter(move |c| c.world_id == id)
    }
}

/// Position in the queue for a full world (`ACWorldQueuePacket`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorldQueue {
    pub world_id: u8,
    pub premium: bool,
    /// Place in line.
    pub my_turn: u16,
    pub normal_length: u16,
    pub premium_length: u16,
}

/// Where and how to connect to the game server (`ACWorldCookiePacket`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorldCookie {
    pub cookie: u32,
    pub addr: SocketAddrV4,
}

/// Account facts from `ACJoinResponse` / `ACAuthResponse`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AccountInfo {
    pub account_id: u32,
    /// Characters per account without expansion.
    pub max_characters: u8,
    pub extra_characters_per_world: u32,
    /// The server is in character pre-creation mode.
    pub pre_select_period: bool,
    /// `slot_count` of `ACAuthResponse`.
    pub slot_count: u8,
}

/// A credential that never shows up in logs: its `Debug` output is redacted.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The plain value.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl From<String> for Secret {
    fn from(v: String) -> Self {
        Self(v)
    }
}

impl From<&str> for Secret {
    fn from(v: &str) -> Self {
        Self(v.to_string())
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// The launcher's Trion ticket for `user` / `password`.
pub fn trion_ticket(user: &str, password: &str) -> String {
    let hash: String = Sha256::digest(password.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
         <authTicket version=\"1.2\"><storeToken>1</storeToken>\
         <username>{}</username><password>{hash}</password></authTicket>",
        xml_escape(user)
    )
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Messages the client sends to the login server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientMsg {
    /// `CARequestAuthTrionPacket`.
    AuthTrion { ticket: Secret },
    /// `CAListWorldPacket`.
    ListWorlds,
    /// `CAEnterWorldPacket`.
    EnterWorld { world_id: u8 },
    /// `CACancelEnterWorldPacket`.
    CancelEnterWorld { world_id: u8 },
    /// `CARequestReconnectPacket`: continue a session after the game server sent
    /// `SCReconnectAuth` (return to world select).
    Reconnect { account_id: u32, world_id: u8, token: u32 },
}

impl ClientMsg {
    pub fn auth(user: &str, password: &str) -> Self {
        Self::AuthTrion {
            ticket: Secret(trion_ticket(user, password)),
        }
    }

    pub fn opcode(&self) -> u16 {
        match self {
            Self::AuthTrion { .. } => ca::CARequestAuthTrionPacket,
            Self::ListWorlds => ca::CAListWorldPacket,
            Self::EnterWorld { .. } => ca::CAEnterWorldPacket,
            Self::CancelEnterWorld { .. } => ca::CACancelEnterWorldPacket,
            Self::Reconnect { .. } => ca::CARequestReconnectPacket,
        }
    }

    pub fn name(&self) -> &'static str {
        ca::name(self.opcode())
    }

    pub fn encode_body(&self) -> Vec<u8> {
        let mut w = Writer::new();
        match self {
            Self::AuthTrion { ticket } => {
                w.u32(0) // pFrom
                    .u32(0) // pTo
                    .bool(false) // dev
                    .blob(&[0x00, 0x16, 0x3e, 0x00, 0x00, 0x01]) // mac
                    .string(&ticket.0)
                    .string("") // signature
                    .bool(true); // isLast
            }
            Self::ListWorlds => {
                w.u64(0);
            }
            Self::EnterWorld { world_id } => {
                w.u64(0).u8(*world_id);
            }
            Self::CancelEnterWorld { world_id } => {
                w.u8(*world_id);
            }
            Self::Reconnect { account_id, world_id, token } => {
                w.u32(0)
                    .u32(0)
                    .u32(*account_id)
                    .u8(*world_id)
                    .u32(*token)
                    .blob(&[0x00, 0x16, 0x3e, 0x00, 0x00, 0x01]);
            }
        }
        w.finish()
    }

    /// `[u16 opcode][body]`, the payload of a frame.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = self.opcode().to_le_bytes().to_vec();
        out.extend(self.encode_body());
        out
    }
}

/// Messages the login server sends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerMsg {
    /// `ACJoinResponsePacket`: protocol accepted (`reason` 0) plus account feature flags.
    JoinResponse { reason: u16, afs: u64 },
    /// `ACAuthResponsePacket`: authenticated.
    AuthResponse { account_id: u32, slot_count: u8 },
    WorldList(WorldList),
    WorldQueue(WorldQueue),
    WorldCookie(WorldCookie),
    EnterWorldDenied { reason: u8 },
    LoginDenied { reason: u8, message: String },
    AccountWarned { source: u8, message: String },
    /// A packet of the Korean authentication flows (challenge, OTP, ARS, ...) or an unknown one.
    Unsupported { opcode: u16 },
}

impl ServerMsg {
    pub fn name(&self) -> &'static str {
        match self {
            Self::JoinResponse { .. } => "ACJoinResponse",
            Self::AuthResponse { .. } => "ACAuthResponse",
            Self::WorldList(_) => "ACWorldList",
            Self::WorldQueue(_) => "ACWorldQueue",
            Self::WorldCookie(_) => "ACWorldCookie",
            Self::EnterWorldDenied { .. } => "ACEnterWorldDenied",
            Self::LoginDenied { .. } => "ACLoginDenied",
            Self::AccountWarned { .. } => "ACAccountWarned",
            Self::Unsupported { .. } => "unsupported",
        }
    }

    /// Decodes a frame payload (`[u16 opcode][body]`).
    pub fn decode(payload: &[u8]) -> Result<Self> {
        let mut r = Reader::new(payload);
        let opcode = r.u16()?;
        Ok(match opcode {
            ac::ACJoinResponsePacket => Self::JoinResponse {
                reason: r.u16()?,
                afs: r.u64()?,
            },
            ac::ACAuthResponsePacket => {
                let account_id = r.u32()?;
                r.blob()?; // wsk
                Self::AuthResponse {
                    account_id,
                    slot_count: r.u8()?,
                }
            }
            ac::ACWorldListPacket => Self::WorldList(read_world_list(&mut r)?),
            ac::ACWorldQueuePacket => Self::WorldQueue(WorldQueue {
                world_id: r.u8()?,
                premium: r.bool()?,
                my_turn: r.u16()?,
                normal_length: r.u16()?,
                premium_length: r.u16()?,
            }),
            ac::ACWorldCookiePacket => {
                let cookie = r.u32()?;
                let ip = r.bytes(4)?;
                let port = r.u16()?;
                // The address is written with its octets reversed.
                Self::WorldCookie(WorldCookie {
                    cookie,
                    addr: SocketAddrV4::new(Ipv4Addr::new(ip[3], ip[2], ip[1], ip[0]), port),
                })
            }
            ac::ACEnterWorldDeniedPacket => Self::EnterWorldDenied { reason: r.u8()? },
            ac::ACLoginDeniedPacket => {
                let reason = r.u8()?;
                r.blob()?; // message parameters
                Self::LoginDenied {
                    reason,
                    message: r.string()?,
                }
            }
            ac::ACAccountWarnedPacket => Self::AccountWarned {
                source: r.u8()?,
                message: r.string()?,
            },
            opcode => Self::Unsupported { opcode },
        })
    }
}

fn read_world_list(r: &mut Reader) -> Result<WorldList> {
    let mut out = WorldList::default();
    for _ in 0..r.u8()? {
        let id = r.u8()?;
        let name = r.string()?;
        let online = r.bool()?;
        let mut world = World {
            id,
            name,
            online,
            congestion: congestion::LOW,
            race_congestion: [congestion::LOW; RACE_SLOTS],
        };
        if online {
            world.congestion = r.u8()?;
            for c in &mut world.race_congestion {
                *c = r.u8()?;
            }
        }
        out.worlds.push(world);
    }
    for _ in 0..r.u8()? {
        let account_id = r.u32()?;
        let world_id = r.u8()?;
        let id = r.u32()?;
        let name = r.string()?;
        let race = r.u8()?;
        let gender = r.u8()?;
        r.blob()?; // guid
        r.skip(8)?;
        out.characters.push(LoginCharacter {
            account_id,
            world_id,
            id,
            name,
            race,
            gender,
        });
    }
    Ok(out)
}

/// Why a login or enter-world request failed, in words.
pub fn login_denied_text(reason: u8) -> &'static str {
    match reason {
        0 => "authentication failed",
        1 => "wrong user name or password",
        2 => "wrong user name or password (bad response)",
        3 => "account already logged in",
        4 => "service unavailable",
        47 => "server congested",
        48 => "invalid mac address",
        _ => "denied",
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::wire::WireError;

    /// A `[u16 len][payload]` frame in hex, as printed by the server's own packet tests, to the
    /// payload.
    pub fn frame_payload(hex: &str) -> Vec<u8> {
        let bytes: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        let len = u16::from_le_bytes([bytes[0], bytes[1]]) as usize;
        assert_eq!(len, bytes.len() - 2, "frame length prefix");
        bytes[2..].to_vec()
    }

    #[test]
    fn ticket_carries_the_hashed_password() {
        // Same vector as aaemu-testclient's `ticket_contains_hashed_password`.
        let t = trion_ticket("bob", "secret");
        assert!(t.contains("<username>bob</username>"));
        assert!(t.contains(
            "<password>2bb80d537b1da3e38bd30361aa855686bde0eacd7162fef6a25fe97bf527a25b</password>"
        ));
        assert!(t.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>"));
        assert!(!format!("{:?}", ClientMsg::auth("bob", "secret")).contains("2bb80d"));
    }

    #[test]
    fn client_message_bodies() {
        // CAListWorld: u64 flag. CAEnterWorld: u64 flag + world id.
        assert_eq!(ClientMsg::ListWorlds.encode(), [0x0b, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(
            ClientMsg::EnterWorld { world_id: 1 }.encode(),
            [0x0c, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]
        );
        assert_eq!(ClientMsg::CancelEnterWorld { world_id: 2 }.encode(), [0x0d, 0, 2]);

        // CARequestAuthTrion: pFrom, pTo, dev, mac blob, ticket, signature, isLast.
        let body = ClientMsg::auth("bob", "secret").encode_body();
        let mut r = Reader::new(&body);
        assert_eq!((r.u32().unwrap(), r.u32().unwrap(), r.bool().unwrap()), (0, 0, false));
        assert_eq!(r.blob().unwrap(), [0x00, 0x16, 0x3e, 0x00, 0x00, 0x01]);
        assert!(r.string().unwrap().contains("<username>bob</username>"));
        assert_eq!(r.string().unwrap(), "");
        assert!(r.bool().unwrap());
        assert!(r.is_empty());
    }

    #[test]
    fn server_golden_auth_join_and_denied() {
        // Frames produced by the server's own tests (`packets/l2c.rs`).
        let m = ServerMsg::decode(&frame_payload(
            "29000300040302012000000000000000000000000000000000000000000000000000000000000000000006",
        ))
        .unwrap();
        assert_eq!(m, ServerMsg::AuthResponse { account_id: 0x0102_0304, slot_count: 6 });

        let m = ServerMsg::decode(&frame_payload("0c00000000000600000000000000")).unwrap();
        assert_eq!(m, ServerMsg::JoinResponse { reason: 0, afs: 6 });
        let m = ServerMsg::decode(&frame_payload("0c00000000000601000002000000")).unwrap();
        assert_eq!(m, ServerMsg::JoinResponse { reason: 0, afs: 0x2_0000_0106 });

        let m = ServerMsg::decode(&frame_payload("07000c000100000000")).unwrap();
        assert_eq!(m, ServerMsg::LoginDenied { reason: 1, message: String::new() });
        let m = ServerMsg::decode(&frame_payload("03000b0007")).unwrap();
        assert_eq!(m, ServerMsg::EnterWorldDenied { reason: 7 });
        let m = ServerMsg::decode(&frame_payload("05000d00000000")).unwrap();
        assert_eq!(m, ServerMsg::AccountWarned { source: 0, message: String::new() });
    }

    #[test]
    fn server_golden_queue_and_cookie() {
        let m = ServerMsg::decode(&frame_payload("0a0009000000000000000000")).unwrap();
        assert_eq!(m, ServerMsg::WorldQueue(WorldQueue::default()));

        let m = ServerMsg::decode(&frame_payload(
            "1e000a004d0000000100007fd7040100007fd7040100007fd7040100007fd704",
        ))
        .unwrap();
        let ServerMsg::WorldCookie(c) = m else { panic!("not a cookie") };
        assert_eq!(c.cookie, 77);
        assert_eq!(c.addr, SocketAddrV4::new(Ipv4Addr::LOCALHOST, 1239));

        let m = ServerMsg::decode(&frame_payload(
            "1e000a004e0000000901a8c0d7040901a8c0d7040901a8c0d7040901a8c0d704",
        ))
        .unwrap();
        let ServerMsg::WorldCookie(c) = m else { panic!("not a cookie") };
        assert_eq!(c.addr, SocketAddrV4::new(Ipv4Addr::new(192, 168, 1, 9), 1239));
    }

    #[test]
    fn server_golden_world_list() {
        let hex = concat!(
            "76000800020105004141456d750002050054c3a97374010200000000000000000002",
            "14000000020a0000000300426f6201021000000000000000000000000000000000000000000000000000",
            "0700000001ddccbbaa0500c39c6ec3af0401100000000000000000000000000000000000000000000000",
            "0000"
        );
        let ServerMsg::WorldList(list) = ServerMsg::decode(&frame_payload(hex)).unwrap() else {
            panic!("not a world list")
        };
        assert_eq!(list.worlds.len(), 2);
        assert_eq!(list.worlds[0].name, "AAEmu");
        assert!(!list.worlds[0].online);
        assert_eq!(list.worlds[1].id, 2);
        assert_eq!(list.worlds[1].name, "Tést");
        assert!(list.worlds[1].online);
        assert_eq!(list.worlds[1].congestion, congestion::HIGH);
        assert_eq!(list.characters.len(), 2);
        assert_eq!(
            list.characters[0],
            LoginCharacter {
                account_id: 20,
                world_id: 2,
                id: 10,
                name: "Bob".into(),
                race: 1,
                gender: 2
            }
        );
        assert_eq!(list.characters[1].name, "Ünï");
        assert_eq!((list.character_count(2), list.character_count(1), list.character_count(9)), (1, 1, 0));

        let empty = ServerMsg::decode(&frame_payload("040008000000")).unwrap();
        assert_eq!(empty, ServerMsg::WorldList(WorldList::default()));
    }

    #[test]
    fn unknown_opcodes_are_reported_not_fatal() {
        assert_eq!(
            ServerMsg::decode(&[0x04, 0x00, 1, 2, 3]).unwrap(),
            ServerMsg::Unsupported { opcode: 4 }
        );
        assert!(matches!(
            ServerMsg::decode(&[0x03, 0x00, 1]),
            Err(WireError::Eof { .. })
        ));
    }
}
