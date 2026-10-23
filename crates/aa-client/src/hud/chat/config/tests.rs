use super::*;

pub(super) fn restart(state: &Shared, path: &std::path::Path) -> Result<(UiRuntime, Shared), Box<dyn std::error::Error>> {
    let mut next = UiRuntime::new("en_us", 1280., 720.)?;
    next.load_ui_texts(std::path::Path::new(aa_anim::db::DEFAULT_DB))?;
    let other = std::rc::Rc::new(std::cell::RefCell::new(super::super::super::api::HudData::default()));
    {
        let source = state.borrow();
        let mut target = other.borrow_mut();
        target.game = source.game.clone();
        target.snapshot = source.snapshot.clone();
        target.feature_limits = source.feature_limits;
        target.panels = source.panels.clone();
        target.chat_config.identity(source.chat_config.world, source.chat_config.owner);
    }
    super::super::super::api::install(&mut next, &other)?;
    crate::settings::reopen_fixture(&next, path.to_path_buf())?;
    next.run_stage(aa_ui::Stage::World);
    locks::restore(&next.lua, &other);
    assert!(next.errors.is_empty(), "{:?}", next.errors);
    Ok((next, other))
}

#[test]
fn default_tabs_restart_restores_independently_saved_selection_and_input_method() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state, path) = super::super::super::breath::fixture::runtime(1280., 720.)?;
    state.borrow_mut().chat_config.identity(Some(17), 12345);
    ensure(&rt.lua, &state)?;
    let window = object_table(&rt.lua, live_window(&rt.lua, 0).unwrap())?.unwrap();
    window.call_method::<()>("SelectTab", 3)?;
    let edit: Table = window.call_method("GetChatEdit", ())?;
    edit.call_method::<()>("Show", true)?;
    edit.call_method::<()>("SetFocus", ())?;
    rt.type_text("/p ");
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&path)?)?;
    assert!(saved["chat"]["17:12345"].get("tabs").is_none());
    assert_eq!(saved["chat"]["17:12345"]["extra"][0], json!({"id":0,"selected":2,"method":4}));
    let (next, _) = restart(&state, &path)?;
    let window = live_window(&next.lua, 0).unwrap();
    let u = ui(&next.lua);
    let chat = u.tree.widget(window).unwrap().chat.as_ref().unwrap();
    assert_eq!(chat.tabs.iter().map(|tab| tab.id).collect::<Vec<_>>(), [0, 1, 2]);
    assert_eq!((chat.selected, chat.method), (2, 4));
    drop(u);
    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
