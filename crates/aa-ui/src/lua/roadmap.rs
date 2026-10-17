//! `roadmap` widgets (the zone map overlay at the top right of the HUD) and a few small natives
//! of other classes (`listbox` tree images).
//!
//! The scripts make one hidden image drawable per zone map and size
//! (`map/road_map_list.lua`: `ui/map/road/<zone>_road_<50|70|100>.dds`, placed at its offset in
//! the roadmap) and register it with `roadmap:SaveMapData(level, "<zone>_<size>", drawable)`;
//! `SetMapSize(50|70|100)` picks the size and `SetPlayerDrawable(d)` the player cursor. The
//! native shows the current zone's drawable, raises `UPDATE_ROADMAP_ANCHOR(level, key)` when it
//! changes (the scripts re-anchor the window) and moves the cursor. The current zone and the
//! player's place come from the app, asked every frame:
//!
//! ```lua
//! AAUI.RoadMap() -> nil (no map) or {
//!   level = 2,                       -- as registered: RMS_ZONE + 1 = 2 zone maps, 3 city maps
//!   zone = 5,                        -- the map key's zone part ("5_100": Solzreed Peninsula)
//!   player = {                       -- optional
//!     u = 0.85, v = 0.27,            --   position on the map image, 0..1
//!     angle = -2.77,                 --   optional heading, see below
//!   },
//!   markers = {                      -- optional: quest / NPC icons on the map
//!     { u = 0.84, v = 0.30,          --   position on the map image, 0..1 (like the player)
//!       kind = "quest_zone_gives",   --   a symbol of the client's map icon sheet, see below
//!       icon = "ui/map/icon/x.dds",  --   optional: a texture instead of `kind`'s sheet cell
//!       coords = { x, y, w, h },     --   optional: its texel rectangle (default whole texture)
//!       tooltip = "Maude" },         --   optional: text shown while the cursor is over it
//!     ...
//!   },
//! }
//! ```
//!
//! `player.angle` is the player's heading in radians, 0 = north (the top of the map, where the
//! cursor texture `ui/map/icon/player_cursor.dds` points: its view cone is above the ring),
//! growing clockwise on screen (pi / 2 = east). It turns the cursor drawable about its centre
//! (`Drawable::rotation`, the same as `drawable:SetRotation(r)`, which the scripts may call too).
//! From a CryEngine entity yaw (counter-clockwise from +Y, `reference/ingame/spawn/camera.json`
//! `player.entity.angles_rad.z`) it is `angle = -yaw`.
//!
//! `markers` are what the client's roadmap draws natively from the quest and NPC data
//! (`UpdateNpcInfo`, `AddGivenQuestInfo`, ...): 24 x 24 icons of the map icon sheet
//! `ui/map/icon/npc_icon.dds` centred on `u, v`, below the player cursor. `kind` is the symbol
//! name of `map/common.lua` (`M_QUEST_ZONE_GIVES` = `"quest_zone_gives"`, lower case without
//! `M_`; the cells are [`KIND_CELLS`]): `quest_main_gives` (green !), `quest_zone_gives` (yellow !),
//! `quest_main_completes` / `quest_zone_completes` (?), `quest_*_progress` (grey ?),
//! `quest_repeat`, `quest_livelihood_*`, `quest_hunt_gives`, `party`, `portal`, `npc_store_*`,
//! `npc_station`, `doodad_*`, `housing_*`, ... An unknown `kind` without `icon` is not drawn.
//! Quest and other markers always show; `npc_*`, `friendly` and `hostile` ones only while the
//! "NPC" check of the map options is on (`roadmap:SetRoadMapNpc(true)`). The markers are only
//! drawn while the zone's map is.
//!
//! A marker with a `tooltip` gets what the client's roadmap does: while the roadmap window takes
//! the mouse (`SET_ROADMAP_PICKABLE`, `roadmapWindow:EnablePick(true)`) and the cursor is over
//! it, the tooltip controller (`roadmap:GetTooltipController()`) is put on the marker and
//! `SHOW_ROADMAP_TOOLTIP({ { tooltipType = "normal", text = tooltip } }, 1)` is raised (the
//! scripts show their map tooltip); `HIDE_ROADMAP_TOOLTIP` when the cursor leaves it.

