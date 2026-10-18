//! Explicit received item results over the unchanged original chat/alarm scripts; no session.
use super::*;
use aa_net::{
    items::container,
    model::{EquipDetail, ItemDetail},
};
use bevy::window::PrimaryWindow;
use std::path::PathBuf;

pub(super) fn item() -> Item {
    Item {
        id: 9_007_199_254_740_993,
        template_id: 24488,
        grade: 1,
        count: 1,
        detail: ItemDetail::Equip(EquipDetail {
            durability: 39,
            ..default()
        }),
        ..default()
    }
}
pub(super) fn seed(state: &Shared) {
    let mut spawn = super::super::expansion::fixture::spawn();
    spawn.state.num_inventory_slots = 10;
    spawn.state.num_bank_slots = 10;
    let mut items = vec![None; 10];
    items[0] = Some(item());
    for event in [
        Event::CharacterSelected(Box::new(spawn.state.clone())),
        Event::PlayerSpawn(Box::new(spawn)),
        Event::InventoryInit {
            bag_slots: 10,
            bank_slots: 10,
        },
        Event::InventoryChunk(aa_net::items::InvenChunk {
            container: container::INVENTORY,
            chunks: 1,
            start_chunk: 0,
            items,
            truncated: false,
        }),
    ] {
        dispatch(None, state, &event);
    }
}
pub(super) fn runtime(width: f32, height: f32) -> Result<(UiRuntime, Shared, PathBuf), Box<dyn std::error::Error>> {
    let result = super::super::breath::fixture::runtime(width, height)?;
    seed(&result.1);
    Ok(result)
}
pub(super) fn confirm(rt: &UiRuntime) -> Result<(), Box<dyn std::error::Error>> {
    let square = aa_ui::lua::value_id(&rt.lua, &rt.lua.load("gradeEnchantAlarm.squareFrame").eval()?).ok_or("grade frame missing")?;
    let button = {
        let ui = aa_ui::lua::ui(&rt.lua);
        ui.tree
            .widget(square)
            .unwrap()
            .children
            .iter()
            .copied()
            .find(|&id| ui.tree.objects[id as usize].name.ends_with("okButton"))
            .ok_or("original private grade Confirm missing")?
    };
    rt.lua.globals().set(
        "ITEM_RESULT_CONFIRM",
        aa_ui::lua::object_table(&rt.lua, button)?.ok_or("grade Confirm object missing")?,
    )?;
    super::super::expansion::fixture::click(rt, "ITEM_RESULT_CONFIRM")
}
fn gem(state: &Shared, magical: bool) -> u32 {
    state
        .borrow()
        .game
        .get()
        .unwrap()
        .items
        .iter()
        .filter(|(_, item)| item.named.name.contains(if magical { "Lunafrost" } else { "Lunagem" }))
        .map(|(&id, _)| id)
        .min()
        .expect("actual catalog lunagem/lunafrost")
}
pub(super) fn result(state: &Shared, mode: &str) -> Event {
    match mode {
        mode if mode.starts_with("grade") => {
            let code = match mode {
                "grade-break" => 0,
                "grade-downgrade" => 1,
                "grade-fail" => 2,
                "grade-great" => 4,
                _ => 3,
            };
            let before = if code == 1 { 2 } else { 1 };
            let after = if code >= 3 { 2 } else { 1 };
            let mut item = item();
            item.grade = after;
            Event::GradeEnchantResult(aa_net::item_effect::GradeEnchantResult {
                raw_result: code,
                item: Some(item),
                item_wire: Vec::new(),
                before_grade: before,
                after_grade: after,
            })
        }
        "magical" => Event::LunastoneSocketingResult {
            raw_result: 1,
            item_id: item().id,
            template_id: gem(state, true),
        },
        _ => Event::LunagemSocketingResult {
            raw_result: u8::from(mode != "socket-fail"),
            item_id: item().id,
            template_id: gem(state, false),
            raw_install: 1,
        },
    }
}
pub(in crate::hud) fn run() {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    let (rt, state, path) = runtime(width as f32, height as f32).unwrap();
    let mode = std::env::args()
        .find_map(|arg| {
            [
                "grade",
                "grade-break",
                "grade-downgrade",
                "grade-fail",
                "grade-great",
                "socket-fail",
                "magical",
            ]
            .into_iter()
            .find(|name| arg == format!("--{name}"))
        })
        .unwrap_or("socket");
    let receipt = result(&state, mode);
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-client: original item-result callbacks"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(crate::login::host::ui_config()),
        })
        .insert_non_send(rt)
        .insert_non_send(HudBridge(state))
        .add_systems(Startup, physical_pixels)
        .add_systems(
            Update,
            (move |rt: NonSend<UiRuntime>, bridge: NonSend<HudBridge>, time: Res<Time>, mut sent: Local<bool>| {
                if !*sent && time.elapsed_secs() >= 1. {
                    dispatch(Some(&rt), &bridge.0, &receipt);
                    *sent = true;
                }
            })
            .before(aa_ui::UiFrame),
        )
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
