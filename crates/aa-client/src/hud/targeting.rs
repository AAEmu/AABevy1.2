//! Choosing a target: a left click on a unit in the 3D view, `Tab` for the next nearest unit,
//! `Escape` to drop it (after the windows that close on it). The server answers with
//! `TargetChanged`, which the target frame follows (see `panels.rs`).

use aa_net::state::{Player, Target, Units};
use aa_net::{Command, Mates, NetCommand};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use super::api::HudBridge;
use super::{HudKeyboardFocus, HudPointerOverUi};

/// How far `Tab` looks, metres.
const TAB_RANGE: f32 = 40.0;
/// A press that moves more than this many pixels before it is released was a camera drag.
const CLICK_SLOP: f32 = 6.0;

/// `Tab`: the nearest enemy that is alive and not the one targeted, the next one after it when pressed again (the
/// client's "target nearest enemy"; with no enemy in range, any unit).
#[allow(clippy::too_many_arguments)]
pub fn tab_target(
    actions: Res<crate::settings::Actions>,
    focus: Res<HudKeyboardFocus>,
    bridge: NonSend<HudBridge>,
    units: Res<Units>,
    poses: super::pick::UnitPoses,
    target: Res<Target>,
    player: Res<Player>,
    mut script: ResMut<super::chat::ScriptedKeys>,
    mut net: MessageWriter<NetCommand>,
) {
    let scripted = script.take_tab();
    let mode = [
        ("cycle_hostile_forward", false, false),
        ("cycle_hostile_backward", true, false),
        ("cycle_friendly_forward", false, true),
        ("cycle_friendly_backward", true, true),
    ]
    .into_iter()
    .find(|(name, _, _)| actions.just_pressed(name, 0));
    if focus.0 || !(scripted || mode.is_some()) {
        return;
    }
    let (_, backwards, friendly) = mode.unwrap_or(("cycle_hostile_forward", false, false));
    let own = player.spawn.as_ref().map_or(0, |s| s.obj_id);
    // seated, the server moves only the slave: the HUD position can stay at the boarding point.
    // The drawn root chain puts an attached child (a cannon) at its world position too.
    let world = |id: u32| units.get(id).and_then(|unit| poses.get(unit, &units));
    let (me, shared) = {
        let d = bridge.0.borrow();
        (super::slave::player_centre(&d, &world), d.game.clone())
    };
    let own_faction = player.spawn.as_ref().map_or(0, |s| s.character().faction_id);
    let mut near: Vec<(f32, u32, bool)> = units
        .iter()
        .filter(|u| u.obj_id != own)
        .filter(|u| !friendly || (!u.dead && super::plates::relation_of(shared.get(), own_faction, u) == "friendly"))
        .map(|u| {
            let at = poses
                .get(u, &units)
                .map_or(Vec3::new(u.pos.x, u.pos.y, u.pos.z), |(pos, _)| pos);
            (me.distance(at), u.obj_id, !u.dead && super::plates::relation_of(shared.get(), own_faction, u) == "hostile")
        })
        .filter(|(d, _, _)| *d <= TAB_RANGE)
        .collect();
    if near.iter().any(|n| n.2) {
        near.retain(|n| n.2);
    }
    near.sort_by(|a, b| a.0.total_cmp(&b.0));
    if near.is_empty() {
        return;
    }
    let next = match target.obj_id.and_then(|t| near.iter().position(|(_, id, _)| *id == t)) {
        Some(i) => near[(i + if backwards { near.len() - 1 } else { 1 }) % near.len()].1,
        None => near[if backwards { near.len() - 1 } else { 0 }].1,
    };
    net.write(NetCommand(Command::Target { obj_id: next }));
}

