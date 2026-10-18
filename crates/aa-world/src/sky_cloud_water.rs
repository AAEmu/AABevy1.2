//! Zone-cloud water halves, from 3012E8A0 -> 300B42A0 -> 300B4410 / 301121C0.
//! The stored centre is not the camera-relative rendered centre.
use bevy::{
    prelude::*,
    render::{
        Extract, ExtractSchedule, RenderApp,
        batching::NoAutomaticBatching,
        extract_component::{ExtractComponent, ExtractComponentPlugin},
    },
};

/// Zone-cloud nodes use native stored Cry (4000,4000,100), independently of their pose.
/// Ordinary, unmirrored views may draw them before the water colour copy.
#[derive(Component, Clone, Copy, ExtractComponent)]
#[extract_app(RenderApp)]
#[require(NoAutomaticBatching)]
pub struct CloudWaterPass;

#[derive(Resource, Default)]
pub(super) struct OceanLevel(pub Option<f32>);

/// Native normal-pass (pass ID zero) strict f32 expression. 301121C0 reads node+68,
/// initialized to 100 by 3012E8A0, rather than the rendered camera+authored height.
pub(super) fn after_water(camera_height: f32, ocean_level: f32) -> bool {
    ((camera_height - ocean_level) * 0.5) * (100.0 - ocean_level) > 0.0
}

pub(super) fn build(app: &mut App) {
    app.add_plugins(ExtractComponentPlugin::<CloudWaterPass>::default());
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render
            .init_resource::<OceanLevel>()
            .add_systems(ExtractSchedule, extract_ocean);
    }
}

// GetWaterLevel(nullptr) returns the global ocean, not a local volume. Only use
// the existing global ocean producer; an absent/ambiguous source leaves clouds late.
fn extract_ocean(
    mut ocean: ResMut<OceanLevel>,
    source: Extract<Query<&GlobalTransform, With<crate::sky::OceanPlane>>>,
) {
    ocean.0 = source
        .single()
        .ok()
        .map(|transform| transform.translation().y)
        .filter(|height| height.is_finite());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_x86_normal_pass_goldens_use_stored_height_and_strict_boundary() {
        // Executed directly in the original 301121C0 x86/SSE code, pass ID zero.
        // Full 36-case normal/recursive/stored-vs-rendered controls are recorded in
        // ignored night/cloud-water-native-oracles.json.
        for (ocean, camera, expected) in [
            (80.0, 60.0, false),
            (80.0, 80.0, false),
            (80.0, 100.0, true),
            (100.0, 80.0, false),
            (100.0, 100.0, false),
            (100.0, 120.0, false),
            (120.0, 100.0, true),
            (120.0, 120.0, false),
            (120.0, 140.0, false),
        ] {
            assert_eq!(after_water(camera, ocean), expected, "{camera}/{ocean}");
        }
        // Main-world oceanLevel=100 exactly equals the stored cloud height.
        // Using the visible camera-relative centre instead would misclassify it.
        assert!(!after_water(150.0, 100.0));
        assert!(((150.0 - 100.0) * 0.5) * (550.0 - 100.0) > 0.0);
    }
}
