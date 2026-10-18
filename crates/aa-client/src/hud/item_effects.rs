//! Original item-result Lua callbacks over receipt-ordered item records.
//! Native operation-controller continuations are separate from these notifications.
use aa_net::{Event, NetEvent, model::Item, state::Inventory};
use aa_ui::UiRuntime;
use bevy::prelude::*;

use super::api::{HudBridge, Shared};
mod grades;
pub(super) use grades::install;

#[derive(Default)]
pub(super) struct View {
    inventory: Inventory,
    owner: Option<u32>,
    active: bool,
    known: bool,
    pending: Vec<Notice>,
}

#[derive(Clone, Debug, PartialEq)]
enum Notice {
    Grade {
        result: u8,
        link: String,
        before: u8,
        after: u8,
    },
    Broadcast {
        name: Vec<u8>,
        result: u8,
        link: String,
        before: u8,
        after: u8,
    },
    Socket {
        success: bool,
        link: String,
        template: f64,
        install: bool,
    },
    Magical {
        success: bool,
        link: String,
        template: f64,
    },
}

fn link(item: Option<&Item>) -> Option<String> {
    // Native394e4ef0 rejects invalid-template0; formatting does not require a catalog name.
    let item = item.filter(|item| item.template_id != 0)?;
    super::itemlinks::link(item)
}

impl View {
    fn current(&self, id: u64) -> Option<&Item> {
        if !self.active || !self.known {
            return None;
        }
        //397ccfb0 searches all28 equipment slots before BAG2, never bank3.
        for slot in self.inventory.equipment.slots.iter().take(28) {
            match slot {
                // Empty native equipment records have ID0. Their first hit stops the lookup;
                // the formatter then rejects template0 instead of falling through to the bag.
                None if id == 0 => return None,
                Some(item) if item.id == id => return Some(item),
                _ => {}
            }
        }
        self.inventory
            .bag_items()
            .find_map(|(_, item)| (item.template_id != 0 && item.id == id).then_some(item))
    }
    fn receive(&mut self, event: &Event) -> Option<Notice> {
        match event {
            Event::CharacterSelected(selected) => {
                *self = Self {
                    owner: Some(selected.character.id),
                    known: true,
                    ..default()
                };
            }
            Event::PlayerSpawn(spawn) => {
                if self.owner != Some(spawn.character().id) {
                    *self = Self {
                        owner: Some(spawn.character().id),
                        known: true,
                        ..default()
                    };
                    self.inventory.apply(&Event::CharacterSelected(Box::new(spawn.state.clone())));
                }
                self.active = true;
            }
            Event::LoginSucceeded(_)
            | Event::ReadyToConnect { .. }
            | Event::LeftWorld { .. }
            | Event::Disconnected { .. }
            | Event::Kicked { .. }
            | Event::PhaseChanged(aa_net::Phase::Authenticating | aa_net::Phase::Connecting) => *self = Self::default(),
            // A lost compressed tail may contain unseen inventory mutations. A fresh selected
            // character establishes a new mirror; an ordinary unknown frame does not erase it.
            Event::GameDecodeFailed { .. } | Event::UnhandledGamePacket { level: 4, len: 1.., .. } => self.known = false,
            Event::ItemsChanged(changes) if changes.truncated => self.known = false,
            _ => {}
        }
        self.inventory.apply(event);
        match event {
            Event::GradeEnchantResult(result) if result.raw_result <= 4 => Some(Notice::Grade {
                result: result.raw_result,
                link: link(result.item.as_ref())?,
                before: result.before_grade,
                after: result.after_grade,
            }),
            Event::GradeEnchantBroadcast(result) => Some(Notice::Broadcast {
                name: result.native_name()?.to_vec(),
                result: result.result.raw_result,
                link: link(result.result.item.as_ref())?,
                before: result.result.before_grade,
                after: result.result.after_grade,
            }),
            Event::LunagemSocketingResult {
                raw_result,
                item_id,
                template_id,
                raw_install,
            } if *raw_install <= 1 => Some(Notice::Socket {
                success: *raw_result == 1,
                link: link(self.current(*item_id))?,
                template: f64::from(*template_id as f32),
                install: *raw_install == 1,
            }),
            Event::LunastoneSocketingResult {
                raw_result,
                item_id,
                template_id,
            } if *raw_result <= 1 => Some(Notice::Magical {
                success: *raw_result == 1,
                link: link(self.current(*item_id))?,
                template: f64::from(*template_id as f32),
            }),
            _ => None,
        }
    }
}

fn emit(rt: &UiRuntime, notice: Notice) -> mlua::Result<()> {
    match notice {
        Notice::Grade {
            result,
            link,
            before,
            after,
        } => rt.fire_event("GRADE_ENCHANT_RESULT", (result, link, before, after)),
        Notice::Broadcast {
            name,
            result,
            link,
            before,
            after,
        } => {
            rt.fire_event(
                "GRADE_ENCHANT_BROADCAST",
                (rt.lua.create_string(name)?, result, link, before, after),
            );
        }
        Notice::Socket {
            success,
            link,
            template,
            install,
        } => rt.fire_event("ITEM_SOCKETING_RESULT", (success, link, template, install)),
        Notice::Magical { success, link, template } => rt.fire_event("ITEM_ENCHANT_MAGICAL_RESULT", (success, link, template)),
    }
    Ok(())
}
fn deliver(rt: Option<&UiRuntime>, state: &Shared, notice: Notice) {
    if let Some(rt) = rt {
        if let Err(error) = emit(rt, notice) {
            warn!("item result: callback: {error}");
        }
    } else {
        state.borrow_mut().item_effects.pending.push(notice);
    }
}
fn dispatch(rt: Option<&UiRuntime>, state: &Shared, event: &Event) {
    let notice = state.borrow_mut().item_effects.receive(event);
    if let Some(notice) = notice {
        deliver(rt, state, notice);
    }
}
fn replay(rt: &UiRuntime, state: &Shared) {
    let pending = std::mem::take(&mut state.borrow_mut().item_effects.pending);
    for notice in pending {
        deliver(Some(rt), state, notice);
    }
}
fn track(mut events: MessageReader<NetEvent>, rt: Option<NonSend<UiRuntime>>, state: NonSend<HudBridge>, built: Res<super::HudBuilt>) {
    let rt = rt.as_deref().filter(|_| built.up);
    if let Some(rt) = rt {
        replay(rt, &state.0);
    }
    for NetEvent(event) in events.read() {
        dispatch(rt, &state.0, event);
    }
}
pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        track
            .before(aa_ui::UiInput)
            .before(super::sync_hud)
            .before(super::prebuild_hud)
            .before(super::build_hud),
    );
}

#[allow(dead_code)]
pub(super) mod fixture;
#[cfg(test)]
mod tests;
