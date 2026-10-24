//! Doodad seats use received bonds, independently of mount attachments.
//!
//! The private fold preserves receipt order. A socket-write completion can detach
//! its captured receipt, while a later accepted bond/snapshot can seat again.

use std::collections::HashMap;

use aa_net::model::{Unit, UnitKind};
use aa_net::{
    DoodadBond, DoodadBondEvent, DoodadBonds, Event, NetEvent, Phase, UnbondDoodadRequest,
};
use bevy::prelude::*;

mod presentation;
pub(crate) use presentation::{SeatPose, Seated};

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum SeatSet {
    Follow,
    Exit,
    Pose,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Actor {
    generation: u64,
    receipt: u64,
    locally_detached: bool,
}

/// Identity of one accepted observation, not an invented wire bond epoch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SeatReceipt {
    generation: u64,
    receipt: u64,
    pub bond: DoodadBond,
}

#[derive(Clone, Copy, Debug)]
struct Pending {
    request: UnbondDoodadRequest,
    actor: Actor,
}

#[derive(Resource, Default)]
pub(crate) struct Seats {
    history: DoodadBonds,
    actors: HashMap<u32, Actor>,
    selected: u32,
    own: u32,
    serial: u64,
    pending: Option<Pending>,
    // This frame only: accepted release MessageId, then whether a newer remote
    // movement report has replaced the pre-seat interpolation track.
    released: HashMap<u32, (usize, bool)>,
}

impl Seats {
    fn next(&mut self) -> u64 {
        self.serial = self
            .serial
            .checked_add(1)
            .expect("seat receipt counter exhausted");
        self.serial
    }

    fn reset(&mut self) {
        // Keep the counter across selections so old physical work cannot match a new actor.
        self.actors.clear();
        self.selected = 0;
        self.own = 0;
        self.pending = None;
        self.released.clear();
    }

    fn snapshot(&mut self, unit: &Unit, replacement: bool) {
        let UnitKind::Character { char_id } = unit.kind else {
            self.actors.remove(&unit.obj_id);
            return;
        };
        if unit.obj_id == 0 || char_id == 0 {
            return;
        }
        let receipt = self.next();
        let generation = self
            .actors
            .get(&unit.obj_id)
            .filter(|_| !replacement)
            .map_or(receipt, |actor| actor.generation);
        self.actors.insert(
            unit.obj_id,
            Actor {
                generation,
                receipt,
                locally_detached: false,
            },
        );
    }

    fn apply(&mut self, event: &Event) {
        let occupied = match event {
            Event::DoodadBond(DoodadBondEvent::Bonded { unit_obj_id, .. }) => {
                self.wanted(*unit_obj_id)
            }
            _ => None,
        };
        // Use the same guarded reducer as session/WorldState. Raw ignored SC139/SC13A
        // must not restart or detach a presented actor.
        let accepted = self.history.apply(event);
        match event {
            Event::CharacterSelected(state) => {
                self.reset();
                self.selected = state.character.id;
            }
            Event::LeftWorld { .. }
            | Event::Disconnected { .. }
            | Event::Kicked { .. }
            | Event::PhaseChanged(
                Phase::Idle
                | Phase::Authenticating
                | Phase::WorldSelect
                | Phase::CharacterSelect
                | Phase::ReadyToConnect
                | Phase::Connecting,
            ) => self.reset(),
            Event::PlayerSpawn(player) if accepted => {
                if self.own != player.obj_id {
                    self.actors.remove(&self.own);
                }
                self.own = player.obj_id;
                self.snapshot(&player.unit, true);
            }
            Event::UnitSpawned(unit) if accepted => self.snapshot(unit, true),
            Event::OwnUnitSnapshot(unit) if accepted => self.snapshot(unit, false),
            Event::UnitsRemoved(ids) => {
                for id in ids {
                    self.actors.remove(id);
                }
                // Keep an outstanding completion as a tombstone until consumed. Its
                // fields alone cannot distinguish a later actor with the same IDs.
            }
            Event::DoodadsSpawned(doodads) => {
                for doodad in doodads {
                    self.actors.remove(&doodad.obj_id);
                }
            }
            Event::DoodadBond(DoodadBondEvent::Bonded { unit_obj_id, .. }) if accepted => {
                // A stale write completion can open the actual session's receive
                // gate without retiring our newer visual receipt. Fold that same
                // protocol gate here; an equal bond over an occupied presentation
                // is not another pose/request lifetime. A changed received bond is
                // fresh authority, and a locally detached equal bond still re-arms.
                if occupied
                    .is_some_and(|old| self.history.by_unit.get(unit_obj_id) == Some(&old.bond))
                {
                    return;
                }
                let receipt = self.next();
                if let Some(actor) = self.actors.get_mut(unit_obj_id) {
                    actor.receipt = receipt;
                    actor.locally_detached = false;
                }
            }
            Event::DoodadBond(DoodadBondEvent::UnbondRequested(request)) => {
                let Some(pending) = self.pending.filter(|pending| pending.request == *request)
                else {
                    return;
                };
                self.pending = None;
                if self.current_capture(&pending) {
                    self.actors
                        .get_mut(&request.actor_obj_id)
                        .unwrap()
                        .locally_detached = true;
                }
            }
            Event::DoodadBond(DoodadBondEvent::UnbondRejected { request, .. })
                if self
                    .pending
                    .is_some_and(|pending| pending.request == *request) =>
            {
                self.pending = None;
            }
            _ => {}
        }
    }

    pub(super) fn wanted(&self, obj_id: u32) -> Option<SeatReceipt> {
        let actor = self.actors.get(&obj_id)?;
        if actor.locally_detached || (obj_id == self.own && !self.history.own_known()) {
            return None;
        }
        Some(SeatReceipt {
            generation: actor.generation,
            receipt: actor.receipt,
            bond: *self.history.by_unit.get(&obj_id)?,
        })
    }

