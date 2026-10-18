//! Original window opening reconfigured while running, plus retained-acceleration controls.
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
    ui.exec(
        "@native_channel_fixture",
        r#"
        board=UIParent:CreateWidget('window','channelBoard','UIParent')
        board:SetExtent(1120,450);board:AddAnchor('CENTER',UIParent,0,0);board:Show(true)
        local bg=board:CreateColorDrawable(.12,.15,.18,1,'background')
        bg:AddAnchor('TOPLEFT',board,0,0);bg:AddAnchor('BOTTOMRIGHT',board,0,0)
        local function label(name,text,x,y,width)
            local l=board:CreateChildWidget('label',name,0,true)
            l:SetExtent(width or 330,32);l:AddAnchor('TOPLEFT',board,x,y)
            l.style:SetFontSize(16);l.style:SetAlign(ALIGN_LEFT);l:SetText(text)
            return l
        end
        label('heading','Native running-channel reconfiguration on original window art',25,15,1080)
        windows={}
        local titles={'Original OnShow','Retained acceleration','Replaced acceleration'}
        for i=1,3 do
            local w=CreateWindow('nativeChannel'..i,board)
            w:SetExtent(310,240);w:AddAnchor('TOPLEFT',board,35+(i-1)*370,85)
            w:SetTitle(titles[i]);w:Show(true)
            local button=w:CreateChildWidget('button','inside',0,true)
            button:SetExtent(160,30);button:AddAnchor('CENTER',w,0,0)
            ApplyButtonSkin(button,BUTTON_BASIC.DEFAULT);button:SetText('Original button')
            windows[i]=w
        end
        label('originalCaption','Current alpha / scale stay in place',25,345)
        label('retainedCaption','Zero acceleration time keeps prior value',395,345)
        label('replaceCaption','Nonzero time replaces the coefficient',765,345)
        status=label('state','Waiting for channel transition',25,390,1080)
    "#,
    )
    .unwrap();
    lua::tick(&ui.lua, 600.);
    fake::report(ui, "native_widget_channel");
}

pub fn step(ui: &UiRuntime, phase: u8) {
    let code = if phase == 0 {
        r#"
        for i,w in ipairs(windows) do
            w:Show(false);w:Show(true)
            if i>1 then
                w:SetAlphaAnimation(.2,1,.5,.4)
                w:SetScaleAnimation(1.1,1,.5,.4,'CENTER')
                w:SetStartAnimation(true,true)
            end
        end
    "#
    } else {
        r#"
        windows[1]:SetAlphaAnimation(.2,1,.8,0)
        windows[1]:SetScaleAnimation(1.5,.9,2,0,'CENTER')
        for i=2,3 do
            local a=i==2 and 0 or .8
            windows[i]:SetAlphaAnimation(.1,.9,.8,a)
            windows[i]:SetScaleAnimation(.9,1.15,.8,a,'CENTER')
        end
        status:SetText('Reconfiguration keeps current values and active flags; velocity changes immediately.')
    "#
    };
    ui.exec("@native_channel_step", code).unwrap();
}

fn scenario(time: Res<Time>, runtime: Option<NonSend<UiRuntime>>, mut phase: Local<u8>) {
    let Some(runtime) = runtime else { return };
    let base = if std::env::args().any(|a| a == "--complete") {
        5.
    } else {
        7.7
    };
    while *phase < 2 && time.elapsed_secs() >= base + [0., 0.08][*phase as usize] {
        step(&runtime, *phase);
        *phase += 1;
    }
}

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: native widget channels"),
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
