//! The beauty salon: the original `x2ui/customizing/beautyshop` window over the character's own look.
//!
//! ## Flow
//!
//! 1. The salon helper NPC (`npc_interaction_sets` 12, skill 22780 "Enter the beauty salon") answers the talk with
//!    that skill in its `SCNpcInteractionSkillList`. [`offers_salon`] tells the talk code which NPC templates do;
//!    the answer raises the original `DLG_TASK_CONFIRM_ENTER_BEAUTYSHOP` dialog (native `393bfab0`), and its Ok
//!    casts the skill at the NPC. (The cash shop's path, `SALON.bypass()`, sends `CSEnterBeautySalon` instead.)
//! 2. The server answers `SCToggleBeautyshopResponse` (1). The world camera is put aside, the HUD windows hidden,
//!    and the stage of the character screens (`login::stage3d`) shows the player's character in its own look
//!    over the salon's backdrop; the original window is shown by `TOGGLE_BEAUTYSHOP(true)`
//!    (native `3931ef40`, the event is 580).
//! 3. The window edits a [`Customizer`] (the character-create state) through the model view widget methods of
//!    `salon.lua`: the lists, colours and sliders are the create screen's, the hair list also holds the salon-only
//!    styles. *Confirm* (`PayBeautyShop`) sends `CSBeautyshopData` (hair item + appearance); the server takes a
//!    Salon Certificate and answers `SCCharacterGenderAndModelModified`, after which the client leaves
//!    (`CSExitBeautySalon`, as the server expects). *Cancel* (`TerminateBeautyShop`) leaves at once.
//! 4. `SCToggleBeautyshopResponse` (0) ends it: `TOGGLE_BEAUTYSHOP(false)`, the camera and the windows come back.
//!    A toggle the client did not ask for (the buff went, the player was attacked) raises
//!    `BEAUTYSHOP_CLOSE_BY_SYSTEM` first, which shows the original notice.
//!
//! The changed look is also what the player is drawn with afterwards: the spawn data is updated and the
//! player model built again (`ingame::player::spawn_player`) at the place it stands.

use std::collections::HashSet;
use std::sync::{Mutex, MutexGuard, OnceLock};

use aa_anim::db::GameDb;
use aa_anim::{ClipLibrary, Costume, Gender, Race};
use aa_cry::scene::CryAssets;
use aa_cry::stage::StageLighting;
use aa_net::interaction::{Interact, InteractEvent};
use aa_net::model::{Equipment, Item, ModelParams, slot};
use aa_net::salon::{SALON_CERTIFICATE, SalonCommand, SalonEvent};
use aa_net::state::{Inventory, Player};
use aa_net::{Command, Event, NetCommand, NetEvent};
use aa_ui::UiRuntime;
use aa_ui::render::UiCamera;
use bevy::camera::visibility::RenderLayers;
use bevy::camera::{ClearColorConfig, Hdr};
use bevy::prelude::*;
use bevy::render::view::Msaa;
use mlua::Lua;

use crate::Screen;
use crate::ingame::appearance;
use crate::ingame::data::Db;
use crate::ingame::player::{LocalPlayer, Mover, PlayerBuilt};
use crate::login::create::api::{Shared, SharedRef, lock};
use crate::login::create::look::{Customizer, HairStyle, Look, MODIFIER_LEN, Scar, decal};
use crate::login::create::poses;
use crate::login::create::preview::PreviewCache;
use crate::login::stage3d::{self, STAGE_LAYER, StageCamera, StageSlot, StageSubject};

/// The skill the salon helper offers: `skills.id` 22780, the effect `EnterBeautyshop`.
pub const ENTER_SKILL: u32 = 22780;
/// Where the backdrop of the salon is (`SetModelViewBackground("ui/beautyshop/bg.dds")`, the locale's variant).
const BACKDROP: &str = "ui/beautyshop/en_us/bg.dds";
/// Distance of the backdrop in front of the stage camera, metres.
const BACKDROP_DISTANCE: f32 = 40.0;
/// Seconds the camera takes to zoom to the face and back.
const ZOOM_SECONDS: f32 = 0.5;
/// Preview rebuilds are at most this far apart while a slider moves.
const REBUILD_SECONDS: f32 = 0.08;

// ---- what the window's scripts ask for ----------------------------------------------------------------------------------

#[derive(Debug, PartialEq)]
enum Req {
    /// The confirm dialog was accepted: cast the skill at the NPC.
    Cast,
    /// `CSStartInteraction` at an NPC (what the talk code does for a salon helper).
    Talk(u32),
    /// `CSEnterBeautySalon` (the cash shop's path).
    Bypass,
    /// `TerminateBeautyShop`
    Exit,
    /// `PayBeautyShop`
    Pay,
    /// `ResetBeautyShop`
    Reset,
    /// `ZoomInOutBeautyShop(dir)`: +1 towards the face, -1 away.
    Zoom(i32),
    /// The costume is hidden / shown on the preview.
    HideCostume(bool),
}

