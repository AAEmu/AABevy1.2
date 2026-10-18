//! Original friend/mail views with deterministic IME events; never changes OS input language.
//! `cargo run -p aa-ui --example native_ime -- --shot shots/aa-ui/native-ime.png --shot-after 5`

#[path = "fake/mod.rs"]
#[allow(dead_code)]
mod fake;

use bevy::{prelude::*, window::Ime};

fn main() {
    App::new()
        .add_plugins(aa_dev::default_plugins("aa-ui: original edits with native IME"))
        .add_plugins(aa_dev::DevPlugin)
        .add_plugins(aa_ui::UiPlugin { config: Some(fake::config()) })
        .insert_resource(aa_ui::UiSetup::new(|ui| {
            fake::install(ui, "en_us", &fake::default_worlds());
            fake::install_ingame(ui);
            ui.run_stage(aa_ui::Stage::World);
            fake::enter_world(ui);
            ui.exec("@original_ime_views", r#"
                friends=SetViewOfRelationshipFrame('imeFriends',UIParent)
                friends:RemoveAllAnchors(); friends:AddAnchor('TOPLEFT',UIParent,65,130)
                friends:Show(true)
                local input=friends.tab.window[1].inputName
                input:SetText('Oldfriend'); input.guideText:Show(false); input:SetFocus()
                mail=SetViewOfWriteMailWindow('imeMail',UIParent)
                mail:RemoveAllAnchors(); mail:AddAnchor('TOPRIGHT',UIParent,-65,130)
                mail:Show(true)
                mail.roleEdit:SetText('Newfriend')
                mail.titleEdit:SetText('A letter composed in Solzreed')
                mail.content:Show(true)
                mail.content:SetText('Dear Newfriend,\n\nThe friends input accepted a composition result. This original mail edit is showing a live preedit buffer.\n\nGetText and OnTextChanged see the preview, while physical Enter confirms composition before it can send chat.\n\n')
                input:SetFocus()
            "#).unwrap();
            let window=Entity::PLACEHOLDER;
            ui.ime_event(&Ime::Enabled {window});
            ui.ime_event(&Ime::Preedit {window,value:"Newfriend".into(),cursor:Some((9,9))});
            ui.ime_event(&Ime::Commit {window,value:"Newfriend".into()});
            assert_eq!(ui.lua.load("friends.tab.window[1].inputName:GetText()").eval::<String>().unwrap(),"Newfriend");
            let content:mlua::Value=ui.lua.load("mail.content").eval().unwrap();
            ui.set_focus(aa_ui::lua::value_id(&ui.lua,&content));
            ui.ime_event(&Ime::Enabled {window});
            ui.ime_event(&Ime::Preedit {window,value:"Welcome to Solzreed Peninsula.".into(),cursor:Some((19,19))});
            assert!(ui.lua.load("string.find(mail.content:GetText(),'Welcome to Solzreed Peninsula.',1,true)~=nil").eval::<bool>().unwrap());
            fake::report(ui,"native_ime");
        }))
        .add_systems(Startup,fake::physical_pixels)
        .add_systems(Update,fake::scripted_input)
        .run();
}
