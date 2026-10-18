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
