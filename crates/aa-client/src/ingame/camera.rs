//! The third person camera, like ArcheAge's: it orbits a pivot just above the player's head.
//!
//! * hold the left mouse button and drag: orbit the camera around the character
//! * hold the right mouse button and drag: orbit and turn the character with the camera
//! * wheel: zoom
//!
//! The default view (the real client's spawn camera, `reference/ingame/spawn/camera.json`) is 4 m
//! behind the character, level with the pivot `PIVOT_HEIGHT` above its feet, vertical field of
//! view 42.75 degrees.
//!
//! Walls: the arm from the pivot to the camera is swept with a sphere against the terrain and the world's big structures
//! ([`WorldCollision::clip_camera`]); the camera stops before the first hit at once and comes back out smoothly
//! ([`ARM_RETURN`]) when the hit clears.
//!
//! Blows and spells shake it: [`CameraShake`] holds the shakes of the effect data (`fx_shake_cameras`), `follow_player` adds
//! their sum to the pose.

use aa_ui::UiBackgroundCamera;
use aa_ui::render::UiCamera;
use aa_world::{TerrainQuery, WorldCollision, bevy_to_cry, cry_to_bevy};
use bevy::camera::{Hdr, RenderTarget};
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::pbr::{ScreenSpaceTransmission, ScreenSpaceTransmissionQuality};
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
/// Radius of the sphere the camera arm is swept with (m): more than the near plane (0.2) reaches.
const CAMERA_RADIUS: f32 = 0.3;
/// How fast the camera comes back out after a wall let go of it (1 / s of an exponential approach).
const ARM_RETURN: f32 = 5.0;
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

/// Screen space transmission, which aa-fx's refraction particles read the scene through. aa-fx puts it on every camera the first time
/// a refraction effect shows up (`ensure_transmission`), and the component is part of what a render pipeline is keyed on (the view
/// layout), so a camera that got it in the middle of the game would make every material compile again. The cameras that draw the
/// world (this one and the prewarm camera) have it from the start, exactly as aa-fx would give it; without a transmissive item it
/// costs nothing (no copy of the scene, no texture).
pub fn transmission() -> ScreenSpaceTransmission {
    ScreenSpaceTransmission { steps: 1, quality: ScreenSpaceTransmissionQuality::Low }
}

/// The in-game camera's lens (also the prewarm camera's, `prewarm`).
pub fn projection() -> Projection {
    projection_fov(FOV_DEG)
}

