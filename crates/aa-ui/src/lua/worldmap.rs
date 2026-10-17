//! The `worldmap` widget: the big map window (M). The scripts (`map/world_map*.lua`) build it
//! from one hidden image drawable per map and register each with
//! `worldmap:SaveMapData(level, id, image, over, imgIndex, maskPath, maskColor, maskBounds)`;
//! this module is the native side: which map is shown, the player's cursor on it, hover
//! highlights, and zooming between the levels by mouse.
//!
//! Levels (the scripts' `WMS_* - 1`): 0 world, 1 continent, 2 zone, 3 city; `id` is the zone
//! group id (`zone_groups.id`, 5 = Solzreed Peninsula) of the map. One map shows at a time: its
//! `image` drawable (the 1856 x 1112 art, stretched over the widget, which the scripts size to
//! 928 x 556 times the expansion ratio 1, 1.5 or 2 with `SetExtent`, `SetExpandRatio`).
//!
//! Hit testing follows the client's assets. A map can have a *mask*: a rectangle of
//! `ui/map/image_map.tga` (`maskPath`, `maskBounds.colorImgCoordX/Y/W/H`) placed at
//! `colorImgOffsetX/Y` on the map at 4 map units (1/2 art pixels at expansion 1) per texel; its
//! pixels are the *colour keys* of the maps one level down (`maskColor`: `r` holds the blue
//! channel, `b` the red one, as the scripts swap them). The pixel under the cursor names the zone
//! (or city, continent) under it; that map's `over` drawable (a feathered, labelled highlight cut
//! from `*_over.dds`) shows while hovered. Left click shows the hovered map ("zone details"),
//! right click the map above ("a larger zone": the one whose mask holds this map's colour key).
//! The mouse wheel is the scripts' (`OnWheelUp` / `OnWheelDown` -> `SetExpandLevel`), panning too
//! (`mapWindow` handlers, which the press bubbles to).
//!
//! Events raised towards the scripts whenever the shown map changes (queued for the next tick):
//! `UPDATE_ZONE_LEVEL_INFO(level, id)` and `SET_DEFAULT_EXPAND_RATIO(isPlayersMap)`.
//!
//! The player's place comes from the app through a hook on `AAUI`, asked every frame while a
//! worldmap is visible:
//!
//! ```lua
//! AAUI.WorldMap() -> nil (no place) or {
//!   level = 2,                        -- level of the map the player is on (2 zone, 3 city)
//!   zone  = 5,                        -- its id ("w_solzreed": Solzreed Peninsula)
//!   player = { u = 0.76, v = 0.26,    -- optional: the player's place on that map image, 0..1
//!              angle = -2.77 },       -- optional heading, radians clockwise from north (cursor rotation)
//!   maps = {                          -- optional: the place on the maps above (up to 4)
//!     { level = 1, id = 3, u = 0.695, v = 0.342 },   -- continent "west"
//!     { level = 0, id = 2, u = 0.469, v = 0.605 },   -- the world map
//!   },
//! }
//! AAUI.MapClimate(zoneId) -> nil or { climateId, ... }   -- optional: the climates of a map's zones
//! AAUI.ConflictZones(worldId) -> { zoneId, ... }          -- optional: a continent's conflict zones
//! AAUI.ConflictState(zoneId) -> state, remainMs            -- optional: HPWS_TROUBLE_0 .. 4, HPWS_BATTLE,
//!                                                          -- HPWS_WAR, HPWS_PEACE; nil / -1 none
//! ```
//!
//! The map ids are the game database's: `zone_groups.id` for zones, `world_groups.id` for the
//! continents (3 west, 4 east, 5 origin) and the world (2). The client places the player from
//! its global position `(x, y)` with the map's rectangle in those tables (`x, y, w, h`, y up):
//! `u = (x - X) / W`, `v = 1 - (y - Y) / H` (checked against reference/ingame/worldmap_*: the
//! Nuian spawn (15578.0, 15382.1) is u 0.7594 v 0.2657 on Solzreed, 0.6952 / 0.3418 on the
//! western continent, 0.4688 / 0.6054 on the world map). Without `maps` the place on the maps
//! above is estimated from the scripts' highlight rectangles and masks (a few pixels off).
//! `MapClimate` gets the zone group id the scripts show (`GetClimateInfo`) and answers with
//! `climates.id` values (2 temperate, 3 tropical, 4 subarctic, 5 arid: the zones'
//! `zone_climate_id` through `zone_climate_elems`). `ConflictZones` gets a continent's
//! `world_groups` id and lists `zone_groups` ids (west: 19 Karkasse Ridgelands, 20 Cinderstone
//! Moor, 22 Halcyona, 26 Hellswamp, 27 Sanddeep); the continent map tints them by
//! `ConflictState` (`reference/ingame/worldmap_continent`: four tension circles), the zone map's
//! bottom panel shows the state of its zone.
//!
//! Without `AAUI.WorldMap` the hook `AAUI.RoadMap()` of the zone overlay is used (same `level`,
//! `zone`, `player`). The map opens on the player's map, and follows it when the player changes
//! maps.
//!
//! The icons of the map (quest "!" and "?", merchants, the warehouse, mailboxes, stables, portals,
//! the gravesite ...) come from the app too, as `markers` of the hook's result, in the form of
//! `AAUI.RoadMap().markers` (see `roadmap.rs`: `kind` of the client's map icon sheet, `tooltip`)
//! with the map each is on: `level`, `id` as for `maps`, and `u`, `v` the place on that map's
//! image. Only the markers of the shown map are drawn (the real client shows none on the continent
//! and the world), 24 x 24 icons centred on the place, under the player's cursor, whatever the
//! zoom. The app does the filtering of the window's "Show Icons" checks (it hears them as
//! `X2Map:SetNpcFilter(iconType, on)` and `SetNpcShowFilter` / `SetDoodadShowFilter` /
//! `SetHouseShowFilter`, natives of the app's game API). While the cursor is over a marker that
//! has a `tooltip`, the tooltip controller (`worldmap:GetTooltipController()`) is put on it and
//! `SHOW_WORLDMAP_TOOLTIP({ { tooltipType = "normal", text = tooltip } }, 1)` is raised;
//! `HIDE_WORLDMAP_TOOLTIP` when it leaves. `markers_version` works as on the roadmap. The
//! telescope, ship and fish school natives are no-ops (their game data is not available).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use mlua::{Function, Lua, Table, Value};

