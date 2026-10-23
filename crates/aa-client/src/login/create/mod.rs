//! Character creation: the original `character_create` + `customizing` scripts (race, appearance,
//! skillset and name) on the game API of [`base`] / [`api`], a live 3D preview of the character on
//! the login stage, and the create request through aa-net.
//!
//! ```text
//! cargo run -p aa-client -- --goto create --race elf --gender female --step 2 --account bevycreate --shot shots/create/step2.png --shot-after 20
//! ```
//!
//! ## Flags (all optional)
//!
//! * `--goto create`: drive the login screens (server list, Connect) to this screen; needs the
//!   server and the account flags of the login screens (`--account NAME`, `--login`,
//!   `--password-file`, see `ingame::enter::Credentials`; the runs of this screen use `bevycreate`).
//! * `--race nuian|elf|ferre|hariharan`, `--gender male|female`, `--step 1|2|3`: replay what the
//!   reference captures did (the race and gender buttons, then the next-step button).
//! * `--clua CODE T`, `--hover X Y T`, `--click X Y T`, `--drag X1 Y1 X2 Y2 T` (physical pixels), `--wheel D T`, `--type TEXT T` (+1 = one notch
//!   up), `--cshot FILE T`: scripted input and screenshots `T` seconds after the screen is up
//!   (repeatable; one run can take several shots, e.g. one per tab).
//! * `--create-offline`: with `--account NAME`, open the screen as soon as the login is through, without
//!   entering a world (dev: screenshots and tuning while the account is busy in the world).
//! * `--exit-after-create`: quit once the server answered the create request.
//!
//! ## Hand-off
//!
//! Enter with `NextState<Screen>::set(Screen::CharacterCreate)`. The screen builds its own UI
//! ([`host::switch_stage`]: fresh Lua state, the game API of this module, the original scripts) and
//! its own 3D ([`stage3d`]). It leaves to [`Screen::CharacterSelect`] on cancel and after the server
//! created the character (the session refreshes the character list by itself).
//!
//! ## Parts
//!
//! * `look`: the customization state and its lists (hair, colours, premade faces, sliders, decals)
//!   in the order of the real client, and the create request;
//! * `api` + `game_create.lua`: the natives (the `CC` table) and the Lua objects built on them
//!   (`X2LoginCharacter`, `X2Customizer`, the customizing unit);
//! * `poses`: the cameras of the captures and where the figure stands;
//! * `preview`: the cache that keeps slider drags cheap;
//! * `saved`: saved looks on disk.

pub(crate) mod api;
mod base;
mod grass;
pub(crate) mod look;
pub(crate) mod poses;
pub(crate) mod preview;
mod saved;
mod scenery;

use aa_anim::cal::Cal;
use aa_anim::{Animator, ClipLibrary, Gender, Race, SampleOptions};
use aa_cry::scene::CryAssets;
use aa_cry::stage::StageLighting;
use aa_net::state::{Characters, Worlds};
use aa_net::{Command, Event, NetCommand, NetEvent};
use aa_ui::{Stage as UiStage, UiRuntime};
use bevy::app::AppExit;
use bevy::prelude::*;

use self::api::{Request, Shared, SharedRef, lock};
use self::look::Customizer;
use super::stage3d::{self, StageCamera, StagePose, StageSlot, StageSubject};
use super::{host, ui_is};
use crate::Screen;

/// Seconds a camera move between two steps takes.
const CAMERA_SECONDS: f32 = 0.9;
/// Preview rebuilds are at most this far apart while a slider moves.
const REBUILD_SECONDS: f32 = 0.08;

pub fn plugin(app: &mut App) {
    grass::plugin(app);
    app.insert_resource(Options::from_args())
        .add_systems(OnEnter(Screen::CharacterSelect), redirect_to_create)
        .add_systems(OnEnter(Screen::CharacterCreate), enter)
        .add_systems(OnExit(Screen::CharacterCreate), exit)
        .add_systems(Update, auto_drive.run_if(ui_is(Screen::WorldSelect)))
        .add_systems(Update, go_offline.run_if(in_state(Screen::WorldSelect)))
        .add_systems(Update, build_ui.run_if(in_state(Screen::CharacterCreate)))
        .add_systems(
            Update,
            (
                sync_net,
                handle_requests,
                update_preview,
                scenery::update,
                set_animation,
                drive_camera,
                run_script,
                net_events,
                stage3d::freeze_subjects.run_if(freeze_for_shots),
            )
                .chain()
                .run_if(in_state(Screen::CharacterCreate)),
        );
}

