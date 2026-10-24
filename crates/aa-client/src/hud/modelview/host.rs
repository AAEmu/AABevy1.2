//! Root-owned offscreen actors. Widget IDs fence runtimes; revisions fence workers.
use super::{
    camera::{self, Height, Pose, Scene},
    trial::Trial,
};
use crate::ingame::{appearance, data::Db, weapons};
use aa_anim::{
    Animator, Cal, Character, CharacterAssets, ClipLibrary, Gender, Race, SampleOptions,
};
use aa_cry::{
    CryLoader, PreparedModel,
    scene::{CryAssets, ModelOptions, Shading, spawn_loaded},
};
use aa_net::{
    model::{Equipment, ModelParams},
    state::Player,
};
use aa_ui::{ModelViewId, ModelViewImage, ModelViewOp, UiRuntime};
use anyhow::{Context, Result};
use bevy::{
    camera::{ClearColorConfig, RenderTarget, visibility::RenderLayers},
    ecs::system::SystemState,
    prelude::*,
    render::{render_resource::TextureFormat, view::Msaa},
    window::PrimaryWindow,
};
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Mutex,
        mpsc::{self, Receiver, Sender},
    },
};

// Application resource limits, not native limits. Each target has its own light layer.
const MAX_VIEWS: usize = 8;
const FIRST_LAYER: usize = 128;
const MAX_JOBS: usize = 2;

fn render_layer(index: usize) -> RenderLayers {
    RenderLayers::from_layers(&[index])
}

#[derive(Clone)]
pub(super) struct Source {
    pub character: u32,
    pub object: u32,
    pub race: Race,
    pub gender: Gender,
    pub params: ModelParams,
    pub equipment: Equipment,
}

#[derive(Component)]
struct PreviewActor(usize);

/// Offscreen widget views never participate in world interaction/projection.
#[derive(Component, Clone, bevy::render::extract_component::ExtractComponent)]
#[extract_app(bevy::render::RenderApp)]
pub(crate) struct ModelViewCamera;

enum CameraOp {
    Shop,
    Adjust(Vec3),
}
struct View {
    source: Option<Source>,
    trial: Option<Trial>,
    bound: bool,
    revision: u64,
    epoch: u64,
    settled: u64,
    create_model: bool,
    yaw: f32,
    animation: Option<(String, bool)>,
    stopped: bool,
    scene: Option<Scene>,
    pose: Option<Pose>,
    camera_ops: Vec<CameraOp>,
    reference_heel: Option<f32>,
    actor: Option<Entity>,
    carried: Vec<Entity>,
    camera: Option<Entity>,
    light: Option<Entity>,
    target: Option<(Handle<Image>, UVec2)>,
    layer: Option<usize>,
}
impl View {
    fn new(layer: Option<usize>) -> Self {
        Self {
            source: None,
            trial: None,
            bound: false,
            revision: 0,
            epoch: 0,
            settled: 0,
            create_model: false,
            yaw: 0.,
            animation: None,
            stopped: false,
            scene: None,
            pose: None,
            camera_ops: Vec::new(),
            reference_heel: None,
            actor: None,
            carried: Vec::new(),
            camera: None,
            light: None,
            target: None,
            layer,
        }
    }
    fn change(&mut self) {
        self.revision = self
            .revision
            .checked_add(1)
            .expect("modelview revision exhausted");
    }
    fn clear(&mut self, world: &mut World) {
        if let Some(root) = self.actor.take() {
            world.despawn(root);
        }
        self.carried.clear();
        // Native Clear releases the object binding, retaining both copied
        // appearance payloads for ResetEquips. Session retirement discards them.
        self.bound = false;
        self.reference_heel = Some(0.); // cleared native race/gender have no reference row
        self.create_model = false;
        self.animation = None;
        self.stopped = false;
        self.change();
        self.settled = self.revision;
        if let Some(camera) = self.camera.and_then(|e| world.get_mut::<Camera>(e)) {
            camera.into_inner().is_active = false;
        }
    }
    fn retire(&mut self, world: &mut World) {
        self.clear(world);
        self.source = None;
        self.trial = None;
    }
    fn destroy(mut self, world: &mut World) {
        self.retire(world);
        for entity in [self.camera, self.light].into_iter().flatten() {
            world.despawn(entity);
        }
        // Handles are released normally, including toolkit's image association.
    }
}