/// State shared by the scripts' natives and the systems (the HUD Lua state is not a Bevy resource).
struct Ctl {
    requests: Vec<Req>,
    /// The edited look while the salon is up.
    shared: Option<SharedRef>,
    /// The look the salon was entered with, for `IsModifierChanged` and the reset.
    original: Option<Look>,
    /// The face sliders of the edited look while a part is held at its original values.
    edited: Option<Vec<u8>>,
    certificates: i64,
    /// The salon helper the confirm dialog is for.
    npc: u32,
}

static CTL: Mutex<Ctl> = Mutex::new(Ctl { requests: Vec::new(), shared: None, original: None, edited: None, certificates: 0, npc: 0 });

fn ctl() -> MutexGuard<'static, Ctl> {
    CTL.lock().unwrap_or_else(|e| e.into_inner())
}

fn push(req: Req) {
    ctl().requests.push(req);
}

fn reset_ctl() {
    *ctl() = Ctl { requests: Vec::new(), shared: None, original: None, edited: None, certificates: 0, npc: 0 };
}

/// The natives of `salon.lua`: the `SALON` table.
pub fn install(lua: &Lua) -> mlua::Result<()> {
    let salon = lua.create_table()?;
    let simple = |name: &str, req: fn() -> Req| -> mlua::Result<()> { salon.set(name, lua.create_function(move |_, ()| Ok(push(req())))?) };
    simple("cast", || Req::Cast)?;
    simple("bypass", || Req::Bypass)?;
    simple("terminate", || Req::Exit)?;
    simple("pay", || Req::Pay)?;
    simple("reset", || Req::Reset)?;
    salon.set("talk", lua.create_function(|_, npc: u32| Ok(push(Req::Talk(npc))))?)?;
    salon.set("zoom", lua.create_function(|_, dir: f64| Ok(push(Req::Zoom(dir.signum() as i32))))?)?;
    salon.set("hide_costume", lua.create_function(|_, hide: bool| Ok(push(Req::HideCostume(hide))))?)?;
    // (item template, how many the bag has, how many a new look needs)
    salon.set("config", lua.create_function(|_, ()| Ok((SALON_CERTIFICATE, ctl().certificates, 1u32)))?)?;
    // whether the sliders of a face part (1 eyes, 2 nose, 3 mouth, 4 head shape) differ from those of the salon's entry
    salon.set(
        "modifier_changed",
        lua.create_function(|_, part: usize| {
            let c = ctl();
            let (Some(shared), Some(original)) = (&c.shared, &c.original) else { return Ok(false) };
            let g = lock(shared);
            let current = c.edited.as_ref().unwrap_or(&g.cz.look.modifier);
            Ok(part_indices(&g.cz, part).into_iter().any(|i| current.get(i) != original.modifier.get(i)))
        })?,
    )?;
    // a part is held at its original values (`original`) or released to the edited ones
    salon.set(
        "modifier_original",
        lua.create_function(|_, (part, original): (usize, bool)| {
            let mut c = ctl();
            let (Some(shared), Some(look)) = (c.shared.clone(), c.original.clone()) else { return Ok(()) };
            let mut g = lock(&shared);
            let indices = part_indices(&g.cz, part);
            let edited = c.edited.get_or_insert_with(|| g.cz.look.modifier.clone()).clone();
            for i in indices {
                let from = if original { &look.modifier } else { &edited };
                g.cz.look.modifier[i] = from.get(i).copied().unwrap_or(0);
            }
            g.cz.revision += 1;
            Ok(())
        })?,
    )?;
    lua.globals().raw_set("SALON", salon)?;
    lua.load(include_str!("salon.lua")).set_name("@salon.lua").exec()
}

/// The slider indices of a face part.
fn part_indices(cz: &Customizer, part: usize) -> Vec<usize> {
    cz.cat.targets_of(part).into_iter().map(|t| t.idx).collect()
}

// ---- who offers the salon -----------------------------------------------------------------------------------------------

/// Whether NPC template `template` is a salon helper (its interaction set holds [`ENTER_SKILL`]): the talk code asks
/// such an NPC for its skill list like it does a merchant.
#[allow(dead_code)]
pub fn offers_salon(template: u32) -> bool {
    static HELPERS: OnceLock<HashSet<u32>> = OnceLock::new();
    HELPERS.get_or_init(load_helpers).contains(&template)
}

