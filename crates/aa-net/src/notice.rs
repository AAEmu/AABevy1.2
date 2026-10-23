//! World notices and small gameplay messages: server notices, siege alerts, tower defense, arrests,
//! bot reports, instance queues, tutorials, NPC voice cues, specialty prices and player music.
//!
//! The layouts are the lane server's writers, checked against the 1.2 client's readers
//! (`x2game_unpacked.dll`, see `docs/aa-net.md` "Small server messages"):
//!
//! | packet | body |
//! |---|---|
//! | `SCDominionTaxRate` 0x020 | `u16 zone group, i32 tax rate` |
//! | `SCNoticeMessage` 0x0d0 | `u8 kind, str alpha ("FF"), i32 visible ms, str "RRGGBB" + text` |
//! | `SCSiegeMember` 0x0ec | `u16 zone group, u32 expedition, u32 character, bool added` |
//! | `SCSiegeAlert` 0x0ed | `u16 zone group, u8 alert (0..6), u32 character, str name (only when character != 0)` |
//! | `SCConflictZoneHonorPointSum` 0x0ef | `u16 zone group, i32 sum` |
//! | `SCTowerDefList` 0x16b | `u8 count` (lane: `i32`), [`TowerDefInfo`] each |
//! | `SCTowerDefStart` / `End` 0x16c / 0x16d | [`TowerDefKey`], `u32 event zone` |
//! | `SCTowerDefWaveStart` 0x16e | [`TowerDefKey`], `u32 event zone, u32 step` |
//! | `SCCriminalArrested` 0x170 | `bc criminal` (lane: `u32`), `str criminal, str arrestor` |
//! | `SCBotSuspectReported` 0x184 | `str reporter, str suspect` |
//! | `SCProcessingInstance` 0x198 | `i32 zone` |
//! | `SCTutorialSaved` 0x1a7 | `u32 tutorial, u64 state` |
//! | `SCPlaySound` 0x1de | `u8 kind`; 0: `u32 sound`; 1: `u32 sound, str`; 2: `u32 bubble, bc npc` (lane: `u32`) |
//! | `SCSendUserMusic` 0x1df | `i32 length, bc player, str author, blob midi` |
//! | `SCUserNoteLoaded` 0x1e0 | empty (unknown song) or `u32 song, bool tooltip, i32 length, str title, str notes` |
//! | `SCPauseUserMusic` 0x1e1 | `bc player` |
//! | `SCSpecialtyCurrent` 0x1f4 | `i32 count, u16 from zone group, u16 to zone group`, `u32 item, i32 ratio` each |
//!
//! Where the lane differs from the client (the tower defense list count, the arrested criminal's
//! id, the NPC id of a voice cue) this reads the client's layout; see the report in
//! `docs/aa-net.md`.
use crate::wire::{Pos, Reader, Result, WireError, long_to_coord};

/// Opcodes of this module (not in [`crate::opcodes`], which only lists what the session drives).
pub mod op {
    pub const DOMINION_TAX_RATE: u16 = 0x020;
    pub const NOTICE_MESSAGE: u16 = 0x0d0;
    pub const SIEGE_MEMBER: u16 = 0x0ec;
    pub const SIEGE_ALERT: u16 = 0x0ed;
    pub const HONOR_POINT_SUM: u16 = 0x0ef;
    pub const TOWER_DEF_LIST: u16 = 0x16b;
    pub const TOWER_DEF_START: u16 = 0x16c;
    pub const TOWER_DEF_END: u16 = 0x16d;
    pub const TOWER_DEF_WAVE_START: u16 = 0x16e;
    pub const CRIMINAL_ARRESTED: u16 = 0x170;
    pub const BOT_SUSPECT_REPORTED: u16 = 0x184;
    pub const PROCESSING_INSTANCE: u16 = 0x198;
    pub const TUTORIAL_SAVED: u16 = 0x1a7;
    pub const PLAY_SOUND: u16 = 0x1de;
    pub const SEND_USER_MUSIC: u16 = 0x1df;
    pub const USER_NOTE_LOADED: u16 = 0x1e0;
    pub const PAUSE_USER_MUSIC: u16 = 0x1e1;
    pub const SPECIALTY_CURRENT: u16 = 0x1f4;
}

