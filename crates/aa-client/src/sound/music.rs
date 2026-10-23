//! The music director: one track at a time, crossfaded.
//!
//! * **Login screens.** What the original stage scripts play: `PlayMusic("login_stage_music_character_select")` when the
//!   character list is built, `StopMusic()` when the game starts. The world select script plays nothing of its own (the
//!   track of the login screen before it goes on: `login_stage_music_before_login`, `login_2013_07`) and the create
//!   screens' script only plays music in the credits window, so those two get the screen's track here
//!   (`login_stage_music_creator`, `login_creator_oblivion`). Whatever a script asks for through `AAUI.PlayMusic` /
//!   `AAUI.StopMusic` replaces it until the next screen. The loading screen is silent.
//! * **In game.** The original's `MusicManager::Update` (x2game, `FUN_39052ea0`) looks once a second at the sound packs of the
//!   place: the tracks are `music_01`, `music_02` ... of the sub zone's `soundmood_*` pack, else of the zone group's
//!   ([`aa_audio::SoundPacks::music_tracks`]). When that list changes (a zone or sub zone border was crossed) a random track
//!   of the new one starts at once and crossfades with the old one by the events' own fades (6 s in, 4 s out); a track that
//!   has played out is followed by another random one after 10 to 30 ms (the delay is in milliseconds; at one look a second
//!   that is the next look). Combat, death and the other moods are not music tracks: they are sounds of the mood that start
//!   and stop with it ([`super::moods`]), and the mood's mix lowers the zone music under them while it plays on.
//!
//! Crossfades are the engine's: the old track fades out while the new one fades in.

use std::sync::Arc;
use std::time::Duration;

use aa_audio::Engine;
use bevy::prelude::*;

use super::zone::ZonePacks;
use super::{Packs, Shared, log_enabled};
use crate::Screen;

/// Fades of the login screens' music and of a stop.
const LOGIN_IN: Duration = Duration::from_millis(2000);
const LOGIN_OUT: Duration = Duration::from_millis(1500);
const STOP: Duration = Duration::from_millis(1000);

/// Seconds between two looks at the zone's tracks (the original's one second).
const POLL: f64 = 1.0;
/// The delay after a track has played out, milliseconds (the original: 10 + 20 * random).
const NEXT_MS: (f32, f32) = (10.0, 30.0);
/// Seconds after which a track that never sounded (an event that is missing) is replaced by another.
const NEVER_SOUNDED: f64 = 10.0;

/// The pack item the login screens play where their scripts are silent.
fn screen_item(screen: Screen) -> Option<&'static str> {
    match screen {
        // the login screen and world select share the track: it goes on across the change
        Screen::Login | Screen::WorldSelect => Some("login_stage_music_before_login"),
        Screen::CharacterSelect => Some("login_stage_music_character_select"),
        Screen::CharacterCreate => Some("login_stage_music_creator"),
        Screen::Loading | Screen::InGame => None,
    }
}

/// What should be heard now.
#[derive(Clone, PartialEq, Debug)]
enum Want {
    Silence,
    Track { path: String, fade_in: Duration, fade_out: Duration },
    /// In game: the zone music is in charge.
    Zone,
}

/// What the zone music did at one look.
#[derive(Debug, PartialEq)]
enum Step {
    Nothing,
    /// Start this track (a crossfade when one plays).
    Play(String),
    /// The place has no music: let the old track fade out.
    Stop,
}

/// The zone's music: the track list it was started from, the last pick and the time of the next start.
#[derive(Default)]
struct ZoneMusic {
    tracks: Vec<String>,
    last: Option<usize>,
    /// When the track in play was asked for.
    requested: Option<f64>,
    /// The engine's count of tracks that played out, as of the last look.
    ended_seen: u64,
    start_at: f64,
    next_poll: f64,
    seed: u64,
}

impl ZoneMusic {
    fn new(seed: u64, ended_seen: u64) -> ZoneMusic {
        ZoneMusic { seed, ended_seen, ..default() }
    }

    fn due(&self, now: f64) -> bool {
        now >= self.next_poll
    }

