//! Opt-in record of every event the engine is asked to play, for aa-client's `AA_SKILL_TRACE`
//! (`docs/aa-client-combat.md`, "Skill trace").
//!
//! Off by default: [`enable`] turns it on once at start-up. While it is off [`Engine::play_with`](crate::Engine::play_with) pays
//! one relaxed atomic load. While it is on, each call leaves a [`Started`] that the owner takes with [`take`] (once a frame).
//! The record is made when the sound is asked for, whether or not the engine is audible, so a muted run traces the same sounds.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use bevy::math::Vec3;
use web_time::{SystemTime, UNIX_EPOCH};

static ENABLED: AtomicBool = AtomicBool::new(false);
static STARTED: Mutex<Vec<Started>> = Mutex::new(Vec::new());

/// At most this many starts wait for [`take`]; a run that never takes them does not grow without bound.
const KEPT: usize = 16_384;

/// Starts recording.
pub fn enable() {
    ENABLED.store(true, Ordering::Relaxed);
}

#[inline]
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// One sound the engine was asked to play.
#[derive(Clone, Debug, PartialEq)]
pub struct Started {
    /// Unix time in microseconds.
    pub wall_us: u64,
    /// The event path as asked for (`sounds/x2skills:wild:fire_arrow_form`).
    pub path: String,
    /// The place in the world (Bevy space) of a 3D sound.
    pub at: Option<Vec3>,
}

pub(crate) fn record(path: &str, at: Option<Vec3>) {
    let wall_us = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_micros() as u64);
    let mut kept = STARTED.lock().unwrap_or_else(|e| e.into_inner());
    if kept.len() < KEPT {
        kept.push(Started { wall_us, path: path.to_string(), at });
    }
}

/// The sounds started since the last call, oldest first.
pub fn take() -> Vec<Started> {
    std::mem::take(&mut *STARTED.lock().unwrap_or_else(|e| e.into_inner()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Engine, PlayOptions, Settings};

    #[test]
    fn a_muted_engine_records_what_it_is_asked_to_play() {
        let engine = Engine::start(Settings { audible: false, ..Settings::default() });
        enable();
        assert!(enabled());
        let at = Vec3::new(1.0, 2.0, 3.0);
        engine.play_at("sounds/x2skills:test:positional", at);
        engine.play_2d("sounds/x2skills:test:flat");
        engine.play_with("sounds/x2skills:test:with", None, PlayOptions::default());
        let mine: Vec<_> = take().into_iter().filter(|s| s.path.starts_with("sounds/x2skills:test:")).collect();
        let summary: Vec<_> = mine.iter().map(|s| (s.path.as_str(), s.at)).collect();
        assert_eq!(
            summary,
            [("sounds/x2skills:test:positional", Some(at)), ("sounds/x2skills:test:flat", None), ("sounds/x2skills:test:with", None)]
        );
        assert!(mine.iter().all(|s| s.wall_us > 1_600_000_000_000_000), "Unix microseconds");
        assert!(take().iter().all(|s| !s.path.starts_with("sounds/x2skills:test:")), "taking them empties the record");
    }
}