/// The client's limit of a tower defense list (`count` is read as a byte, at most 100 entries).
const MAX_TOWER_DEFS: usize = 100;
/// The client's limit of a specialty price list.
const MAX_SPECIALTY_RATIOS: usize = 128;

/// A tower defense (`tower_defs.id`) in a zone group.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TowerDefKey {
    pub tower_def_id: u32,
    pub zone_group: u16,
}

impl TowerDefKey {
    fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self { tower_def_id: r.u32()?, zone_group: r.u16()? })
    }
}

/// A running tower defense: where its target is and which wave it is at.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TowerDefInfo {
    pub key: TowerDefKey,
    pub zone_id: u32,
    pub spot_id: u32,
    pub target_obj_id: u32,
    pub pos: Pos,
    pub step: u32,
}

impl TowerDefInfo {
    fn read(r: &mut Reader) -> Result<Self> {
        let key = TowerDefKey::read(r)?;
        let zone_id = r.u32()?;
        let spot_id = r.u32()?;
        let target_obj_id = r.bc()?;
        let (x, y, z) = (r.i64()?, r.i64()?, r.f32()?);
        Ok(Self { key, zone_id, spot_id, target_obj_id, pos: Pos::new(long_to_coord(x), long_to_coord(y), z), step: r.u32()? })
    }
}

/// What `SCPlaySound` asks to play.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SoundCue {
    /// Kind 0: a sound id.
    Sound(u32),
    /// Kind 1: a sound id with a text argument.
    Named { id: u32, name: String },
    /// Kind 2: the voice of NPC speech bubble `bubble_id` (`npc_chat_bubbles`), spoken by NPC `npc_obj_id`.
    Bubble { bubble_id: u32, npc_obj_id: u32 },
}

#[derive(Clone, Debug, PartialEq)]
pub enum NoticeEvent {
    DominionTaxRate { zone_group: u16, tax_rate: i32 },
    /// A server notice: the big centre-screen message (`kind` picks the style), `color` the
    /// alpha as hex ("FF"), `message` starts with the colour as `RRGGBB`.
    Notice { kind: u8, color: String, visible_ms: i32, message: String },
    SiegeMember { zone_group: u16, expedition_id: u32, character_id: u32, added: bool },
    /// A siege tower alert (0..6, `DOMINION_GUARD_TOWER_STATE_NOTICE`), naming the character who caused it.
    SiegeAlert { zone_group: u16, alert: u8, character_id: u32, name: String },
    /// The honor points gathered in a conflict zone during a war.
    HonorPointSum { zone_group: u16, sum: i32 },
    TowerDefList(Vec<TowerDefInfo>),
    TowerDefStarted { key: TowerDefKey, event_zone_id: u32 },
    TowerDefEnded { key: TowerDefKey, event_zone_id: u32 },
    TowerDefWaveStarted { key: TowerDefKey, event_zone_id: u32, step: u32 },
    CriminalArrested { criminal_obj_id: u32, criminal: String, arrestor: String },
    BotSuspectReported { reporter: String, suspect: String },
    /// The player waits in the queue of an instance (`zone_id`).
    ProcessingInstance { zone_id: i32 },
    TutorialSaved { tutorial_id: u32, state: u64 },
    PlaySound(SoundCue),
    /// Player `obj_id` plays a song (MIDI bytes) by `author`.
    UserMusic { obj_id: u32, author: String, midi: Vec<u8> },
    /// A saved song (`None`: the server does not know it).
    UserNoteLoaded(Option<UserNote>),
    UserMusicPaused { obj_id: u32 },
    /// The specialty prices (`item`, ratio in %) for trade packs carried between two zone groups.
    SpecialtyCurrent { from_zone_group: u16, to_zone_group: u16, ratios: Vec<(u32, i32)> },
}

/// A saved song (`SCUserNoteLoaded`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserNote {
    pub song_id: u32,
    pub tooltip: bool,
    pub title: String,
    pub notes: String,
}

