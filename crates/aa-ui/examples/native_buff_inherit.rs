//! Original BuffUpdate reorders a newer icon into an older button's native animation state.
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
    ui.exec("@original_buff_inherit",r#"
        board=UIParent:CreateWidget('window','inheritBoard','UIParent')
        board:SetExtent(700,380);board:AddAnchor('CENTER',UIParent,0,0);board:Show(true)
        local bg=board:CreateColorDrawable(.12,.15,.18,1,'background')
        bg:AddAnchor('TOPLEFT',board,0,0);bg:AddAnchor('BOTTOMRIGHT',board,0,0)
        local function label(name,text,y)
            local l=board:CreateChildWidget('label',name,0,true)
            l:SetExtent(660,30);l:AddAnchor('TOPLEFT',board,20,y)
            l.style:SetFontSize(16);l.style:SetAlign(ALIGN_LEFT);l:SetText(text)
            return l
        end
        label('heading','Original staggered BuffUpdate: newer icon inherits into button 1',20)
        label('description','ID 11 starts first; ID 22 starts later; removing 11 retains the newer animation.',60)
        status=label('state','Waiting for original buff updates',330)
        owner=board:CreateChildWidget('emptywidget','buffOwner',0,true)
        owner:SetExtent(0,0);owner:AddAnchor('TOPLEFT',board,70,135);owner:SetScale(3)
        nativeBuffs={}
        X2Unit.UnitBuffCount=function() return #nativeBuffs end
        X2Unit.UnitBuff=function(self,target,index) return nativeBuffs[index] end
        buffs=CreateBuffWindow('nativeInheritedBuffs',owner,5,true)
        buffs:AddAnchor('TOPLEFT',owner,0,0);buffs:Show(true)
    "#).unwrap();
    lua::tick(&ui.lua, 0.);
    fake::report(ui, "native_buff_inherit");
}

pub fn step(ui: &UiRuntime, step: u8) {
    let code = match step {
        0 => {
            r#"nativeBuffs={{buff_id=11,path='ui/icon/icon_skill_basic01.dds',timeLeft=60000}};buffs:BuffUpdate('player')"#
        }
        1 => {
            r#"nativeBuffs[2]={buff_id=22,path='ui/icon/icon_skill_basic02.dds',timeLeft=60000};buffs:BuffUpdate('player')"#
        }
        _ => {
            r#"nativeBuffs={nativeBuffs[2]};buffs:BuffUpdate('player');status:SetText('ID 22 moved to button 1 with its current alpha / scale, without restarting.')"#
        }
    };
    ui.exec("@original_buff_step", code).unwrap();
}

fn scenario(time: Res<Time>, runtime: Option<NonSend<UiRuntime>>, mut phase: Local<u8>) {
    let Some(runtime) = runtime else { return };
    let base = if std::env::args().any(|a| a == "--complete") {
        6.
    } else {
        7.92
    };
    while *phase < 3 && time.elapsed_secs() >= base + [0., 0.04, 0.06][*phase as usize] {
        step(&runtime, *phase);
        *phase += 1;
    }
}

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: original buff inheritance"),
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
