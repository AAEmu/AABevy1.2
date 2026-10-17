//! Native character links: display text, wrapped bounds and original chat right-click payload.

#[path = "../examples/fake/mod.rs"]
#[allow(dead_code)]
mod fake;

use aa_ui::{UiRuntime, lua};
use mlua::ObjectLike;

fn eval<T: mlua::FromLua>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().expect(code)
}

fn id(ui: &UiRuntime) -> aa_ui::tree::Id {
    lua::value_id(&ui.lua, &eval(ui, "message")).unwrap()
}

const MESSAGE: &str = r#"
    message=UIParent:CreateWidget('message','linkMessage','UIParent')
    message:AddAnchor('TOPLEFT',UIParent,30,40); message:SetExtent(190,100)
    message:SetInset(7,3,9,5); message.style:SetFontSize(16); message:Show(true)
"#;

fn setup() -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1920.0, 1080.0).unwrap();
    ui.exec("@message", MESSAGE).unwrap();
    ui
}

/// Centers of the linked segments in the painted message, transformed into pointer UI units.
fn centers(ui: &UiRuntime, raw: &str) -> Vec<(f32, f32)> {
    let message = id(ui);
    let mut u = lua::ui(&ui.lua);
    let rect = u.tree.rect(message);
    let view = u.animations.widget_rect(&u.tree, message);
    let text = u.tree.widget(message).unwrap().text.clone().unwrap();
    let s = u.ui_scale;
    let [left, top, right, bottom] = text.inset;
    let (width, height) = (rect.w - left - right, rect.h - top - bottom);
    let lo = u.fonts.layout(
        &text.style.font,
        text.style.size * s,
        raw,
        Some(width * s),
        text.line_space * s,
        false,
    );
    let fx = text.style.align.fx();
    let ox = ((rect.x + left) * s + (width * s - lo.width) * fx).round();
    let oy = ((rect.y + top) * s + height * s - lo.height).round(); // compose bottom-aligns messages.
    lo.character_links
        .iter()
        .map(|link| {
            let dx = ((lo.width - lo.line_widths[link.line]) * fx).round();
            let x = (ox + dx + (link.rect[0] + link.rect[2]) * 0.5) / s;
            let y = (oy + (link.rect[1] + link.rect[3]) * 0.5) / s;
            (
                view.x + (x - rect.x) * view.w / rect.w,
                view.y + (y - rect.y) * view.h / rect.h,
            )
        })
        .collect()
}

fn point(ui: &UiRuntime, p: (f32, f32)) {
    lua::ui(&ui.lua).cursor = p;
}

#[test]
fn character_codes_display_only_names_and_preserve_colored_wrapped_payloads() {
    let mut fonts = aa_ui::text::FontLib::new("en_us");
    let raw = "|cffff0000[|kSocialpeerone,0;]|r says hello |kHostilepeer,1;";
    assert_eq!(
        aa_ui::text::expand_codes(raw),
        "|cffff0000[Socialpeerone]|r says hello Hostilepeer"
    );
    let linked = fonts.layout("default", 16.0, raw, Some(80.0), 3.0, false);
    let plain = fonts.layout(
        "default",
        16.0,
        "|cffff0000[Socialpeerone]|r says hello Hostilepeer",
        Some(80.0),
        3.0,
        false,
    );
    assert_eq!(linked.width, plain.width);
    assert_eq!(linked.height, plain.height);
    assert_eq!(linked.glyphs.len(), plain.glyphs.len());
    assert!(
        linked
            .character_links
            .iter()
            .any(|l| l.name == "Socialpeerone" && !l.is_hostile)
    );
    assert!(
        linked
            .character_links
            .iter()
            .any(|l| l.name == "Hostilepeer" && l.is_hostile)
    );
    assert!(linked.character_links.len() > 2); // Long names wrap into multiple clickable segments.
    let literal = fonts.layout_edit("default", 16.0, raw, None, 0.0);
    assert!(literal.character_links.is_empty());
    assert!(literal.width > plain.width);
    assert_eq!(aa_ui::text::expand_codes("|k友人,0;"), "友人");
    assert_eq!(
        aa_ui::text::expand_codes("|kbad,missing; |kbroken"),
        "bad,missing |kbroken"
    );
}