impl NoticeEvent {
    pub fn handles(opcode: u16) -> bool {
        use op::*;
        matches!(
            opcode,
            DOMINION_TAX_RATE
                | NOTICE_MESSAGE
                | SIEGE_MEMBER
                | SIEGE_ALERT
                | HONOR_POINT_SUM
                | TOWER_DEF_LIST..=TOWER_DEF_WAVE_START
                | CRIMINAL_ARRESTED
                | BOT_SUSPECT_REPORTED
                | PROCESSING_INSTANCE
                | TUTORIAL_SAVED
                | PLAY_SOUND..=PAUSE_USER_MUSIC
                | SPECIALTY_CURRENT
        )
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::DominionTaxRate { .. } => "SCDominionTaxRate",
            Self::Notice { .. } => "SCNoticeMessage",
            Self::SiegeMember { .. } => "SCSiegeMember",
            Self::SiegeAlert { .. } => "SCSiegeAlert",
            Self::HonorPointSum { .. } => "SCConflictZoneHonorPointSum",
            Self::TowerDefList(_) => "SCTowerDefList",
            Self::TowerDefStarted { .. } => "SCTowerDefStart",
            Self::TowerDefEnded { .. } => "SCTowerDefEnd",
            Self::TowerDefWaveStarted { .. } => "SCTowerDefWaveStart",
            Self::CriminalArrested { .. } => "SCCriminalArrested",
            Self::BotSuspectReported { .. } => "SCBotSuspectReported",
            Self::ProcessingInstance { .. } => "SCProcessingInstance",
            Self::TutorialSaved { .. } => "SCTutorialSaved",
            Self::PlaySound(_) => "SCPlaySound",
            Self::UserMusic { .. } => "SCSendUserMusic",
            Self::UserNoteLoaded(_) => "SCUserNoteLoaded",
            Self::UserMusicPaused { .. } => "SCPauseUserMusic",
            Self::SpecialtyCurrent { .. } => "SCSpecialtyCurrent",
        }
    }

    pub fn read(opcode: u16, r: &mut Reader) -> Result<Self> {
        Ok(match opcode {
            op::DOMINION_TAX_RATE => Self::DominionTaxRate { zone_group: r.u16()?, tax_rate: r.i32()? },
            op::NOTICE_MESSAGE => Self::Notice { kind: r.u8()?, color: r.string()?, visible_ms: r.i32()?, message: r.string()? },
            op::SIEGE_MEMBER => {
                Self::SiegeMember { zone_group: r.u16()?, expedition_id: r.u32()?, character_id: r.u32()?, added: r.bool()? }
            }
            op::SIEGE_ALERT => {
                let (zone_group, alert, character_id) = (r.u16()?, r.u8()?, r.u32()?);
                let name = if character_id != 0 { r.string()? } else { String::new() };
                Self::SiegeAlert { zone_group, alert, character_id, name }
            }
            op::HONOR_POINT_SUM => Self::HonorPointSum { zone_group: r.u16()?, sum: r.i32()? },
            op::TOWER_DEF_LIST => {
                let count = usize::from(r.u8()?);
                if count > MAX_TOWER_DEFS {
                    return Err(WireError::Invalid(format!("tower defense list of {count}")));
                }
                Self::TowerDefList((0..count).map(|_| TowerDefInfo::read(r)).collect::<Result<_>>()?)
            }
            op::TOWER_DEF_START => Self::TowerDefStarted { key: TowerDefKey::read(r)?, event_zone_id: r.u32()? },
            op::TOWER_DEF_END => Self::TowerDefEnded { key: TowerDefKey::read(r)?, event_zone_id: r.u32()? },
            op::TOWER_DEF_WAVE_START => {
                Self::TowerDefWaveStarted { key: TowerDefKey::read(r)?, event_zone_id: r.u32()?, step: r.u32()? }
            }
            op::CRIMINAL_ARRESTED => Self::CriminalArrested { criminal_obj_id: r.bc()?, criminal: r.string()?, arrestor: r.string()? },
            op::BOT_SUSPECT_REPORTED => Self::BotSuspectReported { reporter: r.string()?, suspect: r.string()? },
            op::PROCESSING_INSTANCE => Self::ProcessingInstance { zone_id: r.i32()? },
            op::TUTORIAL_SAVED => Self::TutorialSaved { tutorial_id: r.u32()?, state: r.u64()? },
            op::PLAY_SOUND => Self::PlaySound(match r.u8()? {
                0 => SoundCue::Sound(r.u32()?),
                1 => SoundCue::Named { id: r.u32()?, name: r.string()? },
                2 => SoundCue::Bubble { bubble_id: r.u32()?, npc_obj_id: r.bc()? },
                kind => return Err(WireError::Invalid(format!("sound cue kind {kind}"))),
            }),
            op::SEND_USER_MUSIC => {
                let length = r.u32()?;
                if length > crate::music::MAX_MIDI_BYTES as u32 {
                    return Err(WireError::Invalid(format!("music MIDI byte length {length}")));
                }
                let (obj_id, author) = (r.bc()?, r.string()?);
                // This blob's unsigned u16 count includes the valid 0x8000 boundary.
                // Reader::blob treats that bit pattern as a negative/empty generic blob.
                let blob_length = r.u16()?;
                if u32::from(blob_length) != length {
                    return Err(WireError::Invalid("music MIDI byte counts disagree".into()));
                }
                Self::UserMusic { obj_id, author, midi: r.bytes(usize::from(blob_length))?.to_vec() }
            }
            op::USER_NOTE_LOADED if r.is_empty() => Self::UserNoteLoaded(None),
            op::USER_NOTE_LOADED => {
                let (song_id, tooltip) = (r.u32()?, r.bool()?);
                r.i32()?; // the notes' length, repeated by the string
                Self::UserNoteLoaded(Some(UserNote { song_id, tooltip, title: r.string()?, notes: r.string()? }))
            }
            op::PAUSE_USER_MUSIC => Self::UserMusicPaused { obj_id: r.bc()? },
            op::SPECIALTY_CURRENT => {
                let count = r.i32()?;
                let count = usize::try_from(count)
                    .ok()
                    .filter(|&n| n <= MAX_SPECIALTY_RATIOS)
                    .ok_or_else(|| WireError::Invalid(format!("specialty price list of {count}")))?;
                let (from_zone_group, to_zone_group) = (r.u16()?, r.u16()?);
                let ratios = (0..count).map(|_| Ok((r.u32()?, r.i32()?))).collect::<Result<_>>()?;
                Self::SpecialtyCurrent { from_zone_group, to_zone_group, ratios }
            }
            opcode => return Err(WireError::Invalid(format!("not a notice packet: {opcode:#05x}"))),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{Writer, coord_to_long};

    fn read(opcode: u16, body: &Writer) -> NoticeEvent {
        let body = body.as_bytes();
        let event = NoticeEvent::read(opcode, &mut Reader::new(body)).unwrap();
        for n in 0..body.len() {
            assert!(NoticeEvent::read(opcode, &mut Reader::new(&body[..n])).is_err() || n == 0 && opcode == op::USER_NOTE_LOADED, "{opcode:#x} cut {n}");
        }
        event
    }

    #[test]
    fn notices_and_siege_messages_decode() {
        let mut w = Writer::new();
        w.u8(1).string("FF").i32(3000).string("FF0000The server restarts in 5 minutes");
        assert_eq!(
            read(op::NOTICE_MESSAGE, &w),
            NoticeEvent::Notice { kind: 1, color: "FF".into(), visible_ms: 3000, message: "FF0000The server restarts in 5 minutes".into() }
        );
        let mut w = Writer::new();
        w.u16(3).u8(2).u32(77).string("Tower");
        assert_eq!(read(op::SIEGE_ALERT, &w), NoticeEvent::SiegeAlert { zone_group: 3, alert: 2, character_id: 77, name: "Tower".into() });
        // a character-less alert has no name
        let mut w = Writer::new();
        w.u16(3).u8(0).u32(0);
        assert_eq!(read(op::SIEGE_ALERT, &w), NoticeEvent::SiegeAlert { zone_group: 3, alert: 0, character_id: 0, name: String::new() });
        let mut w = Writer::new();
        w.u16(4).u32(9).u32(10).bool(true);
        assert_eq!(read(op::SIEGE_MEMBER, &w), NoticeEvent::SiegeMember { zone_group: 4, expedition_id: 9, character_id: 10, added: true });
        let mut w = Writer::new();
        w.u16(5).i32(1234);
        assert_eq!(read(op::HONOR_POINT_SUM, &w), NoticeEvent::HonorPointSum { zone_group: 5, sum: 1234 });
        let mut w = Writer::new();
        w.u16(5).i32(7);
        assert_eq!(read(op::DOMINION_TAX_RATE, &w), NoticeEvent::DominionTaxRate { zone_group: 5, tax_rate: 7 });
    }

    #[test]
    fn tower_defense_messages_decode() {
        let mut w = Writer::new();
        w.u8(1).u32(2).u16(3).u32(4).u32(5).bc(6).i64(coord_to_long(100.5)).i64(coord_to_long(-20.0)).f32(30.0).u32(1);
        let NoticeEvent::TowerDefList(list) = read(op::TOWER_DEF_LIST, &w) else { panic!() };
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].key, TowerDefKey { tower_def_id: 2, zone_group: 3 });
        assert_eq!((list[0].zone_id, list[0].spot_id, list[0].target_obj_id, list[0].step), (4, 5, 6, 1));
        assert_eq!(list[0].pos, Pos::new(100.5, -20.0, 30.0));
        assert!(NoticeEvent::read(op::TOWER_DEF_LIST, &mut Reader::new(&[101])).is_err());
        let mut w = Writer::new();
        w.u32(2).u16(3).u32(4).u32(2);
        let key = TowerDefKey { tower_def_id: 2, zone_group: 3 };
        assert_eq!(read(op::TOWER_DEF_WAVE_START, &w), NoticeEvent::TowerDefWaveStarted { key, event_zone_id: 4, step: 2 });
        let mut w = Writer::new();
        w.u32(2).u16(3).u32(4);
        assert_eq!(read(op::TOWER_DEF_START, &w), NoticeEvent::TowerDefStarted { key, event_zone_id: 4 });
        assert_eq!(read(op::TOWER_DEF_END, &w), NoticeEvent::TowerDefEnded { key, event_zone_id: 4 });
    }

    #[test]
    fn small_gameplay_messages_decode() {
        let mut w = Writer::new();
        w.bc(0x123456).string("Thief").string("Guard");
        assert_eq!(
            read(op::CRIMINAL_ARRESTED, &w),
            NoticeEvent::CriminalArrested { criminal_obj_id: 0x123456, criminal: "Thief".into(), arrestor: "Guard".into() }
        );
        let mut w = Writer::new();
        w.string("A").string("B");
        assert_eq!(read(op::BOT_SUSPECT_REPORTED, &w), NoticeEvent::BotSuspectReported { reporter: "A".into(), suspect: "B".into() });
        let mut w = Writer::new();
        w.i32(150);
        assert_eq!(read(op::PROCESSING_INSTANCE, &w), NoticeEvent::ProcessingInstance { zone_id: 150 });
        let mut w = Writer::new();
        w.u32(12).u64(3);
        assert_eq!(read(op::TUTORIAL_SAVED, &w), NoticeEvent::TutorialSaved { tutorial_id: 12, state: 3 });
        let mut w = Writer::new();
        w.u8(2).u32(900).bc(42);
        assert_eq!(read(op::PLAY_SOUND, &w), NoticeEvent::PlaySound(SoundCue::Bubble { bubble_id: 900, npc_obj_id: 42 }));
        assert!(NoticeEvent::read(op::PLAY_SOUND, &mut Reader::new(&[3, 0, 0, 0, 0])).is_err());
        let mut w = Writer::new();
        w.i32(2).u16(1).u16(2).u32(30001).i32(110).u32(30002).i32(95);
        assert_eq!(
            read(op::SPECIALTY_CURRENT, &w),
            NoticeEvent::SpecialtyCurrent { from_zone_group: 1, to_zone_group: 2, ratios: vec![(30001, 110), (30002, 95)] }
        );
    }

    #[test]
    fn player_music_decodes() {
        let mut w = Writer::new();
        w.i32(3).bc(7).string("Bard").blob(&[1, 2, 3]);
        assert_eq!(read(op::SEND_USER_MUSIC, &w), NoticeEvent::UserMusic { obj_id: 7, author: "Bard".into(), midi: vec![1, 2, 3] });
        let mut w = Writer::new();
        w.u32(5).u8(1).i32(3).string("Song").string("abc\0");
        let note = UserNote { song_id: 5, tooltip: true, title: "Song".into(), notes: "abc".into() };
        assert_eq!(read(op::USER_NOTE_LOADED, &w), NoticeEvent::UserNoteLoaded(Some(note)));
        assert_eq!(NoticeEvent::read(op::USER_NOTE_LOADED, &mut Reader::new(&[])).unwrap(), NoticeEvent::UserNoteLoaded(None));
        let mut w = Writer::new();
        w.bc(7);
        assert_eq!(read(op::PAUSE_USER_MUSIC, &w), NoticeEvent::UserMusicPaused { obj_id: 7 });
    }
}
