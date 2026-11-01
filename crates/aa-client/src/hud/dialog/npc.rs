//! Ordinary NPC query ownership. Local stamps qualify callbacks, never wire replies.
//!
//! SC07b has no nonce. A fresh user intent may replace a cancelled/missing query;
//! an identical old reply can then be indistinguishable. Do not turn that wire
//! limitation into an indefinite lock on all subsequent ordinary interaction.
use aa_net::interaction::{Interact, InteractEvent};
use aa_net::{Event, model::UnitKind};
use std::collections::HashMap;
mod owners;
mod availability;
#[allow(dead_code)] // Actual orientation/admission providers remain separately qualified.
pub(super) mod candidates;
pub(super) mod fixture;
#[cfg(test)]
mod runtime_tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Scope {
    epoch: u64,
    attempt: u64,
    selection: u64,
    pub(super) character: u32,
    pub(super) actor: u32,
    pub(super) npc: (u32, u32),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spawn_npc(flow: &mut Flow, id: u32) {
        let mut unit = crate::hud::expansion::fixture::spawn().unit;
        unit.obj_id = id;
        unit.kind = UnitKind::Npc {
            template_id: 42,
            owner_id: 0,
        };
        flow.observe(&Event::UnitSpawned(Box::new(unit)), true);
    }

    fn ready() -> (Flow, (u32, u32)) {
        let spawn = crate::hud::expansion::fixture::spawn();
        let identity = (spawn.character().id, spawn.obj_id);
        let mut flow = Flow::default();
        flow.observe(&Event::PlayerSpawn(Box::new(spawn)), true);
        flow.observe(&Event::EnteredWorld, true);
        spawn_npc(&mut flow, 200);
        spawn_npc(&mut flow, 201);
        flow.begin_frame();
        flow.close_owed = false;
        (flow, identity)
    }

    fn list(id: u32, skills: &[u32]) -> InteractEvent {
        InteractEvent::SkillList {
            npc_obj_id: id,
            obj_id: 0,
            extra_info: 0,
            pick_id: 0,
            mouse_button: 2,
            modifier_keys: 0,
            interactable: true,
            skills: skills.to_vec(),
        }
    }

    fn offer(flow: &mut Flow, identity: (u32, u32), id: u32, skills: &[u32]) -> Selected {
        assert!(flow.query(identity, (id, 42)).is_some());
        flow.receive(&list(id, skills), true);
        flow.derive(false);
        flow.take_default().unwrap()
    }

    #[test]
    fn discovery_is_exact_zero_echo_query_without_catalog_or_admission() {
        let (mut flow, identity) = ready();
        let request = flow.query(identity, (200, 42)).unwrap();
        assert_eq!(
            request,
            Interact::Start {
                npc_obj_id: 200,
                obj_id: 0,
                extra_info: 0,
                pick_id: 0,
                mouse_button: 2,
                modifier_keys: 0
            }
        );
        assert!(flow.active().is_none());
        assert!(flow.candidate().is_none());
        assert!(
            flow.query(identity, (200, 42)).is_some(),
            "a later explicit intent may retry an unanswered query"
        );
        assert!(
            flow.query((identity.0 + 1, identity.1), (201, 42))
                .is_none()
        );
        assert!(flow.query(identity, (201, 43)).is_none());
    }

    #[test]
    fn every_echo_field_is_checked_without_using_zero_as_actor_identity() {
        for field in 0..6 {
            let (mut flow, identity) = ready();
            flow.query(identity, (200, 42)).unwrap();
            let mut wrong = list(200, &[12087]);
            let InteractEvent::SkillList {
                npc_obj_id,
                obj_id,
                extra_info,
                pick_id,
                mouse_button,
                modifier_keys,
                ..
            } = &mut wrong
            else {
                unreachable!()
            };
            match field {
                0 => *npc_obj_id = 201,
                1 => *obj_id = identity.1,
                2 => *extra_info = 1,
                3 => *pick_id = 1,
                4 => *mouse_button = 1,
                _ => *modifier_keys = 1,
            }
            flow.receive(&wrong, true);
            assert!(flow.candidate().is_none(), "field{field}");
            assert!(flow.pending.is_some());
            flow.receive(&list(200, &[12087]), true);
            assert!(flow.candidate().is_some());
            flow.derive(false);
            assert_eq!(flow.take_default().unwrap().scope.actor, identity.1);
        }
    }

    #[test]
    fn raw_order_zero_duplicates_and_quest_bound_are_preserved() {
        let (mut flow, identity) = ready();
        let first = offer(&mut flow, identity, 200, &[0, 16912, 16912, 12087]);
        assert_eq!(first.skill, 0);
        assert_eq!(flow.choose(16912).unwrap().index, 1);
        assert!(flow.choose(22780).is_none());
        assert!(flow.take_default().is_none());
        flow.query(identity, (200, 42)).unwrap();
        let raw = [12087, 0, 16912, 16912, 1, 2, 3, 4, 5, 6];
        flow.receive(&list(200, &raw), true);
        flow.derive(true);
        assert_eq!(flow.offer.as_ref().unwrap().raw, raw);
        assert_eq!(
            flow.offer.as_ref().unwrap().effective,
            [13877, 12087, 0, 16912, 16912, 1, 2, 3, 4]
        );
        assert_eq!(flow.take_default().unwrap().skill, 13877);
    }

