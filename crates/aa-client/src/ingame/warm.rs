//! What the world behind the loading screen has got done (`login::loading` waits for it).
//!
//! The player, the camera and the units near the spawn are built while the loading screen is up (the camera draws into an
//! image, see `camera::spawn_camera`), so the first frames of the game screen do not stall on first-use work: pipeline
//! compilation, model and texture uploads, the first skinned meshes.

use aa_net::state::{Doodads, Units};
use bevy::prelude::*;

use super::camera::OrbitCamera;
use super::player::LocalPlayer;
use super::units::Views;

#[derive(Resource, Default, Debug)]
pub struct Warm {
    /// The player character is in the world.
    pub player: bool,
    /// Frames since the world camera exists.
    pub camera_frames: u32,
    /// Units and doodads the server has announced so far.
    pub announced: usize,
    /// Units and doodads in range that are not in the world yet (waiting for a worker, being prepared, waiting to spawn).
    pub outstanding: usize,
}

pub fn update(
    mut warm: ResMut<Warm>,
    player: Query<(), With<LocalPlayer>>,
    camera: Query<(), With<OrbitCamera>>,
    units: Res<Units>,
    doodads: Res<Doodads>,
    views: Res<Views>,
) {
    let (player, camera) = (!player.is_empty(), !camera.is_empty());
    let (announced, outstanding) = (units.by_id.len() + doodads.by_id.len(), views.outstanding());
    if warm.player != player {
        warm.player = player;
    }
    if camera {
        warm.camera_frames += 1;
    }
    if warm.announced != announced {
        warm.announced = announced;
    }
    if warm.outstanding != outstanding {
        warm.outstanding = outstanding;
    }
}