struct Prepared {
    assets: CharacterAssets,
    models: Vec<PreparedModel>,
    carried: Vec<weapons::CarriedItem>,
    scene: Scene,
    reference: f32,
}
struct Ready {
    view: ModelViewId,
    revision: u64,
    result: std::result::Result<Prepared, String>,
}
#[derive(Resource)]
struct Host {
    views: HashMap<ModelViewId, View>,
    tx: Sender<Ready>,
    rx: Mutex<Receiver<Ready>>,
    pending: HashSet<(ModelViewId, u64)>,
    epoch: u64,
    receipts: bevy::ecs::message::MessageCursor<aa_net::NetEvent>,
}
impl Default for Host {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            views: HashMap::new(),
            tx,
            rx: Mutex::new(rx),
            pending: HashSet::new(),
            epoch: 0,
            receipts: default(),
        }
    }
}

fn own_source(world: &World) -> Option<Source> {
    let spawn = world.get_resource::<Player>()?.spawn.as_ref()?;
    let character = spawn.character();
    // A transformed/unknown model is not permission to substitute a base race.
    let (race, gender) = Race::from_model_id(spawn.unit.model_id)?;
    let equipment =
        crate::hud::received_equipment(world.get_non_send::<crate::hud::api::HudBridge>()?)?;
    let params = if spawn.unit.model_params.kind > 0 {
        spawn.unit.model_params.clone()
    } else {
        character.model_params.clone()
    };
    Some(Source {
        character: character.id,
        object: spawn.obj_id,
        race,
        gender,
        params,
        equipment,
    })
}

#[cfg(test)]
mod tests;

fn prepare(
    db: &Db,
    loader: &CryLoader,
    source: Source,
    equipment: Equipment,
    bound: bool,
) -> Result<Prepared> {
    let (assets, reference) = {
        let game = db
            .game
            .lock()
            .map_err(|_| anyhow::anyhow!("preview database lock poisoned"))?;
        let desc = Context::context(
            appearance::describe(
                &game,
                source.race,
                source.gender,
                &equipment,
                &source.params,
            ),
            "cannot describe received preview appearance",
        )?;
        (
            appearance::resolve(&game, &desc, &equipment)?,
            if bound {
                game.modelview_reference_heel(source.race, source.gender)?
            } else {
                0.
            },
        )
    };
    aa_anim::character::prewarm_character_face(&assets)?;
    let carried = weapons::carried(db, &equipment);
    let mut files: Vec<_> = assets
        .parts
        .iter()
        .map(|p| (p.path.clone(), p.material.clone()))
        .collect();
    for part in &assets.parts {
        files.extend(
            part.attachments
                .iter()
                .filter_map(crate::ingame::attachments::model),
        );
    }
    for item in &carried {
        files.extend(weapons::model_files(&item.path));
    }
    let mut models = Vec::new();
    for (path, material) in files {
        models.push(loader.prepare_model(
            &path,
            &ModelOptions {
                material,
                cry_space: true,
                shading: Shading::Full,
                ..default()
            },
        )?);
    }
    Ok(Prepared {
        assets,
        models,
        carried,
        scene: Scene::load(source.race, source.gender)?,
        reference,
    })
}

fn file_time() -> i64 {
    let unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    (116_444_736_000_000_000u64
        + unix.as_secs() * 10_000_000
        + u64::from(unix.subsec_nanos() / 100)) as i64
}

