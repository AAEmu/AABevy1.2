//! The sound moods: which of the original's `soundmoods.xml` moods are active, handed to the audio engine, and the sounds the
//! zone's packs attach to them.
//!
//! Every mood of that file is a complete mix (a volume and a low-pass cutoff for each mixer category); the active mood of
//! the highest priority is the whole mix ([`aa_audio::mood`]). The client knows these states:
//!
//! | mood | priority | when |
//! |---|---|---|
//! | `loading` | 50 | the loading screen: silence |
//! | `death` | 25 | the character is dead: everything drops to 20 % under a 500 Hz low-pass, the zone music is silent |
//! | `combat` | 10 | the fight lasts (and 8 s after it): the zone music is silent for the battle track |
//! | `underwater` | 8 | the camera is below the surface of the sea or a pond: ambience 10 % and every effect low-passed at 300 Hz to 2 kHz |
//! | `low_health` | 8 | the character is alive with a fifth of his health or less (the threshold is a guess) |
//! | `falling` | 8 | the character is in a free fall |
//! | `driving` | 8 | the character steers a vehicle or a mount |
//! | `indoor` | 2 | the listener is inside a hall / room / cave reverb volume: the outdoor ambience goes, the inside ambience comes |
//! | `default` | 1 | always in game, the mix nothing else claims |
//!
//! A mood also has a sound: the item of that name in the zone's `soundmood_*` packs (`combat` -> `battle_normal`, `death` ->
//! `conditions_bgm:death`, `low_health` -> the heartbeat, `falling` -> the wind, `gliding`, `quest_camera`, `siege_period`).
//! The original (`SoundMoodManager::AddSoundMood`, `FUN_3907ac50`) starts it when the mood is added and fades it out when the
//! mood is removed (2 s); [`drive_moods`] does the same, the sound coming from the most specific pack of the place that has
//! the item. The zone music keeps playing under the mix and is heard again when the mood ends.
//!
//! The original's `gliding` (no glider in the 1.2 data the client has), `flyng`, `moving`, `dancing`, `menu`,
//! `quest_camera`, `siege_period` and `machinima_default` are not raised: their triggers are not known. `indoor` is
//! inferred from the reverb preset the player stands in (the `ReverbVolume` entities of halls, rooms and caves), which is
//! the only indoor information the data gives.
//!
//! Diving is the original's too (`FUN_39132910`): going under the surface plays `foley:dive_in` and starts the loop
//! `_zone_loop:underwater`; coming up stops the loop and plays `foley:dive_out`.

use aa_audio::{Engine, SpatialListener, Voice};
use aa_net::state::PlayerStats;
use aa_world::objects::sounds::ListenerReverb;
use aa_world::{WaterQuery, bevy_to_cry};
use bevy::prelude::*;

use super::ui::Gate;
use super::zone::ZonePacks;
use super::{Packs, log_enabled};
use crate::Screen;
use crate::ingame::{LocalJump, LocalMount};

/// The reverb presets of enclosed places.
const INDOOR: [&str; 3] = ["x2_hall_1", "x2_room_1", "x2_cave_1"];
/// Seconds the combat mood goes on after the server said the fight is over (the combat music's tail).
const COMBAT_TAIL: f64 = 8.0;
/// A fifth of the health or less is low health.
const LOW_HEALTH: f32 = 0.2;
/// How often the state is looked at.
const EVERY: f64 = 0.15;

const DIVE_IN: &str = "sounds/x2physics:foley:dive_in";
const DIVE_OUT: &str = "sounds/x2physics:foley:dive_out";
const UNDERWATER: &str = "sounds/x2ambience:_zone_loop:underwater";

/// What decides the moods.
#[derive(Default, Clone, Copy, Debug)]
struct Inputs {
    loading: bool,
    in_game: bool,
    dead: bool,
    /// The fight lasts or ended less than [`COMBAT_TAIL`] ago.
    fighting: bool,
    combat_music: bool,
    /// Health 0 to 1.
    health: Option<f32>,
    falling: bool,
    driving: bool,
    underwater: bool,
    indoor: bool,
}

/// The active moods, the mix is the one with the highest priority ([`aa_audio::MoodTable`]).
fn select(i: &Inputs) -> Vec<&'static str> {
    let mut moods = Vec::new();
    if i.loading {
        moods.push("loading");
    }
    if !i.in_game {
        return moods;
    }
    if i.dead {
        moods.push("death");
    }
    if i.combat_music && i.fighting {
        moods.push("combat");
    }
    if i.underwater {
        moods.push("underwater");
    }
    if !i.dead && i.health.is_some_and(|h| h > 0.0 && h <= LOW_HEALTH) {
        moods.push("low_health");
    }
    if i.falling {
        moods.push("falling");
    }
    if i.driving {
        moods.push("driving");
    }
    if i.indoor {
        moods.push("indoor");
    }
    // the mix nothing else claims
    moods.push("default");
    moods
}

