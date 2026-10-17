//! Friends / blocked users. Lists arrive at login; the original paginated Lua view is local.

use crate::opcodes::{cs, sc};
use crate::session::Event;
use crate::wire::{Pos, Reader, Result, WireError, Writer};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FriendInfo {
    pub id: u32,
    pub name: String,
    pub race: u8,
    pub level: u8,
    pub hp: i32,
    pub abilities: [u8; 3],
    pub pos: Pos,
    pub zone_id: u32,
    pub type_id: u32,
    pub in_party: bool,
    pub online: bool,
    pub last_world_leave_time: i64,
}

impl FriendInfo {
    fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            id: r.u32()?,
            name: r.string()?,
            race: r.u8()?,
            level: r.u8()?,
            hp: r.i32()?,
            abilities: [r.u8()?, r.u8()?, r.u8()?],
            pos: r.long_position()?,
            zone_id: r.u32()?,
            type_id: r.u32()?,
            in_party: r.bool()?,
            online: r.bool()?,
            last_world_leave_time: r.i64()?,
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BlockedUser {
    pub id: u32,
    pub name: String,
}

impl BlockedUser {
    fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self { id: r.u32()?, name: r.string()? })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FriendCommand {
    Add { name: String },
    Remove { name: String },
    Block { name: String },
    Unblock { name: String },
}

impl FriendCommand {
    pub fn name(&self) -> &str {
        match self {
            Self::Add { name } | Self::Remove { name } | Self::Block { name } | Self::Unblock { name } => name,
        }
    }

    pub fn validate(&self) -> std::result::Result<(), &'static str> {
        let name = self.name();
        if name.trim().is_empty() || name.len() > 128 || name.contains('\0') {
            Err("invalid relationship character name")
        } else {
            Ok(())
        }
    }

    pub fn opcode(&self) -> u16 {
        match self {
            Self::Add { .. } => cs::CSAddFriendPacket,
            Self::Remove { .. } => cs::CSDeleteFriendPacket,
            Self::Block { .. } => cs::CSAddBlockedUserPacket,
            Self::Unblock { .. } => cs::CSDeleteBlockedUserPacket,
        }
    }

    pub fn write(&self, w: &mut Writer) {
        w.string(self.name());
    }

    fn matches(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other) && self.name().eq_ignore_ascii_case(other.name())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum FriendEvent {
    List {
        total: u32,
        friends: Vec<FriendInfo>,
    },
    BlockedList {
        total: u32,
        blocked: Vec<BlockedUser>,
    },
    Added {
        friend: FriendInfo,
        success: bool,
        error: i16,
    },
    Removed {
        id: u32,
        name: String,
        success: bool,
        error: i16,
    },
    StatusChanged(FriendInfo),
    Blocked {
        user: BlockedUser,
        success: bool,
        error: i16,
    },
    Unblocked {
        id: u32,
        name: String,
        success: bool,
        error: i16,
    },
    /// Local notification after the session successfully sends the request.
    Requested(FriendCommand),
    /// Local validation failure; no packet was sent.
    RequestRejected {
        command: FriendCommand,
        message: String,
    },
}

impl FriendEvent {
    pub fn handles(opcode: u16) -> bool {
        matches!(
            opcode,
            sc::SCFriendsPacket
                | sc::SCAddFriendPacket
                | sc::SCDeleteFriendPacket
                | sc::SCFriendStatusChangedPacket
                | sc::SCBlockedUsersPacket
                | sc::SCAddBlockedUserPacket
                | sc::SCDeleteBlockedUserPacket
        )
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::List { .. } => "SCFriends",
            Self::BlockedList { .. } => "SCBlockedUsers",
            Self::Added { .. } => "SCAddFriend",
            Self::Removed { .. } => "SCDeleteFriend",
            Self::StatusChanged(_) => "SCFriendStatusChanged",
            Self::Blocked { .. } => "SCAddBlockedUser",
            Self::Unblocked { .. } => "SCDeleteBlockedUser",
            Self::Requested(_) => "FriendRequested",
            Self::RequestRejected { .. } => "FriendRequestRejected",
        }
    }