fn dispatch(
    world: &mut World,
    view: &mut View,
    op: ModelViewOp,
    source: Option<&Source>,
    db: Option<&Db>,
) {
    match op {
        ModelViewOp::Init { unit, create_model } => {
            if unit != "player" {
                return;
            }
            let Some(source) = source else { return };
            // Original same-object Init does not recopy later equipment or rebuild.
            if view.bound
                && view.source.as_ref().is_some_and(|old| {
                    old.object == source.object && old.character == source.character
                })
            {
                return;
            }
            view.clear(world);
            view.scene = None;
            view.reference_heel = None;
            view.source = Some(source.clone());
            view.trial = Some(Trial::new(source.equipment.clone()));
            view.bound = true;
            view.create_model = create_model;
            view.change();
        }
        ModelViewOp::ClearModel => view.clear(world),
        ModelViewOp::SetIngameShopMode(true) if view.bound => view.camera_ops.push(CameraOp::Shop),
        ModelViewOp::AdjustCameraPos(v) => {
            view.camera_ops.push(CameraOp::Adjust(Vec3::from_array(v)))
        }
        ModelViewOp::SetRotation(yaw) => {
            if yaw.is_finite() {
                view.yaw = yaw
            }
        }
        ModelViewOp::EquipItem(id) => {
            let (Some(db), Some(source), Some(trial)) = (db, &view.source, &mut view.trial) else {
                return;
            };
            let result = db
                .game
                .lock()
                .map_err(|_| "preview database lock poisoned".to_owned())
                .and_then(|game| {
                    trial
                        .equip(
                            &game,
                            if view.bound {
                                match source.gender {
                                    Gender::Male => 1,
                                    Gender::Female => 2,
                                }
                            } else {
                                0
                            },
                            id,
                            file_time(),
                        )
                        .map_err(|e| e.to_string())
                });
            match result {
                Ok(true) => {
                    view.create_model = true;
                    view.change();
                }
                Ok(false) => {}
                Err(error) => warn!("modelview trial {id}: {error}"),
            }
        }
        ModelViewOp::ResetEquips => {
            // Native tests the copied model header, not drawable readiness.
            if view.source.is_some()
                && let Some(trial) = &mut view.trial
            {
                trial.reset();
                view.create_model = true;
                view.change();
            }
        }
        ModelViewOp::PlayAnimation { name, looping } => {
            view.animation = Some((name, looping));
            view.stopped = false;
        }
        ModelViewOp::StopAnimation => {
            view.stopped = true;
        }
        ModelViewOp::SetTextureSize(_)
        | ModelViewOp::SetIngameShopMode(true)
        | ModelViewOp::SetIngameShopMode(false)
        | ModelViewOp::Destroy => {}
    }
}

fn bind_animation(world: &mut World, view: &View) -> Result<()> {
    let Some(root) = view.actor else {
        return Ok(());
    };
    let Some(animator) = world.get::<Animator>(root) else {
        return Ok(());
    };
    let rig = animator.rig.clone();
    if let Some((name, looping)) = &view.animation {
        let cal = Context::context(world.get::<Character>(root), "preview character missing")?
            .assets
            .cal
            .clone();
        let cal = Cal::shared(&cal)?;
        let entry =
            Context::with_context(cal.animation(name), || format!("preview animation {name}"))?;
        let clip = world.resource::<ClipLibrary>().bound(
            &rig,
            &entry.path,
            SampleOptions {
                position_scale: entry.position_scale(cal.base_height),
                lock_root_motion: true,
            },
        )?;
        // Resolve completely before changing the retained actor's playback.
        let mut animator = world.get_mut::<Animator>(root).unwrap();
        animator.random_idle = None;
        animator.play(clip, *looping, 0.);
        animator.paused = view.stopped;
    } else {
        // Do not present the spawner's unrelated login random-idle as a requested clip.
        world.get_mut::<Animator>(root).unwrap().paused = true;
    }
    Ok(())
}

