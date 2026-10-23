//! Server notices and small world messages (`aa_net::NoticeEvent`), as the UI events the 1.2 client
//! raises for them; the original scripts show them (centre messages, chat lines, dialogs).
//!
//! | packet | client (x2game) | UI event / what shows |
//! |---|---|---|
//! | `SCNoticeMessage` | `OnNoticeMessage` 391d8d60 | `NOTICE_MESSAGE(kind, alpha, visibleMs, text)` |
//! | `SCSiegeAlert` | `OnSiegeAlert` 391d9520 | `DOMINION_GUARD_TOWER_STATE_NOTICE(alert, name)`, alerts 0..6 only |
//! | `SCConflictZoneHonorPointSum` | 39029b10 / 39028ca0 | `SHOW_ACCUMULATE_HONOR_POINT_DURING_HPW(zone group name, sum)` |
//! | `SCTowerDefStart` / `End` / `WaveStart` | 3936f8c0 / 3936f9d0 / 3936fae0 | `TOWER_DEF_MSG(text, title, "start" / "end" / "progress")` |
//! | `SCCriminalArrested` | 3907ee60 | `SYSMSG(TRIAL_TEXT criminal_arrested(criminal, arrestor))` |
//! | `SCBotSuspectReported` | 3907ca80 | `BOT_SUSPECT_REPORTED(reporter, suspect)` |
//! | `SCProcessingInstance` | 3936d000 | notice dialog `waiting_instance` (title / body texts, the zone name) |
//!
//! The tower defense texts are `tower_defs.start_msg` / `end_msg` / `title_msg` and the wave's
//! `tower_def_progs.msg` (the progs of the defense in id order, `step` indexes them), with `$1` the
//! event zone's name (`zones.display_text`), all English from `localized_texts`.
use std::collections::HashMap;

use aa_net::{Event, NetEvent, NoticeEvent, TowerDefKey};
use aa_ui::UiRuntime;
use bevy::prelude::*;

use super::HudBuilt;
use super::api::HudBridge;

/// The English texts of tower defenses and zone names, read from the game database on first use.
#[derive(Default)]
struct Texts {
    /// `tower_defs.id` -> (start, end, title).
    tower_defs: HashMap<u32, [String; 3]>,
    /// `tower_defs.id` -> wave messages in `tower_def_progs.id` order.
    waves: HashMap<u32, Vec<String>>,
    /// `zones.id` -> display name.
    zones: HashMap<u32, String>,
}

impl Texts {
    fn load() -> Texts {
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        match Self::query(&path) {
            Ok(texts) => texts,
            Err(e) => {
                warn!("hud: notice texts from {path}: {e}");
                Texts::default()
            }
        }
    }

    fn query(path: &str) -> rusqlite::Result<Texts> {
        use rusqlite::{Connection, OpenFlags};
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
        let text = |table: &str, column: &str| -> rusqlite::Result<HashMap<u32, String>> {
            let mut q = conn.prepare("SELECT idx, en_us FROM localized_texts WHERE tbl_name = ?1 AND tbl_column_name = ?2 AND en_us IS NOT NULL")?;
            q.query_map([table, column], |r| Ok((r.get::<_, i64>(0)? as u32, r.get::<_, String>(1)?)))?.collect()
        };
        let (start, end, title) = (text("tower_defs", "start_msg")?, text("tower_defs", "end_msg")?, text("tower_defs", "title_msg")?);
        let mut texts = Texts { zones: text("zones", "display_text")?, ..Texts::default() };
        let mut q = conn.prepare("SELECT id FROM tower_defs")?;
        for id in q.query_map([], |r| r.get::<_, i64>(0))? {
            let id = id? as u32;
            let get = |m: &HashMap<u32, String>| m.get(&id).cloned().unwrap_or_default();
            texts.tower_defs.insert(id, [get(&start), get(&end), get(&title)]);
        }
        let messages = text("tower_def_progs", "msg")?;
        let mut q = conn.prepare("SELECT id, tower_def_id FROM tower_def_progs ORDER BY id")?;
        for row in q.query_map([], |r| Ok((r.get::<_, i64>(0)? as u32, r.get::<_, i64>(1)? as u32)))? {
            let (id, tower_def) = row?;
            texts.waves.entry(tower_def).or_default().push(messages.get(&id).cloned().unwrap_or_default());
        }
        Ok(texts)
    }