// ---- command line -----------------------------------------------------------------------------

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

/// Dev flags of the screen.
#[derive(Resource, Clone, Debug)]
pub struct Options {
    /// `--goto create`
    pub goto: bool,
    /// `--create-offline`: open the screen as soon as the login is through, without entering a world
    /// (dev: screenshots and tuning; the create request cannot be answered).
    pub offline: bool,
    pub race: Race,
    pub gender: Gender,
    /// The step the scripted replay ends on.
    pub step: u8,
    /// `--clua CODE T` pairs.
    pub lua: Vec<(String, f32)>,
    /// `--hover X Y T` / `--click X Y T`: (x, y, seconds, click).
    pub pointer: Vec<(f32, f32, f32, bool)>,
    /// `--drag X1 Y1 X2 Y2 T`: press at the first point, move to the second over a few frames, release.
    pub drags: Vec<(Vec2, Vec2, f32)>,
    /// `--wheel D T`
    pub wheel: Vec<(f32, f32)>,
    /// `--type TEXT T`: typed into the focused edit box.
    pub typed: Vec<(String, f32)>,
    /// `--cshot FILE T`
    pub shots: Vec<(String, f32)>,
    pub exit_after_create: bool,
}

impl Options {
    fn from_args() -> Options {
        let args: Vec<String> = std::env::args().collect();
        let (mut lua, mut pointer, mut wheel, mut shots, mut drags, mut typed) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for (i, a) in args.iter().enumerate() {
            let num = |k: usize| args.get(i + k).and_then(|s| s.parse::<f32>().ok());
            match a.as_str() {
                "--clua" => {
                    if let (Some(code), Some(t)) = (args.get(i + 1), num(2)) {
                        lua.push((code.clone(), t));
                    }
                }
                "--hover" | "--click" => {
                    if let (Some(x), Some(y), Some(t)) = (num(1), num(2), num(3)) {
                        pointer.push((x, y, t, a == "--click"));
                    }
                }
                "--drag" => {
                    if let (Some(x1), Some(y1), Some(x2), Some(y2), Some(t)) = (num(1), num(2), num(3), num(4), num(5)) {
                        drags.push((Vec2::new(x1, y1), Vec2::new(x2, y2), t));
                    }
                }
                "--type" => {
                    if let (Some(text), Some(t)) = (args.get(i + 1), num(2)) {
                        typed.push((text.clone(), t));
                    }
                }
                "--wheel" => {
                    if let (Some(d), Some(t)) = (num(1), num(2)) {
                        wheel.push((d, t));
                    }
                }
                "--cshot" => {
                    if let (Some(path), Some(t)) = (args.get(i + 1), num(2)) {
                        shots.push((path.clone(), t));
                    }
                }
                _ => {}
            }
        }
        Options {
            goto: flag(&args, "--goto").as_deref() == Some("create"),
            offline: args.iter().any(|a| a == "--create-offline"),
            race: api::race_from(&flag(&args, "--race").unwrap_or_default()),
            gender: api::gender_from(&flag(&args, "--gender").unwrap_or_default()),
            step: flag(&args, "--step").and_then(|s| s.parse().ok()).unwrap_or(1).clamp(1, 3),
            lua,
            pointer,
            drags,
            wheel,
            typed,
            shots,
            exit_after_create: args.iter().any(|a| a == "--exit-after-create"),
        }
    }
}

/// Screenshot runs hold the idle pose at its first frame (they compare one moment).
fn freeze_for_shots(args: Option<Res<aa_dev::DevArgs>>, opts: Res<Options>) -> bool {
    args.is_some_and(|a| a.shot.is_some()) || !opts.shots.is_empty()
}

// ---- getting here with `--goto create` ------------------------------------------------------------

/// `--goto create`: pick the first online server and press Connect (the login module does the same
/// for its own `--goto` targets).
fn auto_drive(
    rt: NonSend<UiRuntime>,
    opts: Res<Options>,
    worlds: Res<Worlds>,
    time: Res<Time>,
    mut done: Local<bool>,
    mut since: Local<f32>,
) {
    if *done || !opts.goto {
        return;
    }
    *since += time.delta_secs();
    if *since < 1.0 {
        return;
    }
    let Some(world) = worlds.list.worlds.iter().find(|w| w.online) else { return };
    *done = true;
    info!("character create: --goto: connecting to {}", world.name);
    let code = format!("backgroundWindow.mainWindow:SelectByDataKey({id}) backgroundWindow.enterWorldBtn:OnClick()", id = world.id);
    if let Err(e) = rt.exec("goto", &code) {
        warn!("character create: --goto: {e}");
    }
}