    #[test]
    fn false_interactable_preserves_received_actions_without_quest_augmentation() {
        let (mut flow, identity) = ready();
        flow.query(identity, (200, 42)).unwrap();
        let mut receipt = list(200, &[12087, 0, 16912]);
        let InteractEvent::SkillList { interactable, .. } = &mut receipt else {
            unreachable!()
        };
        *interactable = false;
        flow.receive(&receipt, true);
        flow.derive(true);
        let selected = flow.take_default().unwrap();
        assert_eq!(selected.skill, 12087);
        assert!(flow.selected_current(selected));
        assert_eq!(flow.choose(16912).unwrap().index, 2);
        assert_eq!(flow.offer.as_ref().unwrap().effective, [12087, 0, 16912]);
    }

    #[test]
    fn empty_new_candidate_preserves_prior_admission_and_consumes_default() {
        let (mut flow, identity) = ready();
        let a = offer(&mut flow, identity, 200, &[12087]);
        assert!(flow.stage(a) && flow.finish(a, true));
        flow.query(identity, (201, 42)).unwrap();
        flow.receive(&list(201, &[]), true);
        flow.derive(false);
        assert!(flow.take_default().is_none());
        assert!(flow.admitted(a.scope));
        assert!(!flow.selected_current(a));
    }

    #[test]
    fn callback_cancel_distinguishes_inflight_selection_from_retained_activity() {
        let (mut flow, identity) = ready();
        let a = offer(&mut flow, identity, 200, &[12087]);
        assert!(flow.stage(a));
        flow.cancel(Some(a.scope));
        assert!(
            flow.finish(a, true),
            "inactive public cancel cannot veto changed-target tail"
        );
        let b = offer(&mut flow, identity, 200, &[13238]);
        assert!(flow.stage(b));
        assert_eq!(flow.cancel(Some(a.scope)), Some(a));
        assert!(
            flow.finish(b, false),
            "same-target cancellation still permits tail065"
        );
        assert!(
            flow.active().is_none(),
            "same-target tail must not reinstall activity"
        );
        assert!(
            flow.candidate().is_some(),
            "native cancel retains copied offer"
        );
    }

    #[test]
    fn ordered_remove_and_same_id_reuse_retires_received_candidate() {
        let (mut flow, identity) = ready();
        let a = offer(&mut flow, identity, 200, &[16912]);
        flow.observe(&Event::UnitsRemoved(vec![200]), true);
        spawn_npc(&mut flow, 200);
        flow.receive(&list(200, &[16912]), true);
        assert!(!flow.selected_current(a));
        assert!(flow.candidate().is_none());
        assert!(flow.query(identity, (200, 42)).is_some());
    }

    #[test]
    fn removal_of_new_candidate_does_not_revoke_a_different_admitted_npc() {
        let (mut flow, identity) = ready();
        let a = offer(&mut flow, identity, 200, &[12087]);
        assert!(flow.stage(a) && flow.finish(a, true));
        flow.query(identity, (201, 42)).unwrap();
        flow.observe(&Event::UnitsRemoved(vec![201]), true);
        assert!(flow.admitted(a.scope));
        assert!(flow.pending.is_none() && flow.candidate().is_none());
        assert!(!flow.close_owed);
    }

    #[test]
    fn hidden_receipts_are_consumed_without_replaying_ui_on_resume() {
        let (mut flow, identity) = ready();
        flow.query(identity, (200, 42)).unwrap();
        flow.receive(&list(200, &[12087]), false);
        assert!(flow.pending.is_none() && flow.candidate().is_none());
        flow.receive(&list(200, &[12087]), true);
        assert!(
            flow.candidate().is_none(),
            "unsolicited duplicate cannot reopen"
        );
    }

    #[test]
    fn new_user_intent_can_replace_a_missing_query_with_wire_ambiguity_explicit() {
        let (mut flow, identity) = ready();
        flow.query(identity, (200, 42)).unwrap();
        flow.query(identity, (201, 42)).unwrap();
        flow.receive(&list(200, &[12087]), true);
        assert!(flow.candidate().is_none());
        flow.receive(&list(201, &[13238]), true);
        assert_eq!(flow.candidate().unwrap().npc, (201, 42));
        // After cancellation a fresh identical query has the same wire echo.
        // Local counters cannot tell an old identical reply from its new reply.
        let scope = flow.candidate().unwrap();
        flow.cancel(Some(scope));
        flow.query(identity, (201, 42)).unwrap();
        flow.receive(&list(201, &[13238]), true);
        assert_ne!(flow.candidate().unwrap(), scope);
    }

    #[test]
    fn each_selection_owns_cleanup_independently_of_its_shared_offer() {
        let (mut flow, identity) = ready();
        let a = offer(&mut flow, identity, 200, &[12087, 12083]);
        assert!(flow.stage(a) && flow.finish(a, true));
        let b = flow.choose(12083).unwrap();
        assert!(a.scope.same_offer(b.scope));
        assert_ne!(a.scope, b.scope);
        assert_ne!(a.scope.token(), b.scope.token());
        assert!(flow.stage(b) && flow.finish(b, false));
        assert!(flow.cancel(Some(a.scope)).is_none());
        assert!(flow.admitted(b.scope));
        assert!(flow.candidate().is_some());
        assert_ne!(flow.choose(12083).unwrap().scope, b.scope);
    }

