//! Original tooltip section constructors and SetInfo consumers, without a network connection.
#[path = "fake/mod.rs"]
#[allow(dead_code)]
mod fake;

use aa_ui::UiRuntime;
use bevy::prelude::*;

pub(crate) fn install_at_scale(ui: &mut UiRuntime, scale: f32) {
    fake::install(ui, "en_us", &fake::default_worlds());
    ui.load_base_libs();
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    ui.exec("@native_tooltip_columns", &format!(r#"
        UIParent:SetUIScale({scale})
        local board=UIParent:CreateWidget('emptywidget','tooltipColumnsBoard','UIParent')
        board:AddAnchor('TOPLEFT',UIParent,0,0);board:AddAnchor('BOTTOMRIGHT',UIParent,0,0);board:Show(true)
        local paper=board:CreateColorDrawable(.08,.10,.13,1,'background')
        paper:AddAnchor('TOPLEFT',board,0,0);paper:AddAnchor('BOTTOMRIGHT',board,0,0)
        local function caption(id,text,x,y,width)
            local label=board:CreateChildWidget('label',id,0,true)
            label:SetExtent(width,28);label:AddAnchor('TOPLEFT',board,x,y)
            label.style:SetAlign(ALIGN_LEFT);label.style:SetColor(.88,.88,.83,1);label:SetText(text)
        end
        caption('heading','Original tooltip columns / ratio / section height',60,35,1150)
        caption('narrow','Two icons: right column 68%',60,90,280)
        caption('wide','One icon: right column 88%',360,90,280)
        caption('money','Original sell / repair money rows',660,90,280)
        caption('quest','Original quest title / status',960,90,280)
        local description='Old forests conceal distant roads and hidden valleys. Follow the river toward the eastern watchtower and return before nightfall.'
        for n,index in ipairs({{5,6}}) do
            local window=tooltip.tooltipWindows[index]
            local sections=tooltip.skillSynergyInfos[index]
            sections:DynamicPushSection(window,1)
            local result=n==1 and 'ui/icon/icon_skill_romance18.dds' or nil
            window:SetInfo({{synergyIconInfo={{{{desc=description,conditionicon='ui/icon/icon_skill_romance23.dds',resulticon=result}}}}}})
            window:ShowTooltip(true,nil)
            window:RemoveAllAnchors();window:AddAnchor('TOPLEFT',UIParent,60+(n-1)*300,130);window:Show(true)
        end
        X2Item.IsInRepairMode=function() return true end
        local window=tooltip.tooltipWindows[1]
        window:PushSection(tooltip.sellables[1])
        window:SetInfo({{sellable=true,slotType='backpack',refund=2470005,isStackable=true,stack=2,repairable=true,repairCost=125}})
        window:ShowTooltip(true,nil)
        window:RemoveAllAnchors();window:AddAnchor('TOPLEFT',UIParent,660,130);window:Show(true)
        X2Quest.GetQuestContextMainTitle=function() return 'Old forests conceal distant roads and hidden valleys' end
        X2Quest.IsProgressQuestInJournal=function() return true end
        X2Quest.GetLinkedQuestSummary=function() end
        X2Quest.IsSelectiveQuest=function() return false end
        X2Quest.IsScoreQuest=function() return false end
        X2Quest.IsGroupQuest=function() return false end
        X2Quest.NowIsAggroComponent=function() return false end
        X2Quest.GetLinkedQuestObjectives=function() end
        window=tooltip.tooltipWindows[8];window:PushSection(tooltip.questBody[8]);window:SetInfo(123)
        window:ShowTooltip(true,nil)
        window:RemoveAllAnchors();window:AddAnchor('TOPLEFT',UIParent,960,130);window:Show(true)
    "#)).unwrap();
    fake::report(ui, "native_tooltip_columns");
}

fn install(ui: &mut UiRuntime) {
    let scale = if std::env::args().any(|a| a == "--scale85") {
        0.85
    } else {
        1.0
    };
    install_at_scale(ui, scale);
}

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: original tooltip columns"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(fake::config()),
        })
        .insert_resource(aa_ui::UiSetup::new(install))
        .add_systems(Startup, fake::physical_pixels)
        .run();
}