use super::roadmap::{Marker, marker_cell, parse_markers};
use super::{method, none, queue_ui_event, ret, ui, value_id};
use crate::tree::{Anchor, DrawKind, DrawLayer, Id, Point};

/// The widget size at expansion ratio 1 (`WORLDMAP_VISIBLE_WIDTH` / `HEIGHT`).
const BASE: (f32, f32) = (928.0, 556.0);
/// Map units (at ratio 1) one mask texel covers.
const MASK_SCALE: f32 = 4.0;

/// A decoded mask image (RGBA, top row first).
pub struct Mask {
    pub w: i32,
    pub h: i32,
    pub rgba: Vec<u8>,
}

impl Mask {
    fn get(&self, x: i32, y: i32) -> Option<[u8; 4]> {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return None;
        }
        let i = (y * self.w + x) as usize * 4;
        Some([
            self.rgba[i],
            self.rgba[i + 1],
            self.rgba[i + 2],
            self.rgba[i + 3],
        ])
    }
}

/// Decode an uncompressed or run-length encoded 24 / 32 bit true colour TGA.
pub fn parse_tga(b: &[u8]) -> Option<Mask> {
    if b.len() < 18 {
        return None;
    }
    let id_len = b[0] as usize;
    let (cmap, ty) = (b[1], b[2]);
    let w = u16::from_le_bytes([b[12], b[13]]) as usize;
    let h = u16::from_le_bytes([b[14], b[15]]) as usize;
    let (bpp, desc) = (b[16], b[17]);
    if cmap != 0 || !matches!(ty, 2 | 10) || !matches!(bpp, 24 | 32) || w == 0 || h == 0 {
        return None;
    }
    let bytes = bpp as usize / 8;
    let mut pos = 18 + id_len;
    let want = w * h * bytes;
    let mut raw: Vec<u8> = Vec::with_capacity(want);
    if ty == 2 {
        raw.extend_from_slice(b.get(pos..pos + want)?);
    } else {
        while raw.len() < want {
            let hdr = *b.get(pos)?;
            pos += 1;
            let n = (hdr & 0x7f) as usize + 1;
            if hdr & 0x80 != 0 {
                let p = b.get(pos..pos + bytes)?;
                pos += bytes;
                for _ in 0..n {
                    raw.extend_from_slice(p);
                }
            } else {
                raw.extend_from_slice(b.get(pos..pos + n * bytes)?);
                pos += n * bytes;
            }
        }
        raw.truncate(want);
    }
    let top_origin = desc & 0x20 != 0;
    let mut rgba = vec![0u8; w * h * 4];
    for y in 0..h {
        let sy = if top_origin { y } else { h - 1 - y };
        for x in 0..w {
            let s = (sy * w + x) * bytes;
            let d = (y * w + x) * 4;
            rgba[d] = raw[s + 2];
            rgba[d + 1] = raw[s + 1];
            rgba[d + 2] = raw[s];
            rgba[d + 3] = if bytes == 4 { raw[s + 3] } else { 255 };
        }
    }
    Some(Mask {
        w: w as i32,
        h: h as i32,
        rgba,
    })
}

type MaskCache = Mutex<HashMap<String, Option<Arc<Mask>>>>;

/// The mask image at a resolved pak path (decoded once per process).
fn mask_image(resolved: &str) -> Option<Arc<Mask>> {
    static CACHE: OnceLock<MaskCache> = OnceLock::new();
    let mut g = CACHE.get_or_init(Default::default).lock().ok()?;
    g.entry(resolved.to_string())
        .or_insert_with(|| {
            aa_pak::Pak::global()
                .read(resolved)
                .ok()
                .and_then(|b| parse_tga(&b))
                .map(Arc::new)
        })
        .clone()
}

/// One registered map (`SaveMapData`).
#[derive(Clone, Debug, PartialEq)]
pub struct MapEntry {
    pub level: i64,
    pub id: i64,
    /// The map art drawable.
    pub image: Id,
    /// The hover highlight drawable.
    pub over: Id,
    /// Resolved pak path of the mask image holding this map's children, if any.
    pub mask: Option<Arc<str>>,
    /// Colour of this map in its parent's mask (RGBA as decoded; alpha 0 = none).
    pub key: [u8; 4],
    /// Where the mask sits on this map (units at ratio 1).
    pub mask_offset: [f32; 2],
    /// The mask's texel rectangle `x, y, w, h`.
    pub mask_rect: [i32; 4],
    /// Where this map sits on its parent map (the highlight's rectangle: x, y, w, h at ratio 1),
    /// as the scripts placed `over` when they registered it.
    pub over_rect: [f32; 4],
}

/// The player's place as the app reports it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    pub level: i64,
    pub zone: i64,
    pub u: f32,
    pub v: f32,
    /// The player's heading, radians clockwise from north (the top of the map).
    pub angle: f32,
    /// The place on maps above the player's (`maps` of the hook): `(level, id, u, v)`.
    pub above: [Option<(i64, i64, f32, f32)>; 4],
}

/// What the marker drawables were placed for: (map, scale in 1/1000, `marker_gen`).
type MarkerKey = (Option<(i64, i64)>, i32, u32);

#[derive(Clone, Debug, PartialEq)]
pub struct WorldMapData {
    pub entries: Arc<Vec<MapEntry>>,
    pub player: Option<Id>,
    pub portal: Option<Id>,
    pub common_farm: Option<Id>,
    /// `SetPingWidget(frame, image)` and the ping mode of the ping button.
    pub ping: Option<Id>,
    pub ping_mode: bool,
    /// `SetExpandRatio`: informational, the widget's size is what counts.
    pub ratio: f32,
    /// The shown map `(level, id)`.
    pub current: Option<(i64, i64)>,
    pub place: Option<Place>,
    /// `AAUI.WorldMap().markers`, with the `markers_version` they were read from and a counter of
    /// the lists read (the key of what the drawables were placed for).
    pub markers: Arc<Vec<Marker>>,
    pub markers_version: Option<i64>,
    pub marker_gen: u32,
    /// One image drawable per marker on the shown map (reused), the markers they stand for, and
    /// the one whose tooltip is up.
    pub marker_drawables: Vec<Id>,
    pub marker_shown: Vec<usize>,
    pub marker_hover: Option<usize>,
    /// What the marker drawables were last placed for: (map, scale in 1/1000, `marker_gen`).
    marker_applied: Option<MarkerKey>,
    /// Index of the hovered child map (its `over` is shown).
    pub hover: Option<usize>,
    /// Cursor at the last press on the map: a click only counts when it did not move (a drag pans).
    pub press: Option<(f32, f32)>,
    /// What the player drawable was last placed for: (map, place, scale in 1/1000).
    applied: Option<Applied>,
}

