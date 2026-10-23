//! Small read-only composition and equipped-instrument definitions.
use rusqlite::Connection;
use std::collections::BTreeMap;

#[derive(Default)]
pub(in crate::hud) struct Tables {
    pub(super) limits: BTreeMap<u8, usize>,
    instruments: BTreeMap<u32, aa_midi::mml::Voice>,
}

impl Tables {
    pub(in crate::hud) fn load(conn: &Connection) -> rusqlite::Result<Self> {
        let mut out = Self::default();
        let mut query =
            conn.prepare("SELECT step,note_length FROM music_note_limits ORDER BY step")?;
        for row in query.query_map([], |r| Ok((r.get(0)?, r.get::<_, u32>(1)? as usize)))? {
            let (step, limit) = row?;
            out.limits.insert(step, limit);
        }
        // Native39083960 resolves instrument_sounds first, then the two held types.
        let mut query = conn.prepare("SELECT w.item_id,s.midi,w.holdable_id FROM item_weapons w LEFT JOIN instrument_sounds s ON s.item_id=w.item_id WHERE w.holdable_id IN (21,23) OR s.midi IS NOT NULL")?;
        for row in query.query_map([], |r| {
            Ok((
                r.get::<_, u32>(0)?,
                r.get::<_, Option<u8>>(1)?,
                r.get::<_, u32>(2)?,
            ))
        })? {
            let (item, midi, held) = row?;
            let program = midi.filter(|v| *v != 0).or(match held {
                21 => Some(106),
                23 => Some(73),
                _ => None,
            });
            if let Some(program) = program {
                out.instruments
                    .insert(item, aa_midi::mml::Voice::melodic(program));
            }
        }
        Ok(out)
    }

    pub(super) fn grade(&self, notes: &str) -> Option<u8> {
        let length = notes.chars().count();
        (length != 0).then_some(())?;
        self.limits
            .iter()
            .find_map(|(&grade, &limit)| (length <= limit).then_some(grade))
    }

    pub(super) fn voice(&self, item: u32) -> Option<aa_midi::mml::Voice> {
        self.instruments.get(&item).copied()
    }
}
