//! The portal book (`CharacterPortals`), summoned portals and the portal / teleport requests.
//!
//! Server to client (the lane's `PortalWire` = `Portal.Write`, `SCCharacterPortals` ... in
//! `aaemu-game/src/systems/npcs/portal/packets.rs` and `packets/world.rs`):
//!
//! * `SCCharacterPortals` (0x05b): `i32` count, entries. Sent with the whole book on entering the world and
//!   with the one new entry after a location was recorded (`AddPrivatePortal`): entries are merged by id.
//! * `SCCharacterReturnDistricts` (0x05c): `i32` count, entries, `u32` the return point the character is
//!   bound to (`X2Warp:GetBoundPortalId`). Always the whole district list.
//! * `SCPortalInfoSaved` (0x05d): one entry, a private location renamed.
//! * `SCPortalDeleted` (0x05e): `u8` portal type, `i32` id (the lane never sends it: deletions are local).
//! * `SCUnitPortalUsed` (0x060): `bc` the unit that went through a portal.
//! * `SCNaviTeleport` (0x15c): a long position (`i64` x, `i64` y, `f32` z).
//!
//! An entry: `u32` id, string name, `u32` zone key, `f32` x and y relative to the zone's origin cell
//! (`x - origin_x * 1024`), `f32` z, `f32` z rotation (degrees).
//!
//! Client to server ([`PortalCommand`]): recording, renaming and opening are skills (`CSStartSkill` with a
//! `SkillObject`: `SavePortalInfo` = type 2, `i32` id (0 = a new location), string name; `PortalInfo` = type 1,
//! `u8` portal type, `i32` book id, the portal's long position); deleting is `CSDeletePortal` (0x0df: `u8`
//! type, `u32` id), entering a summoned portal `CSUsePortal` (0x0de: `bc` portal unit, `bool` only my own
//! portal), and the visited districts come from `CSNotifySubZone` (0x112: `u32` sub zone).

use crate::session::{Event, Phase};
use crate::skill::{SkillTarget, StartCaster};
use crate::wire::{Pos, Reader, Result, WireError, Writer};

/// `SCCharacterPortals`.
pub const SC_CHARACTER_PORTALS: u16 = 0x05b;
/// `SCCharacterReturnDistricts`.
pub const SC_CHARACTER_RETURN_DISTRICTS: u16 = 0x05c;
/// `SCPortalInfoSaved`.
pub const SC_PORTAL_INFO_SAVED: u16 = 0x05d;
/// `SCPortalDeleted`.
pub const SC_PORTAL_DELETED: u16 = 0x05e;
/// `SCUnitPortalUsed`.
pub const SC_UNIT_PORTAL_USED: u16 = 0x060;
/// `SCNaviTeleport`.
pub const SC_NAVI_TELEPORT: u16 = 0x15c;

/// `CSUsePortal`.
pub const CS_USE_PORTAL: u16 = 0x0de;
/// `CSDeletePortal`.
pub const CS_DELETE_PORTAL: u16 = 0x0df;
/// `CSNotifySubZone`.
pub const CS_NOTIFY_SUB_ZONE: u16 = 0x112;
/// `CSStartSkill`.
pub const CS_START_SKILL: u16 = 0x052;
/// `CSInstanceLoaded`.
pub const CS_INSTANCE_LOADED: u16 = 0x0e0;

/// Portal types of the book (`CSDeletePortal`, `SkillObjectPortalInfo.type`): the server treats 1 as a
/// district (return point) and everything else as a recorded location.
pub mod portal_type {
    pub const DISTRICT: u8 = 1;
    pub const PRIVATE: u8 = 2;
}

/// `SkillObject` type flags.
mod skill_object {
    pub const PORTAL_INFO: u8 = 1;
    pub const SAVE_PORTAL_INFO: u8 = 2;
}

/// One location of the book (`Portal.Write`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PortalEntry {
    pub id: u32,
    pub name: String,
    /// Zone key (`zones.id`).
    pub zone_id: u32,
    /// Position relative to the origin cell of the zone (metres): add `origin * 1024` for the world position.
    pub off_x: f32,
    pub off_y: f32,
    pub z: f32,
    /// Degrees.
    pub z_rot: f32,
}

