//! `.cal` files: the animation name table of a character model.
//!
//! Plain text (Korean comments in CP949, so read as latin-1), one directive per line:
//!
//! ```text
//! #filepath = animations\nuian\male          folder the following paths are relative to
//! #BaseModelHeight = 1.013                   pelvis height of this model (metres)
//! #AnimModelHeight = 0.964                   pelvis height the following clips were authored for
//! $TracksDBH = animations\nuian\male\nu_m.dbh
//! $AnimEventDatabase = ...animevents
//! $Include = objects\characters\global_player.cal      (parsed in place, inherits the state)
//! fist_ac_stage_idle = fist\fist_ac_stage_idle.caf     name = path   (// starts a comment)
//! ```
//!
//! `#filepath` / `#AnimModelHeight` may appear several times (the elf female uses Nuian clips
//! first, then its own); every entry remembers the values in force when it was declared. A
//! clip authored for a model of another height needs its position tracks scaled by
//! `BaseModelHeight / AnimModelHeight` ([`CalEntry::position_scale`]).
//!
//! `name = x.fsq` lines are facial sequences (kept separately, not decoded here) and
//! `name = x.lmg` lines locomotion groups.

use std::collections::HashMap;

use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq)]
pub struct CalEntry {
    /// Full pak path (`animations/nuian/male/fist/fist_ac_stage_idle.caf`, lowercase, `/`).
    pub path: String,
    /// `#AnimModelHeight` in force when the entry was declared (0 = unknown).
    pub anim_height: f32,
}

#[derive(Clone, Debug, Default)]
pub struct Cal {
    /// `#BaseModelHeight` (last value; 0 = unknown).
    pub base_height: f32,
    pub tracks_dbh: Option<String>,
    pub anim_events: Option<String>,
    pub sound_events: Option<String>,
    /// Lowercase animation name -> `.caf` (later declarations replace earlier ones).
    pub animations: HashMap<String, CalEntry>,
    /// Lowercase name -> `.fsq` facial sequence.
    pub facial: HashMap<String, CalEntry>,
    /// Lowercase name (with its leading `_`) -> `.lmg`.
    pub locomotion: HashMap<String, CalEntry>,
}

impl CalEntry {
    /// Scale for position tracks so a clip authored for `anim_height` fits a model of `base_height`.
    pub fn position_scale(&self, base_height: f32) -> f32 {
        if self.anim_height > 0.0 && base_height > 0.0 { base_height / self.anim_height } else { 1.0 }
    }
}

impl Cal {
    /// Parses `.cal` text; `include` fetches the bytes of `$Include`d files by pak path.
    pub fn parse(text: &str, include: &mut dyn FnMut(&str) -> Option<Vec<u8>>) -> Cal {
        let mut cal = Cal::default();
        let mut state = State::default();
        cal.parse_into(text, include, &mut state, 0);
        cal
    }

    /// Loads a `.cal` (and its includes) from the global pak.
    pub fn load(path: &str) -> Result<Cal> {
        let bytes = aa_cry::read(path)?;
        let mut loader = |p: &str| aa_cry::read(p).ok();
        Ok(Cal::parse(&latin1(&bytes), &mut loader))
    }

    /// Looks up an animation by (case-insensitive) name.
    pub fn animation(&self, name: &str) -> Option<&CalEntry> {
        self.animations.get(&name.to_ascii_lowercase())
    }

    fn parse_into(
        &mut self,
        text: &str,
        include: &mut dyn FnMut(&str) -> Option<Vec<u8>>,
        state: &mut State,
        depth: u32,
    ) {
        for raw in text.lines() {
            let line = raw.split("//").next().unwrap_or("").trim();
            let Some((key, value)) = line.split_once('=') else { continue };
            let (key, value) = (key.trim(), value.trim());
            if key.is_empty() || value.is_empty() {
                continue;
            }
            let lower = key.to_ascii_lowercase();
            match lower.as_str() {
                "#filepath" => state.file_path = norm(value),
                "#basemodelheight" => self.base_height = value.parse().unwrap_or(self.base_height),
                "#animmodelheight" => state.anim_height = value.parse().unwrap_or(0.0),
                "$tracksdbh" => self.tracks_dbh = Some(norm(value)),
                "$animeventdatabase" => self.anim_events = Some(norm(value)),
                "$soundeventdatabase" => self.sound_events = Some(norm(value)),
                "$include" => {
                    if depth < 8 {
                        if let Some(b) = include(&norm(value)) {
                            self.parse_into(&latin1(&b), include, state, depth + 1);
                        }
                    }
                }
                _ if lower.starts_with(['#', '$']) => {}
                _ => {
                    let rel = norm(value);
                    let ext = rel.rsplit_once('.').map(|e| e.1).filter(|e| e.len() <= 4);
                    let (rel, ext) = match ext {
                        Some(e) => (rel.clone(), e.to_string()),
                        None => (format!("{rel}.caf"), "caf".to_string()),
                    };
                    let path = if state.file_path.is_empty() {
                        rel
                    } else {
                        format!("{}/{}", state.file_path, rel)
                    };
                    let entry = CalEntry { path, anim_height: state.anim_height };
                    let map = match ext.as_str() {
                        "caf" => &mut self.animations,
                        "fsq" => &mut self.facial,
                        "lmg" => &mut self.locomotion,
                        _ => continue,
                    };
                    map.insert(lower, entry);
                }
            }
        }
    }
}