fn resume_retained_animation(world: &mut World, view: &View, play_requested: bool) {
    if view.stopped {
        if let Some(mut animator) = view.actor.and_then(|root| world.get_mut::<Animator>(root)) {
            animator.paused = true;
        }
        return;
    }
    if !play_requested
        && !view
            .actor
            .and_then(|root| world.get::<Animator>(root))
            .is_some_and(|a| a.paused)
    {
        return;
    }
    if let Err(error) = bind_animation(world, view) {
        // A failed requested clip must not freeze the last successful clip.
        // bind_animation is atomic, so its clocks and layers are still intact.
        if let Some(mut animator) = view.actor.and_then(|root| world.get_mut::<Animator>(root)) {
            animator.paused = false;
        }
        warn!("modelview: retained playback resumed after binding failure: {error}");
    }
}

fn apply(world: &mut World, view: &mut View, prepared: Prepared) -> Result<()> {
    let layer = Context::context(view.layer, "preview layer not admitted")?;
    let expected = prepared.assets.parts.len();
    let mut state = SystemState::<(Commands, CryAssets, Res<ClipLibrary>)>::new(world);
    let root = {
        let (mut commands, mut cry, clips) = state.get_mut(world)?;
        for model in prepared.models {
            cry.finish_model(model);
        }
        aa_anim::spawn_character_with_facial(
            &mut commands,
            &mut cry,
            &clips,
            prepared.assets,
            Transform::default(),
        )?
    };
    state.apply(world);
    // Spawn historically warns/skips a part. A preview must withhold that partial actor.
    if !world
        .get::<Character>(root)
        .is_some_and(|c| c.parts.len() == expected)
    {
        world.despawn(root);
        anyhow::bail!("incomplete preview character");
    }
    let result = (|| -> Result<Vec<Entity>> {
        let animator = Context::context(world.get::<Animator>(root), "missing preview animator")?;
        let (rig, bones) = (animator.rig.clone(), animator.bones.clone());
        // Native selector policy is separate from the existing Bevy attachment
        // transforms; the latter do not prove the original assembly endpoint.
        let selector = view.trial.as_ref().map_or(0, |trial| trial.selector);
        let mut state = SystemState::<(Commands, CryAssets)>::new(world);
        let spawned = (|| -> Result<Vec<Entity>> {
            let (mut commands, mut cry) = state.get_mut(world)?;
            let mut spawned = Vec::new();
            for item in prepared.carried {
                let held = match item.grip {
                    weapons::Grip::Blade | weapons::Grip::Shield => selector == 1,
                    weapons::Grip::Bow => selector == 2,
                    weapons::Grip::Instrument(_) => selector == 4,
                    weapons::Grip::Pack => false,
                };
                let bone = if held {
                    item.hand_bone.unwrap_or(item.bone)
                } else {
                    item.bone
                };
                let bone = Context::context(
                    rig.skeleton
                        .bones
                        .iter()
                        .position(|b| b.name == bone)
                        .and_then(|i| bones.get(i))
                        .copied(),
                    "preview attachment bone missing",
                )?;
                let transform = if held {
                    item.hand_transform
                } else {
                    item.transform
                };
                let hidden = !held
                    && (matches!(
                        item.grip,
                        weapons::Grip::Shield | weapons::Grip::Instrument(_)
                    ) || item.hide_sheathed);
                for (path, material) in weapons::model_files(&item.path) {
                    let options = ModelOptions {
                        material,
                        cry_space: true,
                        shading: Shading::Full,
                        ..default()
                    };
                    let loaded = cry.load_model(&path, &options)?;
                    let entity = spawn_loaded(&mut commands, &path, &loaded, &options);
                    commands.entity(entity).insert((
                        transform,
                        ChildOf(bone),
                        if hidden {
                            Visibility::Hidden
                        } else {
                            Visibility::Inherited
                        },
                    ));
                    spawned.push(entity);
                }
            }
            Ok(spawned)
        })();
        // Apply even on an attachment failure so every reserved child belongs
        // to this candidate before retirement; no orphaned queued entities.
        state.apply(world);
        spawned
    })();
    let carried = match result {
        Ok(items) => items,
        Err(error) => {
            world.despawn(root);
            return Err(error);
        }
    };
    let old = view.actor.replace(root);
    let old_carried = std::mem::replace(&mut view.carried, carried);
    if let Err(error) = bind_animation(world, view) {
        world.despawn(root);
        view.actor = old;
        view.carried = old_carried;
        return Err(error);
    }
    if let Some(old) = old {
        world.despawn(old);
    }
    world
        .entity_mut(root)
        .insert((Name::new("Widget modelview actor"), PreviewActor(layer)));
    view.scene = Some(prepared.scene);
    view.reference_heel = Some(prepared.reference);
    Ok(())
}

