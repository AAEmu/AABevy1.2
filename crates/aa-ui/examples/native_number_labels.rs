//! Original money/item labels exercising native SetNumberOnly, with explicit fixture values.
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
        "@native_number_labels",
        r#"
        panel=UIParent:CreateWidget('window','numberPanel','UIParent')
        panel:SetExtent(620,380);panel:AddAnchor('CENTER',UIParent,0,0);panel:Show(true)
        local bg=panel:CreateColorDrawable(.12,.105,.085,.97,'background')
        bg:AddAnchor('TOPLEFT',panel,0,0);bg:AddAnchor('BOTTOMRIGHT',panel,0,0)
        local function label(name,text,x,y)
            local l=panel:CreateChildWidget('label',name,0,true)
            l:SetExtent(580,25);l:AddAnchor('TOPLEFT',panel,x,y)
            l.style:SetAlign(ALIGN_LEFT);l.style:SetFontSize(16);l:SetText(text)
            return l
        end
        label('heading','Original NumberOnly labels: money and item stack',20,20)
        local moneyPlate=panel:CreateColorDrawable(.9,.85,.74,1,'background')
        moneyPlate:SetExtent(138,78);moneyPlate:AddAnchor('TOPLEFT',panel,35,65)
        money=CreateTwoLineMoneyWindow('numberMoney',panel)
        money:AddAnchor('TOPLEFT',panel,50,80)
        money:Update('1234567891234')
        stackButton=CreateItemIconButton('numberStack',panel)
        stackButton:AddAnchor('TOPLEFT',panel,270,75)
        stackButton:SetItemIcon(4045,2);stackButton:SetStack('12345');stackButton:Show(true)
        label('moneyCaption','Original gold / silver / copper',20,145)
        label('stackCaption','Original item stack',270,145)
        local ordinary=label('ordinary','1234567',20,215)
        local numbered=label('numbered','',270,215)
        numbered:SetNumberOnly(true);numbered:SetText('1234567')
        label('ordinaryCaption','NumberOnly false',20,245)
        label('numberedCaption','NumberOnly true',270,245)
        local literal=label('literal','',20,305)
        literal:SetNumberOnly(true);literal:SetText('abcdefg')
        label('literalCaption','Native byte grouping also applies to letters',20,335)
        assert(money.goldLabel:GetText()=='123,456,789')
        assert(money.silverLabel:GetText()=='12' and money.copperLabel:GetText()=='34')
        assert(stackButton.stack:GetText()=='12,345')
        assert(ordinary:GetText()=='1234567' and numbered:GetText()=='1,234,567')
        assert(literal:GetText()=='a,bcd,efg')
    "#,
    )
    .unwrap();
    fake::report(ui, "native_number_labels");
}

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: native number labels"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(fake::config()),
        })
        .insert_resource(aa_ui::UiSetup::new(install))
        .add_systems(Startup, fake::physical_pixels)
        .run();
}