    #[test]
    fn npc_availability_preserves_status_across_snapshots_and_ignores_early_receipts() {
        let (mut flow, identity) = ready();
        let selected = offer(&mut flow, identity, 200, &[12087]);
        let status = |id, byte| Event::Interaction(InteractEvent::NpcStatusChanged {
            npc_obj_id: id, status_wire_byte: byte,
        });
        assert_eq!(flow.availability.value((200, 42)), Some(1));
        flow.observe(&status(202, 0), true);
        spawn_npc(&mut flow, 202);
        assert_eq!(flow.availability.value((202, 42)), Some(1));
        flow.observe(&status(200, 0), true);
        assert_eq!(flow.availability.value((200, 42)), Some(0));
        assert!(flow.selected_current(selected), "SC080 is not an SC07b refusal");
        spawn_npc(&mut flow, 200);
        assert_eq!(flow.availability.value((200, 42)), Some(0));
        flow.observe(&Event::UnitsRemoved(vec![200]), true);
        flow.observe(&status(200, 0), true);
        spawn_npc(&mut flow, 200);
        assert_eq!(flow.availability.value((200, 42)), Some(1));
        assert_eq!(flow.availability.value((200, 43)), None);
    }

    #[test]
    fn npc_availability_retains_wire_bytes_without_guessing_native_bool_normalization() {
        let (mut flow, _) = ready();
        for byte in 0..=u8::MAX {
            flow.observe(&Event::Interaction(InteractEvent::NpcStatusChanged {
                npc_obj_id: 200, status_wire_byte: byte,
            }), false);
            assert_eq!(flow.availability.value((200, 42)), (byte <= 1).then_some(byte));
            spawn_npc(&mut flow, 200);
            assert_eq!(flow.availability.value((200, 42)), (byte <= 1).then_some(byte));
        }
    }

    #[test]
    fn npc_availability_does_not_recreate_constructor_defaults_after_a_history_gap() {
        let (mut flow, _) = ready();
        flow.observe(&Event::GameDecodeFailed { message: "lost packet".into() }, true);
        spawn_npc(&mut flow, 200);
        spawn_npc(&mut flow, 203);
        assert_eq!(flow.availability.value((200, 42)), None);
        assert_eq!(flow.availability.value((203, 42)), None);
        flow.observe(&Event::Interaction(InteractEvent::NpcStatusChanged {
            npc_obj_id: 200, status_wire_byte: 1,
        }), false);
        assert_eq!(flow.availability.value((200, 42)), Some(1));
        assert_eq!(flow.availability.value((203, 42)), None);
        flow.observe(&Event::LeftWorld { target: 0 }, true);
        spawn_npc(&mut flow, 200);
        assert_eq!(flow.availability.value((200, 42)), Some(1));
    }

    #[test]
    fn action_deadline_uses_native_unsigned_strict_comparison_and_survives_cancel() {
        let (mut flow, identity) = ready();
        let a = offer(&mut flow, identity, 200, &[12087]);
        assert!(flow.stage(a) && flow.finish(a, true));
        for (now, deadline, accepted) in [
            (0, 0, true),
            (1000, 1000, false),
            (1001, 1000, true),
            (0x8000_0000, 0x7fff_ffff, true),
            (1, 0xffff_fff0, false),
            (0xffff_fff0, 1, true),
            (0xffff_fe0c, 1, true),
        ] {
            flow.next_action_ms = deadline;
            assert_eq!(flow.action_time(a, now), accepted);
            assert_eq!(
                flow.next_action_ms,
                if accepted {
                    now.wrapping_add(500)
                } else {
                    deadline
                }
            );
        }
        flow.next_action_ms = 5000;
        flow.query(identity, (200, 42)).unwrap();
        flow.receive(&list(200, &[12083]), true);
        flow.derive(false);
        let b = flow.take_default().unwrap();
        assert!(
            !flow.action_time(b, 100),
            "a fresh query for the same active actor does not bypass time"
        );
        assert_eq!(flow.cancel(Some(a.scope)), Some(a));
        assert_eq!(flow.next_action_ms, 5000);
        assert!(
            flow.action_time(b, 100),
            "after cancellation the actor is changed"
        );
        assert_eq!(flow.next_action_ms, 600);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::hud) struct Selected {
    pub(super) scope: Scope,
    pub(super) index: usize,
    pub(super) skill: u32,
}

#[derive(Debug)]
struct Pending {
    scope: Scope,
    request: Interact,
}

#[derive(Debug)]
struct Offer {
    scope: Scope,
    raw: Vec<u32>,
    effective: Vec<u32>,
    interactable: bool,
    derived: bool,
    default_pending: bool,
}

#[derive(Debug, Default)]
pub(super) struct Flow {
    epoch: u64,
    attempt: u64,
    selection: u64,
    actor: Option<(u32, u32)>,
    pub(super) next_action_ms: u32,
    in_world: bool,
    dead: bool,
    // Only announcements since the latest invalidation may authorize a query.
    // This is per-object provenance, not a claim of complete world population.
    npcs: HashMap<u32, u32>,
    availability: availability::Availability,
    pending: Option<Pending>,
    offer: Option<Offer>,
    active: Option<Selected>,
    dispatching: Option<Selected>,
    intents: Vec<Intent>,
    choices: Vec<Selected>,
    ends: Vec<Selected>,
    ending: bool,
    pub(super) barrier: bool,
    pub(super) close_owed: bool,
}

impl Flow {
    pub(super) fn retire(&mut self) {
        self.close_owed |= self.active.is_some() || self.dispatching.is_some();
        self.epoch = self.epoch.wrapping_add(1);
        self.pending = None;
        self.offer = None;
        self.active = None;
        self.dispatching = None;
        self.intents.clear();
        self.choices.clear();
        self.ends.clear();
        self.barrier = true;
    }

