//! The 3D side of the login screens: the camera of a real-client capture, the login stage lighting
//! and a slot where a character stands. Used by character select and by character create.
//!
//! The real client draws the painted background (UI layer `background`), then the 3D scene of the
//! login level, then the other UI. Here that is three cameras on the one window: the UI's
//! background camera (order 0), this module's [`StageCamera`] (order [`STAGE_CAMERA_ORDER`]) and the
//! UI camera (order 10); see [`crate::login::ui_config`].
//!
//! ## Isolation from the world
//!
//! The in-game world (sky dome, sun, ocean, terrain) lives in the same app. Everything of the stage
//! sits on [`RenderLayers`] layer [`STAGE_LAYER`] (camera, lights, characters) so the world's
//! entities and lights do not show up in it and the stage's lights do not touch the world. Spawn
//! characters through [`spawn_subject`]: it marks them as [`StageSubject`] and
//! [`apply_stage_layers`] puts the whole mesh hierarchy on the stage layer.
//!
//! ## Coordinates
//!
//! A capture's `camera.json` is in CryEngine space (x east, y north, z up, metres, degrees).
//! [`cry_to_stage`] maps that into Bevy space shifted by [`STAGE_ORIGIN`] (the spot of the
//! character select slot), so all stage numbers stay small. A [`StagePose`] is a camera of a
//! capture; [`StagePose::solve_feet`] finds where a figure stands from the pixel of its feet and
//! the row of its head in the capture, which is how [`SELECT_FEET`] was found.
//!
//! ```ignore
//! // OnEnter(your screen)
//! let stage = spawn_stage(&mut commands, &mut cry, &StagePose::CHARACTER_SELECT, &StageLighting::login());
//! let who = spawn_subject(&mut commands, &mut cry, &clips, assets, &StageSlot::select(race, gender), stage.root);
//! // later: move the camera to another pose (create steps)
//! StagePose::from_json(text).unwrap().apply(&mut transform, &mut projection);
//! // OnExit: commands.entity(stage.root).despawn();
//! ```

use aa_anim::{Animator, CharacterAssets, ClipLibrary, Gender, Race, spawn_character};
use aa_cry::scene::CryAssets;
use aa_cry::stage::{StageLighting, login_color_grading};
use bevy::camera::visibility::RenderLayers;
use bevy::camera::{ClearColorConfig, CompositingSpace};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;

/// Render layer of everything in the login stage.
pub const STAGE_LAYER: usize = 1;
/// Camera order of the stage camera: above the UI's background camera (0), below the UI camera (10).
pub const STAGE_CAMERA_ORDER: isize = 5;

/// CryEngine position of the character select slot (feet of the character), the origin of stage space.
/// Solved from `reference/character_select/1_character` (feet pixel 400,1015, head row 187, 1.9 m).
pub const SELECT_FEET: Vec3 = Vec3::new(727.1069, 563.8529, 296.6709);
/// The origin of stage space (CryEngine coordinates).
pub const STAGE_ORIGIN: Vec3 = SELECT_FEET;

/// CryEngine (Z up) to Bevy (Y up) for points and directions.
pub fn cry_vec(v: Vec3) -> Vec3 {
    Vec3::new(v.x, v.z, -v.y)
}

/// A CryEngine position in stage space (Bevy axes, shifted by [`STAGE_ORIGIN`]).
pub fn cry_to_stage(p: Vec3) -> Vec3 {
    cry_vec(p - STAGE_ORIGIN)
}

/// A camera of a real-client capture (`camera.json`): CryEngine space, degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StagePose {
    pub pos: Vec3,
    /// Unit view direction.
    pub dir: Vec3,
    /// Roll about the view direction (`angles_deg.y`).
    pub roll_deg: f32,
    /// Vertical field of view.
    pub fov_deg: f32,
}

