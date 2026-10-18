//! Source selector fixture: raw GCD / prior accessor-active / excluded kind16.
#[path = "fake/mod.rs"]
#[allow(dead_code)]
mod fake;
#[path = "native_cooldown_caption.rs"]
#[allow(dead_code)]
mod original;
use aa_ui::{UiRuntime, lua};
use bevy::prelude::*;
use lua::slot::{CooldownSample as Sample, update_native_slot_cooldown};
fn samples(rt: Option<NonSend<UiRuntime>>) {
    if let Some(rt) = rt {
        let slots: mlua::Table = rt.lua.globals().get("captionSlots").unwrap();
        for (i, t) in slots.sequence_values::<mlua::Table>().enumerate() {
            let id = lua::value_id(&rt.lua, &mlua::Value::Table(t.unwrap())).unwrap();
            update_native_slot_cooldown(
                &rt.lua,
                id,
                if i == 2 { 16 } else { 15 },
                Sample {
                    remaining_ms: 900,
                    total_ms: 1000,
                },
                |_, _| {
                    Some(Sample {
                        remaining_ms: 8000,
                        total_ms: 12000,
                    })
                },
            );
        }
        let slots: mlua::Table = rt.lua.globals().get("perimeterSlots").unwrap();
        for (i, t) in slots.sequence_values::<mlua::Table>().enumerate() {
            let id = lua::value_id(&rt.lua, &mlua::Value::Table(t.unwrap())).unwrap();
            update_native_slot_cooldown(&rt.lua, id, 15, Sample::default(), |_, _| {
                Some(Sample {
                    remaining_ms: [9375, 8000, 5000, 8000][i],
                    total_ms: 10000,
                })
            });
        }
    }
}
fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: native slot cooldown selector"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(fake::config()),
        })
        .insert_resource(aa_ui::UiSetup::new(|ui| {
            lua::slot::set_native_cooldown_mode(&ui.lua, true);
            original::install(ui, true);
            ui.exec(
                "@selector_caption",
                r#"
   panel.slotTitle:SetText('Native selector: global / retained accessor / kind16')
   panel:SetExtent(640,420);panel.slotTitle:SetExtent(620,24)
   local title=panel:CreateChildWidget('label','perimeterTitle',0,true)
   title:SetExtent(620,24);title:AddAnchor('TOP',panel,0,265);title.style:SetFontSize(16)
   title:SetText('Native mask + edge: 1/16 / 1/5 / 1/2 / rectangular')
   perimeterSlots={}
   local old=AAUI.SlotContent
   AAUI.SlotContent=function(kind,index)
     if index>=4 then return {type='skill',id=index,icon='ui/icon/icon_skill_basic01.dds'} end
     return old(kind,index)
   end
   for i=1,4 do
     local slot=CreateActionSlot(panel,'perimeterAction'..i,ISLOT_ACTION,i+3)
     slot:RemoveAllAnchors();slot:AddAnchor('TOP',panel,(i-2.5)*130,308)
     if i==4 then slot:SetExtent(58,72) end
     slot:Show(true);perimeterSlots[i]=slot
   end
  "#,
            )
            .unwrap();
            let id =
                lua::value_id(&ui.lua, &ui.lua.load("captionSlots[2]").eval().unwrap()).unwrap();
            update_native_slot_cooldown(&ui.lua, id, 15, Sample::default(), |_, _| {
                Some(Sample {
                    remaining_ms: 8000,
                    total_ms: 12000,
                })
            });
        }))
        .add_systems(Startup, fake::physical_pixels)
        .add_systems(Update, samples.after(aa_ui::UiInput).before(aa_ui::UiFrame))
        .run();
}