fn original_chat_open_consumes_pending_trial_before_prefill_and_clear_does_not_switch_the_method() -> Result<(), Box<dyn std::error::Error>>
{
    let (rt, state, path) = fixture::runtime(1280., 720.)?;
    rt.lua.load("HUD.chat_close()").exec()?;
    aa_ui::lua::chat_input::set_forced_method(&rt.lua, 11);
    let window = live_window(&rt.lua, 0).unwrap();
    assert_ne!(ui(&rt.lua).tree.widget(window).unwrap().chat.as_ref().unwrap().method, 11);
    rt.lua
        .load(
            r#"
        HUD.chat_open('fresh draft')
        local edit=HUD.chat_window():GetChatEdit()
        assert(edit:IsVisible() and edit:GetText()=='fresh draft')
    "#,
        )
        .exec()?;
    assert_eq!(ui(&rt.lua).tree.widget(window).unwrap().chat.as_ref().unwrap().method, 11);
    assert_eq!(state.borrow().chat_config.model.extra[&0].method, 11);
    aa_ui::lua::chat_input::set_forced_method(&rt.lua, -6);
    rt.lua.load("HUD.chat_close();HUD.chat_open('second draft')").exec()?;
    assert_eq!(ui(&rt.lua).tree.widget(window).unwrap().chat.as_ref().unwrap().method, 11);
    assert!(state.borrow().requests.is_empty(), "opening/prefilling must not send a chat line");
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
fn original_full_reset_retires_history_deletes_only_four_owner_keys_and_recreates_defaults() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state, path) = fixture::runtime(1280., 720.)?;
    let old = live_window(&rt.lua, 0).unwrap();
    aa_ui::lua::chat_input::set_forced_method(&rt.lua, 11);
    rt.lua.globals().set(
        "RESET_STORE_CHECK",
        rt.lua.create_function(|lua, ()| {
            for key in ["tabs", "extra", "locks", "options"] {
                assert!(
                    crate::settings::chat_read(lua, "17:12345", key).is_none(),
                    "{key} must be deleted before callback"
                );
            }
            Ok(())
        })?,
    )?;
    rt.lua
        .load(
            r#"
        assert(X2Chat:LockChatWindowByTabId(3,true))
        X2Chat:SetOptionDefaultFontSize(18)
        X2Chat:UpdateChatOption({overAlpha=.8,leaveAlpha=.2})
        NET.option_set('reset_pending_probe',99)
        local recreate=OnResetAllChatWindow
        RESET_CALLS=0
        OnResetAllChatWindow=function(...)
            assert(select('#',...)==0)
            RESET_STORE_CHECK()
            RESET_CALLS=RESET_CALLS+1
            assert(#X2Chat:AllChatWindowIds()==1)
            assert(X2Chat:GetChatTabInfoTable(1).name=='Party')
            assert(X2Chat:GetChatTabInfoTable(1).fontSize==18)
            assert(not X2Chat:IsLockedChatWindowByChatTabId(0))
            assert(select('#',X2Chat:GetChatTabInfoTable(3))==0)
            return recreate(...)
        end
    "#,
        )
        .exec()?;
    assert!(crate::settings::chat_save(&rt.lua, "17:12345", "future", json!({"keep":true})));
    assert!(crate::settings::chat_save(&rt.lua, "18:12345", "tabs", json!(["other world"])));
    assert!(crate::settings::chat_save(&rt.lua, "17:54321", "extra", json!(["other character"])));
    fixture::show_options_for(&rt, 0)?;
    super::super::super::expansion::fixture::click(&rt, "CHAT_OPTIONS_FIXTURE.chatResetButton")?;
    aa_ui::lua::tick(&rt.lua, 600.);
    assert!(!ui(&rt.lua).tree.get(old).alive);
    rt.lua
        .load(
            r#"
        assert(RESET_CALLS==1)
        assert(NET.option_get('reset_pending_probe')==99)
        assert(not CHAT_OPTIONS_FIXTURE:IsVisible())
        local o=X2Chat:GetChatOption()
        assert(math.abs(o.overAlpha-.7)<.00001 and math.abs(o.leaveAlpha-.5)<.00001)
    "#,
        )
        .exec()?;
    let new = live_window(&rt.lua, 0).unwrap();
    let u = ui(&rt.lua);
    let chat = u.tree.widget(new).unwrap().chat.as_ref().unwrap();
    assert_eq!(chat.tabs.iter().map(|tab| tab.id).collect::<Vec<_>>(), [0, 1, 2]);
    assert_eq!((chat.selected, chat.method), (0, 0));
    for tab in &chat.tabs {
        assert!(u.tree.widget(tab.message).unwrap().message.as_ref().unwrap().lines.is_empty());
    }
    drop(u);
    rt.lua.load("HUD.chat_open('fresh draft')").exec()?;
    assert_eq!(
        ui(&rt.lua).tree.widget(new).unwrap().chat.as_ref().unwrap().method,
        0,
        "manager reset must clear pending Trial11 before input opens"
    );
    rt.lua.load("HUD.chat_close()").exec()?;
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&path)?)?;
    // Native hydration restores method0 and saves extra even when it did not change.
    assert_eq!(
        saved["chat"]["17:12345"],
        json!({"future":{"keep":true},"extra":[{"id":0,"selected":0,"method":0}]})
    );
    assert_eq!(saved["chat"]["18:12345"]["tabs"], json!(["other world"]));
    assert_eq!(saved["chat"]["17:54321"]["extra"], json!(["other character"]));
    assert!(saved["options"].get("s:reset_pending_probe").is_none());
    assert_eq!(
        (
            state.borrow().chat_config.world,
            state.borrow().chat_config.owner,
            state.borrow().chat_config.font
        ),
        (Some(17), 12345, 18)
    );
    let (next, _) = restart(&state, &path)?;
    next.lua
        .load("assert(X2Chat:GetChatTabInfoTable(1).name=='Party' and not X2Chat:IsLockedChatWindowByChatTabId(0))")
        .exec()?;
    // Native ignores callback return values and a failed/missing script callback does not undo retirement.
    rt.lua
        .load(
            r#"
        assert(select('#',X2Chat:InitChatWindow('ignored'))==0)
        assert(RESET_CALLS==2)
        OnResetAllChatWindow=function() error('reset callback probe') end
        assert(select('#',X2Chat:InitChatWindow())==0)
        OnResetAllChatWindow=nil
        assert(select('#',X2Chat:InitChatWindow())==0)
    "#,
        )
        .exec()?;
    assert!(ui(&rt.lua).tree.of_class("chatwindow").is_empty());
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
fn original_context_lock_unlock_persists_and_drag_feedback_does_not_move_the_window() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state, path) = fixture::runtime(1280., 720.)?;
    let window = live_window(&rt.lua, 0).unwrap();
    let before = ui(&rt.lua).tree.rect(window);
    let selected = state.borrow().chat_config.model.extra[&0].selected;
    fixture::context_action(&rt)?;
    rt.lua.load("assert(X2Chat:IsLockedChatWindowByChatTabId(0))").exec()?;
    assert_eq!(state.borrow().chat_config.model.extra[&0].selected, selected);
    fixture::drag_attempt(&rt);
    let notify = {
        let u = ui(&rt.lua);
        let widget = u.tree.widget(window).unwrap();
        let chat = widget.chat.as_ref().unwrap();
        assert!(!chat.allow_tab_switch && !widget.resizing.enabled);
        assert_eq!(chat.notify_remaining_ms, 3000.);
        assert_eq!(u.tree.rect(window), before);
        chat.lock_notify
    };
    aa_ui::lua::tick(&rt.lua, 400.);
    assert!(ui(&rt.lua).tree.visible(notify));
    fixture::context_action(&rt)?;
    rt.lua.load("assert(not X2Chat:IsLockedChatWindowByChatTabId(0))").exec()?;
    {
        let u = ui(&rt.lua);
        let widget = u.tree.widget(window).unwrap();
        assert!(widget.chat.as_ref().unwrap().allow_tab_switch && widget.resizing.enabled);
        assert_eq!(widget.chat.as_ref().unwrap().notify_remaining_ms, 0.);
        assert!(!u.tree.visible(notify));
    }
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&path)?)?;
    assert_eq!(saved["chat"]["17:12345"]["locks"][0], false);
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}
#[test]
fn original_window_locks_preserve_native_arity_owner_and_missing_live_save_edge() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state, path) = fixture::runtime(1280., 720.)?;
    let window = live_window(&rt.lua, 0).unwrap();
    rt.lua
        .load(
            r#"
        assert(select('#',X2Chat:IsLockedChatWindowByChatTabId())==0)
        assert(select('#',X2Chat:IsLockedChatWindowByChatTabId('3'))==0)
        assert(not X2Chat:IsLockedChatWindowByChatTabId(99))
        assert(select('#',X2Chat:LockChatWindowByTabId(3,'true'))==0)
        assert(select('#',X2Chat:LockChatWindowByTabId('3',true))==0)
        assert(X2Chat:LockChatWindowByTabId(3,1))
        assert(X2Chat:IsLockedChatWindowByChatTabId(0) and X2Chat:IsLockedChatWindowByChatTabId(3))
    "#,
        )
        .exec()?;
    {
        let u = ui(&rt.lua);
        let widget = u.tree.widget(window).unwrap();
        assert!(!widget.chat.as_ref().unwrap().allow_tab_switch && !widget.resizing.enabled);
    }
    let before = std::fs::read(&path)?;
    let saved: serde_json::Value = serde_json::from_slice(&before)?;
    assert_eq!(
        saved["chat"]["17:12345"]["locks"],
        json!([true, false, false, false, false, false, false, false])
    );
    let raw = state.borrow_mut().chat_config.model.allocate(7, "Outpost", 15).unwrap();
    rt.lua.globals().set("ABSENT_LOCK_TAB", raw)?;
    rt.lua
        .load(
            "assert(not X2Chat:LockChatWindowByTabId(ABSENT_LOCK_TAB,true));assert(X2Chat:IsLockedChatWindowByChatTabId(ABSENT_LOCK_TAB))",
        )
        .exec()?;
    assert_eq!(
        std::fs::read(&path)?,
        before,
        "missing live owner changes memory before returning false, but must not save"
    );
    rt.lua
        .load("assert(X2Chat:LockChatWindowByTabId(0,0));assert(not X2Chat:IsLockedChatWindowByChatTabId(3))")
        .exec()?;
    {
        let u = ui(&rt.lua);
        let widget = u.tree.widget(window).unwrap();
        assert!(widget.chat.as_ref().unwrap().allow_tab_switch && widget.resizing.enabled);
    }
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&path)?)?;
    assert_eq!(
        saved["chat"]["17:12345"]["locks"][7], true,
        "next valid save includes all eight in-memory bytes"
    );
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}
#[test]
fn native_partial_table_update_retains_hidden_filters_and_palette() -> mlua::Result<()> {
    let lua = Lua::new();
    let mut old = model::Tab::new(1, 0, "Party".into(), 15).defaults(15);
    old.filters[199] = true;
    old.colors[199] = [0.1, 0.2, 0.3, 0.4];
    let table = table::info(&lua, &old, false)?;
    table.raw_set("name", "X")?;
    table.raw_set("fontSize", 17.9)?;
    let bg: Table = table.raw_get("bgColor")?;
    bg.raw_set("r", -2.0)?;
    bg.raw_set("g", "wrongkind")?;
    let filters: Table = table.raw_get("filters")?;
    filters.raw_set(5, 1)?;
    filters.raw_set(199, false)?;
    let updated = table::update(table.clone(), old.clone())?.unwrap();
    assert_eq!(updated.name, "X");
    assert_eq!(updated.font, 17);
    assert_eq!(updated.background, [-2., 0., 0., 1.]);
    assert_eq!(updated.filters[5], old.filters[5]);
    assert!(updated.filters[199]);
    assert_eq!(updated.colors[199], old.colors[199]);
    let colors: Table = table.raw_get("filterColors")?;
    colors.raw_set(2, lua.create_table()?)?;
    assert!(table::update(table, old)?.is_none());
    Ok(())
}