fn drive(world: &mut World) {
    let suspended = world.get_non_send::<crate::hud::SuspendedStage>().is_some();
    let work = world
        .get_non_send::<UiRuntime>()
        .map(UiRuntime::take_modelview_work);
    // Cloning all received records is an Init operation, not per-frame work.
    let source = work
        .as_ref()
        .is_some_and(|w| {
            w.commands
                .iter()
                .any(|c| matches!(&c.op,ModelViewOp::Init{unit,..} if unit=="player"))
        })
        .then(|| own_source(world))
        .flatten();
    let owner = world
        .get_resource::<Player>()
        .and_then(|p| p.spawn.as_ref())
        .map(|p| (p.character().id, p.obj_id));
    let db = world.get_resource::<Db>().cloned();
    let screen = {
        let mut q = world.query_filtered::<&Window, With<PrimaryWindow>>();
        q.single(world)
            .ok()
            .map(|w| UVec2::new(w.physical_width(), w.physical_height()))
    };
    world.resource_scope(|world, mut host: Mut<Host>| {
        if let Some(messages) = world.get_resource::<Messages<aa_net::NetEvent>>() {
            let changes = host
                .receipts
                .read(messages)
                .filter(|aa_net::NetEvent(e)| {
                    matches!(
                        e,
                        aa_net::Event::PlayerSpawn(_) | aa_net::Event::PhaseChanged(_)
                    )
                })
                .count();
            host.epoch = host
                .epoch
                .checked_add(changes as u64)
                .expect("modelview owner epoch exhausted");
        }
        let epoch = host.epoch;
        for view in host.views.values_mut() {
            if view
                .source
                .as_ref()
                .is_some_and(|s| view.epoch != epoch || owner != Some((s.character, s.object)))
            {
                view.retire(world);
            }
        }
        let live: HashSet<_> = work
            .as_ref()
            .map(|w| w.surfaces.iter().map(|s| s.view).collect())
            .unwrap_or_default();
        let gone: Vec<_> = host
            .views
            .keys()
            .filter(|id| !live.contains(id))
            .copied()
            .collect();
        for id in gone {
            if suspended {
                // Teleport replaces UiRuntime temporarily, then restores its
                // exact widget IDs. Keep revisions and copied state through
                // that absence; normal retirement resumes with the HUD.
                if let Some(mut camera) = host.views[&id]
                    .camera
                    .and_then(|e| world.get_mut::<Camera>(e))
                {
                    camera.is_active = false;
                }
            } else {
                host.views.remove(&id).unwrap().destroy(world);
            }
        }
        let Some(work) = work else { return };
        for surface in &work.surfaces {
            // Hidden/uninitialized toolkit surfaces retain ordered state,
            // but do not consume a scene/target permit.
            host.views
                .entry(surface.view)
                .or_insert_with(|| View::new(None));
        }
        let mut animation_dirty = HashMap::new();
        for command in work.commands {
            if matches!(command.op, ModelViewOp::Destroy) {
                if let Some(view) = host.views.remove(&command.view) {
                    view.destroy(world);
                }
            } else if let Some(view) = host.views.get_mut(&command.view) {
                match &command.op {
                    ModelViewOp::PlayAnimation { .. } => {
                        animation_dirty.insert(command.view, true);
                    }
                    ModelViewOp::StopAnimation => {
                        animation_dirty.insert(command.view, false);
                    }
                    _ => {}
                }
                dispatch(world, view, command.op, source.as_ref(), db.as_ref());
                view.epoch = epoch;
            }
        }
        let waiting: Vec<_> = host
            .views
            .iter()
            .filter(|(_, v)| v.create_model && v.source.is_some() && v.layer.is_none())
            .map(|(id, _)| *id)
            .collect();
        for id in waiting {
            let Some(layer) = (FIRST_LAYER..FIRST_LAYER + MAX_VIEWS)
                .find(|layer| host.views.values().all(|v| v.layer != Some(*layer)))
            else {
                break;
            };
            host.views.get_mut(&id).unwrap().layer = Some(layer);
        }
        let ready: Vec<_> = host.rx.lock().unwrap().try_iter().collect();
        for ready in ready {
            host.pending.remove(&(ready.view, ready.revision));
            let Some(view) = host.views.get_mut(&ready.view) else {
                continue;
            };
            if view.revision != ready.revision || !view.create_model {
                continue;
            }
            view.settled = ready.revision;
            let control = animation_dirty.remove(&ready.view);
            match ready
                .result
                .and_then(|p| apply(world, view, p).map_err(|e| e.to_string()))
            {
                Ok(()) => info!(
                    "modelview: prepared {:?} revision {}",
                    ready.view, ready.revision
                ),
                Err(error) => {
                    resume_retained_animation(world, view, control == Some(true));
                    warn!("modelview: previous complete appearance retained: {error}");
                }
            }
        }
        for (id, play) in animation_dirty {
            if let Some(view) = host.views.get(&id) {
                if play && view.revision == view.settled {
                    if let Err(e) = bind_animation(world, view) {
                        warn!("modelview: {e}");
                    }
                } else if let Some(mut animator) =
                    view.actor.and_then(|root| world.get_mut::<Animator>(root))
                {
                    animator.paused = true;
                }
            }
        }
        for surface in work.surfaces {
            let Some(view) = host.views.get_mut(&surface.view) else {
                continue;
            };
            if let Some(scene) = &view.scene {
                for op in view.camera_ops.drain(..) {
                    match op {
                        CameraOp::Shop => view.pose = Some(scene.shop_pose()),
                        CameraOp::Adjust(v) => {
                            if let Some(p) = &mut view.pose {
                                p.adjust(v);
                            }
                        }
                    }
                }
                if let Some(actor) = view.actor
                    && let Some(transform) = scene.actor_transform(view.yaw)
                {
                    world.entity_mut(actor).insert(transform);
                }
            }
            present(world, view, &surface, screen);
        }
        let Some(db) = db else { return };
        let candidates: Vec<_> = host
            .views
            .iter()
            .filter(|(id, v)| {
                v.create_model
                    && v.layer.is_some()
                    && v.revision != v.settled
                    && !host.pending.iter().any(|(pending, _)| pending == *id)
            })
            .map(|(id, _)| *id)
            .take(MAX_JOBS.saturating_sub(host.pending.len()))
            .collect();
        if candidates.is_empty() {
            return;
        }
        let loader = {
            let mut state = SystemState::<CryAssets>::new(world);
            let loader = state.get_mut(world).unwrap().loader();
            state.apply(world);
            loader
        };
        for id in candidates {
            let view = &host.views[&id];
            let (Some(source), Some(trial)) = (&view.source, &view.trial) else {
                continue;
            };
            let (source, equipment, revision, bound) = (
                source.clone(),
                trial.equipment.clone(),
                view.revision,
                view.bound,
            );
            let (db, loader, tx) = (db.clone(), loader.clone(), host.tx.clone());
            host.pending.insert((id, revision));
            std::thread::spawn(move || {
                let result =
                    prepare(&db, &loader, source, equipment, bound).map_err(|e| e.to_string());
                let _ = tx.send(Ready {
                    view: id,
                    revision,
                    result,
                });
            });
        }
    });
}

