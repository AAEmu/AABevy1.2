//! The third person camera, like ArcheAge's: it orbits a pivot just above the player's head.
//!
//! * hold the left mouse button and drag: orbit the camera around the character
//! * hold the right mouse button and drag: orbit and turn the character with the camera
//! * wheel: zoom, as 1.2's `camera::update_distance` does it ([`zoom_notch`], [`approach_distance`]): a notch moves the wanted
//!   distance by 0.5 x clamp(0.3 x wanted, 1, 20) between 0 and [`MAX_DISTANCE`]; under 1 m it is first person and the own
//!   character is hidden; the camera follows the wanted distance at 5 + gap^1.8 m/s
//!
//! The view at world entry (the real client's spawn camera, `reference/ingame/spawn/camera.json`) stands level with the pivot
//! `PIVOT_HEIGHT` above the feet, vertical field of view 42.75 degrees, at 1.2's start distance by race and gender
//! ([`start_distance`]: 4 m for a Nuian man).
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
/// The distance without a character to take it from (the prewarm camera): 1.2's start for a Nuian man.
pub const DEFAULT_DISTANCE: f32 = 4.0;
/// The far limit, as a running 1.2 shows it: `camera_max_dist` reads 10 there, in the world and at character select, although the
/// code registers 18 (what sets it to 10 is not read, F192).
pub const MAX_DISTANCE: f32 = 10.0;
/// Under this wanted distance the camera is first person and the own character is hidden (`tw_first_person_dist`).
pub const FIRST_PERSON_BELOW: f32 = 1.0;
/// 1.2's camera update caps its frame step at 33 ms (`CameraManager::Update`): the zoom never settles faster after a long frame.
const CAMERA_STEP_CAP: f32 = 0.033;
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
    /// The distance the camera stands at now (m), before a wall shortens it; it follows [`OrbitCamera::wanted`]
    /// ([`approach_distance`]).
    pub distance: f32,
    /// The distance the wheel asks for (m): 0 is first person.
    pub wanted: f32,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        OrbitCamera { yaw: 0.0, pitch: 0.0, distance: DEFAULT_DISTANCE, wanted: DEFAULT_DISTANCE }
    }
}

/// 1.2's distance at world entry (`camera::start_dist_by_race` x2game+0x9c0c58, indexed race x 3 + gender with the server's race
/// and gender numbers): set at once, no easing, never past the far limit. Every other row, gender 0 too, is 10 m.
pub fn start_distance(race: u8, gender: u8) -> f32 {
    use aa_net::model::{gender::*, race::*};
    let start = match (race, gender) {
        (NUIAN, MALE) => 4.0,
        (NUIAN, FEMALE) => 3.2,
        (ELF, MALE) => 3.8,
        (ELF, FEMALE) => 3.4,
        (HARIHARAN, MALE) => 3.6,
        (HARIHARAN, FEMALE) => 3.2,
        (FERRE, MALE) => 4.0,
        (FERRE, FEMALE) => 3.4,
        _ => 10.0,
    };
    f32::min(start, MAX_DISTANCE)
}

/// The wanted distance after one wheel notch (`camera::update_distance` x2game+0x16c2b0, a notch being one frame of the action):
/// `notch` 1 zooms in (1.2's `zoom_in`, an input of -0.5), -1 out. The step is 0.5 x clamp(0.3 x wanted, 1, 20), the result held
/// between 0 and [`MAX_DISTANCE`]; under [`FIRST_PERSON_BELOW`] it snaps to 0 (first person), or to 1 m when zooming out.
pub fn zoom_notch(wanted: f32, notch: f32) -> f32 {
    if notch == 0.0 {
        return wanted;
    }
    let input = -0.5 * notch.signum();
    let next = (wanted + input * (0.3 * wanted).clamp(1.0, 20.0)).clamp(0.0, MAX_DISTANCE);
    match next < FIRST_PERSON_BELOW {
        true if notch < 0.0 => FIRST_PERSON_BELOW,
        true => 0.0,
        false => next,
    }
}