fn load_helpers() -> HashSet<u32> {
    let load = || -> rusqlite::Result<HashSet<u32>> {
        let conn = rusqlite::Connection::open_with_flags(db_path(), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let mut q = conn.prepare(
            "SELECT n.id FROM npcs n JOIN npc_interactions i ON i.npc_interaction_set_id = n.npc_interaction_set_id WHERE i.skill_id = ?1",
        )?;
        q.query_map([ENTER_SKILL], |r| r.get(0))?.collect()
    };
    load().unwrap_or_else(|e| {
        warn!("salon: cannot read the salon helpers: {e}");
        HashSet::new()
    })
}

fn db_path() -> String {
    std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string())
}

// ---- the look of the player -----------------------------------------------------------------------------------------------

/// The number in an icon file name (`hair_017.dds` -> 17), for the list order.
fn number_in(s: &str) -> u32 {
    let digits: String = s.chars().rev().skip_while(|c| !c.is_ascii_digit()).take_while(|c| c.is_ascii_digit()).collect();
    digits.chars().rev().collect::<String>().parse().unwrap_or(u32::MAX)
}

/// Adds the salon-only hair styles to the list the create screen has.
fn add_salon_hair(cz: &mut Customizer) {
    let rows = (|| -> rusqlite::Result<Vec<HairStyle>> {
        let conn = rusqlite::Connection::open_with_flags(db_path(), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let mut q = conn.prepare(
            "SELECT b.item_id, b.asset_id, COALESCE(ic.filename, '') FROM item_body_parts b \
             JOIN items i ON i.id = b.item_id LEFT JOIN icons ic ON ic.id = i.icon_id \
             WHERE b.model_id = ?1 AND b.slot_type_id = 24 AND b.npc_only = 'f' AND b.beautyshop_only = 't'",
        )?;
        q.query_map([cz.cat.model], |r| Ok(HairStyle { item: r.get(0)?, asset: r.get(1)?, icon: r.get(2)? }))?.collect()
    })();
    match rows {
        Ok(rows) => {
            let model = cz.cat.model;
            let drawable: Vec<HairStyle> = rows.into_iter().filter(|h| appearance::hair_drawable(&cz.db, model, h.item)).collect();
            cz.cat.hair.extend(drawable);
            cz.cat.hair.sort_by_key(|h| (number_in(&h.icon), h.item));
        }
        Err(e) => warn!("salon: cannot read the salon hair styles: {e}"),
    }
}

/// The create screen's look of an appearance the server holds (the inverse of `Customizer::create_request`).
fn look_of(cz: &Customizer, params: &ModelParams, hair_item: u32) -> Look {
    let cat = &cz.cat;
    let pos = |list: &[u32], id: u32| list.iter().position(|&x| x == id).map_or(0, |i| i + 1);
    let mut look = cz.look.clone();
    look.hair = cat.hair.iter().position(|h| h.item == hair_item).map_or(0, |i| i + 1);
    look.hair_color = cat.hair_colors_of(&cz.db, look.hair).iter().position(|c| c.id == params.hair_color).map_or(1, |i| i + 1);
    look.skin = pos(&cat.skin, params.skin_color).max(1);
    let Some(face) = params.face.as_ref().filter(|_| params.kind >= ModelParams::FACE) else { return look };
    look.modifier = face.modifier.clone();
    look.modifier.resize(MODIFIER_LEN, 0);
    look.lip_color = face.lip_color;
    look.pupil_color = face.left_pupil_color;
    look.eyebrow_color = face.eyebrow_color;
    look.deco_color = face.deco_color;
    look.smile = false;
    look.wrinkle = (pos(&cat.wrinkle, face.normal_map_id), if face.normal_map_weight > 0.0 { face.normal_map_weight } else { 1.0 });
    look.eyebrow = 0;
    look.makeup = (0, 1.0);
    look.deco = (0, 1.0);
    look.tattoo = 0;
    look.scar = Scar::default();
    if face.movable_decal_asset_id != 0 {
        look.scar = Scar {
            index: pos(&cat.scar, face.movable_decal_asset_id),
            x: f32::from(face.movable_decal_move_x),
            y: f32::from(face.movable_decal_move_y),
            scale: if face.movable_decal_scale > 0.0 { face.movable_decal_scale } else { 1.0 },
            rotate: face.movable_decal_rotate,
        };
    }
    for &(asset, weight) in face.fixed_decals.iter().filter(|(asset, _)| *asset != 0) {
        let weight = if weight > 0.0 { weight } else { 1.0 };
        match cat.decal_category.get(&asset).copied() {
            Some(decal::TATTOO) => look.tattoo = pos(&cat.tattoo, asset),
            Some(decal::MAKEUP) => look.makeup = (pos(&cat.makeup, asset), weight),
            Some(decal::EYEBROW) => look.eyebrow = pos(&cat.eyebrow, asset),
            Some(decal::DECO) => look.deco = (pos(&cat.deco, asset), weight),
            _ => {}
        }
    }
    look
}

/// What `CSBeautyshopData` carries for the edited look: the hair item and the appearance.
fn apply_command(cz: &Customizer) -> SalonCommand {
    let req = cz.create_request("", 0);
    SalonCommand::Apply { hair: req.body_items[1], params: req.model_params }
}

/// The equipment the player model is drawn with (as `spawn_player` picks it).
fn own_equipment(spawn: &aa_net::PlayerSpawn) -> &Equipment {
    if spawn.unit.equipment.template_ids().iter().any(|&t| t != 0) { &spawn.unit.equipment } else { &spawn.character().equipment }
}

/// The appearance the server holds for the player.
fn own_params(spawn: &aa_net::PlayerSpawn) -> ModelParams {
    if spawn.unit.model_params.kind > 0 { spawn.unit.model_params.clone() } else { spawn.character().model_params.clone() }
}

/// The worn items of the preview: the armour of `equipment`, without the helmet (it would hide the hair), with or
/// without the costume.
fn worn_items(db: &GameDb, race: Race, gender: Gender, equipment: &Equipment, params: &ModelParams, costume: bool) -> Vec<u32> {
    let mut worn = equipment.clone();
    worn.slots[slot::HEAD] = None;
    if !costume {
        worn.slots[slot::COSPLAY] = None;
    }
    match appearance::describe(db, race, gender, &worn, params).map(|d| d.costume) {
        Some(Costume::Items(items)) => items,
        _ => Vec::new(),
    }
}

// ---- the salon ----------------------------------------------------------------------------------------------------------

/// Everything the salon owns while it is up.
#[derive(Resource)]
struct Salon {
    shared: SharedRef,
    stage: stage3d::Stage,
    subject: Option<Entity>,
    /// The revision of the look the subject was built from.
    built: Option<u64>,
    last_build: f32,
    cache: PreviewCache,
    /// The worn items of the preview, without and with the costume.
    costume: [Vec<u32>; 2],
    show_costume: bool,
    /// 0 full body, 1 face.
    zoom: f32,
    zoom_to: f32,
    /// The world cameras that were on.
    parked: Vec<Entity>,
    /// Top-level widgets of the HUD hidden while the salon is up.
    hidden: Vec<aa_ui::tree::Id>,
    /// The leave request went out (an unasked toggle is the server's).
    leaving: bool,
    /// The paying request went out and its answer is awaited.
    paying: bool,
}

/// The marker of the backdrop quad (sized to the view every frame).
#[derive(Component)]
struct Backdrop;

type WorldCameras<'w, 's> =
    Query<'w, 's, (Entity, &'static mut Camera, Has<Hdr>, Option<&'static Msaa>), (With<Camera3d>, Without<StageCamera>, Without<UiCamera>)>;

pub fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        // (`stage3d::apply_stage_layers` again: `--enter-world` runs have no login plugin)
        (net_events, handle_requests, update_preview, drive_camera, stage3d::apply_stage_layers.run_if(resource_exists::<Salon>))
            .chain()
            .after(super::sync_hud)
            .run_if(super::hud_up)
            .run_if(in_state(Screen::InGame)),
    );
    app.add_systems(OnExit(Screen::InGame), |mut commands: Commands, salon: Option<Res<Salon>>| {
        if let Some(salon) = salon {
            commands.entity(salon.stage.root).despawn();
            commands.remove_resource::<Salon>();
        }
        reset_ctl();
    });
}