    pub(super) fn begin_frame(&mut self) {
        self.barrier = false;
    }

    fn watches(&self, id: u32) -> bool {
        self.actor.is_some_and(|a| a.1 == id)
            || self.pending.as_ref().is_some_and(|p| p.scope.npc.0 == id)
            || self.offer.as_ref().is_some_and(|o| o.scope.npc.0 == id)
            || self.active.is_some_and(|a| a.scope.npc.0 == id)
            || self.ends.iter().any(|s| s.npc().0 == id)
            || self.intents.iter().any(|i| i.npc.0 == id)
            || self.choices.iter().any(|s| s.npc().0 == id)
    }

    fn retire_object(&mut self, id: u32) {
        if self.actor.is_some_and(|a| a.1 == id) {
            self.retire();
            return;
        }
        self.ends.retain(|s| s.npc().0 != id);
        self.intents.retain(|i| i.npc.0 != id);
        self.choices.retain(|s| s.npc().0 != id);
        if self.pending.as_ref().is_some_and(|p| p.scope.npc.0 == id) {
            self.pending = None;
        }
        if self.offer.as_ref().is_some_and(|o| o.scope.npc.0 == id) {
            self.offer = None;
        }
        if self.active.is_some_and(|a| a.scope.npc.0 == id) {
            self.active = None;
            self.close_owed = true;
        }
        if self.dispatching.is_some_and(|a| a.scope.npc.0 == id) {
            self.dispatching = None;
            self.close_owed = true;
        }
    }

    pub(super) fn observe(&mut self, event: &Event, deliver: bool) {
        self.availability.observe(event);
        match event {
            Event::CharacterSelected(_)
            | Event::Disconnected { .. }
            | Event::LeftWorld { .. }
            | Event::Kicked { .. }
            | Event::LoginFailed { .. } => {
                self.retire();
                self.actor = None;
                self.in_world = false;
                self.npcs.clear();
            }
            Event::PlayerSpawn(spawn) => {
                self.retire();
                self.actor = Some((spawn.character().id, spawn.obj_id));
                self.dead = spawn.unit.dead;
            }
            Event::EnteredWorld => self.in_world = true,
            Event::LoadInstance(_) | Event::GameDecodeFailed { .. } => {
                self.retire();
                self.npcs.clear();
            }
            Event::UnhandledGamePacket { level, len, .. }
                if *level == aa_net::game::LEVEL_COMPRESSED && *len > 0 =>
            {
                self.retire();
                self.npcs.clear();
            }
            Event::UnitSpawned(unit) => {
                if self.watches(unit.obj_id) {
                    self.retire_object(unit.obj_id);
                }
                self.npcs.remove(&unit.obj_id);
                if let UnitKind::Npc { template_id, .. } = unit.kind {
                    self.npcs.insert(unit.obj_id, template_id);
                }
            }
            Event::UnitsRemoved(ids) => {
                for id in ids {
                    self.retire_object(*id);
                    self.npcs.remove(id);
                }
            }
            Event::UnitDeath { obj_id } => {
                self.retire_object(*obj_id);
                if self.actor.is_some_and(|a| a.1 == *obj_id) {
                    self.dead = true;
                }
            }
            Event::Teleported { .. } => self.retire(),
            Event::Resurrected { obj_id, .. } if self.actor.is_some_and(|a| a.1 == *obj_id) => {
                self.retire();
                self.dead = false;
            }
            Event::Interaction(receipt) => self.receive(receipt, deliver),
            _ => {}
        }
    }

    pub(super) fn query(&mut self, identity: (u32, u32), npc: (u32, u32)) -> Option<Interact> {
        if !self.ready(identity, npc) {
            return None;
        }
        self.attempt = self.attempt.wrapping_add(1);
        let scope = Scope {
            epoch: self.epoch,
            attempt: self.attempt,
            selection: 0,
            character: identity.0,
            actor: identity.1,
            npc,
        };
        let request = Interact::Start {
            npc_obj_id: npc.0,
            obj_id: 0,
            extra_info: 0,
            pick_id: 0,
            mouse_button: 2,
            modifier_keys: 0,
        };
        self.pending = Some(Pending {
            scope,
            request: request.clone(),
        });
        self.offer = None;
        Some(request)
    }

