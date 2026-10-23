//! Bounded, reusable mono float PCM for the existing engine (no event or bank required).

use std::sync::{Arc, atomic::{AtomicU8, Ordering}};

use bevy::math::Vec3;
use kira::Frame;

use crate::{Category, cache::Sample};

/// Maximum mono samples per clip, independently of its duration (44 MiB of stereo frames).
pub const MAX_PCM_SAMPLES: usize = 5_760_000;
/// Maximum duration per clip, including any silence or synthesis tail.
pub const MAX_PCM_SECONDS: u32 = 240;
/// Maximum accepted PCM submissions per engine, including queued and fading voices.
pub const MAX_PCM_VOICES: usize = 8;

const STARTED: u8 = 1;
const FINISHED: u8 = 2;
const CANCELLED: u8 = 4;

/// Sticky facts about one [`crate::Engine::play_pcm_tracked`] request, independent of
/// the shared completion drain. Clones retain only atomic status, never PCM or the engine.
#[derive(Clone, Debug)]
pub struct PcmPlaybackObserver(Arc<AtomicU8>);

impl PcmPlaybackObserver {
    /// Whether Kira accepted a mixer handle for this request. This does not prove a mixer
    /// callback ran, native FMOD sound creation, or audibility (zero gains still admit).
    pub fn was_started(&self) -> bool {
        self.0.load(Ordering::Acquire) & STARTED != 0
    }

    /// Whether the request ended, including cancellation, refusal, device/mixer failure,
    /// natural completion, stealing, stop or teardown. This does not identify the reason.
    /// Once true, `was_started()` is final. Check this before `was_started()` when polling.
    pub fn is_finished(&self) -> bool {
        self.0.load(Ordering::Acquire) & FINISHED != 0
    }

    pub(crate) fn new() -> Self {
        Self(Arc::new(AtomicU8::new(0)))
    }

    pub(crate) fn start(&self) {
        let _ = self.0.try_update(Ordering::Release, Ordering::Relaxed, |s| {
            (s & FINISHED == 0).then_some(s | STARTED)
        });
    }

    pub(crate) fn finish(&self) {
        self.0.fetch_or(FINISHED, Ordering::Release);
    }

    pub(crate) fn cancel(&self) {
        self.0.fetch_or(CANCELLED, Ordering::Relaxed);
    }

    pub(crate) fn cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed) & CANCELLED != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PcmError {
    #[error("PCM sample rate must be in 8000..=192000 Hz")]
    Rate,
    #[error("PCM is empty")]
    Empty,
    #[error("PCM exceeds 240 seconds or 5760000 mono samples")]
    TooLong,
    #[error("PCM contains a nonfinite sample")]
    NonFinite,
    #[error("PCM samples must be within -1..=1")]
    Amplitude,
    #[error("PCM volume must be finite in 0..=1, positions finite, and distances 0 <= min < max")]
    Options,
    #[error("all eight PCM submission slots are occupied")]
    Busy,
    #[error("audio engine has stopped")]
    EngineStopped,
}

/// Validated, non-looping mono PCM. Clones and play requests share the stereo frame allocation.
/// The caller must budget retained clips and concurrent construction; clips are outside the bank cache.
#[derive(Clone)]
pub struct PcmClip(pub(crate) Arc<Sample>);

impl PcmClip {
    /// Validates *all* input before allocating stereo frames. Run this on the synthesis worker;
    /// conversion is synchronous and never runs on the engine thread or audio callback.
    /// Both the 240-second duration and [`MAX_PCM_SAMPLES`] limits apply, at every supported rate.
    pub fn mono(rate: u32, samples: &[f32]) -> Result<Self, PcmError> {
        validate_length(rate, samples.len())?;
        if samples.iter().any(|v| !v.is_finite()) {
            return Err(PcmError::NonFinite);
        }
        if samples.iter().any(|v| !(-1.0..=1.0).contains(v)) {
            return Err(PcmError::Amplitude);
        }
        let frames = samples.iter().map(|&v| Frame { left: v, right: v }).collect();
        Ok(Self(Arc::new(Sample { rate, frames, loop_points: None })))
    }

    pub fn rate(&self) -> u32 {
        self.0.rate
    }

    pub fn samples(&self) -> usize {
        self.0.frames.len()
    }

    pub fn seconds(&self) -> f32 {
        self.0.seconds()
    }

    /// Stereo frame storage in bytes, shared by all clones and submissions of this clip.
    pub fn bytes(&self) -> usize {
        self.0.bytes()
    }
}

fn validate_length(rate: u32, samples: usize) -> Result<(), PcmError> {
    if !(8_000..=192_000).contains(&rate) {
        return Err(PcmError::Rate);
    }
    if samples == 0 {
        return Err(PcmError::Empty);
    }
    if samples > MAX_PCM_SAMPLES || samples as u64 > u64::from(rate) * u64::from(MAX_PCM_SECONDS) {
        return Err(PcmError::TooLong);
    }
    Ok(())
}

/// Explicit application policy, not distances recovered from FMOD: linear gain from one at
/// `min_distance` to zero at `max_distance`. Kira supplies panning with strength one.
/// Out-of-range clips still run silently to completion and can move back into range.
#[derive(Clone, Copy, Debug)]
pub struct PcmSpatial {
    pub position: Vec3,
    pub min_distance: f32,
    pub max_distance: f32,
}