/// Starts the salon: the edited look, the stage, the camera.
fn begin(commands: &mut Commands, cry: &mut CryAssets, rt: &UiRuntime, player: &Player, db: &Db, world_cameras: &mut WorldCameras) -> Option<Salon> {
    let spawn = player.spawn.as_ref()?;
    let character = spawn.character();
    let (race, gender) = appearance::race_gender(spawn.unit.model_id, character.race, character.gender)?;
    let params = own_params(spawn);
    let equipment = own_equipment(spawn);
    let mut cz = match Customizer::open(race, gender) {
        Ok(cz) => cz,
        Err(e) => {
            error!("salon: cannot open the customizer: {e:#}");
            return None;
        }
    };
    add_salon_hair(&mut cz);
    // the hair, its colour and the skin as the player is drawn (a character without them wears the race's defaults)
    let drawn = appearance::describe(&db.game.lock().unwrap_or_else(|e| e.into_inner()), race, gender, equipment, &params);
    let mut shown = params.clone();
    let hair_item = drawn.as_ref().and_then(|d| d.hair).unwrap_or(0);
    if let Some(d) = &drawn {
        shown.hair_color = d.hair_color.unwrap_or(0);
        shown.skin_color = d.skin;
    }
    cz.look = look_of(&cz, &shown, hair_item);
    cz.revision += 1;
    let original = cz.look.clone();
    let costume = {
        let game = db.game.lock().unwrap_or_else(|e| e.into_inner());
        [worn_items(&game, race, gender, equipment, &params, false), worn_items(&game, race, gender, equipment, &params, true)]
    };
    let shared: SharedRef = std::sync::Arc::new(Mutex::new(Shared {
        cz,
        step: 2,
        requests: Vec::new(),
        race_congestion: [0; 9],
        zoom: 0.0,
        ability: 0,
        create_pending: false,
        locale: "en_us".into(),
    }));
    // the natives of the create screen's scripts, over this look
    if let Err(e) = crate::login::create::api::install(&rt.lua, &shared) {
        error!("salon: {e}");
        return None;
    }
    {
        let mut c = ctl();
        c.shared = Some(shared.clone());
        c.original = Some(original);
        c.edited = None;
    }
    // the world camera rests; the stage camera takes its HDR / MSAA settings so the UI cameras keep agreeing with it
    let mut parked = Vec::new();
    let mut settings = (false, Msaa::Off);
    for (entity, mut camera, hdr, msaa) in world_cameras.iter_mut() {
        if camera.is_active {
            camera.is_active = false;
            parked.push(entity);
            settings = (hdr, msaa.copied().unwrap_or_default());
        }
    }
    let pose = poses::pose(1, race, gender);
    let stage = stage3d::spawn_stage(commands, cry, &pose, &StageLighting::login());
    commands.entity(stage.camera).insert((Camera { clear_color: ClearColorConfig::Custom(Color::BLACK), ..default() }, settings.1));
    if settings.0 {
        commands.entity(stage.camera).insert(Hdr);
    }
    if let Some(image) = cry.texture(BACKDROP, true) {
        let material = cry.materials.add(StandardMaterial { base_color_texture: Some(image), unlit: true, ..default() });
        let mesh = cry.meshes.add(Rectangle::new(1.0, 1.0));
        commands.spawn((
            Name::new("salon backdrop"),
            Backdrop,
            Mesh3d(mesh),
            MeshMaterial3d(material),
            Transform::from_translation(Vec3::new(0.0, 0.0, -BACKDROP_DISTANCE)),
            RenderLayers::layer(STAGE_LAYER),
            ChildOf(stage.camera),
        ));
    } else {
        warn!("salon: no backdrop {BACKDROP}");
    }
    Some(Salon {
        shared,
        stage,
        subject: None,
        built: None,
        last_build: -10.0,
        cache: PreviewCache::default(),
        costume,
        show_costume: false,
        zoom: 0.0,
        zoom_to: 0.0,
        parked,
        hidden: Vec::new(),
        leaving: false,
        paying: false,
    })
}