/// (map, place, scale in 1/1000) the player drawable was placed for.
type Applied = (Option<(i64, i64)>, Option<Place>, i32);

impl Default for WorldMapData {
    fn default() -> Self {
        WorldMapData {
            entries: Arc::new(Vec::new()),
            player: None,
            portal: None,
            common_farm: None,
            ping: None,
            ping_mode: false,
            ratio: 1.0,
            current: None,
            place: None,
            markers: Arc::default(),
            markers_version: None,
            marker_gen: 0,
            marker_drawables: Vec::new(),
            marker_shown: Vec::new(),
            marker_hover: None,
            marker_applied: None,
            hover: None,
            press: None,
            applied: None,
        }
    }
}

impl WorldMapData {
    fn index_of(&self, map: (i64, i64)) -> Option<usize> {
        self.entries.iter().position(|e| (e.level, e.id) == map)
    }

    fn mask_of(&self, i: usize) -> Option<Arc<Mask>> {
        mask_image(self.entries[i].mask.as_deref()?)
    }

    /// The child map under the map point `(hx, hy)` (units at ratio 1) of the shown map `cur`:
    /// the entry one level down whose colour key is the mask pixel there.
    fn child_at(
        &self,
        cur: usize,
        hx: f32,
        hy: f32,
        usable: impl Fn(&MapEntry) -> bool,
    ) -> Option<usize> {
        let e = &self.entries[cur];
        let mask = self.mask_of(cur)?;
        let [cx, cy, cw, ch] = e.mask_rect;
        let tx = ((hx - e.mask_offset[0]) / MASK_SCALE).floor() as i32;
        let ty = ((hy - e.mask_offset[1]) / MASK_SCALE).floor() as i32;
        if tx < 0 || ty < 0 || tx >= cw || ty >= ch {
            return None;
        }
        let px = mask.get(cx + tx, cy + ty)?;
        if px[3] == 0 {
            return None;
        }
        self.entries.iter().position(|c| {
            c.level == e.level + 1 && c.key[3] > 0 && c.key[..3] == px[..3] && usable(c)
        })
    }

    /// The map one level up that contains this map: the entry whose mask has its colour key.
    fn parent_of(&self, child: usize) -> Option<usize> {
        let c = &self.entries[child];
        if c.level == 0 {
            return None;
        }
        if c.key[3] > 0 {
            for (i, p) in self.entries.iter().enumerate() {
                if p.level != c.level - 1 || p.mask.is_none() {
                    continue;
                }
                if let Some(m) = self.mask_of(i)
                    && key_pixels(&m, p.mask_rect, c.key).0 > 0
                {
                    return Some(i);
                }
            }
        }
        // No mask names it (instances, cities of caves): the world map is the parent of
        // everything above zone level, the zone list of a continent is unknown.
        (c.level == 1).then(|| self.index_of((0, 2))).flatten()
    }

    /// The colour key's centroid in the parent's mask, in map units at ratio 1.
    fn centroid_in(&self, parent: usize, child: usize) -> Option<(f32, f32)> {
        let p = &self.entries[parent];
        let m = self.mask_of(parent)?;
        let (n, sx, sy) = key_pixels(&m, p.mask_rect, self.entries[child].key);
        if n == 0 {
            return None;
        }
        let (mx, my) = (
            sx / n as f32 - p.mask_rect[0] as f32 + 0.5,
            sy / n as f32 - p.mask_rect[1] as f32 + 0.5,
        );
        Some((
            p.mask_offset[0] + mx * MASK_SCALE,
            p.mask_offset[1] + my * MASK_SCALE,
        ))
    }

    /// A point of `child`'s map (map units) as a point of its parent map. Zones and cities sit
    /// in their highlight's rectangle; a continent's mask is the same silhouette as its colour in
    /// the world mask, at twice the resolution; else the colour's centroid.
    fn lift(&self, child: usize, parent: usize, pos: (f32, f32)) -> Option<(f32, f32)> {
        let (c, p) = (&self.entries[child], &self.entries[parent]);
        if c.level >= 2 && c.over_rect[2] > 0.0 && c.over_rect[3] > 0.0 {
            let r = c.over_rect;
            return Some((r[0] + pos.0 / BASE.0 * r[2], r[1] + pos.1 / BASE.1 * r[3]));
        }
        if c.level == 1
            && c.mask_rect[2] > 0
            && c.mask_rect[3] > 0
            && let Some(pm) = self.mask_of(parent)
            && let Some(bb) = key_bbox(&pm, p.mask_rect, c.key)
        {
            let nx = (pos.0 - c.mask_offset[0]) / MASK_SCALE / c.mask_rect[2] as f32;
            let ny = (pos.1 - c.mask_offset[1]) / MASK_SCALE / c.mask_rect[3] as f32;
            let tx = bb[0] as f32 + nx * (bb[2] - bb[0] + 1) as f32;
            let ty = bb[1] as f32 + ny * (bb[3] - bb[1] + 1) as f32;
            return Some((
                p.mask_offset[0] + (tx - p.mask_rect[0] as f32) * MASK_SCALE,
                p.mask_offset[1] + (ty - p.mask_rect[1] as f32) * MASK_SCALE,
            ));
        }
        self.centroid_in(parent, child)
    }

    /// The player's place on the shown map, in map units at ratio 1.
    fn player_pos(&self) -> Option<(f32, f32)> {
        let place = self.place?;
        let cur = self.current?;
        let mine = (place.level, place.zone);
        let mut pos = (place.u * BASE.0, place.v * BASE.1);
        if cur == mine {
            return Some(pos);
        }
        // The app knows the place there.
        if let Some((_, _, u, v)) = place.above.iter().flatten().find(|a| (a.0, a.1) == cur) {
            return Some((u * BASE.0, v * BASE.1));
        }
        // On a map above the player's: lift the place map by map.
        let mut child = self.index_of(mine)?;
        for _ in 0..4 {
            let parent = self.parent_of(child)?;
            pos = self.lift(child, parent, pos)?;
            let p = &self.entries[parent];
            if (p.level, p.id) == cur {
                return Some(pos);
            }
            child = parent;
        }
        None
    }
}