fn present(
    world: &mut World,
    view: &mut View,
    surface: &aa_ui::ModelViewSurface,
    screen: Option<UVec2>,
) {
    let image = (|| -> Option<ModelViewImage> {
        let actor = view.actor?;
        let extent =
            camera::texture_extent(IVec2::from_array(surface.requested_texture_size), screen?)?;
        let current = world.get::<Character>(actor)?.assets.heel;
        let height = match world
            .get_resource::<super::super::salon::AppliedMode>()
            .map(|mode| mode.enabled())
        {
            None => Height::Unknown,
            Some(false) => Height::Disabled,
            Some(true) => Height::Enabled {
                current,
                reference: view.reference_heel?,
            },
        };
        let (transform, projection) = view.scene.as_ref()?.camera(view.pose?, extent, height)?;
        if view.target.as_ref().is_none_or(|(_, size)| *size != extent) {
            let image =
                Image::new_target_texture(extent.x, extent.y, TextureFormat::Rgba8UnormSrgb, None);
            view.target = Some((world.resource_mut::<Assets<Image>>().add(image), extent));
        }
        let target = view.target.as_ref()?.0.clone();
        let layer = render_layer(view.layer?);
        if view.camera.is_none() {
            view.camera = Some(
                world
                    .spawn((
                        Camera3d::default(),
                        ModelViewCamera,
                        Camera {
                            order: -20,
                            clear_color: ClearColorConfig::Custom(Color::NONE),
                            ..default()
                        },
                        RenderTarget::Image(target.clone().into()),
                        Projection::Perspective(projection.clone()),
                        transform,
                        Msaa::Off,
                        layer.clone(),
                        AmbientLight {
                            color: Color::srgb(0.75, 0.8, 0.9),
                            brightness: 500.,
                            ..default()
                        },
                        aa_cry::stage::login_color_grading(),
                    ))
                    .id(),
            );
            view.light = Some(
                world
                    .spawn((
                        DirectionalLight {
                            illuminance: 10000.,
                            shadow_maps_enabled: false,
                            ..default()
                        },
                        Transform::from_xyz(-3., 5., 4.).looking_at(Vec3::Y, Vec3::Y),
                        layer,
                    ))
                    .id(),
            );
        }
        let camera = view.camera?;
        world.entity_mut(camera).insert((
            RenderTarget::Image(target.clone().into()),
            Projection::Perspective(projection),
            transform,
        ));
        world.get_mut::<Camera>(camera)?.is_active = surface.paint_admitted;
        Some(ModelViewImage {
            image: target,
            extent_px: extent.to_array(),
        })
    })();
    if image.is_none()
        && let Some(mut camera) = view.camera.and_then(|e| world.get_mut::<Camera>(e))
    {
        camera.is_active = false;
    }
    if let Some(rt) = world.get_non_send::<UiRuntime>() {
        rt.set_modelview_image(surface.view, image);
    }
}

