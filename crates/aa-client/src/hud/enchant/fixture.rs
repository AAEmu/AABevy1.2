//! Physical controls of the original empty enchant window; no network session.
use super::*;
use bevy::window::PrimaryWindow;
use std::path::PathBuf;

pub(super) fn runtime(width: f32, height: f32) -> Result<(UiRuntime, Shared, PathBuf), Box<dyn std::error::Error>> {
    let (rt, state, path, _) = super::super::expansion::fixture::runtime(width, height)?;
    // The original bag keeps this child private. Obtain its actual widget
    // identity, then send the physical hit/press/release through aa-ui.
    let button = {
        let ui = aa_ui::lua::ui(&rt.lua);
        let found = ui
            .tree
            .objects
            .iter()
            .enumerate()
            .filter(|(_, object)| object.name.ends_with("gradeEnchantBtn"))
            .map(|(id, _)| id as aa_ui::tree::Id)
            .collect::<Vec<_>>();
        assert_eq!(found.len(), 1, "original bag enchant button identity");
        found[0]
    };
    rt.lua.globals().set(
        "ENCHANT_BAG_BUTTON",
        aa_ui::lua::object_table(&rt.lua, button)?.ok_or("bag enchant button missing")?,
    )?;
    super::super::expansion::fixture::click(&rt, "ENCHANT_BAG_BUTTON")?;
    // The click creates the window lazily; the next normal frame lays out
    // its newly skinned tab buttons before another physical click can occur.
    aa_ui::lua::tick(&rt.lua, 16.);
    aa_ui::lua::flush(&rt.lua);
    rt.lua
        .load("assert(GetEnchantWindow():IsVisible() and X2Item:IsInEnchantMode())")
        .exec()?;
    Ok((rt, state, path))
}
pub(super) fn select(rt: &UiRuntime, mode: &str) -> Result<(), Box<dyn std::error::Error>> {
    let page = match mode {
        "socket" | "unsocket" => 2,
        "gem" => 3,
        _ => 1,
    };
    let selected: i64 = rt.lua.load("GetEnchantWindow().tab:GetSelectedTab()").eval()?;
    if selected != page {
        super::super::expansion::fixture::click(rt, &format!("GetEnchantWindow().tab.unselectedButton[{page}]"))?;
        aa_ui::lua::tick(&rt.lua, 16.);
        aa_ui::lua::flush(&rt.lua);
    }
    if mode == "unsocket" {
        super::super::expansion::fixture::click(rt, "GetEnchantWindow().tab.window[2].tabSubMenu.subMenuButtons[2]")?;
    }
    Ok(())
}
pub(super) fn fill(rt: &UiRuntime, state: &Shared, mode: &str, support: bool) -> Result<(), Box<dyn std::error::Error>> {
    use aa_net::{
        Event,
        initial_config::InitialConfig,
        items::InvenChunk,
        model::{EquipDetail, Item, ItemDetail},
        state::Inventory,
    };
    let mut items = vec![None; 50];
    let make = |template_id, id, count| Item {
        template_id,
        id,
        count,
        ..Default::default()
    };
    let mut target = make(31432, 9_007_199_254_740_993, 1);
    target.grade = 6;
    let maximum = state
        .borrow()
        .game
        .get()
        .and_then(|g| g.tips.items.durability(31432, 6))
        .map_or(100, |p| p.0);
    target.detail = ItemDetail::Equip(EquipDetail {
        durability: maximum,
        gem_ids: if mode == "unsocket" { [31003, 0, 0, 0, 0, 0, 0] } else { [0; 7] },
        ..Default::default()
    });
    items[0] = Some(target);
    items[1] = Some(make(28299, 200, 3));
    items[2] = Some(make(31974, 300, 2));
    items[3] = Some(make(31003, 400, 3));
    items[4] = Some(make(327, 500, 1));
    items[5] = Some(make(24632, 600, 3));
    items[6] = Some(make(28299, 700, 4));
    let spawn = super::super::expansion::fixture::spawn();
    let events = [
        Event::InitialConfig(InitialConfig {
            host: vec![],
            feature_bytes: vec![0; 11],
            raw_candidate_count: 0,
            candidate_codes: vec![],
            initial_lp: 0,
            service_flags: [0; 7],
            second_price_type: 0,
            second_password_max_fail_count: 0,
            idle_kick_time: 0,
        }),
        Event::CharacterSelected(Box::new(spawn.state.clone())),
        Event::PlayerSpawn(Box::new(spawn)),
        Event::InventoryInit {
            bag_slots: 50,
            bank_slots: 50,
        },
        Event::InventoryChunk(InvenChunk {
            container: 2,
            chunks: 5,
            start_chunk: 0,
            items,
            truncated: false,
        }),
    ];
    let mut inventory = Inventory::default();
    {
        let mut data = state.borrow_mut();
        // Explicit fixture receipts only; preserve the original mode opened by the bag.
        for event in &events {
            inventory.apply(event);
            data.enchant.selection.receive(event);
        }
        data.character_id = 47;
        let mut world = World::new();
        world.insert_resource(inventory);
        data.panels = super::super::panels::refresh(&world, data.game.get(), 0, 100, data.start, &data.snapshot, Vec3::ZERO).0;
        data.panels.actions_known = true;
    }
    super::super::api::push_changes(rt, state);
    rt.fire_event("BAG_UPDATE", (-1i64, -1i64));
    aa_ui::lua::tick(&rt.lua, 16.);
    aa_ui::lua::flush(&rt.lua);
    right_click(rt, 1)?;
    let source = match mode {
        "socket" => 4,
        "unsocket" => 5,
        "gem" => 6,
        _ => 2,
    };
    right_click(rt, source)?;
    if support {
        right_click(rt, 3)?;
    }
    rt.lua
        .load("assert(X2ItemEnchant:GetTargetItemInfo().itemType==31432);assert(X2ItemEnchant:GetEnchantItemInfo()~=nil)")
        .exec()?;
    Ok(())
}
fn right_click(rt: &UiRuntime, slot: usize) -> Result<(), Box<dyn std::error::Error>> {
    let object: Value = rt.lua.load(format!("newbag.slots.btns[{slot}]")).eval()?;
    let id = aa_ui::lua::value_id(&rt.lua, &object).ok_or("original bag item button missing")?;
    let rect = aa_ui::lua::ui(&rt.lua).tree.rect(id);
    let point = (rect.x + rect.w * 0.5, rect.y + rect.h * 0.5);
    assert_eq!(
        aa_ui::input::pick(&rt.lua, point.0, point.1),
        Some(id),
        "original filled bag slot{slot} physical hit"
    );
    let mut mouse = aa_ui::input::MouseState::default();
    aa_ui::input::update_buttons(
        &rt.lua,
        &mut mouse,
        Some(point),
        aa_ui::input::MouseButtons {
            right_press: true,
            ..Default::default()
        },
        0.,
    );
    aa_ui::input::update_buttons(
        &rt.lua,
        &mut mouse,
        Some(point),
        aa_ui::input::MouseButtons {
            right_release: true,
            ..Default::default()
        },
        0.,
    );
    aa_ui::lua::tick(&rt.lua, 16.);
    aa_ui::lua::flush(&rt.lua);
    Ok(())
}
pub(in crate::hud) fn run() {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    let (rt, state, path) = runtime(width as f32, height as f32).unwrap();
    let mode = std::env::args()
        .find_map(|arg| ["socket", "unsocket", "gem"].into_iter().find(|mode| arg == format!("--{mode}")))
        .unwrap_or("grade");
    // Grade is selected by the real bag button already.
    if mode != "grade" {
        select(&rt, mode).unwrap();
    }
    if std::env::args().any(|arg| arg == "--filled") {
        fill(&rt, &state, mode, std::env::args().any(|arg| arg == "--support")).unwrap();
    }
    if std::env::args().any(|arg| arg == "--execute") {
        // Explicit visual fixture only: the original API queues its captured
        // request and working UPDATE. This example has no Session/transport.
        rt.lua.load("X2ItemEnchant:Execute()").exec().unwrap();
    }
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-client: original enchant mode controls"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(crate::login::host::ui_config()),
        })
        .insert_non_send(rt)
        .insert_non_send(HudBridge(state))
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
