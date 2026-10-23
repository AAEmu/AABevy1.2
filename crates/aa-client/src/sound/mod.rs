//! Sound in the client: `aa-audio` wired to the screens, the Lua UI and the world.
//!
//! * [`ui`]: the `AAUI` sound hooks the UI toolkit calls (`X2Sound:PlayUISound` / `PlayMusic` / `StopMusic`,
//!   `SetSounds` sets: `window`, `button` and `global` pack items) and the sound option sliders, bound into every UI
//!   Lua state by [`bind`].
//! * [`music`]: the one music director. The login screens play what the original scripts play (`PlayMusic` in the stage
//!   scripts, the screen's default where the scripts are silent); in game the zone's `soundmood_*` pack decides.
//! * [`zone`]: the sound packs of the zone and sub zone under the player.
//! * the listener: the active 3D camera of a screen (the login stage camera, the in-game orbit camera).
//! * [`reverb`]: the preset of the `ReverbVolume` around the player goes to the engine's one global reverb.
//! * [`moods`]: the sound mood (death, combat, underwater, low health, falling, driving, indoor ...) the original's
//!   `soundmoods.xml` mixes by, the sounds the packs attach to the moods and the dive sounds.
//! * [`events`]: level up, quest jingles, target select, NPC greetings, item pickup / use sounds, coins.
//! * placed sounds (ambient volumes, random sounds, spots, reverb volumes) are `aa-world`'s (`objects::sounds`).
//!
//! Everything is silent in unattended runs (`--shot`, `--exit-after`, `--mute`, `AA_MUTE=1`; `--sound` overrides):
//! the engine opens no device then and the pak and the database are not read for sound. `AA_AUDIO_LOG=1` logs each
//! sound that is started (and, with it, the sound database is read in muted runs too, so the logs name the events).
//! `AA_VOLUME=0.1` starts the master volume at 10 %.

mod events;
mod moods;
mod music;
mod reverb;
mod tables;
mod ui;
mod zone;

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, OnceLock};

use aa_audio::{Engine, SoundPacks, SpatialListener, Volumes};
use bevy::prelude::*;

use crate::ingame::camera::OrbitCamera;
use crate::login::stage3d::StageCamera;

pub use ui::bind;

/// Whether to log every sound start (`AA_AUDIO_LOG=1`).
pub fn log_enabled() -> bool {
    aa_world::objects::sounds::audio_log()
}

/// The sound packs of the game database, read on a thread at startup (`None` until then, and in muted runs).
#[derive(Resource, Clone)]
pub struct Packs(Arc<OnceLock<SoundPacks>>);

impl Packs {
    pub fn get(&self) -> Option<&SoundPacks> {
        self.0.get()
    }
}

/// What the Lua hooks share with the Bevy systems (the hooks run inside the UI runtime, which has no `World`).
#[derive(Default)]
pub(crate) struct Shared {
    /// The scripts' latest music request: `Some(Some(item))` play, `Some(None)` stop, `None` nothing new.
    music: Mutex<Option<Option<String>>>,
    /// Widget sounds (`SetSounds`) are played: not while a screen's UI is being built.
    widget_sounds: AtomicBool,
    options: Mutex<Options>,
}

/// The sound sliders of the Options window.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct Options {
    pub master: f32,
    /// Music.
    pub bgm: f32,
    /// Effects: skills, ambience, voices and the UI.
    pub sfx: f32,
    pub cinema: f32,
    /// `OPTION_ITEM_USE_COMBAT_SOUND`: the combat music.
    pub combat_music: bool,
    pub changed: bool,
}

impl Default for Options {
    /// Full volume; `AA_VOLUME=0.1` starts the master slider there (quiet checks on a shared machine).
    fn default() -> Self {
        let master = std::env::var("AA_VOLUME").ok().and_then(|v| v.parse::<f32>().ok()).map(|v| v.clamp(0.0, 1.0));
        Options { master: master.unwrap_or(1.0), bgm: 1.0, sfx: 1.0, cinema: 1.0, combat_music: true, changed: master.is_some() }
    }
}