impl PortalEntry {
    pub fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self { id: r.u32()?, name: r.string()?, zone_id: r.u32()?, off_x: r.f32()?, off_y: r.f32()?, z: r.f32()?, z_rot: r.f32()? })
    }

    pub fn write(&self, w: &mut Writer) {
        w.u32(self.id).string(&self.name).u32(self.zone_id).f32(self.off_x).f32(self.off_y).f32(self.z).f32(self.z_rot);
    }
}

/// What the server reports of the book and the portals.
#[derive(Clone, Debug, PartialEq)]
pub enum PortalEvent {
    /// Recorded locations: the whole book on entering the world, the new one after recording.
    Portals(Vec<PortalEntry>),
    /// The visited districts (always all of them) and the return point the character is bound to.
    ReturnDistricts { portals: Vec<PortalEntry>, return_point_id: u32 },
    /// A recorded location was renamed.
    InfoSaved(PortalEntry),
    Deleted { portal_type: u8, portal_id: i32 },
    /// A unit went through a portal.
    UnitPortalUsed { unit_id: u32 },
    NaviTeleport { pos: Pos },
}

/// More entries than any book holds: a corrupt count.
const MAX_ENTRIES: i32 = 4096;

fn read_entries(r: &mut Reader) -> Result<Vec<PortalEntry>> {
    let n = r.i32()?;
    if !(0..=MAX_ENTRIES).contains(&n) {
        return Err(WireError::Invalid(format!("portal count {n}")));
    }
    (0..n).map(|_| PortalEntry::read(r)).collect()
}

impl PortalEvent {
    pub fn handles(opcode: u16) -> bool {
        matches!(
            opcode,
            SC_CHARACTER_PORTALS | SC_CHARACTER_RETURN_DISTRICTS | SC_PORTAL_INFO_SAVED | SC_PORTAL_DELETED | SC_UNIT_PORTAL_USED | SC_NAVI_TELEPORT
        )
    }

    pub fn read(opcode: u16, r: &mut Reader) -> Result<Self> {
        Ok(match opcode {
            SC_CHARACTER_PORTALS => Self::Portals(read_entries(r)?),
            SC_CHARACTER_RETURN_DISTRICTS => {
                let portals = read_entries(r)?;
                Self::ReturnDistricts { portals, return_point_id: r.u32()? }
            }
            SC_PORTAL_INFO_SAVED => Self::InfoSaved(PortalEntry::read(r)?),
            SC_PORTAL_DELETED => Self::Deleted { portal_type: r.u8()?, portal_id: r.i32()? },
            SC_UNIT_PORTAL_USED => Self::UnitPortalUsed { unit_id: r.bc()? },
            SC_NAVI_TELEPORT => Self::NaviTeleport { pos: r.long_position()? },
            _ => return Err(WireError::Invalid(format!("not a portal opcode {opcode:#x}"))),
        })
    }
}

/// What the client asks of the book and the portals. The skill ids are the client's choice (the
/// database: `11215` records, `16841` renames, the Teleport Book's `11216` opens a portal).
#[derive(Clone, Debug, PartialEq)]
pub enum PortalCommand {
    /// Record the current location under `name` (`SavePortal` effect, `SavePortalInfo { id: 0 }`).
    Record { skill_id: u32, name: String },
    /// Rename the recorded location `portal_id` (`SavePortalInfo { id }`).
    Rename { skill_id: u32, portal_id: u32, name: String },
    /// Open a portal to the book entry `portal_id` at `pos` (in front of the player): `OpenPortal` effect,
    /// cast by `caster` (the Teleport Book item).
    Open { skill_id: u32, caster: StartCaster, portal_type: u8, portal_id: u32, pos: Pos },
    /// Remove an entry from the book (`CSDeletePortal`).
    Delete { portal_type: u8, portal_id: u32 },
    /// Go through the summoned portal unit `obj_id` (`CSUsePortal`).
    Use { obj_id: u32, only_my_portal: bool },
    /// The player entered the sub zone (`CSNotifySubZone`): visited districts are added to the book.
    NotifySubZone { sub_zone_id: u32 },
    /// The world around the destination of an `SCLoadInstance` is loaded (`CSInstanceLoaded`, empty): the server
    /// sends the player's unit state again and accepts movement.
    InstanceLoaded,
}