impl StagePose {
    /// `reference/character_select/*/camera.json`.
    pub const CHARACTER_SELECT: StagePose = StagePose {
        pos: Vec3::new(725.677917, 568.358276, 298.23999),
        dir: Vec3::new(0.0337297954, -0.994942665, -0.0946129188),
        roll_deg: -1.27,
        fov_deg: 30.0,
    };

    /// Parse the text of a `camera.json` (`pos`, `dir`, `angles_deg.y`, `fov_deg`).
    pub fn from_json(text: &str) -> Option<StagePose> {
        fn number(text: &str, section: Option<&str>, key: &str) -> Option<f32> {
            let text = match section {
                Some(s) => &text[text.find(&format!("\"{s}\""))?..],
                None => text,
            };
            let at = text.find(&format!("\"{key}\""))? + key.len() + 2;
            let rest = text[at..].trim_start_matches([':', ' ', '\n']);
            let end = rest.find([',', '\n', '}'])?;
            rest[..end].trim().parse().ok()
        }
        Some(StagePose {
            pos: Vec3::new(number(text, Some("pos"), "x")?, number(text, Some("pos"), "y")?, number(text, Some("pos"), "z")?),
            dir: Vec3::new(number(text, Some("dir"), "x")?, number(text, Some("dir"), "y")?, number(text, Some("dir"), "z")?).try_normalize()?,
            roll_deg: number(text, Some("angles_deg"), "y")?,
            fov_deg: number(text, None, "fov_deg")?,
        })
    }

    /// Camera axes (forward, right, up) in CryEngine space.
    pub fn axes(&self) -> (Vec3, Vec3, Vec3) {
        let f = self.dir;
        let right0 = f.cross(Vec3::Z).normalize();
        let up0 = right0.cross(f);
        // roll about the forward axis (right handed): the up vector tilts to the right
        let q = Quat::from_axis_angle(f, self.roll_deg.to_radians());
        (f, q * right0, q * up0)
    }

    /// Pixel `(u, v)` of a CryEngine point in a 1920x1080 capture.
    pub fn project(&self, p: Vec3) -> Vec2 {
        let (f, r, u) = self.axes();
        let focal = 540.0 / (self.fov_deg.to_radians() / 2.0).tan();
        let d = p - self.pos;
        let z = d.dot(f);
        Vec2::new(960.0 + focal * d.dot(r) / z, 540.0 - focal * d.dot(u) / z)
    }

    /// The ray through a pixel of a 1920x1080 capture.
    pub fn ray(&self, uv: Vec2) -> Vec3 {
        let (f, r, u) = self.axes();
        let focal = 540.0 / (self.fov_deg.to_radians() / 2.0).tan();
        (f + r * ((uv.x - 960.0) / focal) + u * (-(uv.y - 540.0) / focal)).normalize()
    }

    /// Where the feet of a `height` m tall figure stand if their pixel is `feet` and the top of the
    /// head is on pixel row `head_v` (CryEngine coordinates).
    pub fn solve_feet(&self, feet: Vec2, head_v: f32, height: f32) -> Vec3 {
        let ray = self.ray(feet);
        let (mut lo, mut hi) = (0.5f32, 40.0f32);
        for _ in 0..60 {
            let t = 0.5 * (lo + hi);
            let f = self.pos + ray * t;
            let top = self.project(f + Vec3::Z * height).y;
            // a nearer figure is taller on the screen: head row further above the feet
            if feet.y - top > feet.y - head_v {
                lo = t;
            } else {
                hi = t;
            }
        }
        self.pos + ray * (0.5 * (lo + hi))
    }

    /// The camera's transform in stage space.
    pub fn transform(&self) -> Transform {
        let (f, _, u) = self.axes();
        Transform::from_translation(cry_to_stage(self.pos)).looking_to(cry_vec(f), cry_vec(u))
    }

    /// The camera's projection (the capture's vertical FOV).
    pub fn projection(&self) -> Projection {
        Projection::Perspective(PerspectiveProjection { fov: self.fov_deg.to_radians(), ..default() })
    }