    fn ready(&self, identity: (u32, u32), npc: (u32, u32)) -> bool {
        self.in_world
            && !self.dead
            && !self.barrier
            && self.actor == Some(identity)
            && identity.0 != 0
            && identity.1 != 0
            && self.npcs.get(&npc.0) == Some(&npc.1)
    }

    fn receive(&mut self, receipt: &InteractEvent, deliver: bool) {
        let InteractEvent::SkillList {
            npc_obj_id,
            obj_id,
            extra_info,
            pick_id,
            mouse_button,
            modifier_keys,
            skills,
            interactable,
        } = receipt
        else {
            return;
        };
        let Some(pending) = &self.pending else { return };
        let echo = Interact::Start {
            npc_obj_id: *npc_obj_id,
            obj_id: *obj_id,
            extra_info: *extra_info,
            pick_id: *pick_id,
            mouse_button: *mouse_button,
            modifier_keys: *modifier_keys,
        };
        if pending.request != echo {
            return;
        }
        let scope = pending.scope;
        self.pending = None;
        // Hidden delivery consumes correlation but cannot queue a delayed window.
        if !deliver || self.barrier || skills.len() > 10 {
            return;
        }
        self.offer = Some(Offer {
            scope,
            raw: skills.clone(),
            effective: Vec::new(),
            interactable: *interactable,
            derived: false,
            default_pending: true,
        });
    }

    pub(super) fn candidate(&self) -> Option<Scope> {
        self.offer.as_ref().map(|o| o.scope)
    }

    pub(super) fn derive(&mut self, quest: bool) {
        let Some(offer) = &mut self.offer else { return };
        if offer.derived {
            return;
        }
        offer.effective = if quest && offer.interactable {
            std::iter::once(13877)
                .chain(offer.raw.iter().copied().take(8))
                .collect()
        } else {
            offer.raw.clone()
        };
        offer.derived = true;
    }

    pub(super) fn take_default(&mut self) -> Option<Selected> {
        let offer = self.offer.as_mut()?;
        if !offer.derived || !std::mem::take(&mut offer.default_pending) {
            return None;
        }
        let mut selected = Selected {
            scope: offer.scope,
            index: 0,
            skill: *offer.effective.first()?,
        };
        self.selection = self.selection.wrapping_add(1);
        selected.scope.selection = self.selection;
        Some(selected)
    }

    pub(super) fn choose(&mut self, skill: u32) -> Option<Selected> {
        let offer = self.offer.as_ref().filter(|o| o.derived)?;
        let mut selected = Selected {
            scope: offer.scope,
            index: offer.effective.iter().position(|id| *id == skill)?,
            skill,
        };
        self.selection = self.selection.wrapping_add(1);
        selected.scope.selection = self.selection;
        Some(selected)
    }

    pub(super) fn selected_current(&self, selected: Selected) -> bool {
        self.ready(
            (selected.scope.character, selected.scope.actor),
            selected.scope.npc,
        ) && selected.scope.epoch == self.epoch
            && self.offer.as_ref().is_some_and(|o| {
                o.scope.same_offer(selected.scope)
                    && o.effective.get(selected.index) == Some(&selected.skill)
            })
    }

    pub(super) fn active(&self) -> Option<Selected> {
        self.active
    }

    pub(super) fn admitted(&self, scope: Scope) -> bool {
        self.active.is_some_and(|a| a.scope == scope)
            && self.ready((scope.character, scope.actor), scope.npc)
    }

    pub(super) fn stage(&mut self, selected: Selected) -> bool {
        if !self.selected_current(selected) {
            return false;
        }
        self.dispatching = Some(selected);
        true
    }

    pub(super) fn action_time(&mut self, selected: Selected, now_ms: u32) -> bool {
        let changed = self.active.is_none_or(|a| a.npc().0 != selected.npc().0);
        if !changed && self.next_action_ms != 0 && now_ms <= self.next_action_ms {
            return false;
        }
        self.next_action_ms = now_ms.wrapping_add(500);
        true
    }

    pub(super) fn finish(&mut self, selected: Selected, target_changed: bool) -> bool {
        if self.dispatching != Some(selected) || !self.selected_current(selected) {
            return false;
        }
        self.dispatching = None;
        // A same-target callback may have cancelled the retained interaction:
        // keep it cancelled.  Only data-only offers send a tail 0x065 (the
        // caller); every talk already sent the early one.
        if target_changed || self.active.is_some() {
            self.active = Some(selected);
        }
        true
    }

