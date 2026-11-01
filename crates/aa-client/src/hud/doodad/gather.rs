//! Natural gathering: the interaction skills a resource doodad offers.
//!
//! The 1.2 client builds a doodad's interaction list from its current phase's funcs; the skill of a
//! func is `doodad_funcs.func_skill_id`, else the func's own row: `doodad_func_uses.skill_id` for
//! `DoodadFuncUse` or `doodad_func_fake_uses.fake_skill_id` for `DoodadFuncFakeUse`. The 2026-10-30
//! original-client capture (`tools/evidence/gather/capture-results.txt`) confirms the direct skill and
//! the fake-use fallback: herb 2274 phase 4513 offers 13990 from `func_skill_id`, and quest lamp 2319
//! phase 4600 (empty `func_skill_id`) offers 13166, then 13167 in phase 5086, from `fake_skill_id`
//! only. The `DoodadFuncUse` fallback (`doodad_func_uses.skill_id`, 23 func groups) was not driven:
//! capture-pending. A detail skill that is not a `skills` row (61 rows on the 1.2 data) is left out.
//! Ore veins, rock piles and the legacy shear funcs carry no skill anywhere, so the AAEmu server maps
//! their func type to a gathering skill (`systems/npcs/doodad/gathering.rs`, the `LegacyGatheringSkill`
//! adapter); this client follows the same mapping so it can play the same data. That mapping is a
//! server compatibility choice, not recovered 1.2 client behaviour, and it is capture-pending: the
//! capture could not drive those funcs (no 1.2 world spawns; GM spawns are character-owned and not
//! interactive), so the legacy skill ids stay unverified.

use std::collections::HashMap;

use bevy::log::debug;
use rusqlite::Connection;

/// The AAEmu server's gathering adapter: a func with no skill id at all is used with the gathering
/// skill its type names (`systems/npcs/doodad/gathering.rs`, `LegacyGatheringSkill`). 1.2 client
/// dispatch for these three func types is capture-pending: the 2026-10-30 capture could not drive an
/// OreMine/RockMine/Shear doodad (no world spawns; GM spawns are not interactive).
pub(super) fn legacy_skill(kind: &str) -> Option<u32> {
    Some(match kind {
        "DoodadFuncOreMine" => 11106,
        "DoodadFuncRockMine" => 11107,
        "DoodadFuncShear" => 11642,
        _ => return None,
    })
}

/// The skills a `DoodadFuncUse` / `DoodadFuncFakeUse` func carries in its own detail table
/// (`doodad_func_uses.skill_id` / `doodad_func_fake_uses.fake_skill_id`), keyed by func id. A row whose
/// skill is not a `skills` row has no name, icon or range, so the bar would list it as "skill N": it is
/// left out (on the 1.2 data 55 fake-use and 6 use rows).
#[derive(Default)]
pub(super) struct DetailSkills {
    uses: HashMap<u32, u32>,
    fake_uses: HashMap<u32, u32>,
}

impl DetailSkills {
    pub(super) fn load(conn: &Connection) -> rusqlite::Result<Self> {
        let (uses, skipped_uses) = read(
            conn,
            "SELECT id, skill_id, skill_id IN (SELECT id FROM skills) FROM doodad_func_uses WHERE skill_id IS NOT NULL AND skill_id <> 0",
        )?;
        let (fake_uses, skipped_fake_uses) = read(
            conn,
            "SELECT id, fake_skill_id, fake_skill_id IN (SELECT id FROM skills) FROM doodad_func_fake_uses WHERE fake_skill_id IS NOT NULL AND fake_skill_id <> 0",
        )?;
        let skipped = skipped_uses + skipped_fake_uses;
        if skipped > 0 {
            debug!(
                "doodad: {skipped} detail skills are not in `skills` and offer nothing ({skipped_uses} use, {skipped_fake_uses} fake use)"
            );
        }
        Ok(DetailSkills { uses, fake_uses })
    }

    /// The skill that triggers the func `func_id` of `kind`, when the func's own row has one.
    pub(super) fn skill(&self, kind: &str, func_id: u32) -> Option<u32> {
        match kind {
            "DoodadFuncUse" => self.uses.get(&func_id).copied(),
            "DoodadFuncFakeUse" => self.fake_uses.get(&func_id).copied(),
            kind => legacy_skill(kind),
        }
    }
}

