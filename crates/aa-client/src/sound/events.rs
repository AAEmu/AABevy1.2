//! The sounds of things that happen to the player, which the original client's C++ plays (the scripts only play the
//! widget sounds and the loot popup, see [`super::ui`]):
//!
//! | what | sound |
//! |---|---|
//! | the level goes up | `interface/levelup` (a chime of 5 s) |
//! | a quest is accepted / handed in / failed / dropped | the `global` pack's `event_quest_<started\|completed\|failed\|dropped>_<kind>` (`quest/accept`, `quest/success`, `quest/fail`, `quest/abandon`; the kind is the quest's `detail_id`) |
//! | a target is selected | `interface/npc_select` for a person NPC, else `interface/target_select` |
//! | talking to an NPC starts / ends | the NPC's sound pack item `<role>_interaction_start` / `_end` (a shopkeeper's greeting), on the voice volume |
//! | items are taken from a corpse | the item's own pickup sound (`items.pickup_sound_id`, else its category's) |
//! | an item is used or worn | its `use_or_equipment_sound_id` |
//! | copper comes in / goes out | `coin/pickup` (`pickup_all` for "loot all") / `coin/use` |
//! | "loot all" is asked | `loot/pick_all` |
//!
//! Nothing here refuses or explains: the red text and the voice of a refused skill are [`crate::hud::alerts`], the loot popup
//! (`event_item_added`) is the script's. What the player did before the world was ready (the equipment and level the server
//! announces at spawn) is not played.

use std::collections::HashMap;

use aa_audio::{Category, Engine, PlayOptions};
use aa_net::interaction::Interact;
use aa_net::items::{ItemTask, container};
use aa_net::model::UnitKind;
use aa_net::quest::status;
use aa_net::state::Units;
use aa_net::{Command, Event, NetCommand, NetEvent};
use bevy::prelude::*;

use super::tables::{Data, Tables};
use super::{Packs, log_enabled};

/// Seconds after the spawn in which the announcements of the server are not sounds.
const QUIET: f64 = 5.0;

const LEVEL_UP: &str = "sounds/x2interface:interface:levelup";
const NPC_SELECT: &str = "sounds/x2interface:interface:npc_select";
const TARGET_SELECT: &str = "sounds/x2interface:interface:target_select";
const COIN_IN: &str = "sounds/x2item:coin:pickup";
const COIN_ALL: &str = "sounds/x2item:coin:pickup_all";
const COIN_OUT: &str = "sounds/x2item:coin:use";
const PICK_ALL: &str = "sounds/x2interface:loot:pick_all";

/// `ItemTaskType`s of items that come off a corpse or a doodad.
const LOOT: u8 = 9;
const LOOT_ALL: u8 = 10;
const AUTO_LOOT: u8 = 35;

#[derive(Default)]
pub(super) struct Heard {
    own: u32,
    level: u8,
    quiet_until: f64,
    target: u32,
    /// When a sound was last played, by name: a burst of the same one is a single sound.
    last: HashMap<&'static str, f64>,
}

/// Plays a sound of the game (2D, the event's own group unless `category`).
struct Sounder<'a> {
    engine: &'a Engine,
    now: f64,
    last: &'a mut HashMap<&'static str, f64>,
}