/// `--create-offline`: to the create screen once the login answered with the server list (once).
fn go_offline(opts: Res<Options>, worlds: Res<Worlds>, mut next: ResMut<NextState<Screen>>, mut done: Local<bool>) {
    // after the login: the server list arriving later would send the screen back to the list
    if opts.offline && !*done && !worlds.list.worlds.is_empty() {
        *done = true;
        next.set(Screen::CharacterCreate);
    }
}

/// `--goto create` with characters on the account: the login module shows character select, this
/// goes on to the create screen (once).
fn redirect_to_create(opts: Res<Options>, mut next: ResMut<NextState<Screen>>, mut done: Local<bool>) {
    if opts.goto && !*done {
        *done = true;
        next.set(Screen::CharacterCreate);
    }
}

// ---- the screen -------------------------------------------------------------------------------

/// Everything the screen owns while it is up.
#[derive(Resource)]
struct CreateScreen {
    shared: SharedRef,
    stage: stage3d::Stage,
    subject: Option<Entity>,
    /// The look (revision, race, gender) the subject was built from.
    built: Option<(u64, Race, Gender)>,
    last_build: f32,
    /// The step the camera is heading for / the subject stands for.
    step: u8,
    tween: Option<Tween>,
    ui_built: bool,
    /// Seconds since the UI of this screen was up (scripted replay).
    ui_time: f32,
    script_done: Vec<bool>,
    /// Where the subject stands: (step, race, gender).
    placed: Option<(u8, Race, Gender)>,
    cache: preview::PreviewCache,
    /// The subject and whether it plays the step 3 stance already.
    anim_for: Option<(Entity, bool)>,
}

struct Tween {
    from: (Vec3, Quat, f32),
    t: f32,
}

fn camera_state(pose: &StagePose) -> (Vec3, Quat, f32) {
    let t = pose.transform();
    (t.translation, t.rotation, pose.fov_deg.to_radians())
}

fn enter(mut commands: Commands, mut cry: CryAssets, opts: Res<Options>) {
    // the real screen starts on a Nuian man, whatever the player picked before
    let (race, gender) = (Race::Nuian, Gender::Male);
    let cz = match Customizer::open(race, gender) {
        Ok(c) => c,
        Err(e) => {
            error!("character create: {e:#}");
            return;
        }
    };
    let shared: SharedRef = std::sync::Arc::new(std::sync::Mutex::new(Shared {
        cz,
        step: 1,
        requests: Vec::new(),
        race_congestion: [0; 9],
        zoom: 0.0,
        ability: 0,
        create_pending: false,
        locale: "en_us".into(),
    }));
    let pose = poses::pose(1, race, gender);
    let stage = stage3d::spawn_stage(&mut commands, &mut cry, &pose, &StageLighting::login());
    commands.insert_resource(scenery::Scenery::start());
    commands.insert_resource(CreateScreen {
        shared,
        stage,
        subject: None,
        built: None,
        last_build: -10.0,
        step: 1,
        tween: None,
        ui_built: false,
        ui_time: 0.0,
        script_done: vec![false; opts.lua.len() + opts.pointer.len() + opts.drags.len() + opts.wheel.len() + opts.typed.len() + opts.shots.len() + 8],
        placed: None,
        cache: Default::default(),
        anim_for: None,
    });
}

fn exit(mut commands: Commands, screen: Option<Res<CreateScreen>>, scenery: Option<Res<scenery::Scenery>>, mut grade: ResMut<stage3d::BackgroundGrade>) {
    // the painted sky of step 3 is graded (see `scenery`)
    grade.0 = None;
    if let Some(scenery) = scenery {
        scenery.despawn_sounds(&mut commands);
    }
    if let Some(screen) = screen {
        commands.entity(screen.stage.root).despawn();
        commands.remove_resource::<CreateScreen>();
        commands.remove_resource::<scenery::Scenery>();
    }
}