    fn pick(&mut self, now: f64) -> Step {
        let n = self.tracks.len();
        let mut i = (random(&mut self.seed) * n as f32) as usize % n;
        // not the one that just played, when there is another
        if n > 1 && self.last == Some(i) {
            i = (i + 1) % n;
        }
        self.last = Some(i);
        self.requested = Some(now);
        Step::Play(self.tracks[i].clone())
    }

    /// One look at the place's tracks. `playing` is whether the engine has a track sounding, `ended` its count of tracks that
    /// played out. Call when [`ZoneMusic::due`].
    fn step(&mut self, tracks: &[&str], playing: bool, ended: u64, now: f64) -> Step {
        self.next_poll = now + POLL;
        if tracks.is_empty() {
            self.tracks.clear();
            self.last = None;
            self.ended_seen = ended;
            return if self.requested.take().is_some() { Step::Stop } else { Step::Nothing };
        }
        if !self.tracks.iter().map(String::as_str).eq(tracks.iter().copied()) {
            // another place: a track of its list, at once
            self.tracks = tracks.iter().map(|t| t.to_string()).collect();
            self.last = None;
            self.ended_seen = ended;
            return self.pick(now);
        }
        if ended != self.ended_seen {
            // the track played out: the next after a moment
            self.ended_seen = ended;
            self.requested = None;
            self.start_at = now + f64::from(NEXT_MS.0 + (NEXT_MS.1 - NEXT_MS.0) * random(&mut self.seed)) / 1000.0;
        }
        if self.requested.is_some_and(|t| !playing && now - t > NEVER_SOUNDED) {
            self.requested = None;
            self.start_at = now;
        }
        if self.requested.is_none() && now >= self.start_at {
            return self.pick(now);
        }
        Step::Nothing
    }
}

#[derive(Resource)]
pub(super) struct Director {
    shared: Arc<Shared>,
    /// The pack item the login screen plays (the screen's, or the scripts' request).
    ui_item: Option<String>,
    /// `--enter-world`: no login music.
    no_login: bool,
    last_screen: Option<Screen>,
    applied: Option<Want>,
    zone: ZoneMusic,
}

impl Director {
    pub fn new(shared: Arc<Shared>, no_login: bool) -> Director {
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64) | 1;
        Director { shared, ui_item: None, no_login, last_screen: None, applied: None, zone: ZoneMusic::new(seed, 0) }
    }
}

fn random(seed: &mut u64) -> f32 {
    let mut x = *seed | 1;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *seed = x;
    (x >> 40) as f32 / (1u64 << 24) as f32
}

