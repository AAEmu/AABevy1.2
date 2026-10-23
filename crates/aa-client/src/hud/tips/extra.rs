//! The parts of an item tooltip that need more than the template and the grade: what the item instance carries
//! (socketed gems, the magical enchant "rune", the dye, the look it wears, its lock), and the tables behind
//! `gradeEnchantable`, `socketInfo`, `requiredCondition`, `procs`, `lifeSpan`, `isDyeable`, `scalable` ...
//!
//! Ported from the native item info builders (x2game `39421e20` template part, `39423560` instance part,
//! `3941fc20` / `39420310` / `394206d0` equipment parts): what each key means is noted where it is made.

use std::collections::{HashMap, HashSet};

use mlua::{Lua, Table};
use rusqlite::Connection;

use super::super::data::{any_flag, any_int};

/// What an item instance carries besides its template, durability and tempers (the equipment detail block).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ItemExtra {
    /// The item has an equipment detail block (native `detail[0] == 1`).
    pub equip: bool,
    /// Socketed gems (`detail + 0x18`), zero for an empty socket.
    pub gems: [u32; 7],
    /// The magical enchant: an impl 15 item template (`detail + 8`).
    pub rune: u32,
    /// The dye item that colours the item (`detail + 0x14`).
    pub dye: u32,
    /// The template whose look the item wears (`detail + 1`).
    pub image: u32,
    /// `securityState`: 1 locked, 2 not locked, 3 unlocking; 0 when the template is unknown.
    pub security: u8,
    /// Source-backed native detail 8 (u64), carried as a string through Lua.
    pub music_note: Option<u64>,
}

impl ItemExtra {
    /// From a received item (the equipment detail block, the flags and the unlock time).
    pub fn of_item(it: &aa_net::model::Item) -> ItemExtra {
        let mut extra = ItemExtra {
            // 394e4b10: the secure flag (item flag 4) reads locked (1), a running unlock timer unlocking (3), else 2
            security: if it.flags & 4 == 0 {
                2
            } else if it.unsecure_time != 0 {
                3
            } else {
                1
            },
            ..ItemExtra::default()
        };
        if let aa_net::model::ItemDetail::Equip(d) = &it.detail {
            extra.equip = true;
            extra.gems = d.gem_ids;
            extra.rune = d.rune_id;
            extra.dye = d.dye_item_id;
            extra.image = d.image_item_template_id;
        }
        if it.detail.kind() == 8 {
            extra.music_note = it.detail_payload.as_deref()
                .and_then(|bytes| bytes.try_into().ok()).map(u64::from_le_bytes);
        }
        extra
    }

    /// The keys of a slot table (`HUD.bag[i]`) that carry it to the tooltip.
    pub fn write(&self, t: &Table, lua: &Lua) -> mlua::Result<()> {
        if let Some(id) = self.music_note { t.raw_set("musicNote", id.to_string())?; }
        if self.equip {
            let gems = lua.create_table()?;
            for (i, g) in self.gems.iter().enumerate() {
                gems.raw_set(i + 1, i64::from(*g))?;
            }
            t.raw_set("gems", gems)?;
            t.raw_set("rune", i64::from(self.rune))?;
            t.raw_set("dye", i64::from(self.dye))?;
            t.raw_set("image", i64::from(self.image))?;
        }
        if self.security != 0 {
            t.raw_set("sec", i64::from(self.security))?;
        }
        Ok(())
    }

