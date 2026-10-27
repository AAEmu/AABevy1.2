//! Real data for the world map's climate and conflict overlays: `AAUI.MapClimate(zoneGroupId)`,
//! `AAUI.ConflictZones(worldId)` and `AAUI.ConflictState(zoneId)` (contract in `aa-ui`'s
//! `lua/worldmap.rs`). They replace the aa-ui example's fakes (`fake_worldmap.lua`).
//!
//! * Climate: `zones.group_id` -> `zones.zone_climate_id` -> `zone_climate_elems.climate_id`
//!   (`climates.id`: 2 temperate, 3 tropical, 4 subarctic, 5 arid). Every zone of the group counts.
//! * Conflict zones: the open `conflict_zones` rows of the continent. A zone group belongs to a
//!   continent by the first letter of its name (`w_` west, `e_` east, `o_` origin: the map rectangles
//!   overlap, so they cannot tell).
//! * Conflict state: the server's `SCConflictZoneState` (`aa_net::state::ConflictZones`, latest report
//!   per zone group). The state byte is `HPWS_*` as is (0..4 trouble, 5 battle, 6 war, 7 peace); the
//!   time left is the report's end time (unix seconds) minus the wall clock.
use std::collections::{BTreeSet, HashMap};

use aa_net::state::ConflictZones;
use bevy::prelude::*;
use mlua::{Lua, Table, Value};

use super::api::Shared;

/// Static tables from the game database.
#[derive(Default)]
pub(super) struct Tables {
    /// `zone_groups.id` -> its zones' `climates.id`, sorted.
    climates: HashMap<u32, Vec<u32>>,
    /// First letter of `world_groups.name` -> `world_groups.id`.
    worlds: HashMap<char, u32>,
    /// Open conflict zone groups: id and the first letter of the group's name.
    conflicts: Vec<(u32, char)>,
}

impl Tables {
    pub(super) fn load(conn: &rusqlite::Connection) -> rusqlite::Result<Self> {
        let mut t = Tables::default();
        let mut per_group: HashMap<u32, BTreeSet<u32>> = HashMap::new();
        let mut q = conn.prepare(
            "SELECT z.group_id, e.climate_id FROM zones z \
             JOIN zone_climate_elems e ON e.zone_climate_id = z.zone_climate_id \
             WHERE z.group_id IS NOT NULL",
        )?;
        let mut rows = q.query([])?;
        while let Some(r) = rows.next()? {
            per_group.entry(r.get(0)?).or_default().insert(r.get(1)?);
        }
        t.climates = per_group.into_iter().map(|(g, c)| (g, c.into_iter().collect())).collect();
        let mut q = conn.prepare("SELECT id, name FROM world_groups")?;
        let mut rows = q.query([])?;
        while let Some(r) = rows.next()? {
            let name: Option<String> = r.get(1)?;
            if let Some(c) = name.and_then(|n| n.chars().next()) {
                t.worlds.insert(c, r.get(0)?);
            }
        }
        let mut q = conn.prepare(
            "SELECT c.zone_group_id, g.name FROM conflict_zones c \
             JOIN zone_groups g ON g.id = c.zone_group_id \
             WHERE COALESCE(c.closed, 'f') NOT IN ('t', '1') ORDER BY c.zone_group_id",
        )?;
        let mut rows = q.query([])?;
        while let Some(r) = rows.next()? {
            let name: Option<String> = r.get(1)?;
            if let Some(c) = name.and_then(|n| n.chars().next()) {
                t.conflicts.push((r.get(0)?, c));
            }
        }
        Ok(t)
    }

    /// The conflict zone groups on a continent (`world_groups.id`); the world map (2) has all.
    fn zones_of(&self, world: u32) -> Vec<u32> {
        self.conflicts
            .iter()
            .filter(|(_, c)| world == 2 || self.worlds.get(c) == Some(&world))
            .map(|(id, _)| *id)
            .collect()
    }
}

/// The latest conflict reports (copied from the net state each frame).
#[derive(Default)]
pub(super) struct View {
    reports: HashMap<u32, (u8, i64)>,
}

