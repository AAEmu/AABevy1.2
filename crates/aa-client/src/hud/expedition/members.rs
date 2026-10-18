//! Native local member ordering, paging and original row presentation.
use super::*;
use aa_net::ExpeditionMember;
use rusqlite::Connection;
use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

const PER_PAGE: usize = 50;

#[derive(Default)]
pub(in crate::hud) struct Tables {
    zones: HashMap<u32, (String, Option<String>)>,
    creators: HashSet<u32>,
}

impl Tables {
    pub(in crate::hud) fn load(conn: &Connection) -> rusqlite::Result<Self> {
        let mut out = Self::default();
        let mut query = conn.prepare("SELECT id,expedition FROM npcs")?;
        let mut rows = query.query([])?;
        while let Some(row) = rows.next()? {
            if super::super::data::any_flag(row, 1)? {
                out.creators.insert(row.get(0)?);
            }
        }
        let mut query = conn.prepare(
            "SELECT z.zone_key, COALESCE(lz.en_us,z.display_text,''), w.name, COALESCE(lw.en_us,u.text,w.name) \
             FROM zones z LEFT JOIN localized_texts lz ON lz.tbl_name='zones' AND lz.tbl_column_name='display_text' AND lz.idx=z.id \
             LEFT JOIN zone_groups g ON g.id=z.group_id LEFT JOIN world_groups w ON w.id=g.target_id \
             LEFT JOIN ui_texts u ON u.category_id=92 AND u.key=w.name \
             LEFT JOIN localized_texts lw ON lw.tbl_name='ui_texts' AND lw.tbl_column_name='text' AND lw.idx=u.id",
        )?;
        let mut rows = query.query([])?;
        while let Some(row) = rows.next()? {
            let world: Option<String> = row.get(2)?;
            let world = match world.as_deref() {
                None | Some("none" | "main_world") => None,
                _ => row.get(3)?,
            };
            out.zones.insert(row.get(0)?, (row.get(1)?, world));
        }
        Ok(out)
    }

    pub(in crate::hud) fn is_creator(&self, template: u32) -> bool {
        self.creators.contains(&template)
    }
}

pub(super) fn integer(value: Value) -> Option<i32> {
    let number = match value {
        Value::Integer(number) => number as f32,
        Value::Number(number) => number as f32,
        _ => return None,
    };
    Some(if !number.is_finite() || !(-2147483648.0..2147483648.0).contains(&number) {
        i32::MIN // Native CVTTSS2SI invalid result; Rust's saturating cast differs.
    } else {
        number as i32
    })
}

fn page(cache: &Expeditions, all: bool, start: usize) -> (usize, usize, Vec<&ExpeditionMember>) {
    let mut rows: Vec<_> = cache.members.values().filter(|row| all || row.online != 0).collect();
    // BTree order supplies the native character-ID order for equal levels.
    rows.sort_by_key(|row| std::cmp::Reverse(row.level));
    let count = rows.len();
    let start = if start >= count { count / PER_PAGE * PER_PAGE } else { start };
    (count, start, rows.into_iter().skip(start).take(PER_PAGE).collect())
}

pub(super) fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |time| time.as_secs())
}

fn offline(lua: &Lua, last: i64, now: u64) -> mlua::Result<Table> {
    // Original xlcommon's XlGetCurrentFileTime/_time64 and XlDiffTime/_difftime64
    // operate in Unix seconds. Its future check compares unsigned 64-bit values.
    let mut age = now.saturating_sub(last as u64);
    let out = lua.create_table()?;
    for (key, seconds) in [
        ("year", 31_536_000),
        ("month", 2_592_000),
        ("day", 86_400),
        ("hour", 3_600),
        ("minute", 60),
        ("second", 1),
    ] {
        out.set(key, f64::from((age / seconds) as i32 as f32))?;
        age %= seconds;
    }
    Ok(out)
}

fn row(lua: &Lua, received: &ExpeditionMember, position: Table) -> mlua::Result<Table> {
    let out = lua.create_table()?;
    out.set(1, text(lua, &received.name)?)?;
    out.set(2, f64::from(received.level))?;
    out.set(3, lua.create_sequence_from(received.abilities.map(f64::from))?)?;
    out.set(4, f64::from(received.role))?;
    out.set(5, position)?;
    out.set(6, text(lua, &received.memo)?)?;
    out.set(7, received.online != 0)?;
    out.set(8, received.in_party != 0)?;
    Ok(out)
}

