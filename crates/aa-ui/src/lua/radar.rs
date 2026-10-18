//! Native Map radar rows. The app injects published contacts; the original Lua event
//! handlers call Update/Remove/Clear without fetching invented Lua row tables.
//! Classification, localized names, and received health belong to the app.

use std::sync::Arc;

use mlua::{Lua, Table, Value};

use super::{method, none, ui, value_id};
use crate::tree::{Anchor, DrawKind, DrawLayer, Id, Point};

/// The original map rectangle, in game coordinates with Y up. Levels are 0 world,
/// 1 continent, 2 zone, 3 city, matching `GameData` / the world-map widget.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapBounds {
    pub level: u8,
    pub id: u32,
    pub rect: [f32; 4],
}

/// Exact symbols from original map/common.lua (native zero-based indices 70..80).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Symbol {
    Ship,
    Shipyard,
    EnemyShip,
    EnemyShipyard,
    Airship,
    Carriage,
    Landship,
    FishSchool,
}

impl Symbol {
    fn kind(self) -> &'static str {
        match self {
            Self::Ship => "ship",
            Self::Shipyard => "shipyard",
            Self::EnemyShip => "ship_enemy",
            Self::EnemyShipyard => "shipyard_enemy",
            Self::Airship => "transfer_airship",
            Self::Carriage => "transfer_carriage",
            Self::Landship => "transfer_landship",
            Self::FishSchool => "fish_school",
        }
    }
}

/// Original `ShowMapTooltip` fields. `kind` is the localized SLAVE_KIND lookup key,
/// not the network's raw unit-kind integer. Omit metadata that has not been resolved.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShipTooltip {
    pub name: Option<String>,
    pub kind: Option<String>,
    pub owner: Option<String>,
    pub expedition: Option<String>,
    pub buff: Option<bool>,
    pub hp: Option<f32>,
    pub max_hp: Option<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Tooltip {
    Text(String),
    Ship(ShipTooltip),
}

/// One classified native map row, identified by its runtime object ID.
#[derive(Clone, Debug, PartialEq)]
pub struct Contact {
    pub obj_id: u32,
    pub position: [f32; 2],
    pub symbol: Symbol,
    pub tooltip: Option<Tooltip>,
}

/// Only completed/published contacts belong here. Pending chunks and unknown toggle
/// state remain in the network layer. An empty channel supplies no displayed rows.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub maps: Arc<Vec<MapBounds>>,
    pub ships: Arc<Vec<Contact>>,
    pub transfers: Arc<Vec<Contact>>,
    pub fish: Arc<Vec<Contact>>,
}

/// Supply native buffers before raising the original UPDATE/CLEAR/REMOVE events.
/// This advances shared manager rows even without any Map widgets. Injection alone
/// does not replace widgets' displayed rows; Update*Info copies its manager channel.
/// There is no aa-net dependency or Lua getter.
pub fn set_snapshot(lua: &Lua, snapshot: Snapshot) {
    for channel in 0..3 {
        let source = published(&snapshot, channel);
        cached(lua, channel, |rows| {
            rows.retain(|row| source.iter().any(|c| c.obj_id == row.obj_id));
            for c in source.iter() {
                if let Some(row) = rows.iter_mut().find(|row| row.obj_id == c.obj_id) {
                    row.position = c.position;
                } else {
                    rows.push(Row {
                        obj_id: c.obj_id,
                        position: c.position,
                        symbol: c.symbol,
                    });
                }
            }
        });
    }
    lua.set_app_data(Arc::new(snapshot));
}

/// Native Map manager rows contain position, first-appearance symbol and object ID.
/// Tooltip strings/health are read separately from the current published buffers.
#[derive(Clone, Debug, PartialEq)]
struct Row {
    obj_id: u32,
    position: [f32; 2],
    symbol: Symbol,
}

#[derive(Default)]
struct Cache([Arc<Vec<Row>>; 3]);