/// The UI of the screen: the original scripts on a fresh Lua state (like the real client builds on
/// every stage change), through the login module's [`host::switch_stage`].
fn build_ui(world: &mut World) {
    let Some(screen) = world.get_resource::<CreateScreen>() else { return };
    if screen.ui_built {
        return;
    }
    let shared = screen.shared.clone();
    let ok = host::switch_stage(world, |ui| {
        if let Err(e) = base::install(ui, &shared) {
            error!("character create: game API: {e:#}");
        }
        ui.run_stage(UiStage::CharacterCreate);
        ui.fire_event("SHOW_CHARACTER_CREATE_WINDOW", true);
    });
    if ok {
        if let Some(mut screen) = world.get_resource_mut::<CreateScreen>() {
            screen.ui_built = true;
        }
    }
}

/// Race congestion from the server into the screen's state.
fn sync_net(screen: Option<ResMut<CreateScreen>>, characters: Res<Characters>) {
    if let Some(screen) = screen {
        lock(&screen.shared).race_congestion = characters.race_congestion;
    }
}

/// Where the figure stands in a step.
fn step_slot(step: u8, race: Race, gender: Gender) -> StageSlot {
    match step {
        3 => poses::slot_step_3(race, gender),
        2 => poses::slot_step_2(race, gender),
        _ => poses::slot_steps_1_2(race, gender),
    }
}

type Lights<'w, 's> = Query<'w, 's, &'static mut Transform, (Without<StageCamera>, Without<StageSubject>, With<DirectionalLight>)>;

/// Points the stage's lights for a figure standing in `slot`.
fn aim_lights(lights: &mut Lights, stage: &stage3d::Stage, slot: &StageSlot) {
    let rig = StageLighting::login();
    let frame = slot.light_frame();
    for (entity, light) in stage.lights.iter().zip(&rig.lights) {
        if let Ok(mut t) = lights.get_mut(*entity) {
            let at = frame * (light.from.normalize() * 10.0);
            *t = Transform::from_translation(at).looking_at(Vec3::ZERO, Vec3::Y);
        }
    }
}

/// The scripts' requests: leave a step, create the character, ...
fn handle_requests(
    mut screen: Option<ResMut<CreateScreen>>,
    ui: Option<NonSend<UiRuntime>>,
    mut next: ResMut<NextState<Screen>>,
    mut out: MessageWriter<NetCommand>,
    cams: Query<(&Transform, &Projection), With<StageCamera>>,
    mut lights: Lights,
) {
    let Some(screen) = screen.as_deref_mut() else { return };
    let requests = std::mem::take(&mut lock(&screen.shared).requests);
    for req in requests {
        match req {
            Request::Step { from, forward } => {
                let to = match (from, forward) {
                    (1, true) => 2,
                    (2, true) => 3,
                    (2, false) => 1,
                    (3, false) => 2,
                    _ => continue,
                };
                let (race, gender) = {
                    let mut g = lock(&screen.shared);
                    g.step = to;
                    g.zoom = 0.0;
                    (g.cz.cat.race, g.cz.cat.gender)
                };
                info!("character create: step {from} -> {to}");
                screen.step = to;
                // step 3 is another place of the login level: the real client cuts there
                screen.tween = None;
                if from.max(to) < 3 {
                    if let Ok((t, p)) = cams.single() {
                        let fov = match p {
                            Projection::Perspective(p) => p.fov,
                            _ => 0.5,
                        };
                        screen.tween = Some(Tween { from: (t.translation, t.rotation, fov), t: 0.0 });
                    }
                }
                if let Some(ui) = ui.as_deref() {
                    let (hide, show) = match (from, to) {
                        (1, 2) => ("SHOW_CHARACTER_CREATE_WINDOW", "SHOW_CHARACTER_CUSTOMIZE_WINDOW"),
                        (2, 3) => ("SHOW_CHARACTER_CUSTOMIZE_WINDOW", "SHOW_CHARACTER_ABILITY_WINDOW"),
                        (3, 2) => ("SHOW_CHARACTER_ABILITY_WINDOW", "SHOW_CHARACTER_CUSTOMIZE_WINDOW"),
                        _ => ("SHOW_CHARACTER_CUSTOMIZE_WINDOW", "SHOW_CHARACTER_CREATE_WINDOW"),
                    };
                    ui.queue_event(hide, false);
                    ui.queue_event(show, true);
                }
                aim_lights(&mut lights, &screen.stage, &step_slot(to, race, gender));
            }
            Request::GotoSelect => {
                info!("character create: back to character select");
                next.set(Screen::CharacterSelect);
            }
            Request::ReloadCosmetic => {
                if let Some(ui) = ui.as_deref() {
                    ui.queue_event("RELOAD_COSMETIC_WINDOW", ());
                }
            }
            Request::SkillAction(ability) => {
                lock(&screen.shared).ability = ability;
            }
            Request::Create { name, ability } => {
                let req = lock(&screen.shared).cz.create_request(&name, ability);
                info!("character create: creating {name} (race {}, ability {ability})", req.race);
                out.write(NetCommand(Command::CreateCharacter(req)));
            }
        }
    }
}