use mlua::{Function, Lua, Table, Value};

use super::{method, new_object_table, none, ret, ui, value_id};
use crate::tree::{Anchor, DrawKind, DrawLayer, Id, Point};

/// The map icon sheet the markers come from.
const MAP_ICON_SHEET: &str = "ui/map/icon/npc_icon.dds";

/// The cells (x, y, w, h in texels) of `ui/map/icon/npc_icon.dds` by marker `kind`: the
/// `npcCoordsList` of `map/common.lua`.
pub const KIND_CELLS: &[(&str, [f32; 4])] = &[
    ("quest_talk_or_employment", [0.0, 0.0, 24.0, 24.0]),
    ("quest_main_gives", [216.0, 0.0, 24.0, 24.0]),
    ("quest_zone_gives", [216.0, 48.0, 24.0, 24.0]),
    ("quest_repeat", [216.0, 168.0, 24.0, 24.0]),
    ("quest_main_completes", [216.0, 24.0, 24.0, 24.0]),
    ("quest_main_progress", [216.0, 96.0, 24.0, 24.0]),
    ("quest_zone_completes", [216.0, 72.0, 24.0, 24.0]),
    ("quest_zone_progress", [216.0, 96.0, 24.0, 24.0]),
    ("quest_livelihood_gives", [216.0, 192.0, 24.0, 24.0]),
    ("quest_livelihood_complete", [216.0, 216.0, 24.0, 24.0]),
    ("quest_hunt_gives", [192.0, 0.0, 24.0, 24.0]),
    ("quest_notifier", [0.0, 0.0, 24.0, 24.0]),
    ("party", [168.0, 0.0, 24.0, 24.0]),
    ("raidteam", [168.0, 24.0, 24.0, 24.0]),
    ("offline_party", [168.0, 48.0, 24.0, 24.0]),
    ("raidteam_owner", [144.0, 96.0, 24.0, 24.0]),
    ("quest_letit", [216.0, 120.0, 24.0, 24.0]),
    ("quest_over", [216.0, 144.0, 24.0, 24.0]),
    ("npc_station", [144.0, 48.0, 24.0, 24.0]),
    ("npc_graveyard", [120.0, 216.0, 24.0, 24.0]),
    ("npc_ability_changer", [0.0, 96.0, 24.0, 24.0]),
    ("npc_stabler", [24.0, 96.0, 24.0, 24.0]),
    ("friendly", [160.0, 32.0, 24.0, 24.0]),
    ("hostile", [160.0, 0.0, 24.0, 24.0]),
    ("npc_store_potion", [0.0, 24.0, 24.0, 24.0]),
    ("npc_store_clothes", [24.0, 0.0, 24.0, 24.0]),
    ("npc_store_food", [24.0, 120.0, 24.0, 24.0]),
    ("npc_store_weapon", [0.0, 0.0, 24.0, 24.0]),
    ("npc_store_defense", [24.0, 0.0, 24.0, 24.0]),
    ("npc_store_tree", [24.0, 72.0, 24.0, 24.0]),
    ("npc_store_plants", [24.0, 216.0, 24.0, 24.0]),
    ("npc_store_material", [0.0, 168.0, 24.0, 24.0]),
    ("npc_store_goods", [0.0, 168.0, 24.0, 24.0]),
    ("npc_store_enlisting", [24.0, 168.0, 24.0, 24.0]),
    ("npc_store_ship", [24.0, 192.0, 24.0, 24.0]),
    ("npc_store_siege_weapon", [0.0, 48.0, 24.0, 24.0]),
    ("npc_store_building", [64.0, 128.0, 24.0, 24.0]),
    ("npc_store_furniture", [0.0, 120.0, 24.0, 24.0]),
    ("npc_store_livestock", [24.0, 144.0, 24.0, 24.0]),
    ("npc_store_bank", [24.0, 48.0, 24.0, 24.0]),
    ("npc_store_auction_house", [0.0, 144.0, 24.0, 24.0]),
    ("npc_store_expedition", [48.0, 120.0, 24.0, 24.0]),
    ("npc_store_pirate_expedition", [0.0, 216.0, 24.0, 24.0]),
    ("npc_store_territory", [64.0, 160.0, 24.0, 24.0]),
    ("npc_store_trade", [48.0, 48.0, 24.0, 24.0]),
    ("npc_store_delphinad", [48.0, 24.0, 24.0, 24.0]),
    ("npc_store_roamer", [0.0, 168.0, 24.0, 24.0]),
    ("npc_store_blacksmith", [48.0, 72.0, 24.0, 24.0]),
    ("npc_store_gladiator", [240.0, 168.0, 24.0, 24.0]),
    ("npc_instance_target_corps1", [192.0, 216.0, 24.0, 24.0]),
    ("npc_instance_target_corps2", [192.0, 192.0, 24.0, 24.0]),
    ("housing_house", [72.0, 96.0, 24.0, 24.0]),
    ("housing_farm", [72.0, 120.0, 24.0, 24.0]),
    ("housing_fishfarm", [144.0, 72.0, 24.0, 24.0]),
    ("my_crops", [72.0, 0.0, 24.0, 24.0]),
    ("corpse_pos", [120.0, 192.0, 24.0, 24.0]),
    ("my_ping", [144.0, 216.0, 24.0, 24.0]),
    ("portal", [168.0, 96.0, 24.0, 24.0]),
    ("ship", [144.0, 0.0, 24.0, 24.0]),
    ("shipyard", [144.0, 24.0, 24.0, 24.0]),
    ("ship_enemy", [168.0, 120.0, 24.0, 24.0]),
    ("shipyard_enemy", [168.0, 144.0, 24.0, 24.0]),
    ("common_farm", [144.0, 120.0, 24.0, 24.0]),
    ("transfer_airship", [168.0, 216.0, 24.0, 24.0]),
    ("transfer_carriage", [168.0, 168.0, 24.0, 24.0]),
    ("transfer_landship", [168.0, 192.0, 24.0, 24.0]),
    ("force_released_ship", [48.0, 168.0, 24.0, 24.0]),
    ("force_released_vehicle", [48.0, 168.0, 24.0, 24.0]),
    ("fish_school", [192.0, 144.0, 24.0, 24.0]),
    ("doodad_mail", [72.0, 72.0, 24.0, 24.0]),
    ("doodad_inn", [120.0, 168.0, 24.0, 24.0]),
    ("doodad_leather", [96.0, 192.0, 24.0, 24.0]),
    ("doodad_craft", [96.0, 120.0, 24.0, 24.0]),
    ("doodad_metal", [120.0, 144.0, 24.0, 24.0]),
    ("doodad_machinery", [120.0, 0.0, 24.0, 24.0]),
    ("doodad_timber", [96.0, 144.0, 24.0, 24.0]),
    ("doodad_woodwork", [96.0, 168.0, 24.0, 24.0]),
    ("doodad_weapon", [120.0, 72.0, 24.0, 24.0]),
    ("doodad_stone", [120.0, 96.0, 24.0, 24.0]),
    ("doodad_archium", [96.0, 216.0, 24.0, 24.0]),
    ("doodad_alchemy", [120.0, 48.0, 24.0, 24.0]),
    ("doodad_cook", [120.0, 24.0, 24.0, 24.0]),
    ("doodad_coatofarms", [120.0, 120.0, 24.0, 24.0]),
    ("doodad_fabric", [96.0, 24.0, 24.0, 24.0]),
    ("doodad_special_product", [96.0, 96.0, 24.0, 24.0]),
    ("doodad_light_armor", [96.0, 72.0, 24.0, 24.0]),
    ("doodad_leather_armor", [96.0, 0.0, 24.0, 24.0]),
    ("doodad_heavy_armor", [96.0, 48.0, 24.0, 24.0]),
    ("doodad_paper", [192.0, 120.0, 24.0, 24.0]),
    ("doodad_print", [192.0, 96.0, 24.0, 24.0]),
    ("doodad_fish_stand", [192.0, 168.0, 24.0, 24.0]),
    ("doodad_art_work", [240.0, 192.0, 24.0, 24.0]),
    ("doodad_portal", [144.0, 144.0, 24.0, 24.0]),
    ("doodad_portal_archemall", [144.0, 168.0, 24.0, 24.0]),
    ("doodad_portal_dungeon", [144.0, 192.0, 24.0, 24.0]),
    ("house_normal_house", [72.0, 216.0, 24.0, 24.0]),
    ("house_sea_farm_house", [192.0, 24.0, 24.0, 24.0]),
    ("house_high_house", [192.0, 48.0, 24.0, 24.0]),
    ("house_pumpkin_house", [192.0, 72.0, 24.0, 24.0]),
    ("house_farm_house", [192.0, 72.0, 24.0, 24.0]),
    ("house_benny_house", [240.0, 120.0, 24.0, 24.0]),
    ("house_bungalow_house", [240.0, 144.0, 24.0, 24.0]),
    ("my_slave", [48.0, 216.0, 24.0, 24.0]),
];

