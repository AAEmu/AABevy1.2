//! Native send escaping and message insertion decoding across the original chat script.

#[path = "../examples/fake/mod.rs"]
#[allow(dead_code)]
mod fake;

use aa_ui::{UiKey, UiRuntime, item_link, lua};

#[test]
fn native_transport_order_preserves_only_lowercase_item_and_quest_prefixes() {
    use lua::chat::encode_for_transport as encode;
    assert_eq!(
        encode("mail@host @@ @@@ | ||"),
        "mail@@host @@@@ @@@@@@ || ||||"
    );
    assert_eq!(
        encode("|cFFAABBCC |r |kName,0; |q2531; |i24488,4,opaque;"),
        "||cFFAABBCC ||r ||kName,0; |q2531; |i24488,4,opaque;"
    );
    // The native replacement restores these prefixes globally, even when malformed or
    // already doubled. It does not recognize uppercase link codes.
    assert_eq!(
        encode("||i ||q |invalid |I1; |Q2;"),
        "|||i |||q |invalid ||I1; ||Q2;"
    );
    assert_eq!(encode("友人@世界 | こんにちは"), "友人@@世界 || こんにちは");
    assert_eq!(
        encode("one\r\ntwo\rthree\nfour\n\rfive"),
        "one two three four  five"
    );
}

#[test]
fn message_insertion_decodes_at_pairs_once_and_keeps_bars_for_glyph_parsing() {
    let ui = UiRuntime::new("en_us", 1920.0, 1080.0).unwrap();
    ui.exec(
        "@message_transport",
        r#"
        message=UIParent:CreateWidget('message','transportMessage','UIParent')
        message:AddMessage('@@@@ @@@ @@ @ ||q2531;')
        message:AddMessageRefresh('@@@@ @@@ @@ @ ||kFake,0;')
    "#,
    )
    .unwrap();
    let id = lua::value_id(&ui.lua, &ui.lua.globals().get("message").unwrap()).unwrap();
    let mut state = lua::ui(&ui.lua);
    let message = state.tree.widget(id).unwrap().message.as_ref().unwrap();
    assert_eq!(message.lines.len(), 1);
    let text = message.lines[0].0.clone();
    assert_eq!(text, "@@ @@ @ @ ||kFake,0;");
    let rendered = state.fonts.layout("default", 16.0, &text, None, 0.0, false);
    let literal = state
        .fonts
        .layout_edit("default", 16.0, "@@ @@ @ @ |kFake,0;", None, 0.0);
    assert_eq!(rendered.width, literal.width);
    assert!(rendered.links.is_empty());
}

#[test]
fn original_input_and_chat_message_keep_literal_codes_and_real_link_payloads() {
    let mut ui = UiRuntime::new("en_us", 1920.0, 1080.0).unwrap();
    fake::install(&mut ui, "en_us", &fake::default_worlds());
    fake::install_ingame(&mut ui);
    ui.run_stage(aa_ui::Stage::World);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    let win = lua::ui(&ui.lua).tree.of_class("chatwindow")[0];
    ui.lua
        .globals()
        .set("win", lua::object_table(&ui.lua, win).unwrap())
        .unwrap();
    let record = item_link::ItemRecord {
        item_type: 24488,
        grade: 4,
        count: 1,
        id: 9007199254740997,
        ..Default::default()
    };
    let item = item_link::format(&item_link::pack(&record));
    ui.lua.globals().set("itemLink", item.clone()).unwrap();
    let hook = ui
        .lua
        .create_function(|lua, (channel, text, target): (i64, String, String)| {
            assert_eq!(channel, 0);
            assert_eq!(target, "");
            lua.globals().set("sentRaw", text.clone())?;
            lua.globals()
                .set("sentWire", lua::chat::encode_for_transport(&text))
        })
        .unwrap();
    ui.lua
        .globals()
        .get::<mlua::Table>("AAUI")
        .unwrap()
        .set("ChatSend", hook)
        .unwrap();
    ui.exec(
        "@original_transport_input",
        r#"
        AAUI.ChatLink=function(kind,id,grade)
            if kind=='item' and id==24488 then return {name='Crushing Grip'} end
            if kind=='quest' and id==2531 then return {name='The Prophet Terrien'} end
        end
        edit=win:GetChatEdit(); edit:Show(true); edit:SetFocus()
        edit:SetText('mail@host @@ |cFFFF0000 |kFake,0; |r ')
        assert(AddLinkItem(itemLink))
        assert(X2Chat:AddQuestLinkToActiveChatInput('|q2531;'))
    "#,
    )
    .unwrap();
    ui.key(&UiKey::Enter);
    let raw: String = ui.lua.globals().get("sentRaw").unwrap();
    let wire: String = ui.lua.globals().get("sentWire").unwrap();
    assert_eq!(
        raw,
        format!("mail@host @@ |cFFFF0000 |kFake,0; |r {item}|q2531;")
    );
    assert_eq!(
        wire,
        format!("mail@@host @@@@ ||cFFFF0000 ||kFake,0; ||r {item}|q2531;")
    );
    ui.fire_event(
        "CHAT_MESSAGE",
        (0, "0", false, "Socialpeerone", wire.clone(), true, ""),
    );
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    let mut state = lua::ui(&ui.lua);
    let message = state.tree.widget(win).unwrap().chat.as_ref().unwrap().tabs[0].message;
    let stored = state
        .tree
        .widget(message)
        .unwrap()
        .message
        .as_ref()
        .unwrap()
        .lines
        .last()
        .unwrap()
        .0
        .clone();
    assert!(stored.ends_with(&wire.replace("@@", "@")));
    let rendered = state
        .fonts
        .layout("default", 16.0, &stored, None, 0.0, false);
    let expected = state.fonts.layout_edit(
        "default", 16.0,
        "[Socialpeerone]: mail@host @@ |cFFFF0000 |kFake,0; |r [Crushing Grip][The Prophet Terrien]",
        None, 0.0,
    );
    assert_eq!(rendered.width, expected.width);
    assert_eq!(rendered.glyphs.len(), expected.glyphs.len());
    assert_eq!(rendered.links.len(), 3);
    assert!(
        matches!(&rendered.links[0].payload, aa_ui::text::ChatLink::Character{name,..} if name=="Socialpeerone")
    );
    assert!(
        matches!(&rendered.links[1].payload, aa_ui::text::ChatLink::Item{encoded,..} if encoded==item.splitn(3,',').nth(2).unwrap().trim_end_matches(';'))
    );
    assert_eq!(
        rendered.links[2].payload,
        aa_ui::text::ChatLink::Quest { quest_type: 2531 }
    );
}
