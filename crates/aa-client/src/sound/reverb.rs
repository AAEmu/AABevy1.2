//! The area reverb: the preset of the `ReverbVolume` the player is in goes to the audio engine.
//!
//! `aa-world`'s placed sounds work out which `ReverbVolume` entity is strongest around the listener
//! ([`ListenerReverb`]: the preset name and a weight, 1 inside a volume and falling to 0 at its `OuterRadius`, as the
//! original entity script `reverbvolume.lua` weights `Sound.UpdateWeightedEaxEnvironment`); this hands it to
//! `Engine::set_reverb`, which glides the one global reverb to it. Outside every volume there is no reverb. The reverb
//! belongs to the world: it is cleared when the game screen is left.

use aa_audio::Engine;
use aa_world::objects::sounds::ListenerReverb;
use bevy::prelude::*;

use super::log_enabled;
use crate::Screen;

/// What was last sent to the engine.
#[derive(Default)]
pub(super) struct Sent {
    preset: Option<String>,
    weight: f32,
}

pub(super) fn drive_reverb(engine: Res<Engine>, reverb: Option<Res<ListenerReverb>>, screen: Res<State<Screen>>, mut sent: Local<Sent>) {
    let want = match (screen.get(), reverb.as_deref()) {
        (Screen::InGame, Some(r)) if r.weight > 0.0 => (r.preset.clone(), r.weight),
        _ => (None, 0.0),
    };
    // the weight moves smoothly with the player; the engine glides, so a coarse step is enough
    if want.0 == sent.preset && (want.1 - sent.weight).abs() < 0.03 {
        return;
    }
    if log_enabled() {
        match &want.0 {
            Some(p) => info!("audio: reverb {p} weight {:.2} (applied)", want.1),
            None => info!("audio: reverb off"),
        }
    }
    engine.set_reverb(want.0.as_deref(), want.1);
    *sent = Sent { preset: want.0, weight: want.1 };
}