/// The lens with another vertical field of view (degrees): `--cam-fov`, to line a shot up with a capture of the real client's `cl_fov`.
fn projection_fov(degrees: f32) -> Projection {
    Projection::Perspective(PerspectiveProjection { fov: degrees.to_radians(), near: 0.2, far: 20000.0, ..default() })
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
        dev.cam_fov.map_or_else(projection, projection_fov),
        transmission(),
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

#[allow(clippy::type_complexity)]
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
    actions: Res<crate::settings::Actions>,
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
    if actions.zoom_lines() != 0.0 && !over_ui {
        let lines = actions.zoom_lines();
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

/// How strong the shakes of the effect data are here: the data is made for a camera that sits in the head of the character,
/// this one is 4 m behind it, so a full shake would throw the whole scene around.
const SHAKE_SCALE: f32 = 0.5;
/// The most a shake may move (m) or turn (degrees) the camera, whatever the data says.
const SHAKE_MAX_SHIFT: f32 = 0.2;
const SHAKE_MAX_ANGLE: f32 = 2.0;
/// Shakes running at once; the oldest goes when there are more.
const SHAKES: usize = 6;

/// The camera shakes that are running. A shake is a wave that alternates every `step` seconds (the data's `frequency`) between
/// two extremes, each a little off by `randomness`, and fades out linearly over `duration`.
#[derive(Resource, Default)]
pub struct CameraShake {
    active: Vec<Running>,
    /// Counts the shakes, so that two of them do not wobble alike.
    count: u32,
}

struct Running {
    ang: Vec3,
    shift: Vec3,
    step: f32,
    randomness: f32,
    duration: f32,
    age: f32,
    seed: u32,
}

/// A number in 0..1 from `x`.
fn hash01(mut x: u32) -> f32 {
    x = x.wrapping_mul(0x9E37_79B1) ^ (x >> 15);
    x = x.wrapping_mul(0x85EB_CA6B) ^ (x >> 13);
    x = x.wrapping_mul(0xC2B2_AE35) ^ (x >> 16);
    (x & 0xFFFF) as f32 / 65535.0
}

impl Running {
    /// The wave (-1..1) of one axis at the shake's age.
    fn wave(&self, axis: u32) -> f32 {
        let x = self.age / self.step;
        let k = x.floor() as u32;
        let sample = |k: u32| {
            let sign = if (k + axis).is_multiple_of(2) { 1.0 } else { -1.0 };
            sign * (1.0 - self.randomness * hash01(self.seed ^ k.wrapping_mul(31) ^ axis.wrapping_mul(0x51)))
        };
        let f = x - x.floor();
        let smooth = f * f * (3.0 - 2.0 * f);
        sample(k) + (sample(k + 1) - sample(k)) * smooth
    }
}

impl CameraShake {
    /// Starts a shake: `ang` in degrees and `shift` in metres (cry axes, the data's amplitudes), `frequency` the seconds
    /// between the two extremes of the wave.
    pub fn add(&mut self, ang: Vec3, shift: Vec3, frequency: f32, randomness: f32, duration: f32) {
        if duration <= 0.0 {
            return;
        }
        if self.active.len() >= SHAKES {
            self.active.remove(0);
        }
        self.count = self.count.wrapping_add(1);
        self.active.push(Running {
            ang,
            shift,
            step: frequency.max(0.03),
            randomness: randomness.clamp(0.0, 1.0),
            duration,
            age: 0.0,
            seed: self.count.wrapping_mul(0x2545_F491),
        });
    }

    /// Moves the shakes on by `dt` seconds; returns the camera's displacement in its own axes and the turn.
    pub fn advance(&mut self, dt: f32) -> (Vec3, Quat) {
        let mut shift = Vec3::ZERO;
        let mut ang = Vec3::ZERO;
        for s in &mut self.active {
            s.age += dt;
            let fade = (1.0 - s.age / s.duration).max(0.0);
            let w = |axis| s.wave(axis) * fade * SHAKE_SCALE;
            shift += Vec3::new(s.shift.x * w(0), s.shift.y * w(1), s.shift.z * w(2));
            ang += Vec3::new(s.ang.x * w(3), s.ang.y * w(4), s.ang.z * w(5));
        }
        self.active.retain(|s| s.age < s.duration);
        // cry x is right, y forward (into the screen), z up; a turn about x tilts, about y rolls, about z yaws
        let shift = (Vec3::new(shift.x, shift.z, -shift.y)).clamp_length_max(SHAKE_MAX_SHIFT);
        let ang = ang.clamp(Vec3::splat(-SHAKE_MAX_ANGLE), Vec3::splat(SHAKE_MAX_ANGLE)).map(f32::to_radians);
        (shift, Quat::from_euler(EulerRot::YXZ, ang.z, ang.x, ang.y))
    }

    pub fn is_idle(&self) -> bool {
        self.active.is_empty()
    }
}

/// Dev: `AA_COMBAT_CHASE=1` puts the camera behind the projectile in flight (the combat code sets the pose), to look at the arrow
/// or the spell from a few metres without guessing where the fight will be.
#[derive(Resource, Default)]
pub struct Chase(pub Option<Transform>);

/// Dev: `AA_COMBAT_CAM_REL=-147` keeps the camera at that heading from the way the player faces (degrees, level; the real client's
/// `right_camera` is -90 until the character turns to its target), to line a fight up with a capture of the real client.
#[derive(Resource)]
pub struct CamRel(pub f32);

/// How far the wall (or the ground) has shortened the camera arm (m), the part of it that is still being given back.
#[derive(Default)]
pub struct ArmClip {
    shortened: f32,
    /// A wall holds the camera (for the log line when that starts and stops).
    held: bool,
    /// `AA_CAMERA_NO_CLIP=1`: the walls do not stop the camera (for before / after shots), read on the first frame.
    off: Option<bool>,
}

impl ArmClip {
    /// Takes the shortening the walls ask for this frame: more of it at once, less of it smoothly over `dt` seconds.
    fn update(&mut self, wanted: f32, dt: f32) -> f32 {
        if wanted >= self.shortened {
            self.shortened = wanted;
        } else {
            self.shortened += (wanted - self.shortened) * (1.0 - (-ARM_RETURN * dt).exp());
        }
        self.shortened
    }
}

/// Puts the camera on its orbit around the player; stops it at walls and keeps it above the ground.
#[allow(clippy::too_many_arguments)]
pub fn follow_player(
    time: Res<Time>,
    mut shake: ResMut<CameraShake>,
    chase: Option<Res<Chase>>,
    rel: Option<Res<CamRel>>,
    player: Query<&Mover, With<LocalPlayer>>,
    mut camera: Query<(&mut OrbitCamera, &mut Transform)>,
    terrain: Res<TerrainQuery>,
    collision: Res<WorldCollision>,
    mut clip: Local<ArmClip>,
) {
    let (Ok(mover), Ok((mut orbit, mut transform))) = (player.single(), camera.single_mut()) else { return };
    if let Some(pose) = chase.and_then(|c| c.0) {
        *transform = pose;
        return;
    }
    if let Some(rel) = rel {
        orbit.yaw = mover.facing + rel.0.to_radians();
        orbit.pitch = 0.0;
    }
    let orbit = &*orbit;
    let mut t = camera_transform(orbit, mover.pos);
    // the arm from the pivot above the head to the wanted place stops before the first wall
    let pivot_game = mover.pos + Vec3::Z * PIVOT_HEIGHT;
    let pivot = cry_to_bevy(pivot_game);
    let wanted = bevy_to_cry(t.translation);
    let full = wanted.distance(pivot_game);
    let off = *clip.off.get_or_insert_with(|| std::env::var_os("AA_CAMERA_NO_CLIP").is_some());
    let free = if off { full } else { collision.clip_camera(pivot_game, wanted, CAMERA_RADIUS).distance(pivot_game) };
    let wanted_cut = (full - free).max(0.0);
    let shortened = clip.update(wanted_cut, time.delta_secs());
    if (wanted_cut > 0.02) != clip.held {
        clip.held = wanted_cut > 0.02;
        if clip.held {
            info!("camera: a wall stops the arm at {free:.2} m of {full:.2} m (player {:.1}, {:.1}, {:.2})", mover.pos.x, mover.pos.y, mover.pos.z);
        } else {
            debug!("camera: the arm is free again ({full:.2} m)");
        }
    }
    if shortened > 1e-4 && full > 1e-4 {
        t.translation = pivot + (t.translation - pivot) * ((full - shortened).max(0.0) / full);
    }
    // not below the terrain (bevy y = game z), whatever the sweep did on a steep slope
    let ground = height_or(&terrain, t.translation.x, -t.translation.z, mover.pos.z) + 0.15;
    if t.translation.y < ground {
        t.translation.y = ground;
        // look at the pivot from the corrected position
        t.look_at(pivot, Vec3::Y);
    }
    if !shake.is_idle() {
        let (shift, turn) = shake.advance(time.delta_secs());
        t.translation += t.rotation * shift;
        t.rotation *= turn;
    }
    *transform = t;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shake_moves_the_camera_and_dies_out() {
        let mut shake = CameraShake::default();
        assert!(shake.is_idle());
        shake.add(Vec3::new(2.0, 4.0, 2.0), Vec3::new(0.0, 0.2, 0.0), 0.01, 0.3, 1.3);
        let mut moved = 0.0f32;
        let mut t = 0.0;
        while t < 1.0 {
            let (shift, turn) = shake.advance(1.0 / 60.0);
            assert!(shift.length() <= SHAKE_MAX_SHIFT + 1e-6);
            assert!(turn.is_normalized());
            moved = moved.max(shift.length()).max(turn.angle_between(Quat::IDENTITY));
            t += 1.0 / 60.0;
        }
        assert!(moved > 0.001, "{moved}");
        assert!(!shake.is_idle());
        for _ in 0..30 {
            shake.advance(1.0 / 60.0);
        }
        assert!(shake.is_idle());
        assert_eq!(shake.advance(0.1), (Vec3::ZERO, Quat::IDENTITY));
    }

    #[test]
    fn a_huge_shake_is_held_in() {
        let mut shake = CameraShake::default();
        shake.add(Vec3::splat(40.0), Vec3::splat(5.0), 0.05, 0.0, 2.0);
        for _ in 0..60 {
            let (shift, turn) = shake.advance(1.0 / 60.0);
            assert!(shift.length() <= SHAKE_MAX_SHIFT + 1e-6);
            assert!(turn.angle_between(Quat::IDENTITY) < 0.2);
        }
    }

    #[test]
    fn only_a_few_shakes_run_at_once() {
        let mut shake = CameraShake::default();
        for _ in 0..20 {
            shake.add(Vec3::ONE, Vec3::ONE, 0.05, 0.1, 1.0);
        }
        assert_eq!(shake.active.len(), SHAKES);
        shake.add(Vec3::ONE, Vec3::ONE, 0.05, 0.1, 0.0);
        assert_eq!(shake.active.len(), SHAKES);
    }
}
