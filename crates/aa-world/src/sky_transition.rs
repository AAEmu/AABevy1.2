//! Native discrete skybox/distant-cloud selection (crysystem 365F0210).
//!
//! Raw ToD values blend separately. One zone supplies sky assets, whose shared
//! opacity falls to zero at borders and while a relic/weather asset layer changes.

use bevy::prelude::Resource;

#[derive(Resource, Clone, Debug, Default)]
pub struct SkySelection {
    /// Strongest spatial input, independent of the containing sector's zone.
    pub zone: Option<u16>,
    /// Currently loaded relic layer, retained until its fade-out reaches zero.
    pub layer: u32,
    /// Native SetZoneSkyBoxOpacity, also passed to every distant-cloud node.
    pub opacity: f32,
    world: Option<String>,
    layer_weight: f32,
}

impl SkySelection {
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn update(&mut self, world: &str, weights: &[(u16, f32)], layer: u32, seconds: f32) {
        if self.world.as_deref() != Some(world) {
            *self = Self {
                world: Some(world.to_string()),
                ..Self::default()
            };
        }
        let mut total = 0.0;
        let mut strongest = 0.0;
        let mut zone = None;
        for &(id, weight) in weights.iter().take(10) {
            if !weight.is_finite() || weight <= 0.0 {
                continue;
            }
            total += weight;
            if weight > strongest {
                zone = Some(id);
                strongest = weight;
            }
        }
        if zone.is_none() {
            self.zone = None;
            self.opacity = 0.0;
            self.layer_weight = 0.0;
            return;
        }
        // 365F0210 resets a new zone to zero, even on a frame with nonzero dt.
        if zone != self.zone {
            self.zone = zone;
            self.layer = layer;
            self.layer_weight = 0.0;
        } else {
            let step = if seconds.is_finite() {
                seconds.max(0.0) * 0.5
            } else {
                0.0
            };
            if self.layer == layer {
                self.layer_weight = (self.layer_weight + step).clamp(0.0, 1.0);
            } else {
                self.layer_weight = (self.layer_weight - step).clamp(0.0, 1.0);
                if self.layer_weight == 0.0 {
                    self.layer = layer;
                }
            }
        }
        self.opacity = (2.0 * strongest - total).clamp(0.0, 1.0) * self.layer_weight;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_spatial_opacity_uses_unnormalized_weights() {
        let mut sky = SkySelection::default();
        let weights = [(179, 1.0), (210, 0.25)];
        sky.update("main_world", &weights, 0, 2.0);
        assert_eq!((sky.zone, sky.opacity), (Some(179), 0.0));
        sky.update("main_world", &weights, 0, 2.0);
        assert_eq!(sky.opacity, 0.75);
        sky.update("main_world", &[(179, 1.0), (210, 1.0)], 0, 0.0);
        assert_eq!(sky.opacity, 0.0);
        sky.update("main_world", &[(179, 1.0), (210, 0.6), (186, 0.6)], 0, 0.0);
        assert_eq!(sky.opacity, 0.0);
        sky.update("main_world", &[(179, 1.0)], 0, 0.0);
        assert_eq!(sky.opacity, 1.0);
    }

    #[test]
    fn native_layer_switches_only_at_zero_and_can_reverse() {
        let mut sky = SkySelection::default();
        let weights = [(179, 1.0)];
        sky.update("main_world", &weights, 0, 0.0);
        sky.update("main_world", &weights, 0, 2.0);
        sky.update("main_world", &weights, 1, 0.5);
        assert_eq!((sky.layer, sky.opacity), (0, 0.75));
        sky.update("main_world", &weights, 0, 0.5);
        assert_eq!((sky.layer, sky.opacity), (0, 1.0));
        sky.update("main_world", &weights, 1, 1.5);
        assert_eq!((sky.layer, sky.opacity), (0, 0.25));
        sky.update("main_world", &weights, 1, 0.5);
        assert_eq!((sky.layer, sky.opacity), (1, 0.0));
        sky.update("main_world", &weights, 1, 1.0);
        assert_eq!((sky.layer, sky.opacity), (1, 0.5));
        sky.update("main_world", &weights, 1, 0.0);
        assert_eq!(
            sky.opacity, 0.5,
            "clock hour does not advance a frame-time fade"
        );
    }

    #[test]
    fn zone_world_and_unload_discard_previous_assets() {
        let mut sky = SkySelection::default();
        sky.update("main_world", &[(179, 1.0)], 0, 0.0);
        sky.update("main_world", &[(179, 1.0)], 0, 2.0);
        sky.update("main_world", &[(210, 1.0)], 1, 1.0);
        assert_eq!((sky.zone, sky.layer, sky.opacity), (Some(210), 1, 0.0));
        sky.update("main_world", &[(210, 1.0)], 1, 2.0);
        sky.update("other_world", &[(210, 1.0)], 1, 1.0);
        assert_eq!(sky.opacity, 0.0);
        sky.update("other_world", &[], 1, 1.0);
        assert_eq!((sky.zone, sky.opacity), (None, 0.0));
        sky.clear();
        assert_eq!((sky.world, sky.zone, sky.opacity), (None, None, 0.0));
    }
}
