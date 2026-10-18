//! Native trial tables over received fields; unresolved spatial strings remain explicitly absent.
use super::*;
use aa_ui::uitext::UiTexts;
use mlua::ObjectLike;

pub(super) fn ready(received: &Trials) -> bool {
    received.defendant.is_some()
        && received.crime_kind_counts.is_some()
        && received.records_total.is_some()
        && received.records_raw_unknown.is_some()
        && received.sentence_time_ms.is_some()
        && received.history.is_some()
}

pub(super) fn faction(lua: &Lua, data: &super::super::api::HudData, id: u32) -> mlua::Result<mlua::LuaString> {
    if id < 100 {
        return lua.create_string("");
    }
    if let Some(name) = data.expedition.faction_name(id) {
        return text(lua, name);
    }
    text(
        lua,
        data.game
            .get()
            .and_then(|game| game.factions.get(&id))
            .map_or(&[][..], |name| name.as_bytes()),
    )
}

pub(super) fn race(lua: &Lua, texts: &UiTexts, id: u8) -> mlua::Result<mlua::LuaString> {
    let races = [
        "none",
        "nuian",
        "fairy",
        "dwarf",
        "elf",
        "hariharan",
        "ferre",
        "returned",
        "warborn",
    ];
    let value = races.get(usize::from(id)).map_or("invalid char_race", |race| {
        texts.get(&(47, (*race).into())).map_or("", String::as_str)
    });
    text(lua, value.as_bytes())
}

pub(super) fn crime_data(lua: &Lua, data: &super::super::api::HudData, texts: &UiTexts) -> mlua::Result<Table> {
    let received = data.trial.current();
    let out = lua.create_table()?;
    out.raw_set("isJury", received.is_jury)?;
    if let Some(defendant) = &received.defendant {
        out.raw_set("defendant", text(lua, &defendant.name)?)?;
        out.raw_set("race", race(lua, texts, defendant.race)?)?;
        out.raw_set("faction", faction(lua, data, defendant.faction_id)?)?;
    }
    if let Some(counts) = received.crime_kind_counts {
        for (key, value) in ["murder", "assault", "theft", "etc"].into_iter().zip(counts) {
            out.raw_set(key, signed(value))?;
        }
    }
    if let Some(total) = received.records_total {
        out.raw_set("total", signed(total))?;
    }
    if let Some(sentence) = received.sentence_time_ms {
        out.raw_set("defaultSentenceTime", unsigned(sentence as u32))?;
    }
    if let Some(history) = &received.history {
        for (key, value) in [
            ("crimePoint", history.crime_point),
            ("arrest", history.arrest),
            ("acceptGuilty", history.accept_guilty),
            ("acceptTrial", history.accept_trial),
            ("notGuilty", history.not_guilty),
            ("guilty", history.guilty),
            ("totalSeconds", history.total_seconds),
            ("curSeconds", history.current_seconds),
        ] {
            out.raw_set(key, signed(value))?;
        }
        if let Some(kind) = received.records_raw_unknown {
            out.raw_set(
                "totalRecords",
                signed(if kind == 0 {
                    history.crime_record_count
                } else {
                    history.bot_report
                }),
            )?;
        }
    }
    Ok(out)
}

fn report_date(lua: &Lua, texts: &UiTexts, timestamp: i64) -> mlua::Result<String> {
    // The original uses the local CRT calendar. Its invalid/uninitialized tm domain is refused.
    if !(0..=32_535_244_799).contains(&timestamp) || !texts.contains_key(&(102, "year_to_minute".into())) {
        return Ok(String::new());
    }
    let os: Table = lua.globals().get("os")?;
    let Some(date) = os.call_function::<Option<Table>>("date", ("*t", timestamp)).ok().flatten() else {
        return Ok(String::new());
    };
    let args = ["year", "month", "day", "hour", "min"].map(|key| date.raw_get::<i32>(key).map(|value| value.to_string()));
    let args = args.into_iter().collect::<mlua::Result<Vec<_>>>()?;
    Ok(aa_ui::uitext::localize(texts, 102, "year_to_minute", &args))
}

pub(super) fn records(lua: &Lua, data: &super::super::api::HudData, texts: &UiTexts, page: i32) -> mlua::Result<Table> {
    let out = lua.create_table()?;
    let page = page.wrapping_sub(1);
    if page < 0 {
        return Ok(out);
    }
    let start = page as usize * 10;
    for record in data.trial.current().records.iter().skip(start).take(10) {
        let row = lua.create_table()?;
        row.raw_set(1, (record.id as i32).to_string())?;
        row.raw_set(2, text(lua, &record.reporter_name)?)?;
        row.raw_set(3, unsigned(u32::from(record.crime_kind)))?;
        row.raw_set(4, text(lua, &record.victim_name)?)?;
        row.raw_set(5, text(lua, &record.message)?)?;
        let id = if record.victim_expedition_id == 0 {
            record.victim_faction_id
        } else {
            record.victim_expedition_id
        };
        row.raw_set(6, faction(lua, data, id)?)?;
        let game = data.game.get();
        row.raw_set(
            7,
            text(
                lua,
                game.and_then(|game| game.quests.doodads.get(&record.doodad_template_id))
                    .map_or(&[][..], |name| name.as_bytes()),
            )?,
        )?;
        row.raw_set(
            8,
            text(
                lua,
                game.and_then(|game| game.skills.get(&record.used_skill_id))
                    .map_or(&[][..], |skill| skill.named.name.as_bytes()),
            )?,
        )?;
        // Source uses actual current-world 3D area geometry and the main-world layer point query.
        // The existing map/player heuristics cannot supply this string faithfully.
        row.raw_set(9, "")?;
        row.raw_set(10, report_date(lua, texts, record.report_time)?)?;
        out.raw_push(row)?;
    }
    Ok(out)
}
