//! Explicit received score fixture; the original bag and tooltip Lua execute unchanged.
use super::*;
use crate::hud::{api, panels};
use aa_net::Event;
use aa_ui::UiRuntime;
use bevy::prelude::*;

pub(super) fn runtime() -> Result<(UiRuntime, api::Shared, PathBuf), Box<dyn std::error::Error>> {
    let (rt, state, path) = crate::hud::user_music::fixture::runtime()?;
    api::mark_pushed(&state);
    let mut inventory = aa_net::state::Inventory::default();
    inventory.bag_slots = 50;
    inventory.bank_slots = 50;
    inventory.bag.resize(50, None);
    inventory.bag[4] = Some(aa_net::model::Item {
        id: 100,
        template_id: 28051,
        count: 1,
        detail: aa_net::model::ItemDetail::Other(8),
        detail_payload: Some(42u64.to_le_bytes().to_vec()),
        ..Default::default()
    });
    crate::hud::enchant::dispatch(
        None,
        &state,
        &Event::InventoryChunk(aa_net::items::InvenChunk {
            container: 2,
            chunks: 5,
            start_chunk: 0,
            items: inventory.bag.clone(),
            truncated: false,
        }),
        None,
    );
    let shared = state.borrow().user_music.cache.clone();
    shared.borrow_mut().identity(Some(17), 47, 100);
    sync_directory(&rt.lua, &shared);
    {
        let mut world = World::new();
        world.insert_resource(inventory);
        let mut data = state.borrow_mut();
        data.panels = panels::refresh(
            &world,
            data.game.get(),
            0,
            100,
            data.start,
            &data.snapshot,
            Vec3::ZERO,
        )
        .0;
        data.panels.actions_known = true;
    }
    api::push_changes(&rt, &state);
    rt.fire_event("BAG_UPDATE", (-1i64, -1i64));
    rt.lua.load("X2Bag:SwitchTab(1);ADDON:ShowContent(UIC_BAG,true);assert(newbag:IsVisible());assert(HUD.bag[5].id==28051)").exec()?;
    aa_ui::lua::tick(&rt.lua, 16.);
    aa_ui::lua::flush(&rt.lua);
    Ok((rt, state, path))
}

fn target(rt: &UiRuntime) -> Vec2 {
    let value = rt.lua.load("newbag.slots.btns[5]").eval().unwrap();
    let id = aa_ui::lua::value_id(&rt.lua, &value).unwrap();
    let ui = aa_ui::lua::ui(&rt.lua);
    let r = ui.tree.rect(id);
    Vec2::new(
        (r.x + r.w / 2.) * ui.ui_scale,
        (r.y + r.h / 2.) * ui.ui_scale,
    )
}

pub(super) fn lines(rt: &UiRuntime) -> Vec<String> {
    let value = rt
        .lua
        .load("tooltip.tooltipWindows[1].tooltipItemAndSkill")
        .eval()
        .unwrap();
    let id = aa_ui::lua::value_id(&rt.lua, &value).unwrap();
    aa_ui::lua::ui(&rt.lua)
        .tree
        .widget(id)
        .unwrap()
        .tip
        .as_ref()
        .unwrap()
        .lines
        .iter()
        .map(|line| line.left.raw.clone())
        .collect()
}

pub(in crate::hud) fn run() {
    let (rt, state, path) = runtime().unwrap();
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-client: received stored score"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(crate::login::host::ui_config()),
        })
        .insert_non_send(rt)
        .insert_non_send(api::HudBridge(state))
        .add_systems(
            Update,
            sequence.after(aa_ui::UiInput).before(aa_ui::UiFrame),
        )
        .run();
    if path.exists() {
        std::fs::remove_file(&path).unwrap();
    }
    let dir = path.with_extension("music");
    if dir.exists() {
        std::fs::remove_dir_all(dir).unwrap();
    }
}

fn sequence(
    rt: NonSend<UiRuntime>,
    bridge: NonSend<api::HudBridge>,
    time: Res<Time>,
    mut pointer: ResMut<aa_ui::UiVirtualPointer>,
    mut phase: Local<u8>,
) {
    let t = time.elapsed_secs();
    let before = std::env::args().any(|a| a == "--before");
    let away = Vec2::new(600., 100.);
    pointer.pos = Some(if t < 0.6 || (4.0..4.3).contains(&t) {
        away
    } else {
        target(&rt)
    });
    if *phase == 0 && t >= 2.5 {
        assert!(!lines(&rt).iter().any(|s| s.contains("Harbor")));
        if !before {
            bridge
                .0
                .borrow()
                .user_music
                .cache
                .borrow_mut()
                .apply(&Event::Notice(NoticeEvent::UserNoteLoaded(Some(
                    UserNote {
                        song_id: 42,
                        tooltip: true,
                        title: "Harbor".into(),
                        notes: "O5 C D E".into(),
                    },
                ))));
        }
        *phase = 1;
        info!(
            "stored score: explicit receipt staged; stationary item tooltip retains its original query"
        );
    }
    if *phase == 1 && t >= 3.8 {
        assert!(
            !lines(&rt).iter().any(|s| s.contains("Harbor")),
            "stationary item tooltip must not refresh"
        );
        *phase = 2;
    }
    if *phase == 2 && t >= 6.0 {
        let text = lines(&rt);
        assert_eq!(
            text.iter().any(|s| s.contains("Harbor")),
            !before,
            "{text:?}"
        );
        assert!(rt.errors.is_empty(), "{:?}", rt.errors);
        info!(
            "stored score: original bag rehover title={}, Lua errors=0",
            !before
        );
        *phase = 3;
    }
}
