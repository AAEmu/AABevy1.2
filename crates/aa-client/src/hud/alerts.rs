//! What a refused action says: the red text in the middle of the screen and the voice of the character, plus the
//! server messages that go with skills (`ABILITY_CHANGED`).
//!
//! Three things refuse an action:
//!
//! * the client's own check before it asks the server ([`super::usable`]: no target, too far, no mana ...), through
//!   [`UseFailed`];
//! * the server's `SCSkillStarted` with a result and no timeline (`aa_net::skill::SkillStarted::failed`): it answers what
//!   the client could not know;
//! * the server's `SCErrorMsg` (`Event::ServerError`: bag full, not enough money ...).
//!
//! The scripts do the showing, as in the original (`x2ui/centermessage/center_message_manager.alb`): `SKILL_MSG(name, arg1, arg2)`
//! with the result's name (`LACK_MANA`) prints the system text `skill_lack_mana` ("Insufficient mana to use this.") in red,
//! `XLARGE`, 180 px under the top edge of the screen (`systemAlertWindow:AddMessageRefresh`); `SYSMSG(text)` prints a ready text
//! the same way. A refused skill also says it: the character's `<race>_<gender>_system_message` sound pack has one voice line
//! per result (`LACK_MANA`, `NO_TARGET`, `TOO_CLOSE_RANGE` ...); results without a line (`TOO_FAR_RANGE`: the database leaves
//! its sound out) are silent.
//!
//! `--hud-fail NAME T` raises the refusal `NAME` (`LACK_MANA`, `TOO_FAR_RANGE` ...) at T seconds, for screenshots of the text.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use aa_audio::{Category, Engine, PlayOptions};
use aa_net::model::{gender, race};
use aa_net::state::Skills;
use aa_net::{Event, NetEvent};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use mlua::{Function, Table};

use super::HudBuilt;
use super::api::HudBridge;
use super::data::{GameData, Snapshot};
use super::usable::{self, code};
use crate::sound::Packs;

/// A refused use of a skill or item.
#[derive(Message, Clone, Copy, Debug)]
pub struct UseFailed {
    /// The skill (0: none, a scripted test).
    pub skill: u32,
    /// `SkillResult` number ([`usable::code`]).
    pub result: u8,
    /// What the text names (an item or buff id).
    pub value: u32,
    /// The server said no (else the client's own check, before asking it).
    pub server: bool,
}

/// `--hud-fail NAME T`.
#[derive(Resource, Default)]
pub struct ScriptedFails {
    /// (time, result), the next one last.
    steps: Vec<(f32, u8)>,
    /// `--hud-ability-swap NEW OLD T`: (time, new ability, old ability), the next one last.
    swaps: Vec<(f32, u8, u8)>,
    started: Option<f32>,
}

impl ScriptedFails {
    pub fn from_args() -> ScriptedFails {
        let args: Vec<String> = std::env::args().collect();
        let mut steps = Vec::new();
        let mut swaps = Vec::new();
        for (i, a) in args.iter().enumerate() {
            if a == "--hud-ability-swap"
                && let (Some(new), Some(old), Some(t)) = (args.get(i + 1), args.get(i + 2), args.get(i + 3).and_then(|t| t.parse::<f32>().ok()))
                && let (Ok(new), Ok(old)) = (new.parse::<u8>(), old.parse::<u8>())
            {
                swaps.push((t, new, old));
            }
            if a != "--hud-fail" {
                continue;
            }
            let (Some(name), Some(t)) = (args.get(i + 1), args.get(i + 2).and_then(|t| t.parse::<f32>().ok())) else { continue };
            match usable::result_number(name) {
                Some(result) => steps.push((t, result)),
                None => warn!("hud: --hud-fail {name}: not a skill result name"),
            }
        }
        steps.sort_by(|a, b| a.0.total_cmp(&b.0));
        steps.reverse();
        swaps.sort_by(|a, b| a.0.total_cmp(&b.0));
        swaps.reverse();
        ScriptedFails { steps, swaps, started: None }
    }
}

/// The scripts name abilities by their archetype key (`locale.common.abilityCategory.fight`), ids 1..=10.
const ABILITY_NAMES: [&str; 10] = ["fight", "illusion", "adamant", "will", "death", "wild", "magic", "vocation", "romance", "love"];

