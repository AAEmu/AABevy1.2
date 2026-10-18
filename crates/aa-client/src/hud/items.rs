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

/// Previous bag instances preserve item details for links of removed/consumed items.
type Known = HashMap<u64, aa_net::model::Item>;

/// Native links when instance details survive; otherwise a plain colored label.
fn link(game: Option<&GameData>, template_id: u32, grade: u8, item: Option<&aa_net::model::Item>) -> Option<String> {
    let game = game?;
    let info = game.items.get(&template_id)?;
    if info.named.name.is_empty() {
        return None;
    }
    if let Some(text) = item.and_then(super::itemlinks::link) {
        return Some(text);
    }
    let grade = i64::from(super::store::ui_grade(grade));
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
                    if let Some(text) = link(game, item.template_id, item.grade, Some(item)) {
                        if changes.reason == 54 { rt.fire_event("STORE_BUY", (text.clone(), i64::from(item.count))); }
                        rt.fire_event("ADDED_ITEM", (text, i64::from(item.count), reason, ""));
                    }
                }
                ItemTask::Count { container: c, item_id, delta, template_id, .. } if *c == container::INVENTORY && *delta != 0 => {
                    let item = known.get(item_id).or_else(|| inventory.bag_items().find_map(|(_, item)| (item.id == *item_id).then_some(item)));
                    let grade = item.map_or(0, |item| item.grade);
                    if let Some(text) = link(game, *template_id, grade, item) {
                        if *delta > 0 {
                            if changes.reason == 54 { rt.fire_event("STORE_BUY", (text.clone(), i64::from(*delta))); }
                            rt.fire_event("ADDED_ITEM", (text, i64::from(*delta), reason, ""));
                        } else {
                            if changes.reason == 53 { rt.fire_event("STORE_SELL", (text.clone(), -i64::from(*delta))); }
                            rt.fire_event("REMOVED_ITEM", (text, i64::from(*delta), state, reason, ""));
                        }
                    }
                }
                ItemTask::Remove { container: c, item_id, template_id, .. } if *c == container::INVENTORY => {
                    let item = known.get(item_id);
                    let (count, grade) = item.map_or((1, 0), |item| (item.count.max(1), item.grade));
                    if let Some(text) = link(game, *template_id, grade, item) {
                        if changes.reason == 53 { rt.fire_event("STORE_SELL", (text.clone(), i64::from(count))); }
                        rt.fire_event("REMOVED_ITEM", (text, -i64::from(count), state, reason, ""));
                    }
                }
                _ => {}
            }
        }
    }
    if inventory.is_changed() {
        known.clear();
        known.extend(inventory.bag_items().map(|(_, it)| (it.id, it.clone())));
    }
}
