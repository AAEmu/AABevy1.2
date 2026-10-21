//! The music director: one track at a time, crossfaded.
//!
//! * **Login screens.** What the original stage scripts play: `PlayMusic("login_stage_music_character_select")` when the
//!   character list is built, `StopMusic()` when the game starts. The world select script plays nothing of its own (the
//!   track of the login screen before it goes on: `login_stage_music_before_login`, `login_2013_07`) and the create
//!   screens' script only plays music in the credits window, so those two get the screen's track here
//!   (`login_stage_music_creator`, `login_creator_oblivion`). Whatever a script asks for through `AAUI.PlayMusic` /
//!   `AAUI.StopMusic` replaces it until the next screen. The loading screen is silent.
//! * **In game.** The zone's `soundmood_*` pack ([`super::zone`]) has up to eight tracks (`music_01` ...), one-shot events
//!   about two minutes long: one is picked at random when the pack changes and, after a pause, another when it has ended;
//!   `combat` replaces it while the player fights (`battle_normal`, `battle_sea`) and `death` while he is dead. The
//!   other moods of the pack (`underwater`, `gliding`, `falling`, `indoor` ...) are not used.
//!
//! Crossfades are the engine's: the old track fades out while the new one fades in.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use aa_audio::Engine;
use aa_net::state::PlayerStats;
use bevy::prelude::*;

use super::zone::ZoneLoad;
use super::{Packs, Shared, log_enabled};
use crate::Screen;

/// Fades of the login screens' music, of the zone music and of a stop.
const LOGIN_IN: Duration = Duration::from_millis(2000);
const LOGIN_OUT: Duration = Duration::from_millis(1500);
const ZONE_IN: Duration = Duration::from_millis(3000);
const ZONE_OUT: Duration = Duration::from_millis(3000);
const COMBAT_IN: Duration = Duration::from_millis(1500);
const STOP: Duration = Duration::from_millis(1000);

/// Seconds the zone music stays silent between two tracks.
const GAP: (f32, f32) = (20.0, 60.0);
/// Seconds the combat music goes on after the server said the fight is over.
const COMBAT_TAIL: f64 = 8.0;

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
}

/// The zone's music: which pack, which track of it, when it started.
#[derive(Default)]
struct ZoneMusic {
    pack: Option<String>,
    /// The base track (`music_NN`), its event path and when it started.
    base: Option<String>,
    started: f64,
    /// Seconds it lasts, from the event's longest wave (found on a thread), and the pause after it.
    length: Option<f32>,
    length_found: Option<Arc<OnceLock<f32>>>,
    gap: f32,
    combat_until: f64,
    last_mood: Mood,
    next_check: f64,
    seed: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
enum Mood {
    #[default]
    Base,
    Combat,
    Death,
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
        let seed = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64) | 1;
        Director { shared, ui_item: None, no_login, last_screen: None, applied: None, zone: ZoneMusic { seed, ..default() } }
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

/// How long an event plays, from its longest wave: read from the FEV on a thread (the project is parsed on first use).
fn event_length(path: String) -> Arc<OnceLock<f32>> {
    let found = Arc::new(OnceLock::new());
    let tx = found.clone();
    crate::task::spawn("sound-length", move || {
            use aa_audio::fev::Wave;
            let Some(event) = aa_audio::Library::global().event(&path) else { return };
            let fev = &event.project.fev;
            let longest = event
                .event()
                .instances()
                .iter()
                .flat_map(|i| fev.sound_defs.get(i.sound_def).map(|d| d.waves.iter()).into_iter().flatten())
                .filter_map(|w| if let Wave::Bank { length_ms, .. } = w { Some(*length_ms) } else { None })
                .max();
            if let Some(ms) = longest {
                let _ = tx.set(ms as f32 / 1000.0);
            }
        });
    found
}

#[allow(clippy::too_many_arguments)]
pub(super) fn drive_music(
    mut d: ResMut<Director>,
    engine: Res<Engine>,
    packs: Res<Packs>,
    zones: Res<ZoneLoad>,
    screen: Res<State<Screen>>,
    time: Res<Time>,
    stats: Option<Res<PlayerStats>>,
    net_world: Option<Res<aa_net::state::WorldInfo>>,
    world: Option<Res<aa_world::stream::WorldInfo>>,
    listener: Query<&GlobalTransform, With<aa_audio::SpatialListener>>,
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
            d.zone = ZoneMusic { seed: d.zone.seed, ..default() };
        }
    }
    // what the scripts ask for
    if let Some(request) = d.shared.music.lock().unwrap_or_else(|e| e.into_inner()).take() {
        d.ui_item = request;
    }
    let want = match screen {
        Screen::Loading => Want::Silence,
        Screen::InGame => {
            let combat_music = d.shared.options.lock().unwrap_or_else(|e| e.into_inner()).combat_music;
            let ear = listener.iter().next().map(|t| aa_world::bevy_to_cry(t.translation()));
            let (in_combat, dead) = stats.as_deref().map_or((false, false), |s| (s.in_combat, s.dead));
            let zone_key = world.as_deref().and_then(|w| ear.and_then(|p| w.0.zone_at(p))).map(u32::from).or_else(|| net_world.as_deref().map(|w| w.zone_key));
            let group = net_world.as_deref().and_then(|w| w.zone_group_id);
            zone_want(&mut d.zone, packs, zones.get(), ear, zone_key, group, time.elapsed_secs_f64(), in_combat && combat_music, dead)
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
    }
    d.applied = Some(want);
}