impl PcmSpatial {
    pub(crate) fn gain(self, listener: Vec3) -> f32 {
        let distance = self.position.distance(listener);
        ((self.max_distance - distance) / (self.max_distance - self.min_distance)).clamp(0.0, 1.0)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PcmOptions {
    pub category: Category,
    /// Linear multiplier in 0..=1; no hidden synth gain or normalization.
    pub volume: f32,
    /// `None` is centered 2D; distances must be specified for 3D.
    pub spatial: Option<PcmSpatial>,
    /// Same voice admission rule as events: lower is more important, oldest loses ties.
    pub priority: u32,
}

impl PcmOptions {
    /// Explicit category, unity gain, 2D, priority 128. PCM has no FEV parameters, loop or reverb send.
    pub fn new(category: Category) -> Self {
        Self { category, volume: 1.0, spatial: None, priority: 128 }
    }

    pub(crate) fn validate(&self) -> Result<(), PcmError> {
        if !valid_volume(self.volume) || self.spatial.is_some_and(|s| {
            !s.position.is_finite() || !s.min_distance.is_finite() || !s.max_distance.is_finite()
                || s.min_distance < 0.0 || s.max_distance <= s.min_distance
        }) {
            return Err(PcmError::Options);
        }
        Ok(())
    }

    pub(crate) fn gain(self, listener: Vec3) -> f32 {
        self.volume * self.spatial.map_or(1.0, |s| s.gain(listener))
    }
}

pub(crate) fn valid_volume(volume: f32) -> bool {
    volume.is_finite() && (0.0..=1.0).contains(&volume)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_before_conversion_and_shares_frames() {
        assert!(matches!(PcmClip::mono(0, &[0.0]), Err(PcmError::Rate)));
        assert!(matches!(PcmClip::mono(7_999, &[0.0]), Err(PcmError::Rate)));
        assert!(matches!(PcmClip::mono(192_001, &[0.0]), Err(PcmError::Rate)));
        assert!(matches!(PcmClip::mono(24_000, &[]), Err(PcmError::Empty)));
        for v in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(matches!(PcmClip::mono(24_000, &[0.0, v]), Err(PcmError::NonFinite)));
        }
        for v in [-1.01, 1.01] {
            assert!(matches!(PcmClip::mono(24_000, &[v]), Err(PcmError::Amplitude)));
        }
        let clip = PcmClip::mono(24_000, &[-1.0, 0.25, 1.0]).unwrap();
        assert_eq!((clip.rate(), clip.samples(), clip.bytes()), (24_000, 3, 24));
        assert_eq!(clip.0.frames[1], Frame { left: 0.25, right: 0.25 });
        assert!(Arc::ptr_eq(&clip.0.frames, &clip.clone().0.frames));
        assert_eq!(clip.0.loop_points, None);
    }

    #[test]
    fn sample_and_duration_caps_both_apply_without_large_test_allocations() {
        assert_eq!(validate_length(24_000, MAX_PCM_SAMPLES), Ok(()));
        assert_eq!(validate_length(24_000, MAX_PCM_SAMPLES + 1), Err(PcmError::TooLong));
        assert_eq!(validate_length(8_000, 8_000 * 240), Ok(()));
        assert_eq!(validate_length(8_000, 8_000 * 240 + 1), Err(PcmError::TooLong));
        assert_eq!(validate_length(192_000, MAX_PCM_SAMPLES), Ok(()));
        assert_eq!(validate_length(192_000, MAX_PCM_SAMPLES + 1), Err(PcmError::TooLong));
        assert_eq!(validate_length(24_000, usize::MAX), Err(PcmError::TooLong));
    }

    #[test]
    fn spatial_gain_is_explicit_and_invalid_options_fail() {
        let mut opts = PcmOptions::new(Category::Music);
        assert_eq!(opts.gain(Vec3::splat(100.0)), 1.0);
        let mut spatial = PcmSpatial { position: Vec3::X * 10.0, min_distance: 2.0, max_distance: 18.0 };
        opts.spatial = Some(spatial);
        assert_eq!(opts.gain(Vec3::ZERO), 0.5);
        assert_eq!(opts.gain(Vec3::X * 10.0), 1.0);
        assert_eq!(opts.gain(Vec3::X * -10.0), 0.0);
        spatial.max_distance = spatial.min_distance;
        opts.spatial = Some(spatial);
        assert_eq!(opts.validate(), Err(PcmError::Options));
        opts.spatial = None;
        for v in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
            opts.volume = v;
            assert_eq!(opts.validate(), Err(PcmError::Options));
        }
        opts.volume = 1.0;
        for v in [f32::NAN, f32::INFINITY, -1.0] {
            spatial.min_distance = v;
            spatial.max_distance = 10.0;
            opts.spatial = Some(spatial);
            assert_eq!(opts.validate(), Err(PcmError::Options));
        }
        opts.spatial = Some(PcmSpatial { position: Vec3::NAN, min_distance: 1.0, max_distance: 10.0 });
        assert_eq!(opts.validate(), Err(PcmError::Options));
    }
}