#[derive(Default)]
struct State {
    file_path: String,
    anim_height: f32,
}

/// Lowercase, `/` separators, no leading `./`.
fn norm(p: &str) -> String {
    let p = p.replace('\\', "/").to_ascii_lowercase();
    p.trim_start_matches("./").trim_matches('/').to_string()
}

/// Every byte as one char (the CP949 comments become garbage, ASCII is unaffected).
pub(crate) fn latin1(b: &[u8]) -> String {
    b.iter().map(|&c| c as char).collect()
}

/// Convenience for callers that only want the clip bytes of an entry.
pub fn load_clip(entry: &CalEntry) -> Result<crate::Clip> {
    if !entry.path.ends_with(".caf") {
        return Err(Error::Format(format!("{} is not a .caf", entry.path)));
    }
    crate::Clip::load(&entry.path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_state_and_includes() {
        let global = "_FIST_MO_NORMAL_WALK = lmg_files\\fist_mo_normal_walk.lmg\nfoo = bar\\foo.caf\n";
        let cal = "#filepath = animations\\nuian\\female\n#BaseModelHeight = 1.013\n#AnimModelHeight = 0.964\n\
                   $TracksDBH = animations\\elf\\female\\el_f.dbh\n$Include = objects\\characters\\global_player.cal\n\
                   fist_ac_a = fist\\fist_ac_a.caf // comment \u{b0}\u{a1}\n\
                   #filepath = animations\\elf\\female\n#AnimModelHeight = 1.013\n\
                   fist_ac_a = fist\\fist_ac_a.caf\nfist_ac_a = facial/fist_ac_a.fsq\nnoext = fist\\noext\n";
        let mut inc = |p: &str| {
            assert_eq!(p, "objects/characters/global_player.cal");
            Some(global.as_bytes().to_vec())
        };
        let c = Cal::parse(cal, &mut inc);
        assert_eq!(c.base_height, 1.013);
        assert_eq!(c.tracks_dbh.as_deref(), Some("animations/elf/female/el_f.dbh"));
        assert_eq!(c.animation("FOO").unwrap().path, "animations/nuian/female/bar/foo.caf");
        assert_eq!(c.animation("FOO").unwrap().anim_height, 0.964);
        let a = c.animation("fist_ac_a").unwrap();
        assert_eq!(a.path, "animations/elf/female/fist/fist_ac_a.caf");
        assert_eq!(a.position_scale(c.base_height), 1.0);
        assert_eq!(c.facial["fist_ac_a"].path, "animations/elf/female/facial/fist_ac_a.fsq");
        assert_eq!(c.animation("noext").unwrap().path, "animations/elf/female/fist/noext.caf");
        assert!(c.locomotion.contains_key("_fist_mo_normal_walk"));
        let foo = c.animation("foo").unwrap();
        assert!((foo.position_scale(c.base_height) - 1.013 / 0.964).abs() < 1e-6);
    }

    #[test]
    fn real_cal_files() {
        if !aa_cry::pak_available() {
            return;
        }
        for (cal, race_dir) in [
            ("objects/characters/nuian/male/nude/nu_m_base.cal", "animations/nuian/male/"),
            ("objects/characters/elf/female/nude/el_f_base.cal", "animations/elf/female/"),
            ("objects/characters/hariharan/male/nude/ha_m_base.cal", "animations/hariharan/male/"),
            ("objects/characters/ferre/female/nude/fe_f_base.cal", "animations/ferre/female/"),
        ] {
            let c = Cal::load(cal).unwrap();
            let idle = c.animation("fist_ac_stage_idle").unwrap_or_else(|| panic!("{cal}: no stage idle"));
            assert!(idle.path.starts_with(race_dir), "{cal}: {}", idle.path);
            assert!(aa_pak::Pak::global().exists(&idle.path), "{}", idle.path);
            assert!(c.base_height > 0.5, "{cal}");
            assert!(c.locomotion.contains_key("_fist_mo_normal_run"), "{cal}");
        }
    }
}
