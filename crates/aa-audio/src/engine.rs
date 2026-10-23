//! The player: one thread owns the mixer ([kira]) and every voice; the rest of the program talks to it
//! through [`Engine`], whose playback calls only send a message. Constructing [`PcmClip`] converts
//! validated float PCM on the caller's worker before submission.
//!
//! * Events are resolved with the [`Library`] and their sounds are decoded on the worker threads
//!   (`aa_cry::workers`) into a [`SampleCache`] of at most [`Settings::cache_bytes`]. Nothing is decoded
//!   or read on the caller's thread.
//! * A voice is one trigger of an event. Its parameters (the listener `distance`, and whatever the game
//!   sets: `daylight`, `speed`, `gender` ...) pick which instances of which layers play and drive the
//!   layers' volume curves; a 3D voice also follows the event's rolloff. The mixer only pans.
//! * Music streams ([`crate::stream`]) and crossfades.
//! * Nothing opens an audio device until the first sound is played, and an engine that is not
//!   [audible](Settings::audible) never does (and never reads the pak): unattended runs stay silent.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bevy::math::{Quat, Vec3};
use bevy::prelude::Resource;
use kira::listener::ListenerHandle;
use kira::effect::filter::{FilterBuilder, FilterHandle, FilterMode};
use kira::sound::PlaybackState;
use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle, StaticSoundSettings};
use kira::sound::streaming::{StreamingSoundData, StreamingSoundHandle};
use kira::sound::{EndPosition, PlaybackPosition, Region};
use kira::track::{SendTrackBuilder, SendTrackHandle, SpatialTrackBuilder, SpatialTrackHandle, TrackBuilder, TrackHandle};
use kira::{AudioManager, AudioManagerSettings, Capacities, Decibels, DefaultBackend, Tween};

use crate::cache::{Key, Sample, SampleCache};
use crate::fev::Body;
use crate::fsb5::{Codec, Error};
use crate::mood::{Mix, MoodTable};
use crate::pcm::{MAX_PCM_VOICES, PcmClip, PcmError, PcmOptions, PcmPlaybackObserver, valid_volume};
use crate::playback::{self, Candidate};
use crate::muffle::{MuffleBuilder, MuffleHandle};
use crate::reverb::{Params as ReverbParams, Presets, ReverbBuilder, ReverbHandle};
use crate::library::{EventRef, Highpass, Library, PickState, Planned, Project, Rng, active_instances, distance_gain, distance_parameters, doppler_factor, initial_values, layer_cutoff, layer_filtered, layer_gain, layer_highpass, layer_pan, layer_pitch, plan, set_value, start_part};
use crate::stream::StreamDecoder;
use crate::vorbis;

/// The volume group a sound belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    Music,
    /// Player scores (native sound semantic 8), independent of BGM and SFX.
    UserMusic,
    Sfx,
    Ambience,
    Voice,
    Ui,
}

impl Category {
    /// The group for an FMOD mixer category path (`master/monster_voice`).
    pub fn of_mixer(category: &str) -> Category {
        let leaf = category.rsplit('/').next().unwrap_or(category);
        if leaf.starts_with("music") {
            Category::Music
        } else if leaf.starts_with("ambience") || leaf == "underwater_only" {
            Category::Ambience
        } else if leaf.ends_with("_voice") || leaf == "cutscene" {
            Category::Voice
        } else if leaf == "ui_button" {
            Category::Ui
        } else {
            Category::Sfx
        }
    }
}

/// Volume of each group, 0 (silent) to 1 (as authored), times `master`.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct Volumes {
    pub master: f32,
    pub music: f32,
    /// Native `s_MIDIVolume` / option 40. Master still applies; BGM and SFX do not.
    pub user_music: f32,
    pub sfx: f32,
    pub ambience: f32,
    pub voice: f32,
    pub ui: f32,
}

impl Default for Volumes {
    fn default() -> Volumes {
        Volumes { master: 1.0, music: 1.0, user_music: 1.0, sfx: 1.0, ambience: 1.0, voice: 1.0, ui: 1.0 }
    }
}

impl Volumes {
    pub fn get(&self, c: Category) -> f32 {
        match c {
            Category::Music => self.music,
            Category::UserMusic => self.user_music,
            Category::Sfx => self.sfx,
            Category::Ambience => self.ambience,
            Category::Voice => self.voice,
            Category::Ui => self.ui,
        }
    }
}

/// A playing (or waiting to play) sound.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Voice(pub u64);

/// Per trigger options.
#[derive(Clone, Debug)]
pub struct PlayOptions {
    /// Multiplies the event's volume.
    pub volume: f32,
    /// Overrides the group chosen from the event's mixer category.
    pub category: Option<Category>,
    /// Event parameters to start with, by name (`("gender", 1.0)`). Unknown names are ignored.
    pub params: Vec<(String, f32)>,
}

impl Default for PlayOptions {
    fn default() -> Self {
        PlayOptions { volume: 1.0, category: None, params: Vec::new() }
    }
}

/// How the engine starts.
#[derive(Clone, Debug)]
pub struct Settings {
    /// `false` keeps the engine silent: it opens no device, reads no sound data and decodes nothing.
    pub audible: bool,
    /// Bound of the decoded sound cache.
    pub cache_bytes: usize,
    /// Most voices playing at once; the least important is stopped for a new one.
    pub max_voices: usize,
    /// Language folder of the voice projects.
    pub language: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { audible: true, cache_bytes: 128 << 20, max_voices: 64, language: None }
    }
}

impl Settings {
    /// Audible unless the run is unattended; see [`audible_by_default`].
    pub fn auto() -> Settings {
        Settings { audible: audible_by_default(), ..Settings::default() }
    }
}

/// Whether a program should make sound unless told otherwise: not when `--shot` or `--exit-after`
/// is on the command line (unattended runs, see `aa-dev`), nor with `--mute` or `AA_MUTE=1`;
/// `--sound` always allows it.
pub fn audible_by_default() -> bool {
    let args: Vec<String> = std::env::args().collect();
    let has = |f: &str| args.iter().any(|a| a == f);
    if has("--sound") {
        return true;
    }
    !(has("--shot") || has("--exit-after") || has("--mute") || std::env::var("AA_MUTE").is_ok_and(|v| v == "1"))
}

/// Counters for logs and tests.
#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub audible: bool,
    pub device_error: Option<String>,
    /// `play` calls received.
    pub requested: u64,
    /// Voices that began to sound.
    pub started: u64,
    pub unknown_events: u64,
    /// Not started because the 3D event was out of range.
    pub culled: u64,
    /// Not started because of the event's max playbacks or the voice limit.
    pub refused: u64,
    /// Voices stopped for a newer one.
    pub stolen: u64,
    pub active_voices: usize,
    pub waiting_for_decode: usize,
    pub cache_bytes: usize,
    pub cache_sounds: usize,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub decoded_sounds: u64,
    pub decoded_seconds: f64,
    pub decode_wall_seconds: f64,
    pub decode_failures: u64,
    /// The reverb preset in effect (`None`: off) and its weight, as last set with [`Engine::set_reverb`] and found.
    pub reverb_preset: Option<String>,
    pub reverb_weight: f32,
    /// Mixer tracks in use (groups, reverb buses and the spatial tracks of playing sounds).
    pub reverb_buses: usize,
    /// The mood in force (`None`: no moods asked for).
    pub mood: Option<String>,
    /// The music track that is playing (or fading in), by the path it was asked for.
    pub music: Option<String>,
    /// How many tracks ran to their end by themselves (a looping track never does; a stop or a crossfade is not an end).
    pub music_ended: u64,
    /// Sounds playing from a stream (long loops and music) rather than from the cache.
    pub streaming: usize,
    /// Accepted PCM submissions still queued, playing or fading (at most eight).
    pub pcm_submissions: usize,
}

struct Shared {
    /// Voices are only tracked in `live` when the engine can be heard.
    audible: bool,
    next: AtomicU64,
    /// Commands sent by [`Engine`] calls and commands the player has handled and published.
    sent: AtomicU64,
    handled: AtomicU64,
    stats: Mutex<Stats>,
    finished: Mutex<Vec<Voice>>,
    /// Voices asked for and not ended yet ([`Engine::is_playing`]).
    live: Mutex<HashSet<Voice>>,
    pcm_slots: Arc<AtomicUsize>,
    /// Only tracked audible requests not yet claimed by the player (at most eight).
    pcm_pending: Arc<Mutex<HashMap<Voice, PcmPlaybackObserver>>>,
}

impl Shared {
    fn new(audible: bool) -> Self {
        Self {
            audible, live: Mutex::default(), next: AtomicU64::new(1),
            sent: AtomicU64::new(0), handled: AtomicU64::new(0),
            stats: Mutex::new(Stats { audible, ..Stats::default() }),
            finished: Mutex::new(Vec::new()), pcm_slots: Arc::new(AtomicUsize::new(0)),
            pcm_pending: Arc::default(),
        }
    }
}

/// Follows a submission from the channel through playback; every failure/drop releases it.
struct PcmPermit {
    slots: Arc<AtomicUsize>,
    tracking: Option<PcmTracking>,
}

impl Drop for PcmPermit {
    fn drop(&mut self) {
        // Remove pending registration before another submitter can reuse this slot.
        drop(self.tracking.take());
        self.slots.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Moves with the command/permit until terminal drop; the observer itself owns no engine data.
struct PcmTracking {
    id: Voice,
    observer: PcmPlaybackObserver,
    pending: Arc<Mutex<HashMap<Voice, PcmPlaybackObserver>>>,
}

impl PcmTracking {
    /// Removing under the same lock as stop establishes the pending cancellation boundary.
    fn claim(&self) -> bool {
        self.pending.lock().unwrap_or_else(|e| e.into_inner()).remove(&self.id);
        !self.observer.cancelled()
    }
}

impl Drop for PcmTracking {
    fn drop(&mut self) {
        self.pending.lock().unwrap_or_else(|e| e.into_inner()).remove(&self.id);
        self.observer.finish();
    }
}

enum Cmd {
    Play { id: Voice, path: String, at: Option<Vec3>, opts: PlayOptions },
    Pcm { id: Voice, clip: PcmClip, opts: PcmOptions, permit: PcmPermit },
    MutedPcm { id: Voice, tracking: Option<PcmTracking> },
    Move { id: Voice, at: Vec3 },
    Param { id: Voice, name: String, value: f32 },
    VoiceVolume { id: Voice, volume: f32 },
    Stop { id: Voice, fade: bool },
    StopAll,
    Listener { position: Vec3, rotation: Quat },
    Volumes(Volumes),
    Reverb { preset: Option<String>, weight: f32 },
    Moods(Vec<String>),
    Music { path: String, fade_in: Option<Duration>, fade_out: Option<Duration> },
    StopMusic { fade: Duration },
    Decoded { key: Key, result: Result<Arc<Sample>, String>, seconds: f64, took: Duration },
    MusicReady { token: u64, result: Result<Prepared, String> },
    /// A long loop opened for streaming (`token` of the request that asked), or the reason it cannot be streamed.
    PartStream { voice: Voice, token: u64, key: Key, result: Result<OpenedStream, String> },
    Quit,
}

/// A music track ready to stream.
struct Prepared {
    stream: vorbis::Stream,
    /// `Some(loop points from the bank)` when the track repeats.
    looping: Option<Option<(u32, u32)>>,
    gain: f32,
    rate: f32,
    /// Reverb wet level of the event, dB.
    wet_db: f32,
    leaf: String,
    /// The fades the event was authored with.
    fade_in: Duration,
    fade_out: Duration,
}

/// A looping sound opened for streaming: the Vorbis stream and the loop points of the bank.
struct OpenedStream {
    stream: vorbis::Stream,
    loop_points: Option<(u32, u32)>,
}

/// Handle to the player thread. Cloning shares the thread; the thread ends when the last handle drops.
#[derive(Clone, Resource)]
pub struct Engine {
    tx: Sender<Cmd>,
    shared: Arc<Shared>,
    _guard: Arc<Guard>,
}

struct Guard(Sender<Cmd>);

impl Drop for Guard {
    fn drop(&mut self) {
        let _ = self.0.send(Cmd::Quit);
    }
}

impl Engine {
    pub fn start(settings: Settings) -> Engine {
        let (tx, rx) = channel();
        let shared = Arc::new(Shared::new(settings.audible));
        let thread_shared = shared.clone();
        let thread_tx = tx.clone();
        std::thread::Builder::new()
            .name("aa-audio".into())
            .spawn(move || {
                let mut player = Player::new(settings, thread_shared, thread_tx);
                player.run(rx);
            })
            .expect("cannot start the audio thread");
        Engine { _guard: Arc::new(Guard(tx.clone())), tx, shared }
    }

    fn send(&self, cmd: Cmd) {
        self.shared.sent.fetch_add(1, Ordering::SeqCst);
        let _ = self.tx.send(cmd);
    }

    fn voice(&self) -> Voice {
        let voice = Voice(self.shared.next.fetch_add(1, Ordering::Relaxed));
        if self.shared.audible {
            self.shared.live.lock().unwrap_or_else(|e| e.into_inner()).insert(voice);
        }
        voice
    }

    /// Whether the voice has been asked for and has not ended: it may still be waiting for its sound to decode, and a
    /// fading out voice still counts. `false` for an unknown event, a refused voice, a one-shot that played out, a stopped
    /// voice and always for a muted engine.
    pub fn is_playing(&self, voice: Voice) -> bool {
        self.shared.live.lock().unwrap_or_else(|e| e.into_inner()).contains(&voice)
    }

    /// Plays an event at a place in the world (a 3D event) or, for 2D events, anywhere.
    pub fn play_at(&self, path: &str, at: Vec3) -> Voice {
        self.play_with(path, Some(at), PlayOptions::default())
    }

    /// Plays an event without a place: 2D events as they are, 3D events at full volume in the middle.
    pub fn play_2d(&self, path: &str) -> Voice {
        self.play_with(path, None, PlayOptions::default())
    }

    pub fn play_with(&self, path: &str, at: Option<Vec3>, opts: PlayOptions) -> Voice {
        let id = self.voice();
        self.send(Cmd::Play { id, path: path.to_string(), at, opts });
        id
    }

    /// Submits shared PCM to this engine's existing mixer and volume groups. Construct the clip
    /// on a worker first; this call never copies/converts its frames. Options are checked before
    /// reserving one of eight queued/playing/fading PCM slots. `Err` creates no voice.
    ///
    /// Every `Ok` voice completes exactly once, including mute, refusal and device failure.
    /// A muted voice is immediately non-playing and retains no clip or slot. Other voices remain
    /// playing through mixer completion (including the 20 ms stop ramp). Use `is_playing` to track
    /// your own voice: `take_finished` is a shared drain also consumed by the Bevy plugin.
    pub fn play_pcm(&self, clip: &PcmClip, opts: PcmOptions) -> Result<Voice, PcmError> {
        self.submit_pcm(clip, opts, None)
    }

    /// Like [`Engine::play_pcm`], with sticky per-request admission and terminal status installed
    /// before submission. Kira handle acceptance is the start boundary, not first callback,
    /// native FMOD sound creation or audibility. Poll `is_finished()` before `was_started()`.
    ///
    /// A tracked request stopped while still pending is cancelled without mixer admission. The
    /// player claims it before admission/device setup; a concurrent stop after that claim can
    /// still follow an accepted handle, even before the first callback. Muted/refused/failed
    /// requests finish without starting. Observers survive `take_finished` and engine teardown,
    /// retain no clip/engine, and do not distinguish completion reasons or send network events.
    pub fn play_pcm_tracked(&self, clip: &PcmClip, opts: PcmOptions) -> Result<(Voice, PcmPlaybackObserver), PcmError> {
        let observer = PcmPlaybackObserver::new();
        let id = self.submit_pcm(clip, opts, Some(observer.clone()))?;
        Ok((id, observer))
    }

    fn submit_pcm(&self, clip: &PcmClip, opts: PcmOptions, observer: Option<PcmPlaybackObserver>) -> Result<Voice, PcmError> {
        opts.validate()?;
        let permit = if self.shared.audible {
            self.shared.pcm_slots.try_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                (n < MAX_PCM_VOICES).then_some(n + 1)
            }).map_err(|_| PcmError::Busy)?;
            Some(PcmPermit { slots: self.shared.pcm_slots.clone(), tracking: None })
        } else {
            None
        };
        let id = self.voice();
        let tracking = observer.map(|observer| {
            if permit.is_some() {
                self.shared.pcm_pending.lock().unwrap_or_else(|e| e.into_inner()).insert(id, observer.clone());
            }
            PcmTracking { id, observer, pending: self.shared.pcm_pending.clone() }
        });
        let cmd = match permit {
            Some(mut permit) => {
                permit.tracking = tracking;
                Cmd::Pcm { id, clip: clip.clone(), opts, permit }
            }
            None => Cmd::MutedPcm { id, tracking },
        };
        self.shared.sent.fetch_add(1, Ordering::SeqCst);
        if self.tx.send(cmd).is_err() {
            self.shared.live.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
            return Err(PcmError::EngineStopped);
        }
        Ok(id)
    }

    /// Moves a playing 3D voice.
    pub fn set_position(&self, voice: Voice, at: Vec3) {
        self.send(Cmd::Move { id: voice, at });
    }