impl Sounder<'_> {
    /// `path` once in `gap` seconds under `key`.
    fn play(&mut self, key: &'static str, gap: f64, path: &str, category: Option<Category>) {
        if self.last.get(key).is_some_and(|t| self.now - t < gap) {
            return;
        }
        self.last.insert(key, self.now);
        if log_enabled() {
            info!("audio: game sound {key} -> {path}");
        }
        self.engine.play_with(path, None, PlayOptions { category, ..default() });
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn game_sounds(
    mut events: MessageReader<NetEvent>,
    mut commands: MessageReader<NetCommand>,
    engine: Res<Engine>,
    packs: Res<Packs>,
    tables: Res<Tables>,
    units: Option<Res<Units>>,
    time: Res<Time>,
    mut heard: Local<Heard>,
) {
    let (Some(packs), Some(data)) = (packs.get(), tables.get()) else {
        events.clear();
        commands.clear();
        return;
    };
    let now = time.elapsed_secs_f64();
    let heard = &mut *heard;
    let mut out = Sounder { engine: &engine, now, last: &mut heard.last };
    let item_path = |template: u32, used: bool| {
        let (pickup, usage) = data.item_sounds(template);
        let id = if used { usage } else { pickup };
        if id == 0 { None } else { packs.sound(id) }
    };

    let template = |obj_id: u32| {
        units.as_ref().and_then(|u| u.get(obj_id)).filter(|u| matches!(u.kind, UnitKind::Npc { .. } | UnitKind::Mate { .. })).map(|u| u.kind.template_id())
    };

    for NetEvent(event) in events.read() {
        match event {
            Event::PlayerSpawn(s) => {
                heard.own = s.obj_id;
                heard.level = s.unit.level;
                heard.quiet_until = now + QUIET;
            }
            Event::LevelChanged { obj_id, level } if *obj_id == heard.own && heard.own != 0 => {
                if *level > heard.level && heard.level > 0 {
                    out.play("levelup", 1.0, LEVEL_UP, None);
                }
                heard.level = *level;
            }
            Event::QuestStarted(q) => out.quest(packs, data, "started", q.template_id),
            // finished objectives (status READY) are an update, not a sound: the jingle is the hand in
            Event::QuestUpdated(q) => match q.status {
                status::FAILED => out.quest(packs, data, "failed", q.template_id),
                status::DROPPED => out.quest(packs, data, "dropped", q.template_id),
                _ => {}
            },
            Event::QuestCompleted { quest_id, .. } => out.quest(packs, data, "completed", *quest_id),
            Event::EquipmentChanged { obj_id, items } if *obj_id == heard.own && now >= heard.quiet_until => {
                if let Some(path) = items.iter().find_map(|(_, it)| it.as_ref().and_then(|it| item_path(it.template_id, true))) {
                    out.play("equip", 0.15, path, None);
                }
            }
            Event::ItemsChanged(changes) => {
                let looted = matches!(changes.reason, LOOT | LOOT_ALL | AUTO_LOOT);
                let mut money = 0i64;
                let mut picked = None;
                for task in &changes.tasks {
                    match task {
                        ItemTask::Money(delta) => money += i64::from(*delta),
                        ItemTask::Add { container: c, item, .. } if looted && *c == container::INVENTORY && picked.is_none() => {
                            picked = item_path(item.template_id, false);
                        }
                        _ => {}
                    }
                }
                if let Some(path) = picked {
                    out.play("pickup", 0.1, path, None);
                }
                if money > 0 {
                    out.play("coin", 0.1, if changes.reason == LOOT_ALL { COIN_ALL } else { COIN_IN }, None);
                } else if money < 0 {
                    out.play("coin_out", 0.1, COIN_OUT, None);
                }
            }
            _ => {}
        }
    }

    for NetCommand(command) in commands.read() {
        match command {
            Command::Target { obj_id } if *obj_id != 0 && *obj_id != heard.target => {
                heard.target = *obj_id;
                let person = template(*obj_id).and_then(|t| data.npc(t)).is_some_and(|n| n.person);
                out.play("target", 0.1, if person { NPC_SELECT } else { TARGET_SELECT }, None);
            }
            Command::Target { obj_id: 0 } => heard.target = 0,
            Command::Interact(Interact::Npc { obj_id, .. }) => out.greet(packs, data, template(*obj_id), "start"),
            Command::Interact(Interact::NpcEnd { obj_id }) => out.greet(packs, data, template(*obj_id), "end"),
            Command::UseItem { template_id, .. } | Command::UseItemAt { template_id, .. } => {
                if let Some(path) = item_path(*template_id, true) {
                    out.play("use_item", 0.15, path, None);
                }
            }
            Command::LootOpen { loot_all: true, .. } => out.play("loot_all", 0.3, PICK_ALL, None),
            _ => {}
        }
    }
}

impl Sounder<'_> {
    /// The jingle of a quest event, of the quest's kind.
    fn quest(&mut self, packs: &aa_audio::SoundPacks, data: &Data, what: &'static str, quest: u32) {
        let item = format!("event_quest_{what}_{}", data.quest_kind(quest));
        if let Some(path) = packs.path("global", &item) {
            let key = match what {
                "started" => "quest_started",
                "completed" => "quest_completed",
                "failed" => "quest_failed",
                _ => "quest_dropped",
            };
            self.play(key, 0.5, path, None);
        }
    }

    /// The NPC's greeting (`start`) or farewell (`end`): its pack's `<role>_interaction_<when>`, else the shop's.
    fn greet(&mut self, packs: &aa_audio::SoundPacks, data: &Data, template: Option<u32>, when: &'static str) {
        let Some(voice) = template.and_then(|t| data.npc(t)) else { return };
        if voice.pack.is_empty() {
            return;
        }
        let path = [voice.role, "store"].into_iter().find_map(|role| packs.path(&voice.pack, &format!("{role}_interaction_{when}")));
        if let Some(path) = path {
            self.play(if when == "start" { "greet" } else { "farewell" }, 0.5, path, Some(Category::Voice));
        }
    }
}
