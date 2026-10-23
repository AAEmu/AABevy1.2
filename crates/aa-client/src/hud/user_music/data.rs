//! Small read-only composition and equipped-instrument definitions.
use rusqlite::Connection;
use std::collections::BTreeMap;

#[derive(Default)]
pub(in crate::hud) struct Tables {
    pub(super) limits: BTreeMap<u8, usize>,
    instruments: BTreeMap<u32, aa_midi::mml::Voice>,
    actions: BTreeMap<u32, Vec<u8>>,
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
        // Native39083960 resolves positive instrument_sounds first, then items.category_id.
        // Holdable21/23 also contain category96 weapons and are not equivalent.
        let mut query = conn.prepare("SELECT i.id,s.midi,i.category_id FROM items i LEFT JOIN instrument_sounds s ON s.item_id=i.id WHERE i.category_id IN (80,81) OR s.midi>0")?;
        for row in query.query_map([], |r| {
            Ok((
                r.get::<_, u32>(0)?,
                r.get::<_, Option<u8>>(1)?,
                r.get::<_, u32>(2)?,
            ))
        })? {
            let (item, midi, category) = row?;
            let program = midi.filter(|v| *v != 0).or(match category {
                80 => Some(106),
                81 => Some(73),
                _ => None,
            });
            if let Some(program) = program {
                out.instruments
                    .insert(item, aa_midi::mml::Voice::melodic(program));
            }
        }
        // Executed native catalog loaders attach all rows; application_method is a
        // target-selection mode, not a Started/Fired timing gate.
        let mut query = conn.prepare("SELECT se.skill_id,s.special_effect_type_id FROM skill_effects se JOIN effects e ON e.id=se.effect_id AND e.actual_type='SpecialEffect' JOIN special_effects s ON s.id=e.actual_id WHERE s.special_effect_type_id IN (93,94) ORDER BY se.id")?;
        for row in query.query_map([], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, u8>(1)?)))? {
            let (skill, action) = row?;
            out.actions.entry(skill).or_default().push(action);
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

    pub(super) fn actions(&self, skill: u32) -> &[u8] {
        self.actions
            .get(&skill)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }
}
