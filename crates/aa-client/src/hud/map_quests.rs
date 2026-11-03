//! Where the active quests want you on the map: the objective areas and the numbered notifier icons of the
//! world map and the roadmap.
//!
//! The client knows the places from `worlds/main_world/level_design/zone/<zone key>/client/quest_sign_sphere.g`
//! (the same files the server reads): blocks of `area / qtype / ctype / pos (x, y, z) / radius`, the quest
//! (`qtype`), its step (`ctype`, a `quest_components.id`), a place relative to the zone's origin cell
//! (`world.xml` `originX/originY` * 1024) and a radius in metres. The server tells the client which step each
//! active quest is on (`Quest::component_id`), so the objective area of a quest is the spheres of that step.
//!
//! The original scripts do the rest and this module only answers their questions (`map_quests.lua`):
//! `quest_context_notifier` gives every tracked quest a decal index (0 for a main quest, 1 to 9 for the others,
//! the number in the notifier list) when `X2Decal:CanMakeGuidedDecal(qtype)` holds, which is when the quest's step
//! has an area here, and tells the map with `X2Map:AddNotifyQuestInfo(index, qtype, show)`. `HUD.quest_targets(qtype)`
//! returns those areas as `{ group, u, v, ru, x, y }`: the zone group's map (`zone_groups.id`), the place on its
//! map rectangle as fractions (v down), the radius as a fraction of the rectangle's width and the place in metres.

use std::collections::HashMap;
use std::sync::OnceLock;

use aa_net::quest::Quest;
use mlua::{Lua, Table};

use super::api::Shared;
use super::data::GameData;

/// One `area` block, in game metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sphere {
    pub zone_key: u32,
    pub x: f32,
    pub y: f32,
    pub radius: f32,
}

/// The areas of a quest's steps: quest -> (step, area).
#[derive(Default)]
struct SignSpheres {
    by_quest: HashMap<u32, Vec<(u32, Sphere)>>,
}

/// The `area` blocks of one `quest_sign_sphere.g` as `(quest, step, area)`; `origin` is the zone's origin cell in metres.
fn parse_sign_spheres(text: &str, zone_key: u32, origin: (f32, f32)) -> Vec<(u32, u32, Sphere)> {
    let mut out = Vec::new();
    let (mut quest, mut step, mut pos) = (None, None, None);
    for line in text.lines() {
        let line = line.trim().to_ascii_lowercase();
        let (key, rest) = line.split_once(char::is_whitespace).unwrap_or((line.as_str(), ""));
        match key {
            "area" => (quest, step, pos) = (None, None, None),
            "qtype" => quest = rest.trim().parse::<u32>().ok(),
            "ctype" => step = rest.trim().parse::<u32>().ok(),
            "pos" => {
                let clean: String = rest.chars().map(|c| if matches!(c, '(' | ')' | ',') { ' ' } else { c }).collect();
                let n: Vec<f32> = clean.split_whitespace().filter_map(|t| t.parse().ok()).collect();
                pos = (n.len() >= 3).then(|| (n[0], n[1]));
            }
            "radius" => {
                if let (Some(q), Some(c), Some((x, y)), Ok(radius)) = (quest, step, pos, rest.trim().parse::<f32>()) {
                    out.push((q, c, Sphere { zone_key, x: origin.0 + x, y: origin.1 + y, radius }));
                }
                (quest, step, pos) = (None, None, None);
            }
            _ => {}
        }
    }
    out
}

/// Every area of the main world, read once (about 70 small files).
fn spheres() -> &'static SignSpheres {
    static SPHERES: OnceLock<SignSpheres> = OnceLock::new();
    SPHERES.get_or_init(|| {
        let mut s = SignSpheres::default();
        let pak = aa_pak::Pak::global();
        let world = crate::ingame::WORLD;
        let Ok(xml) = pak.read_string(&format!("worlds/{world}/world.xml")) else { return s };
        let mut zones: Vec<_> = super::portal::parse_zone_origins(&xml).into_iter().collect();
        zones.sort_by_key(|z| z.0);
        for (zone, origin) in zones {
            let path = format!("worlds/{world}/level_design/zone/{zone}/client/quest_sign_sphere.g");
            let Ok(text) = pak.read_string(&path) else { continue };
            for (q, c, sphere) in parse_sign_spheres(&text, zone, origin) {
                s.by_quest.entry(q).or_default().push((c, sphere));
            }
        }
        s
    })
}

