//! In-game HUD natives without a window: slot cooldown sweeps, the roadmap's player heading and
//! markers, chat wheel scrolling.

use aa_ui::UiRuntime;
use aa_ui::render::Tex;

fn runtime() -> UiRuntime {
    UiRuntime::new("en_us", 1920.0, 1080.0).expect("runtime")
}

fn eval<T: mlua::FromLua>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().expect(code)
}

fn tick(ui: &UiRuntime, ms: f32) {
    aa_ui::lua::tick(&ui.lua, ms);
}

/// Quads of the sweep: free-corner triangles in the sweep's dark colour.
fn sweep_quads(ui: &UiRuntime) -> usize {
    let mut g = aa_ui::lua::ui(&ui.lua);
    aa_ui::render::build_quads(&mut g, None)
        .iter()
        .filter(|q| {
            q.corners.is_some() && ((q.tex == Tex::Atlas && (q.color[3] - 199.0 / 255.0).abs() < 0.01)
                || matches!(&q.tex, Tex::MaskedImage { mask, .. } if mask.ends_with("cooldown_mask.dds")))
        })
        .count()
}

#[test]
fn slot_cooldown_sweep_counts_down() {
    let ui = runtime();
    ui.exec(
        "@setup",
        r#"
        ISLOT_ACTION = 1
        events = {}
        AAUI.SlotContent = function(slotType, idx)
          return { type = "skill", id = 7, icon = "ui/icon/icon_skill_basic01.dds", cooldown = { remain = 3000, total = 4000 } }
        end
        bar = UIParent:CreateWidget("window", "bar", "UIParent")
        bar:SetExtent(100, 60)
        bar:AddAnchor("TOPLEFT", "UIParent", 100, 100)
        bar:Show(true)
        slot = bar:CreateChildWidget("slot", "slot", 1, true)
        slot:SetExtent(42, 42)
        slot:AddAnchor("TOPLEFT", bar, 5, 5)
        slot:SetHandler("OnContentUpdated", function(self, what, v) events[#events + 1] = what .. ":" .. tostring(v) end)
        slot.cooltime_style:SetShadow(true)
        slot:EstablishSlot(ISLOT_ACTION, 1)
        "#,
    )
    .unwrap();
    // Bound while cooling down: the script is told, the sweep is drawn (a quarter of the time
    // has passed, so three of the four sides of the square are dark: 4 triangles at most).
    assert!(eval::<String>(&ui, "table.concat(events, ' ')").contains("skill_cooldown_start:4000"));
    assert!(eval::<bool>(&ui, "slot.cooltime_style ~= nil"));
    let n = sweep_quads(&ui);
    assert!((1..=5).contains(&n), "{n} sweep triangles");
    // 2 s later it is still going; the clock only moves in ticks.
    tick(&ui, 2000.0);
    assert!(sweep_quads(&ui) >= 1);
    assert!(!eval::<String>(&ui, "table.concat(events, ' ')").contains("done"));
    // Past its end the sweep goes and the script is told once.
    tick(&ui, 1500.0);
    assert_eq!(sweep_quads(&ui), 0);
    assert_eq!(
        eval::<String>(&ui, "table.concat(events, ' ')")
            .matches("skill_cooldown_done")
            .count(),
        1
    );
    tick(&ui, 500.0);
    assert_eq!(
        eval::<String>(&ui, "table.concat(events, ' ')")
            .matches("skill_cooldown_done")
            .count(),
        1
    );
}

/// Glyph quads (the time label): atlas quads that are not the sweep's triangles.
fn glyph_quads(ui: &UiRuntime) -> usize {
    let mut g = aa_ui::lua::ui(&ui.lua);
    aa_ui::render::build_quads(&mut g, None)
        .iter()
        .filter(|q| q.corners.is_none() && q.tex == Tex::Atlas)
        .count()
}

#[test]
fn generic_cooldown_button_has_no_time_label_at_any_duration() {
    let ui = runtime();
    ui.exec(
        "@setup",
        r#"
        btn = UIParent:CreateWidget("cooldownbutton", "cdBtn", "UIParent")
        btn:SetExtent(40, 40)
        btn:AddAnchor("TOPLEFT", "UIParent", 300, 300)
        btn:Show(true)
        local image=btn:CreateImageDrawable('ui/icon/icon_skill_basic01.dds','artwork')
        image:AddAnchor('TOPLEFT',btn,0,0);image:AddAnchor('BOTTOMRIGHT',btn,0,0)
        btn:SetNormalBackground(image)
        btn:SetCoolDownMask('ui/icon/cooldown_mask.dds')
        btn:SetCoolDown(700, 700)
        "#,
    )
    .unwrap();
    // Generic cooldown buttons draw a mask without the slot caption function.
    assert!(sweep_quads(&ui) >= 1);
    assert_eq!(glyph_quads(&ui), 0);
    ui.exec("@limit", "btn:SetCoolDown(1500, 1500)").unwrap();
    assert_eq!(glyph_quads(&ui), 0);
    // Duration does not turn a generic button into a slot caption.
    ui.exec("@long", "btn:SetCoolDown(1600, 1600)").unwrap();
    assert_eq!(glyph_quads(&ui), 0);
    tick(&ui, 1000.0);
    assert_eq!(glyph_quads(&ui), 0);
    assert_eq!(aa_ui::lua::slot::time_label(600.0), "1s");
}

#[test]
fn cooldown_button_set_cooldown() {
    let ui = runtime();
    ui.exec(
        "@setup",
        r#"
        btn = UIParent:CreateWidget("cooldownbutton", "cdBtn", "UIParent")
        btn:SetExtent(40, 40)
        btn:AddAnchor("TOPLEFT", "UIParent", 300, 300)
        btn:Show(true)
        local image=btn:CreateImageDrawable('ui/icon/icon_skill_basic01.dds','artwork')
        image:AddAnchor('TOPLEFT',btn,0,0);image:AddAnchor('BOTTOMRIGHT',btn,0,0)
        btn:SetNormalBackground(image)
        btn:SetCoolDownMask("ui/icon/cooldown_mask.dds")
        btn:SetCoolDownMaskCoords(0, 0, 256, 256)
        btn:SetCoolDown(10000, 10000)
        "#,
    )
    .unwrap();
    // Everything left: native generic rendering uses all eight perimeter sectors.
    assert_eq!(sweep_quads(&ui), 8);
    ui.exec("@clear", "btn:SetCoolDown(0, 0)").unwrap();
    assert_eq!(sweep_quads(&ui), 0);
}

#[test]
fn sweep_geometry() {
    use aa_ui::lua::slot::sweep_triangles;
    let area = |t: &[[[f32; 2]; 3]]| {
        t.iter()
            .map(|[a, b, c]| {
                ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])).abs() / 2.0
            })
            .sum::<f32>()
    };
    let r = [0.0, 0.0, 40.0, 40.0];
    assert!((area(&sweep_triangles(r, 1.0)) - 1600.0).abs() < 1.0);
    assert!((area(&sweep_triangles(r, 0.5)) - 800.0).abs() < 1.0);
    assert!((area(&sweep_triangles(r, 0.25)) - 400.0).abs() < 1.0);
    // 36 degrees left of 12 o'clock: a triangle of 20 x 20 * tan(36 degrees) to the top edge.
    assert!((area(&sweep_triangles(r, 0.1)) - 145.3).abs() < 1.0);
    assert!(sweep_triangles(r, 0.0).is_empty());
    // The dark part is what is left, running clockwise to 12 o'clock: at 1/4 left it is the
    // top left quarter (every vertex in the left half, above the middle).
    for [a, b, c] in sweep_triangles(r, 0.25) {
        for p in [a, b, c] {
            assert!(p[0] <= 20.5 && p[1] <= 20.5, "{p:?}");
        }
    }
}

