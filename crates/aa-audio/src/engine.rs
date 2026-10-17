//! The player: one thread owns the mixer ([kira]) and every voice; the rest of the program talks to it
//! through [`Engine`], whose calls only send a message.
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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bevy::math::{Quat, Vec3};
use bevy::prelude::Resource;
use kira::listener::ListenerHandle;
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
use crate::muffle::{MuffleBuilder, MuffleHandle};
use crate::reverb::{Params as ReverbParams, Presets, ReverbBuilder, ReverbHandle};
use crate::library::{EventRef, Library, PickState, Planned, Project, Rng, active_instances, distance_gain, distance_parameters, initial_values, layer_gain, plan, set_value, start_part};
use crate::stream::StreamDecoder;
use crate::vorbis;

/// The volume group a sound belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    Music,
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
    pub sfx: f32,
    pub ambience: f32,
    pub voice: f32,
    pub ui: f32,
}

impl Default for Volumes {
    fn default() -> Volumes {
        Volumes { master: 1.0, music: 1.0, sfx: 1.0, ambience: 1.0, voice: 1.0, ui: 1.0 }
    }
}

impl Volumes {
    pub fn get(&self, c: Category) -> f32 {
        match c {
            Category::Music => self.music,
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
}

struct Shared {
    next: AtomicU64,
    /// Commands sent by [`Engine`] calls and commands the player has handled and published.
    sent: AtomicU64,
    handled: AtomicU64,
    stats: Mutex<Stats>,
    finished: Mutex<Vec<Voice>>,
}

enum Cmd {
    Play { id: Voice, path: String, at: Option<Vec3>, opts: PlayOptions },
    Move { id: Voice, at: Vec3 },
    Param { id: Voice, name: String, value: f32 },
    VoiceVolume { id: Voice, volume: f32 },
    Stop { id: Voice, fade: bool },
    StopAll,
    Listener { position: Vec3, rotation: Quat },
    Volumes(Volumes),
    Reverb { preset: Option<String>, weight: f32 },
    Moods(Vec<String>),
    Music { path: String, fade_in: Duration, fade_out: Duration },
    StopMusic { fade: Duration },
    Decoded { key: Key, result: Result<Arc<Sample>, String>, seconds: f64, took: Duration },
    MusicReady { token: u64, result: Result<Prepared, String> },
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
        let shared = Arc::new(Shared {
            next: AtomicU64::new(1),
            sent: AtomicU64::new(0),
            handled: AtomicU64::new(0),
            stats: Mutex::new(Stats { audible: settings.audible, ..Stats::default() }),
            finished: Mutex::new(Vec::new()),
        });
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
        Voice(self.shared.next.fetch_add(1, Ordering::Relaxed))
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
    pub fn stop(&self, voice: Voice, fade: bool) {
        self.send(Cmd::Stop { id: voice, fade });
    }

    pub fn stop_all(&self) {
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
        self.send(Cmd::Music { path: path.to_string(), fade_in, fade_out });
    }

    pub fn stop_music(&self, fade: Duration) {
        self.send(Cmd::StopMusic { fade });
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
                if s.active_voices == 0 && s.waiting_for_decode == 0 {
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

struct Audio {
    manager: AudioManager<DefaultBackend>,
    listener: ListenerHandle,
    music: TrackHandle,
    sfx: TrackHandle,
    ambience: TrackHandle,
    voice: TrackHandle,
    ui: TrackHandle,
    /// The reverb: a send track whose only effect is the area reverb, and its control.
    reverb_send: SendTrackHandle,
    reverb: ReverbHandle,
    /// Sub tracks of a group that send to the reverb at a wet level (whole dB), for sounds on no spatial track of their own.
    buses: HashMap<(Category, i16), TrackHandle>,
    /// The low-pass filters of the groups (all but the UI).
    muffles: HashMap<Category, MuffleHandle>,
}

impl Audio {
    fn track(&mut self, c: Category) -> &mut TrackHandle {
        match c {
            Category::Music => &mut self.music,
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
        for c in [Category::Music, Category::Sfx, Category::Ambience, Category::Voice, Category::Ui] {
            self.track(c).set_volume(db(v.get(c)), Tween::default());
        }
        for ((c, wet), bus) in self.buses.iter_mut() {
            let _ = bus.set_send(&self.reverb_send, send_db(f32::from(*wet), v.get(*c)), Tween::default());
        }
    }
}

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
    handle: Option<StaticSoundHandle>,
    track: Option<SpatialTrackHandle>,
    last_db: f32,
    /// Reverb wet level of the part's event, dB.
    wet_db: f32,
}

struct ActiveVoice {
    id: Voice,
    event: EventRef,
    category: Category,
    /// The last part of the event's mixer category (`footsteps`): what the mood mixes it by.
    leaf: String,
    /// The mood's gain for the voice, gliding.
    mood: f32,
    /// `Some` for a 3D event at a place.
    at: Option<Vec3>,
    moved: bool,
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
    pick: PickState,
    rng: Rng,
    music: Option<CurrentMusic>,
    music_token: u64,
    music_request: Option<(String, Duration, Duration)>,
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
            pick: PickState::default(),
            rng: Rng::from_time(),
            music: None,
            music_token: 0,
            music_request: None,
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
            let busy = !self.voices.is_empty() || self.music.is_some() || self.music_request.is_some();
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
                if !matches!(cmd, Cmd::Decoded { .. } | Cmd::MusicReady { .. }) {
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
        s.active_voices = self.voices.len();
        s.waiting_for_decode = self.pending.len() + usize::from(self.music_request.is_some());
        s.cache_bytes = self.cache.bytes();
        s.cache_sounds = self.cache.len();
        s.cache_hits = self.cache.hits;
        s.cache_misses = self.cache.misses;
        *self.shared.stats.lock().unwrap_or_else(|e| e.into_inner()) = s;
    }

    fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Play { id, path, at, opts } => self.play(id, &path, at, opts),
            Cmd::Move { id, at } => {
                if let Some(v) = self.voices.get_mut(&id)
                    && v.at.is_some()
                {
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
                if let Some(v) = self.voices.get_mut(&id) {
                    v.volume = volume;
                }
            }
            Cmd::Stop { id, fade } => self.stop_voice(id, fade),
            Cmd::StopAll => {
                let ids: Vec<Voice> = self.voices.keys().copied().collect();
                for id in ids {
                    self.stop_voice(id, true);
                }
            }
            Cmd::Listener { position, rotation } => {
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
                if let Some(m) = self.music.take() {
                    let mut h = m.handle;
                    h.stop(Tween { duration: fade.max(Duration::from_millis(1)), ..Tween::default() });
                }
            }
            Cmd::Decoded { key, result, seconds, took } => self.decoded(key, result, seconds, took),
            Cmd::MusicReady { token, result } => self.music_ready(token, result),
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
                if c != Category::Ui {
                    muffles.insert(c, builder.add_effect(MuffleBuilder));
                }
                manager.add_sub_track(builder).map_err(|e| e.to_string())
            };
            let music = group(&mut manager, Category::Music)?;
            let sfx = group(&mut manager, Category::Sfx)?;
            let ambience = group(&mut manager, Category::Ambience)?;
            let voice = group(&mut manager, Category::Voice)?;
            let ui = group(&mut manager, Category::Ui)?;
            let mut send = SendTrackBuilder::new();
            let reverb = send.add_effect(ReverbBuilder);
            let reverb_send = manager.add_send_track(send).map_err(|e| e.to_string())?;
            Ok(Audio { manager, listener, music, sfx, ambience, voice, ui, reverb_send, reverb, buses: HashMap::new(), muffles })
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
        self.shared.finished.lock().unwrap_or_else(|e| e.into_inner()).push(id);
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

        // Most playbacks of one event.
        let same: Vec<Voice> = {
            let mut v: Vec<&ActiveVoice> = self.voices.values().filter(|v| v.event.key() == ekey && !v.stopping).collect();
            v.sort_by_key(|v| v.requested);
            v.iter().map(|v| v.id).collect()
        };
        if same.len() >= ev.max_playbacks as usize {
            match ev.playback_behaviour {
                4 | 5 => {
                    self.stats.refused += 1;
                    return;
                }
                2 => self.stop_voice(*same.last().unwrap(), false),
                _ => self.stop_voice(same[0], false),
            }
            self.stats.stolen += 1;
        }
        // Most voices overall: the least important (highest priority number, then oldest) goes.
        let active = self.voices.values().filter(|v| !v.stopping).count();
        if active >= self.settings.max_voices {
            let victim = self.voices.values().filter(|v| !v.stopping).max_by_key(|v| (v.event.event().priority, std::cmp::Reverse(v.requested))).map(|v| (v.id, v.event.event().priority));
            match victim {
                Some((vid, prio)) if prio >= ev.priority => {
                    self.stop_voice(vid, false);
                    self.stats.stolen += 1;
                }
                _ => {
                    self.stats.refused += 1;
                    return;
                }
            }
        }
        if !self.ensure_audio() {
            return;
        }

        let values = initial_values(ev, distance, &opts.params);
        let parts = match &ev.body {
            Body::Simple(_) => vec![(0, 0)],
            Body::Complex { layers, .. } => layers.iter().enumerate().flat_map(|(l, layer)| (0..layer.instances.len()).map(move |i| (l, i))).collect(),
        }
        .into_iter()
        .map(|(layer, inst)| Part { layer, inst, state: PartState::Idle, planned: None, key: None, sample: None, handle: None, track: None, last_db: 0.0, wet_db: ev.reverb_wet })
        .collect();
        let leaf = ev.category.rsplit('/').next().unwrap_or_default().to_ascii_lowercase();
        let distance_params = distance_parameters(ev);
        let persistent = !ev.one_shot() && matches!(&ev.body, Body::Complex { parameters, .. } if (0..parameters.len()).any(|i| !distance_params.contains(&i)));
        let mut voice = ActiveVoice {
            id,
            event: event.clone(),
            category: opts.category.unwrap_or_else(|| Category::of_mixer(&ev.category)),
            leaf: leaf.clone(),
            mood: self.mood_gain(&leaf),
            at: if is_3d { at } else { None },
            moved: false,
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
                    let Some(p) = start_part(&event, part.layer, part.inst, &mut self.pick, &mut self.rng) else {
                        part.state = PartState::Done;
                        continue;
                    };
                    let Some(bank) = event.project.fev.banks.get(p.bank as usize) else {
                        part.state = PartState::Done;
                        continue;
                    };
                    let key: Key = (format!("{}/{}", event.project.dir, bank.name.to_ascii_lowercase()), p.index);
                    part.sample = self.cache.get(&key);
                    if part.sample.is_none() {
                        self.request_decode(key.clone(), voice.id, event.project.clone(), p.bank, p.index);
                    }
                    part.key = Some(key);
                    part.planned = Some(p);
                    part.state = PartState::Loading;
                }
                (PartState::Loading | PartState::Playing, false) => {
                    if let Some(h) = &mut part.handle {
                        h.stop(tween_ms(30));
                    }
                    part.handle = None;
                    part.state = PartState::Idle;
                }
                (PartState::Done, false) => part.state = PartState::Idle,
                _ => {}
            }
        }
    }

    /// Starts the parts that are waiting and have their sound. The first start waits for all the
    /// parts that are active then, so that the layers begin together.
    fn try_start(&mut self, voice: &mut ActiveVoice) {
        let loading = voice.parts.iter().filter(|p| p.state == PartState::Loading).count();
        if !voice.started {
            let ready = voice.parts.iter().filter(|p| p.state == PartState::Loading).all(|p| p.sample.is_some());
            if loading > 0 && !ready {
                return;
            }
            voice.started = true;
            if loading > 0 {
                self.stats.started += 1;
            }
        }
        let listener = self.listener.0;
        let volumes = self.volumes;
        let Some(audio) = &mut self.audio else { return };
        for i in 0..voice.parts.len() {
            if voice.parts[i].state == PartState::Loading && voice.parts[i].sample.is_some() {
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
        if self.music.as_ref().is_some_and(|m| m.handle.state() == PlaybackState::Stopped) {
            self.music = None;
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
            if voice.dirty {
                self.reconcile(voice);
            }
            self.try_start(voice);
        }
        voice.mood = self.mood_gain(&voice.leaf);
        let moved = std::mem::take(&mut voice.moved);
        for i in 0..voice.parts.len() {
            if voice.parts[i].state != PartState::Playing {
                continue;
            }
            if voice.parts[i].handle.as_ref().is_none_or(|h| h.state() == PlaybackState::Stopped) {
                let part = &mut voice.parts[i];
                part.handle = None;
                part.track = None;
                part.state = PartState::Done;
                continue;
            }
            let target = voice.target_gain(&voice.parts[i], listener);
            let part = &mut voice.parts[i];
            if moved && let (Some(at), Some(track)) = (voice.at, &mut part.track) {
                track.set_position(at, Tween::default());
            }
            let new_db = db(target).0;
            if (new_db - part.last_db).abs() > 0.2 {
                if let Some(h) = &mut part.handle {
                    h.set_volume(Decibels(new_db), tween_ms(30));
                }
                part.last_db = new_db;
            }
        }
        if voice.stopping || !voice.persistent { voice.live() || !voice.started } else { true }
    }

    fn play_music(&mut self, path: String, fade_in: Duration, fade_out: Duration) {
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
                Ok(Prepared { stream, looping: looping.then_some(sub.loop_points), gain: p.gain, rate: p.rate, wet_db, leaf })
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
        if let Some(old) = self.music.take() {
            let mut h = old.handle;
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
                self.music = Some(CurrentMusic { path, handle, gain: prepared.gain, leaf: prepared.leaf.clone(), last_db: db(prepared.gain * mood).0 });
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

/// Starts the sound of part `i`: on a spatial track for a 3D voice, on its group's track (or the reverb bus of its wet level)
/// otherwise. Both send to the reverb by the event's wet level.
fn begin_part(audio: &mut Audio, voice: &mut ActiveVoice, i: usize, listener: Vec3, volumes: &Volumes) {
    let gain = voice.target_gain(&voice.parts[i], listener);
    let ev = voice.event.event();
    let pan = ev.pan_level.clamp(0.0, 1.0).max(0.2);
    let category = voice.category;
    let at = voice.at;
    let part = &mut voice.parts[i];
    let wet_db = part.wet_db;
    let (Some(sample), Some(p)) = (part.sample.clone(), part.planned.clone()) else {
        part.state = PartState::Done;
        return;
    };
    let mut settings = StaticSoundSettings::default();
    if p.looping {
        settings = settings.loop_region(loop_region(sample.loop_points));
    }
    let mut data = StaticSoundData { sample_rate: sample.rate, frames: sample.frames.clone(), settings, slice: None }.volume(db(gain)).playback_rate(p.rate as f64);
    if p.fade_in_ms > 0 {
        data = data.fade_in_tween(tween_ms(p.fade_in_ms));
    }
    part.last_db = db(gain).0;
    let result = match at {
        Some(at) => {
            let mut builder = SpatialTrackBuilder::new().attenuation_function(None).spatialization_strength(pan).persist_until_sounds_finish(true);
            if wet_db > WET_OFF {
                builder = builder.with_send(&audio.reverb_send, send_db(wet_db, volumes.get(category)));
            }
            let listener_id = audio.listener.id();
            match audio.track(category).add_spatial_sub_track(listener_id, at, builder) {
                Ok(mut track) => {
                    let r = track.play(data).map_err(|e| e.to_string());
                    part.track = Some(track);
                    r
                }
                Err(e) => Err(e.to_string()),
            }
        }
        None => audio.bus(category, wet_db, volumes).play(data).map_err(|e| e.to_string()),
    };
    match result {
        Ok(h) => {
            part.handle = Some(h);
            part.state = PartState::Playing;
        }
        Err(e) => {
            tracing::debug!("aa-audio: cannot play {}: {e}", p.name);
            part.state = PartState::Done;
        }
    }
}