    pub fn read(opcode: u16, r: &mut Reader) -> Result<Self> {
        Ok(match opcode {
            sc::SCFriendsPacket => {
                let (total, count) = counts(r, 53)?;
                let mut friends: Vec<FriendInfo> = Vec::with_capacity(count);
                for _ in 0..count {
                    let friend = FriendInfo::read(r)?;
                    if friend.id == 0 || friends.iter().any(|f| f.id == friend.id) {
                        return Err(WireError::Invalid("invalid or duplicate friend ID".into()));
                    }
                    friends.push(friend);
                }
                Self::List { total, friends }
            }
            sc::SCBlockedUsersPacket => {
                let (total, count) = counts(r, 6)?;
                let mut blocked: Vec<BlockedUser> = Vec::with_capacity(count);
                for _ in 0..count {
                    let user = BlockedUser::read(r)?;
                    if user.id == 0 || blocked.iter().any(|b| b.id == user.id) {
                        return Err(WireError::Invalid("invalid or duplicate blocked-user ID".into()));
                    }
                    blocked.push(user);
                }
                Self::BlockedList { total, blocked }
            }
            sc::SCAddFriendPacket => Self::Added { friend: FriendInfo::read(r)?, success: r.bool()?, error: r.i16()? },
            sc::SCDeleteFriendPacket => {
                let (id, success, name, error) = (r.u32()?, r.bool()?, r.string()?, r.i16()?);
                Self::Removed { id, name, success, error }
            }
            sc::SCFriendStatusChangedPacket => Self::StatusChanged(FriendInfo::read(r)?),
            sc::SCAddBlockedUserPacket => {
                Self::Blocked { user: BlockedUser::read(r)?, success: r.bool()?, error: r.i16()? }
            }
            sc::SCDeleteBlockedUserPacket => {
                let (id, success, name, error) = (r.u32()?, r.bool()?, r.string()?, r.i16()?);
                Self::Unblocked { id, name, success, error }
            }
            _ => return Err(WireError::Invalid(format!("not a friend opcode: {opcode:#x}"))),
        })
    }
}

fn counts(r: &mut Reader, minimum: usize) -> Result<(u32, usize)> {
    let (total, count) = (r.i32()?, r.i32()?);
    if total < 0 || count < 0 || count > total || count as usize > r.remaining() / minimum {
        return Err(WireError::Invalid("invalid relationship list count".into()));
    }
    Ok((total as u32, count as usize))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FriendFailure {
    pub command: FriendCommand,
    /// None for local validation; the unmodified server error code otherwise.
    pub code: Option<i16>,
    pub message: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "bevy", derive(bevy::ecs::resource::Resource))]
pub struct Friends {
    pub friends: Vec<FriendInfo>,
    pub blocked: Vec<BlockedUser>,
    pub total: u32,
    pub blocked_total: u32,
    pub loaded: bool,
    pub blocked_loaded: bool,
    /// Sent requests with no acknowledgement yet. The lane silently ignores some invalid operations.
    pub pending: Vec<FriendCommand>,
    pub last_error: Option<FriendFailure>,
}

impl Friends {
    pub fn friend(&self, id: u32) -> Option<&FriendInfo> {
        self.friends.iter().find(|f| f.id == id)
    }

    pub fn is_blocked(&self, name: &str) -> bool {
        self.blocked.iter().any(|b| b.name.eq_ignore_ascii_case(name))
    }

    /// The server has no paged-list request; filter and slice the login snapshot locally.
    pub fn page(&self, show_offline: bool, start: usize, count: usize) -> Vec<&FriendInfo> {
        self.friends.iter().filter(|f| show_offline || f.online).skip(start).take(count).collect()
    }

    fn complete(&mut self, command: FriendCommand, success: bool, error: i16) {
        self.pending.retain(|p| !p.matches(&command));
        self.last_error = (!success).then(|| FriendFailure {
            command,
            code: Some(error),
            message: format!("relationship request failed ({error})"),
        });
    }