/// Builds the preview character when the look changed (and moves it when the step changed).
fn update_preview(
    mut commands: Commands,
    mut cry: CryAssets,
    clips: Option<Res<ClipLibrary>>,
    screen: Option<ResMut<CreateScreen>>,
    time: Res<Time>,
    mut subjects: Query<&mut Transform, With<StageSubject>>,
    mut lights: Lights,
) {
    let (Some(mut screen), Some(clips)) = (screen, clips) else { return };
    let now = time.elapsed_secs();
    let (rev, race, gender, step) = {
        let g = lock(&screen.shared);
        (g.cz.revision, g.cz.cat.race, g.cz.cat.gender, g.step)
    };
    let model_changed = screen.built.is_none_or(|(_, r, g)| (r, g) != (race, gender));
    let look_changed = screen.built.is_none_or(|(v, _, _)| v != rev);
    // dev: `AA_CREATE_NO_SUBJECT=1` leaves the stage empty (background shots for silhouette measuring)
    if std::env::var_os("AA_CREATE_NO_SUBJECT").is_some() {
        return;
    }
    if look_changed && (model_changed || now - screen.last_build >= REBUILD_SECONDS) {
        let t0 = std::time::Instant::now();
        let assets = {
            let shared = screen.shared.clone();
            let g = lock(&shared);
            screen.cache.assets(&g.cz.desc(), &g.cz.db)
        };
        let resolved = t0.elapsed();
        match assets {
            Ok(assets) => {
                let slot = step_slot(step, race, gender);
                let old = screen.subject.take();
                screen.subject = stage3d::spawn_subject(&mut commands, &mut cry, &clips, assets, &slot, screen.stage.root);
                if let Some(old) = old {
                    commands.entity(old).despawn();
                }
                if model_changed || screen.placed.is_none() {
                    aim_lights(&mut lights, &screen.stage, &slot);
                }
                screen.placed = Some((step, race, gender));
            }
            Err(e) => error!("character create: cannot resolve the look: {e}"),
        }
        debug!("character create: preview rebuilt in {:.1} ms (resolve {:.1} ms)", t0.elapsed().as_secs_f32() * 1000.0, resolved.as_secs_f32() * 1000.0);
        screen.built = Some((rev, race, gender));
        screen.last_build = now;
    }
    // the figure moves to its place of the step
    if let (Some(subject), Some(placed)) = (screen.subject, screen.placed) {
        if placed != (step, race, gender) {
            if let Ok(mut t) = subjects.get_mut(subject) {
                *t = step_slot(step, race, gender).transform();
            }
            screen.placed = Some((step, race, gender));
        }
    }
}

/// Steps 1 and 2 play the login idle (aa-anim's default); step 3 stands in the combat stance of
/// the captures (`fist_ba_combat_idle`, with its idle variants).
fn set_animation(
    screen: Option<ResMut<CreateScreen>>,
    clips: Option<Res<ClipLibrary>>,
    mut subjects: Query<(&mut Animator, &aa_anim::Character), With<StageSubject>>,
) {
    let (Some(mut screen), Some(clips)) = (screen, clips) else { return };
    let Some(subject) = screen.subject else { return };
    let combat = screen.step == 3;
    // a fresh subject already plays the login idle
    if screen.anim_for.is_none_or(|(e, _)| e != subject) {
        screen.anim_for = Some((subject, false));
    }
    if screen.anim_for == Some((subject, combat)) {
        return;
    }
    let Ok((mut animator, character)) = subjects.get_mut(subject) else { return };
    let Ok(cal) = Cal::load(&character.assets.cal) else { return };
    let bind = |name: &str| {
        let entry = cal.animation(name)?;
        let opts = SampleOptions { position_scale: entry.position_scale(cal.base_height), lock_root_motion: true };
        clips.bound(&animator.rig, &entry.path, opts).ok()
    };
    let (idle, variants): (_, Vec<&str>) = if combat {
        (bind("fist_ba_combat_idle"), vec!["fist_ba_combat_idle_rand_1", "fist_ba_combat_idle_rand_2", "fist_ba_combat_idle_rand_3"])
    } else {
        (bind(aa_anim::character::STAGE_IDLE), vec![aa_anim::character::STAGE_VARIANT])
    };
    let variants: Vec<_> = variants.into_iter().filter_map(bind).collect();
    if let Some(idle) = idle {
        let fade = if animator.speed == 0.0 { 0.0 } else { 0.4 };
        animator.play(idle, true, fade);
        animator.random_idle = (!variants.is_empty()).then(|| aa_anim::RandomIdle::new(variants, 2, 7));
        screen.anim_for = Some((subject, combat));
    }
}