#[derive(Clone, Debug, PartialEq)]
struct Applied {
    bounds: Option<MapBounds>,
    size: [f32; 2],
    generation: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct View {
    rows: [Arc<Vec<Row>>; 3],
    drawables: [Vec<Id>; 3],
    generation: u64,
    applied: Option<Applied>,
    hover: Option<(usize, usize, Contact)>,
}

fn snapshot(lua: &Lua) -> Arc<Snapshot> {
    lua.app_data_ref::<Arc<Snapshot>>()
        .map(|s| Arc::clone(&s))
        .unwrap_or_default()
}

fn published(source: &Snapshot, channel: usize) -> &Arc<Vec<Contact>> {
    match channel {
        0 => &source.ships,
        1 => &source.transfers,
        _ => &source.fish,
    }
}

fn cached(lua: &Lua, channel: usize, f: impl FnOnce(&mut Vec<Row>)) -> Arc<Vec<Row>> {
    if lua.app_data_ref::<Cache>().is_none() {
        lua.set_app_data(Cache::default());
    }
    let mut cache = lua.app_data_mut::<Cache>().expect("aa-ui: radar cache");
    f(Arc::make_mut(&mut cache.0[channel]));
    cache.0[channel].clone()
}

fn set_rows(lua: &Lua, id: Id, channel: usize, rows: Arc<Vec<Row>>) {
    with(lua, id, |v| {
        if v.rows[channel] != rows {
            v.rows[channel] = rows;
            v.generation = v.generation.wrapping_add(1);
        }
    });
    update_map(lua, id);
}

fn view(lua: &Lua, id: Id) -> Option<View> {
    ui(lua).tree.widget(id)?.radar.clone()
}

fn with(lua: &Lua, id: Id, f: impl FnOnce(&mut View)) {
    if let Some(w) = ui(lua).tree.widget_mut(id) {
        f(w.radar.get_or_insert_with(Default::default));
    }
}

fn map_key(lua: &Lua, id: Id) -> Option<(u8, u32)> {
    let u = ui(lua);
    let w = u.tree.widget(id)?;
    if let Some(d) = &w.worldmap {
        let (level, id) = d.current?;
        Some((u8::try_from(level).ok()?, u32::try_from(id).ok()?))
    } else {
        let (level, key) = w.roadmap.as_ref()?.current.as_ref()?;
        let id = key.split_once('_')?.0.parse().ok()?;
        Some((u8::try_from(*level).ok()?, id))
    }
}

/// Position icons on the Map widget itself: the road art has a separate crop/frame,
/// but native 392b10ae projects against widget size and world rectangle +2c8..2d4.
pub(crate) fn update_map(lua: &Lua, id: Id) {
    let Some(mut d) = view(lua, id) else { return };
    let source = snapshot(lua);
    let key = map_key(lua, id);
    let bounds = source.maps.iter().copied().find(|m| {
        // Native descriptor flags (Map manager +598) hide all eight radar symbols
        // at world level and enable continent, zone and city, regardless of NPC check.
        Some((m.level, m.id)) == key
            && (1..=3).contains(&m.level)
            && m.rect.iter().all(|v| v.is_finite())
            && m.rect[2] > 0.
            && m.rect[3] > 0.
    });
    let (width, height) = ui(lua).tree.size(id);
    let applied = Applied {
        bounds,
        size: [width, height],
        generation: d.generation,
    };
    if d.applied.as_ref() != Some(&applied) {
        let mut u = ui(lua);
        for channel in 0..3 {
            while d.drawables[channel].len() < d.rows[channel].len() {
                let dr = u.tree.create_drawable(
                    id,
                    DrawKind::Image,
                    "drawable:image",
                    DrawLayer::Artwork,
                );
                u.tree.get_mut(dr).shown = false;
                d.drawables[channel].push(dr);
            }
            for (i, &dr) in d.drawables[channel].iter().enumerate() {
                let spec = d.rows[channel]
                    .get(i)
                    .filter(|c| c.position.iter().all(|p| p.is_finite()));
                let placement = spec.zip(bounds).and_then(|(c, b)| {
                    let cell = super::roadmap::KIND_CELLS
                        .iter()
                        .find(|(kind, _)| *kind == c.symbol.kind())?
                        .1;
                    let x = (c.position[0] - b.rect[0]) / b.rect[2] * width;
                    let y = (1. - (c.position[1] - b.rect[1]) / b.rect[3]) * height;
                    Some((cell, x, y))
                });
                let o = u.tree.get_mut(dr);
                o.shown = placement.is_some();
                if let Some((cell, x, y)) = placement {
                    o.width = cell[2];
                    o.height = cell[3];
                    o.anchors = vec![Anchor {
                        point: Point::Center,
                        target: id,
                        rel: Point::TopLeft,
                        x,
                        y,
                    }];
                    if let Some(drawable) = o.drawable_mut() {
                        drawable.texture = Some("ui/map/icon/npc_icon.dds".into());
                        drawable.coords = Some(cell);
                    }
                }
            }
        }
        d.applied = Some(applied);
        drop(u);
        with(lua, id, |v| *v = d.clone());
    }
    update_tooltip(lua, id, &d);
}

/// Normal map markers defer to the radar contact under the same pointer. Their hover
/// state is reset by those modules so their tooltip can resume when the contact leaves.
pub(crate) fn has_tooltip(lua: &Lua, id: Id) -> bool {
    ui(lua)
        .tree
        .widget(id)
        .and_then(|w| w.radar.as_ref())
        .is_some_and(|v| v.hover.is_some())
}

fn update_tooltip(lua: &Lua, id: Id, d: &View) {
    let source = snapshot(lua);
    let (hover, worldmap) = {
        let u = ui(lua);
        let worldmap = u.tree.widget(id).is_some_and(|w| w.worldmap.is_some());
        let pickable = u.tree.visible(id)
            && if worldmap {
                u.hover
                    .is_some_and(|h| h == id || u.tree.is_descendant(h, id))
            } else {
                u.tree
                    .widget(u.tree.top_of(id))
                    .is_some_and(|w| w.clickable)
            };
        let mut hover = None;
        if pickable {
            for channel in (0..3).rev() {
                for (i, &dr) in d.drawables[channel].iter().enumerate().rev() {
                    if let Some(c) = d.rows[channel].get(i).and_then(|row| {
                        published(&source, channel)
                            .iter()
                            .find(|c| c.obj_id == row.obj_id)
                            .filter(|c| c.tooltip.is_some())
                    }) && u.tree.visible(dr)
                        && u.tree.rect(dr).contains(u.cursor.0, u.cursor.1)
                    {
                        hover = Some((channel, i, c.clone()));
                        break;
                    }
                }
                if hover.is_some() {
                    break;
                }
            }
        }
        (hover, worldmap)
    };
    if hover == d.hover {
        return;
    }
    with(lua, id, |v| v.hover = hover.clone());
    let controller = super::chat::parts(lua, id)
        .ok()
        .and_then(|p| {
            p.raw_get::<Option<Table>>("tooltipController")
                .ok()
                .flatten()
        })
        .and_then(|t| value_id(lua, &Value::Table(t)));
    let suffix = if worldmap {
        "WORLDMAP_TOOLTIP"
    } else {
        "ROADMAP_TOOLTIP"
    };
    match hover {
        Some((channel, i, c)) => {
            if let Some(controller) = controller {
                let mut u = ui(lua);
                let o = u.tree.get_mut(controller);
                o.width = 24.;
                o.height = 24.;
                o.shown = true;
                o.anchors = vec![Anchor {
                    point: Point::TopLeft,
                    target: d.drawables[channel][i],
                    rel: Point::TopLeft,
                    x: 0.,
                    y: 0.,
                }];
            }
            if let Ok(entry) = tooltip_entry(lua, &c) {
                if let Ok(list) = lua.create_sequence_from([entry]) {
                    super::fire_ui_event(
                        lua,
                        &format!("SHOW_{suffix}"),
                        vec![Value::Table(list), Value::Integer(1)],
                    );
                }
            }
        }
        None => {
            if let Some(controller) = controller {
                ui(lua).tree.get_mut(controller).shown = false;
            }
            super::fire_ui_event(lua, &format!("HIDE_{suffix}"), vec![Value::Nil]);
        }
    }
}

fn tooltip_entry(lua: &Lua, contact: &Contact) -> mlua::Result<Table> {
    let entry = lua.create_table()?;
    match contact.tooltip.as_ref().expect("hovered tooltip") {
        Tooltip::Text(text) => {
            entry.raw_set("tooltipType", "normal")?;
            entry.raw_set("text", text.as_str())?;
        }
        Tooltip::Ship(info) => {
            let shipyard = matches!(contact.symbol, Symbol::Shipyard | Symbol::EnemyShipyard);
            entry.raw_set("tooltipType", if shipyard { "shipyard" } else { "slave" })?;
            entry.raw_set("id", contact.obj_id)?;
            entry.raw_set(
                "enemy",
                matches!(contact.symbol, Symbol::EnemyShip | Symbol::EnemyShipyard),
            )?;
            entry.raw_set("name", info.name.as_deref())?;
            entry.raw_set("kind", info.kind.as_deref())?;
            entry.raw_set("owner", info.owner.as_deref())?;
            entry.raw_set("expedition", info.expedition.as_deref())?;
            entry.raw_set("buff", info.buff)?;
            entry.raw_set("hp", info.hp)?;
            entry.raw_set("maxHp", info.max_hp)?;
        }
    }
    Ok(entry)
}

pub(crate) fn methods(lua: &Lua, methods: &Table) -> mlua::Result<()> {
    for (channel, update, remove, clear) in [
        (
            0,
            "UpdateTelescopeInfo",
            "RemoveTelescopeInfo",
            "ClearTelescopeInfo",
        ),
        (
            1,
            "UpdateTransferTelescopeInfo",
            "RemoveTransferTelescopeInfo",
            "ClearTransferTelescopeInfo",
        ),
        (
            2,
            "UpdateFishSchoolInfo",
            "RemoveFishSchoolInfo",
            "ClearFishSchoolInfo",
        ),
    ] {
        methods.raw_set(
            update,
            method(lua, move |lua, id, _| {
                let rows = cached(lua, channel, |_| {});
                set_rows(lua, id, channel, rows);
                none()
            })?,
        )?;
        methods.raw_set(
            remove,
            method(lua, move |lua, id, args| {
                let object_id = match args.v(1) {
                    Value::Integer(n) => u32::try_from(n).ok(),
                    Value::Number(n)
                        if n.is_finite() && n >= 0. && n.fract() == 0. && n <= u32::MAX as f64 =>
                    {
                        Some(n as u32)
                    }
                    _ => None,
                };
                if let Some(object_id) = object_id {
                    let rows = cached(lua, channel, |rows| {
                        rows.retain(|c| c.obj_id != object_id);
                    });
                    set_rows(lua, id, channel, rows);
                }
                none()
            })?,
        )?;
        methods.raw_set(
            clear,
            method(lua, move |lua, id, _| {
                let rows = cached(lua, channel, Vec::clear);
                set_rows(lua, id, channel, rows);
                none()
            })?,
        )?;
    }
    Ok(())
}