impl PortalCommand {
    pub fn opcode(&self) -> u16 {
        match self {
            Self::Record { .. } | Self::Rename { .. } | Self::Open { .. } => CS_START_SKILL,
            Self::Delete { .. } => CS_DELETE_PORTAL,
            Self::Use { .. } => CS_USE_PORTAL,
            Self::NotifySubZone { .. } => CS_NOTIFY_SUB_ZONE,
            Self::InstanceLoaded => CS_INSTANCE_LOADED,
        }
    }

    /// The packet body; `self_obj_id` is the player's unit (the caster and target of the skills).
    pub fn write(&self, w: &mut Writer, self_obj_id: u32) {
        match self {
            Self::Record { skill_id, name } => {
                start_skill(w, *skill_id, self_obj_id, StartCaster::Unit, |w| {
                    w.u8(skill_object::SAVE_PORTAL_INFO).i32(0).string(name);
                });
            }
            Self::Rename { skill_id, portal_id, name } => {
                start_skill(w, *skill_id, self_obj_id, StartCaster::Unit, |w| {
                    w.u8(skill_object::SAVE_PORTAL_INFO).i32(*portal_id as i32).string(name);
                });
            }
            Self::Open { skill_id, caster, portal_type, portal_id, pos } => {
                start_skill(w, *skill_id, self_obj_id, *caster, |w| {
                    w.u8(skill_object::PORTAL_INFO).u8(*portal_type).i32(*portal_id as i32).long_position(*pos);
                });
            }
            Self::Delete { portal_type, portal_id } => {
                w.u8(*portal_type).u32(*portal_id);
            }
            Self::Use { obj_id, only_my_portal } => {
                w.bc(*obj_id).bool(*only_my_portal);
            }
            Self::NotifySubZone { sub_zone_id } => {
                w.u32(*sub_zone_id);
            }
            Self::InstanceLoaded => {}
        }
    }

    /// Checks before sending; the server checks the rest (the reagents, the zone, ownership).
    pub fn validate(&self) -> std::result::Result<(), &'static str> {
        match self {
            Self::Record { name, .. } | Self::Rename { name, .. } if name.trim().is_empty() => Err("a portal needs a name"),
            Self::Rename { portal_id: 0, .. } => Err("no portal to rename"),
            Self::Open { portal_id: 0, .. } | Self::Delete { portal_id: 0, .. } => Err("no portal"),
            Self::Use { obj_id, .. } if *obj_id == 0 || *obj_id > 0xff_ffff => Err("not a portal unit"),
            Self::NotifySubZone { sub_zone_id: 0 } => Err("no sub zone"),
            _ => Ok(()),
        }
    }
}

/// `CSStartSkill` with a skill object: the plain request of [`crate::skill::write_start_skill`] with its
/// "no skill object" flag replaced by `object`.
fn start_skill(w: &mut Writer, skill_id: u32, self_obj_id: u32, caster: StartCaster, object: impl FnOnce(&mut Writer)) {
    let mut plain = Writer::new();
    crate::skill::write_start_skill(&mut plain, skill_id, self_obj_id, caster, &SkillTarget::unit(self_obj_id));
    let mut bytes = plain.finish();
    debug_assert_eq!(bytes.last(), Some(&0));
    bytes.pop();
    w.raw(&bytes);
    object(w);
}

/// The book as the server reported it.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "bevy", derive(bevy::ecs::resource::Resource))]
pub struct Portals {
    /// Recorded locations, in the order they came.
    pub private: Vec<PortalEntry>,
    /// Visited districts (return points).
    pub districts: Vec<PortalEntry>,
    /// The return point the character is bound to (0: none reported).
    pub return_point_id: u32,
    /// Bumped on every change (the HUD raises its events from it).
    pub revision: u32,
    /// What the last change was, for the HUD's events.
    pub last: Option<PortalChange>,
}

/// The kind of the last change of [`Portals`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortalChange {
    /// The whole book or new recorded locations (`SAVE_PORTAL`).
    Saved,
    Districts,
    Renamed,
    Deleted,
}