    pub fn apply(&mut self, event: &Event) -> bool {
        let Event::Friend(event) = event else {
            if matches!(event, Event::LeftWorld { .. } | Event::Disconnected { .. } | Event::CharacterSelected(_)) {
                let changed = *self != Self::default();
                *self = Self::default();
                return changed;
            }
            if matches!(event, Event::EnteredWorld) {
                // The lane omits packets for empty lists. End of initial state confirms both empties.
                let changed = !self.loaded || !self.blocked_loaded;
                self.loaded = true;
                self.blocked_loaded = true;
                return changed;
            }
            return false;
        };
        match event {
            FriendEvent::List { total, friends } => {
                self.friends = friends.clone();
                self.total = *total;
                self.loaded = true;
            }
            FriendEvent::BlockedList { total, blocked } => {
                self.blocked = blocked.clone();
                self.blocked_total = *total;
                self.blocked_loaded = true;
            }
            FriendEvent::Added { friend, success, error } => {
                self.complete(FriendCommand::Add { name: friend.name.clone() }, *success, *error);
                if *success && friend.id != 0 {
                    if let Some(old) = self.friends.iter_mut().find(|f| f.id == friend.id) {
                        *old = friend.clone();
                    } else {
                        self.friends.push(friend.clone());
                        self.total += 1;
                    }
                }
            }
            FriendEvent::Removed { id, name, success, error } => {
                self.complete(FriendCommand::Remove { name: name.clone() }, *success, *error);
                if *success {
                    let before = self.friends.len();
                    self.friends.retain(|f| f.id != *id);
                    self.total = self.total.saturating_sub((before - self.friends.len()) as u32);
                }
            }
            FriendEvent::StatusChanged(friend) => {
                let Some(old) = self.friends.iter_mut().find(|f| f.id == friend.id) else { return false };
                if old == friend {
                    return false;
                }
                *old = friend.clone();
            }
            FriendEvent::Blocked { user, success, error } => {
                self.complete(FriendCommand::Block { name: user.name.clone() }, *success, *error);
                if *success && user.id != 0 {
                    if let Some(old) = self.blocked.iter_mut().find(|b| b.id == user.id) {
                        *old = user.clone();
                    } else {
                        self.blocked.push(user.clone());
                        self.blocked_total += 1;
                    }
                }
            }
            FriendEvent::Unblocked { id, name, success, error } => {
                self.complete(FriendCommand::Unblock { name: name.clone() }, *success, *error);
                if *success {
                    let before = self.blocked.len();
                    self.blocked.retain(|b| b.id != *id);
                    self.blocked_total = self.blocked_total.saturating_sub((before - self.blocked.len()) as u32);
                }
            }
            FriendEvent::Requested(command) => {
                if !self.pending.iter().any(|p| p.matches(command)) {
                    self.pending.push(command.clone());
                }
                self.last_error = None;
            }
            FriendEvent::RequestRejected { command, message } => {
                self.pending.retain(|p| !p.matches(command));
                self.last_error =
                    Some(FriendFailure { command: command.clone(), code: None, message: message.clone() });
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{ClientMsg, ServerMsg};

    fn info_bytes(w: &mut Writer, id: u32, online: bool) {
        w.u32(id)
            .string("Bob")
            .u8(1)
            .u8(10)
            .i32(123)
            .u8(1)
            .u8(2)
            .u8(3)
            .long_position(Pos::new(10.5, -3.25, 30.0))
            .u32(100)
            .u32(7)
            .bool(true)
            .bool(online)
            .i64(123456789);
    }

    fn batch(packets: &[(u16, Vec<u8>)]) -> Vec<ServerMsg> {
        let mut plain = Writer::new();
        for (opcode, body) in packets {
            plain.u16(0).u16(*opcode).raw(body);
        }
        let plain = plain.finish();
        let mut compressed = Writer::new();
        compressed.u8(0xdd).u8(4).u16(packets.len() as u16);
        compressed.u8(1).u16(plain.len() as u16).u16(!(plain.len() as u16)).raw(&plain);
        let ServerMsg::Batch(batch) = ServerMsg::decode(&compressed.finish()).unwrap() else { panic!() };
        batch
    }

    #[test]
    fn friend_requests_match_lane_string_readers() {
        for (command, opcode) in [
            (FriendCommand::Add { name: "Bob".into() }, 0x104),
            (FriendCommand::Remove { name: "Bob".into() }, 0x105),
            (FriendCommand::Block { name: "Bob".into() }, 0x107),
            (FriendCommand::Unblock { name: "Bob".into() }, 0x108),
        ] {
            assert!(command.validate().is_ok());
            let packet = ClientMsg::Friend(command);
            assert_eq!(packet.opcode(), opcode);
            assert_eq!(packet.encode_body(), vec![3, 0, b'B', b'o', b'b']);
        }
        for name in ["".into(), " ".into(), "x".repeat(129), "A\0B".into()] {
            assert!(FriendCommand::Add { name }.validate().is_err());
        }
    }

    #[test]
    fn every_relationship_packet_leaves_following_dd04_opcode_aligned() {
        let mut list = Writer::new();
        list.i32(2).i32(2);
        info_bytes(&mut list, 3, true);
        info_bytes(&mut list, 4, false);
        let mut add = Writer::new();
        info_bytes(&mut add, 3, true);
        add.bool(true).i16(0);
        let mut remove = Writer::new();
        remove.u32(3).bool(false).string("Bob").i16(42);
        let mut status = Writer::new();
        info_bytes(&mut status, 3, false);
        let mut blocked = Writer::new();
        blocked.i32(1).i32(1).u32(4).string("Bob");
        let mut block = Writer::new();
        block.u32(4).string("Bob").bool(true).i16(0);
        let mut unblock = Writer::new();
        unblock.u32(4).bool(true).string("Bob").i16(0);
        for (opcode, bytes) in [
            (sc::SCFriendsPacket, list.finish()),
            (sc::SCAddFriendPacket, add.finish()),
            (sc::SCDeleteFriendPacket, remove.finish()),
            (sc::SCFriendStatusChangedPacket, status.finish()),
            (sc::SCBlockedUsersPacket, blocked.finish()),
            (sc::SCAddBlockedUserPacket, block.finish()),
            (sc::SCDeleteBlockedUserPacket, unblock.finish()),
        ] {
            let messages = batch(&[(opcode, bytes), (sc::SCOnOffSnowPacket, vec![1])]);
            assert_eq!(messages.len(), 2, "{opcode:#x}");
            assert!(matches!(&messages[0], ServerMsg::Friend(_)));
            assert_eq!(messages[1], ServerMsg::Snow(true));
        }
    }

    #[test]
    fn friend_long_coordinates_and_status_fields_are_not_packed_positions() {
        let mut bytes = Writer::new();
        info_bytes(&mut bytes, 3, false);
        let bytes = bytes.finish();
        assert_eq!(bytes.len(), 56);
        let mut r = Reader::new(&bytes);
        let friend = FriendInfo::read(&mut r).unwrap();
        assert!(r.is_empty());
        assert_eq!(friend.pos, Pos::new(10.5, -3.25, 30.0));
        assert_eq!((friend.id, friend.hp, friend.zone_id, friend.type_id), (3, 123, 100, 7));
        assert_eq!(friend.abilities, [1, 2, 3]);
        assert!(friend.in_party && !friend.online);
        assert_eq!(friend.last_world_leave_time, 123456789);
    }

    #[test]
    fn invalid_counts_duplicates_and_truncation_are_rejected() {
        for opcode in [sc::SCFriendsPacket, sc::SCBlockedUsersPacket] {
            for (total, count) in [(-1, 0), (0, -1), (0, 1), (1, i32::MAX), (1, 1)] {
                let mut bytes = Writer::new();
                bytes.i32(total).i32(count);
                assert!(FriendEvent::read(opcode, &mut Reader::new(&bytes.finish())).is_err());
            }
        }
        let mut bytes = Writer::new();
        bytes.i32(2).i32(2);
        info_bytes(&mut bytes, 3, true);
        info_bytes(&mut bytes, 3, false);
        assert!(FriendEvent::read(sc::SCFriendsPacket, &mut Reader::new(&bytes.finish())).is_err());
        let mut bytes = Writer::new();
        bytes.i32(2).i32(2).u32(4).string("A").u32(4).string("B");
        assert!(FriendEvent::read(sc::SCBlockedUsersPacket, &mut Reader::new(&bytes.finish())).is_err());
    }

    #[test]
    fn only_successful_acknowledgements_change_membership_and_pending_is_correlated() {
        let mut state = Friends::default();
        let command = FriendCommand::Add { name: "bob".into() };
        state.apply(&Event::Friend(FriendEvent::Requested(command.clone())));
        assert!(state.friends.is_empty() && state.pending == vec![command]);
        let friend = FriendInfo { id: 3, name: "Bob".into(), online: true, ..FriendInfo::default() };
        state.apply(&Event::Friend(FriendEvent::Added { friend: friend.clone(), success: false, error: 42 }));
        assert!(state.friends.is_empty() && state.pending.is_empty());
        assert_eq!(state.last_error.as_ref().unwrap().code, Some(42));
        state.apply(&Event::Friend(FriendEvent::Added { friend: friend.clone(), success: true, error: 0 }));
        state.apply(&Event::Friend(FriendEvent::Added { friend: friend.clone(), success: true, error: 0 }));
        assert_eq!((state.total, state.friends.len()), (1, 1));
        assert!(state.last_error.is_none());
        state.apply(&Event::Friend(FriendEvent::StatusChanged(FriendInfo { online: false, ..friend.clone() })));
        assert!(!state.friend(3).unwrap().online && state.page(false, 0, 10).is_empty());
        assert_eq!(state.page(true, 0, 10).len(), 1);
        state.apply(&Event::Friend(FriendEvent::Removed { id: 3, name: "Bob".into(), success: false, error: 7 }));
        assert_eq!(state.total, 1);
        state.apply(&Event::Friend(FriendEvent::Removed { id: 3, name: "Bob".into(), success: true, error: 0 }));
        assert_eq!((state.total, state.friends.len()), (0, 0));
        let user = BlockedUser { id: 4, name: "Alice".into() };
        state.apply(&Event::Friend(FriendEvent::Blocked { user, success: true, error: 0 }));
        assert!(state.is_blocked("alice"));
        state.apply(&Event::Friend(FriendEvent::Unblocked { id: 4, name: "Alice".into(), success: false, error: 7 }));
        assert!(state.is_blocked("Alice"));
        state.apply(&Event::Friend(FriendEvent::Unblocked { id: 4, name: "Alice".into(), success: true, error: 0 }));
        assert_eq!((state.blocked_total, state.blocked.len()), (0, 0));
        assert!(!state.apply(&Event::Friend(FriendEvent::StatusChanged(friend))));
        assert!(state.friends.is_empty(), "presence does not add a removed friend again");
        state.apply(&Event::Friend(FriendEvent::RequestRejected {
            command: FriendCommand::Block { name: String::new() },
            message: "invalid name".into(),
        }));
        assert_eq!(state.last_error.as_ref().unwrap().code, None);
        assert!(state.blocked.is_empty() && state.pending.is_empty());
    }

    #[test]
    fn login_snapshot_survives_spawn_and_empty_login_and_logout_are_known() {
        let mut state = Friends::default();
        state.apply(&Event::Friend(FriendEvent::List {
            total: 1,
            friends: vec![FriendInfo { id: 3, name: "Bob".into(), ..FriendInfo::default() }],
        }));
        state.apply(&Event::PlayerSpawn(Box::new(crate::session::PlayerSpawn {
            obj_id: 5,
            state: crate::model::CharacterState::default(),
            unit: crate::state::tests::unit(5),
        })));
        assert_eq!(state.friends.len(), 1);
        state.apply(&Event::EnteredWorld);
        assert!(state.loaded && state.blocked_loaded && state.blocked.is_empty());
        state.apply(&Event::LeftWorld { target: 0 });
        assert_eq!(state, Friends::default());
        state.apply(&Event::EnteredWorld);
        assert!(state.loaded && state.blocked_loaded && state.friends.is_empty());
    }
}