/// `ABILITY_CHANGED(new, old)`: the scripts print "[old] ability changed to [new] ability" in the skill chat tab and play the learn effect.
fn ability_changed(rt: &UiRuntime, new: u8, old: u8) {
    let name = |id: u8| ABILITY_NAMES.get(usize::from(id).wrapping_sub(1)).copied();
    match (name(new), name(old)) {
        (Some(new_name), Some(old_name)) => {
            info!("hud: ABILITY_CHANGED({new_name}, {old_name})");
            rt.fire_event("ABILITY_CHANGED", (new_name, old_name));
        }
        _ => warn!("hud: ability swap {old} -> {new}: not archetypes 1..=10"),
    }
}

/// The name of an `ErrorMessageType` (`SCErrorMsg`), which is the key of its text in the `ERROR_MSG` group.
pub(super) fn error_name(code: i16) -> Option<&'static str> {
    static NAMES: OnceLock<HashMap<i16, &'static str>> = OnceLock::new();
    NAMES
        .get_or_init(|| {
            include_str!("error_names.txt")
                .lines()
                .filter_map(|l| {
                    let (n, name) = l.split_once(' ')?;
                    Some((n.parse().ok()?, name.trim()))
                })
                .collect()
        })
        .get(&code)
        .copied()
}

/// `X2Locale:LocalizeUiText(<group constant>, key)`, `None` for a key without a text.
pub(super) fn localized(rt: &UiRuntime, group: &str, key: &str) -> Option<String> {
    let globals = rt.lua.globals();
    let locale: Table = globals.get("X2Locale").ok()?;
    let f: Function = locale.get("LocalizeUiText").ok()?;
    let group: i64 = globals.get(group).ok()?;
    let text: String = f.call((locale, group, key)).ok()?;
    (!text.is_empty() && text != key).then_some(text)
}

/// The first argument of `SKILL_MSG`: what the text of the result names.
fn detail(game: Option<&GameData>, result: u8, value: u32) -> String {
    let named = |name: Option<&String>| name.cloned().unwrap_or_default();
    match result {
        code::URK_OWN_ITEM | code::URK_EQUIP_ITEM | code::NEED_REAGENT => named(game.and_then(|g| g.items.get(&value)).map(|i| &i.named.name)),
        code::URK_BUFF | code::URK_NOBUFF | code::URK_TARGET_BUFF | code::HIGHER_BUFF => named(game.and_then(|g| g.buffs.get(&value)).map(|b| &b.named.name)),
        // `formatSkillMessage` compares it with "0": a bow, else an instrument
        code::URK_EQUIP_RANGED => value.to_string(),
        _ if value != 0 => value.to_string(),
        _ => String::new(),
    }
}

/// The voice line of a refusal, in the pack of the character's race and gender.
fn say(engine: &Engine, packs: &Packs, me: &Snapshot, item: &str) -> Option<String> {
    let packs = packs.get()?;
    let who = format!("{}_{}_system_message", race::name(me.race).to_ascii_lowercase(), if me.gender == gender::FEMALE { "female" } else { "male" });
    let path = packs.path(&who, item).or_else(|| packs.path("nuian_male_system_message", item))?;
    engine.play_with(path, None, PlayOptions { category: Some(Category::Voice), ..default() });
    Some(path.to_string())
}

/// Keep death state even while the loading screen is building the HUD. Offers may arrive before the death packet.
pub fn track_death(mut events: MessageReader<NetEvent>, bridge: NonSend<HudBridge>, skills: Res<aa_net::state::Skills>) {
    let data = bridge.0.borrow();
    for NetEvent(event) in events.read() {
        match event {
            Event::PlayerSpawn(_) | Event::CharacterSelected(_) => {
                data.death_ends.set(None);
                data.death_started.set(None);
                data.revive_caster.set(None);
            }
            Event::ResurrectionOffered { caster } => data.revive_caster.set(Some(caster.obj_id)),
            Event::DeathDetails { obj_id, resurrect_wait_ms, .. } if skills.obj_id != 0 && *obj_id == skills.obj_id => {
                let now = super::panels::now_ms(data.start);
                data.death_started.set(Some(now));
                data.death_ends.set(Some(now + (*resurrect_wait_ms).max(0) as u64));
            }
            Event::Resurrected { obj_id, .. } if skills.obj_id != 0 && *obj_id == skills.obj_id => {
                data.death_ends.set(None);
                data.death_started.set(None);
                data.revive_caster.set(None);
            }
            _ => {}
        }
    }
}