impl Portals {
    pub fn apply(&mut self, event: &Event) -> bool {
        let before = self.clone();
        match event {
            Event::LoginSucceeded(_)
            | Event::CharacterSelected(_)
            | Event::LeftWorld { .. }
            | Event::Disconnected { .. }
            | Event::Kicked { .. }
            | Event::PhaseChanged(Phase::Authenticating | Phase::Connecting) => *self = Self::default(),
            Event::Portal(e) => {
                let change = match e {
                    PortalEvent::Portals(list) => {
                        for p in list {
                            match self.private.iter_mut().find(|q| q.id == p.id) {
                                Some(q) => *q = p.clone(),
                                None => self.private.push(p.clone()),
                            }
                        }
                        Some(PortalChange::Saved)
                    }
                    PortalEvent::ReturnDistricts { portals, return_point_id } => {
                        self.districts = portals.clone();
                        self.return_point_id = *return_point_id;
                        Some(PortalChange::Districts)
                    }
                    PortalEvent::InfoSaved(p) => {
                        match self.private.iter_mut().find(|q| q.id == p.id) {
                            Some(q) => *q = p.clone(),
                            None => self.private.push(p.clone()),
                        }
                        Some(PortalChange::Renamed)
                    }
                    PortalEvent::Deleted { portal_type, portal_id } => {
                        self.remove(*portal_type, *portal_id as u32);
                        Some(PortalChange::Deleted)
                    }
                    PortalEvent::UnitPortalUsed { .. } | PortalEvent::NaviTeleport { .. } => None,
                };
                if let Some(change) = change {
                    self.revision = self.revision.wrapping_add(1);
                    self.last = Some(change);
                }
            }
            Event::PortalDeleteSent { portal_type, portal_id } => {
                // the lane answers a deletion with nothing: the book drops the entry once the request is out
                if self.remove(*portal_type, *portal_id) {
                    self.revision = self.revision.wrapping_add(1);
                    self.last = Some(PortalChange::Deleted);
                }
            }
            _ => {}
        }
        *self != before
    }

