//! Original unit-frame Bloodlust controls on explicit synthetic received reports.
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use aa_ui::UiRuntime;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use super::super::{api, data::GameData, panels::TargetSnap};

pub(super) fn runtime(width: f32, height: f32) -> Result<(UiRuntime, api::Shared, PathBuf), Box<dyn std::error::Error>> {
    let mut rt = UiRuntime::new("en_us", width, height)?;
    rt.load_ui_texts(std::path::Path::new(aa_anim::db::DEFAULT_DB))?;
    let bridge = Rc::new(RefCell::new(api::HudData::default()));
    {
        let mut d = bridge.borrow_mut();
        d.snapshot.unit_id = "100".into();
        d.snapshot.name = "Bloodlust fixture".into();
        d.snapshot.level = 30;
        d.snapshot.hp = 100;
        d.snapshot.max_hp = 100;
        d.snapshot.mp = 100;
        d.snapshot.max_mp = 100;
        d.panels.actions_known = true;
        d.panels.bag_slots = 50;
        d.panels.target = Some(TargetSnap {
            obj_id: 200,
            name: "Received character".into(),
            level: 30,
            hp: 100,
            max_hp: 100,
            mp: 100,
            max_mp: 100,
            unit_type: "character",
            detail: "character",
            relation: "neutral",
            ..default()
        });
        d.pvp.announce(100, true, true);
        d.pvp.announce(200, true, false);
        assert!(d.game.0.set(GameData::load(aa_anim::db::DEFAULT_DB)?).is_ok());
    }
    api::install(&mut rt, &bridge)?;
    let path = crate::settings::isolated_bindings(&rt)?;
    rt.run_stage(aa_ui::Stage::World);
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    rt.lua.load("PVP_PLAYER=ADDON:GetContent(UIC_PLAYER_UNITFRAME); assert(PVP_PLAYER)").exec()?;
    api::enter_world(&rt, &bridge, &aa_net::state::Chat::default());
    // The original event's broad unit category is "npc" for characters too (target.lua:409).
    rt.fire_event("TARGET_CHANGED", ("200", "npc"));
    Ok((rt, bridge, path))
}

pub(crate) fn run() {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    let (rt, bridge, path) = runtime(width as f32, height as f32).unwrap();
    // Explicit fixture echoes, never represented as a live server exchange.
    super::dispatch(&rt, &bridge, &aa_net::Event::ForceAttackSet { obj_id: 100, on: false });
    super::dispatch(&rt, &bridge, &aa_net::Event::ForceAttackSet { obj_id: 100, on: true });
    super::dispatch(&rt, &bridge, &aa_net::Event::ForceAttackSet { obj_id: 200, on: true });
    rt.lua
        .load("assert(PVP_PLAYER:IsVisible() and targetFrame:IsVisible()); assert(PVP_PLAYER.pvpIcon:IsVisible() and targetFrame.pvpIcon:IsVisible())")
        .exec()
        .unwrap();
    App::new()
        .add_plugins((aa_dev::default_plugins("aa-client: original Bloodlust HUD"), aa_dev::DevPlugin))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(crate::login::host::ui_config()),
        })
        .insert_non_send(rt)
        .insert_non_send(api::HudBridge(bridge))
        .add_systems(Startup, physical_pixels)
        .run();
    if path.exists() {
        std::fs::remove_file(path).unwrap();
    }
}

fn physical_pixels(mut windows: Query<&mut Window, With<PrimaryWindow>>) {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    for mut window in &mut windows {
        window.resolution.set_scale_factor_override(Some(1.0));
        window.resolution.set_physical_resolution(width, height);
    }
}
