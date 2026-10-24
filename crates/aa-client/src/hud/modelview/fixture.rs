//! Supplied received-appearance fixture, original assets/camera/crop; no commerce or socket.
use super::super::{
    api::{HudBridge, HudData},
    salon::AppliedMode,
};
use crate::ingame::data::Db;
use aa_net::{
    model::{EquipDetail, Item, ItemDetail, ModelParams, slot},
    state::Player,
};
use aa_ui::UiRuntime;
use bevy::prelude::*;

#[derive(Resource)]
struct Fixture {
    case: String,
    phase: bool,
    verified: bool,
    before_suspend: Option<(Entity, u64)>,
}
pub(super) fn run() {
    let args: Vec<_> = std::env::args().collect();
    let case = args
        .windows(2)
        .find(|v| v[0] == "--case")
        .map_or("initial".to_owned(), |v| v[1].clone());
    App::new()
        .add_plugins((
            aa_dev::default_plugins("Original-asset widget modelview"),
            aa_dev::DevPlugin,
            aa_cry::CryPlugin,
            aa_anim::AnimPlugin,
            aa_anim::facial::FacialPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(aa_ui::UiConfig {
                clear: bevy::camera::ClearColorConfig::Custom(Color::srgb(0.035, 0.045, 0.065)),
                ..default()
            }),
        })
        .insert_resource(Db::open().expect("read-only game catalog"))
        .init_resource::<AppliedMode>()
        .init_resource::<crate::hud::HudBuilt>()
        .init_resource::<crate::hud::HudSuspended>()
        .add_message::<aa_net::NetEvent>()
        .insert_resource(Fixture {
            case,
            phase: false,
            verified: false,
            before_suspend: None,
        })
        .add_plugins(super::plugin)
        .add_systems(
            Update,
            super::super::salon::track_mode.before(aa_ui::UiModelViews),
        )
        .insert_resource(aa_ui::UiSetup::new(|ui| {
            ui.exec(
                "@modelview_original_asset_fixture",
                include_str!("fixture.lua"),
            )
            .unwrap()
        }))
        .add_systems(Startup, setup)
        .add_systems(Update, controls.before(aa_ui::UiFrame))
        .add_systems(Update, fixed_pose.after(aa_ui::UiModelViews))
        .add_systems(Last, verify)
        .run();
}
fn fixed_pose(mut animators: Query<&mut aa_anim::Animator>) {
    for mut animator in &mut animators {
        animator.speed = 0.;
        if let Some(current) = &mut animator.current {
            current.time = 0.4;
        }
    }
}
fn item(template_id: u32) -> Item {
    Item {
        template_id,
        id: 9_007_199_254_740_993 + u64::from(template_id),
        count: 1,
        detail: ItemDetail::Equip(EquipDetail::default()),
        ..default()
    }
}
fn setup(world: &mut World) {
    world.resource_mut::<crate::hud::HudBuilt>().up = true;
    let mut spawn = crate::hud::fixture_spawn();
    spawn.unit.model_id = 11;
    spawn.state.character.gender = aa_net::model::gender::FEMALE;
    spawn.unit.model_params = ModelParams::default_face();
    for (slot, template) in [
        (slot::HAIR, 24024),
        (slot::CHEST, 23387),
        (slot::LEGS, 23388),
        (slot::FEET, 18180),
        (slot::MAINHAND, 5569),
    ] {
        spawn.unit.equipment.slots[slot] = Some(item(template));
    }
    let bridge = HudBridge(std::rc::Rc::new(
        std::cell::RefCell::new(HudData::default()),
    ));
    bridge.0.borrow_mut().character_id = spawn.character().id;
    // Seed the exact same authority fold used by the live HUD, with an explicitly
    // supplied complete snapshot. No template/default authority fallback in host.
    super::super::enchant::dispatch(
        None,
        &bridge.0,
        &aa_net::Event::PlayerSpawn(Box::new(spawn.clone())),
        None,
    );
    world.insert_non_send(bridge);
    world.insert_resource(Player { spawn: Some(spawn) });
    if world.resource::<Fixture>().case.ends_with("-applied") {
        world.write_message(aa_net::NetEvent(aa_net::Event::Salon(
            aa_net::salon::SalonEvent::Toggled(true),
        )));
    }
    info!(
        "modelview fixture: supplied Nuian female snapshot; actual mesh/camera/clip assets, stock lighting approximation"
    );
}
fn controls(world: &mut World) {
    if world.resource::<Fixture>().case == "suspend"
        && world.resource::<Fixture>().phase
        && world.resource::<Time>().elapsed_secs() >= 5.
        && world.get_non_send::<crate::hud::SuspendedStage>().is_some()
    {
        assert!(crate::hud::resume(world));
    }
    if world.resource::<Fixture>().phase
        || world.resource::<Time>().elapsed_secs() < 3.
        || !super::host::fixture_ready(world)
    {
        return;
    }
    let Some(rt) = world.get_non_send::<UiRuntime>() else {
        return;
    };
    let case = world.resource::<Fixture>().case.clone();
    if case == "suspend" {
        world.resource_mut::<Fixture>().before_suspend = super::host::fixture_identity(world);
        assert!(crate::hud::suspend(world));
        world.insert_non_send(UiRuntime::new("en_us", 1920., 1080.).unwrap());
        world.resource_mut::<Fixture>().phase = true;
        return;
    }
    let code = match case.as_str() {
        "initial" | "initial-applied" => "",
        "baseline" | "clear" => "view:ClearModel()",
        "weapon" => {
            "view:StopAnimation();view:EquipItem(5660);view:PlayAnimation('onehand_ba_combat_idle',true)"
        }
        "staff" => {
            "view:StopAnimation();view:EquipItem(6102);view:PlayAnimation('onehand_ba_combat_idle',true)"
        }
        "armor" => "view:EquipItem(8000028)",
        "reset" => {
            "view:EquipItem(8000028);view:EquipItem(5660);view:ResetEquips();view:PlayAnimation('fist_ba_relaxed_rand_idle',true)"
        }
        "clear-reset" | "clear-reset-applied" => {
            "view:EquipItem(8000028);view:ClearModel();view:SetModelViewExtent(270,450);view:ResetEquips();view:PlayAnimation('fist_ba_relaxed_rand_idle',true)"
        }
        "resize" => "view:SetTextureSize(700,600);view:SetModelViewCoords(210,80,270,450)",
        "hidden" => "view:Show(false)",
        "lifecycle" => {
            "view:EquipItem(8000028);view:ClearModel();view:Init('player',true);view:SetIngameShopMode(true);view:AdjustCameraPos(-.1,.3,-.15);view:PlayAnimation('fist_ba_relaxed_rand_idle',true)"
        }
        _ => panic!("unknown modelview fixture case {case}"),
    };
    rt.exec("@modelview_fixture_control", code).unwrap();
    info!("modelview fixture: control={case}");
    world.resource_mut::<Fixture>().phase = true;
}
fn verify(world: &mut World) {
    let fixture = world.resource::<Fixture>();
    if fixture.verified || !fixture.phase || world.resource::<Time>().elapsed_secs() < 7. {
        return;
    }
    let case = fixture.case.clone();
    if super::host::verify_fixture(world, &case) {
        if case == "suspend" {
            assert_eq!(
                super::host::fixture_identity(world),
                world.resource::<Fixture>().before_suspend
            );
            assert!(world.get_non_send::<crate::hud::SuspendedStage>().is_none());
        }
        world.resource_mut::<Fixture>().verified = true;
        info!(
            "modelview fixture PASS: {case}; actor/clip/target/snapshot/lifecycle assertions (GPU readiness is not implied)"
        );
        info!(
            "modelview fixture: deferred {} draws until their exact Bevy mesh group existed",
            super::readiness::deferred(world)
        );
    }
}
