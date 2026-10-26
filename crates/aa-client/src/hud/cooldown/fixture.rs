//! Actual original equipped-item buttons, bound to two real use-skill items and received timers.
use super::*;
use aa_net::model::{EquipDetail, Item, ItemDetail, slot};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use std::path::PathBuf;

pub(super) fn runtime(width: f32, height: f32) -> Result<(UiRuntime, Shared, PathBuf), Box<dyn std::error::Error>> {
    let (rt, bridge, path) = super::super::breath::fixture::runtime(width, height)?;
    {
        let mut data = bridge.borrow_mut();
        let mut equipment = vec![None; aa_net::model::slot::COUNT];
        for (index, template_id) in [(slot::FEET, 15875), (slot::BACK, 26743)] {
            let item = Item {
                template_id,
                id: 9000 + index as u64,
                grade: 1,
                count: 1,
                detail: ItemDetail::Equip(EquipDetail {
                    durability: 100,
                    ..Default::default()
                }),
                ..Default::default()
            };
            equipment[index] = Some(super::super::panels::item_slot(data.game.get(), template_id, 1, item.grade).with_instance(&item));
        }
        data.panels.equipment = equipment;
        data.cooldowns
            .apply(&aa_net::Event::PlayerSpawn(Box::new(super::super::expansion::fixture::spawn())));
        let catalog = data.game.get().unwrap().skills.iter().map(|(&id, info)| (id, info.tag)).collect();
        data.cooldowns.set_cooldown_catalog(catalog);
        data.cooldowns.apply(&aa_net::Event::Cooldowns(aa_net::skill::SkillCooldowns {
            skills: vec![],
            tags: [(655, 18_000, 30_000), (1004, 12_000, 300_000)]
                .into_iter()
                .map(|(id, remaining_ms, total_ms)| aa_net::Cooldown {
                    id,
                    remaining_ms,
                    total_ms,
                })
                .collect(),
        }));
        let hud: Table = rt.lua.globals().get("HUD")?;
        super::super::panels::push(&rt.lua, &hud, &data.panels, None)?;
    }
    rt.lua
        .load(
            r#"
      ADDON:ShowContent(UIC_BAG,false)
      equippedItem:Update();equippedItem:Show(true);equippedItem:ShowProc()
      local feet=X2Equipment:GetEquippedItemTooltipText('player',ES_FEET)
      local back=X2Equipment:GetEquippedItemTooltipText('player',ES_BACK)
      assert(feet.itemType==15875 and feet.skillType==13419)
      assert(back.itemType==26743 and back.skillType==20676)
    "#,
        )
        .set_name("@equipment_cooldown_fixture")
        .exec()?;
    Ok((rt, bridge, path))
}

pub(crate) fn run() {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    let (rt, bridge, path) = runtime(width as f32, height as f32).unwrap();
    if aa_dev::args().into_iter().any(|arg| arg == "--stopped") {
        let mut data = bridge.borrow_mut();
        data.cooldowns.apply(&aa_net::Event::SkillCooldownReset {
            obj_id: 100,
            skill_id: 13419,
            tag_id: 655,
            gcd: false,
        });
        data.cooldowns.apply(&aa_net::Event::SkillStarted(aa_net::skill::SkillStarted {
            skill_id: 13419,
            tl: 8,
            caster: aa_net::buff::BuffCaster {
                obj_id: 100,
                ..Default::default()
            },
            target: aa_net::skill::SkillTarget::unit(100),
            cast_ms: 100,
            base_cast_ms: 100,
            synergy: 0,
            result: 0,
            result_value: 0,
            raw_result: Default::default(),
            raw_caster: aa_net::SkillCasterData::Unit(100),
            raw_target: aa_net::SkillTargetData::Unit(100),
        }));
        data.cooldowns.apply(&aa_net::Event::CastingStopped { tl: 8, duration_ms: 12000 });
    }
    if aa_dev::args().into_iter().any(|arg| arg == "--ready") {
        for entry in &mut bridge.borrow_mut().cooldowns.native_tag_cooldowns {
            entry.reset = true;
        }
    }
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-client: original equipment cooldowns"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(crate::login::host::ui_config()),
        })
        .insert_non_send(rt)
        .insert_non_send(super::super::api::HudBridge(bridge))
        .add_systems(Startup, physical_pixels)
        .add_systems(Update, advance_fixture.before(aa_ui::UiInput))
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

fn advance_fixture(time: Res<Time<Real>>, bridge: NonSend<super::super::api::HudBridge>) {
    bridge
        .0
        .borrow_mut()
        .cooldowns
        .advance_native_frame(time.elapsed().as_millis() as u32);
}