    /// Move a stage camera to this pose.
    pub fn apply(&self, transform: &mut Transform, projection: &mut Projection) {
        *transform = self.transform();
        *projection = self.projection();
    }
}

/// Where a character stands: the spot of its feet (CryEngine coordinates) and the direction it
/// turns to (a horizontal CryEngine vector, usually towards the camera).
#[derive(Clone, Copy, Debug)]
pub struct StageSlot {
    pub feet: Vec3,
    pub face: Vec3,
    /// Lift so the soles of this model touch the ground (the idle pose holds the figure a few cm
    /// below its model origin; measured per model on the captures).
    pub sole: f32,
    /// Extra turn about the vertical axis, degrees, positive turns to the viewer's left.
    pub yaw_deg: f32,
}

impl StageSlot {
    /// The character select slot, facing the select camera.
    pub fn select(race: Race, gender: Gender) -> StageSlot {
        StageSlot::facing(SELECT_FEET, StagePose::CHARACTER_SELECT.pos, race, gender)
    }

    /// A slot at `feet` for a figure of this model that faces `camera_pos`.
    pub fn facing(feet: Vec3, camera_pos: Vec3, race: Race, gender: Gender) -> StageSlot {
        let to_cam = Vec3::new(camera_pos.x - feet.x, camera_pos.y - feet.y, 0.0).normalize_or_zero();
        StageSlot { feet, face: to_cam, sole: sole_below(race, gender), yaw_deg: 0.0 }
    }

    /// The character root's transform in stage space. Characters face Bevy -Z in their model space.
    pub fn transform(&self) -> Transform {
        let facing = cry_vec(self.face);
        let rotation = Quat::from_rotation_y(self.yaw_deg.to_radians()) * Quat::from_rotation_arc(Vec3::NEG_Z, facing);
        Transform::from_translation(cry_to_stage(self.feet) + Vec3::Y * self.sole).with_rotation(rotation)
    }

    /// The rotation that turns the stage frame of [`StageLighting`] to this slot.
    pub fn light_frame(&self) -> Quat {
        Quat::from_rotation_y(self.yaw_deg.to_radians()) * Quat::from_rotation_arc(Vec3::NEG_Z, cry_vec(self.face))
    }
}

/// The idle pose holds the figure about 9 cm below the origin of its model (measured on the renders
/// of every model): lifted so the soles stand on the ground like in the captures.
pub fn sole_below(race: Race, gender: Gender) -> f32 {
    match (race, gender) {
        (Race::Nuian, Gender::Male) => 0.093,
        (Race::Nuian, _) => 0.09,
        (Race::Elf, Gender::Male) => 0.08,
        (Race::Elf, _) => 0.088,
        (Race::Hariharan, Gender::Male) => 0.083,
        (Race::Hariharan, _) => 0.074,
        (Race::Ferre, Gender::Male) => 0.048,
        (Race::Ferre, _) => 0.10,
    }
}

/// Root of everything the stage spawned (camera, lights, subjects): despawn it to leave the stage.
#[derive(Component)]
pub struct StageRoot;

/// The stage's 3D camera.
#[derive(Component)]
pub struct StageCamera;

/// A character standing on the stage (put on the stage render layer by [`apply_stage_layers`]).
#[derive(Component)]
pub struct StageSubject;

/// Handles of a spawned stage.
#[derive(Clone, Debug)]
pub struct Stage {
    pub root: Entity,
    pub camera: Entity,
    pub lights: Vec<Entity>,
}

