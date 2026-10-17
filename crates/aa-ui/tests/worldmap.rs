//! The world map widget (`aa_ui::lua::worldmap`) without a window: the maps the scripts register
//! (`SaveMapData`, with the real `image_map.tga` masks), the player's map and cursor from
//! `AAUI.WorldMap`, hover and click zooming through the masks, the scroll offset of `mapWindow`.

use aa_ui::UiRuntime;
use aa_ui::input::{MouseState, update};

fn runtime() -> UiRuntime {
    UiRuntime::new("en_us", 1920.0, 1080.0).expect("runtime")
}

fn eval<T: mlua::FromLua>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().expect(code)
}

/// A world map with four registered maps (the scripts' own parameters from
/// `map/world_map_list.lua`): world 2, continent 3 (Nuia), zone 5 (Solzreed Peninsula) and
/// city 335 (Crescent Throne), the player at 0.5, 0.25 of the zone map.
const SETUP: &str = r#"
    frame = UIParent:CreateWidget("emptywidget", "wmFrame", "UIParent")
    frame:AddAnchor("TOPLEFT", "UIParent", 0, 0)
    frame:SetExtent(928, 556)
    frame:Show(true)
    wm = frame:CreateChildWidget("worldmap", "wm", 0, true)
    wm:SetExtent(928, 556)
    wm:AddAnchor("TOPLEFT", frame, 0, 0)
    wm:SetHandler("OnEnter", function() end)
    wm:Show(true)
    player = wm:CreateImageDrawable("ui/map/icon/player_cursor.dds", "overlay")
    player:SetExtent(32, 32)
    wm:SetPlayerDrawable(player)
    events = {}
    wm:SetHandler("OnEvent", function(self, ev, ...)
        local a = {}
        for i, v in ipairs({ ... }) do a[i] = tostring(v) end
        events[#events + 1] = ev .. ":" .. table.concat(a, ",")
    end)
    wm:RegisterEvent("UPDATE_ZONE_LEVEL_INFO")
    wm:RegisterEvent("SET_DEFAULT_EXPAND_RATIO")
    local tga = "Game/ui/map/image_map.tga"
    maps, overs = {}, {}
    local function reg(id, level, r, g, b, mask, ox, oy, cx, cy, cw, ch, vx, vy, vw, vh)
        local img = wm:CreateImageDrawable("Game/ui/map/world/main_world.dds", "background")
        img:SetCoords(0, 0, 1856, 1112)
        img:AddAnchor("TOPLEFT", wm, 0, 0)
        img:AddAnchor("BOTTOMRIGHT", wm, 0, 0)
        local over = wm:CreateImageDrawable("Game/ui/map/world/land_west_over.dds", "background")
        over:SetExtent(vw, vh)
        over:AddAnchor("TOPLEFT", wm, vx, vy)
        -- the scripts swap red and blue (AddColorImg)
        wm:SaveMapData(level, id, img, over, 1, mask, { r = b, g = g, b = r, a = 255 }, {
            colorImgOffsetX = ox, colorImgOffsetY = oy,
            colorImgCoordX = cx, colorImgCoordY = cy, colorImgCoordW = cw, colorImgCoordH = ch,
        })
        maps[id], overs[id] = img, over
    end
    reg(2, 0, 0, 0, 0, tga, 244, 53, 0, 0, 108, 116, 0, 0, 928, 556)
    reg(3, 1, 236, 52, 52, tga, 258, 44, 108, 0, 110, 124, 467, 496, 480, 532)
    reg(5, 2, 93, 159, 226, tga, 635, 420, 500, 46, 11, 10, 1151, 357, 272, 156)
    reg(335, 3, 172, 213, 206, "", 0, 0, 0, 0, 0, 0, 1269, 840, 128, 120)
    AAUI.WorldMap = function()
        return { level = 2, zone = 5, player = { u = 0.5, v = 0.25, angle = 1.0 } }
    end
    function shown()
        local s = {}
        for _, id in ipairs({ 2, 3, 5, 335 }) do
            if maps[id]:IsVisible() then s[#s + 1] = id end
        end
        return table.concat(s, ",")
    end
    function overs_shown()
        local s = {}
        for _, id in ipairs({ 2, 3, 5, 335 }) do
            if overs[id]:IsVisible() then s[#s + 1] = id end
        end
        return table.concat(s, ",")
    end
"#;

fn setup() -> UiRuntime {
    let ui = runtime();
    ui.exec("@setup", SETUP).unwrap();
    ui
}

fn tick(ui: &UiRuntime) {
    aa_ui::lua::tick(&ui.lua, 16.0);
}

fn events(ui: &UiRuntime) -> String {
    eval(ui, "table.concat(events, ' ')")
}

fn pair(ui: &UiRuntime, code: &str) -> (f64, f64) {
    let t: mlua::Table = eval(ui, code);
    (t.get(1).unwrap(), t.get(2).unwrap())
}

fn view_pos(ui: &UiRuntime) -> (f64, f64) {
    pair(ui, "local x, y = wm:GetPlayerViewPos() return {x, y}")
}

fn near(a: (f64, f64), b: (f64, f64), tol: f64) {
    assert!(
        (a.0 - b.0).abs() <= tol && (a.1 - b.1).abs() <= tol,
        "{a:?} vs {b:?}"
    );
}

#[test]
fn opens_on_the_players_map_with_the_cursor() {
    let ui = setup();
    assert_eq!(eval::<String>(&ui, "shown()"), "");
    tick(&ui);
    assert_eq!(eval::<String>(&ui, "shown()"), "5");
    assert_eq!(
        events(&ui),
        "UPDATE_ZONE_LEVEL_INFO:2,5 SET_DEFAULT_EXPAND_RATIO:true"
    );
    // The cursor is shown at the player's place (u 0.5, v 0.25 of 928 x 556).
    near(view_pos(&ui), (464.0, 139.0), 0.01);
    assert!(eval::<bool>(&ui, "player:IsVisible()"));
    // At the next expansion the place scales with the widget.
    ui.exec("@size", "wm:SetExtent(1856, 1112)").unwrap();
    near(view_pos(&ui), (928.0, 278.0), 0.01);
    // Hidden worldmaps cost nothing and keep their state.
    ui.exec("@hide", "wm:Show(false)").unwrap();
    ui.exec(
        "@hook",
        "AAUI.WorldMap = function() return { level = 2, zone = 7 } end",
    )
    .unwrap();
    tick(&ui);
    assert_eq!(eval::<String>(&ui, "shown()"), "5");
}

#[test]
fn right_click_zooms_out_to_the_continent_and_the_world() {
    let ui = setup();
    tick(&ui);
    let mut mouse = MouseState::default();
    update(
        &ui.lua,
        &mut mouse,
        Some((300.0, 300.0)),
        false,
        false,
        true,
        0.0,
    );
    tick(&ui);
    assert_eq!(eval::<String>(&ui, "shown()"), "3");
    assert!(events(&ui).ends_with("UPDATE_ZONE_LEVEL_INFO:1,3 SET_DEFAULT_EXPAND_RATIO:false"));
    // The player on the continent map: the zone's highlight rectangle (575.5, 178.5, 136 x 78)
    // at 0.5, 0.25 of it.
    near(view_pos(&ui), (643.5, 198.0), 0.5);
    update(
        &ui.lua,
        &mut mouse,
        Some((300.0, 300.0)),
        false,
        false,
        true,
        0.0,
    );
    tick(&ui);
    assert_eq!(eval::<String>(&ui, "shown()"), "2");
    // On the world map the continent's mask is registered onto its colour in the world mask:
    // the north east tip of Nuia.
    let (x, y) = view_pos(&ui);
    assert!(x > 420.0 && x < 450.0 && y > 330.0 && y < 360.0, "{x} {y}");
    // Nothing above the world.
    update(
        &ui.lua,
        &mut mouse,
        Some((300.0, 300.0)),
        false,
        false,
        true,
        0.0,
    );
    tick(&ui);
    assert_eq!(eval::<String>(&ui, "shown()"), "2");
}

#[test]
fn hover_highlights_and_left_click_shows_the_zone() {
    let ui = setup();
    tick(&ui);
    let mut mouse = MouseState::default();
    // Up to the continent, then hover Solzreed's colour in the continent mask: texel (200, 42)
    // of image_map.tga is at map unit (258 + 4 * 92, 44 + 4 * 42) = (626, 212).
    update(
        &ui.lua,
        &mut mouse,
        Some((300.0, 300.0)),
        false,
        false,
        true,
        0.0,
    );
    tick(&ui);
    update(
        &ui.lua,
        &mut mouse,
        Some((628.0, 214.0)),
        false,
        false,
        false,
        0.0,
    );
    tick(&ui);
    assert_eq!(eval::<String>(&ui, "overs_shown()"), "5");
    // Elsewhere (the sea): no highlight.
    update(
        &ui.lua,
        &mut mouse,
        Some((60.0, 500.0)),
        false,
        false,
        false,
        0.0,
    );
    tick(&ui);
    assert_eq!(eval::<String>(&ui, "overs_shown()"), "");
    // A press and release in place on the zone shows its map.
    update(
        &ui.lua,
        &mut mouse,
        Some((628.0, 214.0)),
        true,
        false,
        false,
        0.0,
    );
    update(
        &ui.lua,
        &mut mouse,
        Some((628.0, 214.0)),
        false,
        true,
        false,
        0.0,
    );
    tick(&ui);
    assert_eq!(eval::<String>(&ui, "shown()"), "5");
    assert_eq!(eval::<String>(&ui, "overs_shown()"), "");
    assert!(events(&ui).ends_with("UPDATE_ZONE_LEVEL_INFO:2,5 SET_DEFAULT_EXPAND_RATIO:true"));
}

#[test]
fn a_drag_does_not_click() {
    let ui = setup();
    tick(&ui);
    let mut mouse = MouseState::default();
    update(
        &ui.lua,
        &mut mouse,
        Some((300.0, 300.0)),
        false,
        false,
        true,
        0.0,
    );
    tick(&ui);
    update(
        &ui.lua,
        &mut mouse,
        Some((628.0, 214.0)),
        true,
        false,
        false,
        0.0,
    );
    // Dragged away over Solzreed's neighbour before the release.
    update(
        &ui.lua,
        &mut mouse,
        Some((520.0, 300.0)),
        false,
        true,
        false,
        0.0,
    );
    tick(&ui);
    assert_eq!(eval::<String>(&ui, "shown()"), "3");
}

#[test]
fn presses_reach_the_parent_window_for_panning() {
    let ui = setup();
    ui.exec(
        "@handlers",
        r#"
        downs, ups = 0, 0
        frame:SetHandler("OnMouseDown", function(self, btn) downs = downs + 1 end)
        frame:SetHandler("OnMouseUp", function(self, btn) ups = ups + 1 end)
        "#,
    )
    .unwrap();
    tick(&ui);
    let mut mouse = MouseState::default();
    update(
        &ui.lua,
        &mut mouse,
        Some((300.0, 300.0)),
        true,
        false,
        false,
        0.0,
    );
    update(
        &ui.lua,
        &mut mouse,
        Some((310.0, 300.0)),
        false,
        true,
        false,
        0.0,
    );
    assert_eq!(eval::<i64>(&ui, "downs"), 1);
    assert_eq!(eval::<i64>(&ui, "ups"), 1);
}

#[test]
fn scroll_offset_moves_the_children_and_clips() {
    let ui = setup();
    ui.exec("@scroll", "frame:EnableScroll(true) frame:ChangeChildAnchorByScrollValue('horz', 100) frame:ChangeChildAnchorByScrollValue('vert', 20)")
        .unwrap();
    near(
        pair(&ui, "local x, y = wm:GetOffset() return {x, y}"),
        (-100.0, -20.0),
        0.01,
    );
    // The frame itself stays; its children are clipped to it.
    near(
        pair(&ui, "local x, y = frame:GetOffset() return {x, y}"),
        (0.0, 0.0),
        0.01,
    );
    let tree_clip = {
        let u = aa_ui::lua::ui(&ui.lua);
        let wm = u.tree.find("wm").unwrap();
        u.tree.clip_rect(wm)
    };
    let c = tree_clip.expect("clipped");
    assert_eq!((c.x, c.y, c.w, c.h), (0.0, 0.0, 928.0, 556.0));
}

const PING: &str = r#"
    ping = wm:CreateChildWidget('window', 'ping', 0, true)
    ping:SetExtent(24, 24); ping:Show(false)
    wm:SetPingWidget(ping)
    requests, removes, pulses = {}, 0, 0
    wm:RegisterEvent('SET_EFFECT_ICON_VISIBLE')
    wm:SetHandler('OnEvent', function(self, ev, show, widget)
        if ev == 'SET_EFFECT_ICON_VISIBLE' and show and widget == ping then pulses = pulses + 1 end
    end)
    echo = false
    AAUI.WorldMap = function()
        return {level=2, zone=5, player={u=.5,v=.25}, ping=echo}
    end
    AAUI.PlaceMapPing = function(level, id, u, v)
        requests[#requests+1] = {level, id, u, v}; return true
    end
    AAUI.RemoveMapPing = function() removes = removes + 1; return true end
"#;

fn ping_pos(ui: &UiRuntime) -> (f64, f64) {
    pair(ui, "local x,y = ping:GetOffset(); return {x+12,y+12}")
}

fn click(ui: &UiRuntime, x: f32, y: f32) {
    let mut mouse = MouseState::default();
    update(&ui.lua, &mut mouse, Some((x, y)), true, false, false, 0.0);
    update(&ui.lua, &mut mouse, Some((x, y)), false, true, false, 0.0);
}

#[test]
fn party_ping_requests_wait_for_authoritative_echo_and_clear() {
    let ui = setup();
    ui.exec("@ping", PING).unwrap();
    tick(&ui);
    ui.exec("@mode", "wm:SetPingBtn(true)").unwrap();
    click(&ui, 232.0, 417.0);
    assert!(!eval::<bool>(&ui, "ping:IsVisible()"));
    assert_eq!(pair(&ui, "return {requests[1][1],requests[1][2]}"), (2., 5.));
    near(pair(&ui, "return {requests[1][3],requests[1][4]}"), (0.25, 0.75), 0.0001);
    {
        let u = aa_ui::lua::ui(&ui.lua);
        assert!(!u.tree.widget(u.tree.find("wm").unwrap()).unwrap().worldmap.as_ref().unwrap().ping_mode);
    }
    ui.exec("@echo", "echo={level=2,id=5,u=.6,v=.4,version=1}").unwrap();
    tick(&ui);
    assert!(eval::<bool>(&ui, "ping:IsVisible()"));
    near(ping_pos(&ui), (557.0, 222.0), 0.01);
    tick(&ui);
    assert_eq!(eval::<i64>(&ui, "pulses"), 1);
    let content = aa_ui::lua::ui(&ui.lua).tree.settle();
    for _ in 0..8 { tick(&ui); }
    assert_eq!(aa_ui::lua::ui(&ui.lua).tree.settle(), content);
    ui.exec("@remove", "wm:RemovePing()").unwrap();
    assert_eq!(eval::<i64>(&ui, "removes"), 1);
    assert!(eval::<bool>(&ui, "ping:IsVisible()"));
    ui.exec("@clear", "echo=false; wm:UpdatePingInfo()").unwrap();
    assert!(!eval::<bool>(&ui, "ping:IsVisible()"));
}

#[test]
fn ping_normalization_uses_displayed_map_after_zoom_pan_and_scale() {
    let ui = setup();
    ui.exec("@ping", PING).unwrap();
    tick(&ui);
    ui.exec("@transform", "wm:SetExtent(1856,1112); frame:EnableScroll(true); frame:ChangeChildAnchorByScrollValue('horz',100); frame:ChangeChildAnchorByScrollValue('vert',20); frame:SetScale(.5); wm:SetPingBtn(true)").unwrap();
    let (x, y) = {
        let u = aa_ui::lua::ui(&ui.lua);
        let r = u.animations.widget_rect(&u.tree, u.tree.find("wm").unwrap());
        (r.x + r.w * 0.25, r.y + r.h * 0.3)
    };
    click(&ui, x, y);
    near(pair(&ui, "return {requests[1][3],requests[1][4]}"), (0.25, 0.3), 0.0001);
    ui.exec("@echo", "echo={level=2,id=5,u=.25,v=.3,maps={{level=1,id=3,u=.7,v=.2}}}; wm:UpdatePingInfo()").unwrap();
    ui.exec("@zoomout", "frame:SetScale(1); wm:SetExtent(928,556); frame:ChangeChildAnchorByScrollValue('horz',0); frame:ChangeChildAnchorByScrollValue('vert',0)").unwrap();
    // Right click changes the map, and its exact coordinate comes from the server projection.
    let mut mouse = MouseState::default();
    update(&ui.lua, &mut mouse, Some((300.,300.)), false, false, true, 0.);
    tick(&ui);
    near(ping_pos(&ui), (650.0,111.0), 0.01);
}

#[test]
fn absent_or_declined_ping_hooks_keep_local_fallback_but_failed_requests_do_not() {
    let ui = setup();
    ui.exec("@ping", PING).unwrap();
    ui.exec("@local", "AAUI.WorldMap=function() return {level=2,zone=5} end; AAUI.PlaceMapPing=nil; AAUI.RemoveMapPing=nil").unwrap();
    tick(&ui);
    ui.exec("@mode", "wm:SetPingBtn(true)").unwrap();
    click(&ui, 232.,417.);
    assert!(eval::<bool>(&ui, "ping:IsVisible()"));
    near(ping_pos(&ui), (232.,417.), 0.01);
    ui.exec("@remove", "wm:RemovePing(); AAUI.PlaceMapPing=function() return false end; wm:SetPingBtn(true)").unwrap();
    assert!(!eval::<bool>(&ui, "ping:IsVisible()"));
    click(&ui, 100.,100.);
    assert!(eval::<bool>(&ui, "ping:IsVisible()"));
    ui.exec("@fail", "wm:RemovePing(); AAUI.PlaceMapPing=function() error('offline') end; wm:SetPingBtn(true)").unwrap();
    click(&ui, 200.,200.);
    assert!(!eval::<bool>(&ui, "ping:IsVisible()"));
}
