//! Native player-score requests. Preview/staging are local; playing and pausing use skills.
//!
//! `UploadNotes` queues a composition for the sheet item's skill. `CacheMidi`
//! supplies the compiled performance for the play skill. Neither request acknowledges a save
//! or starts playback. See `docs/aa-net.md` for the original call sequence and server differences.

use crate::items::container;
use crate::opcodes::cs;
use crate::wire::Writer;

mod compose;
pub use compose::{COMPOSE_SKILL_ID, MusicComposeRequest};

pub const MAX_TITLE_BYTES: usize = 96;
pub const MAX_NOTES_BYTES: usize = 12_000;
pub const MAX_MIDI_BYTES: usize = 32_768;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MusicCommand {
    /// CS122. Native SaveNotes resolves this item in the main bag. The app also checks editor
    /// length/proficiency, compiles and stages the notes before starting the item skill.
    /// The native skill-start action then uploads that staged data.
    UploadNotes {
        item_id: u64,
        /// Wire `SlotType` (main bag = `items::container::INVENTORY`, 2).
        slot_type: u8,
        /// Zero-based physical bag slot, not a sorted/UI index.
        slot: u8,
        title: String,
        notes: String,
    },
    /// CS121. Exact SMF bytes, without an added NUL; does not itself play the song.
    CacheMidi { midi: Vec<u8> },
    /// CS123. A cache miss uses (tooltip, false). A locally cached playing score uses
    /// (false, true) to request only the memorized-score buff. There is no aa-net score cache.
    RequestNotes {
        note_id: u64,
        is_tooltip: bool,
        show_buff_only: bool,
    },
    /// CS124, empty body. Native pause-action processing and actual own playback voice end
    /// send this. It does not start the pause skill and is never sent automatically here.
    End,
}

impl MusicCommand {
    pub fn opcode(&self) -> u16 {
        match self {
            Self::CacheMidi { .. } => cs::CSSendUserMusicPacket,
            Self::UploadNotes { .. } => cs::CSSaveUserMusicNotesPacket,
            Self::RequestNotes { .. } => cs::CSRequestMusicNotesPacket,
            Self::End => cs::CSEndMusicPacket,
        }
    }

    /// Packet-buffer bounds, separate from the editor's 12/1000-character and proficiency limits.
    /// Invalid inputs are rejected, never silently truncated as the native fixed buffers can be.
    pub fn validate(&self) -> std::result::Result<(), &'static str> {
        match self {
            Self::UploadNotes {
                item_id,
                slot_type,
                title,
                notes,
                ..
            } => {
                if *item_id == 0 || *slot_type != container::INVENTORY {
                    return Err(
                        "music upload requires a sheet item in the main bag (wire SlotType 2)",
                    );
                }
                if title.is_empty() || title.len() > MAX_TITLE_BYTES || title.contains('\0') {
                    return Err("music title must contain 1..96 UTF-8 bytes without NUL");
                }
                if notes.is_empty() || notes.len() > MAX_NOTES_BYTES || notes.contains('\0') {
                    return Err("music notes must contain 1..12000 UTF-8 bytes without NUL");
                }
            }
            Self::CacheMidi { midi } if midi.is_empty() || midi.len() > MAX_MIDI_BYTES => {
                return Err("music MIDI must contain 1..32768 bytes");
            }
            _ => {}
        }
        Ok(())
    }

    /// Call `validate` first when using this low-level writer outside `Command::Music`.
    pub fn write(&self, w: &mut Writer) {
        match self {
            Self::UploadNotes {
                item_id,
                slot_type,
                slot,
                title,
                notes,
            } => {
                w.u32(notes.len() as u32)
                    .u64(*item_id)
                    .u8(*slot_type)
                    .u8(*slot)
                    .string(title);
                w.u16((notes.len() + 1) as u16).raw(notes.as_bytes()).u8(0);
            }
            Self::CacheMidi { midi } => {
                w.u32(midi.len() as u32).u16(midi.len() as u16).raw(midi);
            }
            Self::RequestNotes {
                note_id,
                is_tooltip,
                show_buff_only,
            } => {
                w.u64(*note_id).bool(*is_tooltip).bool(*show_buff_only);
            }
            Self::End => {}
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;
#[cfg(test)]
pub(crate) mod compose_tests;
