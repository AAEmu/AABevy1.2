//! Game server messages.
//!
//! Every frame payload starts with `0xDD` and a level byte. Level 1 (`CS*` / `SC*`) continues with
//! `[hash][count][u16 opcode][body]`; level 2 is the CryNetwork proxy channel (`[u16 opcode][body]`);
//! level 4 is a compressed batch of server packets. The 1.2 protocol is neither encrypted nor
//! compressed otherwise, and the server verifies neither `hash` nor `count`.

use crate::model::{
    ChatMessage, Character, CharacterState, CreateCharacter, Doodad, Movement, Unit,
};
use crate::opcodes::{cs, pp, sc};
use crate::wire::{Pos, Reader, Result, WireError, Writer};

pub const MAGIC: u8 = 0xDD;
pub const LEVEL_GAME: u8 = 1;
pub const LEVEL_PROXY: u8 = 2;
pub const LEVEL_COMPRESSED: u8 = 4;

/// Where `CSLeaveWorld` leads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeaveTarget {
    /// Log out and close the game.
    Quit = 0,
    /// Back to the character list on the same world.
    CharacterSelect = 1,
    /// Back to the world list (needs a new login connection, see `SCReconnectAuth`).
    ServerSelect = 2,
}

impl LeaveTarget {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Quit),
            1 => Some(Self::CharacterSelect),
            2 => Some(Self::ServerSelect),
            _ => None,
        }
    }
}

/// Client side visual options sent with `CSSpawnCharacter`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VisualOptions {
    pub helmet: bool,
    pub back_holdable: bool,
    pub cosplay: bool,
    pub cosplay_backpack: bool,
}

impl Default for VisualOptions {
    fn default() -> Self {
        Self {
            helmet: true,
            back_holdable: true,
            cosplay: true,
            cosplay_backpack: true,
        }
    }
}

/// Messages the client sends to the game server.
#[derive(Clone, Debug, PartialEq)]
pub enum ClientMsg {
    /// `X2EnterWorldPacket`: opens the session with the login cookie.
    EnterWorld { account_id: u32, cookie: u32 },
    /// Proxy `FinishStatePacket`: acknowledges `ChangeState`.
    FinishState(i32),
    /// Proxy `PingPacket`; the server drops silent connections after 30 s.
    Ping { time_ms: i64 },
    ListCharacters,
    CreateCharacter(CreateCharacter),
    DeleteCharacter { id: u32 },
    CancelDeleteCharacter { id: u32 },
    SelectCharacter { id: u32 },
    SpawnCharacter(VisualOptions),
    NotifyInGame,
    NotifyInGameCompleted,
    Move { obj_id: u32, movement: Movement },
    Chat { chat_type: i16, target: String, message: String },
    ChangeTarget { obj_id: u32 },
    /// Answer to `SCTeleportUnit`; the server then accepts movement again.
    TeleportEnded { pos: Pos },
    LeaveWorld(LeaveTarget),
    CancelLeaveWorld,
}

impl ClientMsg {
    pub fn level(&self) -> u8 {
        match self {
            Self::FinishState(_) | Self::Ping { .. } => LEVEL_PROXY,
            _ => LEVEL_GAME,
        }
    }

    pub fn opcode(&self) -> u16 {
        match self {
            Self::EnterWorld { .. } => cs::X2EnterWorldPacket,
            Self::FinishState(_) => pp::FinishStatePacket,
            Self::Ping { .. } => pp::PingPacket,
            Self::ListCharacters => cs::CSListCharacterPacket,
            Self::CreateCharacter(_) => cs::CSCreateCharacterPacket,
            Self::DeleteCharacter { .. } => cs::CSDeleteCharacterPacket,
            Self::CancelDeleteCharacter { .. } => cs::CSCancelCharacterDeletePacket,
            Self::SelectCharacter { .. } => cs::CSSelectCharacterPacket,
            Self::SpawnCharacter(_) => cs::CSSpawnCharacterPacket,
            Self::NotifyInGame => cs::CSNotifyInGamePacket,
            Self::NotifyInGameCompleted => cs::CSNotifyInGameCompletedPacket,
            Self::Move { .. } => cs::CSMoveUnitPacket,
            Self::Chat { .. } => cs::CSSendChatMessagePacket,
            Self::ChangeTarget { .. } => cs::CSChangeTargetPacket,
            Self::TeleportEnded { .. } => cs::CSTeleportEndedPacket,
            Self::LeaveWorld(_) => cs::CSLeaveWorldPacket,
            Self::CancelLeaveWorld => cs::CSCancelLeaveWorldPacket,
        }
    }