/// `self_target` and `round_target` (F1 by default): target the local player. The native
/// `self_target` handler (`FUN_393f8fb0`) asks for the player; `round_target` (`FUN_3942a080`)
/// asks for the player, and when the player already is the target, for the own pet.
pub fn self_target(
    actions: Res<crate::settings::Actions>,
    focus: Res<HudKeyboardFocus>,
    rt: Option<NonSend<UiRuntime>>,
    player: Res<Player>,
    mates: Res<Mates>,
    mut target: ResMut<Target>,
    mut net: MessageWriter<NetCommand>,
) {
    if focus.0 || rt.as_ref().is_some_and(|rt| rt.keyboard_captured()) {
        return;
    }
    let round = actions.just_pressed("round_target", 0);
    if !round && !actions.just_pressed("self_target", 0) {
        return;
    }
    let Some(own) = player.spawn.as_ref().map(|s| s.obj_id).filter(|id| *id != 0) else { return };
    let next = if round && target.obj_id == Some(own) {
        mates.own.as_ref().and_then(|mate| mate.obj_id)
    } else {
        Some(own)
    };
    let Some(obj_id) = next.filter(|id| *id != 0) else { return };
    if !target.set_local(Some(obj_id)) {
        return;
    }
    net.write(NetCommand(Command::Target { obj_id }));
}

/// A left click (not a drag, not on the HUD) on a unit selects it.
#[allow(clippy::too_many_arguments)]
pub fn click_target(
    mouse: Res<ButtonInput<MouseButton>>,
    over_ui: Res<HudPointerOverUi>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform), (With<Camera3d>, Without<crate::hud::ModelViewCamera>)>,
    units: Res<Units>,
    player: Res<Player>,
    poses: super::pick::UnitPoses,
    mut boxes: super::pick::ModelBoxes,
    mut script_pick: ResMut<crate::script::ScriptPick>,
    mut net: MessageWriter<NetCommand>,
    mut pressed_at: Local<Option<Vec2>>,
    bridge: Option<NonSend<HudBridge>>,
) {
    if bridge.as_ref().is_some_and(|bridge| super::blocks_look_pointer(&bridge.0.borrow(), MouseButton::Left)) {
        script_pick.take_left();
        *pressed_at = None;
        return;
    }
    // a pick the script runner made (`target pick ...`) is the click's result
    if let Some(obj_id) = script_pick.take_left() {
        net.write(NetCommand(Command::Target { obj_id }));
        return;
    }
    let Ok(window) = windows.single() else { return };
    let cursor = window.cursor_position();
    if mouse.just_pressed(MouseButton::Left) {
        *pressed_at = if over_ui.0 { None } else { cursor };
    }
    if !mouse.just_released(MouseButton::Left) {
        return;
    }
    let (Some(start), Some(end)) = (pressed_at.take(), cursor) else { return };
    if start.distance(end) > CLICK_SLOP || over_ui.0 {
        return;
    }
    let Some((camera, transform)) = cameras.iter().find(|(c, _)| c.is_active) else { return;
    };
    let Ok(ray) = camera.viewport_to_world(transform, end) else { return };
    let own = player.spawn.as_ref().map_or(0, |s| s.obj_id);
    let ridden = player
        .spawn
        .as_ref()
        .and_then(|s| s.unit.attached_to)
        .map(|a| a.obj_id);
    if let Some(obj_id) = super::pick::pick_unit(
        &ray,
        &units,
        own,
        ridden,
        |_| true,
        |unit, ray| {
            (
                poses.get(unit, &units).map(|(pos, _)| pos),
                boxes.get_near(unit, ray),
            )
        },
    ) {
        net.write(NetCommand(Command::Target { obj_id }));
    }
}

