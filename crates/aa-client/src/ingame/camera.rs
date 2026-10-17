//! The third person camera, like ArcheAge's: it orbits a pivot just above the player's head.
//!
//! * hold the left mouse button and drag: orbit the camera around the character
//! * hold the right mouse button and drag: orbit and turn the character with the camera
//! * wheel: zoom
//!
//! The default view (the real client's spawn camera, `reference/ingame/spawn/camera.json`) is 4 m
//! behind the character, level with the pivot `PIVOT_HEIGHT` above its feet, vertical field of
//! view 42.75 degrees.

use aa_ui::UiBackgroundCamera;
use aa_ui::render::UiCamera;
use aa_world::{TerrainQuery, cry_to_bevy};
use bevy::camera::{Hdr, RenderTarget};
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use bevy::render::view::Msaa;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};

use super::dev::DevOptions;
use super::player::{LocalPlayer, Mover, height_or};
use crate::Screen;

/// Height of the camera pivot above the character's feet (m): the real client's camera sits 1.754 m
/// above the ground at pitch 0.
pub const PIVOT_HEIGHT: f32 = 1.754;
pub const DEFAULT_DISTANCE: f32 = 4.0;
const MIN_DISTANCE: f32 = 0.8;
const MAX_DISTANCE: f32 = 30.0;
const FOV_DEG: f32 = 42.75;
/// Radians per mouse count.
const MOUSE_SENSITIVITY: f32 = 0.004;

#[derive(Component)]
pub struct OrbitCamera {
    /// Heading in game terms: radians counter-clockwise from north (the character's yaw).
    pub yaw: f32,
    /// Radians, positive looks up.
    pub pitch: f32,
    pub distance: f32,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        OrbitCamera { yaw: 0.0, pitch: 0.0, distance: DEFAULT_DISTANCE }
    }
}

/// The world camera still draws into an image behind the loading screen ([`show_world`] puts it on the window).
#[derive(Component)]
pub struct BehindLoading;

/// Spawns the camera behind the player (the world streams around the first `Camera3d`). On the loading screen it renders into
/// an image the size of the window instead: the world draws, compiles its pipelines and loads its models behind the loading
/// UI, and the screen switches to the game when that is done.
#[allow(clippy::too_many_arguments)]
pub fn spawn_camera(
    mut commands: Commands,
    player: Query<&Mover, With<LocalPlayer>>,
    existing: Query<(), With<OrbitCamera>>,
    anchors: Query<Entity, With<crate::login::WorldAnchor>>,
    screen: Res<State<Screen>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut images: ResMut<Assets<Image>>,
    dev: Res<DevOptions>,
) {
    let (Ok(mover), true) = (player.single(), existing.is_empty()) else { return };
    let orbit = OrbitCamera {
        yaw: mover.facing + dev.cam_yaw.unwrap_or(0.0),
        pitch: dev.cam_pitch.unwrap_or(0.0),
        distance: dev.cam_dist.unwrap_or(DEFAULT_DISTANCE),
    };
    let transform = camera_transform(&orbit, mover.pos);
    let mut camera = commands.spawn((
        Name::new("camera"),
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection { fov: FOV_DEG.to_radians(), near: 0.2, far: 20000.0, ..default() }),
        transform,
        orbit,
    ));
    if *screen.get() == Screen::Loading
        && let Ok(window) = windows.single()
    {
        let image = Image::new_target_texture(window.physical_width(), window.physical_height(), bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb, None);
        camera.insert((RenderTarget::Image(images.add(image).into()), BehindLoading));
    }
    // this camera is the world's focus from now on (the anchor kept the world off the login stage camera)
    for e in &anchors {
        commands.entity(e).despawn();
    }
}