// Layers do not inherit. Include late-created model/CDF/effect children before extraction.
fn layers(world: &mut World) {
    let roots: Vec<_> = world
        .query::<(Entity, &PreviewActor)>()
        .iter(world)
        .map(|(e, p)| (e, p.0))
        .collect();
    for (root, layer) in roots {
        let wanted = render_layer(layer);
        let mut stack = vec![root];
        while let Some(entity) = stack.pop() {
            if let Some(children) = world.get::<Children>(entity) {
                stack.extend(children.iter());
            }
            if world
                .get::<RenderLayers>(entity)
                .is_none_or(|v| v != &wanted)
            {
                world.entity_mut(entity).insert(wanted.clone());
            }
        }
    }
}

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<Host>()
        .add_systems(Update, drive.in_set(aa_ui::UiModelViews));
    app.add_systems(
        PostUpdate,
        layers
            .after(bevy::transform::TransformSystems::Propagate)
            .before(bevy::camera::visibility::VisibilitySystems::CheckVisibility),
    );
}

pub(super) fn fixture_ready(world: &World) -> bool {
    world
        .resource::<Host>()
        .views
        .values()
        .any(|view| view.actor.is_some() && view.target.is_some() && view.settled == view.revision)
}

pub(super) fn fixture_identity(world: &World) -> Option<(Entity, u64)> {
    world
        .resource::<Host>()
        .views
        .values()
        .find_map(|view| Some((view.actor?, view.revision)))
}