/// Bounding box `[min x, min y, max x, max y]` (inclusive texels) of the mask pixels equal to
/// `key` inside `rect`.
fn key_bbox(m: &Mask, rect: [i32; 4], key: [u8; 4]) -> Option<[i32; 4]> {
    if key[3] == 0 {
        return None;
    }
    let mut bb: Option<[i32; 4]> = None;
    for y in rect[1]..rect[1] + rect[3] {
        for x in rect[0]..rect[0] + rect[2] {
            if m.get(x, y).is_some_and(|p| p[3] > 0 && p[..3] == key[..3]) {
                let b = bb.get_or_insert([x, y, x, y]);
                *b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
            }
        }
    }
    bb
}

/// Count of mask pixels (inside `rect`) equal to `key`, with the sums of their x and y.
fn key_pixels(m: &Mask, rect: [i32; 4], key: [u8; 4]) -> (u32, f32, f32) {
    let (mut n, mut sx, mut sy) = (0u32, 0.0f32, 0.0f32);
    if key[3] == 0 {
        return (0, 0.0, 0.0);
    }
    for y in rect[1]..rect[1] + rect[3] {
        for x in rect[0]..rect[0] + rect[2] {
            if let Some(p) = m.get(x, y)
                && p[3] > 0
                && p[..3] == key[..3]
            {
                n += 1;
                sx += x as f32;
                sy += y as f32;
            }
        }
    }
    (n, sx, sy)
}

fn with<R>(lua: &Lua, id: Id, f: impl FnOnce(&mut WorldMapData) -> R) -> Option<R> {
    let mut u = ui(lua);
    let w = u.tree.widget_mut(id)?;
    Some(f(w.worldmap.get_or_insert_with(Default::default)))
}

fn snapshot(lua: &Lua, id: Id) -> Option<WorldMapData> {
    ui(lua).tree.widget(id)?.worldmap.clone()
}

/// The place from `AAUI.WorldMap` (or, without it, `AAUI.RoadMap`).
fn read_place(lua: &Lua) -> Option<Place> {
    read_hook(lua).map(|h| h.0)
}

/// The hook's result: the place and the table it came from (the markers are in it).
fn read_hook(lua: &Lua) -> Option<(Place, Table)> {
    let aaui: Table = lua.globals().raw_get("AAUI").ok()?;
    let mut hook = None;
    for name in ["WorldMap", "RoadMap"] {
        if let Ok(Some(f)) = aaui.raw_get::<Option<Function>>(name) {
            hook = Some((name, f));
            break;
        }
    }
    let (name, f) = hook?;
    let t = match f.call::<Value>(()) {
        Ok(Value::Table(t)) => t,
        Ok(_) => return None,
        Err(e) => {
            bevy::log::warn!("aa-ui: AAUI.{name}: {e}");
            return None;
        }
    };
    let level = t
        .raw_get::<Option<i64>>("level")
        .ok()
        .flatten()
        .unwrap_or(2);
    let zone = t.raw_get::<Option<i64>>("zone").ok().flatten()?;
    let (u, v, angle) = match t.raw_get::<Option<Table>>("player") {
        Ok(Some(p)) => {
            let f = |k: &str, d: f32| p.raw_get::<Option<f32>>(k).ok().flatten().unwrap_or(d);
            (f("u", 0.5), f("v", 0.5), f("angle", 0.0))
        }
        _ => (0.5, 0.5, 0.0),
    };
    let mut above = [None; 4];
    if let Ok(Some(maps)) = t.raw_get::<Option<Table>>("maps") {
        for (slot, m) in above
            .iter_mut()
            .zip(maps.sequence_values::<Table>().flatten())
        {
            let n = |k: &str| m.raw_get::<Option<f64>>(k).ok().flatten();
            if let (Some(l), Some(i), Some(u), Some(v)) = (n("level"), n("id"), n("u"), n("v")) {
                *slot = Some((l as i64, i as i64, u as f32, v as f32));
            }
        }
    }
    let place = Place {
        level,
        zone,
        u,
        v,
        angle,
        above,
    };
    Some((place, t))
}

/// Ask an optional `AAUI` map hook (`MapClimate`, `ConflictZones`, `ConflictState`); nothing
/// when the app did not define it (or it failed, logged).
fn map_hook(lua: &Lua, name: &str, arg: Value) -> mlua::MultiValue {
    let Ok(aaui) = lua.globals().raw_get::<Table>("AAUI") else {
        return mlua::MultiValue::new();
    };
    match aaui.raw_get::<Option<Function>>(name) {
        Ok(Some(f)) => f.call::<mlua::MultiValue>(arg).unwrap_or_else(|e| {
            bevy::log::warn!("aa-ui: AAUI.{name}: {e}");
            mlua::MultiValue::new()
        }),
        _ => mlua::MultiValue::new(),
    }
}

/// Show the map `(level, id)`: its image only, no hover. Queues the events the scripts
/// follow (`UPDATE_ZONE_LEVEL_INFO`, `SET_DEFAULT_EXPAND_RATIO`). False when it is not registered.
pub fn show_map(lua: &Lua, wm: Id, level: i64, id: i64) -> bool {
    let Some(d) = snapshot(lua, wm) else {
        return false;
    };
    let Some(cur) = d.index_of((level, id)) else {
        return false;
    };
    {
        let mut u = ui(lua);
        for (i, e) in d.entries.iter().enumerate() {
            for (draw, want) in [(e.image, i == cur), (e.over, false)] {
                if u.tree.get(draw).shown != want {
                    u.tree.get_mut(draw).shown = want;
                }
            }
        }
    }
    let same = d.place.is_some_and(|p| (p.level, p.zone) == (level, id));
    with(lua, wm, |x| {
        x.current = Some((level, id));
        x.hover = None;
        x.applied = None;
    });
    queue_ui_event(
        lua,
        "UPDATE_ZONE_LEVEL_INFO",
        vec![Value::Integer(level), Value::Integer(id)],
    );
    queue_ui_event(lua, "SET_DEFAULT_EXPAND_RATIO", vec![Value::Boolean(same)]);
    true
}

