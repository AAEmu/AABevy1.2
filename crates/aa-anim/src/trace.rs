//! Opt-in record of clip starts, for aa-client's `AA_SKILL_TRACE` (`docs/aa-client-combat.md`, "Skill trace").
//!
//! Off by default: [`enable`] turns it on once at start-up. While it is off every hook is one relaxed atomic load. While it is on,
//! each [`Animator`](crate::Animator) keeps the clips it started (the base clip, the partial-body layer, and the clips it
//! switches to by itself: the return clip of a one-shot, a random idle variant) until the owner drains them with
//! [`Animator::take_starts`](crate::Animator::take_starts). The animator does not know its entity or role; the owner does.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use web_time::{SystemTime, UNIX_EPOCH};

use crate::pose::BoundClip;

static ENABLED: AtomicBool = AtomicBool::new(false);

/// Starts recording clip starts on every animator (call once, before any clip plays).
pub fn enable() {
    ENABLED.store(true, Ordering::Relaxed);
}

/// Whether clip starts are recorded.
#[inline]
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Layer number of the base clip.
pub const BASE: u8 = 0;
/// Layer number of the bone-masked partial-body layer ([`Animator::play_layer`](crate::Animator::play_layer)).
pub const UPPER: u8 = 1;

/// One clip start.
#[derive(Clone, Debug)]
pub struct ClipStart {
    /// Unix time in microseconds when the clip started.
    pub wall_us: u64,
    /// [`BASE`] or [`UPPER`].
    pub layer: u8,
    pub looping: bool,
    /// Seconds of cross-fade (or fade-in for a layer).
    pub blend: f32,
    /// The clip's normalised pak path, when it was bound by the clip library.
    pub file: Option<Arc<str>>,
    /// The `.cal` animation name, lower case, when the clip was bound through a `.cal` ([`BoundClip::name`]).
    pub name: Option<Arc<str>>,
}

impl ClipStart {
    pub(crate) fn of(clip: &BoundClip, layer: u8, looping: bool, blend: f32) -> ClipStart {
        let wall_us = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_micros() as u64);
        ClipStart { wall_us, layer, looping, blend, file: clip.source.clone(), name: clip.name.clone() }
    }
}