impl Options {
    pub fn volumes(&self) -> Volumes {
        Volumes { master: self.master, music: self.bgm, sfx: self.sfx, ambience: self.sfx, voice: self.sfx, ui: self.sfx }
    }
}

/// The handles the Lua hooks need, set once at startup: UI runtimes are built in many places.
pub(crate) struct Binding {
    pub engine: Engine,
    pub packs: Packs,
    pub shared: Arc<Shared>,
}

static BINDING: OnceLock<Binding> = OnceLock::new();

pub struct SoundPlugin;

impl Plugin for SoundPlugin {
    fn build(&self, app: &mut App) {
        let Some(engine) = app.world().get_resource::<Engine>().cloned() else {
            warn!("sound: no audio engine (add aa_audio::AaAudioPlugin first); the client stays silent");
            return;
        };
        let packs = Packs(Arc::new(OnceLock::new()));
        let shared = Arc::new(Shared::default());
        // the databases are read only when something can be heard or logged
        let wanted = engine.stats().audible || log_enabled();
        let zones = zone::ZoneLoad::start(wanted, packs.clone());
        let tables = tables::Tables::start(wanted);
        let _ = BINDING.set(Binding { engine: engine.clone(), packs: packs.clone(), shared: shared.clone() });
        let no_login = std::env::args().any(|a| a == "--enter-world" || a == "--script");
        app.insert_resource(packs)
            .insert_resource(zones)
            .insert_resource(tables)
            .init_resource::<zone::ZonePacks>()
            .insert_resource(ui::Gate(shared.clone()))
            .insert_resource(music::Director::new(shared, no_login))
            .add_systems(
                Update,
                (ui::sync_options, ui::sync_gate, assign_listener, zone::track_zone_packs, music::drive_music, reverb::drive_reverb, moods::drive_moods, events::game_sounds, log_stats).chain(),
            );
    }
}

/// `AA_AUDIO_LOG=1`: what the engine holds, every 10 s while something is playing (voices, the decoded sound cache, the
/// work the decoder did, mixer tracks, the reverb and the mood).
fn log_stats(engine: Res<Engine>, time: Res<Time>, mut next: Local<f64>) {
    let now = time.elapsed_secs_f64();
    if !log_enabled() || now < *next {
        return;
    }
    *next = now + 10.0;
    let s = engine.stats();
    if s.started == 0 {
        return;
    }
    info!(
        "audio: stats {} voices, {} started, cache {:.1} MiB in {} sounds ({} hits / {} misses), decoded {} sounds = {:.0} s of audio in {:.2} s, {} tracks, reverb {:?} {:.2}, mood {:?}",
        s.active_voices,
        s.started,
        s.cache_bytes as f64 / 1048576.0,
        s.cache_sounds,
        s.cache_hits,
        s.cache_misses,
        s.decoded_sounds,
        s.decoded_seconds,
        s.decode_wall_seconds,
        s.reverb_buses,
        s.reverb_preset,
        s.reverb_weight,
        s.mood
    );
}

/// The listener sits on the active 3D camera of the screen: the in-game orbit camera, else the login stage camera
/// (world select has no 3D camera; its music is 2D).
fn assign_listener(
    mut commands: Commands,
    orbit: Query<Entity, With<OrbitCamera>>,
    stage: Query<Entity, With<StageCamera>>,
    listeners: Query<Entity, With<SpatialListener>>,
) {
    let want = orbit.iter().next().or_else(|| stage.iter().next());
    for e in &listeners {
        if Some(e) != want {
            commands.entity(e).remove::<SpatialListener>();
        }
    }
    if let Some(e) = want
        && !listeners.contains(e)
    {
        commands.entity(e).insert(SpatialListener);
    }
}