fn online(lua: &Lua, received: &ExpeditionMember, game: Option<&super::super::data::GameData>) -> mlua::Result<Table> {
    let out = lua.create_table()?;
    if let Some(game) = game {
        // Native392f9fe7 passes id3 first, zone_id second to39303db0.
        if let Some(name) = game.sub_zone_names.get(&received.id3) {
            out.raw_push(name.as_str())?;
        }
        if let Some((name, world)) = game.expedition.zones.get(&received.zone_id) {
            out.raw_push(name.as_str())?;
            if let Some(world) = world {
                out.raw_push(world.as_str())?;
            }
        }
    }
    Ok(out)
}

pub(super) fn install(lua: &Lua, faction: &Table, bridge: &Shared) -> mlua::Result<()> {
    let shared = bridge.clone();
    faction.set(
        "RequestExpeditionMembers",
        lua.create_function(move |lua, (_, all, start): (Value, Value, Value)| {
            let (Value::Boolean(all), Some(start)) = (all, integer(start)) else {
                return Ok(());
            };
            let start = start.wrapping_sub(1);
            if start < 0 {
                return Ok(());
            }
            let data = shared.borrow();
            let (count, start, received) = page(&data.expedition.cache, all, start as usize);
            let rows = lua.create_table()?;
            for member in received {
                let position = if member.online != 0 {
                    online(lua, member, data.game.get())?
                } else {
                    offline(lua, member.last_world_leave_time, now())?
                };
                rows.raw_push(row(lua, member, position)?)?;
            }
            drop(data);
            aa_ui::lua::fire_ui_event(
                lua,
                "EXPEDITION_MANAGEMENT_MEMBERS_INFO",
                vec![
                    Value::Number(f64::from(count as f32)),
                    Value::Number(f64::from(start as f32)),
                    Value::Table(rows),
                ],
            );
            Ok(())
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_offline_age_keeps_native_calendar_components_and_unsigned_future_guard() -> mlua::Result<()> {
        let lua = Lua::new();
        let now = 1_800_000_000;
        let age = 31_536_000 + 2 * 2_592_000 + 3 * 86_400 + 4 * 3_600 + 5 * 60 + 6;
        let time = offline(&lua, now as i64 - age, now)?;
        for (key, expected) in [
            ("year", 1.),
            ("month", 2.),
            ("day", 3.),
            ("hour", 4.),
            ("minute", 5.),
            ("second", 6.),
        ] {
            assert_eq!(time.get::<f64>(key)?, expected);
        }
        for last in [-1, now as i64, now as i64 + 1] {
            let time = offline(&lua, last, now)?;
            for key in ["year", "month", "day", "hour", "minute", "second"] {
                assert_eq!(time.get::<f64>(key)?, 0.);
            }
        }
        let member = ExpeditionMember {
            name: b"Name\0hidden".to_vec(),
            level: 55,
            abilities: [2, 7, 9],
            role: 255,
            memo: b"Memo\0hidden".to_vec(),
            in_party: 2,
            ..default()
        };
        let member = row(&lua, &member, offline(&lua, now as i64 - 3600, now)?)?;
        assert_eq!(member.raw_len(), 8);
        assert_eq!(member.get::<String>(1)?, "Name");
        assert_eq!(member.get::<String>(6)?, "Memo");
        assert!(!member.get::<bool>(7)? && member.get::<bool>(8)?);
        assert_eq!(member.get::<Table>(5)?.get::<f64>("hour")?, 1.);
        assert_eq!(member.get::<Table>(3)?.get::<f64>(2)?, 7.);
        Ok(())
    }

    #[test]
    fn local_pages_keep_level_ties_character_order_filter_and_exact_multiple_empty_page() {
        let mut cache = Expeditions::default();
        for id in (1..=101).rev() {
            cache.members.insert(
                id,
                ExpeditionMember {
                    character_id: id,
                    level: if id <= 3 { 55 } else { 30 },
                    online: u8::from(id != 1),
                    ..default()
                },
            );
        }
        let (count, start, rows) = page(&cache, false, 0);
        assert_eq!((count, start, rows.len()), (100, 0, 50));
        assert_eq!(rows.iter().take(3).map(|row| row.character_id).collect::<Vec<_>>(), [2, 3, 4]);
        let (count, start, rows) = page(&cache, false, 100);
        assert_eq!((count, start, rows.len()), (100, 100, 0));
        let (count, start, rows) = page(&cache, true, usize::MAX);
        assert_eq!((count, start, rows.len()), (101, 100, 1));
        assert_eq!(rows[0].character_id, 101);
        assert_eq!(integer(Value::Number(f64::NAN)).unwrap().wrapping_sub(1), i32::MAX);
        assert_eq!(integer(Value::Number(1.9)).unwrap().wrapping_sub(1), 0);
    }
}
