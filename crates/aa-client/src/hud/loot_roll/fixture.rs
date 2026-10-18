//! Original dice windows on explicit fixture offers, without a network connection.
use std::cell::RefCell;
use std::rc::Rc;

use aa_net::model::{EquipDetail, Item, ItemDetail};
use bevy::window::PrimaryWindow;

use super::*;

pub(super) fn runtime(width: f32, height: f32) -> Result<UiRuntime, Box<dyn std::error::Error>> {
    let mut rt = UiRuntime::new("en_us", width, height)?;
    rt.load_ui_texts(std::path::Path::new(aa_anim::db::DEFAULT_DB))?;
    let bridge = Rc::new(RefCell::new(super::super::api::HudData::default()));
    let game = super::super::data::GameData::load(aa_anim::db::DEFAULT_DB)?;
    let quest = game.quests.defs.iter().filter(|(_, q)| !q.name.is_empty()).map(|(id, _)| *id).min().unwrap();
    rt.lua.globals().set("FIXTURE_QUEST", quest)?;
    for item in offers() {
        assert!(game.items.contains_key(&item.template_id));
    }
    {
        let mut d = bridge.borrow_mut();
        d.snapshot.unit_id = "100".into();
        d.snapshot.name = "Loot fixture".into();
        d.snapshot.level = 30;
        d.snapshot.hp = 100;
        d.snapshot.max_hp = 100;
        d.panels.actions_known = true;
        d.panels.bag_slots = 50;
        assert!(d.game.0.set(game).is_ok());
    }
    super::super::api::install(&mut rt, &bridge)?;
    rt.run_stage(aa_ui::Stage::World);
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    super::super::api::enter_world(&rt, &bridge, &aa_net::state::Chat::default());
    Ok(rt)
}

/// Real templates from original DebugShowLootDiceWindow; explicit synthetic instances.
pub(super) fn offers() -> Vec<Item> {
    [24488, 14698, 24498, 24488]
        .into_iter()
        .enumerate()
        .map(|(index, template_id)| Item {
            template_id,
            id: aa_net::loot_roll::LootRollKey {
                owner_type: 1,
                owner_obj_id: 123,
                item_index: index as u16 + 1,
            }
            .item_id(),
            grade: [4, 3, 2, 4][index],
            count: if template_id == 14698 { 4 } else { 1 },
            detail: if template_id == 14698 {
                ItemDetail::None
            } else {
                ItemDetail::Equip(EquipDetail { durability: 37, ..default() })
            },
            ..default()
        })
        .collect()
}

pub(crate) fn run() {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    let rt = runtime(width as f32, height as f32).unwrap();
    let mut rolls = LootRolls::default();
    for item in offers() {
        rolls.apply(&Event::LootRoll(LootRollEvent::Offered(item)));
    }
    App::new()
        .add_plugins((aa_dev::default_plugins("aa-client: original loot dice"), aa_dev::DevPlugin))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(crate::login::host::ui_config()),
        })
        .insert_non_send(rt)
        .insert_resource(rolls)
        .add_message::<NetEvent>()
        .add_message::<NetCommand>()
        .add_systems(Startup, physical_pixels)
        .add_systems(Update, sync.before(aa_ui::UiFrame))
        .run();
}

fn physical_pixels(mut windows: Query<&mut Window, With<PrimaryWindow>>) {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    for mut window in &mut windows {
        window.resolution.set_scale_factor_override(Some(1.0));
        window.resolution.set_physical_resolution(width, height);
    }
}
