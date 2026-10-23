use super::super::model::{Model, Tab};
use super::*;

fn source() -> View {
    let mut view = View::default();
    view.model = Model::defaults(["All".into(), "Party".into(), "Trade".into()], 15);
    view.model
        .tabs
        .insert(3, Tab::new(3, 1, "Harbor".into(), 17));
    view.model.windows.insert(1, vec![3]);
    view.model.extra.insert(
        1,
        Extra {
            selected: 0,
            method: 4,
        },
    );
    view.locks[1] = true;
    view
}

#[test]
fn split_preserves_raw_metadata_and_pending_recovery_until_attach() {
    let mut view = source();
    let original = view.model.tabs[&3].clone();
    assert_eq!(allocate(&mut view), Some(2));
    assert_eq!(view.model.extra[&2], Extra::default());
    assert!(!view.locks[2]);
    detached(&mut view, 1, 3);
    assert_eq!(view.model.current_window(3), None);
    assert!(!view.model.windows.contains_key(&1));
    assert!(!view.model.extra.contains_key(&1));
    let restored = Model::load(&view.model.tabs_json(), Some(&extra_json(&view))).unwrap();
    assert_eq!(restored.current_window(3), Some(1));
    assert_eq!(restored.tabs[&3], original);
    assert_eq!(restored.extra[&1].method, 4);
    assert!(saved_locks(&view)[1]);
    // Native destination selection callback precedes Attached.
    view.model.extra.get_mut(&2).unwrap().selected = 0;
    attached(&mut view, 2, 3, 0);
    assert_eq!(view.model.current_window(3), Some(2));
    assert_eq!(view.model.tabs[&3].name, original.name);
    assert_eq!(view.model.tabs[&3].filters, original.filters);
    assert!(view.transfers.is_empty());
    assert!(!saved_locks(&view)[1]);
    assert_eq!(
        Model::load(&view.model.tabs_json(), Some(&extra_json(&view)))
            .unwrap()
            .current_window(3),
        Some(2)
    );
}

#[test]
fn aborted_transfer_restores_source_method_lock_and_selected_callback() {
    let mut view = source();
    allocate(&mut view);
    detached(&mut view, 1, 3);
    view.model.extra.insert(
        1,
        Extra {
            selected: 0,
            method: 0,
        },
    );
    attached(&mut view, 1, 3, 0);
    release(&mut view, 2);
    release(&mut view, 2);
    assert_eq!(view.model.current_window(3), Some(1));
    assert_eq!(
        view.model.extra[&1],
        Extra {
            selected: 0,
            method: 4
        }
    );
    assert!(view.locks[1]);
    assert!(!view.model.windows.contains_key(&2));
    assert!(view.transfers.is_empty());
    release(&mut view, 1);
    assert_eq!(
        view.model.current_window(3),
        Some(1),
        "release never deletes a populated window or tab"
    );
}

#[test]
fn bulk_transfer_keeps_order_and_raw_zero_and_allocator_bounds() {
    let mut view = source();
    assert_eq!(view.model.allocate(1, "Second", 15), Some(4));
    allocate(&mut view);
    detached(&mut view, 1, 3);
    detached(&mut view, 1, 4);
    attached(&mut view, 2, 3, 0);
    attached(&mut view, 2, 4, 1);
    attached(&mut view, 2, 4, 1);
    assert_eq!(view.model.windows[&2], [3, 4]);
    assert_eq!(view.model.windows[&0], [0, 1, 2]);
    assert_eq!(
        allocate(&mut view),
        Some(1),
        "lowest freed native ID is reused"
    );
    for id in 0..8 {
        view.model.windows.entry(id).or_default();
    }
    assert_eq!(allocate(&mut view), None);
    attached(&mut view, 7, 0, 0);
    assert_eq!(
        view.model.current_window(0),
        Some(0),
        "unsolicited Attach cannot steal a live tab"
    );
}

