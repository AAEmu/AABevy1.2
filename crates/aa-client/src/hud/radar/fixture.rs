//! Explicit synthetic receipts on the actual catalog and original world-stage maps.
use super::*;
use aa_net::{
    radar::{FishRadarContact, RadarToggle, ShipRadarContact, TransferRadarContact},
    wire::Pos,
};
use bevy::window::PrimaryWindow;

pub(super) fn runtime(width: f32, height: f32) -> Result<(UiRuntime, Shared, std::path::PathBuf), Box<dyn std::error::Error>> {
    let (rt, bridge, path) = super::super::breath::fixture::runtime(width, height)?;
    let (rect, slave, fish, transfers) = {
        let mut data = bridge.borrow_mut();
        let game = data.game.get().unwrap();
        let zone = game.zone_groups.get(&5).unwrap();
        let rect = zone.rect;
        let key = zone.key.clone();
        let slave = *game.radar.kinds.keys().min().unwrap();
        let fish = *game.radar.fish_names.keys().min().unwrap();
        let transfers: Vec<u32> = [Symbol::Airship, Symbol::Carriage, Symbol::Landship]
            .into_iter()
            .map(|symbol| *game.radar.transfers.iter().filter(|(_, s)| **s == symbol).map(|(id, _)| id).min().unwrap())
            .collect();
        data.snapshot.zone_group = 5;
        data.snapshot.zone_rect = rect;
        data.snapshot.zone_key_name = key;
        data.pos = Vec3::new(rect[0] + rect[2] * 0.7, rect[1] + rect[3] * 0.7, 0.);
        data.radar.faction = 4;
        (rect, slave, fish, transfers)
    };
    super::super::api::push_changes(&rt, &bridge);
    rt.lua
        .load("ADDON:ShowContent(UIC_WORLDMAP,true); roadmap:SetMapSize(100); roadmapWindow:SetExtent(926,556); roadmapWindow:Show(true)")
        .exec()?;
    aa_ui::lua::tick(&rt.lua, 500.);
    rt.lua.load("SetExpandLevel(1,true)").exec()?;
    let position = |u: f32, v: f32| Pos::new(rect[0] + u * rect[2], rect[1] + (1. - v) * rect[3], 0.);
    let ships = (0..4)
        .map(|i| ShipRadarContact {
            obj_id: 201 + i,
            summoner_id: 47,
            raw_unit_type: if i % 2 == 0 { 2 } else { 7 },
            template_id: slave,
            pos: position(0.32 + i as f32 * 0.08, 0.44 + i as f32 * 0.07),
            faction_id: if i < 2 { 4 } else { 3 },
            expedition_id: 0,
            name: format!("Radar test {}", i + 1),
            summoner_name: "Test captain".into(),
            hp: 720,
            max_hp: 1000,
            raw_tail: 3554,
        })
        .collect();
    for event in [
        RadarEvent::ShipToggled(RadarToggle { on: true, range: 1200. }),
        RadarEvent::TransferToggled(RadarToggle { on: true, range: 2000. }),
        RadarEvent::FishToggled(RadarToggle { on: true, range: 500. }),
        RadarEvent::ShipContacts { last: true, contacts: ships },
        RadarEvent::TransferContacts {
            last: true,
            contacts: transfers
                .into_iter()
                .enumerate()
                .map(|(i, id)| TransferRadarContact {
                    obj_id: 301 + i as u32,
                    template_id: id,
                    pos: position(0.33 + i as f32 * 0.1, 0.7),
                    name: format!("Received transport {}", i + 1),
                })
                .collect(),
        },
        RadarEvent::FishContacts {
            last: true,
            contacts: vec![FishRadarContact {
                obj_id: 401,
                template_id: fish,
                pos: position(0.24, 0.58),
            }],
        },
    ] {
        dispatch(&rt, &bridge, &Event::Radar(event));
    }
    aa_ui::lua::tick(&rt.lua, 16.);
    Ok((rt, bridge, path))
}

pub(crate) fn run() {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    let (rt, bridge, path) = runtime(width as f32, height as f32).unwrap();
    App::new()
        .add_plugins((aa_dev::default_plugins("aa-client: received radar maps"), aa_dev::DevPlugin))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(crate::login::host::ui_config()),
        })
        .insert_non_send(rt)
        .insert_non_send(HudBridge(bridge))
        .add_systems(Startup, physical_pixels)
        .run();
    if path.exists() {
        std::fs::remove_file(path).unwrap();
    }
}

fn physical_pixels(mut windows: Query<&mut Window, With<PrimaryWindow>>) {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    for mut window in &mut windows {
        window.resolution.set_scale_factor_override(Some(1.));
        window.resolution.set_physical_resolution(width, height);
    }
}