    /// Cancel only retained activity. Pending discovery and the copied offer survive.
    pub(super) fn cancel(&mut self, scope: Option<Scope>) -> Option<Selected> {
        let active = self.active.filter(|a| scope == Some(a.scope));
        if active.is_some() {
            self.active = None;
        }
        active
    }
}

use super::{Facts, HudBridge, HudData, Shared};
use aa_net::state::{Player, Units};
use aa_net::{Command, NetCommand, NetEvent};
use aa_ui::UiRuntime;
use bevy::prelude::*;

#[derive(Debug)]
struct Intent {
    identity: (u32, u32),
    npc: (u32, u32),
    target: Option<u32>,
}

impl Scope {
    fn same_offer(self, other: Self) -> bool {
        Self {
            selection: 0,
            ..self
        } == Self {
            selection: 0,
            ..other
        }
    }
    pub(in crate::hud) fn npc(self) -> (u32, u32) {
        self.npc
    }
    pub(in crate::hud) fn token(self) -> String {
        format!(
            "{}:{}:{}:{}:{}:{}:{}",
            self.epoch,
            self.attempt,
            self.selection,
            self.character,
            self.actor,
            self.npc.0,
            self.npc.1
        )
    }
}

impl Selected {
    pub(in crate::hud) fn npc(self) -> (u32, u32) {
        self.scope.npc
    }
    pub(in crate::hud) fn scope(self) -> Scope {
        self.scope
    }
    pub(in crate::hud) fn skill(self) -> u32 {
        self.skill
    }
}

pub(in crate::hud) fn active_scope(data: &HudData) -> Option<Scope> {
    let flow = &data.dialog.npc_flow;
    if flow.ending {
        return None;
    }
    flow.dispatching.or(flow.active).map(|s| s.scope)
}

pub(super) fn cancel_scope(data: &HudData) -> Option<Scope> {
    let flow = &data.dialog.npc_flow;
    if flow.ending {
        return None;
    }
    flow.active.map(|s| s.scope)
}

pub(in crate::hud) fn npc_menu_current(data: &HudData, scope: Scope) -> bool {
    let flow = &data.dialog.npc_flow;
    !flow.ending
        && data.dialog.npc == Some(scope.npc)
        && flow.candidate() == Some(scope)
        && flow.ready((scope.character, scope.actor), scope.npc)
}

fn fire_end(lua: &mlua::Lua, shared: &Shared) {
    {
        let mut data = shared.borrow_mut();
        data.dialog.npc_flow.ending = true;
        super::stop_directing(lua, &mut data.dialog);
        data.input_modes.set(4, 0);
    }
    aa_ui::lua::fire_ui_event(lua, "NPC_INTERACTION_END", Vec::new());
    shared.borrow_mut().dialog.npc_flow.ending = false;
}

pub(in crate::hud) fn scope_current(data: &HudData, scope: Scope) -> bool {
    let flow = &data.dialog.npc_flow;
    !data.disconnected
        && data.character_id == scope.character
        && data.snapshot.unit_id.parse::<u32>().ok() == Some(scope.actor)
        && data.dialog.npc == Some(scope.npc)
        && (flow.admitted(scope)
            || flow
                .dispatching
                .is_some_and(|s| s.scope == scope && flow.selected_current(s)))
}

/// Revocation is immediate; UI and packets drain in the pre-frame owner pass.
pub(in crate::hud) fn close_npc(data: &mut HudData, scope: Scope) {
    let flow = &mut data.dialog.npc_flow;
    let prior = flow.cancel(Some(scope));
    if let Some(prior) = prior {
        flow.ends.push(prior);
        flow.close_owed = true;
        data.dialog.npc = None;
        data.dialog.start.clear();
        data.dialog.complete.clear();
        data.dialog.talk.clear();
    }
}

/// The native public binding guards on the retained active actor, not dispatch.
pub(super) fn cancel_public(lua: &mlua::Lua, shared: &Shared) {
    let close = {
        let mut data = shared.borrow_mut();
        let Some(scope) = cancel_scope(&data) else {
            return;
        };
        close_npc(&mut data, scope);
        std::mem::take(&mut data.dialog.npc_flow.close_owed)
    };
    if close {
        fire_end(lua, shared);
    }
}

fn flush_ends(
    rt: &UiRuntime,
    shared: &Shared,
    player: &Player,
    units: &Units,
    net: &mut MessageWriter<NetCommand>,
) {
    let (ends, close) = {
        let mut data = shared.borrow_mut();
        if data.dialog.npc_flow.close_owed
            && data.dialog.npc_flow.active.is_none()
            && data.dialog.npc_flow.dispatching.is_none()
        {
            data.dialog.npc = None;
            data.dialog.start.clear();
            data.dialog.complete.clear();
            data.dialog.talk.clear();
        }
        (
            std::mem::take(&mut data.dialog.npc_flow.ends),
            std::mem::take(&mut data.dialog.npc_flow.close_owed),
        )
    };
    if close {
        fire_end(&rt.lua, shared);
    }
    for end in ends {
        if resolved(end.scope, &shared.borrow(), player, units) {
            net.write(NetCommand(Command::Interact(Interact::NpcEnd {
                obj_id: end.npc().0,
            })));
        }
    }
}

pub(in crate::hud) fn query_npc(data: &mut HudData, npc: (u32, u32), target: Option<u32>) {
    let Some(actor) = data.snapshot.unit_id.parse::<u32>().ok() else {
        return;
    };
    let identity = (data.character_id, actor);
    let intents = &mut data.dialog.npc_flow.intents;
    if intents
        .last()
        .is_some_and(|i| i.identity == identity && i.npc == npc)
    {
        return;
    }
    intents.push(Intent {
        identity,
        npc,
        target,
    });
}

fn identity(data: &HudData, player: &Player) -> Option<(u32, u32)> {
    let spawn = player.spawn.as_ref()?;
    let identity = (spawn.character().id, spawn.obj_id);
    (!data.disconnected
        && identity.0 == data.character_id
        && data.snapshot.unit_id.parse::<u32>().ok() == Some(identity.1))
    .then_some(identity)
}

fn resolved(scope: Scope, data: &HudData, player: &Player, units: &Units) -> bool {
    scope.epoch == data.dialog.npc_flow.epoch
        && identity(data, player) == Some((scope.character, scope.actor))
        && data.dialog.npc_flow.ready((scope.character, scope.actor), scope.npc)
        && units.get(scope.npc.0).is_some_and(|unit| matches!(unit.kind, UnitKind::Npc {template_id,..} if template_id == scope.npc.1))
}

pub(super) fn track_npc(
    mut events: MessageReader<NetEvent>,
    bridge: NonSend<HudBridge>,
    built: Res<super::super::HudBuilt>,
    suspended: Res<super::super::HudSuspended>,
) {
    let deliver = built.up && !suspended.0;
    let mut data = bridge.0.borrow_mut();
    data.dialog.npc_flow.begin_frame();
    for NetEvent(event) in events.read() {
        data.dialog.npc_flow.observe(event, deliver);
    }
}

fn quest_kind(
    data: &HudData,
    template: u32,
    facts: &super::super::quests::PlayerFacts,
) -> Option<&'static str> {
    let game = data.game.get()?;
    if !game.quests.talks(template, facts).is_empty() {
        Some("talk")
    } else if !game.quests.completes(template, facts).is_empty() {
        Some("complete")
    } else if !game.quests.offers(template, facts).is_empty() {
        Some("start")
    } else {
        None
    }
}

