//! Original chat options load, physical OK, and strict named color round-trip.
#[path = "fake/mod.rs"]
#[allow(dead_code)]
mod fake;
#[path = "native_chat_alpha.rs"]
#[allow(dead_code)]
mod original;
use aa_ui::{
    UiRuntime,
    input::{self, MouseButtons, MouseState},
    lua,
};
use bevy::prelude::*;

pub fn install(ui: &mut UiRuntime) {
    original::install(ui);
    ui.exec("@native_chat_options",r#"
        local model={id=0,name='All',fontSize=15,bgColor={r=.08,g=.15,b=.2,a=1},filters={},filterColors={}}
        for i=2,59 do model.filters[i]=true;model.filterColors[i]={r=1,g=1,b=1,a=1} end
        local function copy(v)
            if type(v)~='table' then return v end
            local t={};for k,x in pairs(v) do t[k]=copy(x) end;return t
        end
        X2Chat.GetChatTabInfoTable=function() return copy(model) end
        X2Chat.GetChatOption=function() return {overAlpha=.7,leaveAlpha=.5} end
        X2Chat.UpdateChatOption=function(self,info) savedChatAlpha=info;return true end
        X2Chat.UpdateChatTabInfo=function(self,id,info)
            assert(id==0 and type(info.fontSize)=='number')
            for _,key in ipairs({'r','g','b','a'}) do
                assert(type(info.bgColor[key])=='number')
                for _,color in pairs(info.filterColors) do assert(type(color[key])=='number') end
            end
            savedChatInfo=copy(info);model=copy(info);return true
        end
        ShowChatOptionWindow(0)
        assert(chatOptionWindow.chatTabInfos.fontSize==15)
        chatOptionWindow.etcPage.fontSizeScroll:SetValue(16,false)
        chatOptionWindow.checkBoxs.boxMapUseExtraValue[2].colorPick.colorBG:SetColor(.25,.5,.75,.8)
        chatOptionWindow.etcPage.bgColorButton.colorBG:SetColor(.1,.2,.3,.4)
    "#).unwrap();
    lua::tick(&ui.lua, 600.);
}

pub fn save(ui: &UiRuntime) {
    let button = lua::value_id(
        &ui.lua,
        &ui.lua.load("chatOptionWindow.okButton").eval().unwrap(),
    )
    .unwrap();
    let p = {
        let u = lua::ui(&ui.lua);
        let r = u.animations.widget_rect(&u.tree, button);
        (r.x + r.w / 2., r.y + r.h / 2.)
    };
    assert_eq!(input::pick(&ui.lua, p.0, p.1), Some(button));
    let mut mouse = MouseState::default();
    for buttons in [
        MouseButtons {
            press: true,
            ..Default::default()
        },
        MouseButtons {
            release: true,
            ..Default::default()
        },
    ] {
        input::update_buttons(&ui.lua, &mut mouse, Some(p), buttons, 0.);
        lua::tick(&ui.lua, 16.);
    }
    ui.exec(
        "@saved_native_chat_options",
        r#"
        assert(chatOptionWindow==nil and savedChatInfo.fontSize==16)
        assert(savedChatInfo.filterColors[2].a and savedChatInfo.bgColor.r)
    "#,
    )
    .unwrap();
}

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: original chat options"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(fake::config()),
        })
        .insert_resource(aa_ui::UiSetup::new(|ui| {
            install(ui);
            if std::env::args().any(|arg| arg == "--saved") {
                save(ui);
                ui.exec("@reopen_saved_chat_options", "ShowChatOptionWindow(0)")
                    .unwrap();
                lua::tick(&ui.lua, 600.);
            }
            fake::report(ui, "native_chat_options");
        }))
        .add_systems(Startup, fake::physical_pixels)
        .run();
}