    pub fn name(&self) -> &'static str {
        if self.level() == LEVEL_PROXY {
            pp::name(self.opcode())
        } else {
            cs::name(self.opcode())
        }
    }

    pub fn encode_body(&self) -> Vec<u8> {
        let mut w = Writer::new();
        match self {
            Self::EnterWorld { account_id, cookie } => {
                w.u32(0) // pFrom
                    .u32(0) // pTo
                    .u32(*account_id)
                    .u32(*cookie)
                    .i32(0) // zoneId
                    .u8(0) // tb
                    .u64(0); // revision
            }
            Self::FinishState(state) => {
                w.i32(*state);
            }
            Self::Ping { time_ms } => {
                w.i64(*time_ms).i64(*time_ms).u32(*time_ms as u32);
            }
            Self::ListCharacters => {
                w.i32(0).blob(&[]);
            }
            Self::CreateCharacter(c) => c.write(&mut w),
            Self::DeleteCharacter { id } | Self::CancelDeleteCharacter { id } => {
                w.u32(*id);
            }
            Self::ChangeTarget { obj_id } => {
                w.bc(*obj_id);
            }
            Self::SelectCharacter { id } => {
                w.u32(*id).bool(false).u8(0);
            }
            Self::SpawnCharacter(v) => {
                // All five option groups present: stp (6 bytes), helmet, back holdable, cosplay,
                // cosplay backpack.
                w.u8(0x1F)
                    .raw(&[0u8; 6])
                    .bool(v.helmet)
                    .bool(v.back_holdable)
                    .bool(v.cosplay)
                    .bool(v.cosplay_backpack);
            }
            Self::NotifyInGame | Self::NotifyInGameCompleted | Self::CancelLeaveWorld => {}
            Self::Move { obj_id, movement } => {
                w.bc(*obj_id);
                movement.write(&mut w);
            }
            Self::Chat { chat_type, target, message } => {
                w.i16(*chat_type)
                    .i16(0)
                    .i32(0)
                    .string(target)
                    .string(message)
                    .u8(0) // language
                    .i32(0); // ability
            }
            Self::TeleportEnded { pos } => {
                w.i64(crate::wire::coord_to_long(pos.x))
                    .i64(crate::wire::coord_to_long(pos.y))
                    .f32(pos.z)
                    .raw(&[0u8; 12])
                    .f32(1.0); // identity rotation
            }
            Self::LeaveWorld(target) => {
                w.u8(*target as u8);
            }
        }
        w.finish()
    }

    /// The frame payload (without the `u16` length). `counter` is the level-1 packet counter.
    pub fn encode(&self, counter: u8) -> Vec<u8> {
        let mut out = vec![MAGIC, self.level()];
        if self.level() == LEVEL_GAME {
            out.extend([0, counter]);
        }
        out.extend(self.opcode().to_le_bytes());
        out.extend(self.encode_body());
        out
    }
}