/// The specialty trader's data-only interaction skill; 1.2 runs it on the
/// server and closes it with a second CSInteractNPC (0x065) after the skill.
const SPECIALTY_TRADER_SKILL: u32 = 25205;

/// A received interaction skill with no local window. `SPECIALTY_TRADER_SKILL`
/// is the only data-only interaction skill captured. Local store/quest/service
/// skills open their existing windows and have no such tail.
fn data_only(skill: u32) -> bool {
    skill == SPECIALTY_TRADER_SKILL
}

fn dispatch(
    selected: Selected,
    rt: &UiRuntime,
    shared: &Shared,
    player: &Player,
    units: &Units,
    quest: Option<&'static str>,
) -> bool {
    if let Err(error) = owners::install(rt) {
        warn!("hud: NPC dialog-owner cleanup: {error}");
    }
    match selected.skill {
        0 => true,
        13877 => {
            let Some(quest) = quest else { return false };
            rt.fire_event(
                "NPC_INTERACTION_START",
                ("quest", quest, selected.npc().0.to_string()),
            );
            true
        }
        12087 | 13238 => super::super::store::start_selected(rt, shared, selected),
        12083 => super::super::auction::start_selected(rt, shared, selected),
        12660 => super::super::battlefield::start_selected(rt, shared, selected, units),
        13736 => super::super::expedition::start_selected(rt, shared, selected, player, units),
        12195 => super::super::pets::start_selected(rt, shared, selected, player, units),
        22780 => super::super::salon::start_selected(rt, selected),
        // Data-only offers have no local window; 1.2 runs them with the server.
        SPECIALTY_TRADER_SKILL => true,
        // Exact spatial/population suppliers are required before16912 can dispatch.
        16912 => false,
        _ => super::super::store::services::start_selected(rt, shared, selected),
    }
}