/// Reads a detail table's `SELECT func id, skill id, skill id is a skills row` into func id -> skill id,
/// and counts the rows whose skill is not a `skills` row: those are left out.
fn read(conn: &Connection, sql: &str) -> rusqlite::Result<(HashMap<u32, u32>, usize)> {
    let mut q = conn.prepare(sql)?;
    let mut rows = q.query([])?;
    let (mut skills, mut skipped) = (HashMap::new(), 0);
    while let Some(r) = rows.next()? {
        if crate::hud::data::any_flag(r, 2)? {
            skills.insert(
                crate::hud::data::any_int(r, 0)? as u32,
                crate::hud::data::any_int(r, 1)? as u32,
            );
        } else {
            skipped += 1;
        }
    }
    Ok((skills, skipped))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use aa_net::Event;
    use aa_net::model::Doodad;
    use aa_net::state::Doodads;
    use bevy::prelude::Vec3;
    use rusqlite::{Connection, OpenFlags};

    use super::super::Tables;
    use super::DetailSkills;
    use crate::ingame::combat::skills::SkillBook;

    fn database() -> Option<String> {
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        if std::path::Path::new(&path).exists() {
            Some(path)
        } else {
            eprintln!("skip: no client database at {path}");
            None
        }
    }

    /// The real tables, or `None` (after the skip line) when there is no database. A database that
    /// is there but does not load fails the test: it must not skip silently.
    fn real_tables() -> Option<Tables> {
        let conn =
            Connection::open_with_flags(database()?, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        Some(Tables::load(&conn).unwrap())
    }

    fn uses(tables: &Tables, group: u32) -> Option<Vec<u32>> {
        tables.uses(group).map(|u| u.skills.clone())
    }

    /// The func's own detail row is the fallback the server dispatches on; the three legacy gather
    /// types have no skill in any row and follow the server adapter.
    #[test]
    fn detail_rows_and_the_legacy_adapter_supply_the_gather_skill() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE skills(id INT PRIMARY KEY);
             INSERT INTO skills VALUES (13970), (11107);
             CREATE TABLE doodad_func_uses(id INT, skill_id INT);
             INSERT INTO doodad_func_uses VALUES (1, 13970), (2, 0), (3, NULL);
             CREATE TABLE doodad_func_fake_uses(id INT, fake_skill_id INT);
             INSERT INTO doodad_func_fake_uses VALUES (4, 11107), (5, 0);",
        )
        .unwrap();
        let detail = DetailSkills::load(&conn).unwrap();
        assert_eq!(detail.skill("DoodadFuncUse", 1), Some(13970));
        assert_eq!(detail.skill("DoodadFuncUse", 2), None);
        assert_eq!(detail.skill("DoodadFuncUse", 3), None);
        assert_eq!(detail.skill("DoodadFuncFakeUse", 4), Some(11107));
        assert_eq!(detail.skill("DoodadFuncFakeUse", 5), None);
        assert_eq!(detail.skill("DoodadFuncOreMine", 6), Some(11106));
        assert_eq!(detail.skill("DoodadFuncRockMine", 7), Some(11107));
        assert_eq!(detail.skill("DoodadFuncShear", 8), Some(11642));
        assert_eq!(detail.skill("DoodadFuncBinding", 9), None);
    }

    /// An explicit `func_skill_id` is what the server dispatches first; the detail row and the
    /// legacy adapter only fill an empty one (and duplicates stay out of the bar).
    #[test]
    fn an_explicit_func_skill_wins_over_the_fallbacks() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE skills(id INT PRIMARY KEY);
             INSERT INTO skills VALUES (13970);
             CREATE TABLE doodad_func_uses(id INT, skill_id INT);
             INSERT INTO doodad_func_uses VALUES (1, 13970);
             CREATE TABLE doodad_func_fake_uses(id INT, fake_skill_id INT);",
        )
        .unwrap();
        let detail = DetailSkills::load(&conn).unwrap();
        let mut tables = Tables::default();
        tables.add_resolved(1, 13975, "DoodadFuncUse", 1, &detail);
        assert_eq!(
            uses(&tables, 1),
            Some(vec![13975]),
            "explicit func_skill_id"
        );
        tables.add_resolved(2, 0, "DoodadFuncUse", 1, &detail);
        assert_eq!(uses(&tables, 2), Some(vec![13970]), "detail row fallback");
        tables.add_resolved(3, 0, "DoodadFuncBinding", 1, &detail);
        assert_eq!(uses(&tables, 3), None, "nothing to dispatch");
        tables.add_resolved(4, 11106, "DoodadFuncOreMine", 0, &detail);
        tables.add_resolved(4, 0, "DoodadFuncOreMine", 0, &detail);
        assert_eq!(
            uses(&tables, 4),
            Some(vec![11106]),
            "no duplicate bar entry"
        );
    }

    /// A detail row whose skill is not a `skills` row has no name, icon or range: the bar would list
    /// it as "skill N". Such rows are left out (the 1.2 data has 55 fake-use and 6 use rows like that).
    #[test]
    fn a_detail_skill_that_is_not_in_the_skills_table_is_left_out() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE skills(id INT PRIMARY KEY);
             INSERT INTO skills VALUES (13970), (11107);
             CREATE TABLE doodad_func_uses(id INT, skill_id INT);
             INSERT INTO doodad_func_uses VALUES (1, 13970), (2, 99902);
             CREATE TABLE doodad_func_fake_uses(id INT, fake_skill_id INT);
             INSERT INTO doodad_func_fake_uses VALUES (3, 11107), (4, 99904);",
        )
        .unwrap();
        let detail = DetailSkills::load(&conn).unwrap();
        assert_eq!(detail.skill("DoodadFuncUse", 1), Some(13970));
        assert_eq!(detail.skill("DoodadFuncUse", 2), None, "unknown use skill");
        assert_eq!(detail.skill("DoodadFuncFakeUse", 3), Some(11107));
        assert_eq!(
            detail.skill("DoodadFuncFakeUse", 4),
            None,
            "unknown fake-use skill"
        );
        let mut tables = Tables::default();
        tables.add_resolved(1, 0, "DoodadFuncFakeUse", 4, &detail);
        assert_eq!(uses(&tables, 1), None, "no 'skill N' entry on the bar");
    }

    /// The owner's rice phases through the generic `func_skill_id` path: the seedling (4366) offers Water
    /// 13625 (`max_range` 6, 0 LP) and Uproot 13789 (4, 10 LP), the small phase (4403) Uproot, and the grown
    /// phase (4404) Harvest 13980 (4, 1 LP). The 2026-10-30 capture's F15 did not drive the planted crop's
    /// dynamic bar (`capture-results.txt`: `GetDynamicActionCount` nil), so the bar for an owner-planted crop
    /// is capture-pending; this pins the data the generic slice-1 path reads.
    #[test]
    fn the_planted_rice_phases_offer_water_then_harvest() {
        let Some(tables) = real_tables() else { return };
        assert_eq!(uses(&tables, 4366), Some(vec![13625, 13789]), "seedling: water + uproot");
        assert_eq!(uses(&tables, 4403), Some(vec![13789]), "small rice: uproot");
        assert_eq!(uses(&tables, 4404), Some(vec![13980]), "grown rice: harvest");
    }

    /// A natural ore vein (326) has a `DoodadFuncOreMine` with no skill at all and the legacy
    /// shear has none either; the rock pile (1354) is a `DoodadFuncFakeUse` whose fake skill id is
    /// its interaction. Before the fallback the HUD offered no action for those phases. The
    /// directly bound skills (herb, sheep, trees) stay as they were. The 2026-10-30 1.2 capture
    /// confirms the herb's 13990 and the fake-use lamp's 13166; the legacy ids are unverified.
    #[test]
    fn the_real_tables_offer_gathering_for_legacy_and_fake_use_funcs() {
        let Some(tables) = real_tables() else { return };
        assert_eq!(
            uses(&tables, 4513),
            Some(vec![13990]),
            "herb 2274: DoodadFuncUse.func_skill_id, captured"
        );
        assert_eq!(uses(&tables, 4514), None, "herb 2274 loot phase");
        assert_eq!(
            uses(&tables, 4600),
            Some(vec![13166]),
            "lamp 2319: DoodadFuncFakeUse.fake_skill_id, captured"
        );
        assert_eq!(
            uses(&tables, 5086),
            Some(vec![13167]),
            "lamp 2319 on phase: fake_skill_id, captured in the after-use bar"
        );
        assert_eq!(
            uses(&tables, 255),
            Some(vec![11106]),
            "ore vein 326: DoodadFuncOreMine, server adapter skill"
        );
        assert_eq!(
            uses(&tables, 2244),
            Some(vec![11107]),
            "rock 1354: DoodadFuncFakeUse.fake_skill_id"
        );
        assert_eq!(uses(&tables, 637), Some(vec![11642]), "legacy shear");
        assert_eq!(
            uses(&tables, 5649),
            Some(vec![13802, 13970]),
            "sheep 518: two bound func skills"
        );
        assert_eq!(
            uses(&tables, 888),
            Some(vec![13975, 15874]),
            "spruce 386: two bound func skills"
        );
    }

    /// The Use/FakeUse detail fallback is not gather-only: on the 1.2 data it also opens the quest
    /// interactions (search, open, light, summon) of the doodads that carry them. Pin the number of
    /// func groups it changes so new data cannot silently add a much larger set: 1,940 groups (1,917
    /// through a fake-use row, 23 through a use row), 2,010 with the legacy gather adapter. The
    /// groups are counted through [`DetailSkills`] itself, so the 61 detail rows whose skill is not
    /// a `skills` row stay out of the count (2,001 and 2,071 groups while they were in).
    #[test]
    fn the_detail_fallback_gains_a_pinned_number_of_groups() {
        let Some(path) = database() else { return };
        let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let new = Tables::load(&conn).unwrap();
        let detail_skills = DetailSkills::load(&conn).unwrap();
        // The HUD before this slice: only `doodad_funcs.func_skill_id`.
        let mut old = Tables::default();
        // The same HUD with the Use/FakeUse detail fallback, before the legacy gather adapter.
        let mut detail = Tables::default();
        let mut q = conn
            .prepare("SELECT doodad_func_group_id, func_skill_id, actual_func_type, actual_func_id FROM doodad_funcs ORDER BY doodad_func_group_id, id")
            .unwrap();
        let mut rows = q.query([]).unwrap();
        while let Some(r) = rows.next().unwrap() {
            let group = crate::hud::data::any_int(r, 0).unwrap() as u32;
            let skill = crate::hud::data::any_int(r, 1).unwrap() as u32;
            let kind: Option<String> = r.get(2).unwrap();
            let kind = kind.as_deref().unwrap_or("");
            let func_id = crate::hud::data::any_int(r, 3).unwrap() as u32;
            old.add(group, skill, kind);
            // `add_resolved` also maps the legacy gather kinds, which this count leaves out.
            if matches!(kind, "DoodadFuncUse" | "DoodadFuncFakeUse") {
                detail.add_resolved(group, skill, kind, func_id, &detail_skills);
            } else {
                detail.add(group, skill, kind);
            }
        }
        let mut q = conn
            .prepare("SELECT DISTINCT doodad_func_group_id FROM doodad_funcs")
            .unwrap();
        let groups: Vec<u32> = q
            .query_map([], |r| Ok(crate::hud::data::any_int(r, 0).unwrap() as u32))
            .unwrap()
            .flatten()
            .collect();
        let (mut gained_detail, mut gained_total) = (0, 0);
        for group in groups {
            let before: Vec<u32> = old
                .uses(group)
                .map(|u| u.skills.clone())
                .unwrap_or_default();
            if detail
                .uses(group)
                .is_some_and(|u| u.skills.iter().any(|skill| !before.contains(skill)))
            {
                gained_detail += 1;
            }
            if new
                .uses(group)
                .is_some_and(|u| u.skills.iter().any(|skill| !before.contains(skill)))
            {
                gained_total += 1;
            }
        }
        assert!(
            (1929..=1951).contains(&gained_detail),
            "the detail fallback changed {gained_detail} func groups"
        );
        assert!(
            (1999..=2021).contains(&gained_total),
            "the detail and legacy fallbacks changed {gained_total} func groups"
        );
    }

    /// The bar looks a skill's name, icon and range up in `skills`, so every skill a detail row
    /// contributes is a `skills` row. On the 1.2 data fake use 97 names skill 11651, which has none:
    /// it offers nothing, while the lamp's fake use 520 (13166) is kept.
    #[test]
    fn the_real_detail_skills_are_all_skills_rows() {
        let Some(path) = database() else { return };
        let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let detail = DetailSkills::load(&conn).unwrap();
        let mut q = conn.prepare("SELECT id FROM skills").unwrap();
        let known: HashSet<u32> = q
            .query_map([], |r| Ok(crate::hud::data::any_int(r, 0).unwrap() as u32))
            .unwrap()
            .flatten()
            .collect();
        assert!(!detail.uses.is_empty() && !detail.fake_uses.is_empty());
        for (kind, rows) in [("use", &detail.uses), ("fake use", &detail.fake_uses)] {
            for (func, skill) in rows {
                assert!(
                    known.contains(skill),
                    "{kind} {func} names skill {skill}, which is not a skills row"
                );
            }
        }
        assert_eq!(
            detail.skill("DoodadFuncFakeUse", 97),
            None,
            "skill 11651 is not a skills row"
        );
        assert_eq!(
            detail.skill("DoodadFuncFakeUse", 520),
            Some(13166),
            "the lamp's fake use"
        );
    }

    /// The gather cast is the skill's data: its cast time, looping start clip, hand tool and the
    /// English skill name the cast bar shows (`localized_texts`). The 2026-10-30 1.2 capture measured
    /// the herb's 13990: `SCSkillStarted` real cast time 4000 ms, cast bar text "Gather" filling
    /// 1402 -> 2739 -> 3843, labor -1; the tool, clip and the other skills are data only
    /// (capture-pending).
    #[test]
    fn gathering_skills_resolve_tool_clip_cast_time_and_name() {
        let Some(path) = database() else { return };
        let Some(book) = SkillBook::open() else {
            eprintln!("skip: no client database at {path}");
            return;
        };
        let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let name = |id: u32| -> String {
            conn.query_row(
                "SELECT en_us FROM localized_texts WHERE tbl_name = 'skills' AND tbl_column_name = 'name' AND idx = ?1",
                [id],
                |r| r.get(0),
            )
            .unwrap()
        };
        let labor = |id: u32| -> i64 {
            conn.query_row("SELECT consume_lp FROM skills WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .unwrap()
        };
        let cases = [
            (
                11106,
                5000,
                "fist_ac_mine",
                "ndeco_do_pickax01.cgf",
                "Mine Ore",
                3,
            ),
            (
                11107,
                5000,
                "fist_ac_mine",
                "ndeco_do_pickax01.cgf",
                "Mine Stone",
                0,
            ),
            (
                11642,
                5000,
                "fist_ac_furcuting",
                "pocket_knife.cgf",
                "Shear",
                0,
            ),
            (
                13802,
                5000,
                "fist_ac_furcuting",
                "pocket_knife.cgf",
                "Plucking Livestock",
                5,
            ),
            (
                13970,
                7000,
                "fist_ac_slaughter",
                "dagger_16_3.cgf",
                "Butcher",
                15,
            ),
            (
                13990,
                4000,
                "fist_ac_pulling",
                "ndeco_do_trowel01.cgf",
                "Gather",
                1,
            ),
            (
                16771,
                5000,
                "fist_ac_furcuting",
                "pocket_knife.cgf",
                "Shearing Livestock",
                10,
            ),
            (
                11108,
                5000,
                "fist_ac_pulling",
                "ndeco_do_trowel01.cgf",
                "Gather Herbs",
                1,
            ),
        ];
        for (id, cast_ms, clip, expected_tool, english, consume_lp) in cases {
            let skill = book.skill(id).unwrap_or_else(|| panic!("skill {id}"));
            assert_eq!(skill.cast_ms, cast_ms, "skill {id} cast time");
            assert_eq!(
                skill.start_anim.as_deref(),
                Some(clip),
                "skill {id} cast clip"
            );
            let tool = skill
                .mainhand_tool()
                .unwrap_or_else(|| panic!("skill {id} tool"));
            assert!(
                tool.path.ends_with(expected_tool),
                "skill {id} tool is {}",
                tool.path
            );
            assert_eq!(name(id), english, "skill {id} cast bar text");
            assert_eq!(labor(id), consume_lp, "skill {id} labor");
        }
        // Thinning has no tool: the gathering pose is empty-handed.
        let thinning = book.skill(15874).expect("Thinning");
        assert_eq!(thinning.start_anim.as_deref(), Some("fist_ac_furcuting"));
        assert_eq!(thinning.mainhand_tool(), None);
    }

    /// The server advances the vein to its loot group and announces it with `SCDoodadPhaseChanged`;
    /// the reducer keeps the object with the new phase, the HUD then offers no use for it, and
    /// `SCDoodadRemoved` takes it out of the world.
    #[test]
    fn a_gathered_ore_vein_changes_phase_then_is_removed() {
        let Some(tables) = real_tables() else { return };
        let obj_id = 7;
        let mut state = Doodads::default();
        assert!(state.apply(&Event::DoodadsSpawned(vec![Doodad {
            obj_id,
            template_id: 326,
            owner_obj_id: 0,
            parent_obj_id: 0,
            attach_point: 0,
            pos: aa_net::Pos::new(1.0, 2.0, 3.0),
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: 1.0,
            has_loot_item: false,
            func_group_id: 255,
            owner_id: 0,
            ucc_id: 0,
            item_template_id: 0,
            type2: 0,
            time_left: 0,
            plant_time: 0,
            quest_glow: 0,
            family: 0,
            puzzle_group: 0,
            owner_type: 0,
            owner_db_id: 0,
            data: 0,
        }])));
        assert_eq!(
            uses(&tables, 255),
            Some(vec![11106]),
            "the vein is mineable"
        );
        assert!(state.apply(&Event::DoodadPhaseChanged {
            obj_id,
            func_group_id: 425,
            time_left: 60_000,
            puzzle_group: -1,
            item_template_id: 0,
        }));
        assert_eq!(state.by_id.get(&obj_id).map(|d| d.func_group_id), Some(425));
        assert_eq!(uses(&tables, 425), None, "the loot group is not a use");
        assert!(state.apply(&Event::DoodadsRemoved(vec![obj_id])));
        assert!(!state.by_id.contains_key(&obj_id));
    }

    /// The captured herb: after the cast `SCDoodadPhaseChanged` {2075, 4514, time_left 180000} came and no
    /// `SCDoodadRemoved`. The object stays, 180 s left, and the bar has nothing to offer for it.
    #[test]
    fn a_gathered_herb_moves_to_its_loot_phase_and_stays() {
        let Some(tables) = real_tables() else { return };
        let mut state = Doodads::default();
        let herb = super::super::world_doodad(2075, 2274, 4513, Vec3::new(10990.0, 10123.0, 102.5));
        assert!(state.apply(&Event::DoodadsSpawned(vec![herb])));
        assert_eq!(
            uses(&tables, 4513),
            Some(vec![13990]),
            "the herb is gatherable"
        );
        assert!(state.apply(&Event::DoodadPhaseChanged {
            obj_id: 2075,
            func_group_id: 4514,
            time_left: 180_000,
            puzzle_group: -1,
            item_template_id: 0,
        }));
        let herb = &state.by_id[&2075];
        assert_eq!((herb.func_group_id, herb.time_left), (4514, 180_000));
        assert_eq!(
            uses(&tables, 4514),
            None,
            "the loot phase offers no use: the bar is empty after the cast"
        );
    }

    /// The refusal the capture printed with no labor, "Insufficient Labor Points.", is the text of the HUD's
    /// `NEED_LABOR_POWER` refusal (`CHAT_SYSTEM_TEXT`, group 89, key `skill_need_labor_power`).
    #[test]
    fn the_no_labor_refusal_reads_insufficient_labor_points() {
        let Some(path) = database() else { return };
        let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let name = crate::hud::usable::result_name(crate::hud::usable::code::NEED_LABOR_POWER);
        let key = format!("skill_{}", name.to_ascii_lowercase());
        let text: String = conn
            .query_row(
                "SELECT l.en_us FROM ui_texts u JOIN localized_texts l ON l.tbl_name = 'ui_texts' \
                 AND l.tbl_column_name = 'text' AND l.idx = u.id WHERE u.key = ?1 AND u.category_id = 89",
                [&key],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(text, "Insufficient Labor Points.");
    }
}
