//! The icons of the big map (`AAUI.WorldMap().markers`) and the roadmap's `markers_version`, without a
//! window: which markers draw on the shown map, where, tooltips, and that an unchanged version costs no
//! re-read.

use aa_ui::UiRuntime;
use aa_ui::input::{MouseState, update};
use aa_ui::render::Tex;

fn runtime() -> UiRuntime {
    UiRuntime::new("en_us", 1920.0, 1080.0).expect("runtime")
}

fn eval<T: mlua::FromLua>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().expect(code)
}

fn tick(ui: &UiRuntime) {
    aa_ui::lua::tick(&ui.lua, 16.0);
}

/// The screen rectangles of the map icon quads (`x0, y0, x1, y1`) with their texel cell.
fn icons(ui: &UiRuntime) -> Vec<([f32; 4], [f32; 4])> {
    let mut g = aa_ui::lua::ui(&ui.lua);
    aa_ui::render::build_quads(&mut g, None)
        .into_iter()
        .filter(|q| matches!(&q.tex, Tex::Image(p) if p.contains("npc_icon")))
        .map(|q| (q.pos, [q.uv[0] * 512.0, q.uv[1] * 256.0, q.uv[2] * 512.0, q.uv[3] * 256.0]))
        .collect()
}

/// A big map with the zone 5 and the city 335 registered (the scripts' own parameters), the markers read
/// from `AAUI.WorldMap` and the player on the zone map; `reads` counts how often the marker list is built.
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
    local function reg(id, level)
        local img = wm:CreateImageDrawable("Game/ui/map/world/main_world.dds", "background")
        img:SetCoords(0, 0, 1856, 1112)
        img:AddAnchor("TOPLEFT", wm, 0, 0)
        img:AddAnchor("BOTTOMRIGHT", wm, 0, 0)
        local over = wm:CreateImageDrawable("Game/ui/map/world/land_west_over.dds", "background")
        over:SetExtent(100, 100)
        over:AddAnchor("TOPLEFT", wm, 0, 0)
        wm:SaveMapData(level, id, img, over, 1, "", { r = 0, g = 0, b = 0, a = 0 }, {})
    end
    reg(5, 2)
    reg(335, 3)
    tips, hides = {}, 0
    wm:SetHandler("OnEvent", function(self, ev, ...)
        if ev == "SHOW_WORLDMAP_TOOLTIP" then local t = ...; tips[#tips + 1] = t[1].text end
        if ev == "HIDE_WORLDMAP_TOOLTIP" then hides = hides + 1 end
    end)
    wm:RegisterEvent("SHOW_WORLDMAP_TOOLTIP")
    wm:RegisterEvent("HIDE_WORLDMAP_TOOLTIP")
    tc = wm:GetTooltipController()
    reads, level, zone, version = 0, 2, 5, 1
    local list = {
        { level = 2, id = 5, u = 0.5, v = 0.25, kind = "quest_zone_gives", tooltip = "Maude" },
        { level = 2, id = 5, u = 0.25, v = 0.75, kind = "npc_store_bank", tooltip = "Banker" },
        { level = 3, id = 335, u = 0.1, v = 0.9, kind = "doodad_mail" },
        { level = 2, id = 5, u = 0.9, v = 0.1, kind = "no_such_kind" },
    }
    AAUI.WorldMap = function()
        reads = reads + 1
        return {
            level = level, zone = zone, player = { u = 0.5, v = 0.5, angle = 0 },
            markers = list, markers_version = version,
        }
    end
"#;

#[test]
fn only_the_shown_maps_markers_are_drawn() {
    let ui = runtime();
    ui.exec("@setup", SETUP).unwrap();
    tick(&ui);
    let drawn = icons(&ui);
    // the zone's two (the unknown kind has no cell), none of the city's
    assert_eq!(drawn.len(), 2, "{drawn:?}");
    let find = |cell: [f32; 4]| drawn.iter().find(|d| d.1 == cell).unwrap_or_else(|| panic!("no icon {cell:?} in {drawn:?}"));
    // the quest "!" of the sheet, centred on u 0.5, v 0.25 of the 928 x 556 map: (464, 139)
    let q = find([216.0, 48.0, 240.0, 72.0]).0;
    assert_eq!([q[0], q[1], q[2], q[3]], [452.0, 127.0, 476.0, 151.0]);
    // the bank (24, 48) at u 0.25, v 0.75: (232, 417)
    let b = find([24.0, 48.0, 48.0, 72.0]).0;
    assert_eq!([b[0], b[1], b[2], b[3]], [220.0, 405.0, 244.0, 429.0]);
    // the city map shows the city's icon instead
    ui.exec("@city", "level, zone = 3, 335").unwrap();
    tick(&ui);
    let drawn = icons(&ui);
    assert_eq!(drawn.len(), 1, "{drawn:?}");
    assert_eq!(drawn[0].1, [72.0, 72.0, 96.0, 96.0], "the mailbox");
}

#[test]
fn an_unchanged_version_is_not_read_again_and_a_new_one_is() {
    let ui = runtime();
    ui.exec("@setup", SETUP).unwrap();
    tick(&ui);
    let before = icons(&ui);
    // same version, the list itself changed behind it: nothing is re-read
    ui.exec(
        "@same",
        r#"
        AAUI.WorldMap = function()
            reads = reads + 1
            return { level = 2, zone = 5, player = { u = 0.5, v = 0.5, angle = 0 }, markers = {}, markers_version = version }
        end
        "#,
    )
    .unwrap();
    tick(&ui);
    assert_eq!(icons(&ui), before);
    // a new version is
    ui.exec("@new", "version = 2").unwrap();
    tick(&ui);
    assert!(icons(&ui).is_empty());
}

#[test]
fn the_tooltip_of_the_marker_under_the_cursor() {
    let ui = runtime();
    ui.exec("@setup", SETUP).unwrap();
    tick(&ui);
    let mut mouse = MouseState::default();
    // over Maude's "!" at (464, 139)
    update(&ui.lua, &mut mouse, Some((464.0, 139.0)), false, false, false, 0.0);
    tick(&ui);
    assert_eq!(eval::<String>(&ui, "tips[1]"), "Maude");
    assert!(eval::<bool>(&ui, "tc:IsVisible()"));
    // over the bank, which has a tooltip too
    update(&ui.lua, &mut mouse, Some((232.0, 417.0)), false, false, false, 0.0);
    tick(&ui);
    assert_eq!(eval::<String>(&ui, "tips[2]"), "Banker");
    // away from every icon
    update(&ui.lua, &mut mouse, Some((700.0, 300.0)), false, false, false, 0.0);
    tick(&ui);
    assert!(eval::<i64>(&ui, "hides") >= 1);
    assert!(!eval::<bool>(&ui, "tc:IsVisible()"));
}

#[test]
fn the_roadmap_hides_every_npc_like_icon_until_the_npc_check_is_on() {
    let ui = runtime();
    ui.exec(
        "@setup",
        r#"
        roadmapWindow = UIParent:CreateWidget("window", "roadmapWindow", "UIParent")
        roadmapWindow:SetExtent(926, 556)
        roadmapWindow:AddAnchor("TOPRIGHT", "UIParent", -10, 50)
        roadmapWindow:Show(true)
        roadmap = UIParent:CreateWidget("roadmap", "roadmap", roadmapWindow)
        roadmap:AddAnchor("TOPLEFT", roadmapWindow, "TOPLEFT", 0, 0)
        roadmap:AddAnchor("BOTTOMRIGHT", roadmapWindow, "BOTTOMRIGHT", 0, 0)
        map = roadmap:CreateImageDrawable("ui/map/road/w_solzreed_road_100.dds", "background")
        map:SetExtent(780, 516)
        map:AddAnchor("TOPLEFT", roadmap, "TOPLEFT", 43, 6)
        roadmap:SaveMapData(2, "5_100", map)
        reads = 0
        local list = {
            { u = 0.4, v = 0.6, kind = "quest_zone_gives" },
            { u = 0.5, v = 0.6, kind = "doodad_mail" },
            { u = 0.6, v = 0.6, kind = "house_pumpkin_house" },
            { u = 0.7, v = 0.6, kind = "npc_store_bank" },
            { u = 0.8, v = 0.6, kind = "corpse_pos" },
        }
        AAUI.RoadMap = function()
            reads = reads + 1
            return { level = 2, zone = 5, player = { u = 0.5, v = 0.5, angle = 0 }, markers = list, markers_version = 7 }
        end
        "#,
    )
    .unwrap();
    tick(&ui);
    let cells: Vec<[f32; 4]> = icons(&ui).into_iter().map(|d| d.1).collect();
    assert_eq!(cells.len(), 2, "the quest icon and the gravesite: {cells:?}");
    ui.exec("@npc", "roadmap:SetRoadMapNpc(true)").unwrap();
    tick(&ui);
    assert_eq!(icons(&ui).len(), 5);
}