/// The salon is over: the camera and the windows come back.
fn leave(commands: &mut Commands, rt: &UiRuntime, salon: &mut Salon, world_cameras: &mut WorldCameras) {
    for entity in &salon.parked {
        if let Ok((_, mut camera, _, _)) = world_cameras.get_mut(*entity) {
            camera.is_active = true;
        }
    }
    commands.entity(salon.stage.root).despawn();
    let mut u = aa_ui::lua::ui(&rt.lua);
    for id in std::mem::take(&mut salon.hidden) {
        if u.tree.get(id).alive {
            u.tree.get_mut(id).shown = true;
        }
    }
}

/// The salon has the screen to itself: the HUD's top-level widgets are hidden while it is up (as for the NPC dialog).
fn hide_windows(rt: &UiRuntime, salon: &mut Salon) {
    let mut u = aa_ui::lua::ui(&rt.lua);
    let shown: Vec<aa_ui::tree::Id> = u.tree.top.iter().copied().filter(|&id| u.tree.get(id).alive && u.tree.get(id).shown).collect();
    for id in shown {
        u.tree.get_mut(id).shown = false;
        salon.hidden.push(id);
    }
}

/// The look the server confirmed becomes the player's: the spawn data is updated and the model built again where it stands.
fn apply_to_player(
    commands: &mut Commands,
    player: &mut Player,
    built: &mut PlayerBuilt,
    local: &Query<(Entity, &Mover), With<LocalPlayer>>,
    hair: u32,
    params: &ModelParams,
) {
    let Some(spawn) = player.spawn.as_mut() else { return };
    spawn.unit.model_params = params.clone();
    spawn.state.character.model_params = params.clone();
    for equipment in [&mut spawn.unit.equipment, &mut spawn.state.character.equipment] {
        equipment.slots[slot::HAIR] = Item::template_only(hair);
    }
    if let Ok((entity, mover)) = local.single() {
        spawn.unit.pos = aa_net::Pos { x: mover.pos.x, y: mover.pos.y, z: mover.pos.z };
        spawn.state.rotation[2] = mover.facing;
        commands.entity(entity).despawn();
    }
    built.0 = false;
}

