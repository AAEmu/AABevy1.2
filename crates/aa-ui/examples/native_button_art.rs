//! Original button skins, aliased selection art and cooldown-action icons.
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
    ui.exec("@original_button_art", r#"
        panel=UIParent:CreateWidget('window','nativeButtonArtPanel','UIParent')
        panel:SetExtent(780,360);panel:AddAnchor('CENTER',UIParent,0,0);panel:Show(true)
        local bg=panel:CreateColorDrawable(0.12,0.15,0.18,1,'background')
        bg:AddAnchor('TOPLEFT',panel,0,0);bg:AddAnchor('BOTTOMRIGHT',panel,0,0)
        local function label(id,text,x,y,w)
            local l=panel:CreateChildWidget('label',id,0,true)
            l:SetExtent(w,30);l:AddAnchor('TOPLEFT',panel,x,y);l:SetText(text)
            return l
        end
        local title=label('heading','Original button artwork: state and binding lifetime',25,18,730)
        title.style:SetFontSize(20)
        artButtons={};artClicks=0;artStates={}
        local names={'Normal','Highlighted','Pushed','Disabled'}
        for i=1,4 do
            local b=CreateEmptyButton('nativeArtButton'..i,panel)
            ApplyButtonSkin(b,BUTTON_BASIC.DEFAULT)
            b:SetExtent(145,34);b:AddAnchor('TOPLEFT',panel,35+(i-1)*190,90)
            b:SetText(names[i]);b:Show(true);artButtons[i]=b
        end
        artButtons[2]:SetButtonState('HIGHLIGHTED')
        artButtons[3]:SetButtonState('PUSHED')
        artButtons[4]:Enable(false)
        aliasButton=CreateEmptyButton('nativeAliasButton',panel)
        ApplyButtonSkin(aliasButton,BUTTON_BASIC.DEFAULT)
        aliasButton:SetExtent(170,34);aliasButton:AddAnchor('TOPLEFT',panel,35,190)
        aliasButton:SetText('SetBGPushed(true)');aliasButton:Show(true)
        SetBGPushed(aliasButton,true)
        hiddenButton=CreateEmptyButton('nativeHiddenArt',panel)
        ApplyButtonSkin(hiddenButton,BUTTON_BASIC.DEFAULT)
        hiddenButton:SetExtent(170,34);hiddenButton:AddAnchor('TOPLEFT',panel,245,190)
        hiddenButton:SetText('Manually hidden art');hiddenButton:Show(true)
        hiddenButton.bgs[BUTTON_STATE.NORMAL]:SetVisible(false)
        actionButton=CreateCoolDownActionButton('nativeArtAction',panel,'action')
        actionButton:SetExtent(ICON_SIZE.DEFAULT,ICON_SIZE.DEFAULT)
        actionButton:AddAnchor('TOPLEFT',panel,560,182)
        actionButton:SetItemIcon(4045,2);actionButton:Show(true)
        actionButton:ProcessEmptyStackColor(true)
        actionButton.OnClickProc=function() artClicks=artClicks+1 end
        label('actionCaption','Empty stack icon',485,235,195)
        label('note','Original Lua skins and aliases. Missing state art has no normal fallback.',25,298,730)
        artButtons[1]:SetHandler('OnMouseDown',function(self)
            artStates[#artStates+1]=self:GetButtonState()
        end)
        artButtons[1]:SetHandler('OnMouseUp',function(self)
            artStates[#artStates+1]=self:GetButtonState()
        end)
        artButtons[1]:SetHandler('OnClick',function() artClicks=artClicks+1 end)
    "#).unwrap();
    lua::tick(&ui.lua, 0.);
    fake::report(ui, "native_button_art");
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
            &runtime.lua.load("artButtons[1]").eval().unwrap(),
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
            aa_dev::default_plugins("aa-ui: native button artwork"),
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