/// The zone music for this frame: updates the pack, the base track and the mood every half second.
#[allow(clippy::too_many_arguments)]
fn zone_want(
    z: &mut ZoneMusic,
    packs: &aa_audio::SoundPacks,
    zones: Option<&super::zone::ZoneData>,
    ear: Option<Vec3>,
    zone_key: Option<u32>,
    group: Option<u32>,
    now: f64,
    in_combat: bool,
    dead: bool,
) -> Want {
    if now >= z.next_check {
        z.next_check = now + 0.5;
        if let (Some(zones), Some(p)) = (zones, ear) {
            let pack = zones.pack_at(p.x, p.y, zone_key, group).map(str::to_string);
            if pack != z.pack {
                if log_enabled() {
                    info!("audio: zone sound pack {:?} -> {:?}", z.pack, pack);
                }
                z.pack = pack;
                z.base = None;
            }
        }
        if in_combat {
            z.combat_until = now + COMBAT_TAIL;
        }
        let mood = if dead {
            Mood::Death
        } else if now < z.combat_until {
            Mood::Combat
        } else {
            Mood::Base
        };
        if mood != z.last_mood {
            // back to the zone music: a new track right away
            if mood == Mood::Base {
                z.base = None;
            }
            z.last_mood = mood;
        }
        if let Some(pack) = z.pack.clone() {
            // the track that has ended: another one after a pause
            if let (Some(_), Some(len)) = (&z.base, z.length)
                && now > z.started + f64::from(len) + f64::from(z.gap)
            {
                z.base = None;
            }
            if z.base.is_none() && mood == Mood::Base {
                let tracks: Vec<&str> = (1..=8).filter_map(|n| packs.path(&pack, &format!("music_{n:02}"))).collect();
                if !tracks.is_empty() {
                    let pick = tracks[(random(&mut z.seed) * tracks.len() as f32) as usize % tracks.len()];
                    z.base = Some(pick.to_string());
                    z.started = now;
                    z.length = None;
                    z.gap = GAP.0 + (GAP.1 - GAP.0) * random(&mut z.seed);
                    z.length_found = Some(event_length(pick.to_string()));
                }
            }
        }
        if let Some(found) = &z.length_found
            && let Some(len) = found.get()
        {
            z.length = Some(*len);
        }
    }
    let Some(pack) = &z.pack else { return Want::Silence };
    let mood_path = match z.last_mood {
        Mood::Death => packs.path(pack, "death"),
        Mood::Combat => packs.path(pack, "combat"),
        Mood::Base => None,
    };
    if let Some(path) = mood_path {
        let fade_in = if z.last_mood == Mood::Combat { COMBAT_IN } else { ZONE_IN };
        return Want::Track { path: path.to_string(), fade_in, fade_out: ZONE_OUT };
    }
    match (&z.base, z.length) {
        // the track is playing; in its pause nothing is
        (Some(path), len) if len.is_none_or(|l| now < z.started + f64::from(l)) => Want::Track { path: path.clone(), fade_in: ZONE_IN, fade_out: ZONE_OUT },
        _ => Want::Silence,
    }
}