/// One entry of `AAUI.RoadMap().markers`.
#[derive(Clone, Debug, PartialEq)]
pub struct Marker {
    pub u: f32,
    pub v: f32,
    pub kind: String,
    pub icon: Option<String>,
    pub coords: Option<[f32; 4]>,
    pub tooltip: Option<String>,
}

impl Marker {
    /// Hidden unless the map options' "NPC" check is on.
    fn is_npc(&self) -> bool {
        self.kind.starts_with("npc_") || self.kind == "friendly" || self.kind == "hostile"
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RoadMapData {
    pub map_size: i64,
    /// (level, key, drawable) from `SaveMapData`.
    pub maps: Vec<(i64, String, Id)>,
    pub player: Option<Id>,
    /// The shown map (level, key).
    pub current: Option<(i64, String)>,
    /// Player position last applied (u, v).
    pub last_player: Option<(f32, f32)>,
    /// Player heading last applied (radians).
    pub last_angle: f32,
    /// `SetRoadMapNpc(checked)`: the "NPC" check of the map options.
    pub show_npcs: bool,
    /// The markers last placed, with the map drawable and `show_npcs` they were placed for.
    pub markers: Vec<Marker>,
    pub placed: Option<(Option<Id>, bool)>,
    /// One image drawable per marker (reused).
    pub marker_drawables: Vec<Id>,
    /// The marker whose tooltip is up.
    pub hover: Option<usize>,
}

impl Default for RoadMapData {
    fn default() -> Self {
        RoadMapData {
            map_size: 100,
            maps: Vec::new(),
            player: None,
            current: None,
            last_player: None,
            last_angle: 0.0,
            show_npcs: false,
            markers: Vec::new(),
            placed: None,
            marker_drawables: Vec::new(),
            hover: None,
        }
    }
}

fn hook(lua: &Lua) -> Option<Function> {
    let aaui: Table = lua.globals().raw_get("AAUI").ok()?;
    aaui.raw_get("RoadMap").ok()
}

fn with<R>(lua: &Lua, id: Id, f: impl FnOnce(&mut RoadMapData) -> R) -> Option<R> {
    let mut u = ui(lua);
    let w = u.tree.widget_mut(id)?;
    Some(f(w.roadmap.get_or_insert_with(Default::default)))
}

fn data(lua: &Lua, id: Id) -> RoadMapData {
    ui(lua)
        .tree
        .widget(id)
        .and_then(|w| w.roadmap.clone())
        .unwrap_or_default()
}

/// Ask the hook for every live roadmap: show the zone's map, move the player cursor.
pub(crate) fn update(lua: &Lua) {
    let Some(f) = hook(lua) else { return };
    let maps: Vec<Id> = {
        ui(lua).tree.of_class("roadmap")
    };
    if maps.is_empty() {
        return;
    }
    let r = match f.call::<Value>(()) {
        Ok(Value::Table(t)) => Some(t),
        Ok(_) => None,
        Err(e) => {
            bevy::log::warn!("aa-ui: AAUI.RoadMap: {e}");
            return;
        }
    };
    for id in maps {
        if let Err(e) = apply(lua, id, r.as_ref()) {
            bevy::log::warn!("aa-ui: AAUI.RoadMap result: {e}");
        }
    }
}

/// `AAUI.RoadMap().markers`.
fn parse_markers(r: &Table) -> mlua::Result<Vec<Marker>> {
    let Some(list) = r.raw_get::<Option<Table>>("markers")? else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for v in list.sequence_values::<Table>() {
        let v = v?;
        let (Some(mu), Some(mv)) = (v.raw_get::<Option<f32>>("u")?, v.raw_get::<Option<f32>>("v")?) else {
            continue;
        };
        let coords = match v.raw_get::<Option<Table>>("coords")? {
            Some(c) => Some([
                c.raw_get::<f32>(1)?,
                c.raw_get::<f32>(2)?,
                c.raw_get::<f32>(3)?,
                c.raw_get::<f32>(4)?,
            ]),
            None => None,
        };
        out.push(Marker {
            u: mu,
            v: mv,
            kind: v.raw_get::<Option<String>>("kind")?.unwrap_or_default(),
            icon: v.raw_get("icon")?,
            coords,
            tooltip: v.raw_get("tooltip")?,
        });
    }
    Ok(out)
}

/// The texture and texel rectangle a marker is drawn with.
fn marker_cell(u: &mut super::Ui, m: &Marker) -> Option<(String, [f32; 4])> {
    match &m.icon {
        Some(path) => {
            let c = match m.coords {
                Some(c) => c,
                None => {
                    let (w, h) = u.textures.size(path)?;
                    [0.0, 0.0, w as f32, h as f32]
                }
            };
            Some((path.clone(), c))
        }
        None => KIND_CELLS
            .iter()
            .find(|(k, _)| *k == m.kind)
            .map(|(_, c)| (MAP_ICON_SHEET.to_string(), *c)),
    }
}

/// Place the markers' drawables on the shown map (created once, reused; hidden without a map).
fn sync_markers(lua: &Lua, id: Id, shown_map: Option<Id>, markers: Vec<Marker>) {
    let d = data(lua, id);
    let key = Some((shown_map, d.show_npcs));
    if markers == d.markers && d.placed == key {
        return;
    }
    let mut pool = d.marker_drawables.clone();
    {
        let mut g = ui(lua);
        let u = &mut *g;
        while pool.len() < markers.len() {
            let dr = u
                .tree
                .create_drawable(id, DrawKind::Image, "drawable:image", DrawLayer::Artwork);
            u.tree.get_mut(dr).shown = false;
            pool.push(dr);
        }
        for (i, &dr) in pool.iter().enumerate() {
            let spec = markers
                .get(i)
                .filter(|m| shown_map.is_some() && (d.show_npcs || !m.is_npc()));
            let cell = spec.and_then(|m| marker_cell(u, m));
            match (spec, cell, shown_map) {
                (Some(m), Some((path, c)), Some(map)) => {
                    let (mw, mh) = (u.tree.get(map).width, u.tree.get(map).height);
                    let (w, h) = (c[2].abs(), c[3].abs());
                    let o = u.tree.get_mut(dr);
                    o.width = w;
                    o.height = h;
                    o.anchors = vec![Anchor {
                        point: Point::TopLeft,
                        target: map,
                        rel: Point::TopLeft,
                        x: (m.u * mw - w / 2.0).round(),
                        y: (m.v * mh - h / 2.0).round(),
                    }];
                    o.shown = true;
                    if let Some(dd) = o.drawable_mut() {
                        dd.texture = Some(path);
                        dd.coords = Some(c);
                    }
                }
                _ => u.tree.get_mut(dr).shown = false,
            }
        }
    }
    with(lua, id, |x| {
        x.markers = markers;
        x.placed = key;
        x.marker_drawables = pool;
    });
}

/// The map tooltip of the marker under the cursor, like the client's roadmap does it (only while
/// the roadmap window takes the mouse): the tooltip controller goes onto the marker and the map
/// scripts get `SHOW_ROADMAP_TOOLTIP` / `HIDE_ROADMAP_TOOLTIP`.
fn update_tooltip(lua: &Lua, id: Id) {
    let d = data(lua, id);
    let hover = {
        let u = ui(lua);
        let pickable = u.tree.widget(u.tree.top_of(id)).is_some_and(|w| w.clickable) && u.tree.visible(id);
        let (cx, cy) = u.cursor;
        if !pickable {
            None
        } else {
            d.marker_drawables
                .iter()
                .enumerate()
                .rev()
                .find(|&(i, &dr)| {
                    d.markers.get(i).is_some_and(|m| m.tooltip.is_some())
                        && u.tree.visible(dr)
                        && u.tree.rect(dr).contains(cx, cy)
                })
                .map(|(i, _)| i)
        }
    };
    if hover == d.hover {
        return;
    }
    with(lua, id, |x| x.hover = hover);
    let controller = super::chat::parts(lua, id)
        .ok()
        .and_then(|p| p.raw_get::<Option<Table>>("tooltipController").ok().flatten())
        .and_then(|t| value_id(lua, &Value::Table(t)));
    match hover {
        Some(i) => {
            let (Some(text), Some(&dr)) = (d.markers[i].tooltip.clone(), d.marker_drawables.get(i)) else {
                return;
            };
            if let Some(c) = controller {
                let mut u = ui(lua);
                let r = u.tree.rect(dr);
                let o = u.tree.get_mut(c);
                o.anchors = vec![Anchor {
                    point: Point::TopLeft,
                    target: crate::tree::ROOT,
                    rel: Point::TopLeft,
                    x: r.x,
                    y: r.y,
                }];
                o.width = r.w;
                o.height = r.h;
                o.shown = true;
            }
            let build = || -> mlua::Result<Vec<Value>> {
                let entry = lua.create_table()?;
                entry.raw_set("tooltipType", "normal")?;
                entry.raw_set("text", text)?;
                let list = lua.create_table()?;
                list.raw_set(1, entry)?;
                Ok(vec![Value::Table(list), Value::Integer(1)])
            };
            if let Ok(args) = build() {
                super::fire_ui_event(lua, "SHOW_ROADMAP_TOOLTIP", args);
            }
        }
        None => {
            if let Some(c) = controller {
                ui(lua).tree.get_mut(c).shown = false;
            }
            super::fire_ui_event(lua, "HIDE_ROADMAP_TOOLTIP", vec![Value::Nil]);
        }
    }
}

/// Move and turn the player cursor (`angle`: radians clockwise from north).
fn place_player(lua: &Lua, id: Id, shown_map: Option<Id>, player: Option<(f32, f32)>, angle: f32) {
    let d = data(lua, id);
    let Some(p) = d.player else { return };
    let place = match (shown_map, player) {
        (Some(m), Some(uv)) => Some((m, uv)),
        _ => None,
    };
    if place.is_some() && place.map(|x| x.1) == d.last_player && angle == d.last_angle {
        return;
    }
    {
        let mut u = ui(lua);
        match place {
            Some((m, (pu, pv))) => {
                let (mw, mh) = (u.tree.get(m).width, u.tree.get(m).height);
                let o = u.tree.get_mut(p);
                let (w, h) = (o.width, o.height);
                o.anchors = vec![Anchor {
                    point: Point::TopLeft,
                    target: m,
                    rel: Point::TopLeft,
                    x: (pu * mw - w / 2.0).round(),
                    y: (pv * mh - h / 2.0).round(),
                }];
                o.shown = true;
                if let Some(dr) = o.drawable_mut() {
                    dr.rotation = angle;
                }
            }
            None => {
                if u.tree.get(p).shown {
                    u.tree.get_mut(p).shown = false;
                }
            }
        }
    }
    with(lua, id, |x| {
        x.last_player = place.map(|x| x.1);
        x.last_angle = angle;
    });
}

fn apply(lua: &Lua, id: Id, r: Option<&Table>) -> mlua::Result<()> {
    let d = data(lua, id);
    let (want, player, angle, markers) = match r {
        Some(r) => {
            let level = r.raw_get::<Option<i64>>("level")?.unwrap_or(1);
            let zone = r.raw_get::<Option<i64>>("zone")?;
            let (player, angle) = match r.raw_get::<Option<Table>>("player")? {
                Some(p) => (
                    Some((p.raw_get::<f32>("u")?, p.raw_get::<f32>("v")?)),
                    p.raw_get::<Option<f32>>("angle")?.unwrap_or(0.0),
                ),
                None => (None, 0.0),
            };
            (
                zone.map(|z| (level, format!("{z}_{}", d.map_size))),
                player,
                angle,
                parse_markers(r)?,
            )
        }
        None => (None, None, 0.0, Vec::new()),
    };
    let shown_map = want
        .as_ref()
        .and_then(|(l, k)| d.maps.iter().find(|(ml, mk, _)| ml == l && mk == k))
        .map(|m| m.2);
    if want != d.current {
        {
            let mut u = ui(lua);
            for (_, _, m) in &d.maps {
                u.tree.get_mut(*m).shown = Some(*m) == shown_map;
            }
        }
        with(lua, id, |x| {
            x.current = want.clone();
            x.last_player = None;
        });
        if let Some((level, key)) = &want {
            let args = vec![Value::Integer(*level), Value::String(lua.create_string(key)?)];
            super::fire_ui_event(lua, "UPDATE_ROADMAP_ANCHOR", args);
        }
    }
    place_player(lua, id, shown_map, player, angle);
    sync_markers(lua, id, shown_map, markers);
    update_tooltip(lua, id);
    Ok(())
}

pub(crate) fn methods(lua: &Lua, m: &Table) -> mlua::Result<()> {
    let def = |name: &str, f: mlua::Function| m.raw_set(name, f);
    def(
        "SetPlayerDrawable",
        method(lua, |lua, id, a| {
            let p = value_id(lua, &a.v(1));
            // Hidden until the app places it (`AAUI.RoadMap`).
            if let Some(p) = p {
                ui(lua).tree.get_mut(p).shown = false;
            }
            with(lua, id, |d| {
                d.player = p;
                d.last_player = None;
            });
            none()
        })?,
    )?;
    def(
        "SaveMapData",
        method(lua, |lua, id, a| {
            let level = a.f(1) as i64;
            let key = a.s(2).unwrap_or_default();
            if let Some(d) = value_id(lua, &a.v(3)) {
                with(lua, id, |x| {
                    x.maps.retain(|(l, k, _)| !(*l == level && *k == key));
                    x.maps.push((level, key, d));
                    x.current = None;
                });
            }
            none()
        })?,
    )?;
    def(
        "SetMapSize",
        method(lua, |lua, id, a| {
            let s = a.f(1).round() as i64;
            with(lua, id, |x| {
                x.map_size = s;
                x.current = None;
            });
            none()
        })?,
    )?;
    def(
        "GetTooltipController",
        method(lua, |lua, id, _| {
            let parts = super::chat::parts(lua, id)?;
            if let Some(t) = parts.raw_get::<Option<Table>>("tooltipController")? {
                return ret(lua, t);
            }
            let t = super::widget::create_child(
                lua,
                Some(id),
                "emptywidget",
                "tooltipController",
                0,
                false,
            )?;
            if let Some(c) = value_id(lua, &Value::Table(t.clone())) {
                ui(lua).tree.get_mut(c).shown = false;
            }
            parts.raw_set("tooltipController", &t)?;
            ret(lua, t)
        })?,
    )?;
    // The "NPC" check of the map options: `npc_*` markers show only while it is on.
    def(
        "SetRoadMapNpc",
        method(lua, |lua, id, a| {
            let on = a.b(1, false);
            with(lua, id, |d| d.show_npcs = on);
            none()
        })?,
    )?;
    for name in ["UpdateAllDrawableAnchor", "SetTooltipColor", "ReloadAllInfo"] {
        def(name, method(lua, |_, _, _| none())?)?;
    }
    Ok(())
}

/// `listbox` tree images (`CreateOpenedImageDrawable(path)`, closed, separator): image drawables
/// the native list places on tree items; kept hidden (tree lists are not drawn yet).
pub(crate) fn listbox_methods(lua: &Lua, m: &Table) -> mlua::Result<()> {
    for name in [
        "CreateOpenedImageDrawable",
        "CreateClosedImageDrawable",
        "CreateSeparatorImageDrawable",
    ] {
        m.raw_set(
            name,
            method(lua, |lua, id, a| {
                let d = {
                    let mut u = ui(lua);
                    let d = u.tree.create_drawable(
                        id,
                        DrawKind::Image,
                        "drawable:image",
                        DrawLayer::parse(&a.s(2).unwrap_or_else(|| "artwork".into())),
                    );
                    if let Some(dr) = u.tree.drawable_mut(d) {
                        dr.texture = a.s(1);
                    }
                    u.tree.get_mut(d).shown = false;
                    d
                };
                ret(lua, new_object_table(lua, d)?)
            })?,
        )?;
    }
    Ok(())
}