    /// The `TOWER_DEF_MSG` arguments of a tower defense message, `None` for an unknown defense (the
    /// client says nothing then) or wave.
    fn tower_def(&self, key: TowerDefKey, event_zone_id: u32, kind: TowerDefMsg) -> Option<(String, String, &'static str)> {
        let [start, end, title] = self.tower_defs.get(&key.tower_def_id)?;
        let (text, kind) = match kind {
            TowerDefMsg::Start => (start, "start"),
            TowerDefMsg::End => (end, "end"),
            TowerDefMsg::Wave(step) => (self.waves.get(&key.tower_def_id)?.get(usize::try_from(step).ok()?)?, "progress"),
        };
        let zone = self.zones.get(&event_zone_id).map_or("", String::as_str);
        Some((text.replace("$1", zone), title.clone(), kind))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TowerDefMsg {
    Start,
    End,
    Wave(u32),
}

pub(super) fn plugin(app: &mut App) {
    app.add_systems(Update, route.after(super::sync_hud).run_if(in_state(crate::Screen::InGame)));
}

fn route(
    mut events: MessageReader<NetEvent>,
    rt: Option<NonSend<UiRuntime>>,
    built: Res<HudBuilt>,
    bridge: NonSend<HudBridge>,
    mut texts: Local<Option<Texts>>,
) {
    for NetEvent(event) in events.read() {
        let Event::Notice(notice) = event else { continue };
        let Some(rt) = rt.as_deref().filter(|_| built.up) else {
            info!("hud: {} before the HUD is up: {notice:?}", notice.name());
            continue;
        };
        let texts = texts.get_or_insert_with(Texts::load);
        if let Err(e) = show(rt, &bridge, texts, notice) {
            warn!("hud: {}: {e}", notice.name());
        }
    }
}

fn show(rt: &UiRuntime, bridge: &HudBridge, texts: &Texts, notice: &NoticeEvent) -> mlua::Result<()> {
    let tower_def = |key: &TowerDefKey, zone: &u32, kind| {
        match texts.tower_def(*key, *zone, kind) {
            Some(args) => rt.fire_event("TOWER_DEF_MSG", args),
            None => info!("hud: tower defense {} {kind:?}: no texts", key.tower_def_id),
        }
    };
    match notice {
        NoticeEvent::Notice { kind, color, visible_ms, message } => {
            rt.fire_event("NOTICE_MESSAGE", (i64::from(*kind), color.clone(), i64::from(*visible_ms), message.clone()));
        }
        NoticeEvent::SiegeAlert { alert, name, .. } if *alert < 7 => {
            rt.fire_event("DOMINION_GUARD_TOWER_STATE_NOTICE", (i64::from(*alert), name.clone()));
        }
        NoticeEvent::HonorPointSum { zone_group, sum } => {
            let data = bridge.0.borrow();
            if let Some((group, _)) = data.game.get().and_then(|g| g.zone_by_group(u32::from(*zone_group))) {
                rt.fire_event("SHOW_ACCUMULATE_HONOR_POINT_DURING_HPW", (group.text.clone(), i64::from(*sum)));
            }
        }
        NoticeEvent::TowerDefStarted { key, event_zone_id } => tower_def(key, event_zone_id, TowerDefMsg::Start),
        NoticeEvent::TowerDefEnded { key, event_zone_id } => tower_def(key, event_zone_id, TowerDefMsg::End),
        NoticeEvent::TowerDefWaveStarted { key, event_zone_id, step } => tower_def(key, event_zone_id, TowerDefMsg::Wave(*step)),
        NoticeEvent::CriminalArrested { criminal, arrestor, .. } => {
            let text: String = rt
                .lua
                .load("return X2Locale:LocalizeUiText(TRIAL_TEXT, 'criminal_arrested', ...)")
                .call((criminal.as_str(), arrestor.as_str()))?;
            rt.fire_event("SYSMSG", text);
        }
        NoticeEvent::BotSuspectReported { reporter, suspect } => rt.fire_event("BOT_SUSPECT_REPORTED", (reporter.clone(), suspect.clone())),
        NoticeEvent::ProcessingInstance { zone_id } => {
            let zone = u32::try_from(*zone_id).ok().and_then(|z| texts.zones.get(&z)).cloned().unwrap_or_default();
            rt.lua
                .load(
                    "local zone = ...
                     X2DialogManager:RequestNoticeDialog(function(wnd)
                       wnd:SetTitle(X2Locale:LocalizeUiText(MSG_BOX_TITLE_TEXT, 'waiting_instance'))
                       wnd:SetContent(X2Locale:LocalizeUiText(MSG_BOX_BODY_TEXT, 'waiting_instance', zone))
                     end, 'waitingInstance')",
                )
                .call::<()>(zone)?;
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts() -> Texts {
        Texts {
            tower_defs: HashMap::from([(18, ["$1 is under attack.".into(), "$1 is safe.".into(), "Defense".into()])]),
            waves: HashMap::from([(18, vec!["First wave".into(), "Second wave at $1".into()])]),
            zones: HashMap::from([(129, "Two Crowns".into())]),
        }
    }

    #[test]
    fn tower_defense_messages_take_the_zone_name() {
        let t = texts();
        let key = TowerDefKey { tower_def_id: 18, zone_group: 3 };
        assert_eq!(t.tower_def(key, 129, TowerDefMsg::Start), Some(("Two Crowns is under attack.".into(), "Defense".into(), "start")));
        assert_eq!(t.tower_def(key, 129, TowerDefMsg::End).unwrap().2, "end");
        assert_eq!(t.tower_def(key, 129, TowerDefMsg::Wave(1)), Some(("Second wave at Two Crowns".into(), "Defense".into(), "progress")));
        // an unknown defense or a wave past the list says nothing, as in the client
        assert_eq!(t.tower_def(key, 129, TowerDefMsg::Wave(2)), None);
        assert_eq!(t.tower_def(TowerDefKey { tower_def_id: 1, zone_group: 3 }, 129, TowerDefMsg::Start), None);
    }

    #[test]
    fn the_database_has_the_tower_defense_texts() {
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        if !std::path::Path::new(&path).exists() {
            return;
        }
        let t = Texts::query(&path).unwrap();
        assert!(t.tower_defs.len() > 20, "{}", t.tower_defs.len());
        assert!(t.tower_defs.values().any(|[start, ..]| !start.is_empty()));
        assert!(!t.waves.is_empty() && !t.zones.is_empty());
    }
}
