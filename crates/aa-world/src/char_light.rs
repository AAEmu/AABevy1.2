//! The time of day's `Char BackLight` for the characters of the world (`aa_cry::shade::backlight` has the shader term and
//! what the engine does with the two lights).
//!
//! [`EnvSample::char_back`](crate::env::EnvSample) holds each light as `"i Color" x "i Diffuse Multiplier"` in engine
//! units. The engine adds the term to the same light sum as the ambient light (`cAmbientAcc + cDiffuseAcc`, then x the
//! diffuse map), so the units are the ambient light's: `ambient_scale` converts one engine unit of ambient light to
//! the brightness of Bevy's ambient light, i.e. a radiance `L`, and a directional light of illuminance `pi x L`
//! lights a diffuse surface to the same radiance. (The sun uses the smaller `sun_lux_scale`, fitted to the captures
//! of sunlit ground; with it the back lights came out 5.4 times too dark against the palace capture, where they are
//! the main light of a character in the shade: 1.00 x 0.9 against an ambient light of 0.45.) They follow the time
//! of day and the zone like every other environment value (they are constant in the files that exist).
//! `AA_CHAR_BACKLIGHT=<gain>` scales them for comparisons (0 switches the term off).

use aa_cry::shade::backlight::{CDF_POSITIONS, CharBackLight, CharLightPlugin};
use bevy::prelude::*;

use crate::sky::{Environment, update_environment};
use crate::stream::WorldConfig;

#[derive(Resource)]
struct Gain(f32);

/// Registers the character light plugin and the system that feeds it.
pub fn build(app: &mut App) {
    if !app.is_plugin_added::<CharLightPlugin>() {
        app.add_plugins(CharLightPlugin);
    }
    let gain = std::env::var("AA_CHAR_BACKLIGHT").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0);
    app.insert_resource(Gain(gain)).add_systems(Update, update_char_back_light.after(update_environment));
}

fn update_char_back_light(cfg: Res<WorldConfig>, env: Res<Environment>, gain: Res<Gain>, mut lights: ResMut<CharBackLight>) {
    let Some(sample) = &env.sample else { return };
    let scale = std::f32::consts::PI * cfg.ambient_scale * gain.0;
    let next = CharBackLight::from_positions(CDF_POSITIONS, sample.char_back.map(|l| (l.truncate() * scale).extend(l.w)));
    if *lights != next {
        *lights = next;
    }
}