#[test]
fn allocator_disk_failure_rolls_back_unreturned_window_and_settings()
-> Result<(), Box<dyn std::error::Error>> {
    let (rt, state, path) = crate::hud::breath::fixture::runtime(1280., 720.)?;
    state.borrow_mut().chat_config.identity(Some(17), 12345);
    ensure(&rt.lua, &state)?;
    let before = state.borrow().chat_config.model.clone();
    let blocker = path.with_extension("blocked-parent");
    std::fs::write(&blocker, b"not a directory")?;
    crate::settings::reopen_fixture(&rt, blocker.join("settings.json"))?;
    let allocated: Option<i32> = rt.lua.load("return AAUI.ChatAllocateWindow()").eval()?;
    assert_eq!(allocated, None);
    assert_eq!(state.borrow().chat_config.model, before);
    assert!(crate::settings::chat_read(&rt.lua, "17:12345", "tabs").is_none());
    std::fs::remove_file(blocker)?;
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[test]
fn original_split_retains_history_and_restores_window_metadata()
-> Result<(), Box<dyn std::error::Error>> {
    use aa_ui::{input, lua};
    let (rt, state, path) = super::super::fixture::runtime(1280., 720.)?;
    let source = super::super::live_window(&rt.lua, 0).unwrap();
    let row = {
        let u = lua::ui(&rt.lua);
        u.tree
            .widget(source)
            .unwrap()
            .chat
            .as_ref()
            .unwrap()
            .tabs
            .iter()
            .find(|tab| tab.id == 3)
            .unwrap()
            .clone()
    };
    let old = state.borrow().chat_config.model.tabs[&3].clone();
    let old_lines = lua::ui(&rt.lua)
        .tree
        .widget(row.message)
        .unwrap()
        .message
        .as_ref()
        .unwrap()
        .lines
        .clone();
    let r = lua::ui(&rt.lua).tree.rect(row.button);
    let p = (r.x + r.w * 0.5, r.y + r.h * 0.5);
    assert_eq!(input::pick(&rt.lua, p.0, p.1), Some(row.button));
    let mut mouse = input::MouseState::default();
    for (at, down, up) in [
        (p, true, false),
        ((p.0 + 5., p.1), false, false),
        ((870., 260.), false, false),
        ((870., 260.), false, true),
    ] {
        input::update(&rt.lua, &mut mouse, Some(at), down, up, false, 0.);
    }
    lua::tick(&rt.lua, 16.);
    lua::flush(&rt.lua);
    let target = super::super::live_window(&rt.lua, 1).ok_or("original duplicated chat missing")?;
    assert_eq!(lua::ui(&rt.lua).tree.get(row.button).parent, Some(target));
    assert_eq!(lua::ui(&rt.lua).tree.get(row.message).parent, Some(target));
    assert_eq!(
        lua::ui(&rt.lua)
            .tree
            .widget(row.message)
            .unwrap()
            .message
            .as_ref()
            .unwrap()
            .lines,
        old_lines
    );
    assert_eq!(state.borrow().chat_config.model.current_window(3), Some(1));
    assert_eq!(state.borrow().chat_config.model.windows[&0], [0, 1, 2]);
    assert_eq!(
        state.borrow().chat_config.model.tabs[&3].filters,
        old.filters
    );
    assert_eq!(state.borrow().chat_config.model.tabs[&3].colors, old.colors);
    rt.lua.load("AAUI.ChatMethod(3,4)").exec()?;
    assert_eq!(state.borrow().chat_config.model.extra[&1].method, 4);
    assert_eq!(state.borrow().chat_config.model.extra[&0].method, 0);
    let saved = crate::settings::chat_read(&rt.lua, "17:12345", "tabs").unwrap();
    let extra = crate::settings::chat_read(&rt.lua, "17:12345", "extra").unwrap();
    let restarted = Model::load(&saved, Some(&extra)).unwrap();
    assert_eq!(restarted.current_window(3), Some(1));
    assert_eq!(restarted.extra[&1].method, 4);
    assert_eq!(restarted.tabs[&3].filters, old.filters);
    assert!(state.borrow().chat_config.transfers.is_empty());
    let (next, restored_state) = super::super::tests::restart(&state, &path)?;
    let restored_window =
        super::super::live_window(&next.lua, 1).ok_or("saved duplicate did not reopen")?;
    let restored_chat = aa_ui::lua::ui(&next.lua)
        .tree
        .widget(restored_window)
        .unwrap()
        .chat
        .clone()
        .unwrap();
    assert_eq!(
        restored_chat
            .tabs
            .iter()
            .map(|row| row.id)
            .collect::<Vec<_>>(),
        [3]
    );
    assert_eq!(restored_chat.method, 4);
    assert_eq!(
        restored_state.borrow().chat_config.model.tabs[&3].filters,
        old.filters
    );
    assert_eq!(
        restored_state.borrow().chat_config.model.tabs[&3].colors,
        old.colors
    );
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}
