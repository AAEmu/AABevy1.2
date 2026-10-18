//! Original action slots and equipped-item buttons with explicit fixed fixture timers.
#[path = "fake/constant_cooldown.rs"]
mod constant_cooldown;
#[path = "fake/mod.rs"]
#[allow(dead_code)]
mod fake;
use aa_ui::{UiRuntime, lua};
use bevy::prelude::*;

pub fn install(ui: &mut UiRuntime, visible: bool) {
    constant_cooldown::install(ui);
    // Settle original queued HUD initialization/opening before isolating these controls.
    lua::tick(&ui.lua, 600.);
    let roots: Vec<_> = lua::ui(&ui.lua)
        .tree
        .objects
        .iter()
        .enumerate()
        .filter(|(id, object)| *id != 0 && object.alive && object.parent.is_none())
        .map(|(id, _)| id as aa_ui::tree::Id)
        .collect();
    for id in roots {
        lua::ui(&ui.lua).tree.get_mut(id).shown = false;
    }
    ui.exec(
        "@native_captions",
        r#"
        panel=UIParent:CreateWidget('window','captionPanel','UIParent')
        panel:SetExtent(560,280);panel:AddAnchor('CENTER',UIParent,0,0);panel:Show(true)
        local bg=panel:CreateColorDrawable(.08,.075,.06,.95,'background')
        bg:AddAnchor('TOPLEFT',panel,0,0);bg:AddAnchor('BOTTOMRIGHT',panel,0,0)
        local function title(name,text,y)
            local label=panel:CreateChildWidget('label',name,0,true)
            label:SetExtent(540,24);label:AddAnchor('TOP',panel,0,y)
            label:SetText(text);label.style:SetFontSize(16)
        end
        title('slotTitle','Original action slots',18)
        title('equipmentTitle','Original equipment buttons',145)
        captionSamples={{1200,1500},{60999,90000},{86401000,90000000}}
        captionSlots={}
        AAUI.SlotContent=function(kind,index)
            local s=captionSamples[index]
            if s then return {type='skill',id=index,icon='ui/icon/icon_skill_basic01.dds',
                cooldown={remain=s[1],total=s[2]}} end
        end
        for i=1,3 do
            local slot=CreateActionSlot(panel,'captionAction'..i,ISLOT_ACTION,i)
            slot:RemoveAllAnchors();slot:AddAnchor('TOP',panel,(i-2)*130,64)
            slot:Show(true)
            captionSlots[i]=slot
            local button=CreateCoolDownActionButton('captionItem'..i,panel,'constant')
            button:SetExtent(ICON_SIZE.DEFAULT,ICON_SIZE.DEFAULT)
            button:SetItemIcon(4045,2)
            AddOnUpdateConstantCooldown(button,'action')
            button:SetSkill(i)
            button:AddAnchor('TOP',panel,(i-2)*130,192)
            button:Show(true)
            nativeCooldownButtons[i]=button
        end
    "#,
    )
    .unwrap();
    lua::slot::set_cooldown_numbers(&ui.lua, visible);
    fake::report(ui, "native_cooldown_caption");
}

fn fixture_samples(rt: Option<NonSend<UiRuntime>>) {
    // Fixed explicit samples expose float-boundary labels in unattended captures.
    if let Some(rt) = rt {
        rt.exec(
            "@fixed_caption_samples",
            "for i=1,3 do captionSlots[i]:EstablishSlot(ISLOT_ACTION,i) end",
        )
        .unwrap();
    }
}

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: native cooldown captions"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(fake::config()),
        })
        .insert_resource(aa_ui::UiSetup::new(|ui| {
            install(ui, !std::env::args().any(|a| a == "--no-numbers"));
        }))
        .add_systems(Startup, fake::physical_pixels)
        .add_systems(Update, fixture_samples.before(aa_ui::UiFrame))
        .run();
}
