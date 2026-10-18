//! Original cooldown-action empty-stack dim/restore with unchanged state icons.
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
    ui.exec("@original_button_colors", r#"
        panel=UIParent:CreateWidget('window','nativeButtonColorPanel','UIParent')
        panel:SetExtent(680,270);panel:AddAnchor('CENTER',UIParent,0,0);panel:Show(true)
        local bg=panel:CreateColorDrawable(0.12,0.15,0.18,1,'background')
        bg:AddAnchor('TOPLEFT',panel,0,0);bg:AddAnchor('BOTTOMRIGHT',panel,0,0)
        local heading=panel:CreateChildWidget('label','heading',0,true)
        heading:SetExtent(630,35);heading:AddAnchor('TOPLEFT',panel,25,20)
        heading.style:SetFontSize(20);heading.style:SetAlign(ALIGN_LEFT)
        heading:SetText('Original cooldown button: empty stack state colors')
        colorButtons={};colorClicks=0
        local titles={'Available','Empty stack','Restored'}
        for i=1,3 do
            local button=CreateCoolDownActionButton('nativeColorButton'..i,panel,'action')
            button:SetExtent(ICON_SIZE.DEFAULT,ICON_SIZE.DEFAULT)
            button:AddAnchor('TOP',panel,(i-2)*200,115)
            button:SetItemIcon(4045,2);button:Show(true)
            button.OnClickProc=function() colorClicks=colorClicks+1 end
            button:ProcessEmptyStackColor(i~=1)
            if i==3 then button:ProcessEmptyStackColor(false) end
            colorButtons[i]=button
            local title=panel:CreateChildWidget('label','columnTitle',i,true)
            title:SetExtent(180,24);title:AddAnchor('BOTTOM',button,'TOP',0,-15)
            title.style:SetFontSize(17);title:SetText(titles[i])
            local value=panel:CreateChildWidget('label','columnValue',i,true)
            value:SetExtent(180,24);value:AddAnchor('TOP',button,'BOTTOM',0,15)
            value:SetText(i==2 and 'Icon alpha 0.30' or 'Icon alpha 1.00')
        end
        local note=panel:CreateChildWidget('label','note',0,true)
        note:SetExtent(630,30);note:AddAnchor('BOTTOM',panel,0,-15)
        note:SetText('Original 42 px icons. Matching state images change; other artwork and text remain unchanged.')
    "#).unwrap();
    lua::tick(&ui.lua, 0.);
    fake::report(ui, "native_button_colors");
}

fn pointer(
    time: Res<Time>,
    runtime: Option<NonSend<UiRuntime>>,
    mut pointer: ResMut<UiVirtualPointer>,
    mut pressed: Local<bool>,
) {
    pointer.pos = Some(Vec2::ZERO);
    if std::env::args().any(|a| a == "--pressed")
        && time.elapsed_secs() >= 6.
        && let Some(runtime) = runtime
    {
        let id = lua::value_id(
            &runtime.lua,
            &runtime.lua.load("colorButtons[2]").eval().unwrap(),
        )
        .unwrap();
        let u = lua::ui(&runtime.lua);
        let r = u.animations.widget_rect(&u.tree, id);
        pointer.pos = Some(Vec2::new(r.x + r.w * 0.5, r.y + r.h * 0.5) * u.ui_scale);
        if !*pressed {
            pointer.press = true;
            *pressed = true;
        }
    }
}

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: native button state colors"),
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
