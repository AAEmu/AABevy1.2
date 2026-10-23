//! A character rendered with the camera of a real-client capture, next to `real.png`.
//!
//! ```text
//! cargo run -p aa-anim --example compare -- --target step2_elf_female --shot shots/materials/step2_elf_female.png
//!     --target   stepN_<race>_<gender> (reference/character_create/..., N = 1 body, 2 face close-up)
//!                or `select` (reference/character_select/1_character: Refone in the starter outfit)
//!     [--preset N]        create-screen face preset (default 0)
//!     [--bg r,g,b]        background colour 0..255 (default: sampled from the capture)
//!     [--yaw deg]         extra turn of the character (default: faces the capture's camera)
//!     [--light K]         scale of the stage lights (default 1)
//!     [--backlight K]     add the engine's `Char BackLight` term (`aa_cry::shade::backlight`): light 1 = K engine units
//!                         (the login stage zones' `1 Color` x `1 Diffuse Multiplier` is 1 x 1), light 2 = 0
//!     [--no-hair 1] [--costume none]
//! ```
//!
//! The camera is the capture's `camera.json` (CryEngine position, direction, roll, vertical FOV),
//! the character stands where the capture shows its feet: the spot is solved from the pixel of
//! the feet and the pixel row of the head top in the real image (`STAGES`), so the figure has the
//! size and place of the capture and only the shading is left to compare. The flat background
//! is the mean colour of a box of the capture without character or UI.
//! `crates/aa-anim/tools/side_by_side.py` puts the shot and the capture side by side.

use aa_anim::{AnimPlugin, Animator, CharacterDesc, ClipLibrary, Costume, GameDb, Gender, Race, spawn_character};
use aa_cry::scene::{CryAssets, CryPlugin};
use aa_cry::stage::StageLighting;
use bevy::prelude::*;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::render::view::ColorGrading;

const REF: &str = "D:/AABevy1.2/reference";

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

/// Camera of a capture (`camera.json`), CryEngine space (Z up, metres, degrees).
#[derive(Clone, Copy, Debug)]
struct CryCamera {
    pos: Vec3,
    dir: Vec3,
    roll: f32,
    fov: f32,
}

impl CryCamera {
    fn load(folder: &str) -> CryCamera {
        let path = format!("{REF}/{folder}/camera.json");
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let num = |section: &str, key: &str| -> f32 {
            let s = text.find(&format!("\"{section}\"")).unwrap_or_else(|| panic!("{path}: no {section}"));
            let rest = &text[s..];
            let k = rest.find(&format!("\"{key}\"")).unwrap_or_else(|| panic!("{path}: no {section}.{key}"));
            let v = &rest[k + key.len() + 2..];
            let v = v.trim_start_matches([':', ' ']);
            let end = v.find([',', '\n', '}']).unwrap();
            v[..end].trim().parse().unwrap_or_else(|_| panic!("{path}: {section}.{key} = {:?}", &v[..end]))
        };
        let flat = |key: &str| -> f32 {
            let k = text.find(&format!("\"{key}\"")).unwrap_or_else(|| panic!("{path}: no {key}"));
            let v = text[k + key.len() + 2..].trim_start_matches([':', ' ']);
            let end = v.find([',', '\n', '}']).unwrap();
            v[..end].trim().parse().unwrap()
        };
        CryCamera {
            pos: Vec3::new(num("pos", "x"), num("pos", "y"), num("pos", "z")),
            dir: Vec3::new(num("dir", "x"), num("dir", "y"), num("dir", "z")).normalize(),
            roll: num("angles_deg", "y"),
            fov: flat("fov_deg"),
        }
    }

    /// Camera axes (forward, right, up) in CryEngine space.
    fn axes(&self) -> (Vec3, Vec3, Vec3) {
        let f = self.dir;
        let right0 = f.cross(Vec3::Z).normalize();
        let up0 = right0.cross(f);
        // roll about the forward axis (right handed): the up vector tilts to the right
        let q = Quat::from_axis_angle(f, self.roll.to_radians());
        (f, q * right0, q * up0)
    }

    /// Pixel of a world point (1920x1080 capture): `(u, v)`.
    fn project(&self, p: Vec3) -> Vec2 {
        let (f, r, u) = self.axes();
        let focal = 540.0 / (self.fov.to_radians() / 2.0).tan();
        let d = p - self.pos;
        let z = d.dot(f);
        Vec2::new(960.0 + focal * d.dot(r) / z, 540.0 - focal * d.dot(u) / z)
    }