/// Show the player's map, or the world map without a place.
fn show_players_map(lua: &Lua, wm: Id) {
    let place = snapshot(lua, wm).and_then(|d| d.place);
    let shown = place.is_some_and(|p| show_map(lua, wm, p.level, p.zone));
    if !shown {
        show_map(lua, wm, 0, 2);
    }
}

/// Per-frame update of every visible worldmap: the player's place, the map it shows, the
/// player's cursor and the hover highlight.
pub(crate) fn update(lua: &Lua) {
    let maps: Vec<Id> = {
        let u = ui(lua);
        u.tree
            .of_class("worldmap")
            .into_iter()
            .filter(|&i| u.tree.visible(i))
            .collect()
    };
    // a closed window takes its tooltip down
    let hidden: Vec<Id> = {
        let u = ui(lua);
        u.tree
            .of_class("worldmap")
            .into_iter()
            .filter(|&i| !u.tree.visible(i) && u.tree.widget(i).is_some_and(|w| w.worldmap.as_ref().is_some_and(|d| d.marker_hover.is_some())))
            .collect()
    };
    for id in hidden {
        hide_marker_tooltip(lua, id);
    }
    if maps.is_empty() {
        return;
    }
    let hook = read_hook(lua);
    let place = hook.as_ref().map(|h| h.0);
    for id in maps {
        sync(lua, id, place, hook.as_ref().map(|h| &h.1));
    }
}

fn sync(lua: &Lua, id: Id, place: Option<Place>, table: Option<&Table>) {
    let Some(mut d) = snapshot(lua, id) else {
        return;
    };
    if d.entries.is_empty() {
        return;
    }
    if d.place != place || d.current.is_none() {
        let moved = d.place.map(|p| (p.level, p.zone)) != place.map(|p| (p.level, p.zone));
        with(lua, id, |x| x.place = place);
        if d.current.is_none() || moved {
            show_players_map(lua, id);
        }
        d = snapshot(lua, id).unwrap_or(d);
    }
    // The player's cursor.
    let scale = ui(lua).tree.size(id).0 / BASE.0;
    let key = (d.current, d.place, (scale * 1000.0).round() as i32);
    if d.applied != Some(key) {
        place_player(lua, id, &d, scale);
        with(lua, id, |x| x.applied = Some(key));
    }
    // The hover highlight.
    let hover = probe(lua, id, &d);
    if hover != d.hover {
        bevy::log::debug!(
            "aa-ui: worldmap hover {:?} -> {:?}",
            d.hover.map(|h| (d.entries[h].level, d.entries[h].id)),
            hover.map(|h| (d.entries[h].level, d.entries[h].id))
        );
        {
            let mut u = ui(lua);
            for h in [d.hover, hover].into_iter().flatten() {
                let over = d.entries[h].over;
                let want = Some(h) == hover;
                if u.tree.get(over).shown != want {
                    u.tree.get_mut(over).shown = want;
                }
            }
        }
        with(lua, id, |x| x.hover = hover);
    }
    sync_markers(lua, id, table);
}

/// Read the markers when the hook has a new list, and keep the drawables on the shown map's ones.
fn sync_markers(lua: &Lua, wm: Id, table: Option<&Table>) {
    let Some(mut d) = snapshot(lua, wm) else {
        return;
    };
    match table {
        Some(t) => {
            let version = t.raw_get::<Option<i64>>("markers_version").ok().flatten();
            if version.is_none() || version != d.markers_version {
                match parse_markers(t) {
                    Ok(list) => {
                        if *list != *d.markers {
                            d.marker_gen += 1;
                            d.markers = Arc::new(list);
                        }
                    }
                    Err(e) => bevy::log::warn!("aa-ui: AAUI.WorldMap markers: {e}"),
                }
                d.markers_version = version;
                let (markers, gen_) = (d.markers.clone(), d.marker_gen);
                with(lua, wm, |x| {
                    x.markers = markers;
                    x.marker_gen = gen_;
                    x.markers_version = version;
                });
            }
        }
        None if !d.markers.is_empty() => {
            d.marker_gen += 1;
            d.markers = Arc::default();
            d.markers_version = None;
            let gen_ = d.marker_gen;
            with(lua, wm, |x| {
                x.markers = Arc::default();
                x.marker_gen = gen_;
                x.markers_version = None;
            });
        }
        None => {}
    }
    let scale = ui(lua).tree.size(wm).0 / BASE.0;
    let key = (d.current, (scale * 1000.0).round() as i32, d.marker_gen);
    if d.marker_applied != Some(key) {
        place_markers(lua, wm, &d);
        with(lua, wm, |x| x.marker_applied = Some(key));
        d = snapshot(lua, wm).unwrap_or(d);
    }
    update_marker_tooltip(lua, wm, &d);
}

/// Put the shown map's markers on their places over the widget (`u`, `v` of the map image, which
/// covers the widget); the rest of the pool is hidden.
fn place_markers(lua: &Lua, wm: Id, d: &WorldMapData) {
    let shown: Vec<usize> = d
        .markers
        .iter()
        .enumerate()
        .filter(|(_, m)| m.map.is_none() || m.map == d.current)
        .map(|(i, _)| i)
        .collect();
    let mut pool = d.marker_drawables.clone();
    let mut kept = Vec::with_capacity(shown.len());
    {
        let mut g = ui(lua);
        let u = &mut *g;
        while pool.len() < shown.len() {
            let dr = u.tree.create_drawable(wm, DrawKind::Image, "drawable:image", DrawLayer::Artwork);
            u.tree.get_mut(dr).shown = false;
            pool.push(dr);
        }
        let (ww, wh) = u.tree.size(wm);
        let mut next = 0;
        for &i in &shown {
            let m = &d.markers[i];
            let Some((path, c)) = marker_cell(u, m) else { continue };
            let dr = pool[next];
            next += 1;
            let (w, h) = (c[2].abs(), c[3].abs());
            let o = u.tree.get_mut(dr);
            o.width = w;
            o.height = h;
            o.anchors = vec![Anchor {
                point: Point::TopLeft,
                target: wm,
                rel: Point::TopLeft,
                x: (m.u * ww - w / 2.0).round(),
                y: (m.v * wh - h / 2.0).round(),
            }];
            o.shown = true;
            if let Some(dd) = o.drawable_mut() {
                dd.texture = Some(path);
                dd.coords = Some(c);
            }
            kept.push(i);
        }
        for &dr in &pool[next..] {
            if u.tree.get(dr).shown {
                u.tree.get_mut(dr).shown = false;
            }
        }
    }
    with(lua, wm, |x| {
        x.marker_drawables = pool;
        x.marker_shown = kept;
        x.marker_hover = None;
    });
}

