//! Character select: the original `character_select` scripts, `X2LoginCharacter` backed by
//! aa-net, the selected character in 3D in front of the painted background.
//!
//! The scripts list the characters (`GetNumLoginCharacters`, `GetLoginCharacterName(i)`, ...), ask for
//! the 3D character of the selected slot (`ShowSelectedCharacter(i)`), and start (`SelectCharacter`),
//! delete (`DeleteCharacterCheckName`, the server answers with a delete timer), cancel a delete,
//! go back to the server list (`X2World:LeaveWorld(EXIT_TO_WORLD_LIST)`) or create a character
//! (`CreateCharacter`, see `login::create`). The 3D side is [`stage3d`]: the camera of the capture,
//! the login lighting and one character slot, between the UI's painted background and its panels.

use aa_anim::ClipLibrary;
use aa_cry::scene::CryAssets;
use aa_cry::stage::StageLighting;
use aa_net::state::{Characters, Worlds};
use aa_net::{AccountEvent, Event, NetEvent, Phase};
use aa_ui::{Stage, UiRuntime};
use bevy::prelude::*;

use super::gameapi::Request;
use super::stage3d::{self, StagePose, StageSlot};
use super::{Goto, LoginArgs, LoginBridge, UiRequest, gameapi, host, ui_is};
use crate::Screen;
use crate::ingame::appearance;
use crate::ingame::data::Db;
use crate::ingame::weapons;

pub fn plugin(app: &mut App) {
    app.init_resource::<SelectStage>()
        .init_resource::<Selected>()
        .add_systems(OnEnter(Screen::CharacterSelect), enter_stage)
        .add_systems(OnExit(Screen::CharacterSelect), exit_stage)
        .add_systems(
            Update,
            // the weapons of the shown character hang on its bones like in game (the sword on the hip, the bow on the back)
            (net_events, show_character, auto_drive, weapons::attach_carried).run_if(ui_is(Screen::CharacterSelect)),
        );
}

/// Build the UI of this screen (the character list is there when the screen is entered).
pub fn build(world: &mut World) -> bool {
    if world.resource::<aa_net::state::Status>().phase != Phase::CharacterSelect {
        return false;
    }
    let shared = world.non_send::<LoginBridge>().0.clone();
    let (worlds, characters) = (world.resource::<Worlds>().clone(), world.resource::<Characters>().clone());
    shared.borrow_mut().sync(&worlds, &characters);
    // back from the create screen: the list that has the new character, not the one from before it
    if shared.borrow().waiting_for_created() {
        return false;
    }
    host::switch_stage(world, |rt| {
        if let Err(e) = gameapi::install(rt, &shared) {
            error!("character select: game API: {e}");
        }
        rt.run_stage(Stage::CharacterSelect);
        rt.fire_event("SHOW_CHARACTER_SELECT_WINDOW", true);
    })
}

/// The character the list has selected: what the scripts last showed with `ShowSelectedCharacter` (the new character after
/// a create, or the slot the player clicked). The world prewarm (`ingame::prewarm`) follows it.
#[derive(Resource, Default)]
pub struct Selected(pub Option<u32>);

/// The 3D stage of this screen.
#[derive(Resource, Default)]
struct SelectStage {
    stage: Option<stage3d::Stage>,
    subject: Option<Entity>,
    /// The slot (1-based) the subject shows.
    shown: Option<usize>,
}

fn enter_stage(mut commands: Commands, mut cry: CryAssets, mut sel: ResMut<SelectStage>) {
    let stage = stage3d::spawn_stage(&mut commands, &mut cry, &StagePose::CHARACTER_SELECT, &StageLighting::login());
    *sel = SelectStage { stage: Some(stage), subject: None, shown: None };
}

fn exit_stage(mut commands: Commands, mut sel: ResMut<SelectStage>, mut selected: ResMut<Selected>) {
    selected.0 = None;
    if let Some(stage) = sel.stage.take() {
        commands.entity(stage.root).despawn();
    }
    *sel = SelectStage::default();
}