#[test]
fn link_picking_tracks_wrapped_alignment_insets_and_inherited_scale() {
    let ui = setup();
    let raw = "prefix [|kLongcharactername,1;] then plain text";
    ui.exec(
        "@wrapped",
        "message:SetWidth(85); message.style:SetAlign(ALIGN_RIGHT); message:SetScale(1.5)",
    )
    .unwrap();
    let message: mlua::Table = eval(&ui, "message");
    message.call_method::<()>("AddMessage", raw).unwrap();
    let linked = centers(&ui, raw);
    assert!(linked.len() > 1);
    for p in linked {
        point(&ui, p);
        assert!(eval::<bool>(
            &ui,
            "local l=message:GetLinkInfoOnCursor(); return l.linkType=='character' and l.characterName=='Longcharactername' and l.isHostile"
        ));
    }
    point(&ui, (0.0, 0.0));
    assert_eq!(
        eval::<String>(&ui, "message:GetLinkInfoOnCursor().linkType"),
        "none"
    );
    ui.exec("@hidden", "message:Show(false)").unwrap();
    point(&ui, centers(&ui, raw)[0]);
    assert_eq!(
        eval::<String>(&ui, "message:GetLinkInfoOnCursor().linkType"),
        "none"
    );
}

#[test]
fn scrolling_timeout_and_clear_do_not_leave_stale_click_targets() {
    let ui = setup();
    ui.exec("@lines", "message:SetHeight(25); message:AddMessage('|kOldfriend,0;'); message:AddMessage('plain newest')").unwrap();
    let p = centers(&ui, "|kOldfriend,0;")[0];
    point(&ui, p);
    assert_eq!(
        eval::<String>(&ui, "message:GetLinkInfoOnCursor().linkType"),
        "none"
    );
    ui.exec("@old", "message:ScrollToTop()").unwrap();
    assert_eq!(
        eval::<String>(&ui, "message:GetLinkInfoOnCursor().characterName"),
        "Oldfriend"
    );
    ui.exec("@expire","message:ScrollToBottom(); message:Clear(); message:SetTimeVisible(0.1); message:AddMessage('|kOldfriend,0;')").unwrap();
    lua::tick(&ui.lua, 101.0);
    assert_eq!(
        eval::<String>(&ui, "message:GetLinkInfoOnCursor().linkType"),
        "none"
    );
    ui.exec(
        "@clear",
        "message:SetTimeVisible(10); message:AddMessage('|kOldfriend,0;'); message:Clear()",
    )
    .unwrap();
    assert_eq!(
        eval::<String>(&ui, "message:GetLinkInfoOnCursor().linkType"),
        "none"
    );
}

#[test]
fn original_chat_message_and_right_click_keep_character_name_and_hostility() {
    let mut ui = UiRuntime::new("en_us", 1920.0, 1080.0).unwrap();
    ui.load_ui_texts(std::path::Path::new(fake::GAME_DB))
        .unwrap();
    fake::install(&mut ui, "en_us", &fake::default_worlds());
    fake::install_ingame(&mut ui);
    ui.run_stage(aa_ui::Stage::World);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    ui.exec("@message", MESSAGE).unwrap();
    ui.exec(
        "@original_handlers",
        r#"
        AddItemLinkHandlerToMessageWidget(message)
        ActivatePopupMenu=function(self,name,hostile) clicked={self,name,hostile} end
        local dispatch=X2Chat.DispatchChatMessage
        X2Chat.DispatchChatMessage=function(self,filter,text,...)
            raw=text; message:AddMessageRefresh(text); return dispatch(self,filter,text,...)
        end
    "#,
    )
    .unwrap();
    let channel: i64 = eval(&ui, "CHAT_WHISPERED");
    for hostile in [false, true] {
        ui.fire_event(
            "CHAT_MESSAGE",
            (
                channel,
                "peer",
                hostile,
                "Socialpeerone",
                "Hello from the original event",
                true,
                "",
            ),
        );
        let raw: String = eval(&ui, "raw");
        assert!(
            raw.contains(if hostile {
                "|kSocialpeerone,1;"
            } else {
                "|kSocialpeerone,0;"
            }),
            "{raw}"
        );
        assert!(!aa_ui::text::expand_codes(&raw).contains("Socialpeerone,0"));
        assert!(!aa_ui::text::expand_codes(&raw).contains("Socialpeerone,1"));
        let p = centers(&ui, &raw)[0];
        aa_ui::input::update(
            &ui.lua,
            &mut Default::default(),
            Some(p),
            false,
            false,
            true,
            0.0,
        );
        assert!(eval::<bool>(
            &ui,
            "clicked[1]==message and clicked[2]=='Socialpeerone'"
        ));
        assert_eq!(eval::<bool>(&ui, "clicked[3]"), hostile);
    }
}
