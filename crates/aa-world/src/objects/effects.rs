//! Placed particle effects: the `ParticleEffect` entities of a cell's `entities.xml` (torches, candles, smoke, waterfall spray,
//! ground mist, ...) become [`aa_fx::Effect`] entities. They run only near the camera (`aa-fx` starts an effect when the camera
//! comes within its radius and drops it again beyond), so a cell with hundreds of them costs one distance test each.
//!
//! The effects of a cell are despawned with it ([`spawn_cell`] returns their entities, `ObjectState` keeps them per cell).
//! `AA_NO_FX=1` leaves them out.

use aa_fx::{Effect, FxPlugin, FxSettings};
use bevy::camera::Exposure;
use bevy::prelude::*;

use crate::coords::{CRY_TO_BEVY, cell_origin, cry_to_bevy};
use crate::placement::PlacedEffect;
use crate::post::PostUniform;
use crate::sky::{AtmosphereTuning, view_exposure};

/// Particle colours are scaled like the other materials' HDR colours (`radiance x exposure`), times this calibration.
const GAIN: f32 = 1.0;

fn disabled() -> bool {
    std::env::var_os("AA_NO_FX").is_some()
}

pub(crate) fn build(app: &mut App) {
    if disabled() {
        return;
    }
    if !app.is_plugin_added::<FxPlugin>() {
        app.add_plugins(FxPlugin);
    }
    app.add_systems(Update, update_gain);
}

/// Keeps the particle colour gain on the world's exposure (the post pass expects pre-exposed HDR colours).
fn update_gain(tuning: Option<Res<AtmosphereTuning>>, exposure: Query<Option<&Exposure>, With<PostUniform>>, mut fx: ResMut<FxSettings>) {
    let Some(tuning) = tuning else { return };
    if exposure.is_empty() {
        return;
    }
    let gain = tuning.radiance * view_exposure(&exposure) * GAIN;
    if (fx.gain - gain).abs() > 1e-4 * gain.max(1.0) {
        fx.gain = gain;
    }
}

/// Spawns the effects of a cell; returns their entities.
pub(crate) fn spawn_cell(commands: &mut Commands, cell: (i32, i32), placed: &[PlacedEffect]) -> Vec<Entity> {
    if disabled() {
        return Vec::new();
    }
    let origin = cell_origin(cell.0, cell.1);
    placed
        .iter()
        .map(|p| {
            let mut effect = Effect::new(p.effect.clone());
            effect.scale = p.scale;
            effect.speed_scale = p.speed_scale;
            effect.count_scale = p.count_scale;
            effect.prime = p.prime;
            let transform = Transform {
                translation: cry_to_bevy(origin + p.pos),
                // the entity's rotation expressed in Bevy axes
                rotation: CRY_TO_BEVY * p.rot * CRY_TO_BEVY.inverse(),
                scale: Vec3::ONE,
            };
            commands.spawn((effect, transform)).id()
        })
        .collect()
}
