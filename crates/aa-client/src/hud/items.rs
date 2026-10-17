//! Chat lines for the changes of the bag: "Received: [Sword]", "Used: [Heart's Beat]".
//!
//! The server announces every change of the containers (`SCItemTaskSuccess`: a reason, then the tasks).
//! The client's C++ turns the ones in the bag into `ADDED_ITEM(itemLinkText, count, taskType, tradeName)`
//! and `REMOVED_ITEM(itemLinkText, count, removeState, taskType, tradeName)` for `chat/chat_msg_event.lua`,
//! which prints "Received:" / "Used:" (`removeState` "consume") / "Removed:" lines. Moves inside the bag
//! (swaps, splits) are silent. The link is the item's name in brackets in its grade's colour (the chat of
//! the captures shows `[Heart's Beat]` in the tan of the basic grade).

use std::collections::HashMap;

use aa_net::items::{ItemTask, container};
use aa_net::state::Inventory;
use aa_net::{Event, NetEvent};
use bevy::prelude::*;

use super::api::HudBridge;
use super::data::GameData;

/// `ItemTaskType`s that only shuffle or wear items: no chat line.
const SILENT: [u8; 6] = [3, 4, 5, 6, 7, 8];
/// `ItemTaskType`s of items used up by a skill: "Used:".
const CONSUME: [u8; 4] = [13, 40, 41, 45];

/// What the bag held when the last events were handled: `item id -> (template, count, grade)`, for the
/// lines of items that left without their count.
type Known = HashMap<u64, (u32, i32, u8)>;

/// The item as the scripts write it: its name in brackets, coloured by its grade.
fn link(game: Option<&GameData>, template_id: u32, grade: u8) -> Option<String> {
    let game = game?;
    let info = game.items.get(&template_id)?;
    if info.named.name.is_empty() {
        return None;
    }
    let grade = if grade == 0 && info.fixed_grade > 0 { info.fixed_grade } else { i64::from(grade) };
    let color = game.grades.get(&grade).map_or("FFFFFFFF", |g| g.1.as_str());
    Some(format!("|c{color}[{}]|r", info.named.name))
}

/// aa-net's item changes as `ADDED_ITEM` / `REMOVED_ITEM`.
pub fn route_items(mut events: MessageReader<NetEvent>, rt: Option<NonSend<aa_ui::UiRuntime>>, bridge: NonSend<HudBridge>, inventory: Res<Inventory>, mut known: Local<Known>) {
    let Some(rt) = rt else {
        events.clear();
        return;
    };
    let shared = bridge.0.borrow().game.clone();
    let game = shared.get();
    for NetEvent(event) in events.read() {
        let Event::ItemsChanged(changes) = event else { continue };
        if SILENT.contains(&changes.reason) {
            continue;
        }
        let reason = i64::from(changes.reason);
        let state = if CONSUME.contains(&changes.reason) { "consume" } else { "remove" };
        for task in &changes.tasks {
            match task {
                ItemTask::Add { container: c, item, .. } if *c == container::INVENTORY => {
                    if let Some(text) = link(game, item.template_id, item.grade) {
                        rt.fire_event("ADDED_ITEM", (text, i64::from(item.count), reason, ""));
                    }
                }
                ItemTask::Count { container: c, item_id, delta, template_id, .. } if *c == container::INVENTORY && *delta != 0 => {
                    let grade = known.get(item_id).map_or(0, |k| k.2);
                    if let Some(text) = link(game, *template_id, grade) {
                        if *delta > 0 {
                            rt.fire_event("ADDED_ITEM", (text, i64::from(*delta), reason, ""));
                        } else {
                            rt.fire_event("REMOVED_ITEM", (text, i64::from(*delta), state, reason, ""));
                        }
                    }
                }
                ItemTask::Remove { container: c, item_id, template_id, .. } if *c == container::INVENTORY => {
                    let (count, grade) = known.get(item_id).map_or((1, 0), |k| (k.1.max(1), k.2));
                    if let Some(text) = link(game, *template_id, grade) {
                        rt.fire_event("REMOVED_ITEM", (text, -i64::from(count), state, reason, ""));
                    }
                }
                _ => {}
            }
        }
    }
    if inventory.is_changed() {
        known.clear();
        known.extend(inventory.bag_items().map(|(_, it)| (it.id, (it.template_id, it.count, it.grade))));
    }
}