/// One synchronous selected action, before feature command flushers and UiFrame.
pub(in crate::hud) fn drive_npc(
    rt: Option<NonSend<UiRuntime>>,
    bridge: NonSend<HudBridge>,
    facts: Facts,
    units: Res<Units>,
    mut net: MessageWriter<NetCommand>,
    teleport: Option<Res<crate::ingame::teleport::Teleport>>,
    suspended: Res<super::super::HudSuspended>,
    time: Res<Time<Virtual>>,
    mut script: Option<ResMut<crate::script::ScriptPick>>,
) {
    let Some(rt) = rt else { return };
    let player = &*facts.player;
    flush_ends(&rt, &bridge.0, player, &units, &mut net);
    if suspended.0 || teleport.is_some_and(|t| t.loading()) {
        let mut data = bridge.0.borrow_mut();
        data.dialog.npc_flow.intents.clear();
        data.dialog.npc_flow.choices.clear();
        return;
    }
    let Some(player_facts) = facts.get() else {
        return;
    };
    let (selected, quest) = {
        let mut data = bridge.0.borrow_mut();
        let fresh = data
            .dialog
            .npc_flow
            .offer
            .as_ref()
            .is_some_and(|o| !o.derived);
        let needs_quest = fresh
            || data
                .dialog
                .npc_flow
                .choices
                .iter()
                .any(|s| s.skill == 13877);
        let quest = needs_quest
            .then(|| {
                data.dialog
                    .npc_flow
                    .candidate()
                    .and_then(|s| quest_kind(&data, s.npc.1, &player_facts))
            })
            .flatten();
        data.dialog.npc_flow.derive(quest.is_some());
        if let Some(offer) = data.dialog.npc_flow.offer.as_ref().filter(|_| fresh) {
            let (scope, skills) = (offer.scope, offer.effective.clone());
            super::super::store::services::record_offer(&mut data, scope, &skills);
        }
        let default = data.dialog.npc_flow.take_default();
        let choices = std::mem::take(&mut data.dialog.npc_flow.choices);
        (
            default.or_else(|| {
                choices
                    .into_iter()
                    .find(|s| data.dialog.npc_flow.selected_current(*s))
            }),
            quest,
        )
    };
    if let Some(selected) = selected {
        let allowed = {
            let data = bridge.0.borrow();
            resolved(selected.scope, &data, player, &units)
                && data.dialog.npc_flow.selected_current(selected)
                && units.get(selected.npc().0).is_some_and(|unit| !unit.dead)
        };
        if allowed {
            let prior = bridge.0.borrow().dialog.npc_flow.active;
            if prior.is_none_or(|p| p.npc() != selected.npc()) {
                {
                    let mut data = bridge.0.borrow_mut();
                    data.dialog.npc_flow.active = None;
                    data.dialog.npc = None;
                }
                fire_end(&rt.lua, &bridge.0);
                if let Some(prior) =
                    prior.filter(|p| resolved(p.scope, &bridge.0.borrow(), player, &units))
                {
                    net.write(NetCommand(Command::Interact(Interact::NpcEnd {
                        obj_id: prior.npc().0,
                    })));
                }
            }
            let staged = {
                let mut data = bridge.0.borrow_mut();
                let staged = data
                    .dialog
                    .npc_flow
                    .action_time(selected, time.elapsed().as_millis() as u32)
                    && resolved(selected.scope, &data, player, &units)
                    && units.get(selected.npc().0).is_some_and(|unit| {
                        data.pos
                            .distance(Vec3::new(unit.pos.x, unit.pos.y, unit.pos.z))
                            <= super::TALK_RANGE
                    })
                    && data.dialog.npc_flow.stage(selected);
                if staged {
                    data.dialog.npc = Some(selected.npc());
                    data.dialog.doodad = None;
                    super::refresh_npc(&mut data, &player_facts);
                }
                staged
            };
            if staged {
                let target_changed = prior.is_none_or(|p| p.npc().0 != selected.npc().0);
                // Every talk starts with CSInteractNPC (0x065), written before
                // the offer's service runs: 1.2 sends it for ordinary NPCs and
                // for the specialty trader alike.  A talk that opened a local
                // window has no other 0x065 (1.2 captures: one per talk, both for
                // a merchant whose store window opens and for an NPC without one).
                net.write(NetCommand(Command::Interact(Interact::Npc {
                    obj_id: selected.npc().0,
                    target_changed,
                })));
                let ran = dispatch(selected, &rt, &bridge.0, player, &units, quest);
                flush_ends(&rt, &bridge.0, player, &units, &mut net);
                let mut data = bridge.0.borrow_mut();
                let data_only = data_only(selected.skill);
                let finished = ran
                    && resolved(selected.scope, &data, player, &units)
                    && data.dialog.npc_flow.finish(selected, target_changed);
                if !finished {
                    data.dialog.npc_flow.dispatching = None;
                } else if data_only {
                    // 1.2's second 0x065 follows the interaction skill/service
                    // (the trader capture: 4a 5f 00 00).  The early 0x065
                    // already announced the target, so the tail reports 00 even
                    // on the first talk, where `target_changed` was true.
                    net.write(NetCommand(Command::Interact(Interact::Npc {
                        obj_id: selected.npc().0,
                        target_changed: false,
                    })));
                }
                data.dialog.npc = data.dialog.npc_flow.active.map(|a| a.npc());
                // A data-only offer opens no local window, so `active` alone can
                // be empty (a repeat talk to the same NPC finishes with
                // target_changed false).  Keep the trader for
                // X2Store:SellBackPackGoods; a data-only offer runs no Lua, so
                // nothing can have cancelled it during dispatch.
                if finished && data_only {
                    data.dialog.npc = Some(selected.npc());
                }
                if let Some(script) = script.as_deref_mut() {
                    script.talked(
                        selected.npc().0,
                        (ran && selected.skill == 13877).then_some(quest).flatten(),
                    );
                }
            }
        }
    }
    let intents = std::mem::take(&mut bridge.0.borrow_mut().dialog.npc_flow.intents);
    for intent in intents {
        let mut data = bridge.0.borrow_mut();
        let current = identity(&data, player) == Some(intent.identity)
            && units.get(intent.npc.0).is_some_and(|unit| !unit.dead && matches!(unit.kind, UnitKind::Npc {template_id,..} if template_id==intent.npc.1)
                && data.pos.distance(Vec3::new(unit.pos.x, unit.pos.y, unit.pos.z)) <= super::TALK_RANGE);
        if current && let Some(query) = data.dialog.npc_flow.query(intent.identity, intent.npc) {
            if intent.target != Some(intent.npc.0) {
                net.write(NetCommand(Command::Target {
                    obj_id: intent.npc.0,
                }));
            }
            net.write(NetCommand(Command::Interact(query)));
        }
    }
}

pub(in crate::hud) fn choose_npc(data: &mut HudData, scope: Scope, skill: u32) {
    if !npc_menu_current(data, scope) {
        return;
    }
    if let Some(selected) = data
        .dialog
        .npc_flow
        .choose(skill)
        .filter(|s| s.scope.same_offer(scope))
    {
        data.dialog.npc_flow.choices.push(selected);
    }
}