    /// Ray through a pixel.
    fn ray(&self, uv: Vec2) -> Vec3 {
        let (f, r, u) = self.axes();
        let focal = 540.0 / (self.fov.to_radians() / 2.0).tan();
        (f + r * ((uv.x - 960.0) / focal) + u * (-(uv.y - 540.0) / focal)).normalize()
    }

    /// Where the feet of a `height` m tall figure stand if their pixel is `feet` and the top of
    /// the head is on pixel row `head_v`.
    fn solve_feet(&self, feet: Vec2, head_v: f32, height: f32) -> Vec3 {
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
}

/// Where a figure stands in a capture: feet pixel, head top row and the figure's height (m).
struct Stage {
    feet: Vec2,
    head_v: f32,
    height: f32,
}

/// The idle pose of the stage holds the figure about 9 cm below the origin of its model (measured
/// on the renders of every model): lifted so the soles stand on the ground like in the capture.
fn sole_below(race: Race, gender: Gender) -> f32 {
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

/// Measured on the step 1 captures: the pixel row of the soles and of the head top of every
/// figure, and the figure's height above its soles (from the renders: the nominal actor
/// heights are not the visual ones). Both genders of a race stand at about the same spot but
/// are scaled a little differently in the client, so each model is solved on its own.
fn stage_of(race: Race, gender: Gender) -> Stage {
    let (feet_v, head_v, height) = match (race, gender) {
        (Race::Nuian, Gender::Male) => (1020.0, 97.0, 2.03),
        (Race::Nuian, _) => (1009.0, 157.0, 1.81),
        (Race::Elf, Gender::Male) => (1007.0, 83.0, 2.02),
        (Race::Elf, _) => (1007.0, 147.0, 1.89),
        (Race::Hariharan, Gender::Male) => (947.0, 164.0, 1.80),
        (Race::Hariharan, _) => (960.0, 217.0, 1.68),
        (Race::Ferre, Gender::Male) => (1020.0, 87.0, 2.07),
        (Race::Ferre, _) => (1033.0, 217.0, 1.84),
    };
    Stage { feet: Vec2::new(960.0, feet_v), head_v, height }
}

struct Target {
    folder: String,
    /// Folder of the camera that frames the stage (step 1) when `folder` is a close-up.
    stage_folder: String,
    race: Race,
    gender: Gender,
    stage: Stage,
    costume: Costume,
    /// Box `x0,y0,x1,y1` of the capture sampled for the background colour.
    bg_box: [u32; 4],
}

fn parse_target(name: &str) -> Target {
    if name == "select" {
        return Target {
            folder: "character_select/1_character".into(),
            stage_folder: "character_select/1_character".into(),
            race: Race::Nuian,
            gender: Gender::Male,
            stage: Stage { feet: Vec2::new(400.0, 1015.0), head_v: 187.0, height: 1.9 },
            // Refone: the "first steps" armour pack (shirt, pants, shoes)
            costume: Costume::Items(vec![23387, 23388, 23390]),
            bg_box: [60, 300, 200, 700],
        };
    }
    let parts: Vec<&str> = name.split('_').collect();
    assert!(parts.len() == 3 && parts[0].starts_with("step"), "--target stepN_<race>_<gender> or select");
    let race = match parts[1] {
        "elf" => Race::Elf,
        "hariharan" => Race::Hariharan,
        "ferre" => Race::Ferre,
        _ => Race::Nuian,
    };
    let gender = if parts[2] == "female" { Gender::Female } else { Gender::Male };
    let stage = stage_of(race, gender);
    Target {
        folder: format!("character_create/{name}"),
        stage_folder: format!("character_create/step1_{}_{}", parts[1], parts[2]),
        race,
        gender,
        stage,
        costume: Costume::Login,
        bg_box: [640, 350, 780, 750],
    }
}

fn main() {
    let name = arg("--target").unwrap_or_else(|| "step1_elf_female".into());
    let target = parse_target(&name);
    let bg = arg("--bg")
        .map(|v| {
            let p: Vec<f32> = v.split(',').filter_map(|c| c.trim().parse().ok()).collect();
            Vec3::new(p[0], p[1], p[2]) / 255.0
        })
        .unwrap_or_else(|| sample_background(&target));
    App::new()
        .add_plugins((aa_dev::default_plugins("aa-anim compare"), aa_dev::DevPlugin, CryPlugin, AnimPlugin))
        .add_plugins(aa_cry::shade::backlight::CharLightPlugin)
        .insert_resource(ClearColor(Color::srgb(bg.x, bg.y, bg.z)))
        .insert_resource(GlobalAmbientLight { brightness: 0.0, ..default() })
        .insert_resource(CompareTarget(target))
        .add_systems(Startup, setup)
        .add_systems(Update, hold_pose)
        .run();
}

#[derive(Resource)]
struct CompareTarget(Target);

/// Mean sRGB colour of the background box of `real.png`.
fn sample_background(t: &Target) -> Vec3 {
    let path = format!("{REF}/{}/real.png", t.folder);
    let img = image::open(&path).unwrap_or_else(|e| panic!("{path}: {e}")).to_rgb8();
    let [x0, y0, x1, y1] = t.bg_box;
    let mut sum = Vec3::ZERO;
    let mut n = 0.0f32;
    for y in y0..y1.min(img.height()) {
        for x in x0..x1.min(img.width()) {
            let p = img.get_pixel(x, y).0;
            sum += Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32);
            n += 1.0;
        }
    }
    let m = sum / n.max(1.0);
    info!("compare: background {:.0},{:.0},{:.0} (box {:?} of {path})", m.x, m.y, m.z, t.bg_box);
    m / 255.0
}

/// CryEngine (Z up) -> Bevy (Y up) for points and directions.
fn to_bevy(v: Vec3) -> Vec3 {
    Vec3::new(v.x, v.z, -v.y)
}

fn setup(mut commands: Commands, mut cry: CryAssets, clips: Res<ClipLibrary>, target: Res<CompareTarget>) {
    let t = &target.0;
    let db = GameDb::open().expect("game database");
    let preset: usize = arg("--preset").and_then(|v| v.parse().ok()).unwrap_or(0);

    let cam = CryCamera::load(&t.folder);
    let stage_cam = CryCamera::load(&t.stage_folder);
    let feet = stage_cam.solve_feet(t.stage.feet, t.stage.head_v, t.stage.height);
    let (dist, drop) = (
        Vec2::new(feet.x - stage_cam.pos.x, feet.y - stage_cam.pos.y).length(),
        stage_cam.pos.z - feet.z,
    );
    info!("compare: figure at {feet:?} ({dist:.2} m from the stage camera, camera {drop:.2} m above the feet)");

    // the figure faces the stage camera
    let to_cam = Vec3::new(stage_cam.pos.x - feet.x, stage_cam.pos.y - feet.y, 0.0).normalize();
    let facing = to_bevy(to_cam);
    let yaw_extra: f32 = arg("--yaw").and_then(|v| v.parse().ok()).unwrap_or(0.0);
    let rotation = Quat::from_rotation_y(yaw_extra.to_radians()) * Quat::from_rotation_arc(Vec3::NEG_Z, facing);

    let mut desc = CharacterDesc::from_preset(&db, t.race, t.gender, preset).expect("preset");
    desc.costume = t.costume.clone();
    if arg("--costume").is_some_and(|c| c == "none") {
        desc.costume = Costume::None;
    }
    if arg("--no-hair").is_some() {
        desc.hair = None;
    }
    // `sole_below` was measured before the spawn stood heeled shoes on their heels
    let spawn = |a: aa_anim::CharacterAssets| {
        let lift = sole_below(t.race, t.gender) - a.heel;
        spawn_character(&mut commands, &mut cry, &clips, a, Transform::from_translation(Vec3::Y * lift).with_rotation(rotation))
    };
    match desc.resolve(&db).and_then(spawn) {
        Ok(e) => {
            commands.entity(e).insert(Name::new("subject"));
        }
        Err(e) => error!("{:?} {:?}: {e}", t.race, t.gender),
    }

    // camera: relative to the feet, CryEngine -> Bevy
    let eye = to_bevy(cam.pos - feet);
    let (fwd, _, up) = cam.axes();
    let camera = commands
        .spawn((
            Camera3d::default(),
            tonemapping(),
            grading(),
            Projection::Perspective(PerspectiveProjection { fov: cam.fov.to_radians(), ..default() }),
            Transform::from_translation(eye).looking_to(to_bevy(fwd), to_bevy(up)),
        ))
        .id();

    // `--ssao low|medium|high|ultra[,radius[,thickness]]`: screen space ambient occlusion (needs no MSAA)
    if let Some(v) = arg("--ssao") {
        use bevy::pbr::{ScreenSpaceAmbientOcclusion, ScreenSpaceAmbientOcclusionQualityLevel as Q};
        let mut it = v.split(',');
        let quality_level = match it.next().unwrap_or("low") {
            "medium" => Q::Medium,
            "high" => Q::High,
            "ultra" => Q::Ultra,
            _ => Q::Low,
        };
        let radius = it.next().and_then(|r| r.parse().ok()).unwrap_or(1.2);
        let constant_object_thickness = it.next().and_then(|r| r.parse().ok()).unwrap_or(0.25);
        commands.entity(camera).insert((Msaa::Off, ScreenSpaceAmbientOcclusion { quality_level, radius, constant_object_thickness }));
    }
    let k: f32 = arg("--light").and_then(|v| v.parse().ok()).unwrap_or(1.0);
    stage_lights(&mut commands, &mut cry, camera, rotation, k);
}

/// The login stage lights of `aa_cry::stage`, turned to face the camera; `--key K --fill K --rim K
/// --env K --amb K` scale the parts (calibration), `--light K` scales all.
fn stage_lights(commands: &mut Commands, cry: &mut CryAssets, camera: Entity, rotation: Quat, k: f32) {
    let f = |name: &str| arg(name).and_then(|v| v.parse::<f32>().ok()).unwrap_or(1.0);
    let mut stage = StageLighting::login().scaled(k);
    for (l, name) in stage.lights.iter_mut().zip(["--key", "--fill", "--rim"]) {
        l.intensity *= f(name);
    }
    if let Some((_, i)) = &mut stage.environment {
        *i *= f("--env");
    }
    if let Some((_, i)) = &mut stage.ambient {
        *i *= f("--amb");
    }
    for (i, name) in ["--key-from", "--fill-from", "--rim-from"].into_iter().enumerate() {
        if let Some(v) = arg(name).map(|v| v.split(',').filter_map(|x| x.trim().parse::<f32>().ok()).collect::<Vec<_>>()).filter(|v| v.len() == 3) {
            stage.lights[i].from = Vec3::new(v[0], v[1], v[2]);
        }
    }
    if arg("--shadows").is_some_and(|v| v == "0") {
        for l in &mut stage.lights {
            l.shadows = false;
        }
    }
    let lights = stage.spawn(commands, cry, camera, rotation);
    // `--shadow-bias depth,normal` and `--cascade n,near,far,first` for the shadow experiments
    let nums = |name: &str| arg(name).map(|v| v.split(',').filter_map(|x| x.trim().parse::<f32>().ok()).collect::<Vec<_>>());
    if let Some(b) = nums("--shadow-bias").filter(|b| b.len() == 2) {
        let (depth, normal) = (b[0], b[1]);
        for e in &lights {
            commands.entity(*e).entry::<DirectionalLight>().and_modify(move |mut l| {
                l.shadow_depth_bias = depth;
                l.shadow_normal_bias = normal;
            });
        }
    }
    if let Some(c) = nums("--cascade").filter(|c| c.len() == 4) {
        for e in &lights {
            commands.entity(*e).insert(bevy::light::CascadeShadowConfigBuilder {
                num_cascades: c[0] as usize,
                minimum_distance: c[1],
                maximum_distance: c[2],
                first_cascade_far_bound: c[3],
                ..default()
            }.build());
        }
    }
    if let Some(n) = arg("--shadow-map").and_then(|v| v.parse::<usize>().ok()) {
        commands.insert_resource(bevy::light::DirectionalLightShadowMap { size: n });
    }
    if let Some(k) = arg("--backlight").and_then(|v| v.parse::<f32>().ok()) {
        let lux = k * aa_cry::stage::LIGHT_UNIT_LUX * aa_cry::stage::LOGIN_EXPOSURE;
        use aa_cry::shade::backlight::{CDF_POSITIONS, CharBackLight};
        commands.insert_resource(CharBackLight::from_positions(CDF_POSITIONS, [Vec4::new(lux, lux, lux, 1.0), Vec4::ZERO]));
    }
}

/// The capture shows one moment of the idle; hold the first frame so renders are comparable.
fn hold_pose(mut q: Query<&mut Animator>) {
    for mut a in &mut q {
        a.speed = 0.0;
    }
}

fn tonemapping() -> Tonemapping {
    match arg("--tonemap").as_deref() {
        Some("none") => Tonemapping::None,
        Some("reinhard") => Tonemapping::Reinhard,
        Some("reinhard_lum") => Tonemapping::ReinhardLuminance,
        Some("aces") => Tonemapping::AcesFitted,
        Some("agx") => Tonemapping::AgX,
        Some("blender") => Tonemapping::BlenderFilmic,
        Some("somewhat") => Tonemapping::SomewhatBoringDisplayTransform,
        Some("neutral") => Tonemapping::KhronosPbrNeutral,
        _ => Tonemapping::TonyMcMapface,
    }
}

/// The login stage's colour grading; `--sat K` / `--contrast K` override it.
fn grading() -> ColorGrading {
    match (arg("--sat"), arg("--contrast")) {
        (None, None) => aa_cry::stage::login_color_grading(),
        (s, c) => aa_cry::stage::color_grading(
            s.and_then(|v| v.parse().ok()).unwrap_or(1.0),
            c.and_then(|v| v.parse().ok()).unwrap_or(1.0),
        ),
    }
}
