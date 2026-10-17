//! The sound mood: which of the original's `soundmoods.xml` moods are active, handed to the audio engine.
//!
//! Every mood of that file is a complete mix (a volume and a low-pass cutoff for each mixer category); the active mood of
//! the highest priority is the whole mix ([`aa_audio::mood`]). The client knows these states:
//!
//! | mood | priority | when |
//! |---|---|---|
//! | `death` | 25 | the character is dead: everything drops to 20 % under a 500 Hz low-pass, the music stops |
//! | `combat` | 10 | the fight lasts (and 8 s after it, as the combat music): the zone music is silent for the battle track |
//! | `underwater` | 8 | the camera is below the surface of the sea or a pond: ambience 10 % and every effect low-passed at 300 Hz to 2 kHz |
//! | `indoor` | 2 | the listener is inside a hall / room / cave reverb volume: the outdoor ambience goes, the inside ambience comes |
//! | `default` | 1 | always in game, the mix nothing else claims: the inside ambience and the underwater only sounds are silent |
//!
//! The original's `loading` (silence), `low_health`, `gliding`, `driving`, `falling` and `dancing` are not raised: the
//! first two have no known trigger here (the low health threshold is in the C++), the others only mute `underwater_only`
//! like `default` does. `indoor` is inferred from the reverb preset the player stands in (the `ReverbVolume` entities of
//! halls, rooms and caves), which is the only indoor information the data gives.

use aa_audio::{Engine, SpatialListener};
use aa_net::state::PlayerStats;
use aa_world::objects::sounds::ListenerReverb;
use aa_world::{WaterQuery, bevy_to_cry};
use bevy::prelude::*;

use super::ui::Gate;
use super::log_enabled;
use crate::Screen;

/// The reverb presets of enclosed places.
const INDOOR: [&str; 3] = ["x2_hall_1", "x2_room_1", "x2_cave_1"];
/// Seconds the combat mood goes on after the server said the fight is over (the combat music's tail).
const COMBAT_TAIL: f64 = 8.0;
/// How often the state is looked at.
const EVERY: f64 = 0.15;

#[derive(Default)]
pub(super) struct Sent {
    moods: Vec<&'static str>,
    combat_until: f64,
    next: f64,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn drive_moods(
    engine: Res<Engine>,
    gate: Res<Gate>,
    screen: Res<State<Screen>>,
    time: Res<Time>,
    stats: Option<Res<PlayerStats>>,
    reverb: Option<Res<ListenerReverb>>,
    water: Option<Res<WaterQuery>>,
    ears: Query<&GlobalTransform, With<SpatialListener>>,
    mut sent: Local<Sent>,
) {
    let now = time.elapsed_secs_f64();
    if now < sent.next {
        return;
    }
    sent.next = now + EVERY;

    let mut moods: Vec<&'static str> = Vec::new();
    if *screen.get() == Screen::InGame {
        let (in_combat, dead) = stats.as_deref().map_or((false, false), |s| (s.in_combat, s.dead));
        let combat_music = gate.0.options.lock().unwrap_or_else(|e| e.into_inner()).combat_music;
        if in_combat {
            sent.combat_until = now + COMBAT_TAIL;
        }
        if dead {
            moods.push("death");
        }
        if combat_music && now < sent.combat_until {
            moods.push("combat");
        }
        if let (Some(water), Some(ear)) = (&water, ears.iter().next()) {
            let p = ear.translation();
            // the camera below the surface: the same test the post pass tints the view by
            if water.body_at(bevy_to_cry(p)).is_some_and(|b| p.y < b.height - 0.02) {
                moods.push("underwater");
            }
        }
        if reverb.as_deref().is_some_and(|r| r.weight >= 0.99 && r.preset.as_deref().is_some_and(|p| INDOOR.contains(&p))) {
            moods.push("indoor");
        }
        // the mix nothing else claims: the inside ambience and the underwater only sounds are silent
        moods.push("default");
    } else {
        sent.combat_until = 0.0;
    }

    if moods != sent.moods {
        if log_enabled() {
            info!("audio: moods {moods:?}");
        }
        engine.set_moods(&moods);
        sent.moods = moods;
    }
}