/// Shows the refusals and the server messages of this frame.
#[allow(clippy::too_many_arguments)]
pub fn show(
    mut failed: MessageReader<UseFailed>,
    mut events: MessageReader<NetEvent>,
    rt: Option<NonSend<UiRuntime>>,
    built: Res<HudBuilt>,
    bridge: NonSend<HudBridge>,
    skills: Res<Skills>,
    engine: Option<Res<Engine>>,
    packs: Option<Res<Packs>>,
    time: Res<Time>,
    mut script: ResMut<ScriptedFails>,
    mut last_voice: Local<Option<(Instant, &'static str)>>,
) {
    let Some(rt) = rt.filter(|_| built.up) else {
        failed.clear();
        events.clear();
        return;
    };
    let own = skills.obj_id;
    let mut alerts: Vec<UseFailed> = failed.read().copied().collect();
    // scripted refusals, relative to the first frame they were looked at
    let now = time.elapsed_secs();
    let started = *script.started.get_or_insert(now);
    while script.steps.last().is_some_and(|s| now - started >= s.0) {
        let (_, result) = script.steps.pop().unwrap();
        alerts.push(UseFailed { skill: 0, result, value: 0, server: false });
    }
    while script.swaps.last().is_some_and(|s| now - started >= s.0) {
        let (_, new, old) = script.swaps.pop().unwrap();
        ability_changed(&rt, new, old);
    }
    let data = bridge.0.borrow();
    let game = data.game.get();
    for NetEvent(event) in events.read() {
        match event {
            Event::SkillStarted(s) if s.failed() && own != 0 && s.caster.obj_id == own => {
                alerts.push(UseFailed { skill: s.skill_id, result: s.result, value: s.result_value, server: true });
            }
            Event::AbilitySwapped { obj_id, old_ability, ability } if own != 0 && *obj_id == own => ability_changed(&rt, *ability, *old_ability),
            // the death window (`UNIT_DEAD` shows it with the exp lost; the wait before standing up counts down on the HUD clock)
            Event::DeathDetails { obj_id, resurrect_wait_ms, lost_exp, .. } if own != 0 && *obj_id == own => {
                info!("hud: the player died, may stand up in {} ms, lost {lost_exp} exp", resurrect_wait_ms);
                rt.fire_event("UNIT_DEAD", (own.to_string(), i64::from(*lost_exp), 0i64));
            }
            Event::Resurrected { obj_id, .. } if own != 0 && *obj_id == own => {
                rt.fire_event("PLAYER_RESURRECTED", ());
            }
            Event::ServerError { code } => {
                let text = error_name(*code).and_then(|name| localized(&rt, "ERROR_MSG", name));
                match text {
                    Some(text) => {
                        info!("hud: server message {code} {}: {text}", error_name(*code).unwrap_or(""));
                        rt.fire_event("SYSMSG", text);
                    }
                    // no text for it: the red chat line of refused actions
                    None => rt.fire_event("CHAT_FAILED", (format!("Server error {code}"), "")),
                }
            }
            _ => {}
        }
    }
    for a in alerts {
        let name = usable::result_name(a.result);
        let skill = game.and_then(|g| g.skills.get(&a.skill)).map_or("", |s| s.named.name.as_str());
        let arg = detail(game, a.result, a.value);
        let heard = match (&engine, &packs) {
            (Some(engine), Some(packs)) if !last_voice.is_some_and(|(at, item)| item == name && at.elapsed() < Duration::from_millis(1200)) => {
                let path = say(engine, packs, &data.snapshot, name);
                if path.is_some() {
                    *last_voice = Some((Instant::now(), name));
                }
                path
            }
            _ => None,
        };
        info!(
            "hud: {} refused {name:?} ({}) for skill {} {skill:?}: {}; voice {}",
            if a.server { "server" } else { "client" },
            a.value,
            a.skill,
            localized(&rt, "CHAT_SYSTEM_TEXT", &format!("skill_{}", name.to_ascii_lowercase())).unwrap_or_default(),
            heard.as_deref().unwrap_or("none")
        );
        rt.fire_event("SKILL_MSG", (name, arg, ""));
    }
}
