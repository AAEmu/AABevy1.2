//! Original breath bar driven by explicit synthetic reports, with a running local timer.
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use aa_ui::UiRuntime;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use super::super::{api, data::GameData};

pub(crate) fn runtime(width: f32, height: f32) -> Result<(UiRuntime, api::Shared, PathBuf), Box<dyn std::error::Error>> {
    let mut rt = UiRuntime::new("en_us", width, height)?;
    rt.load_ui_texts(std::path::Path::new(aa_anim::db::DEFAULT_DB))?;
    let bridge = Rc::new(RefCell::new(api::HudData::default()));
    {
        let mut d = bridge.borrow_mut();
        d.snapshot.unit_id = "100".into();
        d.snapshot.name = "Breath fixture".into();
        d.snapshot.level = 30;
        d.snapshot.hp = 100;
        d.snapshot.max_hp = 100;
        d.snapshot.mp = 100;
        d.snapshot.max_mp = 100;
        // Explicit received feature bytes, independent of XP tables.
        d.feature_limits = (Some(55), Some(50));
        d.panels.actions_known = true;
        d.panels.bag_slots = 50;
        d.breath.actor = Some(100);
        assert!(d.game.0.set(GameData::load(aa_anim::db::DEFAULT_DB)?).is_ok());
    }
    api::install(&mut rt, &bridge)?;
    let path = crate::settings::isolated_bindings(&rt)?;
    rt.run_stage(aa_ui::Stage::World);
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    api::enter_world(&rt, &bridge, &aa_net::state::Chat::default());
    Ok((rt, bridge, path))
}

pub(crate) fn run() {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    let (rt, bridge, path) = runtime(width as f32, height as f32).unwrap();
    super::dispatch(Some(&rt), &bridge, &aa_net::Event::UnderWater { start: true });
    super::dispatch(Some(&rt), &bridge, &aa_net::Event::SetBreath { breath: 30_000 });
    App::new()
        .add_plugins((aa_dev::default_plugins("aa-client: original breath bar"), aa_dev::DevPlugin))
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
