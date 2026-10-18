//! Original Achievement end handler observes old scale, then installs the final OnScale.
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
    let roots: Vec<_> = lua::ui(&ui.lua)
        .tree
        .objects
        .iter()
        .enumerate()
        .filter(|(id, o)| *id != 0 && o.alive && o.parent.is_none())
        .map(|(id, _)| id as u32)
        .collect();
    for id in roots {
        lua::ui(&ui.lua).tree.get_mut(id).shown = false;
    }
    ui.lua
        .globals()
        .set(
            "ObserveCallbackScale",
            ui.lua
                .create_function(|lua, value: mlua::Value| {
                    let id = lua::value_id(lua, &value).unwrap();
                    Ok(lua::ui(lua).animations.widget_sample(id).scale)
                })
                .unwrap(),
        )
        .unwrap();
    ui.exec("@original_callback_achievement",r#"
        achievement=CreateAchievementWindow('nativeCallbackAchievement','UIParent')
        achievement:RemoveAllAnchors();achievement:AddAnchor('CENTER',UIParent,180,0)
        board=UIParent:CreateWidget('window','callbackBoard','UIParent')
        board:SetExtent(330,310);board:AddAnchor('RIGHT',achievement,'LEFT',-25,0);board:Show(true)
        local bg=board:CreateColorDrawable(.12,.15,.18,1,'background')
        bg:AddAnchor('TOPLEFT',board,0,0);bg:AddAnchor('BOTTOMRIGHT',board,0,0)
        local heading=board:CreateChildWidget('label','heading',0,true)
        heading:SetExtent(290,32);heading:AddAnchor('TOPLEFT',board,20,20)
        heading.style:SetAlign(ALIGN_LEFT);heading.style:SetFontSize(18)
        heading:SetText('Original Achievement callbacks')
        callbackStatus=board:CreateChildWidget('textbox','status',0,true)
        callbackStatus:SetExtent(290,230);callbackStatus:AddAnchor('TOPLEFT',board,20,65)
        callbackStatus.style:SetAlign(ALIGN_LEFT);callbackStatus.style:SetFontSize(16)
        callbackStatus:SetText('Waiting for the original opening animation.')
        callbackScales={}
        local adjust=achievement.categoryWnd.AdjustScale
        achievement.categoryWnd.AdjustScale=function(self,...)
            table.insert(callbackScales,ObserveCallbackScale(achievement))
            adjust(self,...)
            callbackStatus:SetText(string.format(
                'Actual original handlers retained.\n\nEnd callback: %.6f\nFinal OnScale: %s\n\nThe end handler installs OnScale before the final geometry is written.',
                callbackScales[1],callbackScales[2] and string.format('%.6f',callbackScales[2]) or 'pending'))
        end
    "#).unwrap();
    lua::tick(&ui.lua, 0.);
    fake::report(ui, "native_widget_channel_callbacks");
}

pub fn demonstrate(ui: &UiRuntime) {
    ui.exec("@callback_open", "callbackScales={};achievement:Show(true)")
        .unwrap();
    lua::tick(&ui.lua, 80.);
    let old: f32 = ui
        .lua
        .load("ObserveCallbackScale(achievement)")
        .eval()
        .unwrap();
    lua::tick(&ui.lua, 40.);
    let observations: Vec<f32> = ui.lua.load("callbackScales").eval().unwrap();
    assert_eq!(observations.len(), 2);
    assert_eq!(observations[0].to_bits(), old.to_bits());
    assert_eq!(observations[1], 1.);
    assert!(observations[0] < observations[1]);
}

fn scenario(time: Res<Time>, runtime: Option<NonSend<UiRuntime>>, mut done: Local<bool>) {
    if !*done
        && time.elapsed_secs() >= 6.
        && let Some(runtime) = runtime
    {
        demonstrate(&runtime);
        *done = true;
    }
}

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: native Achievement callback order"),
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