/// Messages the game server sends that this client understands.
#[derive(Clone, Debug, PartialEq)]
pub enum ServerMsg {
    // -- proxy channel
    /// CryNetwork state machine: answer with `FinishState` of the same state.
    ChangeState(i32),
    Pong,
    // -- lobby
    EnterWorldResponse { reason: i16, gm: bool, stream_token: u32, stream_port: u16, world_time: i64 },
    /// The server asks the client to reconnect to the login server (return to world select).
    ReconnectAuth { token: u32 },
    SlotCount(u8),
    AccountInfo { pay_method: i32, pay_location: i32, pay_start: i64, pay_end: i64 },
    PremiumPoint { point: i32, old_grade: u8, grade: u8 },
    /// Congestion per race (index = race id) for character creation.
    RaceCongestion([u8; 9]),
    CharacterList { last: bool, characters: Vec<Character> },
    CharacterCreated(Box<Character>),
    CharacterCreationFailed { reason: u8 },
    CharacterDeleteResponse { id: u32, status: u8, delete_request_time: i64, delete_time: i64 },
    CancelCharacterDeleteResponse { id: u32, status: u8 },
    CharacterDeleted { id: u32, name: String },
    // -- entering the world
    CharacterState(Box<CharacterState>),
    // -- world
    UnitState(Box<Unit>),
    UnitsRemoved(Vec<u32>),
    UnitMoved { obj_id: u32, movement: Movement },
    UnitPoints { obj_id: u32, hp: i32, mp: i32 },
    UnitDeath { obj_id: u32 },
    Teleport { reason: u8, error: i16, pos: Pos },
    DoodadsCreated(Vec<Doodad>),
    DoodadsRemoved(Vec<u32>),
    DoodadPhaseChanged { obj_id: u32, func_group_id: u32 },
    Chat(ChatMessage),
    ErrorMsg { code: i16, kind: u32, notify: bool },
    Kicked { reason: u8, message: String },
    // -- leaving
    PrepareLeaveWorld { time_ms: i32, target: u8, idle_kick: bool },
    LeaveWorldGranted { target: u8 },
    LeaveWorldCanceled,
    // -- anything else
    /// A compressed batch (`DD04`) of server packets; not interpreted.
    Compressed { count: u16 },
    Unhandled { level: u8, opcode: u16, len: usize },
}

