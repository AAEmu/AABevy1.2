//! Actual original checkbutton callbacks and native shown-flag lifetime.
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
    ui.exec("@original_checkbuttons",r#"
        panel=UIParent:CreateWidget('window','nativeCheckPanel','UIParent')
        panel:SetExtent(620,330);panel:AddAnchor('CENTER',UIParent,0,0);panel:Show(true)
        local bg=panel:CreateColorDrawable(0.12,0.15,0.18,1,'background')
        bg:AddAnchor('TOPLEFT',panel,0,0);bg:AddAnchor('BOTTOMRIGHT',panel,0,0)
        local title=panel:CreateChildWidget('label','heading',0,true)
        title:SetExtent(560,30);title:AddAnchor('TOPLEFT',panel,30,20)
        title.style:SetFontSize(20);title.style:SetAlign(ALIGN_LEFT)
        title:SetText('Original checkbutton artwork lifetime')
        checks={};checkClicks=0
        local text={'Checked','Unchecked','Disabled checked','Manually hidden mark','Repeat SetChecked restores mark'}
        for i=1,5 do
            local check=CreateCheckButton('nativeCheck'..i,panel,text[i])
            check:AddAnchor('TOPLEFT',panel,40,70+(i-1)*42);check:Show(true)
            check.CheckBtnCheckChagnedProc=function() checkClicks=checkClicks+1 end
            check:SetChecked(i~=2,false)
            if i==3 then check:SetEnableCheckButton(false) end
            if i==4 or i==5 then check.bgs[5]:SetVisible(false) end
            if i==5 then check:SetChecked(true) end
            checks[i]=check
        end
        checkClicks=0
        local note=panel:CreateChildWidget('label','note',0,true)
        note:SetExtent(560,24);note:AddAnchor('BOTTOM',panel,0,-15)
        note:SetText('Original mark art and text clicks; explicit visibility lasts until a native state write.')
    "#).unwrap();
    lua::tick(&ui.lua, 0.);
    fake::report(ui, "native_checkbutton");
}
fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: native checkbutton lifetime"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(fake::config()),
        })
        .insert_resource(aa_ui::UiSetup::new(install))
        .add_systems(Startup, fake::physical_pixels)
        .run();
}