#[test]
fn original_chat_controls_and_character_scoped_restart_keep_native_rows() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state, path) = super::super::super::breath::fixture::runtime(1280., 720.)?;
    {
        let mut data = state.borrow_mut();
        data.chat_config.identity(Some(17), 0x8000_0001);
    }
    // Rehydrate the existing original window against this explicit test identity.
    let window = live_window(&rt.lua, 0).expect("original main window");
    ensure(&rt.lua, &state)?;
    rt.lua
        .load(
            r#"
        local ids=X2Chat:AllChatWindowIds()
        assert(#ids==1 and ids[1]==0)
        local tab=X2Chat:GetChatTabInfoTable(0)
        assert(tab.id==0 and tab.name=='All' and tab.fontSize==15 and tab.windowId==nil)
        assert(tab.filters[2] and tab.filters[59] and tab.filters[60]==nil)
        assert(select('#',X2Chat:GetChatTabInfoTable(99))==0)
        assert(not X2Chat:AddNewChatTabByUser(0,'X'))
        assert(not X2Chat:AddNewChatTabByUser(7,'NoWindow'))
        assert(X2Chat:AddNewChatTabByUser(0,'Harbor'))
        local tab=X2Chat:GetChatTabInfoTable(3)
        for id=2,59 do assert(tab.filters[id]==false) end
        assert(X2Chat:RenameChatTabByUser(3,'X'))
        tab=X2Chat:GetChatTabInfoTable(3)
        tab.fontSize=19;tab.filters[2]=true;tab.filterColors[2]={r=.25,g=.5,b=.75,a=.8}
        assert(X2Chat:UpdateChatTabInfo(3,tab))
        local fresh=X2Chat:GetDefaultChatTabInfoTable(3)
        assert(fresh.name=='X' and fresh.fontSize==15 and not fresh.filters[2])
    "#,
        )
        .exec()?;
    {
        let u = ui(&rt.lua);
        let chat = u.tree.widget(window).unwrap().chat.as_ref().unwrap();
        assert_eq!(chat.tabs.iter().map(|t| t.id).collect::<Vec<_>>(), [0, 1, 2, 3]);
        assert_eq!(chat.selected, 3);
    }
    let select = object_table(&rt.lua, window)?.unwrap();
    select.call_method::<()>("SelectTab", 2)?;
    assert_eq!(state.borrow().chat_config.model.extra[&0].selected, 1);
    // The save hook runs before original selection callbacks and stores numeric index, not rawID.
    rt.lua
        .load("assert(X2Chat:DeleteChatTabByUser(1)); assert(X2Chat:AddNewChatTabByUser(0,'Crew'))")
        .exec()?;
    assert_eq!(state.borrow().chat_config.model.windows[&0], [0, 2, 3, 1]);
    assert_eq!(state.borrow().chat_config.model.extra[&0].selected, 3);
    let edit: Table = select.call_method("GetChatEdit", ())?;
    edit.call_method::<()>("Show", true)?;
    edit.call_method::<()>("SetFocus", ())?;
    rt.type_text("/p ");
    assert_eq!(state.borrow().chat_config.model.extra[&0], model::Extra { selected: 3, method: 4 });
    rt.lua.load("assert(X2Chat:LockChatWindowByTabId(1,true))").exec()?;
    let bytes = std::fs::read(&path)?;
    let saved: serde_json::Value = serde_json::from_slice(&bytes)?;
    assert!(saved["chat"]["17:-2147483647"]["tabs"].is_array());
    assert_eq!(saved["chat"]["17:-2147483647"]["extra"][0]["selected"], 3);
    assert_eq!(saved["chat"]["17:-2147483647"]["extra"][0]["method"], 4);
    assert_eq!(saved["chat"]["17:-2147483647"]["locks"][0], true);
    // The input can retain a removed raw ID; an unknown owner must not save any extra row.
    rt.lua.load("AAUI.ChatMethod(-1,7);AAUI.ChatMethod(99,7)").exec()?;
    assert_eq!(std::fs::read(&path)?, bytes);
    // Real fresh Lua/store, independent of the in-memory writer.
    let mut next = UiRuntime::new("en_us", 1280., 720.)?;
    next.load_ui_texts(std::path::Path::new(aa_anim::db::DEFAULT_DB))?;
    let other = std::rc::Rc::new(std::cell::RefCell::new(super::super::super::api::HudData::default()));
    {
        let source = state.borrow();
        let mut target = other.borrow_mut();
        target.game = source.game.clone();
        target.snapshot = source.snapshot.clone();
        target.feature_limits = source.feature_limits;
        target.panels = source.panels.clone();
    }
    other.borrow_mut().chat_config.identity(Some(17), 0x8000_0001);
    super::super::super::api::install(&mut next, &other)?;
    crate::settings::reopen_fixture(&next, path.clone())?;
    next.run_stage(aa_ui::Stage::World);
    locks::restore(&next.lua, &other);
    assert!(next.errors.is_empty(), "{:?}", next.errors);
    next.lua
        .load(
            r#"
        local tab=X2Chat:GetChatTabInfoTable(3)
        assert(tab.name=='X' and tab.fontSize==19 and tab.filters[2])
        assert(tab.filterColors[2].r==.25 and tab.filterColors[2].g==.5)
        assert(X2Chat:GetChatTabInfoTable(1).name=='Crew')
    "#,
        )
        .exec()?;
    let window = live_window(&next.lua, 0).unwrap();
    {
        let u = ui(&next.lua);
        let chat = u.tree.widget(window).unwrap().chat.as_ref().unwrap();
        assert_eq!(chat.tabs.iter().map(|t| t.id).collect::<Vec<_>>(), [0, 1, 2, 3]);
        assert_eq!(chat.selected, 3);
        assert_eq!(chat.method, 4);
        assert!(!chat.allow_tab_switch);
        assert!(!u.tree.widget(window).unwrap().resizing.enabled);
    }
    other.borrow_mut().chat_config.identity(Some(18), 0x8000_0001);
    next.lua
        .load("assert(X2Chat:GetChatTabInfoTable(1).name=='Party'); assert(select('#',X2Chat:GetChatTabInfoTable(3))==0)")
        .exec()?;
    // User ID0 deletion removes its widget first, then fails its protected model guard.
    assert!(!next.lua.load("return X2Chat:DeleteChatTabByUser(0)").eval::<bool>()?);
    assert!(other.borrow().chat_config.model.tabs.contains_key(&0));
    assert!(
        !ui(&next.lua)
            .tree
            .widget(window)
            .unwrap()
            .chat
            .as_ref()
            .unwrap()
            .tabs
            .iter()
            .any(|t| t.id == 0)
    );
    drop(next);
    drop(rt);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[test]
fn raw_f32_metadata_save_preserves_nan_payload_and_negative_zero() {
    let mut m = model::Model::defaults(["All".into(), "Party".into(), "Trade".into()], 15);
    let tab = m.tabs.get_mut(&0).unwrap();
    tab.colors[199] = [f32::from_bits(0x7fc01234), -0., f32::INFINITY, -5.];
    let restored = model::Model::load(&m.tabs_json(), Some(&m.extra_json())).unwrap();
    assert_eq!(
        restored.tabs[&0].colors[199].map(f32::to_bits),
        m.tabs[&0].colors[199].map(f32::to_bits)
    );
}

#[test]
fn original_chat_options_save_cancel_and_default_controls_edit_real_configuration() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state, path) = fixture::runtime(1920., 1080.)?;
    fixture::show_options(&rt)?;
    rt.lua
        .load(
            r#"
        assert(CHAT_OPTIONS_FIXTURE.chatTabId==3)
        CHAT_OPTIONS_FIXTURE.menuListWindow.menu[4]:OnClick('LeftButton')
        CHAT_OPTIONS_FIXTURE.etcPage.fontSizeScroll:SetValue(16,false)
        CHAT_OPTIONS_FIXTURE.etcPage.defaultAlphaScroll:SetValue(35,false)
        CHAT_OPTIONS_FIXTURE.etcPage.overlAlphaScroll:SetValue(85,false)
        assert(CHAT_OPTIONS_FIXTURE.chatTabInfos~=nil, 'original OnShow must load metadata')
        assert(CHAT_OPTIONS_FIXTURE.etcPage.fontSizeScroll:GetValue()==16, 'font slider value')
        local update=X2Chat.UpdateChatTabInfo
        X2Chat.UpdateChatTabInfo=function(self,id,info)
            SAVED_CHAT_INPUT=info;SAVED_CHAT_RESULT=update(self,id,info);return SAVED_CHAT_RESULT
        end
    "#,
        )
        .exec()?;
    super::super::super::expansion::fixture::click(&rt, "CHAT_OPTIONS_FIXTURE.okButton")?;
    rt.lua
        .load(
            r#"
        assert(SAVED_CHAT_INPUT~=nil,'original SaveOption must call UpdateChatTabInfo')
        assert(SAVED_CHAT_RESULT, 'metadata update refused: '..tostring(SAVED_CHAT_INPUT.fontSize)..'/'..tostring(SAVED_CHAT_INPUT.filterColors[2].a))
        local tab=X2Chat:GetChatTabInfoTable(3)
        assert(tab.fontSize==16 and tab.filters[2], string.format("savedfont=%s filter2=%s visible=%s enabled=%s",tostring(tab.fontSize),tostring(tab.filters[2]),tostring(CHAT_OPTIONS_FIXTURE:IsVisible()),tostring(CHAT_OPTIONS_FIXTURE.okButton:IsEnabled())))
        local option=X2Chat:GetChatOption()
        assert(math.abs(option.leaveAlpha-.35)<.00001 and math.abs(option.overAlpha-.85)<.00001)
        assert(select('#',X2Chat:UpdateChatOption({overAlpha='bad'}))==0)
        assert(math.abs(X2Chat:GetChatOption().overAlpha-.85)<.00001)
    "#,
        )
        .exec()?;
    fixture::show_options(&rt)?;
    rt.lua
        .load("CHAT_OPTIONS_FIXTURE.etcPage.fontSizeScroll:SetValue(14,false)")
        .exec()?;
    super::super::super::expansion::fixture::click(&rt, "CHAT_OPTIONS_FIXTURE.cancelButton")?;
    assert_eq!(state.borrow().chat_config.model.tabs[&3].font, 16);
    fixture::show_options(&rt)?;
    super::super::super::expansion::fixture::click(&rt, "CHAT_OPTIONS_FIXTURE.resetButton")?;
    super::super::super::expansion::fixture::click(&rt, "CHAT_OPTIONS_FIXTURE.okButton")?;
    assert_eq!(state.borrow().chat_config.model.tabs[&3].font, 15);
    assert!(!state.borrow().chat_config.model.tabs[&3].filters.iter().any(|&on| on));
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    drop(rt);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[test]
fn six_characters_with_eight_palettes_each_fit_the_existing_account_file_limit() {
    let mut m = model::Model::defaults(["All".into(), "Party".into(), "Trade".into()], 15);
    for _ in 3..8 {
        m.allocate(0, "Crew", 15).unwrap();
    }
    let mut profiles = serde_json::Map::new();
    for character in 1..=6 {
        profiles.insert(format!("17:{character}"), json!({"tabs":m.tabs_json(),"extra":m.extra_json()}));
    }
    let file = serde_json::to_vec_pretty(&json!({"chat":profiles})).unwrap();
    assert!(
        file.len() < 512 * 1024,
        "native palettes must leave room for other account settings: {}",
        file.len()
    );
    assert!(model::Model::load(&m.tabs_json(), Some(&m.extra_json())).is_some());
}