/// `ShowSelectedCharacter(i)`: the character of slot `i` stands in the select slot.
#[allow(clippy::too_many_arguments)]
fn show_character(
    mut requests: MessageReader<UiRequest>,
    mut commands: Commands,
    mut cry: CryAssets,
    clips: Res<ClipLibrary>,
    db: Option<Res<Db>>,
    characters: Res<Characters>,
    mut sel: ResMut<SelectStage>,
    mut selected: ResMut<Selected>,
) {
    for UiRequest(request) in requests.read() {
        let Request::ShowCharacter(index) = request else { continue };
        selected.0 = characters.list.get(index.wrapping_sub(1)).map(|c| c.id);
        let (Some(stage), Some(db)) = (sel.stage.clone(), db.as_ref()) else { continue };
        if sel.shown == Some(*index) {
            continue;
        }
        if let Some(old) = sel.subject.take() {
            commands.entity(old).despawn();
        }
        sel.shown = Some(*index);
        let Some(c) = characters.list.get(index.wrapping_sub(1)) else { continue };
        let Some((race, gender)) = appearance::race_gender(0, c.race, c.gender) else {
            warn!("character select: unsupported race {} of {}", c.race, c.name);
            continue;
        };
        let game = db.game.lock().unwrap();
        let Some(desc) = appearance::describe(&game, race, gender, &c.equipment, &c.model_params) else {
            warn!("character select: cannot describe {}", c.name);
            continue;
        };
        let assets = match appearance::resolve(&game, &desc, &c.equipment) {
            Ok(a) => a,
            Err(e) => {
                warn!("character select: cannot resolve {}: {e}", c.name);
                continue;
            }
        };
        drop(game);
        let slot = StageSlot::select(race, gender);
        sel.subject = stage3d::spawn_subject(&mut commands, &mut cry, &clips, assets, &slot, stage.root);
        let carried = weapons::select_weapons(weapons::carried(db, &c.equipment));
        info!(
            "character select: weapons main {:?} off {:?} ranged {:?}: showing {:?}",
            c.equipment.get(aa_net::model::slot::MAINHAND).map(|i| i.template_id),
            c.equipment.get(aa_net::model::slot::OFFHAND).map(|i| i.template_id),
            c.equipment.get(aa_net::model::slot::RANGED).map(|i| i.template_id),
            carried.iter().map(|i| i.path.rsplit('/').next().unwrap_or("")).collect::<Vec<_>>()
        );
        if let Some(subject) = sel.subject.filter(|_| !carried.is_empty()) {
            commands.entity(subject).insert(weapons::Carried(carried));
        }
        info!("character select: showing {} ({race:?} {gender:?}) zone {} faction {} {:?} pos {:?} leave {} delete {}/{}", c.name, c.zone_id, c.faction_id, c.faction_name, c.pos, c.leave_time, c.delete_request_time, c.delete_time);
    }
}

/// What the server tells while this screen is up, as the UI events the client raises.
fn net_events(
    rt: NonSend<UiRuntime>,
    bridge: NonSend<LoginBridge>,
    mut events: MessageReader<NetEvent>,
    mut requests: MessageReader<UiRequest>,
    worlds: Res<Worlds>,
    characters: Res<Characters>,
    mut pending: Local<Option<(&'static str, i64)>>,
) {
    for UiRequest(request) in requests.read() {
        if let Request::CancelDelete(index) = request {
            *pending = Some(("delete_canceled", *index as i64));
        }
    }
    for NetEvent(event) in events.read() {
        match event {
            Event::CharacterList(_) => {
                // the scripts read the list again from the bridge: bring it up to date first (its houses follow it)
                let mut b = bridge.0.borrow_mut();
                b.sync(&worlds, &characters);
                b.houses.clear();
                drop(b);
                match pending.take() {
                    Some((status, index)) => rt.fire_event("LOGIN_CHARACTER_UPDATED", (status, index)),
                    None => rt.fire_event("LOGIN_CHARACTER_UPDATED", "refreshed"),
                }
            }
            // the answer comes first, the refreshed list (with the delete timer) right behind it
            Event::CharacterDeleteResponse { id, status, .. } => {
                let index = characters.list.iter().position(|c| c.id == *id).map_or(1, |i| i as i64 + 1);
                let name = match status {
                    2 => "delete_requested",
                    3 => "delete_canceled",
                    4 => "error_cancel_character_delete_failed",
                    _ => "error_character_delete_failed",
                };
                info!("character select: delete response {status} for character {id}");
                if name.starts_with("error") {
                    rt.fire_event("LOGIN_CHARACTER_UPDATED", (name, index));
                } else {
                    *pending = Some((name, index));
                }
            }
            Event::CharacterDeleted { .. } => *pending = Some(("deleted", 1)),
            Event::Housing(aa_net::housing::HousingEvent::LoginCharInfoHouse { house, .. }) => bridge.0.borrow_mut().add_house(house),
            // `OnRefreshInCharacterList` (x2game 391d2ff0): the congestion, then "refreshed"
            Event::Account(AccountEvent::CharacterListRefreshed { race_congestion }) => {
                bridge.0.borrow_mut().race_congestion = *race_congestion;
                rt.fire_event("LOGIN_CHARACTER_UPDATED", "refreshed");
            }
            // `OnLpManaged` (394d33e0): `LP_MANAGE_CHARACTER_CHANGED(character id, index)`
            Event::Account(AccountEvent::LpManaged { character_id }) => {
                let mut b = bridge.0.borrow_mut();
                b.lp_managed = Some(*character_id);
                let index = b.characters.iter().position(|c| c.id == *character_id).map_or(0, |i| i as i64 + 1);
                drop(b);
                rt.fire_event("LP_MANAGE_CHARACTER_CHANGED", (i64::from(*character_id), index));
            }
            _ => {}
        }
    }
}

/// `--goto loading`: press Start.
fn auto_drive(rt: NonSend<UiRuntime>, args: Res<LoginArgs>, time: Res<Time>, characters: Res<Characters>, mut since: Local<f32>, mut done: Local<bool>) {
    if *done || args.goto != Some(Goto::Loading) {
        return;
    }
    *since += time.delta_secs();
    if *since < 2.0 || characters.list.is_empty() {
        return;
    }
    *done = true;
    info!("login: --goto: pressing Start");
    if let Err(e) = rt.exec("goto", "characterSelectWindow.bottomPanel.startButton:OnClick('LeftButton')") {
        warn!("login: --goto: {e}");
    }
}
