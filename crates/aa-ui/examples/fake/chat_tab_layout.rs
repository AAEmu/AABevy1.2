//! Unchanged original chat constructors, with fixed and auto tab layouts.

use aa_ui::{UiRuntime, lua};

pub fn install(ui: &mut UiRuntime) {
    super::auto_clip::install(ui);
    ui.exec(
        "@native_chat_layout",
        r#"
        nativeClipChat:RemoveAllAnchors()
        nativeClipChat:AddAnchor('TOPLEFT',UIParent,580,350)
        nativeClipChat:SetExtent(345,150)
        nativeClipChat:SetTabWidth(140)
        fixedChat=UIParent:CreateWidget('chatwindow','nativeFixedChat','UIParent')
        fixedChat:SetChatWindowId(1)
        fixedChat:RemoveAllAnchors()
        fixedChat:AddAnchor('TOPLEFT',UIParent,580,580)
        fixedChat:SetExtent(430,150)
        fixedChat:SetTabWidth(140)
        fixedChat:UseAutoResizingTabButtonMode(false)
        fixedChat:UseAddTabButton(true)
        fixedChat:Show(true)
        for i,name in ipairs({'Auto tabs: 345 px window / 104 px tabs','Fixed tabs: 430 px window / 140 px tabs'}) do
            local label=UIParent:CreateWidget('label','nativeLayoutCaption'..i,'UIParent')
            label:SetExtent(430,25);label:AddAnchor('TOPLEFT',UIParent,580,305+(i-1)*230)
            label.style:SetAlign(ALIGN_LEFT);label:SetText(name);label:Show(true)
        end
        HideTooltip()
    "#,
    )
    .unwrap();
    lua::tick(&ui.lua, 0.0);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
}
