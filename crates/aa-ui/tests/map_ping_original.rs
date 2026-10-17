//! Original ping Lua receives authoritative points on the world map and roadmap.

#[path = "../examples/fake/mod.rs"]
#[allow(dead_code)]
mod fake;

use aa_ui::{UiRuntime, lua};

fn eval<T: mlua::FromLua>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().expect(code)
}

#[test]
fn original_ping_widgets_wait_for_server_and_pulse_on_both_maps() {
    let mut ui = UiRuntime::new("en_us", 1920.0, 1080.0).unwrap();
    fake::install(&mut ui, "en_us", &fake::default_worlds());
    fake::install_ingame(&mut ui);
    ui.run_stage(aa_ui::Stage::World);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    ui.exec(
        "@authoritative_ping",
        r#"
        echo=false
        AAUI.WorldMap=function() return {level=2,zone=5,player={u=.5,v=.5},ping=echo} end
        AAUI.RoadMap=AAUI.WorldMap
        ADDON:ShowContent(UIC_WORLDMAP,true)
        pulses=0
        for _,frame in ipairs({mapFrame.wPing,mapFrame.rPing}) do
            local start=frame.effect.SetStartEffect
            frame.effect.SetStartEffect=function(self,on)
                if on then pulses=pulses+1 end
                return start(self,on)
            end
        end
        AAUI.PlaceMapPing=function() return true end
        AAUI.RemoveMapPing=function() return true end
    "#,
    )
    .unwrap();
    lua::tick(&ui.lua, 16.0);
    assert!(!eval::<bool>(&ui, "mapFrame.wPing:IsVisible()"));
    assert!(!eval::<bool>(&ui, "mapFrame.rPing.bg:IsVisible()"));
    ui.exec("@echo", "echo={level=2,id=5,u=.4,v=.6,version=1}; worldmap:UpdatePingInfo(); roadmap:UpdatePingInfo()").unwrap();
    lua::tick(&ui.lua, 16.0);
    assert!(eval::<bool>(&ui, "mapFrame.wPing.bg:IsVisible()"));
    assert!(eval::<bool>(&ui, "mapFrame.rPing.bg:IsVisible()"));
    assert_eq!(eval::<i64>(&ui, "pulses"), 2);
    for _ in 0..4 {
        lua::tick(&ui.lua, 16.0);
    }
    assert_eq!(eval::<i64>(&ui, "pulses"), 2);
    // Original global RemovePing delegates to worldmap, but the server still owns both icons.
    ui.exec("@request", "RemovePing()").unwrap();
    assert!(eval::<bool>(&ui, "mapFrame.wPing.bg:IsVisible()"));
    ui.exec(
        "@clear",
        "echo=false; worldmap:UpdatePingInfo(); roadmap:UpdatePingInfo()",
    )
    .unwrap();
    lua::tick(&ui.lua, 16.0);
    assert!(!eval::<bool>(&ui, "mapFrame.wPing.bg:IsVisible()"));
    assert!(!eval::<bool>(&ui, "mapFrame.rPing.bg:IsVisible()"));
    ui.exec("@repeat", "echo={level=2,id=5,u=.4,v=.6,version=2}")
        .unwrap();
    lua::tick(&ui.lua, 16.0);
    lua::tick(&ui.lua, 16.0);
    assert_eq!(eval::<i64>(&ui, "pulses"), 4);
}