/// The server's answers.
#[allow(clippy::too_many_arguments)]
fn net_events(
    mut commands: Commands,
    mut cry: CryAssets,
    mut events: MessageReader<NetEvent>,
    mut net: MessageWriter<NetCommand>,
    rt: Option<NonSend<UiRuntime>>,
    mut player: ResMut<Player>,
    inventory: Res<Inventory>,
    db: Option<Res<Db>>,
    salon: Option<ResMut<Salon>>,
    mut built: ResMut<PlayerBuilt>,
    local: Query<(Entity, &Mover), With<LocalPlayer>>,
    mut world_cameras: WorldCameras,
) {
    let Some(rt) = rt else {
        events.clear();
        return;
    };
    let mut salon = salon;
    for NetEvent(event) in events.read() {
        match event {
            Event::Interaction(InteractEvent::SkillList { npc_obj_id, skills, interactable: true, .. }) if skills.contains(&ENTER_SKILL) => {
                ctl().npc = *npc_obj_id;
                info!("salon: npc {npc_obj_id} offers the beauty salon");
                if let Err(e) = rt.exec("salon confirm", "SALON_CONFIRM()") {
                    warn!("salon: {e}");
                }
            }
            Event::Salon(SalonEvent::Toggled(true)) if salon.is_none() => {
                let Some(db) = db.as_deref() else { continue };
                ctl().certificates = inventory.count_of(SALON_CERTIFICATE);
                match begin(&mut commands, &mut cry, &rt, &player, db, &mut world_cameras) {
                    Some(mut new) => {
                        info!("salon: entered");
                        hide_windows(&rt, &mut new);
                        commands.insert_resource(new);
                        rt.fire_event("TOGGLE_BEAUTYSHOP", true);
                    }
                    None => {
                        net.write(NetCommand(Command::Salon(SalonCommand::Exit)));
                    }
                }
            }
            Event::Salon(SalonEvent::Toggled(false)) => {
                let Some(s) = salon.as_deref_mut() else { continue };
                info!("salon: left");
                if !s.leaving {
                    rt.fire_event("BEAUTYSHOP_CLOSE_BY_SYSTEM", ());
                }
                rt.fire_event("TOGGLE_BEAUTYSHOP", false);
                leave(&mut commands, &rt, s, &mut world_cameras);
                commands.remove_resource::<Salon>();
                reset_ctl();
                salon = None;
            }
            Event::Salon(SalonEvent::Modified { character_id, hair, params }) => {
                let mine = player.spawn.as_ref().is_some_and(|s| s.character().id == *character_id);
                if mine {
                    info!("salon: the look was changed (hair item {hair})");
                    apply_to_player(&mut commands, &mut player, &mut built, &local, *hair, params);
                    if let Some(s) = salon.as_deref_mut()
                        && s.paying
                    {
                        s.paying = false;
                        s.leaving = true;
                        net.write(NetCommand(Command::Salon(SalonCommand::Exit)));
                    }
                }
            }
            _ => {}
        }
    }
}