/// An objective area on a zone group's map.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Target {
    pub group: u32,
    pub u: f32,
    pub v: f32,
    pub ru: f32,
    pub x: f32,
    pub y: f32,
}

/// Where the quest's current step wants the player; nothing for a step without an area (talk to an NPC ...).
///
/// The server may report the quest's start component while it is in progress (the `bevy` lane sends
/// `component_id` 383, the start, for quest 251 whose areas belong to its progress step 633). When the reported
/// step has no area and the quest is not waiting to be handed in, the quest's own areas stand for it.
fn targets_of(game: &GameData, all: &SignSpheres, quest: &Quest) -> Vec<Target> {
    let Some(list) = all.by_quest.get(&quest.template_id) else { return Vec::new() };
    let exact = list.iter().any(|(step, _)| *step == quest.component_id);
    let open = !matches!(quest.status, aa_net::quest::status::READY | aa_net::quest::status::COMPLETED);
    list.iter()
        .filter(|(step, _)| *step == quest.component_id || (!exact && open))
        .filter_map(|(_, s)| {
            let group = game.zones.get(&s.zone_key)?.0;
            let r = game.zone_groups.get(&group)?.rect;
            (r[2] > 0.0 && r[3] > 0.0).then(|| Target {
                group,
                u: (s.x - r[0]) / r[2],
                v: 1.0 - (s.y - r[1]) / r[3],
                ru: s.radius / r[2],
                x: s.x,
                y: s.y,
            })
        })
        .collect()
}

/// `HUD.quest_targets(qtype)`: the areas of an active quest's step (an empty list for none).
pub fn install(lua: &Lua, hud: &Table, bridge: &Shared) -> mlua::Result<()> {
    let b = bridge.clone();
    hud.raw_set(
        "quest_targets",
        lua.create_function(move |lua, qtype: u32| {
            let out = lua.create_table()?;
            let d = b.borrow();
            let (Some(game), Some(q)) = (d.game.get(), d.dialog.quests.iter().find(|q| q.template_id == qtype)) else { return Ok(out) };
            for (i, t) in targets_of(game, spheres(), q).iter().enumerate() {
                let e = lua.create_table_with_capacity(0, 6)?;
                e.raw_set("group", t.group)?;
                for (k, v) in [("u", t.u), ("v", t.v), ("ru", t.ru), ("x", t.x), ("y", t.y)] {
                    e.raw_set(k, f64::from(v))?;
                }
                out.raw_set(i + 1, e)?;
            }
            Ok(out)
        })?,
    )
}

