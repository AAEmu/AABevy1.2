//! ArcheAge 1.2 sound: FSB5 banks (FMOD Vorbis and MPEG), FEV events and playback in Bevy.
//!
//! * [`fsb5`], [`vorbis`], [`mpeg`], [`decode`]: the banks and their codecs, down to PCM.
//! * [`fev`]: the event projects (events, sound definitions, parameters, envelopes).
//! * [`library`]: projects and banks out of the pak, event paths, what an event plays.
//! * [`engine`]: the player thread (kira), volume groups, music streaming.
//! * [`reverb`]: the area reverb presets and the reverb effect. [`mood`]: the sound moods (mixer snapshots) and [`muffle`], the
//!   low-pass filter they use.
//! * [`packs`]: [`SoundPacks`], the pack item name to event path table of the game database.
//! * [`plugin`]: [`AaAudioPlugin`], [`Sound`], [`SpatialListener`].
//!
//! ```no_run
//! use aa_audio::{AaAudioPlugin, Sound, SpatialListener};
//! use bevy::prelude::*;
//!
//! fn setup(mut commands: Commands) {
//!     commands.spawn((Camera3d::default(), SpatialListener));
//!     commands.spawn((Transform::from_xyz(3.0, 0.0, -5.0), Sound::event("sounds/x2skills:wild:fire_arrow_form")));
//! }
//! # fn main() { App::new().add_plugins(AaAudioPlugin::default()).add_systems(Startup, setup); }
//! ```

pub mod cache;
pub mod decode;
pub mod engine;
pub mod fev;
pub mod fsb5;
pub mod library;
pub mod mood;
pub mod mpeg;
pub mod muffle;
pub mod packs;
pub mod pcm;
mod playback;
pub mod plugin;
pub mod reverb;
pub mod stream;
pub mod vorbis;

pub use decode::{Pcm, decode, decode_data};
pub use engine::{Category, Engine, PlayOptions, Settings, Stats, Voice, Volumes, audible_by_default};
pub use fev::Fev;
pub use fsb5::Fsb5;
pub use library::{EventRef, Library};
pub use packs::SoundPacks;
pub use pcm::{PcmClip, PcmError, PcmOptions, PcmPlaybackObserver, PcmSpatial};
pub use plugin::{AaAudioPlugin, Sound, SpatialListener};