    /// Empty received history is free only after an actual matched snapshot.
    /// A successful local detach also opens the native effective receive gate.
    pub(crate) fn own_free(&self) -> bool {
        self.history.own_known()
            && self.actors.contains_key(&self.own)
            && (self.history.own().is_none() || self.actors[&self.own].locally_detached)
    }

    pub(super) fn capture_unbond(&mut self) -> Option<UnbondDoodadRequest> {
        if self.pending.is_some() || !self.history.own_known() {
            return None;
        }
        let actor = *self.actors.get(&self.own)?;
        if actor.locally_detached {
            return None;
        }
        let request = UnbondDoodadRequest {
            character_id: self.selected,
            actor_obj_id: self.own,
            bond: *self.history.own()?,
        };
        self.pending = Some(Pending { request, actor });
        Some(request)
    }

    fn current_capture(&self, pending: &Pending) -> bool {
        self.history.own_known()
            && self.selected == pending.request.character_id
            && self.own == pending.request.actor_obj_id
            && self.actors.get(&pending.request.actor_obj_id) == Some(&pending.actor)
            && self.history.own() == Some(&pending.request.bond)
    }

    pub(super) fn remote_report_after_release(&mut self, obj_id: u32, message_id: usize) -> bool {
        let Some((release, replaced)) = self.released.get_mut(&obj_id) else {
            return false;
        };
        if message_id <= *release || *replaced {
            return false;
        }
        *replaced = true;
        true
    }

    fn release_has_report(&mut self, obj_id: u32) -> bool {
        self.released
            .remove(&obj_id)
            .is_some_and(|(_, report)| report)
    }
}

pub(crate) fn track(mut events: MessageReader<NetEvent>, mut seats: ResMut<Seats>) {
    seats.released.clear();
    for (NetEvent(event), id) in events.read_with_id() {
        // Movement is hot traffic; only receipts capable of releasing a bond
        // need this small list. Message IDs preserve the actual batch boundary.
        let before: Vec<_> = if matches!(
            event,
            Event::DoodadBond(_)
                | Event::OwnUnitSnapshot(_)
                | Event::PlayerSpawn(_)
                | Event::UnitSpawned(_)
                | Event::DoodadsSpawned(_)
                | Event::DoodadsRemoved(_)
                | Event::UnitsRemoved(_)
                | Event::GameDecodeFailed { .. }
                | Event::UnhandledGamePacket {
                    level: aa_net::game::LEVEL_COMPRESSED,
                    len: 1..,
                    ..
                }
        ) {
            seats
                .history
                .by_unit
                .keys()
                .copied()
                .filter(|obj| seats.wanted(*obj).is_some())
                .collect()
        } else {
            Vec::new()
        };
        seats.apply(event);
        for obj in before {
            if seats.actors.contains_key(&obj) && seats.wanted(obj).is_none() {
                seats.released.insert(obj, (id.id, false));
            }
        }
    }
}

pub(crate) fn plugin(app: &mut App) {
    // Observe selections/loading receipts too; restricting this fold to InGame loses
    // the selected persistent identity before the first PlayerSpawn.
    app.init_resource::<Seats>()
        .add_systems(Update, track.before(super::player::control_player))
        .add_systems(
            Update,
            presentation::follow
                .in_set(SeatSet::Follow)
                .after(track)
                .after(super::units::move_units)
                .after(super::doodad_visual::grow)
                .after(super::player::spawn_player)
                .before(super::player::follow_server)
                .before(super::player::control_player)
                .before(presentation::pose)
                .before(super::camera::follow_player)
                .before(super::anim::drive_locomotion)
                .run_if(super::world_screens),
        )
        .add_systems(
            Update,
            (
                exit_on_input
                    .in_set(SeatSet::Exit)
                    .run_if(in_state(crate::Screen::InGame))
                    .run_if(super::teleport::idle),
                presentation::pose.in_set(SeatSet::Pose),
            )
                .chain()
                .after(presentation::follow)
                .after(super::player::control_player)
                .after(super::combat::InputCancellation)
                .after(super::combat::end_actions)
                .before(super::camera::follow_player)
                .before(super::anim::drive_locomotion)
                .run_if(super::world_screens),
        );
}

pub(super) fn exit_on_input(
    input: Res<super::player::PlayerInput>,
    mut seats: ResMut<Seats>,
    local: Query<&super::player::LocalPlayer>,
    bridge: Option<NonSend<crate::hud::api::HudBridge>>,
    rt: Option<NonSend<aa_ui::UiRuntime>>,
    mut out: MessageWriter<aa_net::NetCommand>,
) {
    if !local.iter().any(|local| local.obj_id == seats.own) || !(input.moving || input.turned) {
        return;
    }
    let Some(request) = seats.capture_unbond() else {
        return;
    };
    // cancel_on_move already submitted eligible cast stops. Native then closes
    // loot before CS0xcd; neither action invents a received buff/music result.
    let loot = bridge
        .as_ref()
        .and_then(|bridge| bridge.0.borrow_mut().loot.last_owner.take());
    if let Some((owner_type, owner_obj_id)) = loot {
        out.write(aa_net::NetCommand(aa_net::Command::LootClose {
            owner_type,
            owner_obj_id,
        }));
        if let Some(rt) = rt {
            rt.fire_event("LOOT_BAG_CLOSE", ());
        }
    }
    out.write(aa_net::NetCommand(aa_net::Command::UnbondDoodad(request)));
}

#[cfg(test)]
mod tests;
#[cfg(test)]
pub(super) use tests::unit as fixture_unit;

#[cfg(test)]
pub(super) use presentation::fixture_app;