    /// Read them back from a slot table.
    pub fn of(table: Option<&Table>) -> ItemExtra {
        let Some(t) = table else { return ItemExtra::default() };
        let get = |k: &str| t.raw_get::<Option<i64>>(k).ok().flatten().unwrap_or(0).max(0) as u32;
        let mut extra = ItemExtra { security: get("sec") as u8, ..ItemExtra::default() };
        extra.music_note = match t.raw_get::<mlua::Value>("musicNote") {
            Ok(mlua::Value::String(id)) => id.to_str().ok().and_then(|id| id.parse().ok()),
            _ => None,
        };
        if let Ok(Some(gems)) = t.raw_get::<Option<Table>>("gems") {
            extra.equip = true;
            for (i, g) in extra.gems.iter_mut().enumerate() {
                *g = gems.raw_get::<Option<i64>>(i + 1).ok().flatten().unwrap_or(0).max(0) as u32;
            }
            extra.rune = get("rune");
            extra.dye = get("dye");
            extra.image = get("image");
        }
        extra
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Misc {
    /// `items.char_gender_id`: 0 none, 1 male, 2 female.
    gender: u8,
    grade_enchantable: bool,
    /// `exp_abs_lifetime` and `exp_online_lifetime` in minutes.
    life_abs: i32,
    life_online: i32,
    /// `skin_kind_id == 2` of the equipment template (the item is a skin).
    skin: bool,
}

/// An `item_enchanting_gems` row: the level, the grade and the slot group the gem needs.
#[derive(Clone, Copy, Debug, Default)]
struct GemNeeds {
    level: i32,
    grade: Option<u8>,
    group: u32,
}

#[derive(Default)]
pub struct Extra {
    misc: HashMap<u32, Misc>,
    /// Dye item -> the colour's red, green, blue bytes.
    dye_color: HashMap<u32, [u8; 3]>,
    dyeable: HashSet<u32>,
    /// `item_cap_scale_forbids`: items that cannot be tempered.
    scale_forbidden: HashSet<u32>,
    /// `content_configs` 62: the item level from which items have sockets.
    socket_minimum: Option<i32>,
    /// `(slot type, raw grade)` -> sockets.
    socket_count: HashMap<(u32, u8), u8>,
    /// Impl 15 (magical enchant gems).
    enchant_gem: HashMap<u32, GemNeeds>,
    /// Impl 21 (socket gems): level limit and slot group.
    socket_gem: HashMap<u32, (i32, u32)>,
    /// Slot group -> slot types in table order.
    groups: HashMap<u32, Vec<u32>>,
    /// Item -> its proc descriptions (`item_proc_bindings` -> `item_procs`).
    procs: HashMap<u32, Vec<String>>,
}

fn rows<T>(conn: &Connection, sql: &str, mut f: impl FnMut(&rusqlite::Row) -> rusqlite::Result<T>, mut each: impl FnMut(T)) -> rusqlite::Result<()> {
    let mut q = conn.prepare(sql)?;
    let mut r = q.query([])?;
    while let Some(row) = r.next()? {
        each(f(row)?);
    }
    Ok(())
}

impl Extra {
    pub fn load(conn: &Connection) -> rusqlite::Result<Extra> {
        let mut t = Extra::default();
        rows(
            conn,
            "SELECT id, char_gender_id, grade_enchantable, exp_abs_lifetime, exp_online_lifetime FROM items",
            |r| Ok((any_int(r, 0)? as u32, any_int(r, 1)?, any_flag(r, 2).unwrap_or(false), any_int(r, 3)?, any_int(r, 4)?)),
            |(id, gender, grade_enchantable, abs, online)| {
                t.misc.insert(id, Misc { gender: gender.clamp(0, 2) as u8, grade_enchantable, life_abs: abs as i32, life_online: online as i32, skin: false });
            },
        )?;
        for sql in ["SELECT item_id, skin_kind_id FROM item_weapons", "SELECT item_id, skin_kind_id FROM item_armors"] {
            rows(conn, sql, |r| Ok((any_int(r, 0)? as u32, any_int(r, 1)?)), |(id, kind)| {
                if let Some(m) = t.misc.get_mut(&id) {
                    m.skin = kind == 2;
                }
            })?;
        }
        let mut colors: HashMap<i64, i64> = HashMap::new();
        rows(conn, "SELECT id, color_rgb FROM dyeing_colors", |r| Ok((any_int(r, 0)?, any_int(r, 1)?)), |(id, rgb)| {
            colors.insert(id, rgb);
        })?;
        rows(conn, "SELECT item_id, dyeing_color_id FROM item_dyeings", |r| Ok((any_int(r, 0)? as u32, any_int(r, 1)?)), |(item, color)| {
            if let Some(rgb) = colors.get(&color) {
                // `color_rgb` is 0x00BBGGRR
                t.dye_color.insert(item, [*rgb as u8, (*rgb >> 8) as u8, (*rgb >> 16) as u8]);
            }
        })?;
        rows(conn, "SELECT item_id FROM dyeable_items", |r| Ok(any_int(r, 0)? as u32), |id| {
            t.dyeable.insert(id);
        })?;
        rows(conn, "SELECT item_id FROM item_cap_scale_forbids", |r| Ok(any_int(r, 0)? as u32), |id| {
            t.scale_forbidden.insert(id);
        })?;
        t.socket_minimum = conn.query_row("SELECT value FROM content_configs WHERE id=62", [], |r| any_int(r, 0)).ok().map(|v| v as i32);
        rows(conn, "SELECT slot_id, grade_id, num_socket FROM item_socket_num_limits", |r| Ok(((any_int(r, 0)? as u32, any_int(r, 1)? as u8), any_int(r, 2)? as u8)), |(key, n)| {
            t.socket_count.insert(key, n);
        })?;
        rows(
            conn,
            "SELECT item_id, equip_slot_group_id, equip_level, item_grade_id FROM item_enchanting_gems",
            |r| Ok((any_int(r, 0)? as u32, any_int(r, 1)? as u32, any_int(r, 2)? as i32, any_int(r, 3).ok())),
            |(id, group, level, grade)| {
                let grade = grade.filter(|g| (0..=255).contains(g)).map(|g| g as u8);
                t.enchant_gem.entry(id).or_insert(GemNeeds { level, grade, group });
            },
        )?;
        let mut limits: HashMap<u32, i32> = HashMap::new();
        rows(conn, "SELECT item_id, level FROM item_socket_level_limits", |r| Ok((any_int(r, 0)? as u32, any_int(r, 1)? as i32)), |(id, level)| {
            limits.insert(id, level);
        })?;
        rows(conn, "SELECT item_id, equip_slot_group_id FROM item_sockets", |r| Ok((any_int(r, 0)? as u32, any_int(r, 1)? as u32)), |(id, group)| {
            t.socket_gem.entry(id).or_insert((limits.get(&id).copied().unwrap_or(0), group));
        })?;
        rows(conn, "SELECT equip_slot_group_id, equip_slot_type_id FROM equip_slot_group_maps ORDER BY id", |r| Ok((any_int(r, 0)? as u32, any_int(r, 1)? as u32)), |(group, slot)| {
            t.groups.entry(group).or_default().push(slot);
        })?;
        let mut proc_text: HashMap<u32, String> = HashMap::new();
        rows(conn, "SELECT id, description FROM item_procs", |r| Ok((any_int(r, 0)? as u32, r.get::<_, Option<String>>(1)?.unwrap_or_default())), |(id, text)| {
            proc_text.insert(id, text);
        })?;
        rows(conn, "SELECT item_id, proc_id FROM item_proc_bindings ORDER BY id", |r| Ok((any_int(r, 0)? as u32, any_int(r, 1)? as u32)), |(item, proc_id)| {
            if let Some(text) = proc_text.get(&proc_id) {
                t.procs.entry(item).or_default().push(text.clone());
            }
        })?;
        Ok(t)
    }

    /// `gradeEnchantable` (`39422d36`): the template allows it, and the item is not at the last grade (raw id 11).
    pub fn grade_enchantable(&self, template: u32, raw_grade: i32) -> bool {
        self.misc.get(&template).is_some_and(|m| m.grade_enchantable) && raw_grade != 11
    }

    /// `gender` (`3956fdd0`).
    pub fn gender(&self, template: u32) -> &'static str {
        match self.misc.get(&template).map_or(0, |m| m.gender) {
            1 => "male",
            2 => "female",
            _ => "none",
        }
    }

    /// `lifeSpan` (`397cd400`): the shortest of the template's lifetimes, in seconds.
    pub fn life_span(&self, template: u32) -> Option<f64> {
        let m = self.misc.get(&template)?;
        [m.life_abs, m.life_online].into_iter().filter(|v| *v > 0).min().map(|v| f64::from(v as f32) * 60.0)
    }

    /// The equipment template is a skin (`useAsSkin`).
    pub fn is_skin(&self, template: u32) -> bool {
        self.misc.get(&template).is_some_and(|m| m.skin)
    }

    /// `dyeingRed` / `Green` / `Blue` of a dye item (`39422400`): the colour's bytes over 255, to the power 1 / 2.2.
    pub fn dye_color(&self, template: u32) -> Option<[f32; 3]> {
        let rgb = self.dye_color.get(&template)?;
        Some(rgb.map(|c| f64::from(c as f32 * 0.003_921_569).powf(0.454_545_44) as f32))
    }

    pub fn is_dyeable(&self, template: u32) -> bool {
        self.dyeable.contains(&template)
    }

    /// `scalable` (`39423c27`): no temper yet and the template is not forbidden.
    pub fn scalable(&self, template: u32, temper: (u16, u16)) -> bool {
        temper == (0, 0) && !self.scale_forbidden.contains(&template)
    }

    /// The sockets an equipment of `slot_type` and `raw_grade` at item level `level` has (`395aca00`).
    pub fn max_sockets(&self, slot_type: u32, raw_grade: u8, level: i32) -> u8 {
        if self.socket_minimum.is_some_and(|min| level < min) {
            return 0;
        }
        self.socket_count.get(&(slot_type, raw_grade)).copied().unwrap_or(0)
    }

    /// The proc descriptions of an item template (`procs`).
    pub fn procs(&self, template: u32) -> &[String] {
        self.procs.get(&template).map_or(&[], Vec::as_slice)
    }

    /// `requiredCondition` of an enchanting or socket gem (`3941db80`, `39422d36` case 0x15): `equipLevel`, `itemGrade` (the
    /// grade's name through `grade_name`) and `equipSlotTypes` (`3941da50`: the group's slot names, "allEquip" for none).
    pub fn requirements(&self, lua: &Lua, impl_id: i64, template: u32, grade_name: impl Fn(u8) -> Option<String>) -> mlua::Result<Option<Table>> {
        let (level, grade, group) = match impl_id {
            15 => {
                let Some(n) = self.enchant_gem.get(&template) else { return Ok(None) };
                (n.level, n.grade, n.group)
            }
            21 => {
                let Some((level, group)) = self.socket_gem.get(&template) else { return Ok(None) };
                (*level, None, *group)
            }
            _ => return Ok(None),
        };
        let t = lua.create_table()?;
        if level != 0 {
            t.raw_set("equipLevel", f64::from(level as f32))?;
        }
        if let Some(name) = grade.and_then(grade_name) {
            t.raw_set("itemGrade", name)?;
        }
        let slots = lua.create_table()?;
        let mut n = 0;
        for slot in self.groups.get(&group).into_iter().flatten() {
            if let Some(name) = super::super::tipitem::slot_type_name(*slot) {
                n += 1;
                slots.raw_set(n, name)?;
            }
        }
        if n == 0 {
            slots.raw_set(1, "allEquip")?;
        }
        t.raw_set("equipSlotTypes", slots)?;
        Ok(Some(t))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tables() -> Option<Extra> {
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
        Extra::load(&conn).ok()
    }

    #[test]
    fn the_item_tables_load_and_answer() {
        let Some(t) = tables() else { return };
        // a Weapon Regrade Scroll is not itself regradable; the Crushing Grip is, up to the last grade
        assert!(t.grade_enchantable(24488, 1));
        assert!(!t.grade_enchantable(24488, 11));
        assert!(!t.grade_enchantable(28298, 0));
        // the dye "Dark Red" has a colour, the red channel leads
        let [r, g, b] = t.dye_color(29382).expect("dye colour");
        assert!(r > g && r > b && r <= 1.0, "{r} {g} {b}");
        assert!(t.max_sockets(17, 4, 45) == 3 && t.max_sockets(17, 4, 1) == 0 && t.max_sockets(17, 0, 45) == 0);
        assert_eq!(t.gender(24488), "none");
        assert!(t.scalable(24488, (0, 0)) && !t.scalable(24488, (101, 0)));
        assert!(!t.procs(24461).is_empty() || t.procs.values().any(|v| !v.is_empty()));
    }

    #[test]
    fn gem_requirements_name_the_slots() {
        let Some(t) = tables() else { return };
        let lua = Lua::new();
        // Faint Red Lunagem: Strong Blow (impl 21)
        let need = t.requirements(&lua, 21, 30907, |_| None).unwrap().expect("socket gem");
        let slots: Table = need.get("equipSlotTypes").unwrap();
        assert!(slots.raw_len() >= 1);
        // a Lunafrost-style magical enchant gem (impl 15) names a level or a grade
        let id = t.enchant_gem.keys().copied().min().unwrap();
        let need = t.requirements(&lua, 15, id, |g| Some(format!("g{g}"))).unwrap().expect("enchant gem");
        assert!(need.get::<Option<f64>>("equipLevel").unwrap().is_some() || need.get::<Option<String>>("itemGrade").unwrap().is_some());
    }

    #[test]
    fn an_item_instance_round_trips_through_its_slot_table() {
        let lua = Lua::new();
        let extra = ItemExtra { equip: true, gems: [11, 0, 33, 0, 0, 0, 0], rune: 14694, dye: 29382, image: 24488, security: 3, music_note: None };
        let t = lua.create_table().unwrap();
        extra.write(&t, &lua).unwrap();
        assert_eq!(ItemExtra::of(Some(&t)), extra);
        assert_eq!(ItemExtra::of(None), ItemExtra::default());
    }

    #[test]
    fn the_lock_follows_the_secure_flag_and_the_unlock_timer() {
        let mut it = aa_net::model::Item::default();
        assert_eq!(ItemExtra::of_item(&it).security, 2);
        it.flags = 4;
        assert_eq!(ItemExtra::of_item(&it).security, 1);
        it.unsecure_time = 99;
        assert_eq!(ItemExtra::of_item(&it).security, 3);
    }
}