/// The scripts' requests become packets (and camera moves).
fn handle_requests(mut net: MessageWriter<NetCommand>, inventory: Res<Inventory>, player: Res<Player>, salon: Option<ResMut<Salon>>) {
    let requests = std::mem::take(&mut ctl().requests);
    let mut salon = salon;
    if salon.is_some() {
        ctl().certificates = inventory.count_of(SALON_CERTIFICATE);
    }
    for req in requests {
        match req {
            Req::Cast => {
                let npc = ctl().npc;
                info!("salon: casting {ENTER_SKILL} at npc {npc}");
                net.write(NetCommand(Command::UseSkill { skill_id: ENTER_SKILL, target_obj_id: npc }));
            }
            Req::Talk(npc) => {
                let me = player.spawn.as_ref().map_or(0, |s| s.obj_id);
                net.write(NetCommand(Command::Interact(Interact::Start {
                    npc_obj_id: npc,
                    obj_id: me,
                    extra_info: 0,
                    pick_id: 0,
                    mouse_button: 2,
                    modifier_keys: 0,
                })));
            }
            Req::Bypass => {
                net.write(NetCommand(Command::Salon(SalonCommand::Enter)));
            }
            Req::Exit => {
                if let Some(s) = salon.as_deref_mut() {
                    s.leaving = true;
                }
                net.write(NetCommand(Command::Salon(SalonCommand::Exit)));
            }
            Req::Pay => {
                let Some(s) = salon.as_deref_mut() else { continue };
                let command = apply_command(&lock(&s.shared).cz);
                info!("salon: paying, {command:?}");
                s.paying = true;
                net.write(NetCommand(Command::Salon(command)));
            }
            Req::Reset => {
                let Some(s) = salon.as_deref_mut() else { continue };
                let original = ctl().original.clone();
                if let Some(original) = original {
                    let mut g = lock(&s.shared);
                    g.cz.look = original;
                    g.cz.revision += 1;
                    ctl().edited = None;
                }
            }
            Req::Zoom(dir) => {
                if let Some(s) = salon.as_deref_mut() {
                    s.zoom_to = if dir > 0 { 1.0 } else { 0.0 };
                }
            }
            Req::HideCostume(hide) => {
                if let Some(s) = salon.as_deref_mut() {
                    s.show_costume = !hide;
                    // the cache keeps the structure of the worn items, which just changed
                    s.cache = PreviewCache::default();
                    lock(&s.shared).cz.revision += 1;
                }
            }
        }
    }
}

type Lights<'w, 's> = Query<'w, 's, &'static mut Transform, (With<DirectionalLight>, Without<StageCamera>, Without<StageSubject>)>;

/// Points the stage's lights for a figure standing in `slot` (as the create screen does).
fn aim_lights(lights: &mut Lights, stage: &stage3d::Stage, slot: &StageSlot) {
    let rig = StageLighting::login();
    let frame = slot.light_frame();
    for (entity, light) in stage.lights.iter().zip(&rig.lights) {
        if let Ok(mut t) = lights.get_mut(*entity) {
            *t = Transform::from_translation(frame * (light.from.normalize() * 10.0)).looking_at(Vec3::ZERO, Vec3::Y);
        }
    }
}

/// Builds the preview character when the look changed.
fn update_preview(mut commands: Commands, mut cry: CryAssets, clips: Option<Res<ClipLibrary>>, salon: Option<ResMut<Salon>>, time: Res<Time>, mut lights: Lights) {
    let (Some(mut salon), Some(clips)) = (salon, clips) else { return };
    let now = time.elapsed_secs();
    let shared = salon.shared.clone();
    let (rev, race, gender) = {
        let g = lock(&shared);
        (g.cz.revision, g.cz.cat.race, g.cz.cat.gender)
    };
    if salon.built == Some(rev) || (salon.built.is_some() && now - salon.last_build < REBUILD_SECONDS) {
        return;
    }
    let assets = {
        let g = lock(&shared);
        let mut desc = g.cz.desc();
        desc.costume = Costume::Items(salon.costume[usize::from(salon.show_costume)].clone());
        salon.cache.assets(&desc, &g.cz.db)
    };
    match assets {
        Ok(assets) => {
            let slot = poses::slot_steps_1_2(race, gender);
            let first = salon.built.is_none();
            let old = salon.subject.take();
            salon.subject = stage3d::spawn_subject(&mut commands, &mut cry, &clips, assets, &slot, salon.stage.root);
            if let Some(old) = old {
                commands.entity(old).despawn();
            }
            if first {
                aim_lights(&mut lights, &salon.stage, &slot);
            }
        }
        Err(e) => error!("salon: cannot resolve the look: {e}"),
    }
    salon.built = Some(rev);
    salon.last_build = now;
}