    /// Sets an event parameter of a playing voice by name (`daylight`, `speed` ...): it changes
    /// which instances play and the layers' volume curves.
    pub fn set_param(&self, voice: Voice, name: &str, value: f32) {
        self.send(Cmd::Param { id: voice, name: name.to_string(), value });
    }

    /// Changes the volume multiplier of a playing voice.
    pub fn set_voice_volume(&self, voice: Voice, volume: f32) {
        self.send(Cmd::VoiceVolume { id: voice, volume });
    }

    /// Stops a voice, letting it fade out when `fade` (the event's fade out time).
    /// PCM uses a 20 ms anti-click ramp for either value (no authored event fade).
    pub fn stop(&self, voice: Voice, fade: bool) {
        if let Some(observer) = self.shared.pcm_pending.lock().unwrap_or_else(|e| e.into_inner()).get(&voice) {
            observer.cancel();
        }
        self.send(Cmd::Stop { id: voice, fade });
    }

    pub fn stop_all(&self) {
        for observer in self.shared.pcm_pending.lock().unwrap_or_else(|e| e.into_inner()).values() {
            observer.cancel();
        }
        self.send(Cmd::StopAll);
    }

    /// Where the listener is and which way it faces (a Bevy camera transform: -Z forward).
    pub fn set_listener(&self, position: Vec3, rotation: Quat) {
        self.send(Cmd::Listener { position, rotation });
    }

    pub fn set_volumes(&self, volumes: Volumes) {
        self.send(Cmd::Volumes(volumes));
    }

    /// Sets the area reverb: a preset of `libs/reverbpresets/reverbpresets.xml` (`x2_cave_1`, `x2_outdoor_1` ...) at a
    /// weight from 0 (none) to 1 (fully in the volume), or `None` for no reverb. Every sound sends to it by its event's
    /// reverb wet level; the effect glides to the new values in about a second.
    pub fn set_reverb(&self, preset: Option<&str>, weight: f32) {
        self.send(Cmd::Reverb { preset: preset.map(str::to_string), weight });
    }

    /// Sets which sound moods are active (`combat`, `underwater`, `death`, `indoor`, `low_health` ...; see [`crate::mood`]).
    /// The one with the highest priority is the mix, `default` when none is named. Sounds fade to the mix in about a second.
    pub fn set_moods(&self, moods: &[&str]) {
        self.send(Cmd::Moods(moods.iter().map(|m| m.to_string()).collect()));
    }

    /// Starts a music event, crossfading from the current track. The same track again is ignored.
    pub fn play_music(&self, path: &str, fade_in: Duration, fade_out: Duration) {
        self.send(Cmd::Music { path: path.to_string(), fade_in: Some(fade_in), fade_out: Some(fade_out) });
    }

    /// Like [`Engine::play_music`] with the fades the event was authored with: the new track fades in as the event says, and
    /// the old one out as its own event says (the zone music of the game: 6 s in, 4 s out).
    pub fn play_music_event(&self, path: &str) {
        self.send(Cmd::Music { path: path.to_string(), fade_in: None, fade_out: None });
    }

    pub fn stop_music(&self, fade: Duration) {
        self.send(Cmd::StopMusic { fade });
    }

    /// The music track that is playing: the path it was asked for. `None` between tracks. See also [`Stats::music_ended`].
    pub fn music_playing(&self) -> Option<String> {
        self.shared.stats.lock().unwrap_or_else(|e| e.into_inner()).music.clone()
    }

