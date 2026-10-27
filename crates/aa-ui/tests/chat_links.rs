//! Native chat links: display text, wrapped bounds and original click/tooltip payloads.

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
    lo.links
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
            .links
            .iter()
            .any(|l| matches!(&l.payload,aa_ui::text::ChatLink::Character {name,is_hostile:false} if name=="Socialpeerone"))
    );
    assert!(
        linked
            .links
            .iter()
            .any(|l| matches!(&l.payload,aa_ui::text::ChatLink::Character {name,is_hostile:true} if name=="Hostilepeer"))
    );
    assert!(linked.links.len() > 2); // Long names wrap into multiple clickable segments.
    let literal = fonts.layout_edit("default", 16.0, raw, None, 0.0);
    assert!(literal.links.is_empty());
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
    ui.load_ui_texts(std::path::Path::new(&fake::game_db()))
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

#[test]
fn item_quest_labels_colors_raw_payloads_and_missing_lookups() {
    let ui = setup();
    ui.exec("@lookup",r#"
        lookups=0
        AAUI.ChatLink=function(kind,id,grade)
            lookups=lookups+1
            assert(message:GetText()~=nil) -- Callback may read UI; no Ui appdata borrow is held.
            if kind=='item' and id==24488 then assert(grade==4); return {name='Linked Sword',color={0.2,0.8,0.3,1}} end
            if kind=='quest' and id==2531 then assert(grade==nil); return {name='The Prophet Terrien',color={1,0.5,0.1,1}} end
        end
        raw='|cffabcdefbefore |i24488,4,opaqueInstance; after |q2531; tail |q999999;'
        message:SetExtent(600,100); message:AddMessage(raw)
    "#).unwrap();
    let message = id(&ui);
    let raw: String = eval(&ui, "raw");
    let lo = {
        let mut u = lua::ui(&ui.lua);
        u.fonts
            .layout("default", 16.0, &raw, Some(584.0), 0.0, false)
    };
    assert_eq!(eval::<u32>(&ui, "lookups"), 3);
    assert_eq!(lo.links.len(), 2);
    let original = lua::ui(&ui.lua)
        .tree
        .widget(message)
        .unwrap()
        .message
        .as_ref()
        .unwrap()
        .lines[0]
        .0
        .clone();
    assert_eq!(original, raw);
    assert!(
        lo.glyphs
            .iter()
            .any(|g| g.color.is_some_and(|c| (c[0] - 0.2).abs() < 0.001))
    );
    let last = lo.glyphs.last().unwrap().color.unwrap();
    assert!((last[0] - 171.0 / 255.0).abs() < 0.001); // Enclosing chat color restored after links.
    let points = centers(&ui, &raw);
    point(&ui, points[0]);
    assert!(eval::<bool>(
        &ui,
        "local i=message:GetLinkInfoOnCursor(); return i.linkType=='item' and i.itemType==24488 and i.itemGrade==4 and i.itemLinkText=='opaqueInstance'"
    ));
    point(&ui, points[1]);
    assert!(eval::<bool>(
        &ui,
        "local q=message:GetLinkInfoOnCursor(); return q.linkType=='quest' and q.questType==2531"
    ));
    assert_eq!(eval::<u32>(&ui, "lookups"), 3); // Font rendering/picking never calls Lua lookup.
}

#[test]
fn item_payload_bounds_and_native_visible_link_capacity() {
    let ui = setup();
    ui.exec(
        "@bounds",
        r#"
        AAUI.ChatLink=function() return {name='A'} end
        message:SetExtent(190,1000)
        message:AddMessage('|i1,1,'..string.rep('a',382)..';')
    "#,
    )
    .unwrap();
    let first = centers(&ui, &format!("|i1,1,{};", "a".repeat(382)));
    assert_eq!(first.len(), 1);
    ui.exec(
        "@too_long",
        "message:Clear(); message:AddMessage('|i1,1,'..string.rep('a',383)..';')",
    )
    .unwrap();
    assert!(centers(&ui, &format!("|i1,1,{};", "a".repeat(383))).is_empty());
    ui.exec(
        "@malformed_prefix",
        "message:AddMessage('|iBROKEN |q2531;')",
    )
    .unwrap();
    assert_eq!(centers(&ui, "|iBROKEN |q2531;").len(), 1);
    let raw = "|kA,0; ".repeat(151);
    let lo = lua::ui(&ui.lua)
        .fonts
        .layout("default", 16.0, &raw, None, 0.0, false);
    assert_eq!(lo.links.len(), 150);
}

#[test]
fn escaped_link_codes_stay_literal_without_lookup_or_click_targets() {
    let ui = setup();
    ui.exec(
        "@escaped",
        r#"
        AAUI.ChatLink=function() error('escaped codes must not resolve') end
        message:AddMessage('||i24488,4,opaque; ||q2531; ||kName,0; ||cffffffff')
    "#,
    )
    .unwrap();
    let mut u = lua::ui(&ui.lua);
    let escaped = u.fonts.layout(
        "default",
        16.0,
        "||i24488,4,opaque; ||q2531; ||kName,0; ||cffffffff",
        None,
        0.0,
        false,
    );
    let literal = u.fonts.layout_edit(
        "default",
        16.0,
        "|i24488,4,opaque; |q2531; |kName,0; |cffffffff",
        None,
        0.0,
    );
    assert_eq!(escaped.width, literal.width);
    assert_eq!(escaped.glyphs.len(), literal.glyphs.len());
    assert!(escaped.links.is_empty());
}

#[test]
fn original_item_and_quest_left_click_open_original_tooltip_views() {
    let mut ui = UiRuntime::new("en_us", 1920.0, 1080.0).unwrap();
    ui.load_ui_texts(std::path::Path::new(&fake::game_db()))
        .unwrap();
    fake::install(&mut ui, "en_us", &fake::default_worlds());
    fake::install_ingame(&mut ui);
    ui.run_stage(aa_ui::Stage::World);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    ui.exec("@message", MESSAGE).unwrap();
    ui.exec("@native_tooltip_dependencies",r#"
        AddItemLinkHandlerToMessageWidget(message)
        originalLinks={}
        local original,index
        for i=1,20 do
            local name,v=debug.getupvalue(DebugShowLootDiceWindow,i)
            if type(v)=='function' and name=='LootDiceEventHandler' then original,index=v,i; break end
        end
        assert(index~=nil)
        debug.setupvalue(DebugShowLootDiceWindow,index,function(key,time,link) table.insert(originalLinks,link) end)
        DebugShowLootDiceWindow()
        debug.setupvalue(DebugShowLootDiceWindow,index,original)
        AAUI.ChatLink=function(kind,id,grade)
            if kind=='item' then return {name='Original Item Link',color={1,0.7,0.2,1}} end
            if kind=='quest' then return {name=X2Quest:GetQuestContextMainTitle(id),color={1,1,1,1}} end
        end
        -- Application tooltip data is a known bag fixture; encoded bytes remain opaque to UI.
        X2Item.InfoFromLink=function(self,payload) requestedPayload=payload; return X2Bag:GetBagItemInfo(1,1,true) end
        X2Quest.GetLinkedQuestSummary=function(self,id) return X2Quest:GetQuestContextSummary(id) end
        X2Quest.GetLinkedQuestObjectives=function(self,id) return {} end
        message:AddMessageRefresh(originalLinks[1])
    "#).unwrap();
    let raw: String = eval(&ui, "originalLinks[1]");
    let p = centers(&ui, &raw)[0];
    aa_ui::input::update(
        &ui.lua,
        &mut Default::default(),
        Some(p),
        true,
        false,
        false,
        0.0,
    );
    aa_ui::input::update(
        &ui.lua,
        &mut Default::default(),
        Some(p),
        false,
        false,
        true,
        0.0,
    );
    assert!(eval::<bool>(&ui, "requestedPayload==nil")); // Right-click does not open item tooltip.
    let mut mouse = Default::default();
    aa_ui::input::update(&ui.lua, &mut mouse, Some(p), true, false, false, 0.0);
    aa_ui::input::update(&ui.lua, &mut mouse, Some(p), false, true, false, 0.0);
    let encoded = raw
        .splitn(3, ',')
        .nth(2)
        .unwrap()
        .strip_suffix(';')
        .unwrap();
    assert_eq!(eval::<String>(&ui, "requestedPayload"), encoded);
    assert!(eval::<bool>(&ui, "tooltip.tooltipWindows[4]:IsVisible()"));
    ui.exec(
        "@quest",
        "tooltip.tooltipWindows[4]:Show(false); message:AddMessageRefresh('|q2531;')",
    )
    .unwrap();
    let p = centers(&ui, "|q2531;")[0];
    aa_ui::input::update(&ui.lua, &mut mouse, Some(p), true, false, false, 0.0);
    aa_ui::input::update(&ui.lua, &mut mouse, Some(p), false, true, false, 0.0);
    assert!(eval::<bool>(&ui, "tooltip.tooltipWindows[8]:IsVisible()"));
}