/// The tooltip controller of the widget (made by `GetTooltipController`, the scripts' anchor).
fn tooltip_controller(lua: &Lua, wm: Id) -> Option<Id> {
    super::chat::parts(lua, wm)
        .ok()
        .and_then(|p| p.raw_get::<Option<Table>>("tooltipController").ok().flatten())
        .and_then(|t| value_id(lua, &Value::Table(t)))
}

/// The map tooltip of the marker under the cursor, as the roadmap's: the controller goes onto the
/// marker and the map scripts get `SHOW_WORLDMAP_TOOLTIP` / `HIDE_WORLDMAP_TOOLTIP`.
fn update_marker_tooltip(lua: &Lua, wm: Id, d: &WorldMapData) {
    let hover = {
        let u = ui(lua);
        let over_map = u.hover.is_some_and(|h| h == wm || u.tree.is_descendant(h, wm));
        let (cx, cy) = u.cursor;
        if !over_map {
            None
        } else {
            d.marker_shown
                .iter()
                .enumerate()
                .rev()
                .find(|&(slot, &i)| {
                    d.markers[i].tooltip.is_some()
                        && d.marker_drawables.get(slot).is_some_and(|&dr| u.tree.visible(dr) && u.tree.rect(dr).contains(cx, cy))
                })
                .map(|(slot, _)| slot)
        }
    };
    if hover == d.marker_hover {
        return;
    }
    with(lua, wm, |x| x.marker_hover = hover);
    let Some(slot) = hover else {
        hide_marker_tooltip(lua, wm);
        return;
    };
    let (Some(&i), Some(&dr)) = (d.marker_shown.get(slot), d.marker_drawables.get(slot)) else {
        return;
    };
    let Some(text) = d.markers[i].tooltip.clone() else { return };
    if let Some(c) = tooltip_controller(lua, wm) {
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
        super::fire_ui_event(lua, "SHOW_WORLDMAP_TOOLTIP", args);
    }
}

/// Take a marker's tooltip down (the cursor left it, or the window closed).
fn hide_marker_tooltip(lua: &Lua, wm: Id) {
    with(lua, wm, |x| x.marker_hover = None);
    if let Some(c) = tooltip_controller(lua, wm) {
        ui(lua).tree.get_mut(c).shown = false;
    }
    super::fire_ui_event(lua, "HIDE_WORLDMAP_TOOLTIP", vec![Value::Nil]);
}

fn place_player(lua: &Lua, wm: Id, d: &WorldMapData, scale: f32) {
    let Some(p) = d.player else { return };
    let pos = d.player_pos();
    let mut u = ui(lua);
    match pos {
        Some((bx, by)) => {
            let (w, h) = (u.tree.get(p).width, u.tree.get(p).height);
            let o = u.tree.get_mut(p);
            o.anchors = vec![Anchor {
                point: Point::TopLeft,
                target: wm,
                rel: Point::TopLeft,
                x: (bx * scale - w / 2.0).round(),
                y: (by * scale - h / 2.0).round(),
            }];
            o.shown = true;
            if let (Some(dr), Some(pl)) = (o.drawable_mut(), d.place) {
                dr.rotation = pl.angle;
            }
        }
        None => u.tree.get_mut(p).shown = false,
    }
}

/// The child map under the cursor (None when the cursor is elsewhere, outside the clipped
/// viewport, or over no zone).
fn probe(lua: &Lua, wm: Id, d: &WorldMapData) -> Option<usize> {
    let cur = d.index_of(d.current?)?;
    let u = ui(lua);
    let over_map = u
        .hover
        .is_some_and(|h| h == wm || u.tree.is_descendant(h, wm));
    if !over_map {
        return None;
    }
    let (cx, cy) = u.cursor;
    let r = u.tree.rect(wm);
    if !r.contains(cx, cy) || r.w <= 0.0 {
        return None;
    }
    let scale = r.w / BASE.0;
    d.child_at(cur, (cx - r.x) / scale, (cy - r.y) / scale, |c| {
        let o = u.tree.get(c.over);
        o.width > 0.0 && o.height > 0.0
    })
}

/// The worldmap widget `target` is or lies in.
fn worldmap_of(lua: &Lua, target: Id) -> Option<Id> {
    let u = ui(lua);
    let mut t = Some(target);
    while let Some(id) = t {
        if u.tree.widget(id).is_some_and(|w| w.class == "worldmap") {
            return Some(id);
        }
        t = u.tree.get(id).parent;
    }
    None
}

/// A mouse press / release on a worldmap: remember where it started (a click that moved was a
/// drag) and hand the event to the parent window (`mapWindow` pans on `OnMouseDown` /
/// `OnMouseUp`), which the client's message routing does for widgets without the handler.
pub(crate) fn bubble(lua: &Lua, target: Id, handler: &str, button: &str) {
    let Some(wm) = worldmap_of(lua, target) else {
        return;
    };
    if handler == "OnMouseDown" {
        let c = ui(lua).cursor;
        with(lua, wm, |x| x.press = Some(c));
    }
    if super::has_handler(lua, wm, handler) {
        return;
    }
    let mut t = ui(lua).tree.get(wm).parent;
    while let Some(id) = t {
        if super::has_handler(lua, id, handler) {
            let arg = Value::String(lua.create_string(button).unwrap());
            super::call_handler(lua, id, handler, vec![arg]);
            return;
        }
        t = ui(lua).tree.get(id).parent;
    }
}

/// A click on a worldmap. Left: show the hovered child map (or place the ping while the ping
/// button is on); right: show the map above. True when it was a worldmap click.
pub(crate) fn clicked(lua: &Lua, target: Id, button: &str) -> bool {
    let Some(wm) = worldmap_of(lua, target) else {
        return false;
    };
    let Some(d) = snapshot(lua, wm) else {
        return false;
    };
    let cursor = ui(lua).cursor;
    if button == "LeftButton" {
        let press = d.press;
        with(lua, wm, |x| x.press = None);
        if let Some(p) = press
            && ((p.0 - cursor.0).abs() > 3.0 || (p.1 - cursor.1).abs() > 3.0)
        {
            return true; // a drag
        }
    }
    let Some(cur) = d.current.and_then(|c| d.index_of(c)) else {
        return true;
    };
    match button {
        "LeftButton" => {
            if d.ping_mode {
                place_ping(lua, wm, &d, cursor);
            } else if let Some(h) = probe(lua, wm, &d) {
                let e = &d.entries[h];
                show_map(lua, wm, e.level, e.id);
            }
        }
        "RightButton" => {
            if let Some(p) = d.parent_of(cur) {
                let e = &d.entries[p];
                show_map(lua, wm, e.level, e.id);
            }
        }
        _ => {}
    }
    true
}