/// `Escape`: close the topmost window that closes on it; with none open, drop the target; else
/// the toolkit opens the system menu.
#[allow(clippy::too_many_arguments)]
pub fn escape_key(
    keys: Res<ButtonInput<KeyCode>>,
    actions: Res<crate::settings::Actions>,
    focus: Res<HudKeyboardFocus>,
    rt: Option<NonSend<UiRuntime>>,
    bridge: Option<NonSend<HudBridge>>,
    target: Res<Target>,
    mut net: MessageWriter<NetCommand>,
    mut script: ResMut<super::chat::ScriptedKeys>,
    mut aiming: Option<ResMut<crate::ingame::ground_target::Aiming>>,
) {
    let scripted = script.take_escape();
    let mapped = actions.just_pressed("open_config", 0);
    let physical = keys.just_pressed(KeyCode::Escape);
    let escape = scripted || physical || mapped;
    if focus.0 || rt.as_ref().is_some_and(|rt| rt.keyboard_captured()) || !escape {
        return;
    }
    if let Some(rt) = rt.as_deref() {
        if super::repair::escape(rt) {
            return;
        }
        if bridge
            .as_ref()
            .is_some_and(|bridge| super::look_conversion::escape(rt, &bridge.0))
        {
            return;
        }
    }
    if aiming.as_deref_mut().is_some_and(|aiming| aiming.cancel()) {
        return;
    }
    let Some(rt) = rt else { return };
    if mapped && !physical && !scripted {
        if let Err(e) = rt.hotkey("open_config") {
            warn!("hud: open_config: {e}");
        }
        return;
    }
    if rt.holding() {
        rt.clear_cursor();
        return;
    }
    let window_open = {
        let u = aa_ui::lua::ui(&rt.lua);
        u.tree.paint_order().into_iter().any(|id| {
            u.tree.get(id).alive
                && u.tree.widget(id).is_some_and(|w| w.close_on_escape)
                && u.tree.visible(id)
        })
    };
    if !window_open && target.obj_id.is_some() {
        net.write(NetCommand(Command::Target { obj_id: 0 }));
    } else if (window_open || mapped || scripted) && let Err(e) = rt.escape() {
        warn!("hud: escape: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aa_net::model::{Unit, UnitKind, UnitStateAttachments};
    use aa_net::session::PlayerSpawn;
    use aa_net::state::{Player, Target};
    use std::cell::RefCell;
    use std::rc::Rc;

    use crate::hud::api::{HudBridge, HudData, Request, Shared};
    use crate::hud::mounted;
    use crate::hud::usable::InFlight;

    fn unit(obj_id: u32, kind: UnitKind, pos: Vec3, parent: Option<u32>) -> Unit {
        let mut unit = crate::hud::fixture_spawn().unit;
        unit.obj_id = obj_id;
        unit.name = format!("Unit {obj_id}");
        unit.kind = kind;
        unit.pos = aa_net::Pos::new(pos.x, pos.y, pos.z);
        unit.state_attachments = parent.map(|parent| UnitStateAttachments {
            first_point: 1,
            first_obj_id: Some(parent),
            bonding_obj_id: None,
        });
        unit
    }

    fn slave(obj_id: u32, pos: Vec3, parent: Option<u32>) -> Unit {
        unit(
            obj_id,
            UnitKind::Slave {
                id: 700,
                tl_id: 700,
                template_id: 60,
                summoner_id: 100,
            },
            pos,
            parent,
        )
    }

    fn own_spawn(obj_id: u32) -> PlayerSpawn {
        let mut spawn = crate::hud::fixture_spawn();
        spawn.obj_id = obj_id;
        spawn.unit.obj_id = obj_id;
        spawn
    }

    fn target_app(spawn: PlayerSpawn, mates: Mates, target: Target) -> App {
        let mut app = App::new();
        app.insert_resource(Player { spawn: Some(spawn) })
            .insert_resource(mates)
            .insert_resource(target)
            .init_resource::<HudKeyboardFocus>()
            .init_resource::<crate::settings::Actions>()
            .add_message::<NetCommand>()
            .add_systems(Update, self_target);
        app
    }

    fn sent(app: &mut App) -> Vec<Command> {
        app.world_mut()
            .resource_mut::<Messages<NetCommand>>()
            .drain()
            .map(|message| message.0)
            .collect()
    }

    #[test]
    fn f1_targets_the_player_and_sends_the_own_change_target_bc() {
        let own = 0x5f21;
        let mut app = target_app(own_spawn(own), Mates::default(), Target::default());
        app.world_mut().resource_mut::<crate::settings::Actions>().tap_for_test("round_target");
        app.update();
        let commands = sent(&mut app);
        assert!(matches!(&commands[..], [Command::Target { obj_id }] if *obj_id == own), "{commands:?}");
        assert_eq!(app.world().resource::<Target>().obj_id, Some(own));
        let message = aa_net::game::ClientMsg::ChangeTarget { obj_id: own };
        assert_eq!(message.opcode(), 0x02c);
        assert_eq!(message.encode_body(), [0x21, 0x5f, 0x00]);
    }

    #[test]
    fn the_self_target_action_also_targets_the_player() {
        let own = 0x5f21;
        let mut app = target_app(own_spawn(own), Mates::default(), Target::default());
        app.world_mut().resource_mut::<crate::settings::Actions>().tap_for_test("self_target");
        app.update();
        assert!(matches!(&sent(&mut app)[..], [Command::Target { obj_id }] if *obj_id == own));
        assert_eq!(app.world().resource::<Target>().obj_id, Some(own));
    }

    #[test]
    fn a_second_round_target_press_targets_the_own_pet() {
        let (own, pet) = (0x5f21, 0x2a);
        let mut mates = Mates::default();
        mates.own = Some(aa_net::OwnedMate {
            spawn: aa_net::MateSpawn::default(),
            obj_id: Some(pet),
            dynamic: None,
        });
        let mut target = Target::default();
        target.set_local(Some(own));
        let mut app = target_app(own_spawn(own), mates, target);
        app.world_mut().resource_mut::<crate::settings::Actions>().tap_for_test("round_target");
        app.update();
        assert!(matches!(&sent(&mut app)[..], [Command::Target { obj_id }] if *obj_id == pet));
        assert_eq!(app.world().resource::<Target>().obj_id, Some(pet));
    }

    #[test]
    fn round_target_stays_on_the_player_without_a_pet() {
        let own = 0x5f21;
        let mut target = Target::default();
        target.set_local(Some(own));
        let mut app = target_app(own_spawn(own), Mates::default(), target);
        app.world_mut().resource_mut::<crate::settings::Actions>().tap_for_test("round_target");
        app.update();
        assert!(sent(&mut app).is_empty());
        assert_eq!(app.world().resource::<Target>().obj_id, Some(own));
    }

    #[test]
    fn a_self_target_builds_the_target_frame_from_the_player_without_a_view_unit() {
        // aa-net keeps the local player in `Player`, not in `Units`; the target frame still has to
        // get the player's unit (the 1.2 client shows the player as the target).
        let own = crate::hud::fixture_spawn();
        let mut world = World::new();
        world.insert_resource(Player { spawn: Some(own.clone()) });
        world.init_resource::<Units>();
        let mut target = Target::default();
        target.set_local(Some(own.obj_id));
        world.insert_resource(target);
        let snapshot = crate::hud::data::Snapshot {
            unit_id: own.obj_id.to_string(),
            name: own.name().to_string(),
            level: 30,
            hp: 87,
            max_hp: 100,
            mp: 40,
            max_mp: 80,
            ..Default::default()
        };
        let at = Vec3::new(1.0, 2.0, 3.0);
        let (panels, target_pos, _) =
            crate::hud::panels::refresh(&world, None, 0, own.obj_id, std::time::Instant::now(), &snapshot, at);
        let shown = panels.target.expect("target frame for self");
        assert_eq!(shown.obj_id, own.obj_id);
        assert_eq!(shown.detail, "character");
        assert_eq!((shown.hp, shown.max_hp, shown.mp, shown.max_mp), (87, 100, 40, 80));
        assert_eq!(target_pos, Some(at));
    }

    /// The original world-stage HUD with an explicit synthetic player (the breath fixture, but
    /// loading its UI texts from `AA_DB` rather than the working directory's compact.sqlite3).
    fn world_runtime(width: f32, height: f32) -> Result<(UiRuntime, Shared, std::path::PathBuf), Box<dyn std::error::Error>> {
        let mut rt = UiRuntime::new("en_us", width, height)?;
        let db = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.into());
        rt.load_ui_texts(std::path::Path::new(&db))?;
        let bridge: Shared = Rc::new(RefCell::new(HudData::default()));
        {
            let mut d = bridge.borrow_mut();
            d.snapshot.unit_id = "100".into();
            d.snapshot.name = "Target self fixture".into();
            d.snapshot.level = 30;
            d.snapshot.hp = 100;
            d.snapshot.max_hp = 100;
            d.snapshot.mp = 100;
            d.snapshot.max_mp = 100;
            d.feature_limits = (Some(55), Some(50));
            d.panels.actions_known = true;
            d.panels.bag_slots = 50;
            if let Ok(game) = crate::hud::data::GameData::load(&db) {
                let _ = d.game.0.set(game);
            }
        }
        crate::hud::api::install(&mut rt, &bridge)?;
        let path = crate::settings::isolated_bindings(&rt)?;
        rt.run_stage(aa_ui::Stage::World);
        assert!(rt.errors.is_empty(), "{:?}", rt.errors);
        crate::hud::api::enter_world(&rt, &bridge, &aa_net::state::Chat::default());
        Ok((rt, bridge, path))
    }

    #[test]
    fn clicking_the_player_frame_targets_the_player() -> Result<(), Box<dyn std::error::Error>> {
        let pak = std::env::var("AA_PAK").unwrap_or_else(|_| aa_pak::DEFAULT_PAK.into());
        let db = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.into());
        if !std::path::Path::new(&pak).exists() || !std::path::Path::new(&db).exists() {
            eprintln!("clicking_the_player_frame_targets_the_player: skipped, no game_pak / compact.sqlite3 (set AA_PAK / AA_DB)");
            return Ok(());
        }
        let (rt, bridge, path) = world_runtime(1280.0, 720.0)?;
        let own = 0x5f21u32;
        let name = "Target self fixture".to_string();
        let mut player_spawn = own_spawn(own);
        player_spawn.unit.name = name.clone();
        {
            let mut data = bridge.borrow_mut();
            data.snapshot.unit_id = own.to_string();
            data.snapshot.name = name.clone();
            data.requests.clear();
        }
        // the original player frame's Click handler calls X2Unit:TargetUnit('player')
        crate::hud::expansion::fixture::click(&rt, "ADDON:GetContent(UIC_PLAYER_UNITFRAME).eventWindow")?;
        assert!(
            bridge.borrow().requests.iter().any(|r| matches!(r, Request::Target { obj_id } if *obj_id == own)),
            "{:?}",
            bridge.borrow().requests
        );

        let mut app = App::new();
        app.insert_non_send(HudBridge(bridge.clone()))
            .insert_resource(Player { spawn: Some(player_spawn) })
            .init_resource::<NextState<crate::Screen>>()
            .init_resource::<Units>()
            .init_resource::<aa_net::state::ActionBar>()
            .init_resource::<aa_net::state::Inventory>()
            .init_resource::<Target>()
            .init_resource::<aa_net::state::Skills>()
            .init_resource::<aa_net::state::Buffs>()
            .init_resource::<InFlight>()
            .add_message::<NetCommand>()
            .add_message::<crate::hud::alerts::UseFailed>()
            .add_message::<crate::ingame::ground_target::SkillPressed>()
            .add_systems(Update, crate::hud::api::handle_requests);
        app.update();
        assert!(matches!(&sent(&mut app)[..], [Command::Target { obj_id }] if *obj_id == own));
        assert_eq!(app.world().resource::<Target>().obj_id, Some(own));

        // the target frame the original script builds from the pushed panels
        let snapshot = bridge.borrow().snapshot.clone();
        let (panels, _, _) =
            crate::hud::panels::refresh(app.world(), None, 0, own, std::time::Instant::now(), &snapshot, Vec3::ZERO);
        {
            let mut data = bridge.borrow_mut();
            data.panels = panels;
            data.panels_pushed = Some(crate::hud::panels::Panels::default());
        }
        crate::hud::api::push_changes(&rt, &bridge);
        let shown: String = rt.lua.load("return X2Unit:UnitName('target')").eval()?;
        assert_eq!(shown, name);
        let visible: bool = rt.lua.load("return targetFrame ~= nil and targetFrame:IsVisible()").eval()?;
        assert!(visible, "the target frame must show the player");
        std::fs::remove_file(path).ok();
        Ok(())
    }

    #[test]
    fn tab_from_a_manned_cannon_measures_from_its_world_position() {
        // The cannon's packet position is its parent-local attach point; the ship is far away, so
        // only composing the parent puts the cannon (and the Tab search) at world (5, 0, 0).
        let ship = slave(20, Vec3::new(1000.0, 0.0, 0.0), None);
        let cannon = slave(21, Vec3::new(-995.0, 0.0, 0.0), Some(20));
        let npc = unit(
            30,
            UnitKind::Npc {
                template_id: 1,
                owner_id: 0,
            },
            Vec3::new(10.0, 0.0, 0.0),
            None,
        );
        let mut units = Units::default();
        for unit in [&ship, &cannon, &npc] {
            units.by_id.insert(unit.obj_id, unit.clone());
        }
        let own = crate::hud::fixture_spawn();
        let state: Shared = Rc::new(RefCell::new(HudData::default()));
        mounted::receive(&state, &aa_net::Event::CharacterSelected(Box::new(own.state.clone())));
        mounted::receive(&state, &aa_net::Event::PlayerSpawn(Box::new(own.clone())));
        mounted::receive(&state, &aa_net::Event::UnitSpawned(Box::new(ship)));
        mounted::receive(&state, &aa_net::Event::UnitSpawned(Box::new(cannon.clone())));
        mounted::receive(
            &state,
            &aa_net::Event::UnitAttached {
                obj_id: own.obj_id,
                mount: cannon.obj_id,
                point: 2,
                reason: 0,
            },
        );
        // the HUD point stayed at the boarding point, 500 m from every unit
        state.borrow_mut().pos = Vec3::new(500.0, 0.0, 0.0);

        let mut app = App::new();
        app.insert_non_send(HudBridge(state))
            .insert_resource(units)
            .insert_resource(Player {
                spawn: Some(PlayerSpawn {
                    obj_id: own.obj_id,
                    state: own.state.clone(),
                    unit: own.unit.clone(),
                }),
            })
            .init_resource::<Target>()
            .init_resource::<HudKeyboardFocus>()
            .init_resource::<super::super::chat::ScriptedKeys>()
            .init_resource::<crate::settings::Actions>()
            .add_message::<NetCommand>()
            .add_systems(Update, tab_target);
        app.world_mut()
            .resource_mut::<crate::settings::Actions>()
            .tap_for_test("cycle_hostile_forward");
        app.update();
        let commands: Vec<_> = app
            .world_mut()
            .resource_mut::<Messages<NetCommand>>()
            .drain()
            .map(|message| message.0)
            .collect();
        // from the cannon's world position the cannon itself is the nearest unit in range; from
        // the stale HUD point nothing would be in the 40 m range and no command would be sent
        assert_eq!(commands.len(), 1, "{commands:?}");
        assert!(
            matches!(commands[0], Command::Target { obj_id } if obj_id == cannon.obj_id),
            "{commands:?}"
        );
    }
}