/// The camera between the full body and the face, and the backdrop sized to the view.
#[allow(clippy::type_complexity)]
fn drive_camera(
    salon: Option<ResMut<Salon>>,
    time: Res<Time>,
    mut cameras: Query<(&mut Transform, &mut Projection), (With<StageCamera>, Without<Backdrop>, Without<StageSubject>)>,
    mut subjects: Query<&mut Transform, (With<StageSubject>, Without<StageCamera>, Without<Backdrop>)>,
    mut backdrop: Query<&mut Transform, (With<Backdrop>, Without<StageCamera>, Without<StageSubject>)>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
) {
    let Some(mut salon) = salon else { return };
    let step = time.delta_secs() / ZOOM_SECONDS;
    salon.zoom += (salon.zoom_to - salon.zoom).clamp(-step, step);
    let (race, gender) = {
        let g = lock(&salon.shared);
        (g.cz.cat.race, g.cz.cat.gender)
    };
    let (far, near) = (poses::pose(1, race, gender), poses::pose(2, race, gender));
    let k = salon.zoom * salon.zoom * (3.0 - 2.0 * salon.zoom);
    let (a, b) = (far.transform(), near.transform());
    let fov = (far.fov_deg + (near.fov_deg - far.fov_deg) * k).to_radians();
    if let Ok((mut t, mut projection)) = cameras.single_mut() {
        *t = Transform { translation: a.translation.lerp(b.translation, k), rotation: a.rotation.slerp(b.rotation, k), scale: Vec3::ONE };
        if let Projection::Perspective(p) = &mut *projection {
            p.fov = fov;
        }
    }
    if let Some(subject) = salon.subject
        && let Ok(mut t) = subjects.get_mut(subject)
    {
        let (t1, t2) = (poses::slot_steps_1_2(race, gender).transform(), poses::slot_step_2(race, gender).transform());
        *t = Transform { translation: t1.translation.lerp(t2.translation, k), rotation: t1.rotation, scale: t1.scale };
    }
    // the backdrop fills the view
    if let (Ok(mut t), Ok(window)) = (backdrop.single_mut(), windows.single()) {
        let height = 2.0 * BACKDROP_DISTANCE * (fov / 2.0).tan();
        let aspect = window.physical_width() as f32 / window.physical_height().max(1) as f32;
        t.scale = Vec3::new(height * aspect, height, 1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_numbers_order_the_hair_list() {
        assert_eq!(number_in("customizing_elf_f/hair_017.dds"), 17);
        assert_eq!(number_in("icon_item_hair.dds"), u32::MAX);
    }

    #[test]
    fn the_original_salon_initializes_its_lists_and_edits_the_model_view() -> Result<(), Box<dyn std::error::Error>> {
        let (rt, _, path) = super::super::breath::fixture::runtime(1920.0, 1080.0)?;
        let cz = Customizer::open(Race::Nuian, Gender::Male)?;
        let shared = std::sync::Arc::new(Mutex::new(Shared {
            cz, step: 2, requests: Vec::new(), race_congestion: [0; 9], zoom: 0.0,
            ability: 0, create_pending: false, locale: "en_us".into(),
        }));
        crate::login::create::api::install(&rt.lua, &shared)?;
        rt.fire_event("TOGGLE_BEAUTYSHOP", true);
        rt.lua.load(r#"
            assert(IsBeautyShop())
            local certificateName = X2Item:Name(30811)
            assert(type(certificateName) == 'string' and certificateName ~= '')
            assert(certificateName == HUD.item_info(30811))
            assert(X2Customizer:GetNumCustomHair() > 3)
            local view = UIParent:CreateWidget("modelview", "salonTestView", "UIParent")
            view:Show(false)
            assert(view:GetRace() == "nuian" and view:GetGender() == "male")
            local path = GetCosmeticItemInfo(RACE_NUIAN, GENDER_MALE, HAIR_TYPE, 3)
            assert(type(path) == "string" and path ~= "")
            SelectCosmeticItem(HAIR_TYPE, 3)
            assert(GetCurrentCosmeticItem(HAIR_TYPE) == 3)
        "#).exec()?;
        assert_eq!(lock(&shared).cz.look.hair, 3);
        rt.fire_event("TOGGLE_BEAUTYSHOP", false);
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }

    /// The look the salon starts from is the one the server was given: `look_of` inverts `create_request`.
    #[test]
    #[ignore = "needs the game data"]
    fn the_look_round_trips_through_the_server_params() {
        for (race, gender) in [Race::Nuian, Race::Elf, Race::Hariharan, Race::Ferre]
            .into_iter().flat_map(|race| [Gender::Male, Gender::Female].map(|gender| (race, gender)))
        {
            let mut cz = Customizer::open(race, gender).unwrap();
            add_salon_hair(&mut cz);
            let l = &mut cz.look;
            l.hair = cz.cat.hair.len();
            l.hair_color = 4;
            l.skin = 2;
            l.tattoo = 1;
            l.makeup = (1, 0.5);
            l.eyebrow = 1;
            // (a Ferre male has no deco)
            l.deco = if cz.cat.deco.is_empty() { (0, 1.0) } else { (1, 0.75) };
            l.scar = Scar { index: 1, x: 20.0, y: -4.0, scale: 1.5, rotate: 0.25 };
            l.wrinkle = (1, 0.5);
            l.modifier[5] = 40;
            l.lip_color = 0xFF3030D0;
            let SalonCommand::Apply { hair, params } = apply_command(&cz) else { unreachable!() };
            assert_eq!(look_of(&cz, &params, hair), cz.look, "{race:?} {gender:?}");
        }
    }
}