pub(super) fn drive_music(
    mut d: ResMut<Director>,
    engine: Res<Engine>,
    packs: Res<Packs>,
    zone_packs: Res<ZonePacks>,
    screen: Res<State<Screen>>,
    time: Res<Time>,
) {
    let Some(packs) = packs.get() else { return };
    let screen = *screen.get();
    let d = &mut *d;
    // a screen came up: its own track, and the scripts' latest request is stale
    if d.last_screen != Some(screen) {
        d.last_screen = Some(screen);
        d.ui_item = screen_item(screen).filter(|_| !d.no_login).map(str::to_string);
        d.shared.music.lock().unwrap_or_else(|e| e.into_inner()).take();
        if screen == Screen::InGame {
            d.zone = ZoneMusic::new(d.zone.seed, engine.stats().music_ended);
        }
    }
    // what the scripts ask for
    if let Some(request) = d.shared.music.lock().unwrap_or_else(|e| e.into_inner()).take() {
        d.ui_item = request;
    }
    let want = match screen {
        Screen::Loading => Want::Silence,
        Screen::InGame => {
            let now = time.elapsed_secs_f64();
            if d.zone.due(now) {
                let tracks = packs.music_tracks(&zone_packs.packs);
                let stats = engine.stats();
                match d.zone.step(&tracks, stats.music.is_some(), stats.music_ended, now) {
                    Step::Nothing => {}
                    Step::Play(path) => {
                        if log_enabled() {
                            info!("audio: zone music {path} (packs {:?}, {} tracks)", zone_packs.packs, tracks.len());
                        }
                        engine.play_music_event(&path);
                    }
                    Step::Stop => {
                        if log_enabled() {
                            info!("audio: zone music stop (packs {:?})", zone_packs.packs);
                        }
                        engine.stop_music(STOP);
                    }
                }
            }
            d.applied = Some(Want::Zone);
            return;
        }
        _ => match d.ui_item.as_deref() {
            None => Want::Silence,
            Some(item) => match packs.ui(item) {
                Some(path) => Want::Track { path: path.to_string(), fade_in: LOGIN_IN, fade_out: LOGIN_OUT },
                None => return,
            },
        },
    };
    if d.applied.as_ref() == Some(&want) {
        return;
    }
    match &want {
        Want::Silence => {
            // nothing was playing before the first track: no stop to send
            if d.applied.is_some() {
                if log_enabled() {
                    info!("audio: music stop");
                }
                engine.stop_music(STOP);
            }
        }
        Want::Track { path, fade_in, fade_out } => {
            if log_enabled() {
                info!("audio: music {path} (screen {screen:?}, fade in {} ms)", fade_in.as_millis());
            }
            engine.play_music(path, *fade_in, *fade_out);
        }
        Want::Zone => {}
    }
    d.applied = Some(want);
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAND: [&str; 3] = ["land:01", "land:02", "land:03"];
    const TOWN: [&str; 2] = ["town:01", "town:02"];

    fn play(step: Step) -> String {
        match step {
            Step::Play(p) => p,
            other => panic!("expected a track, got {other:?}"),
        }
    }

    #[test]
    fn a_place_starts_a_track_of_its_list_at_once() {
        let mut z = ZoneMusic::new(7, 0);
        let first = play(z.step(&LAND, false, 0, 100.0));
        assert!(LAND.contains(&first.as_str()));
        // a second later, with the track sounding: nothing
        assert!(z.due(101.0) && !z.due(100.5));
        assert_eq!(z.step(&LAND, true, 0, 101.0), Step::Nothing);
    }

    #[test]
    fn crossing_into_another_place_crossfades_to_its_track() {
        let mut z = ZoneMusic::new(7, 0);
        play(z.step(&LAND, false, 0, 100.0));
        let town = play(z.step(&TOWN, true, 0, 101.0));
        assert!(TOWN.contains(&town.as_str()));
        // the same list again is no change
        assert_eq!(z.step(&TOWN, true, 0, 102.0), Step::Nothing);
    }

    #[test]
    fn a_track_that_played_out_is_followed_by_another_after_a_moment() {
        let mut z = ZoneMusic::new(7, 3);
        let first = play(z.step(&LAND, false, 3, 100.0));
        // the engine counted an end: the next start waits the few milliseconds, so the next look starts it
        assert_eq!(z.step(&LAND, false, 4, 101.0), Step::Nothing);
        assert!(z.start_at > 101.0 && z.start_at < 101.05, "{}", z.start_at);
        let second = play(z.step(&LAND, false, 4, 102.0));
        assert_ne!(first, second, "not the track that just played");
    }

    #[test]
    fn a_track_that_never_sounded_is_replaced() {
        let mut z = ZoneMusic::new(7, 0);
        play(z.step(&LAND, false, 0, 100.0));
        assert_eq!(z.step(&LAND, false, 0, 105.0), Step::Nothing, "decoding takes a while");
        assert!(matches!(z.step(&LAND, false, 0, 111.0), Step::Play(_)));
    }

    #[test]
    fn a_place_without_music_stops_it_once() {
        let mut z = ZoneMusic::new(7, 0);
        play(z.step(&LAND, false, 0, 100.0));
        assert_eq!(z.step(&[], true, 0, 101.0), Step::Stop);
        assert_eq!(z.step(&[], false, 0, 102.0), Step::Nothing);
        // and back to a place with music
        assert!(matches!(z.step(&LAND, false, 0, 103.0), Step::Play(_)));
    }

    #[test]
    fn a_single_track_list_repeats_its_track() {
        let mut z = ZoneMusic::new(7, 0);
        let one = ["only:01"];
        play(z.step(&one, false, 0, 100.0));
        z.step(&one, false, 1, 101.0);
        assert_eq!(play(z.step(&one, false, 1, 102.0)), "only:01");
    }
}
