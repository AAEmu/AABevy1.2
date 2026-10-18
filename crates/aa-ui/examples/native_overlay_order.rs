//! Unchanged original cooldown-action counters under black/red owner overlays.
#[path = "fake/mod.rs"]
#[allow(dead_code)]
mod fake;
#[path = "native_button_art.rs"]
#[allow(dead_code)]
mod original;
use aa_ui::{UiRuntime, UiVirtualPointer, lua};
use bevy::prelude::*;

pub fn install(ui: &mut UiRuntime) {
    original::install(ui);
    ui.exec("@native_overlay_order", r#"
        for _,b in ipairs(artButtons) do b:Show(false) end
        aliasButton:Show(false);hiddenButton:Show(false);actionButton:Show(false)
        panel.heading:SetText('Original counters: child labels paint beneath owner overlays')
        panel.note:SetText('Original SetStack / SetOverlayColor / ProcessDurabilityColor, with unchanged icons and text.')
        panel.actionCaption:Show(false)
        overlaySlots={};overlayClicks=0
        local captions={'Available: clear overlay','Unavailable: black overlay','Broken: original red overlay'}
        for i=1,3 do
            local slot=CreateCoolDownActionButton('overlayOrderSlot'..i,panel,'action')
            slot:SetExtent(42,42);slot:AddAnchor('TOPLEFT',panel,140+(i-1)*230,140)
            slot:SetItemIcon(4045,2);slot:SetStack(42);slot:Show(true)
            slot.OnClickProc=function() overlayClicks=overlayClicks+1 end
            local caption=panel:CreateChildWidget('label','overlayCaption',i,true)
            caption:SetExtent(225,30);caption:AddAnchor('TOP',slot,'BOTTOM',0,12)
            caption:SetText(captions[i]);caption:Show(true)
            overlaySlots[i]=slot
        end
        overlaySlots[2]:SetOverlayColor('black')
        overlaySlots[3]:ProcessDurabilityColor({durability=0,maxDurability=100})
    "#).unwrap();
    if std::env::args().any(|a| a == "--clear") {
        ui.exec("@clear_original_overlays", r#"
            overlaySlots[2]:OverlayInvisible();overlaySlots[3]:OverlayInvisible()
            panel.heading:SetText('Original counters: overlays cleared without replacing child labels')
        "#).unwrap();
    }
    lua::tick(&ui.lua, 0.);
    fake::report(ui, "native_overlay_order");
}
fn pointer(
    time: Res<Time>,
    runtime: Option<NonSend<UiRuntime>>,
    mut pointer: ResMut<UiVirtualPointer>,
    mut state: Local<u8>,
) {
    pointer.pos = Some(Vec2::ZERO);
    let Some(runtime) = runtime else { return };
    if time.elapsed_secs() >= 5. && *state < 2 {
        let id = lua::value_id(
            &runtime.lua,
            &runtime.lua.load("overlaySlots[2]").eval().unwrap(),
        )
        .unwrap();
        let u = lua::ui(&runtime.lua);
        let r = u.animations.widget_rect(&u.tree, id);
        pointer.pos = Some(Vec2::new(r.x + r.w * 0.5, r.y + r.h * 0.5) * u.ui_scale);
        if *state == 0 {
            pointer.press = true
        } else {
            pointer.release = true
        }
        *state += 1;
    } else if *state == 2 {
        assert_eq!(runtime.lua.load("overlayClicks").eval::<u32>().unwrap(), 1);
        runtime.exec("@overlay_click_pass", "panel.note:SetText('Physical original slot click passed; counters retain their original text and handlers.')").unwrap();
        *state = 3;
    }
}
fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: native child / overlay order"),
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
