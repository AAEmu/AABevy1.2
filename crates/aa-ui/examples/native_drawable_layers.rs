//! Original skins/icon refresh with independent native owner layer admission.
#[path = "fake/mod.rs"]
#[allow(dead_code)]
mod fake;
#[path = "native_button_art.rs"]
#[allow(dead_code)]
mod original;
use aa_ui::{UiRuntime, UiVirtualPointer, lua};
use bevy::prelude::*;

pub fn install(ui: &mut UiRuntime) {
    original::install(ui);
    ui.exec("@native_drawable_layers", r#"
        panel:SetExtent(780,360)
        panel.heading:SetText('Original skins: owner layers preserve drawable visibility')
        artButtons[1]:SetText('Background on')
        artButtons[2]:SetButtonState('NORMAL');artButtons[2]:SetText('Background off')
        artButtons[2]:DisableDrawables('background')
        artButtons[2]:SetHandler('OnClick',function() layerClicks=layerClicks+1 end)
        layerClicks=0
        artButtons[3]:SetButtonState('NORMAL');artButtons[3]:SetText('Own art hidden')
        artButtons[3].bgs[BUTTON_STATE.NORMAL]:SetVisible(false)
        ButtonInit(artButtons[3])
        artButtons[4]:SetText('Disabled control')
        aliasButton:SetText('Original icon refresh')
        actionButton:DisableDrawables('artwork')
        iconMark=actionButton:CreateColorDrawable(0.1,0.8,0.2,1,'background')
        iconMark:SetExtent(24,24);iconMark:AddAnchor('CENTER',actionButton,0,0)
        iconMark:SetVisible(false)
        actionButton:DisableDrawables('background');actionButton:SetItemIcon(4045,2)
        panel.actionCaption:SetText('Artwork layer off')
        panel.note:SetText('Layer-off buttons still respond. Hidden art stays hidden after ButtonInit / icon refresh.')
        assert(artButtons[2].bgs[BUTTON_STATE.NORMAL]:IsVisible())
        assert(not artButtons[3].bgs[BUTTON_STATE.NORMAL]:IsVisible())
        assert(not iconMark:IsVisible())
    "#).unwrap();
    if std::env::args().any(|a| a == "--restored") {
        ui.exec(
            "@restore_native_layers",
            r#"
            ButtonInit(artButtons[2]);artButtons[2]:SetText('ButtonInit restored')
            actionButton:EnableDrawables('artwork')
            panel.actionCaption:SetText('Artwork layer restored')
        "#,
        )
        .unwrap();
    }
    lua::tick(&ui.lua, 0.);
    fake::report(ui, "native_drawable_layers");
}

fn pointer(
    time: Res<Time>,
    runtime: Option<NonSend<UiRuntime>>,
    mut pointer: ResMut<UiVirtualPointer>,
    mut state: Local<u8>,
) {
    pointer.pos = Some(Vec2::ZERO);
    let Some(runtime) = runtime else { return };
    if time.elapsed_secs() >= 5. && *state < 2 {
        let id = lua::value_id(
            &runtime.lua,
            &runtime.lua.load("artButtons[2]").eval().unwrap(),
        )
        .unwrap();
        let u = lua::ui(&runtime.lua);
        let r = u.animations.widget_rect(&u.tree, id);
        pointer.pos = Some(Vec2::new(r.x + r.w * 0.5, r.y + r.h * 0.5) * u.ui_scale);
        if *state == 0 {
            pointer.press = true
        } else {
            pointer.release = true
        }
        *state += 1;
    } else if *state == 2 {
        assert_eq!(runtime.lua.load("layerClicks").eval::<u32>().unwrap(), 1);
        runtime.exec("@layer_click_report", "panel.note:SetText('Physical original button click passed; own hidden art remains hidden.')").unwrap();
        *state = 3;
    }
}
fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: native drawable layer admission"),
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
