//! Original chat restored through its native raw-tab method persistence point.
#[path = "fake/mod.rs"]
#[allow(dead_code)]
mod fake;
use aa_ui::{UiRuntime, lua};
use bevy::prelude::*;

pub fn install(ui: &mut UiRuntime) -> u32 {
    fake::install(ui, "en_us", &fake::default_worlds());
    fake::install_ingame(ui);
    ui.exec(
        "@method_configuration",
        r#"
        savedMethod=nil;savedRaw=nil
        AAUI.ChatTabs=function() return {{id=0,name='All'},selected=0,method=4} end
        AAUI.ChatMethod=function(raw,method)
            if raw==0 then savedRaw=raw;savedMethod=method end
        end
        X2Chat.GetChatTabInfoTable=function() return {id=0,name='All'} end
    "#,
    )
    .unwrap();
    ui.run_stage(aa_ui::Stage::World);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    let window = lua::ui(&ui.lua).tree.of_class("chatwindow")[0];
    ui.lua
        .globals()
        .set(
            "methodChat",
            lua::object_table(&ui.lua, window).unwrap().unwrap(),
        )
        .unwrap();
    ui.exec(
        "@method_input",
        r#"
        assert(savedRaw==0 and savedMethod==4)
        methodChat:RemoveAllAnchors();methodChat:AddAnchor('CENTER',UIParent,0,0)
        methodChat:SetExtent(460,220)
        methodEdit=methodChat:GetChatEdit();methodEdit:Show(true);methodEdit:SetFocus()
    "#,
    )
    .unwrap();
    ui.type_text("Party method restored");
    lua::tick(&ui.lua, 600.);
    window
}

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: native chat method"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(fake::config()),
        })
        .insert_resource(aa_ui::UiSetup::new(|ui| {
            install(ui);
            fake::report(ui, "native_chat_method");
        }))
        .add_systems(Startup, fake::physical_pixels)
        .run();
}