/// Put the ping marker where the cursor is (the ping button's mode).
fn place_ping(lua: &Lua, wm: Id, d: &WorldMapData, cursor: (f32, f32)) {
    let Some(frame) = d.ping else { return };
    let r = ui(lua).tree.rect(wm);
    {
        let mut u = ui(lua);
        let (w, h) = (u.tree.get(frame).width, u.tree.get(frame).height);
        let o = u.tree.get_mut(frame);
        o.anchors = vec![Anchor {
            point: Point::TopLeft,
            target: wm,
            rel: Point::TopLeft,
            x: (cursor.0 - r.x - w / 2.0).round(),
            y: (cursor.1 - r.y - h / 2.0).round(),
        }];
    }
    show_widget(lua, frame, true);
    // The ping mode ends with the ping (the button pops back up).
    queue_ui_event(lua, "SET_PING_MODE", vec![Value::Boolean(false)]);
}

/// `widget:Show(on)` through the script-visible method (fires `OnShow` / `OnHide`).
fn show_widget(lua: &Lua, id: Id, on: bool) {
    if let Ok(Some(t)) = super::object_table(lua, id)
        && let Ok(f) = t.get::<Function>("Show")
    {
        let _ = f.call::<()>((t, on));
    }
}

pub(crate) fn methods(lua: &Lua, m: &Table) -> mlua::Result<()> {
    // Information the app has no data for: shown nowhere.
    for name in [
        "SetTempNotifyCoord",
        "UpdateTransferTelescopeInfo",
        "UpdateTransferTelescopeArea",
        "UpdateTelescopeInfo",
        "UpdateTelescopeArea",
        "UpdatePingInfo",
        "UpdateNpcInfo",
        "UpdateMySlaveInfo",
        "UpdateHousingInfo",
        "UpdateFishSchoolInfo",
        "UpdateFishSchoolArea",
        "UpdateDoodadInfo",
        "UpdateDominionInfo",
        "UpdateCorpseInfo",
        "UpdateCompletedQuestInfo",
        "UpdateCarryingBackpackSlaveInfo",
        "StartNotifyQuestEffect",
        "ShowQuest",
        "ShowPortal",
        "ShowCommonFarm",
        "SetTempNotifyColor",
        "SaveConflictZoneImg",
        "RemoveTransferTelescopeInfo",
        "RemoveTelescopeInfo",
        "RemoveNotifyQuestInfo",
        "RemoveGivenQuestInfo",
        "RemoveFishSchoolInfo",
        "RemoveCarryingBackpackSlaveInfo",
        "ExploredRegionUpdated",
        "ClearTransferTelescopeInfo",
        "ClearTelescopeInfo",
        "ClearNpcInfo",
        "ClearNotifyQuestInfo",
        "ClearMySlaveInfo",
        "ClearHousingInfo",
        "SetExpandRatio",
        "ResetCursor",
    ] {
        m.raw_set(name, method(lua, |_, _, _| none())?)?;
    }
    // The anchor of the map tooltip: an empty widget the app puts on the marker under the cursor.
    m.raw_set(
        "GetTooltipController",
        method(lua, |lua, id, _| {
            let parts = super::chat::parts(lua, id)?;
            if let Some(t) = parts.raw_get::<Option<Table>>("tooltipController")? {
                return ret(lua, t);
            }
            let t = super::widget::create_child(lua, Some(id), "emptywidget", "tooltipController", 0, false)?;
            if let Some(c) = value_id(lua, &Value::Table(t.clone())) {
                ui(lua).tree.get_mut(c).shown = false;
            }
            parts.raw_set("tooltipController", &t)?;
            ret(lua, t)
        })?,
    )?;
    m.raw_set(
        "GetClimateInfo",
        method(lua, |lua, _, a| {
            let v = map_hook(lua, "MapClimate", a.v(1)).into_iter().next();
            ret(lua, v.unwrap_or(Value::Nil))
        })?,
    )?;
    // Conflict zones: the continent map tints the zones the app lists (`map/world_map.lua`
    // `UpdateContinentConflictState`, the scripts' `conflictImg` drawables), the zone map's
    // bottom panel shows the zone's state and time left.
    m.raw_set(
        "GetConflictZoneListByWorldId",
        method(lua, |lua, _, a| {
            match map_hook(lua, "ConflictZones", a.v(1)).into_iter().next() {
                Some(Value::Table(t)) => ret(lua, t),
                _ => ret(lua, lua.create_table()?),
            }
        })?,
    )?;
    m.raw_set(
        "GetConflictStateByZoneId",
        method(lua, |lua, _, a| Ok(map_hook(lua, "ConflictState", a.v(1))))?,
    )?;
    // Drawables the scripts hand over.
    m.raw_set(
        "SetPlayerDrawable",
        method(lua, |lua, id, a| {
            let p = value_id(lua, &a.v(1));
            // It stays as the scripts left it (shown): they show the cursor's pulse effects when
            // the map opens only if the cursor is visible then. The per-frame update places it,
            // or hides it while the player is not on the shown map.
            with(lua, id, |d| {
                d.player = p;
                d.applied = None;
            });
            none()
        })?,
    )?;
    m.raw_set(
        "SetPortalDrawable",
        method(lua, |lua, id, a| {
            let p = value_id(lua, &a.v(1));
            with(lua, id, |d| d.portal = p);
            none()
        })?,
    )?;
    m.raw_set(
        "SetCommonFarmDrawable",
        method(lua, |lua, id, a| {
            let p = value_id(lua, &a.v(1));
            with(lua, id, |d| d.common_farm = p);
            none()
        })?,
    )?;
    m.raw_set(
        "SetPingWidget",
        method(lua, |lua, id, a| {
            let p = value_id(lua, &a.v(1));
            with(lua, id, |d| d.ping = p);
            none()
        })?,
    )?;
    m.raw_set(
        "SetPingBtn",
        method(lua, |lua, id, a| {
            let on = a.b(1, false);
            with(lua, id, |d| d.ping_mode = on);
            none()
        })?,
    )?;
    m.raw_set(
        "RemovePing",
        method(lua, |lua, id, _| {
            if let Some(f) = snapshot(lua, id).and_then(|d| d.ping) {
                show_widget(lua, f, false);
            }
            none()
        })?,
    )?;
    m.raw_set(
        "OnLeftClick",
        method(lua, |lua, id, _| {
            // A click on the ping marker itself: place it again under the cursor.
            if let Some(d) = snapshot(lua, id)
                && d.ping_mode
            {
                let c = ui(lua).cursor;
                place_ping(lua, id, &d, c);
            }
            none()
        })?,
    )?;
    m.raw_set(
        "SaveMapData",
        method(lua, |lua, id, a| {
            let (level, mid) = (a.f(1) as i64, a.f(2) as i64);
            let (Some(image), Some(over)) = (value_id(lua, &a.v(3)), value_id(lua, &a.v(4))) else {
                return none();
            };
            let mask = a
                .s(6)
                .filter(|p| !p.is_empty())
                .and_then(|p| ui(lua).textures.resolve(&p))
                .map(Arc::<str>::from);
            // `r` and `b` are swapped by the scripts: `r` is the blue channel of the mask pixel.
            let key = match a.v(7) {
                Value::Table(t) => {
                    let c =
                        |k: &str| t.raw_get::<Option<f64>>(k).ok().flatten().unwrap_or(0.0) as u8;
                    [c("b"), c("g"), c("r"), c("a")]
                }
                _ => [0; 4],
            };
            let (offset, rect) = match a.v(8) {
                Value::Table(t) => {
                    let f =
                        |k: &str| t.raw_get::<Option<f64>>(k).ok().flatten().unwrap_or(0.0) as f32;
                    (
                        [f("colorImgOffsetX"), f("colorImgOffsetY")],
                        [
                            f("colorImgCoordX") as i32,
                            f("colorImgCoordY") as i32,
                            f("colorImgCoordW") as i32,
                            f("colorImgCoordH") as i32,
                        ],
                    )
                }
                _ => ([0.0; 2], [0; 4]),
            };
            // The highlight's place on the parent map: the scripts sized and anchored `over` in
            // art pixels (twice the map units of ratio 1) right before registering.
            let over_rect = {
                let u = ui(lua);
                let o = u.tree.get(over);
                let (ax, ay) = o.anchors.first().map_or((0.0, 0.0), |a| (a.x, a.y));
                [ax / 2.0, ay / 2.0, o.width / 2.0, o.height / 2.0]
            };
            let entry = MapEntry {
                level,
                id: mid,
                image,
                over,
                mask,
                key,
                mask_offset: offset,
                mask_rect: rect,
                over_rect,
            };
            // The map art and highlights start hidden (the script hid them too).
            {
                let mut u = ui(lua);
                u.tree.get_mut(image).shown = false;
                u.tree.get_mut(over).shown = false;
            }
            with(lua, id, |d| {
                let list = Arc::make_mut(&mut d.entries);
                list.retain(|e| (e.level, e.id) != (level, mid));
                list.push(entry);
                d.current = None;
            });
            none()
        })?,
    )?;
    // Where the player is on the shown map, in widget pixels; (-1, -1) when not on it (the
    // scripts then centre the view).
    m.raw_set(
        "GetPlayerViewPos",
        method(lua, |lua, id, _| {
            let scale = ui(lua).tree.size(id).0 / BASE.0;
            match snapshot(lua, id).and_then(|d| d.player_pos()) {
                Some((x, y)) => ret(lua, (x * scale, y * scale)),
                None => ret(lua, (-1.0, -1.0)),
            }
        })?,
    )?;
    // The player changed zone (or the UI reloaded): show the player's map again.
    for name in ["UpdateZoneInfo", "ReloadAllInfo"] {
        m.raw_set(
            name,
            method(lua, |lua, id, _| {
                let place = read_place(lua);
                with(lua, id, |d| d.place = place);
                show_players_map(lua, id);
                none()
            })?,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 4 x 2 bottom-up uncompressed 32 bit TGA: row 0 (bottom) red, row 1 (top) green.
    fn tga(rle: bool) -> Vec<u8> {
        let mut b = vec![0u8; 18];
        b[2] = if rle { 10 } else { 2 };
        b[12] = 4;
        b[14] = 2;
        b[16] = 32;
        b[17] = 8;
        let red = [0, 0, 255, 255]; // BGRA
        let green = [0, 255, 0, 255];
        for px in [red, green] {
            if rle {
                b.push(0x80 | 3);
                b.extend_from_slice(&px);
            } else {
                for _ in 0..4 {
                    b.extend_from_slice(&px);
                }
            }
        }
        b
    }

    #[test]
    fn tga_decodes_to_top_down_rgba() {
        for rle in [false, true] {
            let m = parse_tga(&tga(rle)).unwrap();
            assert_eq!((m.w, m.h), (4, 2));
            assert_eq!(
                m.get(0, 0),
                Some([0, 255, 0, 255]),
                "top row is the last stored"
            );
            assert_eq!(m.get(3, 1), Some([255, 0, 0, 255]));
            assert_eq!(m.get(4, 0), None);
        }
        assert!(parse_tga(&[0; 10]).is_none());
    }

    #[test]
    fn mask_pixels_and_centroid() {
        // 3 x 2 mask: keyed pixels at (1,0) and (2,0).
        let k = [10, 20, 30, 255];
        let mut rgba = vec![255u8; 3 * 2 * 4];
        for x in [1usize, 2] {
            rgba[x * 4..x * 4 + 4].copy_from_slice(&k);
        }
        let m = Mask { w: 3, h: 2, rgba };
        assert_eq!(key_pixels(&m, [0, 0, 3, 2], k), (2, 3.0, 0.0));
        assert_eq!(key_pixels(&m, [0, 0, 1, 2], k).0, 0);
        assert_eq!(key_pixels(&m, [0, 0, 3, 2], [10, 20, 30, 0]).0, 0);
        assert_eq!(key_bbox(&m, [0, 0, 3, 2], k), Some([1, 0, 2, 0]));
        assert_eq!(key_bbox(&m, [0, 1, 3, 1], k), None);
    }
}
