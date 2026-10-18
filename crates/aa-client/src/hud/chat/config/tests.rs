use super::*;
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
    let bytes = std::fs::read(&path)?;
    let saved: serde_json::Value = serde_json::from_slice(&bytes)?;
    assert!(saved["chat"]["17:-2147483647"]["tabs"].is_array());
    assert_eq!(saved["chat"]["17:-2147483647"]["extra"][0]["selected"], 3);
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
