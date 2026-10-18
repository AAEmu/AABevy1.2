//! Original 1.2 scheduled-event start receipts. The active lane defines only its opcode.
use crate::{
    session::Event,
    wire::{Reader, Result, WireError},
};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScheduledEventStarted {
    pub id: u32,
    pub values: [i32; 21],
    /// All wire bits retained; native expands only low21 into change flags.
    pub changes: u32,
    pub priority: u8,
    pub start_date: u8,
    pub period_date: u8,
    pub hour: u8,
    pub minute: u8,
    pub period_minutes: i32,
    pub week_day: u8,
    /// Exact source bytes, including NUL/non-UTF8; native destination is127 bytes plus terminator.
    pub open_message: Vec<u8>,
}
impl ScheduledEventStarted {
    pub fn open_message_text(&self) -> String {
        String::from_utf8_lossy(&self.open_message).trim_matches('\0').to_string()
    }
    /// Source-native serializer397b01f0?398a7ed0/398a7d80. No lane writer is claimed.
    pub fn read_native(r: &mut Reader) -> Result<Self> {
        let id = r.u32()?;
        let mut values = [0; 21];
        for value in &mut values {
            *value = r.i32()?;
        }
        let changes = r.u32()?;
        let priority = r.u8()?;
        let start_date = r.u8()?;
        let period_date = r.u8()?;
        let hour = r.u8()?;
        let minute = r.u8()?;
        let period_minutes = r.i32()?;
        let week_day = r.u8()?;
        let open_message = r.blob()?.to_vec();
        if open_message.len() > 127 {
            return Err(WireError::Invalid("scheduled-event open message exceeds native 127-byte destination".into()));
        }
        Ok(Self {
            id,
            values,
            changes,
            priority,
            start_date,
            period_date,
            hour,
            minute,
            period_minutes,
            week_day,
            open_message,
        })
    }
}
/// Latest genuine start per ID. This is receipt history, not an invented active/priority merge policy.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "bevy", derive(bevy::ecs::resource::Resource))]
pub struct ScheduledEvents {
    pub starts: HashMap<u32, ScheduledEventStarted>,
}
impl ScheduledEvents {
    pub fn apply(&mut self, e: &Event) -> bool {
        match e {
            Event::ScheduledEventStarted(start) => self.starts.insert(start.id, start.clone()).as_ref() != Some(start),
            Event::CharacterSelected(_) | Event::Kicked { .. } | Event::LeftWorld { .. } | Event::Disconnected { .. } => {
                let changed = !self.starts.is_empty();
                self.starts.clear();
                changed
            }
            _ => false,
        }
    }
}
#[cfg(test)]
mod tests;
