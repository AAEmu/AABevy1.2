//! Original equipped-item mask and explicit alternate texture/UV diagnostic.
#[path = "fake/mod.rs"]
#[allow(dead_code)]
mod fake;
#[path = "native_cooldown_caption.rs"]
#[allow(dead_code)]
mod original;
use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins((aa_dev::default_plugins("aa-ui: native cooldown mask"),aa_dev::DevPlugin))
        .add_plugins(aa_ui::UiPlugin{config:Some(fake::config())})
        .insert_resource(aa_ui::UiSetup::new(|ui|{
            original::install(ui,true);
            ui.exec("@mask_sources",r#"
                panel:SetExtent(560,390)
                local title=panel:CreateChildWidget('label','maskTitle',0,true)
                title:SetExtent(540,24);title:AddAnchor('TOP',panel,0,265)
                title:SetText('Custom mask/crop       Transparent art       No mask');title.style:SetFontSize(16)
                for i=1,3 do
                    local b=CreateCoolDownActionButton('maskSource'..i,panel,'generic')
                    b:SetExtent(42,42);b:AddAnchor('TOP',panel,(i-2)*170,310)
                    b:SetItemIcon(4045,2);b:SetCoolDown(8000,12000);b:Show(true)
                    if i>=2 then
                        local art=b:CreateImageDrawable('ui/icon/item_pick_cursor.dds','artwork')
                        art:AddAnchor('TOPLEFT',b,1,1);art:AddAnchor('BOTTOMRIGHT',b,-1,-1)
                        b:SetNormalBackground(art)
                    end
                    if i==1 then
                        b:SetCoolDownMask('ui/icon/icon_skill_basic01.dds')
                        b:SetCoolDownMaskCoords(8,8,16,16)
                    elseif i==2 then
                        b:SetCoolDownMask('ui/icon/cooldown_mask.dds')
                    end
                end
            "#).unwrap();
        }))
        .add_systems(Startup,fake::physical_pixels)
        .run();
}
