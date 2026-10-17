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
pub fn match_world_camera(
    mut commands: Commands,
    world: Query<(Has<Hdr>, Option<&Msaa>), (With<Camera3d>, Without<UiCamera>)>,
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
        if let Some(mut space) = space {
            if *space != CompositingSpace::Linear {
                *space = CompositingSpace::Linear;
            }
        }
    }
}
