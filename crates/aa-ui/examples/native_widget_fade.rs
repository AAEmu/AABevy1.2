//! Original window art/OnShow animation with the independent native manager fade factor.
#[path = "fake/mod.rs"]
#[allow(dead_code)]
mod fake;

use aa_ui::{UiRuntime, lua};
use bevy::prelude::*;

fn install(ui: &mut UiRuntime) {
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
    ui.exec("@native_fade_example", r#"
        board=UIParent:CreateWidget('window','fadeBoard','UIParent')
        board:SetExtent(1120,450);board:AddAnchor('CENTER',UIParent,0,0);board:Show(true)
        local bg=board:CreateColorDrawable(.12,.15,.18,1,'background')
        bg:AddAnchor('TOPLEFT',board,0,0);bg:AddAnchor('BOTTOMRIGHT',board,0,0)
        local function label(name,text,x,y,width)
            local l=board:CreateChildWidget('label',name,0,true)
            l:SetExtent(width or 330,32);l:AddAnchor('TOPLEFT',board,x,y)
            l.style:SetFontSize(16);l.style:SetAlign(ALIGN_LEFT);l:SetText(text)
            return l
        end
        label('heading','Original window animation / native Show fade / preserved base alpha',25,15,1080)
        anim=CreateWindow('nativeOpening',board)
        hide=CreateWindow('nativeHiding',board)
        reverse=CreateEmptyWindow('nativeReversal',board)
        local plate=reverse:CreateColorDrawable(.5,.46,.36,1,'background')
        plate:AddAnchor('TOPLEFT',reverse,0,0);plate:AddAnchor('BOTTOMRIGHT',reverse,0,0)
        for i,w in ipairs({anim,hide,reverse}) do
            w:Show(false)
            w:SetExtent(310,240);w:AddAnchor('TOPLEFT',board,35+(i-1)*370,85)
            local title=({'OnShow animation','Hidden fade-out','Immediate reversal'})[i]
            if w.SetTitle then w:SetTitle(title) else w:SetTitleText(title) end
            w:Show(true)
            local button=w:CreateChildWidget('button','inside',0,true)
            button:SetExtent(160,30);button:AddAnchor('CENTER',w,0,0)
            ApplyButtonSkin(button,BUTTON_BASIC.DEFAULT);button:SetText('Original button')
        end
        label('animCaption','OnShow keeps alpha + scale channels',25,345)
        label('hideCaption','Hidden for input; painted during fade',395,345)
        label('reverseCaption','SetAlpha(.4) survives Show reversal',765,345)
        status=label('state','Waiting for capture transition',25,390,1080)
        fadeClicks=0;hide:SetHandler('OnClick',function() fadeClicks=fadeClicks+1 end)
    "#).unwrap();
    lua::tick(&ui.lua, 600.);
    fake::report(ui, "native_widget_fade");
}

fn transition(time: Res<Time>, runtime: Option<NonSend<UiRuntime>>, mut done: Local<bool>) {
    let complete = std::env::args().any(|a| a == "--complete");
    if !*done
        && time.elapsed_secs() >= if complete { 6. } else { 7.94 }
        && let Some(runtime) = runtime
    {
        runtime.exec("@fade_transition", r#"
            anim:Show(false);anim:Show(true)
            hide:SetAlpha(.8);hide:Show(false,120)
            reverse:SetAlpha(.4);reverse:Show(false,1000);reverse:Show(true)
            status:SetText('OnShow channels active; hidden fade cannot receive pointer; reversal retains .4 alpha')
            assert(anim:IsNowAnimation() and not hide:IsVisible())
            assert(reverse:GetAlpha()>.399 and reverse:GetAlpha()<.401)
        "#).unwrap();
        let hidden: mlua::Value = runtime.lua.load("hide").eval().unwrap();
        let id = lua::value_id(&runtime.lua, &hidden).unwrap();
        let rect = {
            let u = lua::ui(&runtime.lua);
            u.animations.widget_rect(&u.tree, id)
        };
        assert_ne!(
            aa_ui::input::pick(&runtime.lua, rect.x + rect.w / 2., rect.y + rect.h / 2.),
            Some(id)
        );
        *done = true;
    }
}

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: native widget fade"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(fake::config()),
        })
        .insert_resource(aa_ui::UiSetup::new(install))
        .add_systems(Startup, fake::physical_pixels)
        .add_systems(Update, transition.before(aa_ui::UiFrame))
        .run();
}