/// `map_quests.lua`: the decal and notifier calls of the quest scripts and the markers of the maps (after `quest.lua`).
pub fn load_script(lua: &Lua) -> mlua::Result<()> {
    lua.load(include_str!("map_quests.lua")).set_name("@map_quests.lua").exec()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "area\r\n    qtype 179\r\n    ctype 2381\r\n    pos ( x 543.493, y 1064.27, z 188.048 )\r\n    radius 30\r\n\
        area\r\n    qtype 191\r\n    ctype 2770\r\n    pos ( x 619.159, y 2128.68, z 122.021 )\r\n    radius 50\r\n\
        area\r\n    qtype 7\r\n    pos ( x 1, y 2, z 3 )\r\n    radius 5\r\n";

    #[test]
    fn areas_get_the_zone_origin() {
        let s = parse_sign_spheres(FILE, 133, (1024.0, 2048.0));
        assert_eq!(s.len(), 2, "a block without a step is dropped");
        assert_eq!((s[0].0, s[0].1), (179, 2381));
        assert_eq!(s[0].2, Sphere { zone_key: 133, x: 1024.0 + 543.493, y: 2048.0 + 1064.27, radius: 30.0 });
        assert_eq!((s[1].0, s[1].1, s[1].2.radius), (191, 2770, 50.0));
    }

    /// The script compiles, and with the toolkit's globals stubbed it makes the notifier's calls drive the maps' markers.
    #[test]
    fn script_turns_notifier_calls_into_markers() {
        let lua = Lua::new();
        lua.load(
            r#"
            X2Decal, X2Map, X2Quest = {}, {}, {}
            X2Quest.GetActiveQuestTitle = function() return "Title" end
            HUD = { state = { zone_group = 2, zone_key_name = "w_a" }, quest_version = function() return 1 end,
              quest_targets = function(q) if q == 179 then return { { group = 2, u = 0.5, v = 0.25, ru = 0.01 } } end return {} end,
              road_layout = function() return { 0, 0, 1, 1 }, 926, 556 end,
              road_image_point = function(f, w, h, u, v) return u, v end }
            AAUI = { RoadMap = function() return { level = 2, zone = 2, markers = { { u = 0.1, v = 0.1, kind = "party" } }, markers_version = 1 } end,
              WorldMap = function() return { markers = {}, markers_version = 1 } end }
            "#,
        )
        .exec()
        .unwrap();
        lua.load(include_str!("map_quests.lua")).exec().unwrap();
        let ok: bool = lua.load("return X2Decal:CanMakeGuidedDecal(179) and not X2Decal:CanMakeGuidedDecal(5) and X2Decal:GetMaxDecalCount() == 10").eval().unwrap();
        assert!(ok);
        let n: i64 = lua.load("return #AAUI.WorldMap().markers").eval().unwrap();
        assert_eq!(n, 0, "nothing tracked: nothing drawn");
        lua.load("X2Map:AddNotifyQuestInfo(3, 179, true)").exec().unwrap();
        let (n, first, last, r): (i64, String, String, f64) = lua
            .load("local m = AAUI.WorldMap().markers; return #m, m[1].icon, m[#m].icon, m[1].radius")
            .eval()
            .unwrap();
        assert_eq!((n, first.as_str(), last.as_str()), (2, "ui/map/territory_circle.dds", "ui/map/icon/npc_icon.dds"));
        assert!((r - 0.01).abs() < 1e-9);
        let (n, kinds): (i64, String) = lua
            .load("local m = AAUI.RoadMap().markers; return #m, (m[1].icon or '') .. '|' .. (m[2].kind or '') .. '|' .. (m[3].coords[1])")
            .eval()
            .unwrap();
        assert_eq!((n, kinds.as_str()), (3, "ui/map/territory_circle.dds|party|272"), "circle, the party icon, then the number 3");
        lua.load("X2Map:UpdateNotifyQuestInfo(3, 179, false)").exec().unwrap();
        let n: i64 = lua.load("return #AAUI.WorldMap().markers").eval().unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn only_the_current_step_counts() {
        let mut game = GameData::default();
        game.zones.insert(133, (2, 0));
        game.zone_groups.insert(2, super::super::data::ZoneGroup { rect: [1000.0, 2000.0, 2000.0, 1000.0], ..Default::default() });
        let mut all = SignSpheres::default();
        for (q, c, s) in parse_sign_spheres(FILE, 133, (1024.0, 2048.0)) {
            all.by_quest.entry(q).or_default().push((c, s));
        }
        let quest = Quest { template_id: 179, component_id: 2381, ..Default::default() };
        let t = targets_of(&game, &all, &quest);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].group, 2);
        assert!((t[0].u - (1567.493 - 1000.0) / 2000.0).abs() < 1e-4);
        assert!((t[0].v - (1.0 - (3112.27 - 2000.0) / 1000.0)).abs() < 1e-4);
        assert!((t[0].ru - 0.015).abs() < 1e-6);
        // a step the server names without an area: the quest's areas while it runs, none once it waits to be handed in
        assert_eq!(targets_of(&game, &all, &Quest { template_id: 179, component_id: 99, ..Default::default() }).len(), 1);
        let ready = Quest { template_id: 179, component_id: 99, status: aa_net::quest::status::READY, ..Default::default() };
        assert!(targets_of(&game, &all, &ready).is_empty());
        assert!(targets_of(&game, &all, &Quest { template_id: 5, ..Default::default() }).is_empty());
    }

    /// The real files: every area lies on the map of the zone group it is in (when the client files are there).
    #[test]
    fn real_areas_are_on_their_zone_map() {
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        let Ok(game) = GameData::load(&path) else { return };
        let all = spheres();
        let total: usize = all.by_quest.values().map(Vec::len).sum();
        if total == 0 {
            return;
        }
        let (mut inside, mut outside) = (0, 0);
        for list in all.by_quest.values() {
            for (_, s) in list {
                let Some(r) = game.zones.get(&s.zone_key).and_then(|z| game.zone_groups.get(&z.0)).map(|g| g.rect) else { continue };
                if s.x >= r[0] && s.x <= r[0] + r[2] && s.y >= r[1] && s.y <= r[1] + r[3] { inside += 1 } else { outside += 1 }
            }
        }
        assert!(inside > 500, "{inside} inside, {outside} outside");
        assert!(outside * 20 < inside, "{inside} inside, {outside} outside");
    }
}