    pub fn stats(&self) -> Stats {
        self.shared.stats.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Voices that ended since the last call.
    pub fn take_finished(&self) -> Vec<Voice> {
        std::mem::take(&mut *self.shared.finished.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// Waits until nothing plays and nothing is being decoded, or `timeout` passes. For tools and tests.
    pub fn wait_idle(&self, timeout: Duration) -> bool {
        let end = Instant::now() + timeout;
        while Instant::now() < end {
            // everything sent so far has been handled (the first sound waits for the pak to open)
            if self.shared.handled.load(Ordering::SeqCst) >= self.shared.sent.load(Ordering::SeqCst) {
                let s = self.stats();
                if s.active_voices == 0 && s.waiting_for_decode == 0 && s.music.is_none() {
                    return true;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }
}

fn db(gain: f32) -> Decibels {
    if gain <= 0.001 { Decibels::SILENCE } else { Decibels(20.0 * gain.log10()) }
}

fn tween_ms(ms: u32) -> Tween {
    Tween { duration: Duration::from_millis(ms.max(1) as u64), ..Tween::default() }
}

/// Event reverb wet levels at or below this (dB) send nothing: the project files use -60, FMOD's "off".
const WET_OFF: f32 = -45.0;

/// What a sound sends to the reverb: the event's wet level times its group's volume (the send leaves the track after the
/// group, so the group's fader has to be applied by hand).
fn send_db(wet_db: f32, group_volume: f32) -> Decibels {
    Decibels(wet_db + db(group_volume).0)
}

struct Audio<B: kira::backend::Backend = DefaultBackend> {
    manager: AudioManager<B>,
    listener: ListenerHandle,
    music: TrackHandle,
    user_music: TrackHandle,
    sfx: TrackHandle,
    ambience: TrackHandle,
    voice: TrackHandle,
    ui: TrackHandle,
    /// The reverb: a send track whose only effect is the area reverb, and its control.
    reverb_send: SendTrackHandle,
    reverb: ReverbHandle,
    /// Sub tracks of a group that send to the reverb at a wet level (whole dB), for sounds on no spatial track of their own.
    buses: HashMap<(Category, i16), TrackHandle>,
    /// The broad mood low-pass filters (UI and player scores have none).
    muffles: HashMap<Category, MuffleHandle>,
}

impl<B: kira::backend::Backend> Audio<B> {
    fn track(&mut self, c: Category) -> &mut TrackHandle {
        match c {
            Category::Music => &mut self.music,
            Category::UserMusic => &mut self.user_music,
            Category::Sfx => &mut self.sfx,
            Category::Ambience => &mut self.ambience,
            Category::Voice => &mut self.voice,
            Category::Ui => &mut self.ui,
        }
    }

    /// The track a flat (2D) sound of this group and reverb wet level (dB) plays on: the group's own track when it sends
    /// nothing, else a sub track that sends to the reverb.
    fn bus(&mut self, c: Category, wet_db: f32, volumes: &Volumes) -> &mut TrackHandle {
        if wet_db <= WET_OFF {
            return self.track(c);
        }
        let key = (c, wet_db.round() as i16);
        if !self.buses.contains_key(&key) {
            let builder = TrackBuilder::new().with_send(&self.reverb_send, send_db(f32::from(key.1), volumes.get(c)));
            match self.track(c).add_sub_track(builder) {
                Ok(t) => {
                    self.buses.insert(key, t);
                }
                Err(e) => {
                    tracing::debug!("aa-audio: no track for a reverb bus: {e}");
                    return self.track(c);
                }
            }
        }
        self.buses.get_mut(&key).expect("just inserted")
    }

    fn apply(&mut self, v: &Volumes) {
        self.manager.main_track().set_volume(db(v.master), Tween::default());
        for c in [Category::Music, Category::UserMusic, Category::Sfx, Category::Ambience, Category::Voice, Category::Ui] {
            self.track(c).set_volume(db(v.get(c)), Tween::default());
        }
        for ((c, wet), bus) in self.buses.iter_mut() {
            let _ = bus.set_send(&self.reverb_send, send_db(f32::from(*wet), v.get(*c)), Tween::default());
        }
    }
}

/// A sound the mixer plays: decoded whole or streamed.
enum Handle {
    Static(StaticSoundHandle),
    Stream(StreamingSoundHandle<Error>),
}

impl Handle {
    fn state(&self) -> PlaybackState {
        match self {
            Handle::Static(h) => h.state(),
            Handle::Stream(h) => h.state(),
        }
    }

    fn set_volume(&mut self, volume: Decibels, tween: Tween) {
        match self {
            Handle::Static(h) => h.set_volume(volume, tween),
            Handle::Stream(h) => h.set_volume(volume, tween),
        }
    }

    fn set_rate(&mut self, rate: f32, tween: Tween) {
        match self {
            Handle::Static(h) => h.set_playback_rate(f64::from(rate), tween),
            Handle::Stream(h) => h.set_playback_rate(f64::from(rate), tween),
        }
    }

    fn stop(&mut self, tween: Tween) {
        match self {
            Handle::Static(h) => h.stop(tween),
            Handle::Stream(h) => h.stop(tween),
        }
    }
}

/// Long looping sounds (ambience beds) play from a stream: decoded whole they would take about 10 MiB a minute from the cache.
const STREAM_MS: u32 = 10_000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PartState {
    /// Not active at the current parameter values.
    Idle,
    /// Active, waiting for its sound to be decoded (or for the other parts of the first start).
    Loading,
    Playing,
    /// Played out (or failed) while still active: it does not restart.
    Done,
}

/// One instance of one layer of the event.
struct Part {
    layer: usize,
    inst: usize,
    state: PartState,
    planned: Option<Planned>,
    key: Option<Key>,
    sample: Option<Arc<Sample>>,
    /// A long loop plays from a stream: the request that was made for it, and the stream once it is open.
    stream_token: Option<u64>,
    stream: Option<OpenedStream>,
    handle: Option<Handle>,
    track: Option<SpatialTrackHandle>,
    /// The part's own track when it is not spatial but filtered.
    flat_track: Option<TrackHandle>,
    /// The low-pass of the layer's curves (occlusion by distance, the DSP filters), and the cutoff last given to it.
    filter: Option<MuffleHandle>,
    cutoff: Option<f32>,
    highpass: Option<FilterHandle>,
    highpass_state: Option<Highpass>,
    last_db: f32,
    /// The playback rate last given to the sound (pitch, pitch curves and doppler).
    rate: f32,
    /// Spatial strength last sent to the part's track (event times layer pan curve).
    pan: f32,
    /// Reverb wet level of the part's event, dB.
    wet_db: f32,
    /// Started by the spawning of its sound definition, not by the instance: it is a copy that plays once.
    spawned: bool,
    /// The next sound to spawn, for the part that spawns.
    next_spawn: Option<Instant>,
    /// When a looping playlist, or a silent pick, starts the next sound.
    restart_at: Option<Instant>,
}

impl Part {
    fn new(layer: usize, inst: usize, wet_db: f32) -> Part {
        Part {
            layer,
            inst,
            state: PartState::Idle,
            planned: None,
            key: None,
            sample: None,
            stream_token: None,
            stream: None,
            handle: None,
            track: None,
            flat_track: None,
            filter: None,
            cutoff: None,
            highpass: None,
            highpass_state: None,
            last_db: 0.0,
            rate: 1.0,
            pan: 1.0,
            wet_db,
            spawned: false,
            next_spawn: None,
            restart_at: None,
        }
    }

    /// The sound is there to start: decoded, or opened as a stream.
    fn ready(&self) -> bool {
        self.sample.is_some() || self.stream.is_some()
    }
}

struct ActiveVoice {
    id: Voice,
    /// Stable per-event FMOD pool order, inherited when this slot is stolen.
    playback_slot: usize,
    event: EventRef,
    category: Category,
    /// The last part of the event's mixer category (`footsteps`): what the mood mixes it by.
    leaf: String,
    /// The mood's gain for the voice, gliding.
    mood: f32,
    /// `Some` for a 3D event at a place.
    at: Option<Vec3>,
    moved: bool,
    /// How fast the place moves (m/s, smoothed) and when it was last moved: the doppler effect.
    velocity: Vec3,
    moved_at: Instant,
    volume: f32,
    values: Vec<f32>,
    distance_params: Vec<usize>,
    /// A parameter changed since the active instances were last worked out.
    dirty: bool,
    /// Lives until stopped even when nothing plays (an ambience waiting for the daylight).
    persistent: bool,
    parts: Vec<Part>,
    requested: Instant,
    last_tick: Instant,
    started: bool,
    stopping: bool,
}

impl ActiveVoice {
    fn live(&self) -> bool {
        self.parts.iter().any(|p| matches!(p.state, PartState::Loading | PartState::Playing))
    }

    /// More sounds are to come though nothing plays right now: a sound definition of an active instance spawns new ones (gusts,
    /// animal calls) or a looping playlist moves on to its next wave. (A one-shot event plays its pass and ends.)
    fn waiting(&self) -> bool {
        !self.stopping && !self.event.event().one_shot() && self.parts.iter().any(|p| !p.spawned && p.state != PartState::Idle && (p.next_spawn.is_some() || p.restart_at.is_some()))
    }

    /// Loudness the voice should have now for the part: event and sound definition, voice volume,
    /// the layer's curves and, for a 3D voice, the rolloff.
    fn target_gain(&self, part: &Part, listener: Vec3) -> f32 {
        let Some(p) = &part.planned else { return 0.0 };
        let ev = self.event.event();
        let mut g = p.gain * self.volume * self.mood * layer_gain(ev, &self.values, p.layer);
        if let Some(at) = self.at {
            g *= distance_gain(ev, at.distance(listener));
        }
        g
    }
}

struct CurrentMusic {
    path: String,
    handle: StreamingSoundHandle<Error>,
    /// The event's own gain and mixer category leaf; the mood's gain is applied on top.
    gain: f32,
    leaf: String,
    last_db: f32,
    /// How the event fades out when another track replaces it.
    fade_out: Duration,
}

/// PCM has no authored layers/parameters. It shares the engine's IDs, admission, controls,
/// completion and category tracks, retaining only the mixer handle and its submission permit.
struct PcmVoice {
    opts: PcmOptions,
    handle: StaticSoundHandle,
    track: Option<SpatialTrackHandle>,
    requested: Instant,
    stopping: bool,
    last_db: f32,
    _permit: PcmPermit,
}

impl PcmVoice {
    fn begin<B: kira::backend::Backend>(
        audio: &mut Audio<B>, clip: &PcmClip, opts: PcmOptions, listener: Vec3, permit: PcmPermit,
    ) -> Result<Self, String> {
        let (handle, track) = begin_pcm(audio, clip, opts, listener)?;
        if let Some(tracking) = &permit.tracking {
            tracking.observer.start();
        }
        Ok(Self {
            opts, handle, track, requested: Instant::now(), stopping: false,
            last_db: db(opts.gain(listener)).0, _permit: permit,
        })
    }
}

struct Player {
    settings: Settings,
    shared: Arc<Shared>,
    tx: Sender<Cmd>,
    audio: Option<Audio>,
    device_failed: bool,
    listener: (Vec3, Quat),
    volumes: Volumes,
    cache: SampleCache,
    pending: HashMap<Key, Vec<Voice>>,
    voices: HashMap<Voice, ActiveVoice>,
    /// Last idle slot used by each event; native fixed pools start this cursor at zero.
    playback_cursor: HashMap<(usize, usize), usize>,
    pcm_voices: HashMap<Voice, PcmVoice>,
    pick: PickState,
    rng: Rng,
    music: Option<CurrentMusic>,
    music_token: u64,
    music_request: Option<(String, Option<Duration>, Option<Duration>)>,
    /// Streams asked for (their tokens), and the sounds that turned out not to be streamable (decoded instead).
    stream_token: u64,
    not_streamable: HashSet<Key>,
    /// How fast the listener moves (m/s, smoothed) and when it was last moved.
    listener_velocity: Vec3,
    listener_moved: Instant,
    unknown: HashSet<String>,
    stats: Stats,
    /// The area reverb that was asked for: preset name and weight; the table of presets is read when it is first needed.
    reverb_want: (Option<String>, f32),
    presets: Option<Presets>,
    /// The moods that are active, the table they come from (read on first use) and the mix they make.
    moods: Vec<String>,
    mood_table: Option<MoodTable>,
    mix: Mix,
    /// The gain the mix has reached for each category leaf (it glides to the mix's).
    mood_cur: HashMap<String, f32>,
    last_update: Instant,
}

impl Drop for Player {
    fn drop(&mut self) {
        // Includes requests still in the channel if the player exits before starting them.
        let ids: Vec<Voice> = self.shared.live.lock().unwrap_or_else(|e| e.into_inner()).drain().collect();
        self.shared.finished.lock().unwrap_or_else(|e| e.into_inner()).extend(ids);
    }
}

impl Player {
    fn new(settings: Settings, shared: Arc<Shared>, tx: Sender<Cmd>) -> Player {
        // The pak is only opened for an engine that can be heard.
        if let (true, Some(l)) = (settings.audible, &settings.language) {
            Library::global().set_language(l);
        }
        Player {
            cache: SampleCache::new(settings.cache_bytes),
            stats: Stats { audible: settings.audible, ..Stats::default() },
            settings,
            shared,
            tx,
            audio: None,
            device_failed: false,
            listener: (Vec3::ZERO, Quat::IDENTITY),
            volumes: Volumes::default(),
            pending: HashMap::new(),
            voices: HashMap::new(),
            playback_cursor: HashMap::new(),
            pcm_voices: HashMap::new(),
            pick: PickState::default(),
            rng: Rng::from_time(),
            music: None,
            music_token: 0,
            music_request: None,
            stream_token: 0,
            not_streamable: HashSet::new(),
            listener_velocity: Vec3::ZERO,
            listener_moved: Instant::now(),
            unknown: HashSet::new(),
            reverb_want: (None, 0.0),
            presets: None,
            moods: Vec::new(),
            mood_table: None,
            mix: Mix::default(),
            mood_cur: HashMap::new(),
            last_update: Instant::now(),
        }
    }

    fn run(&mut self, rx: Receiver<Cmd>) {
        let mut last_update = Instant::now();
        let mut handled = 0u64;
        loop {
            let busy = !self.voices.is_empty() || !self.pcm_voices.is_empty() || self.music.is_some() || self.music_request.is_some();
            let wait = if busy { Duration::from_millis(16) } else { Duration::from_millis(500) };
            let mut next = match rx.recv_timeout(wait) {
                Ok(Cmd::Quit) | Err(RecvTimeoutError::Disconnected) => break,
                Ok(cmd) => Some(cmd),
                Err(RecvTimeoutError::Timeout) => None,
            };
            while let Some(cmd) = next.take().or_else(|| rx.try_recv().ok()) {
                if matches!(cmd, Cmd::Quit) {
                    return;
                }
                // results from the workers are not counted: only what [`Engine`] sent
                if !matches!(cmd, Cmd::Decoded { .. } | Cmd::MusicReady { .. } | Cmd::PartStream { .. }) {
                    handled += 1;
                }
                self.handle(cmd);
            }
            if last_update.elapsed() >= Duration::from_millis(16) {
                last_update = Instant::now();
                self.update();
            }
            self.publish();
            self.shared.handled.store(handled, Ordering::SeqCst);
        }
    }

    fn publish(&mut self) {
        let mut s = self.stats.clone();
        s.active_voices = self.voices.len() + self.pcm_voices.len();
        s.pcm_submissions = self.shared.pcm_slots.load(Ordering::SeqCst);
        s.waiting_for_decode = self.pending.len() + usize::from(self.music_request.is_some());
        s.cache_bytes = self.cache.bytes();
        s.cache_sounds = self.cache.len();
        s.cache_hits = self.cache.hits;
        s.cache_misses = self.cache.misses;
        s.streaming = usize::from(self.music.is_some())
            + self.voices.values().flat_map(|v| v.parts.iter()).filter(|p| matches!(p.handle, Some(Handle::Stream(_)))).count();
        *self.shared.stats.lock().unwrap_or_else(|e| e.into_inner()) = s;
    }

    fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Play { id, path, at, opts } => {
                self.play(id, &path, at, opts);
                // A failed request will never reach tick(), but Bevy still needs its completion
                // to remove/despawn the Sound. Muted engines retain their no-playback behavior.
                if self.settings.audible && !self.voices.contains_key(&id) {
                    self.finish(id);
                }
            }
            Cmd::Pcm { id, clip, opts, permit } => {
                self.play_pcm(id, clip, opts, permit);
                if !self.pcm_voices.contains_key(&id) {
                    self.finish(id);
                }
            }
            Cmd::MutedPcm { id, tracking } => {
                self.stats.requested += 1;
                drop(tracking);
                self.finish(id);
            }
            Cmd::Move { id, at } => {
                if at.is_finite() && let Some(v) = self.pcm_voices.get_mut(&id)
                    && let Some(s) = &mut v.opts.spatial
                {
                    s.position = at;
                    if let Some(t) = &mut v.track {
                        t.set_position(at, Tween::default());
                    }
                }
                if let Some(v) = self.voices.get_mut(&id)
                    && let Some(old) = v.at
                {
                    let dt = v.moved_at.elapsed().as_secs_f32();
                    if dt > 0.001 {
                        v.velocity = smooth_velocity(v.velocity, old, at, dt);
                        v.moved_at = Instant::now();
                    }
                    v.at = Some(at);
                    v.moved = true;
                }
            }
            Cmd::Param { id, name, value } => {
                if let Some(v) = self.voices.get_mut(&id)
                    && set_value(v.event.event(), &mut v.values, &name, value)
                {
                    v.dirty = true;
                }
            }
            Cmd::VoiceVolume { id, volume } => {
                if valid_volume(volume) && let Some(v) = self.pcm_voices.get_mut(&id) {
                    v.opts.volume = volume;
                }
                if let Some(v) = self.voices.get_mut(&id) {
                    v.volume = volume;
                }
            }
            Cmd::Stop { id, fade } => self.stop_voice(id, fade),
            Cmd::StopAll => {
                let ids: Vec<Voice> = self.voices.keys().chain(self.pcm_voices.keys()).copied().collect();
                for id in ids {
                    self.stop_voice(id, true);
                }
            }
            Cmd::Listener { position, rotation } => {
                if !position.is_finite() || !rotation.is_finite() {
                    return;
                }
                let dt = self.listener_moved.elapsed().as_secs_f32();
                if dt > 0.001 {
                    self.listener_velocity = smooth_velocity(self.listener_velocity, self.listener.0, position, dt);
                    self.listener_moved = Instant::now();
                }
                self.listener = (position, rotation);
                if let Some(a) = &mut self.audio {
                    a.listener.set_position(position, Tween::default());
                    a.listener.set_orientation(rotation, Tween::default());
                }
            }
            Cmd::Volumes(v) => {
                self.volumes = v;
                if let Some(a) = &mut self.audio {
                    a.apply(&v);
                    for voice in self.voices.values_mut() {
                        for part in voice.parts.iter_mut().filter(|p| p.wet_db > WET_OFF) {
                            if let Some(t) = &mut part.track {
                                let _ = t.set_send(&a.reverb_send, send_db(part.wet_db, v.get(voice.category)), Tween::default());
                            }
                            if let Some(t) = &mut part.flat_track {
                                let _ = t.set_send(&a.reverb_send, send_db(part.wet_db, v.get(voice.category)), Tween::default());
                            }
                        }
                    }
                }
            }
            Cmd::Reverb { preset, weight } => {
                self.reverb_want = (preset, weight);
                self.apply_reverb();
            }
            Cmd::Moods(moods) => {
                self.moods = moods;
                self.apply_moods();
            }
            Cmd::Music { path, fade_in, fade_out } => self.play_music(path, fade_in, fade_out),
            Cmd::StopMusic { fade } => {
                self.music_request = None;
                self.music_token += 1;
                self.stats.music = None;
                if let Some(m) = self.music.take() {
                    let mut h = m.handle;
                    h.stop(Tween { duration: fade.max(Duration::from_millis(1)), ..Tween::default() });
                }
            }
            Cmd::Decoded { key, result, seconds, took } => self.decoded(key, result, seconds, took),
            Cmd::MusicReady { token, result } => self.music_ready(token, result),
            Cmd::PartStream { voice, token, key, result } => self.part_stream(voice, token, key, result),
            Cmd::Quit => {}
        }
    }

    /// Opens the mixer on first use. `false` when muted or when there is no device.
    fn ensure_audio(&mut self) -> bool {
        if !self.settings.audible || self.device_failed {
            return false;
        }
        if self.audio.is_some() {
            return true;
        }
        let built = (|| -> Result<Audio, String> {
            // every 3D sound of an event with several layers has a spatial track of its own: the default 128 tracks are few
            let settings = AudioManagerSettings { capacities: Capacities { sub_track_capacity: 512, ..Capacities::default() }, ..AudioManagerSettings::default() };
            let mut manager = AudioManager::<DefaultBackend>::new(settings).map_err(|e| e.to_string())?;
            let listener = manager.add_listener(self.listener.0, self.listener.1).map_err(|e| e.to_string())?;
            let mut muffles = HashMap::new();
            let mut group = |manager: &mut AudioManager<DefaultBackend>, c: Category| {
                let mut builder = TrackBuilder::new();
                if !matches!(c, Category::Ui | Category::UserMusic) {
                    muffles.insert(c, builder.add_effect(MuffleBuilder));
                }
                manager.add_sub_track(builder).map_err(|e| e.to_string())
            };
            let music = group(&mut manager, Category::Music)?;
            let user_music = group(&mut manager, Category::UserMusic)?;
            let sfx = group(&mut manager, Category::Sfx)?;
            let ambience = group(&mut manager, Category::Ambience)?;
            let voice = group(&mut manager, Category::Voice)?;
            let ui = group(&mut manager, Category::Ui)?;
            let mut send = SendTrackBuilder::new();
            let reverb = send.add_effect(ReverbBuilder);
            let reverb_send = manager.add_send_track(send).map_err(|e| e.to_string())?;
            Ok(Audio { manager, listener, music, user_music, sfx, ambience, voice, ui, reverb_send, reverb, buses: HashMap::new(), muffles })
        })();
        match built {
            Ok(mut a) => {
                a.apply(&self.volumes);
                self.audio = Some(a);
                self.apply_reverb();
                self.apply_mix_filters();
                true
            }
            Err(e) => {
                tracing::warn!("aa-audio: no audio device, staying silent: {e}");
                self.stats.device_error = Some(e);
                self.device_failed = true;
                false
            }
        }
    }

    /// Gives the reverb effect the wanted preset at its weight (once the mixer is open and the presets are read).
    fn apply_reverb(&mut self) {
        if !self.settings.audible {
            return;
        }
        let Some(audio) = &self.audio else { return };
        let (name, weight) = (&self.reverb_want.0, self.reverb_want.1);
        let params = match name {
            Some(n) => {
                let presets = self.presets.get_or_insert_with(|| Presets::load(Library::global().pak()));
                match presets.get(n) {
                    Some(p) => ReverbParams::of(p, weight),
                    None => {
                        tracing::debug!("aa-audio: no reverb preset {n}");
                        ReverbParams::OFF
                    }
                }
            }
            None => ReverbParams::OFF,
        };
        audio.reverb.set(params);
        self.stats.reverb_preset = name.clone().filter(|_| params.audible());
        self.stats.reverb_weight = if params.audible() { weight } else { 0.0 };
    }

    /// Works out the mix of the active moods (reading the mood file on first use).
    fn apply_moods(&mut self) {
        if !self.settings.audible {
            return;
        }
        if self.moods.is_empty() {
            self.mix = Mix::default();
            self.stats.mood = None;
        } else {
            let table = self.mood_table.get_or_insert_with(|| MoodTable::load(Library::global().pak()));
            self.mix = table.winner(&self.moods).map(Mix::of).unwrap_or_default();
            self.stats.mood = Some(self.mix.name.clone());
        }
        self.apply_mix_filters();
    }

    /// Sets the groups' low-pass filters to the mix's.
    fn apply_mix_filters(&mut self) {
        let Some(audio) = &self.audio else { return };
        for (group, filter) in &audio.muffles {
            filter.set(self.mix.lowpass(*group));
        }
    }

    /// The gain the mood gives sounds of the mixer category `leaf` now (gliding to the mix's).
    fn mood_gain(&mut self, leaf: &str) -> f32 {
        let target = self.mix.gain(leaf);
        *self.mood_cur.entry(leaf.to_string()).or_insert(target)
    }

    /// Moves the gains towards the mix's, `dt` seconds on.
    fn glide_mood(&mut self, dt: f32) {
        let k = 1.0 - (-dt / 0.5).exp();
        for (leaf, cur) in self.mood_cur.iter_mut() {
            let target = self.mix.gain(leaf);
            *cur = if (target - *cur).abs() < 0.002 { target } else { *cur + (target - *cur) * k };
        }
    }

    fn note_unknown(&mut self, path: &str) {
        self.stats.unknown_events += 1;
        if self.unknown.insert(path.to_ascii_lowercase()) {
            tracing::debug!("aa-audio: no event for {path}");
        }
    }

    fn finish(&mut self, id: Voice) {
        self.shared.live.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
        self.shared.finished.lock().unwrap_or_else(|e| e.into_inner()).push(id);
    }

    /// One overall limit and priority/age policy for event and PCM voices.
    fn admit_voice(&mut self, priority: u32) -> bool {
        let candidates = || self.voices.values().filter(|v| !v.stopping)
            .map(|v| (v.id, v.event.event().priority, v.requested))
            .chain(self.pcm_voices.iter().filter(|(_, v)| !v.stopping)
                .map(|(&id, v)| (id, v.opts.priority, v.requested)));
        if candidates().count() < self.settings.max_voices {
            return true;
        }
        let victim = candidates().max_by_key(|(id, p, t)| (*p, std::cmp::Reverse(*t), std::cmp::Reverse(id.0)));
        if let Some((id, p, _)) = victim && p >= priority {
            self.stop_voice(id, false);
            self.stats.stolen += 1;
            true
        } else {
            self.stats.refused += 1;
            false
        }
    }

    fn play_pcm(&mut self, id: Voice, clip: PcmClip, opts: PcmOptions, permit: PcmPermit) {
        self.stats.requested += 1;
        if permit.tracking.as_ref().is_some_and(|t| !t.claim()) {
            return;
        }
        if !self.settings.audible || !self.admit_voice(opts.priority) || !self.ensure_audio() {
            return;
        }
        match PcmVoice::begin(self.audio.as_mut().expect("opened"), &clip, opts, self.listener.0, permit) {
            Ok(voice) => {
                self.pcm_voices.insert(id, voice);
                self.stats.started += 1;
            }
            Err(e) => tracing::debug!("aa-audio: cannot play PCM: {e}"),
        }
    }

    /// Evaluate admission from current controls, including commands received since
    /// the last 16 ms tick. Never decode, pick a random wave or use mixer fade state.
    fn playback_gain(&self, event: &EventRef, values: &[f32], at: Option<Vec3>, volume: f32, category: Category) -> f32 {
        let ev = event.event();
        let distance = at.map(|p| p.distance(self.listener.0));
        let mut values = values.to_vec();
        if let (Some(d), Body::Complex { parameters, .. }) = (distance, &ev.body) {
            for i in distance_parameters(ev) {
                values[i] = d.clamp(parameters[i].min.min(parameters[i].max), parameters[i].min.max(parameters[i].max));
            }
        }
        let leaf = ev.category.rsplit('/').next().unwrap_or_default().to_ascii_lowercase();
        let mood = self.mood_cur.get(&leaf).copied().unwrap_or_else(|| self.mix.gain(&leaf));
        let category_gain = playback::authored_category_gain(&event.project.fev.categories, &ev.category)
            * mood * self.volumes.get(category) * self.volumes.master;
        playback::audibility(ev, &values, distance, volume, category_gain)
    }

    fn play(&mut self, id: Voice, path: &str, at: Option<Vec3>, opts: PlayOptions) {
        self.stats.requested += 1;
        if !self.settings.audible {
            return;
        }
        let Some(event) = Library::global().event(path) else {
            self.note_unknown(path);
            return;
        };
        let ev = event.event();
        let ekey = event.key();
        let is_3d = ev.is_3d() && at.is_some();
        let distance = at.filter(|_| is_3d).map(|p| p.distance(self.listener.0));
        let loops = ev.instances().iter().any(|i| i.loops());
        if let Some(d) = distance
            && ev.max_distance > 0.0
            && d > ev.max_distance
            && !loops
        {
            self.stats.culled += 1;
            return;
        }

        let at = if is_3d { at } else { None };
        let values = initial_values(ev, distance, &opts.params);
        let category = opts.category.unwrap_or_else(|| Category::of_mixer(&ev.category));
        let quietest = matches!(ev.playback_behaviour, 3 | 5);
        let mut same: Vec<Candidate> = self.voices.values()
            .filter(|v| v.event.key() == ekey && !v.stopping)
            .map(|v| Candidate { id: v.id, slot: v.playback_slot, requested: v.requested, gain: 0.0 })
            .collect();
        let playback_slot = if same.len() >= ev.max_playbacks as usize {
            if quietest {
                for candidate in &mut same {
                    let v = &self.voices[&candidate.id];
                    candidate.gain = self.playback_gain(&v.event, &v.values, v.at, v.volume, v.category);
                }
            }
            let incoming = if quietest { self.playback_gain(&event, &values, at, opts.volume, category) } else { 0.0 };
            let Some(victim) = playback::victim(ev.playback_behaviour, &same, incoming) else {
                self.stats.refused += 1;
                return;
            };
            let slot = self.voices[&victim].playback_slot;
            // Native steals dispatch stop(immediate=true), bypassing authored
            // fades. Preserve our existing 20 ms anti-click ramp.
            self.stop_voice(victim, false);
            self.stats.stolen += 1;
            slot
        } else {
            let cursor = self.playback_cursor.get(&ekey).copied().unwrap_or(0);
            let slot = playback::free_slot(ev.max_playbacks as usize, cursor, &same);
            self.playback_cursor.insert(ekey, slot);
            slot
        };
        if !self.admit_voice(ev.priority) || !self.ensure_audio() {
            return;
        }

        let parts = match &ev.body {
            Body::Simple(_) => vec![(0, 0)],
            Body::Complex { layers, .. } => layers.iter().enumerate().flat_map(|(l, layer)| (0..layer.instances.len()).map(move |i| (l, i))).collect(),
        }
        .into_iter()
        .map(|(layer, inst)| Part::new(layer, inst, ev.reverb_wet))
        .collect();
        let leaf = ev.category.rsplit('/').next().unwrap_or_default().to_ascii_lowercase();
        let distance_params = distance_parameters(ev);
        let persistent = !ev.one_shot() && matches!(&ev.body, Body::Complex { parameters, .. } if (0..parameters.len()).any(|i| !distance_params.contains(&i)));
        let mut voice = ActiveVoice {
            id,
            playback_slot,
            event: event.clone(),
            category,
            leaf: leaf.clone(),
            mood: self.mood_gain(&leaf),
            at,
            moved: false,
            velocity: Vec3::ZERO,
            moved_at: Instant::now(),
            volume: opts.volume,
            values,
            distance_params,
            dirty: true,
            persistent,
            parts,
            requested: Instant::now(),
            last_tick: Instant::now(),
            started: false,
            stopping: false,
        };
        self.reconcile(&mut voice);
        self.try_start(&mut voice);
        self.voices.insert(id, voice);
    }

    /// Brings the parts in line with the parameters: starts the ones that became active, stops the
    /// ones that no longer are.
    fn reconcile(&mut self, voice: &mut ActiveVoice) {
        voice.dirty = false;
        let event = voice.event.clone();
        let active = active_instances(event.event(), &voice.values);
        for i in 0..voice.parts.len() {
            let is_active = active.contains(&(voice.parts[i].layer, voice.parts[i].inst));
            let part = &mut voice.parts[i];
            match (part.state, is_active) {
                (PartState::Idle, true) => {
                    part.restart_at = None;
                    let spawn = spawn_range(&event, part.layer, part.inst);
                    if let Some((min, max, _)) = spawn
                        && part.next_spawn.is_none()
                    {
                        part.next_spawn = Some(Instant::now() + spawn_gap(min, max, &mut self.rng));
                    }
                    let Some(p) = start_part(&event, part.layer, part.inst, &mut self.pick, &mut self.rng) else {
                        part.state = PartState::Done;
                        // a silent pick of a looping definition: try the next one a moment later
                        if spawn.is_none() && instance_loops(&event, part.layer, part.inst) {
                            part.restart_at = Some(Instant::now() + Duration::from_millis(500));
                        }
                        continue;
                    };
                    self.prepare(voice.id, &event, part, p);
                }
                (PartState::Loading | PartState::Playing, false) => {
                    if let Some(h) = &mut part.handle {
                        h.stop(tween_ms(30));
                    }
                    part.handle = None;
                    part.stream = None;
                    part.stream_token = None;
                    part.state = PartState::Idle;
                    part.next_spawn = None;
                }
                (PartState::Done, false) => {
                    part.state = PartState::Idle;
                    part.next_spawn = None;
                    part.restart_at = None;
                }
                _ => {}
            }
        }
        // sounds that were spawned are copies of the instance: gone with it
        voice.parts.retain(|p| !(p.spawned && matches!(p.state, PartState::Idle | PartState::Done)));
    }

    /// Gets the sound of a picked part ready: from the cache, else decoded (or opened for streaming) on a worker.
    fn prepare(&mut self, id: Voice, event: &EventRef, part: &mut Part, p: Planned) {
        let Some(bank) = event.project.fev.banks.get(p.bank as usize) else {
            part.state = PartState::Done;
            return;
        };
        let key: Key = (format!("{}/{}", event.project.dir, bank.name.to_ascii_lowercase()), p.index);
        part.sample = self.cache.get(&key);
        part.stream = None;
        part.stream_token = None;
        if part.sample.is_none() {
            if p.looping && p.length_ms >= STREAM_MS && !self.not_streamable.contains(&key) {
                self.request_stream(key.clone(), id, part, event.project.clone(), p.bank, p.index);
            } else {
                self.request_decode(key.clone(), id, event.project.clone(), p.bank, p.index);
            }
        }
        part.key = Some(key);
        part.planned = Some(p);
        part.state = PartState::Loading;
    }

    /// Starts the parts that are waiting and have their sound. The first start waits for all the
    /// parts that are active then, so that the layers begin together.
    fn try_start(&mut self, voice: &mut ActiveVoice) {
        let loading = voice.parts.iter().filter(|p| p.state == PartState::Loading).count();
        if !voice.started {
            let ready = voice.parts.iter().filter(|p| p.state == PartState::Loading).all(Part::ready);
            if loading > 0 && !ready {
                return;
            }
            voice.started = true;
            if loading > 0 {
                self.stats.started += 1;
            }
        }
        let listener = (self.listener.0, self.listener_velocity);
        let volumes = self.volumes;
        let Some(audio) = &mut self.audio else { return };
        for i in 0..voice.parts.len() {
            if voice.parts[i].state == PartState::Loading && voice.parts[i].ready() {
                begin_part(audio, voice, i, listener, &volumes);
            }
        }
    }

    fn request_decode(&mut self, key: Key, voice: Voice, project: Arc<Project>, bank: u32, index: u32) {
        let waiters = self.pending.entry(key.clone()).or_default();
        let first = waiters.is_empty();
        if !waiters.contains(&voice) {
            waiters.push(voice);
        }
        if !first {
            return;
        }
        let tx = self.tx.clone();
        aa_cry::workers::Workers::global().spawn(-1000.0, move || {
            let t = Instant::now();
            let mut seconds = 0.0;
            let result = (|| -> Result<Arc<Sample>, Error> {
                let lib = Library::global();
                let bank = lib.bank(&project, bank)?;
                let pcm = lib.decode(&bank, index as usize)?;
                seconds = pcm.seconds() as f64;
                Ok(Arc::new(Sample::from_pcm(&pcm)))
            })()
            .map_err(|e| format!("{}/{}: {e}", project.name, index));
            let _ = tx.send(Cmd::Decoded { key, result, seconds, took: t.elapsed() });
        });
    }

    /// Opens a long loop for streaming on a worker: only its compressed bytes are kept.
    fn request_stream(&mut self, key: Key, voice: Voice, part: &mut Part, project: Arc<Project>, bank: u32, index: u32) {
        self.stream_token += 1;
        let token = self.stream_token;
        part.stream_token = Some(token);
        let tx = self.tx.clone();
        aa_cry::workers::Workers::global().spawn(-1000.0, move || {
            let result = (|| -> Result<OpenedStream, Error> {
                let lib = Library::global();
                let bank = lib.bank(&project, bank)?;
                if bank.fsb.codec != Codec::Vorbis {
                    return Err(Error::Codec(bank.fsb.codec));
                }
                let sub = bank.fsb.subsounds.get(index as usize).ok_or(Error::NoSubsound(index as usize))?;
                let data = lib.read_subsound(&bank, index as usize)?;
                Ok(OpenedStream { stream: vorbis::Stream::new(sub, data)?, loop_points: sub.loop_points })
            })()
            .map_err(|e| format!("{}/{}: {e}", project.name, index));
            let _ = tx.send(Cmd::PartStream { voice, token, key, result });
        });
    }

    /// A stream is open: the part that asked starts with it. A sound that cannot be streamed is decoded instead.
    fn part_stream(&mut self, id: Voice, token: u64, key: Key, result: Result<OpenedStream, String>) {
        let Some(mut voice) = self.voices.remove(&id) else { return };
        let project = voice.event.project.clone();
        let mut fallback = None;
        if let Some(part) = voice.parts.iter_mut().find(|p| p.stream_token == Some(token) && p.state == PartState::Loading) {
            match result {
                Ok(s) => part.stream = Some(s),
                Err(e) => {
                    tracing::debug!("aa-audio: cannot stream {e}");
                    part.stream_token = None;
                    if let Some(p) = &part.planned {
                        fallback = Some((p.bank, p.index));
                    }
                }
            }
        }
        if let Some((bank, index)) = fallback {
            self.not_streamable.insert(key.clone());
            self.request_decode(key, id, project, bank, index);
        }
        self.try_start(&mut voice);
        self.voices.insert(id, voice);
    }

    fn decoded(&mut self, key: Key, result: Result<Arc<Sample>, String>, seconds: f64, took: Duration) {
        let waiters = self.pending.remove(&key).unwrap_or_default();
        match &result {
            Ok(sample) => {
                self.stats.decoded_sounds += 1;
                self.stats.decoded_seconds += seconds;
                self.stats.decode_wall_seconds += took.as_secs_f64();
                self.cache.insert(key.clone(), sample.clone());
            }
            Err(e) => {
                self.stats.decode_failures += 1;
                tracing::debug!("aa-audio: cannot decode {e}");
            }
        }
        for id in waiters {
            let Some(mut voice) = self.voices.remove(&id) else { continue };
            for part in voice.parts.iter_mut().filter(|p| p.state == PartState::Loading && p.key.as_ref() == Some(&key)) {
                match &result {
                    Ok(s) => part.sample = Some(s.clone()),
                    Err(_) => part.state = PartState::Done,
                }
            }
            self.try_start(&mut voice);
            self.voices.insert(id, voice);
        }
    }

    fn stop_voice(&mut self, id: Voice, fade: bool) {
        if let Some(voice) = self.pcm_voices.get_mut(&id) {
            if !voice.stopping {
                voice.stopping = true;
                voice.handle.stop(tween_ms(20));
            }
            return;
        }
        let Some(voice) = self.voices.get_mut(&id) else { return };
        voice.stopping = true;
        let ev_fade = voice.event.event().fade_out_ms;
        for part in voice.parts.iter_mut() {
            if let Some(h) = &mut part.handle {
                let ms = if fade { ev_fade.max(part.planned.as_ref().map_or(0, |p| p.fade_out_ms)).max(20) } else { 20 };
                h.stop(tween_ms(ms));
            }
            if part.state == PartState::Loading {
                part.state = PartState::Done;
            }
        }
        if !voice.started || !voice.live() {
            self.voices.remove(&id);
            self.finish(id);
        }
    }

    fn update(&mut self) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_update).as_secs_f32().min(0.5);
        self.last_update = now;
        self.glide_mood(dt);
        if self.listener_moved.elapsed() > Duration::from_millis(150) {
            self.listener_velocity = Vec3::ZERO;
        }
        let ids: Vec<Voice> = self.voices.keys().copied().collect();
        let listener = self.listener.0;
        for id in ids {
            let Some(mut voice) = self.voices.remove(&id) else { continue };
            let keep = self.tick(&mut voice, listener);
            if keep {
                self.voices.insert(id, voice);
            } else {
                self.finish(id);
            }
        }
        let mut ended = Vec::new();
        for (&id, voice) in &mut self.pcm_voices {
            if voice.handle.state() == PlaybackState::Stopped {
                ended.push(id);
            } else if !voice.stopping {
                let new_db = db(voice.opts.gain(listener)).0;
                if (new_db - voice.last_db).abs() > 0.2 {
                    voice.handle.set_volume(Decibels(new_db), tween_ms(30));
                    voice.last_db = new_db;
                }
            }
        }
        for id in ended {
            self.pcm_voices.remove(&id);
            self.finish(id);
        }
        if self.music.as_ref().is_some_and(|m| m.handle.state() == PlaybackState::Stopped) {
            // (a track we stopped is no longer here: this one played out)
            self.music = None;
            self.stats.music = None;
            self.stats.music_ended += 1;
        }
        if let Some(leaf) = self.music.as_ref().map(|m| m.leaf.clone()) {
            let mood = self.mood_gain(&leaf);
            if let Some(m) = &mut self.music {
                let new_db = db(m.gain * mood).0;
                if (new_db - m.last_db).abs() > 0.2 {
                    m.handle.set_volume(Decibels(new_db), tween_ms(100));
                    m.last_db = new_db;
                }
            }
        }
    }

    /// One update of a voice; `false` when it has ended.
    fn tick(&mut self, voice: &mut ActiveVoice, listener: Vec3) -> bool {
        if !voice.started && voice.requested.elapsed() > Duration::from_secs(5) {
            return false;
        }
        let now = Instant::now();
        let dt = now.duration_since(voice.last_tick).as_secs_f32();
        voice.last_tick = now;
        if voice.moved_at.elapsed() > Duration::from_millis(150) {
            voice.velocity = Vec3::ZERO;
        }
        if !voice.stopping {
            if voice.started {
                // parameters with a velocity (`time_loop`) run by themselves
                let event = voice.event.clone();
                if let Body::Complex { parameters, .. } = &event.event().body {
                    for (i, p) in parameters.iter().enumerate() {
                        if p.velocity != 0.0 && !voice.distance_params.contains(&i) {
                            let span = (p.max - p.min).max(1e-3);
                            let mut v = voice.values[i] + p.velocity * dt;
                            if v > p.max {
                                v = if p.flags & 0x8 != 0 { p.min + (v - p.min) % span } else { p.max };
                            }
                            voice.values[i] = v;
                            voice.dirty = true;
                        }
                    }
                }
            }
            if let Some(at) = voice.at {
                let d = at.distance(listener);
                let event = voice.event.clone();
                for &p in &voice.distance_params {
                    if (voice.values[p] - d).abs() > 0.02
                        && let Body::Complex { parameters, .. } = &event.event().body
                    {
                        voice.values[p] = d.clamp(parameters[p].min.min(parameters[p].max), parameters[p].max.max(parameters[p].min));
                        voice.dirty = true;
                    }
                }
            }
            // the next wave of a looping playlist, the next try after a silent pick
            for part in voice.parts.iter_mut() {
                if part.state == PartState::Done && part.restart_at.is_some_and(|t| now >= t) {
                    part.restart_at = None;
                    part.state = PartState::Idle;
                    voice.dirty = true;
                }
            }
            if voice.dirty {
                self.reconcile(voice);
            }
            if voice.started {
                self.spawn_more(voice, now);
            }
            self.try_start(voice);
        }
        voice.mood = self.mood_gain(&voice.leaf);
        let moved = std::mem::take(&mut voice.moved);
        let listener_velocity = self.listener_velocity;
        for i in 0..voice.parts.len() {
            if voice.parts[i].state != PartState::Playing {
                continue;
            }
            if voice.parts[i].handle.as_ref().is_none_or(|h| h.state() == PlaybackState::Stopped) {
                let part = &mut voice.parts[i];
                part.handle = None;
                part.track = None;
                part.flat_track = None;
                part.filter = None;
                part.highpass = None;
                part.highpass_state = None;
                part.state = PartState::Done;
                // a looping playlist goes on with the next pick of its definition
                if !voice.stopping && !part.spawned && part.planned.as_ref().is_some_and(|p| p.looping && p.playlist) {
                    part.restart_at = Some(now);
                }
                continue;
            }
            let target = voice.target_gain(&voice.parts[i], listener);
            let rate = part_rate(voice, &voice.parts[i], (listener, listener_velocity));
            let pan = layer_pan(voice.event.event(), &voice.values, voice.parts[i].layer);
            let cutoff = if voice.parts[i].filter.is_some() { layer_cutoff(voice.event.event(), &voice.values, voice.parts[i].layer) } else { None };
            let highpass = layer_highpass(&voice.event, &voice.values, voice.parts[i].layer);
            let part = &mut voice.parts[i];
            if moved && let (Some(at), Some(track)) = (voice.at, &mut part.track) {
                track.set_position(at, Tween::default());
            }
            if pan != part.pan && let Some(track) = &mut part.track {
                track.set_spatialization_strength(pan, tween_ms(30));
                part.pan = pan;
            }
            let new_db = db(target).0;
            if (new_db - part.last_db).abs() > 0.2 {
                if let Some(h) = &mut part.handle {
                    h.set_volume(Decibels(new_db), tween_ms(30));
                }
                part.last_db = new_db;
            }
            // pitch curves and doppler
            if (rate / part.rate).ln().abs() > 0.003 {
                if let Some(h) = &mut part.handle {
                    h.set_rate(rate, tween_ms(40));
                }
                part.rate = rate;
            }
            if cutoff != part.cutoff
                && let Some(f) = &part.filter
            {
                f.set(cutoff);
                part.cutoff = cutoff;
            }
            if let (Some(state), Some(previous), Some(filter)) = (highpass, part.highpass_state, &mut part.highpass) {
                if state.cutoff != previous.cutoff {
                    filter.set_cutoff(f64::from(state.cutoff), tween_ms(0));
                }
                if state.enabled != previous.enabled {
                    filter.set_mix(if state.enabled { kira::Mix::WET } else { kira::Mix::DRY }, tween_ms(0));
                }
                part.highpass_state = Some(state);
            }
        }
        voice.parts.retain(|p| !(p.spawned && p.state == PartState::Done));
        if voice.stopping || !voice.persistent { voice.live() || !voice.started || voice.waiting() } else { true }
    }

    /// Spawns the sounds that are due: while an instance is active, its sound definition starts a new sound every
    /// `min..=max` ms (the template's spawn time) as long as fewer than `max spawned` of them are playing. Each is one pass of a
    /// wave picked as for any trigger.
    fn spawn_more(&mut self, voice: &mut ActiveVoice, now: Instant) {
        if voice.stopping || voice.event.event().one_shot() {
            return;
        }
        let event = voice.event.clone();
        for i in 0..voice.parts.len() {
            let part = &voice.parts[i];
            if part.spawned || part.state == PartState::Idle || !part.next_spawn.is_some_and(|t| now >= t) {
                continue;
            }
            let (layer, inst, wet_db) = (part.layer, part.inst, part.wet_db);
            let Some((min, max, cap)) = spawn_range(&event, layer, inst) else { continue };
            voice.parts[i].next_spawn = Some(now + spawn_gap(min, max, &mut self.rng));
            let playing = voice.parts.iter().filter(|q| q.layer == layer && q.inst == inst && matches!(q.state, PartState::Loading | PartState::Playing)).count();
            if playing as u32 >= cap {
                continue;
            }
            let Some(mut p) = start_part(&event, layer, inst, &mut self.pick, &mut self.rng) else { continue };
            // the copies play once; only the instance's own sound loops
            p.looping = false;
            let mut part = Part::new(layer, inst, wet_db);
            part.spawned = true;
            self.prepare(voice.id, &event, &mut part, p);
            voice.parts.push(part);
        }
    }

    fn play_music(&mut self, path: String, fade_in: Option<Duration>, fade_out: Option<Duration>) {
        self.stats.requested += 1;
        if !self.settings.audible {
            return;
        }
        let key = path.to_ascii_lowercase();
        if self.music.as_ref().is_some_and(|m| m.path == key) || self.music_request.as_ref().is_some_and(|r| r.0 == key) {
            return;
        }
        let Some(event) = Library::global().event(&path) else {
            self.note_unknown(&path);
            return;
        };
        let values = initial_values(event.event(), None, &[]);
        let planned = plan(&event, &values, &mut self.pick, &mut self.rng);
        let Some(p) = planned.into_iter().next() else { return };
        if !self.ensure_audio() {
            return;
        }
        self.music_token += 1;
        let token = self.music_token;
        self.music_request = Some((key, fade_in, fade_out));
        let tx = self.tx.clone();
        let project = event.project.clone();
        // FMOD plays a music event through: the track repeats when its instance loops.
        let looping = p.looping;
        let (event_in, event_out) = (Duration::from_millis(p.fade_in_ms.into()), Duration::from_millis(p.fade_out_ms.into()));
        let wet_db = event.event().reverb_wet;
        let leaf = event.event().category.rsplit('/').next().unwrap_or_default().to_ascii_lowercase();
        aa_cry::workers::Workers::global().spawn(-1000.0, move || {
            let result = (|| -> Result<Prepared, Error> {
                let lib = Library::global();
                let bank = lib.bank(&project, p.bank)?;
                if bank.fsb.codec != Codec::Vorbis {
                    return Err(Error::Codec(bank.fsb.codec));
                }
                let sub = bank.fsb.subsounds.get(p.index as usize).ok_or(Error::NoSubsound(p.index as usize))?;
                let data = lib.read_subsound(&bank, p.index as usize)?;
                let stream = vorbis::Stream::new(sub, data)?;
                Ok(Prepared { stream, looping: looping.then_some(sub.loop_points), gain: p.gain, rate: p.rate, wet_db, leaf, fade_in: event_in, fade_out: event_out })
            })()
            .map_err(|e| e.to_string());
            let _ = tx.send(Cmd::MusicReady { token, result });
        });
    }

    fn music_ready(&mut self, token: u64, result: Result<Prepared, String>) {
        let Some((path, fade_in, fade_out)) = self.music_request.take_if(|_| token == self.music_token) else { return };
        let prepared = match result {
            Ok(p) => p,
            Err(e) => {
                tracing::debug!("aa-audio: cannot stream {path}: {e}");
                return;
            }
        };
        if self.audio.is_none() {
            return;
        }
        let mood = self.mood_gain(&prepared.leaf);
        let Some(audio) = &mut self.audio else { return };
        let fade_in = fade_in.unwrap_or(prepared.fade_in);
        if let Some(old) = self.music.take() {
            let mut h = old.handle;
            // asked for with the event's fades, the old track leaves the way its own event says
            let fade_out = fade_out.unwrap_or(old.fade_out);
            h.stop(Tween { duration: fade_out.max(Duration::from_millis(1)), ..Tween::default() });
        }
        let mut data = StreamingSoundData::from_decoder(StreamDecoder::new(prepared.stream)).volume(db(prepared.gain * mood)).playback_rate(prepared.rate as f64);
        if let Some(points) = prepared.looping {
            data = data.loop_region(loop_region(points));
        }
        if !fade_in.is_zero() {
            data = data.fade_in_tween(Tween { duration: fade_in, ..Tween::default() });
        }
        match audio.bus(Category::Music, prepared.wet_db, &self.volumes).play(data) {
            Ok(handle) => {
                self.stats.started += 1;
                self.stats.music = Some(path.clone());
                self.music = Some(CurrentMusic { path, handle, gain: prepared.gain, leaf: prepared.leaf.clone(), last_db: db(prepared.gain * mood).0, fade_out: prepared.fade_out });
            }
            Err(e) => tracing::debug!("aa-audio: cannot play music {path}: {e}"),
        }
    }
}

/// The part of a sound that repeats: the loop points from the bank, else all of it.
fn loop_region(points: Option<(u32, u32)>) -> Region {
    match points {
        Some((a, b)) if b > a => Region { start: PlaybackPosition::Samples(a as usize), end: EndPosition::Custom(PlaybackPosition::Samples(b as usize + 1)) },
        _ => Region { start: PlaybackPosition::Samples(0), end: EndPosition::EndOfAudio },
    }
}

/// `(min ms, max ms, most playing)` of the spawning of an instance's sound definition, `None` when it does not spawn.
fn spawn_range(event: &EventRef, layer: usize, inst: usize) -> Option<(u32, u32, u32)> {
    let fev = &event.project.fev;
    let instance = match &event.event().body {
        Body::Simple(i) => i,
        Body::Complex { layers, .. } => layers.get(layer)?.instances.get(inst)?,
    };
    let template = fev.templates.get(fev.sound_defs.get(instance.sound_def)?.template)?;
    (template.spawn_time_max_ms > 0).then_some((template.spawn_time_min_ms, template.spawn_time_max_ms, template.max_spawned.max(1)))
}

fn instance_loops(event: &EventRef, layer: usize, inst: usize) -> bool {
    match &event.event().body {
        Body::Simple(i) => i.loops(),
        Body::Complex { layers, .. } => layers.get(layer).and_then(|l| l.instances.get(inst)).is_some_and(|i| i.loops()),
    }
}

/// A random time between spawns.
fn spawn_gap(min: u32, max: u32, rng: &mut Rng) -> Duration {
    let ms = min as f32 + max.saturating_sub(min) as f32 * rng.unit();
    Duration::from_millis(ms.max(1.0) as u64)
}

/// Velocity after a move from `old` to `new` in `dt` seconds, smoothed against the last one; a jump (a teleport, a camera cut)
/// is not a speed.
fn smooth_velocity(last: Vec3, old: Vec3, new: Vec3, dt: f32) -> Vec3 {
    let now = (new - old) / dt;
    if now.length() > 120.0 { Vec3::ZERO } else { last.lerp(now, 0.35) }
}

/// The playback rate of a part now, with `(listener position, listener velocity)`: the sound's own pitch, its layer's pitch
/// curves and the doppler shift of a moving source or listener (for an event with a doppler level).
fn part_rate(voice: &ActiveVoice, part: &Part, listener: (Vec3, Vec3)) -> f32 {
    let Some(p) = &part.planned else { return 1.0 };
    let ev = voice.event.event();
    let doppler = voice.at.map_or(1.0, |at| doppler_factor(ev.doppler, at - listener.0, voice.velocity, listener.1));
    p.rate * layer_pitch(ev, &voice.values, part.layer) * doppler
}

/// What the mixer is given to play.
enum Data {
    Static(StaticSoundData),
    Stream(StreamingSoundData<Error>),
}

/// Plays `Data` on a track (or sub track), whichever kind of sound it is.
macro_rules! start {
    ($track:expr, $data:expr) => {
        match $data {
            Data::Static(d) => $track.play(d).map(Handle::Static).map_err(|e| e.to_string()),
            Data::Stream(d) => $track.play(d).map(Handle::Stream).map_err(|e| e.to_string()),
        }
    };
}

/// PCM uses the very same group and spatial tracks as events, but no authored FEV gain/loop/DSP.
fn begin_pcm<B: kira::backend::Backend>(
    audio: &mut Audio<B>, clip: &PcmClip, opts: PcmOptions, listener: Vec3,
) -> Result<(StaticSoundHandle, Option<SpatialTrackHandle>), String> {
    let data = StaticSoundData {
        sample_rate: clip.0.rate, frames: clip.0.frames.clone(),
        settings: StaticSoundSettings::default(), slice: None,
    }.volume(db(opts.gain(listener)));
    if let Some(spatial) = opts.spatial {
        let builder = SpatialTrackBuilder::new().attenuation_function(None)
            .spatialization_strength(1.0).persist_until_sounds_finish(true);
        let listener_id = audio.listener.id();
        let mut track = audio.track(opts.category).add_spatial_sub_track(listener_id, spatial.position, builder)
            .map_err(|e| e.to_string())?;
        let handle = track.play(data).map_err(|e| e.to_string())?;
        Ok((handle, Some(track)))
    } else {
        let handle = audio.track(opts.category).play(data).map_err(|e| e.to_string())?;
        Ok((handle, None))
    }
}

/// Starts the sound of part `i`: on a spatial track for a 3D voice, on its group's track (or the reverb bus of its wet level)
/// otherwise, on a track of its own when its layer has a low-pass. Both send to the reverb by the event's wet level.
fn begin_part<B: kira::backend::Backend>(audio: &mut Audio<B>, voice: &mut ActiveVoice, i: usize, listener: (Vec3, Vec3), volumes: &Volumes) {
    let gain = voice.target_gain(&voice.parts[i], listener.0);
    let rate = part_rate(voice, &voice.parts[i], listener);
    let ev = voice.event.event();
    let layer = voice.parts[i].layer;
    let pan = layer_pan(ev, &voice.values, layer);
    let filtered = layer_filtered(ev, layer);
    let cutoff = if filtered { layer_cutoff(ev, &voice.values, layer) } else { None };
    let highpass_state = layer_highpass(&voice.event, &voice.values, layer);
    let highpass_builder = highpass_state.map(|s| FilterBuilder::new().mode(FilterMode::HighPass)
        .cutoff(f64::from(s.cutoff)).resonance(1.0 / 1.9)
        .mix(if s.enabled { kira::Mix::WET } else { kira::Mix::DRY }));
    let category = voice.category;
    let at = voice.at;
    let part = &mut voice.parts[i];
    let wet_db = part.wet_db;
    let Some(p) = part.planned.clone() else {
        part.state = PartState::Done;
        return;
    };
    // a playlist goes on with its next wave when one ends instead of repeating it
    let repeats = p.looping && !p.playlist;
    let data = if let Some(opened) = part.stream.take() {
        let mut d = StreamingSoundData::from_decoder(StreamDecoder::new(opened.stream)).volume(db(gain)).playback_rate(f64::from(rate));
        if repeats {
            d = d.loop_region(loop_region(opened.loop_points));
        }
        if p.fade_in_ms > 0 {
            d = d.fade_in_tween(tween_ms(p.fade_in_ms));
        }
        Data::Stream(d)
    } else if let Some(sample) = part.sample.clone() {
        let mut settings = StaticSoundSettings::default();
        if repeats {
            settings = settings.loop_region(loop_region(sample.loop_points));
        }
        let mut d = StaticSoundData { sample_rate: sample.rate, frames: sample.frames.clone(), settings, slice: None }.volume(db(gain)).playback_rate(f64::from(rate));
        if p.fade_in_ms > 0 {
            d = d.fade_in_tween(tween_ms(p.fade_in_ms));
        }
        Data::Static(d)
    } else {
        part.state = PartState::Done;
        return;
    };
    part.last_db = db(gain).0;
    part.rate = rate;
    part.pan = pan;
    let mut filter = None;
    let mut highpass = None;
    let result = match at {
        Some(at) => {
            let mut builder = SpatialTrackBuilder::new().attenuation_function(None).spatialization_strength(pan).persist_until_sounds_finish(true);
            if let Some(hp) = highpass_builder { highpass = Some(builder.add_effect(hp)); }
            if filtered {
                filter = Some(builder.add_effect(MuffleBuilder));
            }
            if wet_db > WET_OFF {
                builder = builder.with_send(&audio.reverb_send, send_db(wet_db, volumes.get(category)));
            }
            let listener_id = audio.listener.id();
            match audio.track(category).add_spatial_sub_track(listener_id, at, builder) {
                Ok(mut track) => {
                    let r = start!(track, data);
                    part.track = Some(track);
                    r
                }
                Err(e) => Err(e.to_string()),
            }
        }
        None if filtered || highpass_builder.is_some() => {
            let mut builder = TrackBuilder::new().persist_until_sounds_finish(true);
            if let Some(hp) = highpass_builder { highpass = Some(builder.add_effect(hp)); }
            if filtered { filter = Some(builder.add_effect(MuffleBuilder)); }
            if wet_db > WET_OFF {
                builder = builder.with_send(&audio.reverb_send, send_db(wet_db, volumes.get(category)));
            }
            match audio.track(category).add_sub_track(builder) {
                Ok(mut track) => {
                    let r = start!(track, data);
                    part.flat_track = Some(track);
                    r
                }
                Err(e) => Err(e.to_string()),
            }
        }
        None => {
            let track = audio.bus(category, wet_db, volumes);
            start!(track, data)
        }
    };
    match result {
        Ok(h) => {
            if let Some(f) = &filter {
                f.set(cutoff);
            }
            part.filter = filter;
            part.highpass = highpass;
            part.highpass_state = highpass_state;
            part.cutoff = cutoff;
            part.handle = Some(h);
            part.state = PartState::Playing;
        }
        Err(e) => {
            tracing::debug!("aa-audio: cannot play {}: {e}", p.name);
            part.track = None;
            part.flat_track = None;
            part.state = PartState::Done;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PcmSpatial;

    /// A deterministic engine channel, held before the player can receive commands.
    fn queued_engine(settings: Settings) -> (Engine, Player, Receiver<Cmd>) {
        let (tx, rx) = channel();
        let shared = Arc::new(Shared::new(settings.audible));
        let engine = Engine { tx: tx.clone(), shared: shared.clone(), _guard: Arc::new(Guard(tx.clone())) };
        let player = Player::new(settings, shared, tx);
        (engine, player, rx)
    }

    #[test]
    fn pcm_queue_is_bounded_and_drops_release_all_slots() {
        let (engine, player, rx) = queued_engine(Settings::default());
        let clip = PcmClip::mono(24_000, &[0.25; 240]).unwrap();
        let mut opts = PcmOptions::new(Category::Music);
        opts.volume = f32::NAN;
        assert_eq!(engine.play_pcm(&clip, opts), Err(PcmError::Options));
        assert_eq!(engine.shared.pcm_slots.load(Ordering::SeqCst), 0);
        assert_eq!(engine.shared.next.load(Ordering::Relaxed), 1);
        opts.volume = 1.0;
        let ids: Vec<_> = (0..MAX_PCM_VOICES).map(|_| engine.play_pcm(&clip, opts).unwrap()).collect();
        assert_eq!(engine.play_pcm(&clip, opts), Err(PcmError::Busy));
        assert!(ids.iter().all(|&id| engine.is_playing(id)));
        assert_eq!(engine.shared.pcm_slots.load(Ordering::SeqCst), MAX_PCM_VOICES);
        // Player exit releases unstarted IDs; channel drop releases their clip/slot ownership.
        drop(player);
        drop(rx);
        assert_eq!(engine.shared.pcm_slots.load(Ordering::SeqCst), 0);
        assert!(ids.iter().all(|&id| !engine.is_playing(id)));
        let mut finished = engine.take_finished();
        finished.sort_by_key(|v| v.0);
        assert_eq!(finished, ids);
        assert!(engine.take_finished().is_empty());
        assert_eq!(engine.play_pcm(&clip, opts), Err(PcmError::EngineStopped));
        assert_eq!(engine.shared.pcm_slots.load(Ordering::SeqCst), 0);
        assert!(engine.shared.live.lock().unwrap().is_empty());
    }

    #[test]
    fn pcm_refusal_device_failure_and_muting_complete_once_and_release_quota() {
        let clip = PcmClip::mono(24_000, &[0.25; 240]).unwrap();
        let opts = PcmOptions::new(Category::Music);
        for (audible, max_voices, failed) in [(true, 0, false), (true, 64, true), (false, 64, false)] {
            let (engine, mut player, rx) = queued_engine(Settings { audible, max_voices, ..Settings::default() });
            player.device_failed = failed; // deterministic already-failed device path
            for _ in 0..MAX_PCM_VOICES * 2 {
                let id = engine.play_pcm(&clip, opts).unwrap();
                assert_eq!(engine.is_playing(id), audible);
                player.handle(rx.recv().unwrap());
                assert!(!engine.is_playing(id));
                assert_eq!(engine.take_finished(), [id]);
                player.handle(Cmd::Stop { id, fade: true });
                player.handle(Cmd::StopAll);
                player.update();
                assert!(engine.take_finished().is_empty());
                assert_eq!(engine.shared.pcm_slots.load(Ordering::SeqCst), 0);
            }
            assert!(player.audio.is_none(), "no device should be opened for this path");
            if max_voices == 0 {
                assert_eq!(player.stats.refused, (MAX_PCM_VOICES * 2) as u64);
            }
        }
    }

    #[test]
    fn last_engine_drop_quits_and_releases_queued_pcm() {
        let (engine, player, rx) = queued_engine(Settings::default());
        let slots = engine.shared.pcm_slots.clone();
        let clip = PcmClip::mono(24_000, &[0.25; 240]).unwrap();
        let _id = engine.play_pcm(&clip, PcmOptions::new(Category::Music)).unwrap();
        assert_eq!(slots.load(Ordering::SeqCst), 1);
        drop(engine);
        let cmd = rx.recv().unwrap();
        assert!(matches!(cmd, Cmd::Pcm { .. }));
        drop(cmd);
        assert!(matches!(rx.recv().unwrap(), Cmd::Quit));
        drop(rx);
        drop(player);
        assert_eq!(slots.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn tracked_pcm_pending_stop_and_stop_all_never_admit() {
        let (engine, mut player, rx) = queued_engine(Settings::default());
        let clip = PcmClip::mono(24_000, &[0.25]).unwrap();
        let opts = PcmOptions::new(Category::UserMusic);
        let (id, observer) = engine.play_pcm_tracked(&clip, opts).unwrap();
        let clone = observer.clone();
        assert!(!observer.was_started() && !observer.is_finished());
        assert_eq!(engine.shared.pcm_pending.lock().unwrap().len(), 1);
        engine.clone().stop(id, false);
        player.handle(rx.recv().unwrap());
        assert!(clone.is_finished() && !clone.was_started());
        player.handle(rx.recv().unwrap());
        assert_eq!(engine.take_finished(), [id]);

        let requests: Vec<_> = (0..MAX_PCM_VOICES).map(|_| engine.play_pcm_tracked(&clip, opts).unwrap()).collect();
        assert_eq!(engine.shared.pcm_pending.lock().unwrap().len(), MAX_PCM_VOICES);
        assert!(matches!(engine.play_pcm_tracked(&clip, opts), Err(PcmError::Busy)));
        engine.stop_all();
        while let Ok(cmd) = rx.try_recv() {
            player.handle(cmd);
        }
        player.update();
        assert!(requests.iter().all(|(id, o)| !engine.is_playing(*id) && o.is_finished() && !o.was_started()));
        assert!(observer.is_finished() && !observer.was_started(), "preclaim cancel is permanently terminal");
        assert_eq!(engine.take_finished(), requests.iter().map(|r| r.0).collect::<Vec<_>>());
        assert!(engine.take_finished().is_empty());
        assert!(engine.shared.pcm_pending.lock().unwrap().is_empty());
        assert_eq!(engine.shared.pcm_slots.load(Ordering::SeqCst), 0);
        assert_eq!((player.stats.requested, player.stats.started, player.stats.refused), (9, 0, 0));
        assert!(player.audio.is_none() && !player.device_failed);

        // A later tracked request is not cancelled by an earlier stop_all. The old API
        // does not register for pending cancellation and retains its original FIFO path.
        let (_, later) = engine.play_pcm_tracked(&clip, opts).unwrap();
        let Cmd::Pcm { permit, .. } = rx.recv().unwrap() else { panic!("PCM expected") };
        assert!(permit.tracking.as_ref().unwrap().claim());
        drop(permit);
        assert!(later.is_finished() && !later.was_started());
        engine.play_pcm(&clip, opts).unwrap();
        engine.stop_all();
        let Cmd::Pcm { permit, .. } = rx.recv().unwrap() else { panic!("PCM expected") };
        assert!(permit.tracking.is_none());
    }

    #[test]
    fn tracked_pcm_failure_paths_finish_without_admission_or_retained_history() {
        let clip = PcmClip::mono(24_000, &[0.25; 240]).unwrap();
        let opts = PcmOptions::new(Category::UserMusic);
        for (audible, max_voices, failed) in [(true, 0, false), (true, 64, true), (false, 64, false)] {
            let (engine, mut player, rx) = queued_engine(Settings { audible, max_voices, ..Settings::default() });
            player.device_failed = failed;
            let mut observers = Vec::new();
            for _ in 0..MAX_PCM_VOICES * 4 {
                let (id, observer) = engine.play_pcm_tracked(&clip, opts).unwrap();
                assert!(!observer.is_finished() && !observer.was_started());
                assert_eq!(engine.shared.pcm_pending.lock().unwrap().len(), usize::from(audible));
                player.handle(rx.recv().unwrap());
                assert_eq!(engine.take_finished(), [id]);
                assert!(observer.is_finished() && !observer.was_started());
                assert!(engine.shared.pcm_pending.lock().unwrap().is_empty());
                assert_eq!(engine.shared.pcm_slots.load(Ordering::SeqCst), 0);
                player.handle(Cmd::Stop { id, fade: true });
                player.handle(Cmd::StopAll);
                player.update();
                assert!(engine.take_finished().is_empty());
                assert_eq!(Arc::strong_count(&clip.0), 1);
                observers.push(observer);
            }
            assert!(observers.iter().all(|o| o.is_finished() && !o.was_started()));
            assert!(player.audio.is_none());
            assert_eq!(player.stats.started, 0);
            let mut invalid = opts;
            invalid.volume = f32::NAN;
            let next = engine.shared.next.load(Ordering::Relaxed);
            assert!(matches!(engine.play_pcm_tracked(&clip, invalid), Err(PcmError::Options)));
            assert_eq!(engine.shared.next.load(Ordering::Relaxed), next);
        }
    }

    #[test]
    fn tracked_pcm_queue_teardown_and_observers_retain_no_engine_or_clip() {
        let (engine, player, rx) = queued_engine(Settings::default());
        let clip = PcmClip::mono(24_000, &[0.25]).unwrap();
        let weak_clip = Arc::downgrade(&clip.0);
        let weak_shared = Arc::downgrade(&engine.shared);
        let weak_guard = Arc::downgrade(&engine._guard);
        let pending = engine.shared.pcm_pending.clone();
        let opts = PcmOptions::new(Category::UserMusic);
        let (id, observer) = engine.play_pcm_tracked(&clip, opts).unwrap();
        drop(clip);
        drop(engine); // observer must not retain the last Engine guard
        assert!(weak_guard.upgrade().is_none());
        let cmd = rx.recv().unwrap();
        assert!(matches!(cmd, Cmd::Pcm { .. }));
        assert!(!observer.is_finished());
        drop(cmd);
        assert!(observer.is_finished() && !observer.was_started());
        assert!(pending.lock().unwrap().is_empty());
        assert!(weak_clip.upgrade().is_none());
        assert!(matches!(rx.recv().unwrap(), Cmd::Quit));
        assert!(!player.shared.live.lock().unwrap().is_empty());
        drop(rx);
        drop(player);
        assert!(weak_shared.upgrade().is_none());
        assert!(observer.clone().is_finished() && !observer.was_started(), "{id:?}");

        let (engine, player, rx) = queued_engine(Settings::default());
        drop(player);
        drop(rx);
        let clip = PcmClip::mono(24_000, &[0.25]).unwrap();
        assert!(matches!(engine.play_pcm_tracked(&clip, opts), Err(PcmError::EngineStopped)));
        assert!(engine.shared.pcm_pending.lock().unwrap().is_empty());
        assert!(engine.shared.live.lock().unwrap().is_empty());
        assert_eq!(engine.shared.pcm_slots.load(Ordering::SeqCst), 0);
        assert_eq!(Arc::strong_count(&clip.0), 1);
    }

    struct CaptureBackend(Option<kira::backend::Renderer>);

    impl kira::backend::Backend for CaptureBackend {
        type Settings = ();
        type Error = ();

        fn setup(_: (), _: usize) -> Result<(Self, u32), ()> {
            Ok((Self(None), 24_000))
        }

        fn start(&mut self, renderer: kira::backend::Renderer) -> Result<(), ()> {
            self.0 = Some(renderer);
            Ok(())
        }
    }

    impl Audio<CaptureBackend> {
        fn capture(&mut self, frames: usize) -> Vec<f32> {
            let mut out = vec![0.0; frames * 2];
            let renderer = self.manager.backend_mut().0.as_mut().unwrap();
            renderer.on_start_processing();
            renderer.process(&mut out, 2);
            out
        }
    }

    fn capture_audio() -> Audio<CaptureBackend> {
        let mut manager = AudioManager::<CaptureBackend>::new(AudioManagerSettings::default()).unwrap();
        let listener = manager.add_listener(Vec3::ZERO, Quat::IDENTITY).unwrap();
        let music = manager.add_sub_track(TrackBuilder::new()).unwrap();
        let user_music = manager.add_sub_track(TrackBuilder::new()).unwrap();
        let sfx = manager.add_sub_track(TrackBuilder::new()).unwrap();
        let ambience = manager.add_sub_track(TrackBuilder::new()).unwrap();
        let voice = manager.add_sub_track(TrackBuilder::new()).unwrap();
        let ui = manager.add_sub_track(TrackBuilder::new()).unwrap();
        let mut send = SendTrackBuilder::new();
        let reverb = send.add_effect(ReverbBuilder);
        let reverb_send = manager.add_send_track(send).unwrap();
        Audio { manager, listener, music, user_music, sfx, ambience, voice, ui, reverb_send, reverb,
            buses: HashMap::new(), muffles: HashMap::new() }
    }

    fn channel_mean(frames: &[f32], channel: usize) -> f32 {
        frames[frames.len() / 2..].chunks_exact(2).map(|f| f[channel].abs()).sum::<f32>() / (frames.len() / 4) as f32
    }

    fn housing_voice(distance: f32) -> ActiveVoice {
        let event = Library::global().event("sounds/x2eventsounds:housing:housing_01").unwrap();
        let values = initial_values(event.event(), Some(distance), &[]);
        let planned = plan(&event, &values, &mut PickState::default(), &mut Rng::new(3)).remove(0);
        let bank = Library::global().bank(&event.project, planned.bank).unwrap();
        let pcm = Library::global().decode(&bank, planned.index as usize).unwrap();
        assert!(pcm.rms() > 0.001, "the original housing wave is audible data");
        let mut part = Part::new(0, 0, WET_OFF);
        part.sample = Some(Arc::new(Sample::from_pcm(&pcm)));
        part.planned = Some(planned);
        ActiveVoice {
            id: Voice(1), playback_slot: 0, category: Category::Sfx, leaf: "particle".into(), mood: 1.0,
            at: Some(Vec3::X * distance), moved: false, velocity: Vec3::ZERO, moved_at: Instant::now(),
            volume: 1.0, values, distance_params: distance_parameters(event.event()), dirty: false,
            persistent: false, parts: vec![part], requested: Instant::now(), last_tick: Instant::now(),
            started: true, stopping: false, event,
        }
    }

    /// Optional PCM evidence from the actual manually-clocked mixer, without an OS device.
    fn save_pan_capture(name: &str, frames: &[f32]) {
        save_capture("AA_AUDIO_PAN_EVIDENCE", name, frames);
    }

    fn save_capture(variable: &str, name: &str, frames: &[f32]) {
        let Some(dir) = std::env::var_os(variable) else { return };
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        let size = (frames.len() * 2) as u32;
        let mut wav = Vec::with_capacity(44 + size as usize);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + size).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&24_000u32.to_le_bytes());
        wav.extend_from_slice(&96_000u32.to_le_bytes());
        wav.extend_from_slice(&4u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&size.to_le_bytes());
        for v in frames {
            wav.extend_from_slice(&((v.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
        }
        std::fs::write(dir.join(format!("{name}.wav")), wav).unwrap();
    }

    fn highpass_voice(path: &str) -> ActiveVoice {
        let event = Library::global().event(path).unwrap();
        let at = event.event().is_3d().then_some(Vec3::X * 3.0);
        let daylight = if path.contains("world2_night") { 0.0 } else { 1.0 };
        let values = initial_values(event.event(), at.map(|v| v.length()), &[("daylight".into(), daylight)]);
        let planned = plan(&event, &values, &mut PickState::default(), &mut Rng::new(3)).into_iter().find(|p| p.layer == 0).unwrap();
        let bank = Library::global().bank(&event.project, planned.bank).unwrap();
        let pcm = Library::global().decode(&bank, planned.index as usize).unwrap();
        assert!(pcm.rms() > 0.001, "original high-pass asset contains audible data");
        let mut part = Part::new(0, planned.instance, WET_OFF);
        part.sample = Some(Arc::new(Sample::from_pcm(&pcm)));
        part.planned = Some(planned);
        ActiveVoice {
            id: Voice(1), playback_slot: 0, category: Category::Sfx, leaf: "particle".into(), mood: 1.0,
            at, moved: false, velocity: Vec3::ZERO, moved_at: Instant::now(), volume: 1.0, values,
            distance_params: distance_parameters(event.event()), dirty: false, persistent: false,
            parts: vec![part], requested: Instant::now(), last_tick: Instant::now(), started: true, stopping: false, event,
        }
    }

    /// Independent same-mixer control: original sample/rate/gain/LPF/pan, explicit native cutoff.
    /// Reverb and the event's other layers are excluded to isolate the affected layer.
    fn highpass_control(audio: &mut Audio<CaptureBackend>, voice: &ActiveVoice, hz: Option<f64>) {
        use kira::effect::filter::{FilterBuilder, FilterMode};
        let sample = voice.parts[0].sample.as_ref().unwrap();
        let planned = voice.parts[0].planned.as_ref().unwrap();
        let mut data = StaticSoundData { sample_rate: sample.rate, frames: sample.frames.clone(),
            settings: StaticSoundSettings::default(), slice: None }
            .volume(db(voice.target_gain(&voice.parts[0], Vec3::ZERO)))
            .playback_rate(f64::from(part_rate(voice, &voice.parts[0], (Vec3::ZERO, Vec3::ZERO))));
        if planned.looping { data = data.loop_region(loop_region(sample.loop_points)); }
        if planned.fade_in_ms > 0 { data = data.fade_in_tween(tween_ms(planned.fade_in_ms)); }
        let highpass = hz.map(|hz| FilterBuilder::new().mode(FilterMode::HighPass).cutoff(hz).resonance(1.0 / 1.9));
        let filtered = layer_filtered(voice.event.event(), 0);
        if let Some(at) = voice.at {
            let mut builder = SpatialTrackBuilder::new().attenuation_function(None)
                .spatialization_strength(layer_pan(voice.event.event(), &voice.values, 0)).persist_until_sounds_finish(true);
            if let Some(hp) = highpass { builder.add_effect(hp); }
            if filtered { builder.add_effect(MuffleBuilder).set(layer_cutoff(voice.event.event(), &voice.values, 0)); }
            let mut track = audio.sfx.add_spatial_sub_track(audio.listener.id(), at, builder).unwrap();
            track.play(data).unwrap();
        } else {
            let mut builder = TrackBuilder::new().persist_until_sounds_finish(true);
            if let Some(hp) = highpass { builder.add_effect(hp); }
            if filtered { builder.add_effect(MuffleBuilder).set(layer_cutoff(voice.event.event(), &voice.values, 0)); }
            let mut track = audio.sfx.add_sub_track(builder).unwrap();
            track.play(data).unwrap();
        }
    }

    #[test]
    #[ignore = "needs the game pak; captures all five original high-pass layers without a device"]
    fn original_highpass_assets_match_native_cutoff_mixer_controls() {
        let paths = ["sounds/x2ambience:login_stage:countryside02", "sounds/x2ambience:login_stage:mar03",
            "sounds/x2ambience:world2_day:countryside02", "sounds/x2ambience:world2_night:wind_night1",
            "sounds/x2weapons:x2weapons:shield_metal"];
        let mut errors = Vec::new();
        for (path, row) in paths.into_iter().zip(include_str!("../tests/native_highpass_cases.tsv").lines().filter(|r| !r.starts_with('#'))) {
            let fields: Vec<_> = row.split('\t').collect();
            let hz = fields[2].parse().unwrap();
            let mut voice = highpass_voice(path);
            let mut audio = capture_audio();
            let mut control = capture_audio();
            let mut legacy = capture_audio();
            highpass_control(&mut control, &voice, Some(hz));
            highpass_control(&mut legacy, &voice, None);
            begin_part(&mut audio, &mut voice, 0, (Vec3::ZERO, Vec3::ZERO), &Volumes::default());
            let (actual, control, legacy) = (audio.capture(14_400), control.capture(14_400), legacy.capture(14_400));
            let relative = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(a, b)| (a - b).abs()).sum::<f32>() / b.iter().map(|v| v.abs()).sum::<f32>();
            let error = relative(&actual, &control);
            let effect = relative(&legacy, &control);
            eprintln!("original highpass {}: native_hz={hz} mean_abs_LR actual=({}, {}) control=({}, {}) legacy=({}, {}) relative_error={error} legacy_error={effect}",
                fields[0], channel_mean(&actual, 0), channel_mean(&actual, 1), channel_mean(&control, 0), channel_mean(&control, 1), channel_mean(&legacy, 0), channel_mean(&legacy, 1));
            assert!(control.iter().all(|v| v.is_finite()) && channel_mean(&control, 0) > 1e-6 && effect > 0.001);
            for (suffix, out) in [("actual", &actual), ("native-cutoff-control", &control), ("legacy-control", &legacy)] {
                save_capture("AA_AUDIO_HIGHPASS_EVIDENCE", &format!("{}-{suffix}", fields[0]), out);
            }
            errors.push(error);
        }
        assert_eq!(errors.len(), 5);
        assert!(errors.iter().all(|e| *e < 0.0002), "native-cutoff mixer controls: {errors:?}");
    }

    #[test]
    fn highpass_response_matches_original_core_coefficients() {
        for row in include_str!("../tests/native_highpass_cases.tsv").lines().filter(|r| !r.starts_with('#')) {
            let f: Vec<_> = row.split('\t').collect();
            let hz: f64 = f[2].parse().unwrap();
            let c: Vec<f64> = f[3..].iter().map(|v| v.parse().unwrap()).collect();
            for multiplier in [0.25, 1.0, 4.0] {
                let frequency = hz * multiplier;
                let w = std::f64::consts::TAU * frequency / 24_000.0;
                let power = |a: f64, b: f64, c: f64| (a + b * w.cos() + c * (2.0 * w).cos()).powi(2)
                    + (b * w.sin() + c * (2.0 * w).sin()).powi(2);
                let native = (power(c[0], c[1], c[2]) / power(1.0, c[3], c[4])).sqrt();
                let mut audio = capture_audio();
                let mut builder = TrackBuilder::new();
                builder.add_effect(FilterBuilder::new().mode(FilterMode::HighPass).cutoff(hz).resonance(1.0 / 1.9));
                let mut track = audio.sfx.add_sub_track(builder).unwrap();
                let frames = (0..24_000).map(|i| kira::Frame::from_mono((0.125 * (w * i as f64).sin()) as f32)).collect::<Vec<_>>();
                let input_power = frames[12_000..].iter().map(|v| f64::from(v.left).powi(2)).sum::<f64>();
                let data = StaticSoundData { sample_rate: 24_000, frames: frames.into(), settings: StaticSoundSettings::default(), slice: None };
                let _sound = track.play(data).unwrap();
                let output = audio.capture(24_000);
                let output_power = output[24_000..].as_chunks::<2>().0.iter().map(|v| f64::from(v[0]).powi(2)).sum::<f64>();
                let measured = (output_power / input_power).sqrt();
                assert!((measured / native - 1.0).abs() < 0.005, "{} frequency={frequency} native={native} Kira={measured}", f[0]);
            }
        }
    }

    /// Clone only test data; the original project and the pak remain unchanged.
    fn highpass_fixture() -> ActiveVoice {
        use crate::fev::EnvPoint;
        let mut voice = highpass_voice("sounds/x2ambience:login_stage:mar03");
        let mut fev = voice.event.project.fev.clone();
        let Body::Complex { layers, .. } = &mut fev.events[voice.event.index].body else { unreachable!() };
        layers[0].envelopes[0].points = vec![EnvPoint { x: 0.0, y: 0.25, curve: 2 }, EnvPoint { x: 1.0, y: 0.75, curve: 2 }];
        voice.event.project = Arc::new(Project { name: voice.event.project.name.clone(), dir: voice.event.project.dir.clone(), fev });
        voice.values[0] = 0.0;
        voice
    }

    #[test]
    #[ignore = "needs the game pak; dynamic cutoff/bypass are explicit test-fixture curves"]
    fn highpass_runtime_controls_bypass_mute_clock_and_retirement() {
        let mut audio = capture_audio();
        let mut control_audio = capture_audio();
        let mut voice = highpass_fixture();
        let mut control_voice = highpass_fixture();
        begin_part(&mut audio, &mut voice, 0, (Vec3::ZERO, Vec3::ZERO), &Volumes::default());
        begin_part(&mut control_audio, &mut control_voice, 0, (Vec3::ZERO, Vec3::ZERO), &Volumes::default());
        let (_engine, mut player, _rx) = queued_engine(Settings::default());
        let (_control_engine, mut reference, _reference_rx) = queued_engine(Settings::default());
        let id = voice.id;
        player.voices.insert(id, voice);
        reference.voices.insert(id, control_voice);
        for (step, hz, enabled) in [(0, 68.48661, true), (1, 3212.3066, true), (2, 3212.3066, true),
            (3, 3212.3066, false), (4, 469.04156, false), (5, 469.04156, true),
            (6, 469.04156, true), (7, 469.04156, true)]
        {
            for p in [&mut player, &mut reference] {
                match step {
                    1 | 4 => p.handle(Cmd::Param { id, name: "daylight".into(), value: if step == 1 { 1.0 } else { 0.5 } }),
                    2 => p.handle(Cmd::Listener { position: Vec3::X * 50.0, rotation: Quat::IDENTITY }),
                    3 | 5 => {
                        let index = p.voices[&id].event.index;
                        let project = Arc::get_mut(&mut p.voices.get_mut(&id).unwrap().event.project).unwrap();
                        let Body::Complex { layers, .. } = &mut project.fev.events[index].body else { unreachable!() };
                        if step == 3 { layers[0].envelopes[0].flags |= 1; } else { layers[0].envelopes[0].flags &= !1; }
                    }
                    _ => {}
                }
            }
            if step == 6 { player.handle(Cmd::VoiceVolume { id, volume: 0.0 }); }
            if step == 7 { player.handle(Cmd::VoiceVolume { id, volume: 1.0 }); }
            player.update();
            reference.update();
            let actual_state = player.voices[&id].parts[0].highpass_state.unwrap();
            assert!((actual_state.cutoff / hz - 1.0).abs() < 1e-6 && actual_state.enabled == enabled);
            let filter = reference.voices.get_mut(&id).unwrap().parts[0].highpass.as_mut().unwrap();
            filter.set_cutoff(f64::from(hz), tween_ms(0));
            filter.set_mix(if enabled { kira::Mix::WET } else { kira::Mix::DRY }, tween_ms(0));
            let actual = audio.capture(4_800);
            let expected = control_audio.capture(4_800);
            if step == 6 {
                assert!(channel_mean(&actual, 0) < 1e-7 && channel_mean(&expected, 0) > 1e-5);
            } else {
                // The existing voice gain has a 30 ms anti-click ramp. Compare the resumed
                // steady tail to the continuously audible reference to verify clock continuity.
                let from = if step == 7 { actual.len() / 2 } else { 0 };
                let error = actual[from..].iter().zip(&expected[from..]).map(|(a, b)| (a - b).abs()).sum::<f32>()
                    / expected[from..].iter().map(|v| v.abs()).sum::<f32>();
                eprintln!("highpass dynamic step={step} native_hz={hz} enabled={enabled} relative_error={error}");
                assert!(error < 0.0002, "step {step}: {error}");
            }
            assert_eq!(player.voices[&id].parts[0].handle.as_ref().unwrap().state(), PlaybackState::Playing);
        }
        player.handle(Cmd::Stop { id, fade: false });
        audio.capture(4_800);
        player.update();
        assert!(!player.voices.contains_key(&id));
        assert_eq!(*player.shared.finished.lock().unwrap(), [id]);
        assert!(audio.capture(4_800).iter().all(|v| v.abs() < 1e-7));
    }

    #[test]
    #[ignore = "needs the game pak; modified copies prove unsupported modes are skipped"]
    fn highpass_unsupported_mapping_and_topologies_are_skipped() {
        let mut bytes = aa_pak::Pak::global().read("sounds/x2ambience/x2ambience.fev").unwrap();
        let needle = b"FMOD Highpass\0";
        let at = bytes.windows(needle.len()).position(|w| w == needle).unwrap() + needle.len();
        let points = u32::from_le_bytes(bytes[at + 12..at + 16].try_into().unwrap()) as usize;
        let mapping = at + 20 + 4 * points;
        assert_eq!(u32::from_le_bytes(bytes[mapping..mapping + 4].try_into().unwrap()), 1);
        bytes[mapping..mapping + 4].copy_from_slice(&0u32.to_le_bytes());
        let fev = crate::fev::Fev::parse(&bytes).unwrap();
        let index = fev.event_index("login_stage/countryside02").unwrap();
        let event = EventRef { index, project: Arc::new(Project { name: fev.name.clone(), dir: String::new(), fev }) };
        assert_eq!(layer_highpass(&event, &[], 0), None);
        for mode in 0..3 {
            let mut voice = highpass_fixture();
            let index = voice.event.index;
            let project = Arc::get_mut(&mut voice.event.project).unwrap();
            let Body::Complex { layers, .. } = &mut project.fev.events[index].body else { unreachable!() };
            match mode {
                0 => layers[0].envelopes[0].parent = 1,
                1 => layers[0].envelopes[1].points[0].y = 0.5,
                _ => { let duplicate = layers[0].envelopes[0].clone(); layers[0].envelopes.push(duplicate); }
            }
            assert_eq!(layer_highpass(&voice.event, &voice.values, 0), None);
            let mut audio = capture_audio();
            begin_part(&mut audio, &mut voice, 0, (Vec3::ZERO, Vec3::ZERO), &Volumes::default());
            assert!(voice.parts[0].highpass.is_none());
        }
    }

    #[test]
    #[ignore = "needs the game pak; captures the original housing sample without a device"]
    fn housing_pan_mixer_matches_original_near_ramp_and_far_controls() {
        let capture = |distance, control| {
            let mut audio = capture_audio();
            let mut voice = housing_voice(distance);
            begin_part(&mut audio, &mut voice, 0, (Vec3::ZERO, Vec3::ZERO), &Volumes::default());
            assert_eq!(voice.parts[0].state, PartState::Playing);
            if let Some(pan) = control {
                voice.parts[0].track.as_mut().unwrap().set_spatialization_strength(pan, tween_ms(0));
            }
            audio.capture(14_400)
        };
        let mut errors = Vec::new();
        for (name, distance, native_pan) in [("near", 3.5, 0.0), ("ramp", 9.6488, 0.323077), ("far", 17.5, 0.646154)] {
            let actual = capture(distance, None);
            let control = capture(distance, Some(native_pan));
            let old = capture(distance, Some(1.0));
            let error = actual.iter().zip(&control).map(|(a, b)| (a - b).abs()).sum::<f32>()
                / control.iter().map(|v| v.abs()).sum::<f32>();
            let lr = |out: &[f32]| (channel_mean(out, 0), channel_mean(out, 1));
            eprintln!("housing pan {name}: distance={distance} native_pan={native_pan} mean_abs_LR actual={:?} native_control={:?} legacy_control={:?} relative_error={error}", lr(&actual), lr(&control), lr(&old));
            save_pan_capture(&format!("housing-{name}"), &actual);
            save_pan_capture(&format!("housing-{name}-native-coefficient-control"), &control);
            save_pan_capture(&format!("housing-{name}-legacy-control"), &old);
            assert!(channel_mean(&control, 1) > 0.001, "real sample produced nonzero mixer output");
            errors.push(error);
        }
        assert!(errors.iter().all(|e| *e < 0.0001), "original pan coefficient controls: {errors:?}");
    }

    #[test]
    #[ignore = "needs the game pak; captures original housing during live controls without a device"]
    fn housing_pan_mixer_updates_on_source_listener_and_parameter_commands() {
        let mut audio = capture_audio();
        let mut control_audio = capture_audio();
        let mut voice = housing_voice(3.5);
        let mut control_voice = housing_voice(3.5);
        begin_part(&mut audio, &mut voice, 0, (Vec3::ZERO, Vec3::ZERO), &Volumes::default());
        begin_part(&mut control_audio, &mut control_voice, 0, (Vec3::ZERO, Vec3::ZERO), &Volumes::default());
        let (_engine, mut player, _rx) = queued_engine(Settings::default());
        let (_control_engine, mut control_player, _control_rx) = queued_engine(Settings::default());
        let id = voice.id;
        player.voices.insert(id, voice);
        control_player.voices.insert(id, control_voice);
        for (step, name, native_pan) in [(0, "near", 0.0), (1, "source_far", 0.646154),
            (2, "listener_near", 0.0), (3, "parameter_far", 0.646154),
            (4, "source_left", 0.646154), (5, "parameter_near", 0.0)]
        {
            for p in [&mut player, &mut control_player] {
                match step {
                    1 => p.handle(Cmd::Move { id, at: Vec3::X * 17.5 }),
                    2 => p.handle(Cmd::Listener { position: Vec3::X * 14.0, rotation: Quat::IDENTITY }),
                    3 => {
                        // Isolate manual control from the automatic distance binding, retaining
                        // the original parameter range, curve and decoded sample.
                        p.voices.get_mut(&id).unwrap().distance_params.clear();
                        p.handle(Cmd::Param { id, name: "distance".into(), value: 17.5 });
                    }
                    4 => p.handle(Cmd::Move { id, at: Vec3::X * 10.5 }),
                    5 => p.handle(Cmd::Param { id, name: "distance".into(), value: 3.5 }),
                    _ => {}
                }
                p.update();
            }
            if step == 2 {
                // Players hold the default backend; mirror their listener command to captures.
                audio.listener.set_position(Vec3::X * 14.0, tween_ms(0));
                control_audio.listener.set_position(Vec3::X * 14.0, tween_ms(0));
            }
            assert!((player.voices[&id].parts[0].pan - native_pan).abs() < 1e-6, "{name}");
            control_player.voices.get_mut(&id).unwrap().parts[0].track.as_mut().unwrap()
                .set_spatialization_strength(native_pan, tween_ms(30));
            let actual = audio.capture(4_800);
            let control = control_audio.capture(4_800);
            let error = actual.iter().zip(&control).map(|(a, b)| (a - b).abs()).sum::<f32>()
                / control.iter().map(|v| v.abs()).sum::<f32>();
            eprintln!("housing runtime {name}: native_pan={native_pan} mean_abs_LR=({}, {}) relative_error={error}",
                channel_mean(&actual, 0), channel_mean(&actual, 1));
            assert!(channel_mean(&control, 1) > 0.001 && error < 0.0001, "{name}: error={error}");
        }
        player.handle(Cmd::Stop { id, fade: false });
        audio.capture(2_400);
        player.update();
        assert!(!player.voices.contains_key(&id), "stopped event still retires normally");
    }

    /// Route an actual queued request into a manually-clocked mixer using production admission,
    /// voice construction and completion polling, without opening the OS output device.
    fn capture_request(player: &mut Player, rx: &Receiver<Cmd>, audio: &mut Audio<CaptureBackend>) {
        let Cmd::Pcm { id, clip, opts, permit } = rx.recv().unwrap() else { panic!("PCM expected") };
        assert!(permit.tracking.as_ref().unwrap().claim());
        assert!(player.admit_voice(opts.priority));
        let voice = PcmVoice::begin(audio, &clip, opts, player.listener.0, permit).unwrap();
        player.pcm_voices.insert(id, voice);
    }

    /// Isolate the original CELT layer, retaining its event, curves and wave. The fixture replaces
    /// OS output/admission setup with CaptureBackend; prepare/worker/cache/controls/retirement run
    /// through the production player, with no injected PCM or altered FEV definitions.
    fn queued_celt_layer(engine: &Engine, player: &mut Player, rx: &Receiver<Cmd>) -> ActiveVoice {
        let path = "sounds/x2weapons_sword:sword:sword_wind";
        let id = engine.play_with(path, Some(Vec3::X), PlayOptions {
            params: vec![("critical".into(), 1.0), ("twohand".into(), 1.0), ("miss".into(), 0.0)],
            ..PlayOptions::default()
        });
        let Cmd::Play { id: queued, path, at, opts } = rx.recv().unwrap() else { panic!("event request expected") };
        assert_eq!(queued, id);
        let event = Library::global().event(&path).unwrap();
        let values = initial_values(event.event(), Some(1.0), &opts.params);
        let planned = plan(&event, &values, &mut PickState::default(), &mut Rng::new(3))
            .into_iter().find(|p| p.layer == 6).unwrap();
        assert_eq!((planned.bank, planned.index, planned.looping), (54, 0, false));
        let mut part = Part::new(6, 0, WET_OFF);
        player.prepare(id, &event, &mut part, planned);
        ActiveVoice {
            id, playback_slot: 0, category: Category::Sfx, leaf: "weapon".into(), mood: 1.0,
            at, moved: false, velocity: Vec3::ZERO, moved_at: Instant::now(), volume: 1.0, values,
            distance_params: distance_parameters(event.event()), dirty: false, persistent: false,
            parts: vec![part], requested: Instant::now(), last_tick: Instant::now(), started: false,
            stopping: false, event,
        }
    }

    #[test]
    #[ignore = "needs the game pak; captures the original CELT layer without an OS device"]
    fn original_celt_worker_cache_gates_and_completion() {
        let (engine, mut player, rx) = queued_engine(Settings::default());
        let mut audio = capture_audio();
        let voice = queued_celt_layer(&engine, &mut player, &rx);
        let first = voice.id;
        assert!(voice.parts[0].sample.is_none());
        let key = voice.parts[0].key.clone().unwrap();
        player.voices.insert(first, voice);
        let decoded = rx.recv_timeout(Duration::from_secs(5)).expect("the actual CELT decode worker replies");
        assert!(matches!(decoded, Cmd::Decoded { .. }));
        player.handle(decoded);
        assert_eq!((player.stats.decoded_sounds, player.stats.decode_failures), (1, 0));
        let cached = player.cache.get(&key).unwrap();
        assert_eq!((cached.rate, cached.frames.len(), cached.loop_points), (44_100, 116_224, Some((0, 116_003))));
        assert_eq!(cached.bytes(), 929_792);
        let voice = player.voices.get_mut(&first).unwrap();
        begin_part(&mut audio, voice, 0, (Vec3::ZERO, Vec3::ZERO), &Volumes::default());
        let audible = audio.capture(14_400);
        assert!(channel_mean(&audible, 0) > 0.001);
        save_capture("AA_AUDIO_CELT_EVIDENCE", "event-first", &audible);
        audio.capture(72_000); // past the original one-shot's end, including randomized pitch
        player.update();
        assert!(!engine.is_playing(first) && !player.voices.contains_key(&first));
        assert_eq!(engine.take_finished(), [first]);
        assert!(engine.take_finished().is_empty());

        let hits = player.cache.hits;
        let mut voice = queued_celt_layer(&engine, &mut player, &rx);
        let second = voice.id;
        assert!(Arc::ptr_eq(voice.parts[0].sample.as_ref().unwrap(), &cached));
        assert_eq!(player.cache.hits, hits + 1);
        assert!(player.pending.is_empty() && rx.try_recv().is_err());
        player.try_start(&mut voice);
        begin_part(&mut audio, &mut voice, 0, (Vec3::ZERO, Vec3::ZERO), &Volumes::default());
        player.voices.insert(second, voice);
        let actual = audio.capture(9_600);
        assert!(channel_mean(&actual, 0) > 0.001);
        save_capture("AA_AUDIO_CELT_EVIDENCE", "event-cached", &actual);
        for (name, value, audible) in [("twohand", 0.0, false), ("twohand", 1.0, true),
            ("miss", 1.0, false), ("miss", 0.0, true)]
        {
            player.handle(Cmd::Param { id: second, name: name.into(), value });
            player.update();
            let actual = audio.capture(4_800); // tail is past the existing 30 ms gain tween
            let mean = channel_mean(&actual, 0);
            assert_eq!(mean > 1e-6, audible, "{name}={value} mean={mean}");
            save_capture("AA_AUDIO_CELT_EVIDENCE", &format!("event-{name}-{value}"), &actual);
        }
        player.handle(Cmd::Param { id: second, name: "critical".into(), value: 0.0 });
        player.update();
        audio.capture(2_400);
        player.update();
        assert!(!engine.is_playing(second) && !player.voices.contains_key(&second));
        assert_eq!(engine.take_finished(), [second]);
        assert!(engine.take_finished().is_empty());
        assert_eq!((player.stats.decoded_sounds, player.stats.decode_failures), (1, 0));
        assert!(player.audio.is_none() && player.stats.device_error.is_none());
        eprintln!("original CELT worker/cache/capture: one decode, same cached Arc on repeat, 929792 cache bytes; twohand/miss live gains and critical retirement; both requests completed once without a device");
    }

    #[test]
    fn tracked_pcm_capture_admission_tiny_completion_and_zero_gain_clock() {
        // A single frame can end before the caller first reads its already-installed observer.
        // Category and master zero are still real Kira admission/sample-clock completion.
        for (samples, master, user_music) in [(1, 1.0, 1.0), (2_400, 1.0, 0.0), (2_400, 0.0, 1.0)] {
            let (engine, mut player, rx) = queued_engine(Settings::default());
            let mut audio = capture_audio();
            audio.apply(&Volumes { master, user_music, ..Volumes::default() });
            audio.capture(2_400); // settle Kira's fader tween before adding the request
            let clip = PcmClip::mono(24_000, &vec![0.25; samples]).unwrap();
            let (id, observer) = engine.play_pcm_tracked(&clip, PcmOptions::new(Category::UserMusic)).unwrap();
            capture_request(&mut player, &rx, &mut audio);
            assert!(engine.shared.pcm_pending.lock().unwrap().is_empty());
            if samples > 1 {
                assert!(observer.was_started() && !observer.is_finished(), "accepted before any mixer callback");
                let out = audio.capture(120);
                assert!(out.iter().all(|v| v.abs() < 0.00001));
                player.update();
                assert!(!observer.is_finished(), "a zero gain does not skip the sample clock");
            }
            audio.capture(4_800);
            player.update();
            assert_eq!(engine.take_finished(), [id]);
            assert!(observer.is_finished() && observer.was_started());
            let clone = observer.clone();
            player.update();
            assert!(engine.take_finished().is_empty());
            assert!(clone.is_finished() && clone.was_started());
            assert_eq!(engine.shared.pcm_slots.load(Ordering::SeqCst), 0);
        }
    }

    #[test]
    fn tracked_pcm_capture_stop_and_steal_finish_after_mixer_ramp() {
        for stop_all in [false, true] {
            let (engine, mut player, rx) = queued_engine(Settings::default());
            let mut audio = capture_audio();
            let clip = PcmClip::mono(24_000, &[0.25; 24_000]).unwrap();
            let (id, observer) = engine.play_pcm_tracked(&clip, PcmOptions::new(Category::UserMusic)).unwrap();
            capture_request(&mut player, &rx, &mut audio);
            // Accepted but no callback has run. This is beyond the pending-cancel boundary.
            if stop_all { engine.stop_all() } else { engine.stop(id, false) }
            player.handle(rx.recv().unwrap());
            player.update();
            assert!(observer.was_started() && !observer.is_finished());
            audio.capture(2_400);
            player.update();
            assert!(observer.is_finished() && observer.was_started());
            assert_eq!(engine.take_finished(), [id]);
            player.handle(Cmd::StopAll);
            player.update();
            assert!(engine.take_finished().is_empty());
        }

        let (engine, mut player, rx) = queued_engine(Settings { max_voices: 1, ..Settings::default() });
        let mut audio = capture_audio();
        let clip = PcmClip::mono(24_000, &[0.25; 24_000]).unwrap();
        let opts = PcmOptions::new(Category::UserMusic);
        let (first, first_observer) = engine.play_pcm_tracked(&clip, opts).unwrap();
        capture_request(&mut player, &rx, &mut audio);
        let (second, second_observer) = engine.play_pcm_tracked(&clip, opts).unwrap();
        capture_request(&mut player, &rx, &mut audio);
        assert_eq!(player.stats.stolen, 1);
        assert!(!first_observer.is_finished());
        audio.capture(2_400);
        player.update();
        assert_eq!(engine.take_finished(), [first]);
        assert!(first_observer.is_finished() && first_observer.was_started());
        assert!(second_observer.was_started() && !second_observer.is_finished());
        engine.stop_all();
        player.handle(rx.recv().unwrap());
        audio.capture(2_400);
        player.update();
        assert_eq!(engine.take_finished(), [second]);
        assert!(second_observer.is_finished() && second_observer.was_started());
        assert_eq!(engine.shared.pcm_slots.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn tracked_pcm_capture_mixer_refusal_and_active_teardown_are_terminal() {
        let (engine, mut player, rx) = queued_engine(Settings::default());
        let mut audio = capture_audio();
        audio.user_music = audio.manager.add_sub_track(TrackBuilder::new().sound_capacity(1)).unwrap();
        let clip = PcmClip::mono(24_000, &[0.25; 240]).unwrap();
        let opts = PcmOptions::new(Category::UserMusic);
        let occupant = begin_pcm(&mut audio, &clip, opts, Vec3::ZERO).unwrap();
        let (id, observer) = engine.play_pcm_tracked(&clip, opts).unwrap();
        let Cmd::Pcm { clip: submitted, opts, permit, .. } = rx.recv().unwrap() else { panic!("PCM expected") };
        assert!(permit.tracking.as_ref().unwrap().claim());
        assert!(PcmVoice::begin(&mut audio, &submitted, opts, Vec3::ZERO, permit).is_err());
        drop(submitted);
        player.finish(id);
        audio.capture(4_800);
        player.update();
        assert!(observer.is_finished() && !observer.was_started(), "Kira refused its sound capacity");
        assert_eq!(engine.take_finished(), [id]);
        assert!(engine.shared.pcm_pending.lock().unwrap().is_empty());
        assert_eq!(engine.shared.pcm_slots.load(Ordering::SeqCst), 0);
        drop(occupant);
        drop(audio);

        let weak_clip = Arc::downgrade(&clip.0);
        let mut audio = capture_audio();
        let (id, active_observer) = engine.play_pcm_tracked(&clip, opts).unwrap();
        drop(clip);
        capture_request(&mut player, &rx, &mut audio);
        assert!(active_observer.was_started() && !active_observer.is_finished());
        drop(player); // accepted voices terminate even without another callback/update
        assert!(active_observer.is_finished() && active_observer.was_started());
        assert_eq!(engine.take_finished(), [id]);
        assert!(engine.take_finished().is_empty());
        assert_eq!(engine.shared.pcm_slots.load(Ordering::SeqCst), 0);
        drop(audio); // Kira may retain retired frames until the mixer is dropped
        assert!(weak_clip.upgrade().is_none());
        let weak_shared = Arc::downgrade(&engine.shared);
        drop(engine);
        drop(rx);
        assert!(weak_shared.upgrade().is_none());
        assert!(active_observer.is_finished() && active_observer.was_started());
    }

    #[test]
    fn pcm_mixer_uses_runtime_voice_category_and_master_gain() {
        let mut audio = capture_audio();
        let clip = PcmClip::mono(24_000, &vec![0.25; 24_000]).unwrap();
        let mut opts = PcmOptions::new(Category::Music);
        opts.volume = 0.5;
        let (mut handle, track) = begin_pcm(&mut audio, &clip, opts, Vec3::ZERO).unwrap();
        assert!(track.is_none());
        let mut volumes = Volumes { master: 0.5, music: 0.25, ..Volumes::default() };
        audio.apply(&volumes);
        let out = audio.capture(4_800);
        // Centered source: 0.25 sample * 0.5 voice * 0.25 music * 0.5 master.
        let expected = 0.25 * 0.5 * 0.25 * 0.5;
        assert!((channel_mean(&out, 0) - expected).abs() < 0.0001, "{}", channel_mean(&out, 0));
        assert_eq!(channel_mean(&out, 0), channel_mean(&out, 1));
        volumes.sfx = 0.0; // another group's fader does not affect music
        audio.apply(&volumes);
        assert!((channel_mean(&audio.capture(2_400), 0) - expected).abs() < 0.0001);
        handle.set_volume(db(0.25), tween_ms(30));
        assert!((channel_mean(&audio.capture(4_800), 0) - expected * 0.5).abs() < 0.0001);
        volumes.music = 0.0;
        audio.apply(&volumes);
        assert!(channel_mean(&audio.capture(4_800), 0) < 0.00001);
        volumes.music = 1.0;
        volumes.master = 0.0;
        audio.apply(&volumes);
        assert!(channel_mean(&audio.capture(4_800), 0) < 0.00001);
        audio.capture(8_000);
        assert_eq!(handle.state(), PlaybackState::Stopped, "silent PCM still ends naturally");
    }

    #[test]
    fn pcm_user_music_uses_pre_device_goldens_with_once_master_policy() {
        assert_eq!(Volumes::default().user_music, 1.0);
        let clip = PcmClip::mono(24_000, &vec![0.25; 24_000]).unwrap();
        let mut cases = 0;
        for line in include_str!("../tests/native_user_music_cases.tsv").lines().skip(1) {
            let fields: Vec<_> = line.split('\t').collect();
            let values: Vec<f32> = fields[1..].iter().map(|v| v.parse().unwrap()).collect();
            let mut audio = capture_audio();
            let mut opts = PcmOptions::new(Category::UserMusic);
            opts.volume = values[0];
            let (handle, track) = begin_pcm(&mut audio, &clip, opts, Vec3::ZERO).unwrap();
            assert!(track.is_none());
            let volumes = Volumes { user_music: values[1], master: values[2], sfx: values[3], music: values[4], ..Volumes::default() };
            assert_eq!(volumes.get(Category::UserMusic), values[1]);
            audio.apply(&volumes);
            let out = audio.capture(4_800);
            // Preserve the engine's once-master policy. Native downstream FMOD total is unresolved.
            assert!((channel_mean(&out, 0) - 0.25 * values[5]).abs() < 0.0001, "{}", fields[0]);
            assert_eq!(channel_mean(&out, 0), channel_mean(&out, 1));
            audio.capture(24_000);
            assert_eq!(handle.state(), PlaybackState::Stopped, "{}", fields[0]);
            cases += 1;
        }
        assert_eq!(cases, 12);
    }

    #[test]
    fn pcm_user_music_runtime_mute_leaves_other_tracks_and_sample_clock_running() {
        let mut audio = capture_audio();
        let clip = PcmClip::mono(24_000, &vec![0.125; 48_000]).unwrap();
        let (mut score, _) = begin_pcm(&mut audio, &clip, PcmOptions::new(Category::UserMusic), Vec3::ZERO).unwrap();
        let (bgm, _) = begin_pcm(&mut audio, &clip, PcmOptions::new(Category::Music), Vec3::ZERO).unwrap();
        let (sfx, _) = begin_pcm(&mut audio, &clip, PcmOptions::new(Category::Sfx), Vec3::ZERO).unwrap();
        let mut volumes = Volumes { master: 0.5, user_music: 0.25, music: 0.0, sfx: 0.0, ..Volumes::default() };
        audio.apply(&volumes);
        assert!((channel_mean(&audio.capture(4_800), 0) - 0.015625).abs() < 0.0001);
        volumes.user_music = 0.0;
        volumes.music = 1.0;
        volumes.sfx = 1.0;
        audio.apply(&volumes);
        assert!((channel_mean(&audio.capture(4_800), 0) - 0.125).abs() < 0.0001);
        assert_eq!(score.state(), PlaybackState::Playing);
        // Root owns actor/option identity: a zero per-voice factor silences only this score.
        score.set_volume(db(0.0), tween_ms(30));
        volumes.user_music = 1.0;
        audio.apply(&volumes);
        assert!((channel_mean(&audio.capture(4_800), 0) - 0.125).abs() < 0.0001);
        score.set_volume(db(0.5), tween_ms(30));
        assert!((channel_mean(&audio.capture(4_800), 0) - 0.15625).abs() < 0.0001);
        volumes.master = 0.0;
        audio.apply(&volumes);
        assert!(channel_mean(&audio.capture(4_800), 0) < 0.00001);
        audio.capture(48_000);
        assert_eq!(score.state(), PlaybackState::Stopped);
        assert_eq!(bgm.state(), PlaybackState::Stopped);
        assert_eq!(sfx.state(), PlaybackState::Stopped);
    }

    #[test]
    fn pcm_spatial_mixer_pans_and_stop_before_first_callback_ends() {
        let mut audio = capture_audio();
        let clip = PcmClip::mono(24_000, &vec![0.25; 24_000]).unwrap();
        let mut opts = PcmOptions::new(Category::Voice);
        opts.spatial = Some(PcmSpatial { position: Vec3::X * 10.0, min_distance: 1.0, max_distance: 21.0 });
        let (mut handle, mut track) = begin_pcm(&mut audio, &clip, opts, Vec3::ZERO).unwrap();
        let out = audio.capture(4_800);
        assert!(channel_mean(&out, 1) > channel_mean(&out, 0) * 5.0, "source on right pans right");
        track.as_mut().unwrap().set_position(Vec3::X * -10.0, tween_ms(20));
        let out = audio.capture(4_800);
        assert!(channel_mean(&out, 0) > channel_mean(&out, 1) * 5.0, "moved source pans left");
        audio.listener.set_position(Vec3::X * -20.0, tween_ms(20));
        let out = audio.capture(4_800);
        assert!(channel_mean(&out, 1) > channel_mean(&out, 0) * 5.0, "moving listener also changes pan");
        handle.stop(tween_ms(20));
        audio.capture(2_400);
        assert_eq!(handle.state(), PlaybackState::Stopped);
        let (mut unstarted, _) = begin_pcm(&mut audio, &clip, PcmOptions::new(Category::Ui), Vec3::ZERO).unwrap();
        unstarted.stop(tween_ms(20));
        audio.capture(2_400);
        assert_eq!(unstarted.state(), PlaybackState::Stopped);
    }

    #[test]
    fn velocity_is_smoothed_and_a_jump_is_not_a_speed() {
        // 6 m/s along x, a tenth of a second at a time: the estimate climbs to 6
        let mut v = Vec3::ZERO;
        let mut pos = Vec3::ZERO;
        for _ in 0..30 {
            let next = pos + Vec3::new(0.6, 0.0, 0.0);
            v = smooth_velocity(v, pos, next, 0.1);
            pos = next;
        }
        assert!((v.x - 6.0).abs() < 0.05, "{v}");
        // a teleport of 500 m in a frame is no speed at all
        assert_eq!(smooth_velocity(v, pos, pos + Vec3::new(500.0, 0.0, 0.0), 0.016), Vec3::ZERO);
    }

    #[test]
    fn spawn_gaps_lie_between_the_template_times() {
        let mut rng = Rng::new(11);
        for _ in 0..200 {
            let g = spawn_gap(1000, 3000, &mut rng);
            assert!((Duration::from_millis(1000)..=Duration::from_millis(3000)).contains(&g), "{g:?}");
        }
        // 0 to 1 ms is "as soon as there is room": never a zero wait
        assert!(spawn_gap(0, 1, &mut rng) >= Duration::from_millis(1));
        assert_eq!(spawn_gap(3000, 3000, &mut rng), Duration::from_millis(3000));
    }

    #[test]
    fn a_muted_engine_has_no_live_voices() {
        let engine = Engine::start(Settings { audible: false, ..Settings::default() });
        let v = engine.play_2d("sounds/x2interface:interface:button_default");
        assert!(!engine.is_playing(v));
        assert!(engine.music_playing().is_none());
        assert_eq!(engine.stats().music_ended, 0);
    }
}