impl ServerMsg {
    pub fn name(&self) -> &'static str {
        match self {
            Self::ChangeState(_) => "ChangeState",
            Self::Pong => "Pong",
            Self::EnterWorldResponse { .. } => "X2EnterWorldResponse",
            Self::ReconnectAuth { .. } => "SCReconnectAuth",
            Self::SlotCount(_) => "SCGetSlotCount",
            Self::AccountInfo { .. } => "SCAccountInfo",
            Self::PremiumPoint { .. } => "SCUpdatePremiumPoint",
            Self::RaceCongestion(_) => "SCRaceCongestion",
            Self::CharacterList { .. } => "SCCharacterList",
            Self::CharacterCreated(_) => "SCCreateCharacterResponse",
            Self::CharacterCreationFailed { .. } => "SCCharacterCreationFailed",
            Self::CharacterDeleteResponse { .. } => "SCDeleteCharacterResponse",
            Self::CancelCharacterDeleteResponse { .. } => "SCCancelCharacterDeleteResponse",
            Self::CharacterDeleted { .. } => "SCCharacterDeleted",
            Self::CharacterState(_) => "SCCharacterState",
            Self::UnitState(_) => "SCUnitState",
            Self::UnitsRemoved(_) => "SCUnitsRemoved",
            Self::UnitMoved { .. } => "SCOneUnitMovement",
            Self::UnitPoints { .. } => "SCUnitPoints",
            Self::UnitDeath { .. } => "SCUnitDeath",
            Self::Teleport { .. } => "SCTeleportUnit",
            Self::DoodadsCreated(_) => "SCDoodad(s)Created",
            Self::DoodadsRemoved(_) => "SCDoodad(s)Removed",
            Self::DoodadPhaseChanged { .. } => "SCDoodadPhaseChanged",
            Self::Chat(_) => "SCChatMessage",
            Self::ErrorMsg { .. } => "SCErrorMsg",
            Self::Kicked { .. } => "SCKicked",
            Self::PrepareLeaveWorld { .. } => "SCPrepareLeaveWorld",
            Self::LeaveWorldGranted { .. } => "SCLeaveWorldGranted",
            Self::LeaveWorldCanceled => "SCLeaveWorldCanceled",
            Self::Compressed { .. } => "CompressedGamePackets",
            Self::Unhandled { .. } => "unhandled",
        }
    }

    /// Decodes a frame payload (starting at the `0xDD` byte).
    pub fn decode(frame: &[u8]) -> Result<Self> {
        let mut r = Reader::new(frame);
        let magic = r.u8()?;
        if magic != MAGIC {
            return Err(WireError::Invalid(format!("game frame starts with {magic:#04x}")));
        }
        let level = r.u8()?;
        match level {
            LEVEL_GAME => {
                r.skip(2)?; // hash, count
                let opcode = r.u16()?;
                Self::decode_game(opcode, &mut r)
            }
            LEVEL_PROXY => {
                let opcode = r.u16()?;
                Self::decode_proxy(opcode, &mut r)
            }
            LEVEL_COMPRESSED => Ok(Self::Compressed { count: r.u16()? }),
            level => Ok(Self::Unhandled {
                level,
                opcode: 0,
                len: frame.len(),
            }),
        }
    }

    fn decode_proxy(opcode: u16, r: &mut Reader) -> Result<Self> {
        Ok(match opcode {
            pp::ChangeStatePacket => Self::ChangeState(r.i32()?),
            pp::PongPacket => Self::Pong,
            opcode => Self::Unhandled {
                level: LEVEL_PROXY,
                opcode,
                len: r.remaining(),
            },
        })
    }

    fn decode_game(opcode: u16, r: &mut Reader) -> Result<Self> {
        Ok(match opcode {
            sc::X2EnterWorldResponsePacket => Self::EnterWorldResponse {
                reason: r.i16()?,
                gm: r.bool()?,
                stream_token: r.u32()?,
                stream_port: r.u16()?,
                world_time: r.i64()?,
            },
            sc::SCReconnectAuthPacket => Self::ReconnectAuth { token: r.u32()? },
            sc::SCGetSlotCountPacket => Self::SlotCount(r.u8()?),
            sc::SCAccountInfoPacket => Self::AccountInfo {
                pay_method: r.i32()?,
                pay_location: r.i32()?,
                pay_start: r.i64()?,
                pay_end: r.i64()?,
            },
            sc::SCUpdatePremiumPointPacket => Self::PremiumPoint {
                point: r.i32()?,
                old_grade: r.u8()?,
                grade: r.u8()?,
            },
            sc::SCRaceCongestionPacket => {
                let mut c = [0u8; 9];
                for v in &mut c {
                    *v = r.u8()?;
                }
                Self::RaceCongestion(c)
            }
            sc::SCCharacterListPacket => {
                let last = r.bool()?;
                let n = r.u8()?;
                let mut characters = Vec::with_capacity(n as usize);
                for _ in 0..n {
                    characters.push(Character::read(r)?);
                }
                Self::CharacterList { last, characters }
            }
            sc::SCCreateCharacterResponsePacket => {
                Self::CharacterCreated(Box::new(Character::read(r)?))
            }
            sc::SCCharacterCreationFailedPacket => Self::CharacterCreationFailed { reason: r.u8()? },
            sc::SCDeleteCharacterResponsePacket => Self::CharacterDeleteResponse {
                id: r.u32()?,
                status: r.u8()?,
                delete_request_time: r.i64()?,
                delete_time: r.i64()?,
            },
            sc::SCCancelCharacterDeleteResponsePacket => Self::CancelCharacterDeleteResponse {
                id: r.u32()?,
                status: r.u8()?,
            },
            sc::SCCharacterDeletedPacket => Self::CharacterDeleted {
                id: r.u32()?,
                name: r.string()?,
            },
            sc::SCCharacterStatePacket => Self::CharacterState(Box::new(CharacterState::read(r)?)),
            sc::SCUnitStatePacket => Self::UnitState(Box::new(Unit::read(r)?)),
            sc::SCUnitsRemovedPacket => {
                let n = r.u16()?;
                let mut ids = Vec::with_capacity(n as usize);
                for _ in 0..n {
                    ids.push(r.bc()?);
                }
                Self::UnitsRemoved(ids)
            }
            sc::SCOneUnitMovementPacket => Self::UnitMoved {
                obj_id: r.bc()?,
                movement: Movement::read(r)?,
            },
            sc::SCUnitPointsPacket => Self::UnitPoints {
                obj_id: r.bc()?,
                hp: r.i32()? / 100,
                mp: r.i32()? / 100,
            },
            sc::SCUnitDeathPacket => Self::UnitDeath { obj_id: r.bc()? },
            sc::SCTeleportUnitPacket => Self::Teleport {
                reason: r.u8()?,
                error: r.i16()?,
                pos: r.position()?,
            },
            sc::SCDoodadCreatedPacket => Self::DoodadsCreated(vec![Doodad::read(r)?]),
            sc::SCDoodadsCreatedPacket => {
                let n = r.u8()?;
                let mut doodads = Vec::with_capacity(n as usize);
                for _ in 0..n {
                    doodads.push(Doodad::read(r)?);
                }
                Self::DoodadsCreated(doodads)
            }
            sc::SCDoodadRemovedPacket => Self::DoodadsRemoved(vec![r.bc()?]),
            sc::SCDoodadsRemovedPacket => {
                let n = r.u16()?;
                r.skip(1)?; // last
                let mut ids = Vec::with_capacity(n as usize);
                for _ in 0..n {
                    ids.push(r.bc()?);
                    r.skip(1)?;
                }
                Self::DoodadsRemoved(ids)
            }
            sc::SCDoodadPhaseChangedPacket => Self::DoodadPhaseChanged {
                obj_id: r.bc()?,
                func_group_id: r.u32()?,
            },
            sc::SCChatMessagePacket => Self::Chat(ChatMessage::read(r)?),
            sc::SCErrorMsgPacket => {
                let code = r.i16()?;
                r.skip(2)?;
                Self::ErrorMsg {
                    code,
                    kind: r.u32()?,
                    notify: r.bool()?,
                }
            }
            sc::SCKickedPacket => Self::Kicked {
                reason: r.u8()?,
                message: r.string()?,
            },
            sc::SCPrepareLeaveWorldPacket => Self::PrepareLeaveWorld {
                time_ms: r.i32()?,
                target: r.u8()?,
                idle_kick: r.bool()?,
            },
            sc::SCLeaveWorldGrantedPacket => Self::LeaveWorldGranted { target: r.u8()? },
            sc::SCLeaveWorldCanceledPacket => Self::LeaveWorldCanceled,
            opcode => Self::Unhandled {
                level: LEVEL_GAME,
                opcode,
                len: r.remaining(),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{race, gender, slot};

    #[test]
    fn level1_header_matches_the_server_layout() {
        // `[dd][01][hash][count][u16 opcode][body]`, see the server's `net::packet` tests.
        let m = ClientMsg::NotifyInGame.encode(7);
        assert_eq!(m, [0xDD, 0x01, 0x00, 0x07, 0x29, 0x00]);
        // Proxy packets have no hash / count.
        assert_eq!(ClientMsg::FinishState(3).encode(7), [0xDD, 0x02, 0x01, 0x00, 3, 0, 0, 0]);
    }

    #[test]
    fn client_bodies_match_the_testclient() {
        // X2EnterWorld: pFrom, pTo, account, cookie, zone, tb, revision.
        let b = ClientMsg::EnterWorld { account_id: 3, cookie: 4 }.encode_body();
        assert_eq!(b.len(), 4 * 4 + 4 + 1 + 8);
        assert_eq!(&b[8..16], &[3, 0, 0, 0, 4, 0, 0, 0]);
        // CSListCharacter: i32 size, empty blob.
        assert_eq!(ClientMsg::ListCharacters.encode_body(), [0, 0, 0, 0, 0, 0]);
        // CSSelectCharacter: id, gm flag, unknown.
        assert_eq!(ClientMsg::SelectCharacter { id: 9 }.encode_body(), [9, 0, 0, 0, 0, 0]);
        // CSSpawnCharacter with every option: flag 0x1F, 6 stp bytes, four bools.
        let mut want = vec![0x1F];
        want.extend([0u8; 6]);
        want.extend([1, 1, 1, 1]);
        assert_eq!(ClientMsg::SpawnCharacter(VisualOptions::default()).encode_body(), want);
        // CSLeaveWorld: the target byte.
        assert_eq!(ClientMsg::LeaveWorld(LeaveTarget::CharacterSelect).encode_body(), [1]);
        assert_eq!(ClientMsg::DeleteCharacter { id: 258 }.encode_body(), [2, 1, 0, 0]);
        assert_eq!(ClientMsg::ChangeTarget { obj_id: 0x010203 }.encode_body(), [3, 2, 1]);
        // CSSendChatMessage: type, sub, i32, target, message, language, ability.
        let b = ClientMsg::Chat {
            chat_type: 0,
            target: String::new(),
            message: "hi".into(),
        }
        .encode_body();
        assert_eq!(b, [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, b'h', b'i', 0, 0, 0, 0, 0]);
        // Ping: tPhy, ping, local.
        let b = ClientMsg::Ping { time_ms: 258 }.encode_body();
        assert_eq!(b.len(), 20);
        assert_eq!(&b[..2], &[2, 1]);
        // CSTeleportEnded: two fixed point longs, height, identity quaternion.
        let b = ClientMsg::TeleportEnded { pos: Pos::new(1.0, 2.0, 3.0) }.encode_body();
        assert_eq!(b.len(), 8 + 8 + 4 + 12 + 4);
        assert_eq!(&b[b.len() - 4..], &[0, 0, 0x80, 0x3f]);
    }

    #[test]
    fn create_character_message_carries_the_request() {
        let req = CreateCharacter::new("Bob", race::ELF, gender::FEMALE);
        let b = ClientMsg::CreateCharacter(req).encode_body();
        assert_eq!(&b[..5], &[3, 0, b'B', b'o', b'b']);
        assert_eq!(&b[5..7], &[race::ELF, gender::FEMALE]);
    }

    fn game_frame(opcode: u16, body: &[u8]) -> Vec<u8> {
        let mut f = vec![MAGIC, LEVEL_GAME, 0, 0];
        f.extend(opcode.to_le_bytes());
        f.extend(body);
        f
    }

    #[test]
    fn server_frames_decode() {
        // Server layout: `[dd][01][00][00][u16 opcode][body]`.
        let mut w = Writer::new();
        w.i16(0).bool(false).u32(0xAABB).u16(27250).i64(1_700_000_000).u16(0).string("");
        let m = ServerMsg::decode(&game_frame(sc::X2EnterWorldResponsePacket, &w.finish())).unwrap();
        assert_eq!(
            m,
            ServerMsg::EnterWorldResponse {
                reason: 0,
                gm: false,
                stream_token: 0xAABB,
                stream_port: 27250,
                world_time: 1_700_000_000
            }
        );

        let m = ServerMsg::decode(&[MAGIC, LEVEL_PROXY, 0, 0, 5, 0, 0, 0]).unwrap();
        assert_eq!(m, ServerMsg::ChangeState(5));

        let m = ServerMsg::decode(&game_frame(sc::SCRaceCongestionPacket, &[0, 1, 2, 3, 0, 0, 0, 0, 0])).unwrap();
        assert_eq!(m, ServerMsg::RaceCongestion([0, 1, 2, 3, 0, 0, 0, 0, 0]));

        let m = ServerMsg::decode(&game_frame(sc::SCCharacterCreationFailedPacket, &[4])).unwrap();
        assert_eq!(m, ServerMsg::CharacterCreationFailed { reason: 4 });

        let m = ServerMsg::decode(&game_frame(sc::SCLeaveWorldGrantedPacket, &[1])).unwrap();
        assert_eq!(m, ServerMsg::LeaveWorldGranted { target: 1 });

        let mut w = Writer::new();
        w.i32(10_000).u8(0).bool(false);
        let m = ServerMsg::decode(&game_frame(sc::SCPrepareLeaveWorldPacket, &w.finish())).unwrap();
        assert_eq!(m, ServerMsg::PrepareLeaveWorld { time_ms: 10_000, target: 0, idle_kick: false });

        let mut w = Writer::new();
        w.u16(2).bc(0x101).bc(0x102);
        let m = ServerMsg::decode(&game_frame(sc::SCUnitsRemovedPacket, &w.finish())).unwrap();
        assert_eq!(m, ServerMsg::UnitsRemoved(vec![0x101, 0x102]));

        let mut w = Writer::new();
        w.bc(0x101).i32(5000).i32(200);
        let m = ServerMsg::decode(&game_frame(sc::SCUnitPointsPacket, &w.finish())).unwrap();
        assert_eq!(m, ServerMsg::UnitPoints { obj_id: 0x101, hp: 50, mp: 2 });

        let mut w = Writer::new();
        w.u16(1).bool(true).bc(0x303).bool(true);
        let m = ServerMsg::decode(&game_frame(sc::SCDoodadsRemovedPacket, &w.finish())).unwrap();
        assert_eq!(m, ServerMsg::DoodadsRemoved(vec![0x303]));

        let mut w = Writer::new();
        w.i16(95).i16(95).u32(0).bool(false);
        let m = ServerMsg::decode(&game_frame(sc::SCErrorMsgPacket, &w.finish())).unwrap();
        assert_eq!(m, ServerMsg::ErrorMsg { code: 95, kind: 0, notify: false });

        // Unknown packets are reported with their opcode, not an error.
        let m = ServerMsg::decode(&game_frame(0x1ff, &[1, 2, 3])).unwrap();
        assert_eq!(m, ServerMsg::Unhandled { level: LEVEL_GAME, opcode: 0x1ff, len: 3 });
        assert!(ServerMsg::decode(&[0x00, 0x01]).is_err());
        assert!(ServerMsg::decode(&game_frame(sc::SCUnitPointsPacket, &[1])).is_err());
    }

    #[test]
    fn character_list_chunk_with_one_character() {
        // A character with empty equipment, built field by field like the server's `Character.Write`.
        let mut w = Writer::new();
        w.bool(true).u8(1);
        w.u32(5).string("Ab").u8(1).u8(1).u8(1).i32(10).i32(10).u32(8).u32(0).string("").u32(0).u32(0);
        for _ in 0..slot::COUNT {
            w.u32(0);
        }
        w.u8(1).u8(11).u8(11).long_position(Pos::new(1.0, 2.0, 3.0));
        crate::model::ModelParams::default_face().write(&mut w);
        w.i16(0).i64(0).i16(0).i64(0).i32(0).i64(0).i32(0).i64(0).i64(0).i64(0).i16(0).i32(0).i16(0);
        w.i64(0).i64(0).i64(0).i32(0).i64(0).i64(0).i64(0).bool(false).i32(0).i32(0).i32(0).i64(0).u8(0);
        let m = ServerMsg::decode(&game_frame(sc::SCCharacterListPacket, &w.finish())).unwrap();
        let ServerMsg::CharacterList { last, characters } = m else { panic!("not a list") };
        assert!(last);
        assert_eq!(characters.len(), 1);
        assert_eq!(characters[0].name, "Ab");
        assert_eq!(characters[0].zone_id, 8);
    }
}
