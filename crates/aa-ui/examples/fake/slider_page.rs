//! Unchanged original Sound Options control used by positive-page pointer tests/examples.
use aa_ui::UiRuntime;
#[path = "mod.rs"]
#[allow(dead_code)]
mod fake;
#[cfg(not(test))]
pub(crate) use fake::{config, physical_pixels};

pub fn load(ui: &mut UiRuntime, scale: f32) {
    fake::install(ui, "en_us", &fake::default_worlds());
    fake::install_ingame(ui);
    ui.load_base_libs();
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    ui.exec("@slider_scale", &format!("UIParent:SetUIScale({scale})"))
        .unwrap();
}

pub fn install(ui: &mut UiRuntime, scale: f32) {
    load(ui, scale);
    ui.exec(
        "@original_sound_page",
        &format!(
            r#"
        UIParent:SetUIScale({scale})
        X2Option:SetItemFloatValue(OPTION_ITEM_SOUND_QUALITY,2)
        optionFrame=CreateOptionFrame()
        optionFrame:Show(true)
        local pages=optionFrame.contentWindow.pageWindow.subFrames
        for i=1,#pages do pages[i]:Show(i==3) end
        soundFrame=pages[3]
        soundFrame:Init()
        quality=soundFrame.content.optionFrames[1].sliderbar
        assert(quality:GetValue()==2)
        local lo,hi=quality:GetMinMaxValues()
        assert(lo==1 and hi==4 and quality:GetValueStep()==1)
        qualityChanges=0;qualityLast=0
        quality:SetHandler('OnSliderChanged',function(self,value)
            qualityChanges=qualityChanges+1;qualityLast=value
        end)
    "#
        ),
    )
    .unwrap();
}

/// The same complete original Sound subframe in a controlled original window.
/// Avoid constructing the other thirteen option pages in a short visual control.
pub fn install_visual(ui: &mut UiRuntime, scale: f32) {
    load(ui, scale);
    ui.exec("@original_sound_visual", r#"
        X2Option:SetItemFloatValue(OPTION_ITEM_SOUND_QUALITY,2)
        soundHost=CreateWindow('soundPageHost','UIParent')
        soundHost:SetExtent(460,600);soundHost:AddAnchor('CENTER',UIParent,0,0)
        soundHost:SetTitle(locale.optionWindow.sound.title);soundHost:Show(true)
        soundHost.titleLabel=CreateLabel('soundPageTitle',soundHost)
        soundHost.titleLabel:SetExtent(400,25);soundHost.titleLabel:AddAnchor('TOPLEFT',soundHost,20,40)
        soundHost.titleLabel:SetText(locale.optionWindow.sound.title)
        soundFrame=CreateSoundOptionFrame(soundHost,3)
        soundFrame:Show(true);soundFrame:Init()
        quality=soundFrame.content.optionFrames[1].sliderbar
        assert(quality:GetValue()==2)
        qualityChanges=0;qualityLast=0
        quality:SetHandler('OnSliderChanged',function(self,value)
            qualityChanges=qualityChanges+1;qualityLast=value
        end)
    "#).unwrap();
}
