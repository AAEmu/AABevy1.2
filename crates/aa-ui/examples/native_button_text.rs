//! Original shop confirmation: direct radio-label color and later native state writes.
#[path = "fake/mod.rs"]
#[allow(dead_code)]
mod fake;
use aa_ui::{UiRuntime, UiVirtualPointer, lua};
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
    ui.exec("@original_shop_text",r#"
        textShop=SetViewOfConfirmFrame('nativeTextShop',UIParent)
        textShop:SetTitle('Original purchase confirmation')
        textShop:Show(true)
        textShop.titleGroup.goodName:SetText('Radio label current color')
        textShop.btnOk:SetText(locale.common.ok)
        textShop.btnCancel:SetText(locale.common.cancel)
        for i=1,4 do
            textShop.detailGroup.details[i].textButton:SetText('Purchase option '..i)
        end
        textDetails=textShop.detailGroup.details
        textDetails:Check(1,false)
        local heading=UIParent:CreateWidget('label','textPaletteHeading','UIParent')
        heading:SetExtent(850,30);heading:AddAnchor('TOP',UIParent,0,24)
        heading:SetText('Original shop radio labels: direct style, then hover / press / leave')
        heading.style:SetFontSize(19)
        local note=UIParent:CreateWidget('label','textPaletteNote','UIParent')
        note:SetExtent(850,30);note:AddAnchor('BOTTOM',UIParent,0,-24)
        note:SetText('Unvisited rows retain the authored direct color; state writes restore their own palette.')
        textEvents={}
        textDetails[2].textButton:SetHandler('OnMouseDown',function(self)
            textEvents[#textEvents+1]=self:GetButtonState()
        end)
    "#).unwrap();
    lua::tick(&ui.lua, 600.);
    fake::report(ui, "native_button_text");
}

fn pointer(
    time: Res<Time>,
    runtime: Option<NonSend<UiRuntime>>,
    mut pointer: ResMut<UiVirtualPointer>,
    mut pressed: Local<bool>,
) {
    pointer.pos = Some(Vec2::ZERO);
    if time.elapsed_secs() >= 6.
        && std::env::args().any(|a| a == "--hover" || a == "--pressed")
        && let Some(runtime) = runtime
    {
        let id = lua::value_id(
            &runtime.lua,
            &runtime
                .lua
                .load("textDetails[2].textButton")
                .eval()
                .unwrap(),
        )
        .unwrap();
        let u = lua::ui(&runtime.lua);
        let r = u.animations.widget_rect(&u.tree, id);
        pointer.pos = Some(Vec2::new(r.x + r.w * 0.5, r.y + r.h * 0.5) * u.ui_scale);
        if !*pressed && std::env::args().any(|a| a == "--pressed") {
            pointer.press = true;
            *pressed = true;
        }
    }
}

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: native button text"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(fake::config()),
        })
        .insert_resource(aa_ui::UiSetup::new(install))
        .add_systems(Startup, fake::physical_pixels)
        .add_systems(Update, pointer.before(aa_ui::UiFrame))
        .run();
}
