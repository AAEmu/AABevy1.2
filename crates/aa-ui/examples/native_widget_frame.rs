//! Actual battlefield waiting-alarm OnUpdate starts a fade before the channel step.
#[path = "fake/mod.rs"]
#[allow(dead_code)]
mod fake;
use aa_ui::{UiRuntime, lua};
use bevy::prelude::*;

pub fn install(ui: &mut UiRuntime) {
    fake::install(ui, "en_us", &fake::default_worlds());
    fake::install_ingame(ui);
    ui.run_stage(aa_ui::Stage::World);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    lua::tick(&ui.lua, 600.);
    let roots = lua::ui(&ui.lua).tree.top.clone();
    for id in roots {
        lua::ui(&ui.lua).tree.get_mut(id).shown = false;
    }
    // The factory/helper are locals in the actual bytecode. Observe its registration
    // while reloading this one original asset, then invoke its unchanged end callback.
    ui.exec(
        "@observe_alarm_registration",
        r#"
        savedAlarmRegister=UIParent.SetEventHandler
        UIParent.SetEventHandler=function(self,name,callback)
            if name=='INSTANT_GAME_END' then OriginalAlarmEnd=callback end
            return savedAlarmRegister(self,name,callback)
        end
    "#,
    )
    .unwrap();
    aa_lua::run_alb(&ui.lua, "scriptsbin/x2ui/battlefield/waiting_alarm.alb").unwrap();
    ui.exec(
        "@original_battlefield_frame",
        r#"
        UIParent.SetEventHandler=savedAlarmRegister
        -- Received-data stand-in and external scoreboard endpoint; all alarm handlers
        -- and its constructor/art/configuration remain original bytecode.
        X2BattleField.GetProgressEntireInfo=function() return {endReason=-999} end
        scoreboardRequests=0
        ShowScoreBoard=function(show,automatic)
            assert(show==true and automatic==true);scoreboardRequests=scoreboardRequests+1
        end
        report=UIParent:CreateWidget('window','nativeFrameReport','UIParent')
        report:SetExtent(700,230);report:AddAnchor('CENTER',UIParent,0,100);report:Show(true)
        local bg=report:CreateColorDrawable(.12,.15,.18,1,'background')
        bg:AddAnchor('TOPLEFT',report,0,0);bg:AddAnchor('BOTTOMRIGHT',report,0,0)
        local heading=report:CreateChildWidget('label','heading',0,true)
        heading:SetExtent(650,35);heading:AddAnchor('TOPLEFT',report,25,20)
        heading.style:SetFontSize(21);heading.style:SetAlign(ALIGN_LEFT)
        heading:SetText('Original battlefield frame order')
        status=report:CreateChildWidget('textbox','status',0,true)
        status:SetExtent(650,150);status:AddAnchor('TOPLEFT',report,25,70)
        status.style:SetFontSize(18);status.style:SetAlign(ALIGN_LEFT)
        status:SetText('Waiting for the actual alarm hold callback.')
    "#,
    )
    .unwrap();
    lua::tick(&ui.lua, 0.);
    fake::report(ui, "native_widget_frame");
}

pub fn demonstrate(ui: &UiRuntime) {
    ui.exec(
        "@frame_alarm_start",
        "OriginalAlarmEnd();alarm=battlefield.waitingAlarm",
    )
    .unwrap();
    lua::tick(&ui.lua, 500.);
    lua::tick(&ui.lua, 9980.);
    // The handler installed by alpha-end must not also consume the preceding500ms
    // frame. The real hold still has20ms remaining at this point.
    assert_eq!(
        ui.lua.load("alarm.animState").eval::<String>().unwrap(),
        "end_anim_start"
    );
    assert_eq!(ui.lua.load("alarm:GetAlpha()").eval::<f32>().unwrap(), 1.);
    lua::tick(&ui.lua, 20.);
    let alpha: f32 = ui.lua.load("alarm:GetAlpha()").eval().unwrap();
    assert_eq!(alpha.to_bits(), 0.96f32.to_bits());
    assert_eq!(
        ui.lua.load("alarm.animState").eval::<String>().unwrap(),
        "end_anim_end"
    );
    ui.exec("@frame_alarm_observation", r#"
        status:SetText(string.format(
            'Original OnUpdate reaches the 10-second hold.\nIt starts the fade; the SAME 20 ms frame advances alpha to %.6f.\n\nNative: OnUpdate / OnEffect / channels / shown children.\nAll original waiting-alarm handlers remain active.',alarm:GetAlpha()))
    "#).unwrap();
}

fn scenario(time: Res<Time>, runtime: Option<NonSend<UiRuntime>>, mut done: Local<bool>) {
    if !*done
        && time.elapsed_secs() >= 7.8
        && let Some(runtime) = runtime
    {
        demonstrate(&runtime);
        *done = true;
    }
}

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: native widget frame order"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(fake::config()),
        })
        .insert_resource(aa_ui::UiSetup::new(install))
        .add_systems(Startup, fake::physical_pixels)
        .add_systems(Update, scenario.before(aa_ui::UiFrame))
        .run();
}
