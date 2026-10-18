//! Original action-slot text drawables, with explicit content/colors for visual inspection.

use aa_ui::{UiRuntime, lua};

pub fn install(ui: &mut UiRuntime) {
    super::fake::install(ui, "en_us", &super::fake::default_worlds());
    super::fake::install_ingame(ui);
    ui.run_stage(aa_ui::Stage::World);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    ui.exec(
        "@native_drawable_text_slots",
        r#"
        nativeTextSlots={}
        local original=AAUI.SlotContent
        AAUI.SlotContent=function(slotType,index)
            if slotType==ISLOT_ABILITY_VIEW and index>=9001 and index<=9003 then
                local i=index-9000
                local source=assert(original(ISLOT_ACTION,i))
                local content={}
                for key,value in pairs(source) do content[key]=value end
                content.type='buff';content.learned=false;content.canLearn=true
                content.reqPoints=i;content.skillLevel=nil
                return content
            end
            return original(slotType,index)
        end
        nativeTextHost=UIParent:CreateWidget('emptywidget','nativeTextHost','UIParent')
        nativeTextHost:SetExtent(ICON_SIZE.DEFAULT+160,90)
        nativeTextHost:AddAnchor('CENTER',UIParent,0,0);nativeTextHost:Show(true)
        for i=1,3 do
            local slot=CreateActionSlot(nativeTextHost,'nativeTextSlot'..i,ISLOT_ABILITY_VIEW,9000+i)
            slot:AddAnchor('TOPLEFT',nativeTextHost,(i-1)*80,0)
            slot:Show(true)
            nativeTextSlots[i]=slot
        end
    "#,
    )
    .unwrap();
    lua::tick(&ui.lua, 0.0);
    ui.exec(
        "@native_drawable_text_content",
        r#"
        local colors={{1,1,1,1},{0.4,0.8,1,1},{1,0.35,0.2,1}}
        for i,slot in ipairs(nativeTextSlots) do
            local text=slot.reqPointText
            text:SetColor(unpack(colors[i]))
            text:SetShadow(i==2);text:SetOutline(i==3)
        end
    "#,
    )
    .unwrap();
}
