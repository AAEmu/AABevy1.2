//! The UI cameras over the 3D world.
//!
//! The world camera renders HDR (aa-world's post processing) with MSAA off; Bevy gives cameras on
//! one window a shared main texture only when their HDR and MSAA settings agree, otherwise the UI
//! camera's output replaces the world. So in game the UI cameras take the world camera's settings,
//! and blend in linear space (a stack with a `Camera3d` cannot composite in gamma space).

use aa_ui::render::UiCamera;
use aa_ui::UiBackgroundCamera;
use bevy::camera::{CompositingSpace, Hdr};
use bevy::prelude::*;
use bevy::render::view::Msaa;

/// Makes the UI cameras agree with the 3D world camera (every frame in game, cheap when they do).
#[allow(clippy::type_complexity)]
pub fn match_world_camera(
    mut commands: Commands,
    world: Query<(Has<Hdr>, Option<&Msaa>), (With<Camera3d>, Without<UiCamera>, Without<crate::hud::ModelViewCamera>)>,
    mut ui: Query<(Entity, Has<Hdr>, Option<&Msaa>, Option<&mut CompositingSpace>, Has<UiBackgroundCamera>, &mut Camera), With<UiCamera>>,
) {
    let Some((hdr, msaa)) = world.iter().next() else { return };
    let msaa = msaa.copied().unwrap_or_default();
    for (entity, ui_hdr, ui_msaa, space, background, mut camera) in &mut ui {
        if background {
            // the painted login backgrounds' camera has nothing to draw in game
            if camera.is_active {
                camera.is_active = false;
            }
            continue;
        }
        if ui_hdr != hdr {
            if hdr {
                commands.entity(entity).insert(Hdr);
            } else {
                commands.entity(entity).remove::<Hdr>();
            }
        }
        if ui_msaa.copied() != Some(msaa) {
            commands.entity(entity).insert(msaa);
        }
        if let Some(mut space) = space
            && *space != CompositingSpace::Linear
        {
            *space = CompositingSpace::Linear;
        }
    }
}

/// `--hud-fov DEG`: the world camera's vertical field of view, to match the combat captures of
/// `reference/ingame/combat_*` (`cl_fov` 75 = 71.25; the camera module's own lens is the client's 42.75).
#[derive(Resource)]
pub struct DevFov(pub Option<f32>, pub bool);

impl DevFov {
    pub fn from_args() -> DevFov {
        let args: Vec<String> = std::env::args().collect();
        DevFov(args.iter().position(|a| a == "--hud-fov").and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()), args.iter().any(|a| a == "--hud-side-cam"))
    }
}

/// Applies [`DevFov`] to the world camera; `--hud-side-cam` also keeps it on the left side of the player, as the
/// captures' `right_camera` does (`--cam-yaw -90` is only right until the player turns).
pub fn dev_fov(
    fov: Res<DevFov>,
    mut cameras: Query<(&mut Projection, &mut crate::ingame::camera::OrbitCamera)>,
    bridge: NonSend<super::api::HudBridge>,
) {
    if fov.1 {
        let facing = bridge.0.borrow().yaw;
        for (_, mut orbit) in &mut cameras {
            orbit.yaw = facing - std::f32::consts::FRAC_PI_2;
        }
    }
    let Some(deg) = fov.0 else { return };
    for (mut projection, _) in &mut cameras {
        if let Projection::Perspective(p) = &mut *projection
            && (p.fov - deg.to_radians()).abs() > 1e-4
        {
            p.fov = deg.to_radians();
        }
    }
}
