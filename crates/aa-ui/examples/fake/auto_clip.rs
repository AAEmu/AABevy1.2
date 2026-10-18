//! Original chat-tab constructor and hover tooltip, with long received tab names.

use aa_ui::{UiRuntime, lua};

pub fn install(ui: &mut UiRuntime) {
    super::fake::install(ui, "en_us", &super::fake::default_worlds());
    super::fake::install_ingame(ui);
    ui.exec("@native_clip_tabs", r#"
        nativeClipNames={'A very long community channel','Expéditions et commerce','Friends and neighbours'}
        AAUI.ChatTabs=function()
            local tabs={}
            for i,name in ipairs(nativeClipNames) do tabs[i]={id=i,name=name} end
            return tabs
        end
        X2Chat.GetChatTabInfoTable=function(self,id) return {name=nativeClipNames[id]} end
    "#).unwrap();
    ui.run_stage(aa_ui::Stage::World);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    let (window, tabs) = {
        let u = lua::ui(&ui.lua);
        let window = u.tree.of_class("chatwindow")[0];
        let tabs = u
            .tree
            .widget(window)
            .unwrap()
            .chat
            .as_ref()
            .unwrap()
            .tabs
            .clone();
        (window, tabs)
    };
    ui.lua
        .globals()
        .set(
            "nativeClipChat",
            lua::object_table(&ui.lua, window).unwrap().unwrap(),
        )
        .unwrap();
    let list = ui.lua.create_table().unwrap();
    for (i, tab) in tabs.iter().enumerate() {
        list.raw_set(
            i + 1,
            lua::object_table(&ui.lua, tab.button).unwrap().unwrap(),
        )
        .unwrap();
    }
    ui.lua.globals().set("nativeClipTabs", list).unwrap();
    ui.exec(
        "@native_clip_position",
        r#"
        nativeClipChat:RemoveAllAnchors()
        nativeClipChat:AddAnchor('CENTER',UIParent,0,0)
        nativeClipChat:SetExtent(430,180)
        nativeClipChat:SetTabWidth(100)
        for i,tab in ipairs(nativeClipTabs) do
            tab:SetText(nativeClipNames[i])
            assert(tab:GetText()==nativeClipNames[i])
        end
        nativeClipTabs[1]:OnEnter()
        HideTooltip()
    "#,
    )
    .unwrap();
    lua::tick(&ui.lua, 0.0);
    let r = lua::ui(&ui.lua).tree.rect(tabs[0].button);
    aa_ui::input::update_buttons(
        &ui.lua,
        &mut Default::default(),
        Some((r.x + r.w * 0.5, r.y + r.h * 0.5)),
        Default::default(),
        0.0,
    );
}
