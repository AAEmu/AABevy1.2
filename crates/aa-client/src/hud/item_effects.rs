//! Original item-result Lua callbacks over receipt-ordered item records.
//! Result callbacks run before their admitted operation-controller continuation.
use aa_net::{Event, model::Item};
use aa_ui::UiRuntime;
use bevy::prelude::*;

use super::api::{HudBridge, Shared};
mod grades;
pub(super) use grades::install;

#[derive(Default)]
pub(super) struct View {
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

impl Notice {
    fn receive(data: &super::api::HudData, event: &Event) -> Option<Self> {
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
                link: link(super::enchant::current(data, *item_id))?,
                template: f64::from(*template_id as f32),
                install: *raw_install == 1,
            }),
            Event::LunastoneSocketingResult {
                raw_result,
                item_id,
                template_id,
            } if *raw_result <= 1 => Some(Notice::Magical {
                success: *raw_result == 1,
                link: link(super::enchant::current(data, *item_id))?,
                template: f64::from(*template_id as f32),
            }),
            _ => None,
        }
    }
    fn kind(&self) -> Option<super::enchant::ResultKind> {
        use super::enchant::ResultKind;
        match self {
            Self::Grade { .. } => Some(ResultKind::Grade),
            Self::Socket { .. } => Some(ResultKind::Socket),
            Self::Magical { .. } => Some(ResultKind::Magical),
            Self::Broadcast { .. } => None,
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
pub(super) fn receive(rt: Option<&UiRuntime>, state: &Shared, event: &Event) {
    let notice = Notice::receive(&state.borrow(), event);
    if let Some(notice) = notice {
        let kind = notice.kind();
        deliver(rt, state, notice);
        if let Some(kind) = kind {
            super::enchant::result(rt.map(|rt| &rt.lua), state, kind);
        }
    }
}
pub(super) fn clear(state: &Shared) {
    state.borrow_mut().item_effects.pending.clear();
}
pub(super) fn replay(rt: &UiRuntime, state: &Shared) {
    let pending = std::mem::take(&mut state.borrow_mut().item_effects.pending);
    for notice in pending {
        deliver(Some(rt), state, notice);
    }
}
// One production receipt pass belongs to enchant::track. Fixtures use that same
// dispatcher; results never maintain a second, legacy post-batch item mirror.
fn dispatch(rt: Option<&UiRuntime>, state: &Shared, event: &Event) {
    super::enchant::dispatch(rt, state, event, None);
}

#[allow(dead_code)]
pub(super) mod fixture;
#[cfg(test)]
mod tests;