#[test]
fn roadmap_heading_and_markers() {
    let ui = runtime();
    ui.exec(
        "@setup",
        r#"
        roadmapWindow = UIParent:CreateWidget("window", "roadmapWindow", "UIParent")
        roadmapWindow:SetExtent(926, 556)
        roadmapWindow:AddAnchor("TOPRIGHT", "UIParent", -10, 50)
        roadmapWindow:EnablePick(false)
        roadmapWindow:Show(true)
        roadmap = UIParent:CreateWidget("roadmap", "roadmap", roadmapWindow)
        roadmap:AddAnchor("TOPLEFT", roadmapWindow, "TOPLEFT", 0, 0)
        roadmap:AddAnchor("BOTTOMRIGHT", roadmapWindow, "BOTTOMRIGHT", 0, 0)
        map = roadmap:CreateImageDrawable("ui/map/road/w_solzreed_road_100.dds", "background")
        map:SetExtent(780, 516)
        map:AddAnchor("TOPLEFT", roadmap, "TOPLEFT", 43, 6)
        roadmap:SaveMapData(2, "5_100", map)
        player = roadmap:CreateImageDrawable("ui/map/icon/player_cursor.dds", "overlay")
        player:SetExtent(32, 32)
        player:SetCoords(0, 0, 32, 32)
        roadmap:SetPlayerDrawable(player)
        tooltips, hides = {}, 0
        roadmap:SetHandler("OnEvent", function(self, ev, ...)
          if ev == "SHOW_ROADMAP_TOOLTIP" then local t = ...; tooltips[#tooltips + 1] = t[1].text end
          if ev == "HIDE_ROADMAP_TOOLTIP" then hides = hides + 1 end
        end)
        roadmap:RegisterEvent("SHOW_ROADMAP_TOOLTIP")
        roadmap:RegisterEvent("HIDE_ROADMAP_TOOLTIP")
        tc = roadmap:GetTooltipController()
        angle = 0
        AAUI.RoadMap = function()
          return {
            level = 2, zone = 5,
            player = { u = 0.5, v = 0.5, angle = angle },
            markers = {
              { u = 0.4, v = 0.6, kind = "quest_zone_gives", tooltip = "Maude" },
              { u = 0.7, v = 0.2, kind = "npc_store_potion" },
              { u = 0.1, v = 0.1, kind = "no_such_kind" },
            },
          }
        end
        "#,
    )
    .unwrap();
    tick(&ui, 16.0);
    let quads = |ui: &UiRuntime| {
        let mut g = aa_ui::lua::ui(&ui.lua);
        aa_ui::render::build_quads(&mut g, None)
    };
    let icon_quads = |ui: &UiRuntime, name: &str| {
        quads(ui)
            .into_iter()
            .filter(|q| matches!(&q.tex, Tex::Image(p) if p.contains(name)))
            .collect::<Vec<_>>()
    };
    // Unrotated: the cursor is a plain rectangle.
    let cursor = icon_quads(&ui, "player_cursor");
    assert_eq!(cursor.len(), 1);
    assert!(cursor[0].corners.is_none());
    // Quest markers show, NPC ones wait for the map option, an unknown kind is not drawn.
    let markers = icon_quads(&ui, "npc_icon");
    assert_eq!(markers.len(), 1, "quest marker only");
    let uv = markers[0].uv;
    assert_eq!(
        [uv[0] * 512.0, uv[1] * 256.0, uv[2] * 512.0, uv[3] * 256.0],
        [216.0, 48.0, 240.0, 72.0]
    );
    ui.exec("@npc", "roadmap:SetRoadMapNpc(true)").unwrap();
    tick(&ui, 16.0);
    assert_eq!(icon_quads(&ui, "npc_icon").len(), 2);
    ui.exec("@npc", "roadmap:SetRoadMapNpc(false)").unwrap();
    tick(&ui, 16.0);
    assert_eq!(icon_quads(&ui, "npc_icon").len(), 1);
    // Heading: east (pi / 2) turns the cone from the top to the right; the quad is rotated about
    // its centre, so its centre stays and the corners turn clockwise.
    let before = cursor[0].pos;
    ui.exec("@turn", "angle = math.pi / 2").unwrap();
    tick(&ui, 16.0);
    let turned = icon_quads(&ui, "player_cursor");
    let c = turned[0].corners.expect("rotated corners");
    let centre = [(before[0] + before[2]) / 2.0, (before[1] + before[3]) / 2.0];
    let mid = [(c[0][0] + c[2][0]) / 2.0, (c[0][1] + c[2][1]) / 2.0];
    assert!((mid[0] - centre[0]).abs() < 0.01 && (mid[1] - centre[1]).abs() < 0.01);
    // Top left (-16, -16) turned 90 degrees clockwise is the top right (+16, -16).
    assert!(
        (c[0][0] - (centre[0] + 16.0)).abs() < 0.01 && (c[0][1] - (centre[1] - 16.0)).abs() < 0.01,
        "{c:?}"
    );
    // Tooltips need the window to take the mouse and the cursor over the marker.
    let (mx, my) = {
        let u = aa_ui::lua::ui(&ui.lua);
        let r = u.tree.rect(
            u.tree
                .objects
                .iter()
                .position(|o| {
                    o.alive && o.parent.is_some() && o.shown && o.width == 24.0 && o.height == 24.0
                })
                .unwrap() as u32,
        );
        (r.x + r.w / 2.0, r.y + r.h / 2.0)
    };
    let mut mouse = aa_ui::input::MouseState::default();
    aa_ui::input::update(
        &ui.lua,
        &mut mouse,
        Some((mx, my)),
        false,
        false,
        false,
        0.0,
    );
    tick(&ui, 16.0);
    assert_eq!(
        eval::<i64>(&ui, "#tooltips"),
        0,
        "the roadmap is click-through by default"
    );
    ui.exec("@pick", "roadmapWindow:EnablePick(true)").unwrap();
    tick(&ui, 16.0);
    assert_eq!(eval::<String>(&ui, "tooltips[1]"), "Maude");
    assert!(eval::<bool>(&ui, "tc:IsVisible()"));
    aa_ui::input::update(
        &ui.lua,
        &mut mouse,
        Some((10.0, 10.0)),
        false,
        false,
        false,
        0.0,
    );
    tick(&ui, 16.0);
    assert_eq!(eval::<i64>(&ui, "hides"), 1);
    assert!(!eval::<bool>(&ui, "tc:IsVisible()"));
}

#[test]
fn chat_wheel_scrolls_and_holds_the_view() {
    let ui = runtime();
    ui.exec(
        "@setup",
        r#"
        msgs = {}
        reads, unreads = 0, 0
        function OnCreateChatMessage(m)
          msgs[#msgs + 1] = m
          m:SetHandler("OnContentUpdated", function(self, what)
            if what == "read_chat_message" then reads = reads + 1 end
            if what == "unread_chat_message" then unreads = unreads + 1 end
          end)
        end
        win = UIParent:CreateWidget("chatwindow", "chatWindow[1]", "UIParent")
        win:AddAnchor("BOTTOMLEFT", "UIParent", 0, -145)
        win:SetExtent(430, 350)
        win:SetTabAreaHeight(30)
        win:SetChatWindowId(0)
        win:Show(true)
        m = msgs[1]
        slider = m:GetSlider()
        slider:SetExtent(17, 200)
        slider:AddAnchor("TOPLEFT", m, 2, 20)
        for i = 1, 30 do X2Chat:DispatchChatMessage(CMF_SAY, "line " .. i) end
        "#,
    )
    .unwrap();
    // The frame's middle, where no widget takes the mouse.
    let mid: mlua::Table = eval(
        &ui,
        "local x, y = m:GetOffset() local w, h = m:GetExtent() return { x + w / 2, y + h / 2 }",
    );
    let (x, y): (f32, f32) = (mid.get(1).unwrap(), mid.get(2).unwrap());
    let mut mouse = aa_ui::input::MouseState::default();
    let wheel = |mouse: &mut aa_ui::input::MouseState, w: f32| {
        aa_ui::input::update(&ui.lua, mouse, Some((x, y)), false, false, false, w);
    };
    let scroll = || eval::<i64>(&ui, "m:GetCurrentScroll()");
    wheel(&mut mouse, 0.0);
    assert_eq!(scroll(), 0);
    // A notch up scrolls back 3 messages, a notch down returns (and marks the frame read).
    wheel(&mut mouse, 1.0);
    assert_eq!(scroll(), 3);
    wheel(&mut mouse, -1.0);
    assert_eq!(scroll(), 0);
    assert_eq!(eval::<i64>(&ui, "reads"), 1);
    // Scrolled back, a new message does not move the view: the scroll moves along.
    wheel(&mut mouse, 2.0);
    assert_eq!(scroll(), 6);
    ui.exec("@new", "X2Chat:DispatchChatMessage(CMF_SAY, 'line 31')")
        .unwrap();
    assert_eq!(scroll(), 7);
    assert_eq!(eval::<i64>(&ui, "unreads"), 1);
    // At the bottom nothing is unread and the view follows the newest line.
    ui.exec("@bottom", "m:ScrollToBottom()").unwrap();
    assert_eq!(scroll(), 0);
    ui.exec("@new", "X2Chat:DispatchChatMessage(CMF_SAY, 'line 32')")
        .unwrap();
    assert_eq!(scroll(), 0);
    assert_eq!(eval::<i64>(&ui, "unreads"), 1);
    // The native slider follows: over the frame it shows, with the thumb at the bottom for the
    // newest messages (value == max) and one step up per message scrolled back.
    tick(&ui, 16.0);
    assert!(eval::<bool>(&ui, "slider:IsVisible()"));
    let range = |ui: &UiRuntime| -> (f64, f64) {
        let t: mlua::Table = eval(
            ui,
            "local lo, hi = slider:GetMinMaxValues() return { lo, hi, slider:GetValue() }",
        );
        (t.get::<f64>(2).unwrap(), t.get::<f64>(3).unwrap())
    };
    let (max, value) = range(&ui);
    assert!((1.0..32.0).contains(&max), "scroll range {max}");
    assert_eq!(value, max);
    wheel(&mut mouse, 1.0);
    assert_eq!(range(&ui), (max, max - 3.0));
    // Dragging the slider to its middle scrolls the frame there.
    let track: mlua::Table = eval(
        &ui,
        "local x, y = slider:GetOffset() local w, h = slider:GetExtent() return { x + w / 2, y + h / 2 }",
    );
    let (sx, sy): (f32, f32) = (track.get(1).unwrap(), track.get(2).unwrap());
    aa_ui::input::update(&ui.lua, &mut mouse, Some((sx, sy)), true, false, false, 0.0);
    aa_ui::input::update(&ui.lua, &mut mouse, Some((sx, sy)), false, true, false, 0.0);
    assert!(
        (scroll() as f64 - max / 2.0).abs() <= 1.0,
        "scroll {} of {max}",
        scroll()
    );
    // The up button scrolls one message back, the down button one forward.
    ui.exec(
        "@buttons",
        "up = m:GetUpButton() up:SetExtent(17, 17) up:AddAnchor('TOPLEFT', m, 2, 2)          down = m:GetDownButton() down:SetExtent(17, 17) down:AddAnchor('TOPLEFT', m, 2, 230)",
    )
    .unwrap();
    wheel(&mut mouse, 0.0);
    tick(&ui, 16.0);
    let before = scroll();
    let at = |ui: &UiRuntime, w: &str| -> (f32, f32) {
        let t: mlua::Table = eval(
            ui,
            &format!("local x, y = {w}:GetOffset() return {{ x + 8, y + 8 }}"),
        );
        (t.get(1).unwrap(), t.get(2).unwrap())
    };
    let (ux, uy) = at(&ui, "up");
    aa_ui::input::update(&ui.lua, &mut mouse, Some((ux, uy)), true, false, false, 0.0);
    aa_ui::input::update(&ui.lua, &mut mouse, Some((ux, uy)), false, true, false, 0.0);
    assert_eq!(scroll(), before + 1);
    let (dx, dy) = at(&ui, "down");
    aa_ui::input::update(&ui.lua, &mut mouse, Some((dx, dy)), true, false, false, 0.0);
    aa_ui::input::update(&ui.lua, &mut mouse, Some((dx, dy)), false, true, false, 0.0);
    assert_eq!(scroll(), before);
    // Past the oldest message the scroll stops where the oldest one is at the top.
    for _ in 0..8 {
        wheel(&mut mouse, 5.0);
    }
    assert_eq!(scroll() as f64, max);
}