/// Spawn the stage: root, camera at `pose` and the lighting rig. Everything is a child of the root.
pub fn spawn_stage(commands: &mut Commands, cry: &mut CryAssets, pose: &StagePose, lighting: &StageLighting) -> Stage {
    let layers = RenderLayers::layer(STAGE_LAYER);
    let root = commands.spawn((Name::new("login stage"), StageRoot, Transform::default(), Visibility::default())).id();
    let camera = commands
        .spawn((
            Name::new("login stage camera"),
            StageCamera,
            Camera3d::default(),
            Camera {
                order: STAGE_CAMERA_ORDER,
                // the painted background of the UI's background camera is below
                clear_color: ClearColorConfig::None,
                ..default()
            },
            // same blending space as the UI cameras, or Bevy falls back to linear for all of them
            CompositingSpace::Srgb,
            Tonemapping::TonyMcMapface,
            login_color_grading(),
            pose.projection(),
            pose.transform(),
            layers.clone(),
            ChildOf(root),
        ))
        .id();
    // lights face the camera: the stage frame has +Z from the character to the camera
    let frame = {
        let to_cam = Vec3::new(pose.pos.x - STAGE_ORIGIN.x, pose.pos.y - STAGE_ORIGIN.y, 0.0).normalize_or_zero();
        Quat::from_rotation_arc(Vec3::NEG_Z, cry_vec(to_cam))
    };
    let lights = lighting.spawn(commands, cry, camera, frame);
    for l in &lights {
        commands.entity(*l).insert((layers.clone(), ChildOf(root)));
    }
    Stage { root, camera, lights }
}

/// Spawn a character into a slot as a child of the stage root. Returns the character's root.
pub fn spawn_subject(
    commands: &mut Commands,
    cry: &mut CryAssets,
    clips: &ClipLibrary,
    assets: CharacterAssets,
    slot: &StageSlot,
    stage_root: Entity,
) -> Option<Entity> {
    let name = assets.desc.race;
    match spawn_character(commands, cry, clips, assets, slot.transform()) {
        Ok(e) => {
            commands.entity(e).insert((StageSubject, RenderLayers::layer(STAGE_LAYER), ChildOf(stage_root)));
            Some(e)
        }
        Err(err) => {
            error!("login stage: cannot spawn a {name:?} character: {err}");
            None
        }
    }
}

/// Hold the idle pose of every stage subject at its first frame (screenshots compare one moment).
pub fn freeze_subjects(mut q: Query<&mut Animator, With<StageSubject>>) {
    for mut a in &mut q {
        a.speed = 0.0;
    }
}

/// Puts the whole mesh hierarchy of every [`StageSubject`] on the stage layer (`RenderLayers` is
/// not inherited, and `spawn_character` builds the hierarchy without it).
pub fn apply_stage_layers(
    mut commands: Commands,
    subjects: Query<Entity, With<StageSubject>>,
    children: Query<&Children>,
    layered: Query<(), With<RenderLayers>>,
) {
    for subject in &subjects {
        let mut stack = vec![subject];
        while let Some(e) = stack.pop() {
            if !layered.contains(e) {
                commands.entity(e).insert(RenderLayers::layer(STAGE_LAYER));
            }
            if let Ok(c) = children.get(e) {
                stack.extend(c.iter());
            }
        }
    }
}

pub struct Stage3dPlugin;

impl Plugin for Stage3dPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, apply_stage_layers);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_slot_is_the_solved_feet() {
        let p = StagePose::CHARACTER_SELECT;
        let feet = p.solve_feet(Vec2::new(400.0, 1015.0), 187.0, 1.9);
        assert!((feet - SELECT_FEET).length() < 0.01, "{feet:?}");
        // the feet project back onto their pixel
        let px = p.project(SELECT_FEET);
        assert!((px - Vec2::new(400.0, 1015.0)).length() < 1.0, "{px:?}");
    }

    #[test]
    fn camera_json_parses() {
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../reference/character_select/1_character/camera.json"));
        let Ok(text) = text else { return };
        let pose = StagePose::from_json(&text).unwrap();
        assert!((pose.pos - StagePose::CHARACTER_SELECT.pos).length() < 1e-3);
        assert!((pose.fov_deg - 30.0).abs() < 1e-4);
        assert!((pose.roll_deg + 1.27).abs() < 1e-3);
    }
}