    fn remove(&mut self, portal_type: u8, portal_id: u32) -> bool {
        let list = if portal_type == portal_type::DISTRICT { &mut self.districts } else { &mut self.private };
        let n = list.len();
        list.retain(|p| p.id != portal_id);
        list.len() != n
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{ClientMsg, ServerMsg};

    fn entry(id: u32, name: &str) -> PortalEntry {
        PortalEntry { id, name: name.into(), zone_id: 129, off_x: 10.0, off_y: 20.0, z: 30.0, z_rot: 45.0 }
    }

    fn body(op: u16, f: impl FnOnce(&mut Writer)) -> Vec<u8> {
        // a whole level-1 frame: magic, level, hash, count, opcode, body
        let mut w = Writer::new();
        w.u8(0xdd).u8(crate::game::LEVEL_GAME).u8(0).u8(0).u16(op);
        f(&mut w);
        w.finish()
    }

    /// The lane's `s2c_byte_exact` golden of `SCPortalInfoSaved` (`portal/packets.rs`).
    #[test]
    fn lane_portal_info_saved_golden() {
        let mut e = Vec::new();
        e.extend_from_slice(&4097u32.to_le_bytes());
        e.extend_from_slice(&3u16.to_le_bytes());
        e.extend_from_slice(b"abc");
        e.extend_from_slice(&129u32.to_le_bytes());
        for f in [10.0f32, 20.0, 30.0, 45.0] {
            e.extend_from_slice(&f.to_le_bytes());
        }
        let mut r = Reader::new(&e);
        assert_eq!(PortalEvent::read(SC_PORTAL_INFO_SAVED, &mut r).unwrap(), PortalEvent::InfoSaved(entry(4097, "abc")));
        assert!(r.is_empty());
        let mut w = Writer::new();
        entry(4097, "abc").write(&mut w);
        assert_eq!(w.finish(), e);
        // truncations fail
        for n in 0..e.len() {
            assert!(PortalEvent::read(SC_PORTAL_INFO_SAVED, &mut Reader::new(&e[..n])).is_err(), "{n}");
        }
    }

    #[test]
    fn lists_deleted_used_and_navi_decode_through_the_game_decoder() {
        let mut list = Writer::new();
        list.i32(2);
        entry(1, "Home").write(&mut list);
        entry(2, "Farm").write(&mut list);
        let list = list.finish();
        let raw = body(SC_CHARACTER_PORTALS, |w| {
            w.raw(&list);
        });
        assert_eq!(ServerMsg::decode(&raw).unwrap(), ServerMsg::Portal(PortalEvent::Portals(vec![entry(1, "Home"), entry(2, "Farm")])));
        let raw = body(SC_CHARACTER_RETURN_DISTRICTS, |w| {
            w.raw(&list).u32(240);
        });
        assert_eq!(
            ServerMsg::decode(&raw).unwrap(),
            ServerMsg::Portal(PortalEvent::ReturnDistricts { portals: vec![entry(1, "Home"), entry(2, "Farm")], return_point_id: 240 })
        );
        // the lane golden of SCPortalDeleted: [1, 0x34, 0x12, 0, 0]
        let raw = body(SC_PORTAL_DELETED, |w| {
            w.raw(&[1, 0x34, 0x12, 0, 0]);
        });
        assert_eq!(ServerMsg::decode(&raw).unwrap(), ServerMsg::Portal(PortalEvent::Deleted { portal_type: 1, portal_id: 0x1234 }));
        let raw = body(SC_UNIT_PORTAL_USED, |w| {
            w.raw(&[0x0c, 0x0b, 0x0a]);
        });
        assert_eq!(ServerMsg::decode(&raw).unwrap(), ServerMsg::Portal(PortalEvent::UnitPortalUsed { unit_id: 0x0a0b0c }));
        let raw = body(SC_NAVI_TELEPORT, |w| {
            w.long_position(Pos::new(1000.5, 2000.0, 50.0));
        });
        assert_eq!(ServerMsg::decode(&raw).unwrap(), ServerMsg::Portal(PortalEvent::NaviTeleport { pos: Pos::new(1000.5, 2000.0, 50.0) }));
        // a negative or huge count is refused
        let negative = (-1i32).to_le_bytes();
        assert!(PortalEvent::read(SC_CHARACTER_PORTALS, &mut Reader::new(&negative)).is_err());
    }

    /// The lane's `c2s_decode` goldens: `CSUsePortal` `[01 02 03 01]`, `CSDeletePortal` `[02 01 10 00 00]`.
    #[test]
    fn requests_match_the_lane_readers() {
        let use_portal = ClientMsg::Portal { self_obj_id: 7, command: PortalCommand::Use { obj_id: 0x030201, only_my_portal: true } };
        assert_eq!(use_portal.opcode(), 0x0de);
        assert_eq!(use_portal.encode_body(), vec![0x01, 0x02, 0x03, 0x01]);
        let delete = ClientMsg::Portal { self_obj_id: 7, command: PortalCommand::Delete { portal_type: 2, portal_id: 0x1001 } };
        assert_eq!(delete.opcode(), 0x0df);
        assert_eq!(delete.encode_body(), vec![0x02, 0x01, 0x10, 0x00, 0x00]);
        let notify = ClientMsg::Portal { self_obj_id: 7, command: PortalCommand::NotifySubZone { sub_zone_id: 188 } };
        assert_eq!((notify.opcode(), notify.encode_body()), (0x112, 188u32.to_le_bytes().to_vec()));
        let loaded = ClientMsg::Portal { self_obj_id: 7, command: PortalCommand::InstanceLoaded };
        assert_eq!((loaded.opcode(), loaded.encode_body()), (0x0e0, vec![]));
    }

    /// `CSStartSkill` + `SkillObject` as the lane reads it (`CSStartSkillPacket.read`, `SkillObject::read`).
    #[test]
    fn skill_requests_carry_the_portal_skill_objects() {
        let me = 0x010203;
        let record = ClientMsg::Portal { self_obj_id: me, command: PortalCommand::Record { skill_id: 11215, name: "Home".into() } };
        let b = record.encode_body();
        let mut r = Reader::new(&b);
        assert_eq!(r.u32().unwrap(), 11215);
        assert_eq!((r.u8().unwrap(), r.bc().unwrap()), (0, me)); // unit caster
        assert_eq!((r.u8().unwrap(), r.bc().unwrap()), (0, me)); // unit target
        assert_eq!(r.u8().unwrap(), 2); // SavePortalInfo
        assert_eq!((r.i32().unwrap(), r.string().unwrap()), (0, "Home".into()));
        assert!(r.is_empty());

        let rename = ClientMsg::Portal { self_obj_id: me, command: PortalCommand::Rename { skill_id: 16841, portal_id: 9, name: "Farm".into() } };
        let b = rename.encode_body();
        assert_eq!(&b[b.len() - 11..], &[2, 9, 0, 0, 0, 4, 0, b'F', b'a', b'r', b'm']);

        let pos = Pos::new(1000.5, 2000.25, 100.0);
        let open = ClientMsg::Portal {
            self_obj_id: me,
            command: PortalCommand::Open { skill_id: 11216, caster: StartCaster::Item { item_id: 77, template_id: 4045 }, portal_type: 2, portal_id: 9, pos },
        };
        let b = open.encode_body();
        let mut r = Reader::new(&b);
        assert_eq!(r.u32().unwrap(), 11216);
        assert_eq!(r.u8().unwrap(), 2); // item caster
        assert_eq!(r.bc().unwrap(), me);
        assert_eq!((r.u64().unwrap(), r.u32().unwrap(), r.u8().unwrap(), r.u32().unwrap()), (77, 4045, 0, 0));
        assert_eq!((r.u8().unwrap(), r.bc().unwrap()), (0, me)); // unit target
        assert_eq!(r.u8().unwrap(), 1); // PortalInfo
        assert_eq!((r.u8().unwrap(), r.i32().unwrap()), (2, 9));
        assert_eq!(r.long_position().unwrap(), pos);
        assert!(r.is_empty());
    }

    /// The lane's `SCTeleportUnit` golden: reason, error, `encode_position`, then `z2`; the yaw is kept and the next
    /// packet of a batch still decodes.
    #[test]
    fn teleport_unit_keeps_its_yaw() {
        let mut w = Writer::new();
        w.u8(0).i16(0).position(Pos::new(1000.0, 2000.0, 100.0)).f32(1.5);
        let raw = body(crate::opcodes::sc::SCTeleportUnitPacket, |b| {
            b.raw(&w.finish());
        });
        match ServerMsg::decode(&raw).unwrap() {
            ServerMsg::Teleport { reason: 0, error: 0, pos, yaw } => {
                assert!((pos.x - 1000.0).abs() < 0.01 && (pos.y - 2000.0).abs() < 0.01 && (pos.z - 100.0).abs() < 0.01);
                assert_eq!(yaw, 1.5);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn validation_refuses_what_the_server_cannot_use() {
        assert!(PortalCommand::Record { skill_id: 1, name: " ".into() }.validate().is_err());
        assert!(PortalCommand::Use { obj_id: 0x0100_0000, only_my_portal: false }.validate().is_err());
        assert!(PortalCommand::Delete { portal_type: 2, portal_id: 0 }.validate().is_err());
        assert!(PortalCommand::Delete { portal_type: 2, portal_id: 3 }.validate().is_ok());
    }

    #[test]
    fn the_book_merges_new_locations_renames_and_deletes() {
        let mut book = Portals::default();
        assert!(book.apply(&Event::Portal(PortalEvent::Portals(vec![entry(1, "Home"), entry(2, "Farm")]))));
        assert!(book.apply(&Event::Portal(PortalEvent::Portals(vec![entry(3, "Cave")]))));
        assert_eq!(book.private.iter().map(|p| p.id).collect::<Vec<_>>(), [1, 2, 3]);
        assert_eq!(book.last, Some(PortalChange::Saved));
        assert!(book.apply(&Event::Portal(PortalEvent::InfoSaved(entry(2, "Barn")))));
        assert_eq!((book.private[1].name.as_str(), book.last), ("Barn", Some(PortalChange::Renamed)));
        assert!(book.apply(&Event::Portal(PortalEvent::ReturnDistricts { portals: vec![entry(240, "Harani")], return_point_id: 240 })));
        assert_eq!(book.return_point_id, 240);
        assert!(book.apply(&Event::PortalDeleteSent { portal_type: portal_type::PRIVATE, portal_id: 1 }));
        assert_eq!(book.private.len(), 2);
        assert!(!book.apply(&Event::PortalDeleteSent { portal_type: portal_type::PRIVATE, portal_id: 1 }));
        assert!(book.apply(&Event::Portal(PortalEvent::Deleted { portal_type: portal_type::DISTRICT, portal_id: 240 })));
        assert!(book.districts.is_empty());
        let rev = book.revision;
        assert!(!book.apply(&Event::Portal(PortalEvent::UnitPortalUsed { unit_id: 5 })));
        assert_eq!(book.revision, rev);
        assert!(book.apply(&Event::LeftWorld { target: 1 }));
        assert_eq!(book, Portals::default());
    }
}