#[derive(Default)]
pub(super) struct Sent {
    moods: Vec<&'static str>,
    combat_until: f64,
    next: f64,
    /// The sounds of the active moods.
    voices: Vec<(&'static str, Voice)>,
    /// Whether the camera was under water at the last look, and the loop that goes with it.
    submerged: Option<bool>,
    underwater_loop: Option<Voice>,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn drive_moods(
    engine: Res<Engine>,
    gate: Res<Gate>,
    packs: Res<Packs>,
    zone_packs: Res<ZonePacks>,
    screen: Res<State<Screen>>,
    time: Res<Time>,
    stats: Option<Res<PlayerStats>>,
    reverb: Option<Res<ListenerReverb>>,
    water: Option<Res<WaterQuery>>,
    jump: Option<Res<LocalJump>>,
    mount: Option<Res<LocalMount>>,
    ears: Query<&GlobalTransform, With<SpatialListener>>,
    mut sent: Local<Sent>,
) {
    let now = time.elapsed_secs_f64();
    if now < sent.next {
        return;
    }
    sent.next = now + EVERY;

    let screen = *screen.get();
    let mut input = Inputs { loading: screen == Screen::Loading, in_game: screen == Screen::InGame, ..default() };
    let mut submerged = false;
    if input.in_game {
        let in_combat = stats.as_deref().is_some_and(|s| s.in_combat);
        if in_combat {
            sent.combat_until = now + COMBAT_TAIL;
        }
        input.dead = stats.as_deref().is_some_and(|s| s.dead);
        input.fighting = now < sent.combat_until;
        input.combat_music = gate.0.options.lock().unwrap_or_else(|e| e.into_inner()).combat_music;
        input.health = stats.as_deref().map(PlayerStats::hp_fraction);
        input.falling = jump.as_deref().is_some_and(|j| j.freefall);
        input.driving = mount.as_deref().is_some_and(|m| m.driving().is_some());
        if let (Some(water), Some(ear)) = (&water, ears.iter().next()) {
            let p = ear.translation();
            // the camera below the surface: the same test the post pass tints the view by
            submerged = water.body_at(bevy_to_cry(p)).is_some_and(|b| p.y < b.height - 0.02);
        }
        input.underwater = submerged;
        input.indoor = reverb.as_deref().is_some_and(|r| r.weight >= 0.99 && r.preset.as_deref().is_some_and(|p| INDOOR.contains(&p)));
    } else {
        sent.combat_until = 0.0;
    }
    let moods = select(&input);

    if moods != sent.moods {
        if log_enabled() {
            info!("audio: moods {moods:?}");
        }
        engine.set_moods(&moods);
        mood_sounds(&engine, packs.get(), &zone_packs.packs, &moods, &mut sent.voices);
        sent.moods = moods;
    }

    dive(&engine, input.in_game, submerged, &mut sent);
}

/// Starts the sound of each mood that came up and fades the sound of each that went.
fn mood_sounds(engine: &Engine, packs: Option<&aa_audio::SoundPacks>, zone: &[String], active: &[&'static str], voices: &mut Vec<(&'static str, Voice)>) {
    voices.retain(|&(mood, voice)| {
        let keep = active.contains(&mood);
        if !keep {
            engine.stop(voice, true);
        }
        keep
    });
    let Some(packs) = packs else { return };
    for &mood in active {
        if voices.iter().any(|&(m, _)| m == mood) {
            continue;
        }
        let Some(path) = packs.mood_event(zone, mood) else { continue };
        if log_enabled() {
            info!("audio: mood sound {mood} -> {path}");
        }
        voices.push((mood, engine.play_2d(path)));
    }
}

/// The dive sounds: the plunge, the underwater loop and the way out.
fn dive(engine: &Engine, in_game: bool, submerged: bool, sent: &mut Sent) {
    let was = sent.submerged;
    let now = in_game.then_some(submerged);
    if was == now {
        return;
    }
    sent.submerged = now;
    match (was, now) {
        (_, Some(true)) => {
            // (spawning under water has the loop and no plunge)
            if was.is_some() {
                engine.play_2d(DIVE_IN);
            }
            sent.underwater_loop = Some(engine.play_2d(UNDERWATER));
        }
        (_, other) => {
            if let Some(voice) = sent.underwater_loop.take() {
                engine.stop(voice, true);
            }
            if was == Some(true) && other == Some(false) {
                engine.play_2d(DIVE_OUT);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game() -> Inputs {
        Inputs { in_game: true, combat_music: true, ..default() }
    }

    #[test]
    fn a_quiet_walk_is_the_default_mix() {
        assert_eq!(select(&game()), ["default"]);
        assert_eq!(select(&Inputs { indoor: true, ..game() }), ["indoor", "default"]);
    }

    #[test]
    fn the_screens_outside_the_game_are_loading_or_nothing() {
        assert_eq!(select(&Inputs { loading: true, ..default() }), ["loading"]);
        assert!(select(&Inputs::default()).is_empty());
    }

    #[test]
    fn combat_follows_the_option_and_death_wins_the_list() {
        assert_eq!(select(&Inputs { fighting: true, ..game() }), ["combat", "default"]);
        assert_eq!(select(&Inputs { fighting: true, combat_music: false, ..game() }), ["default"]);
        assert_eq!(select(&Inputs { dead: true, fighting: true, health: Some(0.0), ..game() }), ["death", "combat", "default"]);
    }

    #[test]
    fn low_health_needs_a_living_character_at_a_fifth() {
        assert_eq!(select(&Inputs { health: Some(0.2), ..game() }), ["low_health", "default"]);
        assert_eq!(select(&Inputs { health: Some(0.21), ..game() }), ["default"]);
        assert_eq!(select(&Inputs { health: Some(0.0), ..game() }), ["default"]);
        assert_eq!(select(&Inputs { health: Some(0.1), dead: true, ..game() }), ["death", "default"]);
    }

    #[test]
    fn falling_driving_and_diving_are_moods() {
        let all = Inputs { falling: true, driving: true, underwater: true, ..game() };
        assert_eq!(select(&all), ["underwater", "falling", "driving", "default"]);
    }
}