/// Moves the stage camera towards the pose of the step (and applies the mouse-wheel zoom).
fn drive_camera(
    screen: Option<ResMut<CreateScreen>>,
    time: Res<Time>,
    mut cams: Query<(&mut Transform, &mut Projection), With<StageCamera>>,
) {
    let Some(mut screen) = screen else { return };
    let Ok((mut tf, mut proj)) = cams.single_mut() else { return };
    let (race, gender, zoom) = {
        let g = lock(&screen.shared);
        (g.cz.cat.race, g.cz.cat.gender, g.zoom)
    };
    let target = camera_state(&poses::pose(screen.step, race, gender));
    let (pos, rot, fov) = match screen.tween.as_mut() {
        Some(tw) => {
            tw.t += time.delta_secs() / CAMERA_SECONDS;
            let k = tw.t.clamp(0.0, 1.0);
            let k = k * k * (3.0 - 2.0 * k);
            let state = (tw.from.0.lerp(target.0, k), tw.from.1.slerp(target.1, k), tw.from.2 + (target.2 - tw.from.2) * k);
            if tw.t >= 1.0 {
                screen.tween = None;
            }
            state
        }
        None => target,
    };
    // mouse wheel in step 2: zoom the close-up
    let fov = if screen.step == 2 { fov * (1.0 - 0.5 * zoom) } else { fov };
    *tf = Transform { translation: pos, rotation: rot, scale: Vec3::ONE };
    if let Projection::Perspective(p) = &mut *proj {
        p.fov = fov;
    }
}

