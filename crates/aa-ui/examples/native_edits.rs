//! Original friend/mail views with native edit selection, wrapping and scrolling.
//! `cargo run -p aa-ui --example native_edits -- --shot shots/aa-ui/native-edits.png --shot-after 5`

#[path = "fake/mod.rs"]
#[allow(dead_code)]
mod fake;

use aa_ui::UiKey;
use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins(aa_dev::default_plugins("aa-ui: original friend and mail edits"))
        .add_plugins(aa_dev::DevPlugin)
        .add_plugins(aa_ui::UiPlugin { config: Some(fake::config()) })
        .insert_resource(aa_ui::UiSetup::new(|ui| {
            fake::install(ui, "en_us", &fake::default_worlds());
            fake::install_ingame(ui);
            ui.run_stage(aa_ui::Stage::World);
            fake::enter_world(ui);
            ui.exec("@original_edit_views", r#"
                friends = SetViewOfRelationshipFrame('selectionFriends', UIParent)
                friends:RemoveAllAnchors(); friends:AddAnchor('TOPLEFT', UIParent, 65, 130)
                friends:Show(true)
                local input = friends.tab.window[1].inputName
                input:SetText('Oldfriend'); input.guideText:Show(false); input:SetFocus()
                mail = SetViewOfWriteMailWindow('selectionMail', UIParent)
                mail:RemoveAllAnchors(); mail:AddAnchor('TOPRIGHT', UIParent, -65, 130)
                mail:Show(true)
                mail.roleEdit:SetText('Newfriend')
                mail.titleEdit:SetText('A letter from Solzreed Peninsula')
                mail.content:Show(true)
                mail.content:SetText('Dear Newfriend,\n\nThis original mail edit wraps its text at the authored inset. Dragging selects characters, Shift extends the range, and Home or End moves within a visual line.\n\nLong letters scroll to keep the caret visible while the paper, inset and attachment frame stay in place.\n\nLiteral markup stays editable: |cffffffffhello|r.\n\nSee you in Crescent Throne!\nRefone\n')
            "#).unwrap();
            let content: mlua::Value = ui.lua.load("return mail.content").eval().unwrap();
            ui.set_focus(aa_ui::lua::value_id(&ui.lua, &content));
            ui.key(&UiKey::Up);
            ui.key(&UiKey::Home);
            ui.key_event(KeyCode::ShiftLeft, true, None);
            for _ in 0..4 { ui.key(&UiKey::Up); }
            ui.key_event(KeyCode::ShiftLeft, false, None);
            fake::report(ui, "native_edits");
        }))
        .add_systems(Startup, fake::physical_pixels)
        .add_systems(Update, fake::scripted_input)
        .run();
}