impl View {
    /// `(state, remaining ms)`; the time left is 0 once the report's end time passed or when it has none.
    fn state(&self, zone: u32, now_s: i64) -> Option<(u8, i64)> {
        let (state, end) = *self.reports.get(&zone)?;
        Some((state, if end > 0 { (end - now_s).max(0) * 1000 } else { 0 }))
    }
}

pub(super) fn refresh(world: &World, bridge: &Shared) {
    let Some(zones) = world.get_resource::<ConflictZones>() else {
        return;
    };
    let mut data = bridge.borrow_mut();
    let view = &mut data.map_overlays;
    if view.reports.len() != zones.by_group.len() || zones.by_group.values().any(|s| {
        view.reports.get(&u32::from(s.zone_group_id)) != Some(&(s.state, s.lane_end_time_seconds()))
    }) {
        view.reports =
            zones.by_group.values().map(|s| (u32::from(s.zone_group_id), (s.state, s.lane_end_time_seconds()))).collect();
    }
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// Define the three hooks (after the fake worldmap file, so they win).
pub(super) fn install(lua: &Lua, bridge: &Shared) -> mlua::Result<()> {
    let aaui: Table = lua.globals().get("AAUI")?;
    let state = bridge.clone();
    aaui.set(
        "MapClimate",
        lua.create_function(move |lua, zone: Value| {
            let Some(zone) = as_id(&zone) else { return Ok(Value::Nil) };
            let data = state.borrow();
            match data.game.get().and_then(|g| g.map_overlays.climates.get(&zone)) {
                Some(list) if !list.is_empty() => Ok(Value::Table(lua.create_sequence_from(list.iter().copied())?)),
                _ => Ok(Value::Nil),
            }
        })?,
    )?;
    let state = bridge.clone();
    aaui.set(
        "ConflictZones",
        lua.create_function(move |lua, world: Value| {
            let data = state.borrow();
            let list = match (as_id(&world), data.game.get()) {
                (Some(w), Some(g)) => g.map_overlays.zones_of(w),
                _ => Vec::new(),
            };
            lua.create_sequence_from(list)
        })?,
    )?;
    let state = bridge.clone();
    aaui.set(
        "ConflictState",
        lua.create_function(move |_, zone: Value| {
            let data = state.borrow();
            Ok(match as_id(&zone).and_then(|z| data.map_overlays.state(z, unix_now())) {
                Some((s, ms)) => (i64::from(s), ms),
                None => (-1, 0),
            })
        })?,
    )?;
    Ok(())
}

fn as_id(v: &Value) -> Option<u32> {
    match v {
        Value::Integer(i) => u32::try_from(*i).ok(),
        Value::Number(n) if *n >= 0.0 => Some(*n as u32),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Option<Tables> {
        let conn = rusqlite::Connection::open_with_flags(
            aa_anim::db::DEFAULT_DB,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .ok()?;
        Tables::load(&conn).ok()
    }

    #[test]
    fn climates_and_continents_from_the_database() {
        let Some(t) = db() else { return };
        assert_eq!(t.climates.get(&5), Some(&vec![2]), "Solzreed is temperate");
        assert!(t.climates.get(&61).is_some_and(|c| c.len() > 1), "origin zones mix climates");
        assert_eq!(t.zones_of(3), vec![19, 20, 22, 26, 27], "the western continent");
        assert!(t.zones_of(4).contains(&14) && !t.zones_of(4).contains(&19));
        assert!(t.zones_of(5).is_empty());
    }

    #[test]
    fn state_is_the_servers_report_and_counts_down() {
        let mut v = View::default();
        v.reports.insert(19, (1, 1_000_100));
        v.reports.insert(22, (7, 0));
        assert_eq!(v.state(19, 1_000_000), Some((1, 100_000)));
        assert_eq!(v.state(19, 2_000_000), Some((1, 0)));
        assert_eq!(v.state(22, 5), Some((7, 0)));
        assert_eq!(v.state(5, 5), None);
    }
}
