//! Original friend/mail views with native selection, clipboard, wrapping and scrolling.
//! The fixture supplies clipboard hooks, so automatic captures never modify the OS clipboard.
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
                local clipboard=''
                AAUI.GetClipboard=function() return clipboard end
                AAUI.SetClipboard=function(text) clipboard=text end
                friends = SetViewOfRelationshipFrame('selectionFriends', UIParent)
                friends:RemoveAllAnchors(); friends:AddAnchor('TOPLEFT', UIParent, 65, 130)
                friends:Show(true)
                local input = friends.tab.window[1].inputName
                input:SetText('Newfriend'); input.guideText:Show(false); input:SetFocus()
                mail = SetViewOfWriteMailWindow('selectionMail', UIParent)
                mail:RemoveAllAnchors(); mail:AddAnchor('TOPRIGHT', UIParent, -65, 130)
                mail:Show(true)
                mail.titleEdit:SetText('A letter from Solzreed Peninsula')
                mail.content:Show(true)
                mail.content:SetText('Dear Newfriend,\n\nThe recipient was copied from the original friends edit and pasted here through native clipboard shortcuts.\n\nDragging selects characters, Shift extends the range, and Home or End moves within a visual line.\n\nLong letters scroll to keep the caret visible while the paper, inset and attachment frame stay in place.\n\nLiteral markup stays editable: |cffffffffhello|r.\n\nSee you in Crescent Throne!\nRefone\n')
                input:SetFocus()
            "#).unwrap();
            ui.key(&UiKey::Copy);
            ui.exec("@mail_recipient_focus", "mail.roleEdit:SetFocus()").unwrap();
            ui.key(&UiKey::Paste);
            assert_eq!(ui.lua.load("return mail.roleEdit:GetText()").eval::<String>().unwrap(), "Newfriend");
            let content: mlua::Value = ui.lua.load("return mail.content").eval().unwrap();
            ui.set_focus(aa_ui::lua::value_id(&ui.lua, &content));
            ui.key(&UiKey::Up);
            ui.key(&UiKey::Home);
            ui.key_event(KeyCode::ShiftLeft, true, None);
            for _ in 0..4 { ui.key(&UiKey::Up); }
            ui.key_event(KeyCode::ShiftLeft, false, None);
            let whispered: i64 = ui.lua.load("return CHAT_WHISPERED").eval().unwrap();
            ui.fire_event("CHAT_MESSAGE", (whispered, "peer", false, "Socialpeerone",
                "The original chat name link keeps its click payload.", true, ""));
            fake::report(ui, "native_edits");
        }))
        .add_systems(Startup, fake::physical_pixels)
        .add_systems(Update, fake::scripted_input)
        .run();
}
