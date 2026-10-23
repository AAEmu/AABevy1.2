//! Original World/chat bytecode, with a secondary tab dragged to an independent window.
#[path = "fake/mod.rs"]
#[allow(dead_code)]
pub(crate) mod fake;

use aa_ui::{UiRuntime, input, lua, tree::Id};
use bevy::prelude::*;

pub fn install(ui: &mut UiRuntime) -> Id {
    fake::install(ui, "en_us", &fake::default_worlds());
    fake::install_ingame(ui);
    ui.exec(
        "@split_config",
        r#"
        AAUI.ChatTabs=function() return {
            {id=0,name='All'}, {id=2,name='Harbor',filters={2}}
        } end
        X2Chat.GetChatTabInfoTable=function(self,id)
            return {name=id==0 and 'All' or 'Harbor'}
        end
        splitBounds={}
        X2.GetCharacterUiData=function(self,key) return splitBounds[key] end
        X2.SetCharacterUiData=function(self,key,value) splitBounds[key]=value end
    "#,
    )
    .unwrap();
    ui.run_stage(aa_ui::Stage::World);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    let window = lua::ui(&ui.lua).tree.of_class("chatwindow")[0];
    ui.lua
        .globals()
        .set(
            "splitSource",
            lua::object_table(&ui.lua, window).unwrap().unwrap(),
        )
        .unwrap();
    ui.exec("@split_initial", r#"
        splitSource:RemoveAllAnchors();splitSource:AddAnchor('TOPLEFT',UIParent,80,220)
        splitSource:SetExtent(460,220)
        X2Chat:DispatchChatMessage(2,'[Harbor] |kLiora,0;: Meet us at the harbor after sunset.')
        X2Chat:DispatchChatMessage(2,'|cFF80BFFFThe same message frame and scrollback follow the tab.|r')
        X2Chat:DispatchChatMessage(9,'Trade stays in the original All tab.')
    "#).unwrap();
    lua::tick(&ui.lua, 0.);
    window
}

pub fn drag(ui: &UiRuntime, mouse: &mut input::MouseState, button: Id, to: (f32, f32)) {
    let r = lua::ui(&ui.lua).tree.rect(button);
    let p = (r.x + r.w / 2., r.y + r.h / 2.);
    assert_eq!(input::pick(&ui.lua, p.0, p.1), Some(button));
    input::update(&ui.lua, mouse, Some(p), true, false, false, 0.);
    input::update(
        &ui.lua,
        mouse,
        Some((p.0 + 5., p.1)),
        false,
        false,
        false,
        0.,
    );
    input::update(&ui.lua, mouse, Some(to), false, false, false, 0.);
    input::update(&ui.lua, mouse, Some(to), false, true, false, 0.);
}

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: original chat tab split"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(fake::config()),
        })
        .insert_resource(aa_ui::UiSetup::new(|ui| {
            install(ui);
            fake::report(ui, "native_chat_split");
        }))
        .add_systems(Startup, fake::physical_pixels)
        .add_systems(Update, attempt.after(aa_ui::UiInput).before(aa_ui::UiFrame))
        .run();
}

fn attempt(
    ui: Option<NonSend<UiRuntime>>,
    time: Res<Time>,
    mut done: Local<bool>,
    mut pointer: ResMut<aa_ui::UiVirtualPointer>,
) {
    let Some(ui) = ui else { return };
    pointer.pos = Some(Vec2::new(1100., 40.));
    if *done || time.elapsed_secs() < 0.5 {
        return;
    }
    *done = true;
    let (window, button) = {
        let u = lua::ui(&ui.lua);
        let w = u.tree.of_class("chatwindow")[0];
        (
            w,
            u.tree.widget(w).unwrap().chat.as_ref().unwrap().tabs[1].button,
        )
    };
    drag(&ui, &mut input::MouseState::default(), button, (870., 260.));
    ui.exec("@split_tooltip", "HideTooltip()").unwrap();
    lua::tick(&ui.lua, 0.);
    let u = lua::ui(&ui.lua);
    let windows = u.tree.of_class("chatwindow");
    println!(
        "native_chat_split: windows={} source={} button_parent={:?}",
        windows.len(),
        window,
        u.tree.get(button).parent
    );
    if !std::env::args().any(|a| a == "--baseline") {
        assert_eq!(windows.len(), 2);
    }
}