/// Puts the world camera on the window once the UI cameras agree with it (HDR and MSAA, see `hud::cameras`; cameras that
/// disagree get separate main textures and the UI would replace the world), or after a second.
pub fn show_world(
    mut commands: Commands,
    time: Res<Time>,
    mut waited: Local<f32>,
    hidden: Query<(Entity, Has<Hdr>, Option<&Msaa>), (With<BehindLoading>, With<OrbitCamera>)>,
    ui: Query<(Has<Hdr>, Option<&Msaa>), (With<UiCamera>, Without<UiBackgroundCamera>)>,
) {
    let Ok((entity, hdr, msaa)) = hidden.single() else {
        *waited = 0.0;
        return;
    };
    *waited += time.delta_secs();
    let msaa = msaa.copied().unwrap_or_default();
    let agree = ui.iter().all(|(ui_hdr, ui_msaa)| ui_hdr == hdr && ui_msaa.copied() == Some(msaa));
    if agree || *waited > 1.0 {
        commands.entity(entity).insert(RenderTarget::default()).remove::<BehindLoading>();
        info!("world camera on the window (UI cameras agree: {agree})");
    }
}

/// Camera pose for an orbit around the pivot above `pos` (game coordinates of the feet).
pub fn camera_transform(orbit: &OrbitCamera, pos: Vec3) -> Transform {
    let pivot = cry_to_bevy(pos + Vec3::Z * PIVOT_HEIGHT);
    let rotation = Quat::from_rotation_y(orbit.yaw) * Quat::from_rotation_x(orbit.pitch);
    Transform { translation: pivot + rotation * Vec3::Z * orbit.distance, rotation, scale: Vec3::ONE }
}

/// Mouse orbit / zoom. Runs before the player moves so the character turns with the same input.
pub fn orbit_input(
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut cameras: Query<&mut OrbitCamera>,
    mut window: Query<&mut CursorOptions, With<PrimaryWindow>>,
    over_ui: Option<Res<crate::hud::HudPointerOverUi>>,
) {
    let Ok(mut orbit) = cameras.single_mut() else { return };
    // a drag that began on a HUD window (or the wheel over a list) is the HUD's
    let over_ui = over_ui.is_some_and(|o| o.0);
    let dragging = !over_ui && (mouse.pressed(MouseButton::Left) || mouse.pressed(MouseButton::Right));
    if dragging {
        orbit.yaw -= motion.delta.x * MOUSE_SENSITIVITY;
        orbit.pitch = (orbit.pitch - motion.delta.y * MOUSE_SENSITIVITY).clamp(-1.3, 1.45);
    }
    if scroll.delta.y != 0.0 && !over_ui {
        let lines = match scroll.unit {
            MouseScrollUnit::Line => scroll.delta.y,
            MouseScrollUnit::Pixel => scroll.delta.y / 40.0,
        };
        orbit.distance = (orbit.distance * 0.88f32.powf(lines)).clamp(MIN_DISTANCE, MAX_DISTANCE);
    }
    // the pointer disappears and stays put while a button drags the view, like the real client
    if let Ok(mut cursor) = window.single_mut() {
        let want = if dragging { (CursorGrabMode::Locked, false) } else { (CursorGrabMode::None, true) };
        if cursor.grab_mode != want.0 || cursor.visible != want.1 {
            cursor.grab_mode = want.0;
            cursor.visible = want.1;
        }
    }
}

/// Puts the camera on its orbit around the player; keeps it above the ground.
pub fn follow_player(
    player: Query<&Mover, With<LocalPlayer>>,
    mut camera: Query<(&OrbitCamera, &mut Transform)>,
    terrain: Res<TerrainQuery>,
) {
    let (Ok(mover), Ok((orbit, mut transform))) = (player.single(), camera.single_mut()) else { return };
    let mut t = camera_transform(orbit, mover.pos);
    // not below the terrain (bevy y = game z)
    let ground = height_or(&terrain, t.translation.x, -t.translation.z, mover.pos.z) + 0.35;
    if t.translation.y < ground {
        t.translation.y = ground;
        // look at the pivot from the corrected position
        let pivot = cry_to_bevy(mover.pos + Vec3::Z * PIVOT_HEIGHT);
        t.look_at(pivot, Vec3::Y);
    }
    *transform = t;
}