/// The drawn distance moved towards the wanted one for `seconds` (capped at 33 ms, as 1.2's camera update caps its step) at
/// 5 + gap^1.8 m/s, never past it (`tw_zoom_speed` 5, `tw_zoom_speed_power` 1.8).
pub fn approach_distance(current: f32, wanted: f32, seconds: f32) -> f32 {
    let gap = wanted - current;
    if gap == 0.0 || seconds <= 0.0 {
        return current;
    }
    let step = (5.0 + gap.abs().powf(1.8)) * seconds.min(CAMERA_STEP_CAP);
    if gap.abs() <= step { wanted } else { current + gap.signum() * step }
}

/// First person: the wanted distance is under 1 m, and 1.2 hides the own character (`ProcessUnitCamFadeOut`).
pub fn first_person(wanted: f32) -> bool {
    wanted < FIRST_PERSON_BELOW
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

/// The world camera's own vertical field of view (degrees), what a buff's view item widens (`super::cam_fov`): 1.2's spawn
/// camera, or `--cam-fov`.
pub fn base_fov(dev: &DevOptions) -> f32 {
    dev.cam_fov.unwrap_or(FOV_DEG)
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
    data: Res<aa_net::state::Player>,
    existing: Query<(), With<OrbitCamera>>,
    anchors: Query<Entity, With<crate::login::WorldAnchor>>,
    screen: Res<State<Screen>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut images: ResMut<Assets<Image>>,
    dev: Res<DevOptions>,
) {
    let (Ok(mover), true) = (player.single(), existing.is_empty()) else { return };
    // 1.2 sets the wanted and the drawn distance at once when its own unit is bound, by the race and gender the server sent
    let distance = dev.cam_dist.unwrap_or_else(|| {
        let (race, gender) = data.spawn.as_ref().map_or((0, 0), |s| (s.character().race, s.character().gender));
        let start = data.spawn.as_ref().map_or(DEFAULT_DISTANCE, |_| start_distance(race, gender));
        info!("camera start distance {start:.2} m (race {race}, gender {gender})");
        start
    });
    let orbit = OrbitCamera {
        yaw: mover.facing + dev.cam_yaw.unwrap_or(0.0),
        pitch: dev.cam_pitch.unwrap_or(0.0),
        distance,
        wanted: distance,
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

/// 1.2's fixed views (`input::front_camera` and the five other handlers, x2game+0x408450..0x408620): the camera's yaw is an
/// offset from the character's heading, set at once, pitch and distance unchanged.
pub const FRONT_VIEW: f32 = std::f32::consts::PI;
pub const LEFT_VIEW: f32 = std::f32::consts::FRAC_PI_2;
pub const RIGHT_VIEW: f32 = 3.0 * std::f32::consts::FRAC_PI_2;
pub const BACK_VIEW: f32 = 0.0;

/// The offset a fixed-view action sets (Insert `front_camera`, Delete `left_camera`, Home `right_camera`, End `back_camera`).
pub fn fixed_view_offset(action: &str) -> Option<f32> {
    match action {
        "front_camera" => Some(FRONT_VIEW),
        "left_camera" => Some(LEFT_VIEW),
        "right_camera" => Some(RIGHT_VIEW),
        "back_camera" => Some(BACK_VIEW),
        _ => None,
    }
}

/// Page Down (`cycle_camera_clockwise`) steps the **kept** last fixed value by minus a quarter turn, Page Up by plus: from the
/// value kept for it, not from where the mouse has turned the camera since. Which side is which is the action's name (a guess).
pub fn cycle_view(kept: f32, clockwise: bool) -> f32 {
    let step = std::f32::consts::FRAC_PI_2;
    (if clockwise { kept - step } else { kept + step }).rem_euclid(std::f32::consts::TAU)
}

/// The camera's world yaw for a character heading and an offset.
pub fn view_yaw(facing: f32, offset: f32) -> f32 {
    facing + offset
}

/// Mouse orbit / zoom. Runs before the player moves so the character turns with the same input.
pub fn orbit_input(
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    actions: Res<crate::settings::Actions>,
    mut cameras: Query<&mut OrbitCamera>,
    mut window: Query<&mut CursorOptions, With<PrimaryWindow>>,
    over_ui: Option<Res<crate::hud::HudPointerOverUi>>,
    mut notches: Local<f32>,
    player: Query<&Mover, With<LocalPlayer>>,
    // the last fixed view's offset, which Page Down / Page Up step from (`input::g_fixed_view_yaw`)
    mut kept: Local<f32>,
) {
    let Ok(mut orbit) = cameras.single_mut() else { return };
    // the fixed views: the yaw is set at once relative to the character's heading
    if let Ok(mover) = player.single() {
        let mut set = None;
        for action in ["front_camera", "left_camera", "right_camera", "back_camera"] {
            if actions.just_pressed(action, 0) {
                set = fixed_view_offset(action);
            }
        }
        if actions.just_pressed("cycle_camera_clockwise", 0) {
            set = Some(cycle_view(*kept, true));
        }
        if actions.just_pressed("cycle_camera_counter_clockwise", 0) {
            set = Some(cycle_view(*kept, false));
        }
        if let Some(offset) = set {
            *kept = offset;
            orbit.yaw = view_yaw(mover.facing, offset);
        }
    }
    // a drag that began on a HUD window (or the wheel over a list) is the HUD's
    let over_ui = over_ui.is_some_and(|o| o.0);
    let dragging = !over_ui && (mouse.pressed(MouseButton::Left) || mouse.pressed(MouseButton::Right));
    if dragging {
        orbit.yaw -= motion.delta.x * MOUSE_SENSITIVITY;
        orbit.pitch = (orbit.pitch - motion.delta.y * MOUSE_SENSITIVITY).clamp(-1.3, 1.45);
    }
    // a whole notch is one step of 1.2's zoom; the fractions a touchpad sends add up to notches
    if !over_ui {
        *notches += actions.zoom_lines();
    }
    while notches.abs() >= 1.0 {
        let notch = notches.signum();
        let before = orbit.wanted;
        orbit.wanted = zoom_notch(orbit.wanted, notch);
        *notches -= notch;
        if first_person(orbit.wanted) != first_person(before) {
            debug!("camera: {} (wanted {:.2} m)", if first_person(orbit.wanted) { "first person" } else { "third person" }, orbit.wanted);
        }
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
    mut player: Query<(&Mover, &mut Visibility), With<LocalPlayer>>,
    mut camera: Query<(&mut OrbitCamera, &mut Transform)>,
    terrain: Res<TerrainQuery>,
    collision: Res<WorldCollision>,
    lag: Option<Res<super::skill_motion::CameraLag>>,
    mut clip: Local<ArmClip>,
    mut hidden: Local<bool>,
) {
    let (Ok((mover, mut visibility)), Ok((mut orbit, mut transform))) = (player.single_mut(), camera.single_mut()) else { return };
    // the zoom settles at 1.2's pace; in first person the own character is hidden (only this system's own decision is written)
    let distance = approach_distance(orbit.distance, orbit.wanted, time.delta_secs());
    if distance != orbit.distance {
        orbit.distance = distance;
    }
    // a player entity rebuilt while in first person comes back visible: hide it again
    let fp = first_person(orbit.wanted);
    if fp != *hidden || (fp && *visibility != Visibility::Hidden) {
        *hidden = fp;
        *visibility = if fp { Visibility::Hidden } else { Visibility::Inherited };
    }
    // the pivot trails the player through a charge or a blink (`skill_motion`) and catches up
    let pivot_pos = mover.pos + lag.map_or(Vec3::ZERO, |l| l.0);
    if let Some(pose) = chase.and_then(|c| c.0) {
        *transform = pose;
        return;
    }
    if let Some(rel) = rel {
        orbit.yaw = mover.facing + rel.0.to_radians();
        orbit.pitch = 0.0;
    }
    let orbit = &*orbit;
    let mut t = camera_transform(orbit, pivot_pos);
    // the arm from the pivot above the head to the wanted place stops before the first wall
    let pivot_game = pivot_pos + Vec3::Z * PIVOT_HEIGHT;
    let pivot = cry_to_bevy(pivot_game);
    let wanted = bevy_to_cry(t.translation);
    let full = wanted.distance(pivot_game);
    let off = *clip.off.get_or_insert_with(|| std::env::var_os("AA_CAMERA_NO_CLIP").is_some());
    // in first person the camera stands on the pivot: there is no arm to sweep
    let free = if off || full < 1e-3 { full } else { collision.clip_camera(pivot_game, wanted, CAMERA_RADIUS).distance(pivot_game) };
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
        // look at the pivot from the corrected position (on the pivot, first person, the orbit's own rotation stays)
        if t.translation.distance(pivot) > 1e-3 {
            t.look_at(pivot, Vec3::Y);
        }
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

#[cfg(test)]
mod zoom_tests {
    use super::*;

    #[test]
    fn the_start_distance_is_1_2s_table_by_race_and_gender() {
        use aa_net::model::{gender::*, race::*};
        let table = [(NUIAN, 4.0, 3.2), (ELF, 3.8, 3.4), (HARIHARAN, 3.6, 3.2), (FERRE, 4.0, 3.4)];
        for (race, male, female) in table {
            assert_eq!(start_distance(race, MALE), male, "race {race} male");
            assert_eq!(start_distance(race, FEMALE), female, "race {race} female");
        }
        // any other row of the table, gender 0 too, is 10 m, never past the far limit
        assert_eq!(start_distance(2, MALE), 10.0);
        assert_eq!(start_distance(NUIAN, 0), 10.0);
        assert!(start_distance(0, 0) <= MAX_DISTANCE);
    }

    #[test]
    fn a_notch_steps_as_a_running_1_2_showed_it() {
        // every value of 1.2's camera floats sampled notch by notch (2026-10-20): one in from 3.6, out to the far limit, in to
        // first person, out twice
        let mut wanted = zoom_notch(3.6, 1.0);
        assert!((wanted - 3.06).abs() < 0.002, "{wanted}");
        for expected in [3.56, 4.094, 4.708, 5.414, 6.226, 7.160, 8.235, 9.470, 10.0] {
            wanted = zoom_notch(wanted, -1.0);
            assert!((wanted - expected).abs() < 0.002, "{wanted} against 1.2's {expected}");
        }
        assert_eq!(zoom_notch(wanted, -1.0), MAX_DISTANCE, "the far limit holds");
        for expected in [8.5, 7.225, 6.141, 5.220, 4.437, 3.772, 3.206, 2.706, 2.206, 1.706, 1.206, 0.0] {
            wanted = zoom_notch(wanted, 1.0);
            assert!((wanted - expected).abs() < 0.002, "{wanted} against 1.2's {expected}");
        }
        for expected in [1.0, 1.5] {
            wanted = zoom_notch(wanted, -1.0);
            assert!((wanted - expected).abs() < 0.002, "{wanted} against 1.2's {expected}");
        }
    }

    #[test]
    fn first_person_is_a_snap_under_one_metre() {
        // 1.2: 1.206 -> 0 zooming in; out of first person 0 -> 1.0 -> 1.5
        assert_eq!(zoom_notch(1.206, 1.0), 0.0);
        assert_eq!(zoom_notch(0.0, -1.0), 1.0);
        assert_eq!(zoom_notch(1.0, -1.0), 1.5);
        assert_eq!(zoom_notch(0.0, 1.0), 0.0);
        assert!(first_person(0.0) && !first_person(1.0));
    }

    #[test]
    fn from_a_nuian_mans_start_the_sixth_notch_in_is_first_person_and_fourteen_take_it_out_to_ten_metres() {
        let mut wanted = start_distance(aa_net::model::race::NUIAN, aa_net::model::gender::MALE);
        for notch in 1..=6 {
            wanted = zoom_notch(wanted, 1.0);
            assert_eq!(first_person(wanted), notch == 6, "notch {notch}: {wanted}");
        }
        let mut notches = 0;
        while wanted < MAX_DISTANCE {
            wanted = zoom_notch(wanted, -1.0);
            notches += 1;
            if notches == 13 {
                assert!((wanted - 9.310).abs() < 0.002, "{wanted}");
            }
        }
        assert_eq!(notches, 14, "first person (0 m) to the far limit");
    }

    #[test]
    fn the_camera_settles_as_fast_as_1_2s() {
        // 1.2's notch steps and settle times from its run of 2026-10-19 (step m, ms), at 1.2's own 7 ms frames: within 10 %
        for (gap, ms) in [(0.700, 134.0), (0.805, 153.0), (0.926, 170.0), (1.066, 198.0), (0.904, 170.0), (0.769, 147.0)] {
            let (mut distance, mut t, mut last) = (5.0f32, 0.0f32, 5.0f32);
            let wanted = 5.0 - gap;
            while distance != wanted {
                distance = approach_distance(distance, wanted, 0.007);
                assert!(distance <= last && distance >= wanted, "never past the wanted distance");
                last = distance;
                t += 0.007;
            }
            assert!((t * 1000.0 - ms).abs() <= ms * 0.1, "{gap} m settled in {:.0} ms, 1.2 {ms}", t * 1000.0);
        }
    }

    #[test]
    fn each_fixed_view_key_sets_its_offset_from_the_heading_at_once() {
        use std::f32::consts::{FRAC_PI_2, PI};
        assert_eq!(fixed_view_offset("front_camera"), Some(PI));
        assert_eq!(fixed_view_offset("left_camera"), Some(FRAC_PI_2));
        assert_eq!(fixed_view_offset("right_camera"), Some(3.0 * FRAC_PI_2));
        assert_eq!(fixed_view_offset("back_camera"), Some(0.0));
        assert_eq!(fixed_view_offset("zoom"), None);
        // relative to the heading: facing 0.5 rad, the front view looks back at the face
        assert!((view_yaw(0.5, FRONT_VIEW) - (0.5 + PI)).abs() < 1e-6);
        // the character turns with A or D by d and the camera by d too: the offset is kept
        let (facing, yaw) = (0.5f32, view_yaw(0.5, LEFT_VIEW));
        let d = 0.3;
        assert!(((yaw + d) - (facing + d) - LEFT_VIEW).abs() < 1e-6);
    }

    #[test]
    fn the_cycle_keys_step_from_the_last_fixed_value_not_from_the_mouse() {
        use std::f32::consts::{FRAC_PI_2, PI, TAU};
        let close = |a: f32, b: f32| (a - b).rem_euclid(TAU).min((b - a).rem_euclid(TAU)) < 1e-5;
        // Insert, then the mouse drags the camera elsewhere (the kept value stays PI), then Page Down
        let kept = FRONT_VIEW;
        let next = cycle_view(kept, true);
        assert!(close(next, PI - FRAC_PI_2));
        // three Page Downs from the back view go round: 3pi/2, pi, pi/2
        let mut v = BACK_VIEW;
        let mut seen = Vec::new();
        for _ in 0..3 {
            v = cycle_view(v, true);
            seen.push(v);
        }
        assert!(close(seen[0], 3.0 * FRAC_PI_2) && close(seen[1], PI) && close(seen[2], FRAC_PI_2));
        assert!(close(cycle_view(BACK_VIEW, false), FRAC_PI_2));
    }

    #[test]
    fn a_long_frame_moves_the_camera_no_further_than_33_ms() {
        assert_eq!(approach_distance(4.0, 10.0, 0.25), approach_distance(4.0, 10.0, 0.033));
        assert_eq!(approach_distance(4.0, 4.0, 0.1), 4.0);
        assert_eq!(approach_distance(4.0, 3.9, 0.033), 3.9, "a small gap closes in one frame");
    }
}