pub(super) fn verify_fixture(world: &World, case: &str) -> bool {
    let host = world.resource::<Host>();
    let Some(view) = host.views.values().next() else {
        return false;
    };
    if view.settled != view.revision {
        return false;
    }
    if matches!(case, "clear" | "baseline") {
        assert!(
            view.actor.is_none() && !view.bound && view.source.is_some() && view.trial.is_some()
        );
        assert!(view.target.is_some(), "Clear retains target allocation");
        assert!(!world.get::<Camera>(view.camera.unwrap()).unwrap().is_active);
    } else {
        let Some(actor) = view.actor else {
            return false;
        };
        let character = world.get::<Character>(actor).unwrap();
        assert_eq!(character.assets.model_id, 11);
        assert_eq!(character.parts.len(), character.assets.parts.len());
        assert!(world.get::<Animator>(actor).unwrap().current.is_some());
        let camera = view.camera.unwrap();
        assert!(world.get::<ModelViewCamera>(camera).is_some());
        assert_eq!(
            world.get::<Camera>(camera).unwrap().is_active,
            case != "hidden"
        );
        assert_eq!(
            view.target.as_ref().unwrap().1,
            if case == "resize" {
                UVec2::new(700, 600)
            } else {
                UVec2::splat(512)
            }
        );
        let equipment = &view.trial.as_ref().unwrap().equipment;
        match case {
            "weapon" => assert_eq!(
                equipment
                    .get(aa_net::model::slot::MAINHAND)
                    .unwrap()
                    .template_id,
                5660
            ),
            "staff" => assert_eq!(
                equipment
                    .get(aa_net::model::slot::MAINHAND)
                    .unwrap()
                    .template_id,
                6102
            ),
            "armor" => assert_eq!(
                equipment
                    .get(aa_net::model::slot::COSPLAY)
                    .unwrap()
                    .template_id,
                8000028
            ),
            "reset" | "clear-reset" | "clear-reset-applied" | "lifecycle" => {
                assert!(equipment.get(aa_net::model::slot::COSPLAY).is_none());
                assert_eq!(
                    equipment
                        .get(aa_net::model::slot::MAINHAND)
                        .unwrap()
                        .template_id,
                    5569
                );
            }
            _ => {}
        }
        if case.ends_with("-applied") {
            assert!(
                world
                    .resource::<super::super::salon::AppliedMode>()
                    .enabled()
            );
            let reference = if case == "clear-reset-applied" {
                0.
            } else {
                0.07999999821186066_f32
            };
            assert_eq!(view.reference_heel, Some(reference));
            let (expected, _) = view
                .scene
                .as_ref()
                .unwrap()
                .camera(
                    view.pose.unwrap(),
                    view.target.as_ref().unwrap().1,
                    Height::Enabled {
                        current: character.assets.heel,
                        reference,
                    },
                )
                .unwrap();
            let actual = world.get::<Transform>(camera).unwrap();
            assert!(actual.translation.abs_diff_eq(expected.translation, 1e-6));
            assert_eq!(view.bound, case != "clear-reset-applied");
        }
    }
    true
}