/// The scripted replay of the reference captures (`--race`, `--gender`, `--step`), `--lua`, the
/// virtual pointer (`--hover`, `--click`, `--wheel`) and `--cshot`.
fn run_script(
    mut commands: Commands,
    screen: Option<ResMut<CreateScreen>>,
    ui: Option<NonSend<UiRuntime>>,
    opts: Res<Options>,
    time: Res<Time>,
    mut pointer: ResMut<aa_ui::UiVirtualPointer>,
    mut release_next: Local<bool>,
    mut wheel_to: Local<Option<f32>>,
    mut active_drags: Local<Vec<(Vec2, Vec2, u32)>>,
) {
    let (Some(mut screen), Some(ui)) = (screen, ui) else { return };
    if !screen.ui_built {
        return;
    }
    if std::mem::take(&mut *release_next) {
        pointer.release = true;
    }
    screen.ui_time += time.delta_secs();
    let t = screen.ui_time;
    let mut base = 0;
    let mut once = |idx: usize, at: f32| {
        let due = t >= at && !screen.script_done[idx];
        if due {
            screen.script_done[idx] = true;
        }
        due
    };
    if opts.goto || opts.offline {
        let race = match opts.race {
            Race::Nuian => "RACE_NUIAN",
            Race::Elf => "RACE_ELF",
            Race::Ferre => "RACE_FERRE",
            Race::Hariharan => "RACE_HARIHARAN",
        };
        let gender = if opts.gender == Gender::Female { 2 } else { 1 };
        let exec = |code: &str| {
            if let Err(e) = ui.exec("create script", code) {
                warn!("character create: script: {e}");
            }
        };
        if once(0, 1.0) {
            exec(&format!(
                "for k = 1, #RACE_TYPE do if RACE_TYPE[k] == {race} then step1.content.raceBtn[k].OnClick() end end
                 step1.content.rightPanel.genderBtn[{gender}].OnClick()"
            ));
        }
        if opts.step >= 2 && once(1, 2.5) {
            exec("characterCreate.OnNextStep()");
        }
        if opts.step >= 3 && once(2, 5.0) {
            exec("characterCreate.OnNextStep()");
        }
    }
    base += 8;
    for (i, (code, at)) in opts.lua.iter().enumerate() {
        if once(base + i, *at) {
            if let Err(e) = ui.exec("@--clua", code) {
                warn!("character create: --clua: {e}");
            }
        }
    }
    base += opts.lua.len();
    for (i, (x, y, at, click)) in opts.pointer.iter().enumerate() {
        if once(base + i, *at) {
            pointer.pos = Some(Vec2::new(*x, *y));
            if *click {
                pointer.press = true;
                *release_next = true;
            }
            info!("character create: {} {x} {y}", if *click { "click" } else { "hover" });
        }
    }
    base += opts.pointer.len();
    for (i, (from, to, at)) in opts.drags.iter().enumerate() {
        if once(base + i, *at) {
            active_drags.push((*from, *to, 0));
            info!("character create: drag {from} -> {to}");
        }
    }
    base += opts.drags.len();
    // a drag takes one frame to move to its start, one to press, 8 to move, one to release
    for (from, to, frame) in active_drags.iter_mut() {
        match *frame {
            0 => pointer.pos = Some(*from),
            1 => pointer.press = true,
            2..=9 => pointer.pos = Some(from.lerp(*to, (*frame - 1) as f32 / 8.0)),
            _ => pointer.release = true,
        }
        *frame += 1;
    }
    active_drags.retain(|(_, _, frame)| *frame <= 10);
    for (i, (d, at)) in opts.wheel.iter().enumerate() {
        if once(base + i, *at) {
            *wheel_to = Some(*d);
        }
    }
    base += opts.wheel.len();
    if let Some(d) = wheel_to.take() {
        // the wheel goes to the screen's background window like the OS wheel would
        let code = if d < 0.0 { "characterCreate.bgWindow:OnWheelDown()" } else { "characterCreate.bgWindow:OnWheelUp()" };
        if let Err(e) = ui.exec("@--wheel", code) {
            warn!("character create: --wheel: {e}");
        }
    }
    for (i, (text, at)) in opts.typed.iter().enumerate() {
        if once(base + i, *at) {
            ui.type_text(text);
            info!("character create: typed {} character(s)", text.chars().count());
        }
    }
    base += opts.typed.len();
    for (i, (path, at)) in opts.shots.iter().enumerate() {
        if once(base + i, *at) {
            if let Some(dir) = std::path::Path::new(path).parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            commands
                .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
                .observe(bevy::render::view::screenshot::save_to_disk(path.clone()));
            info!("character create: screenshot -> {path}");
        }
    }
}

// ---- network ----------------------------------------------------------------------------------

/// The server's answers to the create request.
fn net_events(
    screen: Option<ResMut<CreateScreen>>,
    ui: Option<NonSend<UiRuntime>>,
    mut events: MessageReader<NetEvent>,
    mut next: ResMut<NextState<Screen>>,
    mut exit: MessageWriter<AppExit>,
    opts: Res<Options>,
) {
    let Some(screen) = screen else {
        events.clear();
        return;
    };
    for NetEvent(event) in events.read() {
        match event {
            Event::CharacterCreated(c) => {
                info!("character create: the server created {} (id {}, race {}, gender {})", c.name, c.id, c.race, c.gender);
                lock(&screen.shared).create_pending = false;
                if opts.exit_after_create {
                    exit.write(AppExit::Success);
                }
                next.set(Screen::CharacterSelect);
            }
            Event::CharacterCreateFailed { reason } => {
                warn!("character create: the server refused the character (reason {reason})");
                lock(&screen.shared).create_pending = false;
                // the keys of the client's own message boxes
                let key = match reason {
                    3 => "error_char_name_pending",
                    4 => "error_char_name_duplicate",
                    5 => "error_char_name_format",
                    _ => "error_char_name_forbidden",
                };
                if let Some(ui) = ui.as_deref() {
                    ui.queue_event("CREATE_CHARACTER_FAILED", key);
                }
                if opts.exit_after_create {
                    exit.write(AppExit::error());
                }
            }
            _ => {}
        }
    }
}
