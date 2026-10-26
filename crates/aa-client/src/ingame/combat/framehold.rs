//! Receipt-backed animation permission, separate from distance-cropped buff art.
use super::*;
use aa_net::{
    buff::{Buff, BuffKind},
    state::{BuffEntry, NativeBuffs},
};

#[derive(Resource, Default)]
pub(super) struct Frameholds {
    native: NativeBuffs,
    actors: HashMap<u32, ((aa_net::model::UnitKind, u32), bool)>,
    own: Option<u32>,
    released: Vec<u32>,
    poses: HashMap<(u32, u32, u32), bool>,
}

pub(super) enum BuffChange {
    None,
    Seed {
        obj: u32,
        entries: Vec<BuffEntry>,
    },
    Created {
        obj: u32,
        previous: Option<u32>,
        entry: BuffEntry,
    },
    Removed {
        obj: u32,
        entry: BuffEntry,
    },
}

fn order(kind: BuffKind) -> u8 {
    match kind {
        BuffKind::Good => 0,
        BuffKind::Bad => 1,
        BuffKind::Hidden => 2,
        BuffKind::Unknown => 3,
    }
}

/// Only allocation/merge admission is inspected here; NativeBuffs performs the
/// actual stack fold. A repeated admitted Created invokes ApplyAttrs again.
fn admits(native: &NativeBuffs, obj: u32, buff: &Buff) -> bool {
    let (Some(rule), Some(list)) = (native.rules.get(&buff.buff_id), native.of(obj)) else {
        return false;
    };
    if rule.kind == BuffKind::Unknown {
        return false;
    }
    let found = (rule.stack_rule_id != 6)
        .then(|| {
            list.iter().position(|entry| {
                entry.buff.kind == rule.kind
                    && entry.buff.buff_id == buff.buff_id
                    && (!rule.per_unit_creation || entry.buff.caster_char_id == buff.caster_char_id)
            })
        })
        .flatten();
    let cap = if rule.kind == BuffKind::Good { 32 } else { 24 };
    (found.is_some()
        || list
            .iter()
            .filter(|entry| entry.buff.kind == rule.kind)
            .count()
            < cap)
        && !list
            .iter()
            .position(|entry| entry.buff.index == buff.index)
            .is_some_and(|i| Some(i) != found)
}

impl Frameholds {
    fn prepare(&mut self, book: &SkillBook) {
        if !self.native.catalog_loaded() {
            let rules = book.native_buff_rules();
            self.native.set_catalog((*rules).clone());
            self.native.rules = rules;
        }
    }

    pub(super) fn before_receipt(&mut self, event: &Event, st: &mut Stage) -> BuffChange {
        self.prepare(&st.book);
        let change = self.apply(event, &st.book);
        let ids: Vec<u32> = match event {
            Event::PlayerSpawn(_)
            | Event::CharacterSelected(_)
            | Event::LeftWorld { .. }
            | Event::Disconnected { .. }
            | Event::Kicked { .. } => st
                .local
                .iter()
                .map(|(_, p)| p.obj_id)
                .chain(st.remote.iter().map(|(_, p)| p.obj_id))
                .collect(),
            Event::UnitSpawned(unit) => vec![unit.obj_id],
            Event::BuffCreated { obj_id, .. } | Event::BuffRemoved { obj_id, .. } => vec![*obj_id],
            Event::UnitsRemoved(ids) => ids.clone(),
            _ => Vec::new(),
        };
        for id in ids {
            if let Some(entity) = st.entity(id)
                && let Ok((mut animator, ..)) = st.actors.get_mut(entity)
            {
                animator.set_framehold(self.held(id));
            }
        }
        // End stops belong at their receipt, before any later admitted start.
        // The ordinary post-receipt sync must not replay them over a successor.
        for id in self.released.drain(..) {
            if let Some(entity) = st.entity(id) {
                st.commands.entity(entity).try_remove::<Acting>();
                st.commands.queue(move |world: &mut World| {
                    if let Some(mut loco) = world.get_mut::<Locomotion>(entity) {
                        loco.invalidate();
                    }
                });
            }
        }
        change
    }

    pub(super) fn held(&self, id: u32) -> bool {
        self.actors.get(&id).is_some_and(|(_, held)| *held)
    }

    pub(super) fn entries(&self, id: u32) -> Option<&[BuffEntry]> {
        self.native.of(id)
    }

    pub(super) fn pose_allowed(&self, id: u32, buff: &Buff) -> bool {
        self.poses
            .get(&(id, buff.index, buff.buff_id))
            .copied()
            .unwrap_or(false)
    }

    fn gliding(&self, id: u32, book: &SkillBook) -> bool {
        self.native.of(id).is_some_and(|list| {
            list.iter().any(|entry| {
                book.buff(entry.buff.buff_id)
                    .is_some_and(|look| look.gliding)
            })
        })
    }

    fn seed(&mut self, unit: &aa_net::model::Unit, event: &Event, book: &SkillBook) -> BuffChange {
        self.released.retain(|id| *id != unit.obj_id);
        self.poses.retain(|(id, _, _), _| *id != unit.obj_id);
        self.native.apply(event);
        let mut entries = self.native.of(unit.obj_id).unwrap_or_default().to_vec();
        entries.sort_by_key(|entry| order(entry.buff.kind));
        let gliding = self.gliding(unit.obj_id, book);
        let mut held = false;
        for entry in &entries {
            let look = book.buff(entry.buff.buff_id).unwrap_or_default();
            let hold = look.framehold;
            self.poses.insert(
                (unit.obj_id, entry.buff.index, entry.buff.buff_id),
                !held && !hold && (!gliding || look.ragdoll || look.knock_down),
            );
            held |= hold;
        }
        self.actors.insert(
            unit.obj_id,
            (super::super::units::source_identity(unit), held),
        );
        BuffChange::Seed {
            obj: unit.obj_id,
            entries,
        }
    }

    fn apply(&mut self, event: &Event, book: &SkillBook) -> BuffChange {
        let mut change = BuffChange::None;
        match event {
            Event::PlayerSpawn(spawn) => {
                self.actors.clear();
                self.poses.clear();
                self.released.clear();
                let rules = self.native.rules.clone();
                let loaded = self.native.catalog_loaded();
                self.native = NativeBuffs::default();
                if loaded {
                    self.native.set_catalog((*rules).clone());
                }
                self.native.rules = rules;
                self.own = Some(spawn.obj_id);
                change = self.seed(&spawn.unit, event, book);
            }
            Event::UnitSpawned(unit) if Some(unit.obj_id) != self.own => {
                let identity = super::super::units::source_identity(unit);
                if !self
                    .actors
                    .get(&unit.obj_id)
                    .is_some_and(|(old, _)| *old == identity)
                {
                    change = self.seed(unit, event, book);
                }
            }
            // Both existing own and remote SCUnitState snapshots exit the
            // original handler before buff initialization. Appearance refresh
            // must not reassert a previously released overlapping hold.
            Event::OwnUnitSnapshot(_) | Event::UnitSpawned(_) => {}
            Event::BuffCreated { obj_id, buff } => {
                let admitted = admits(&self.native, *obj_id, buff);
                let previous = self
                    .native
                    .rules
                    .get(&buff.buff_id)
                    .filter(|r| r.stack_rule_id != 6)
                    .and_then(|r| {
                        self.native.of(*obj_id).and_then(|list| {
                            list.iter().find(|entry| {
                                entry.buff.kind == r.kind
                                    && entry.buff.buff_id == buff.buff_id
                                    && (!r.per_unit_creation
                                        || entry.buff.caster_char_id == buff.caster_char_id)
                            })
                        })
                    })
                    .map(|entry| entry.buff.index);
                // Rejected/ambiguous allocations are not runtime erasures.
                if admitted {
                    self.native.apply(event);
                }
                if admitted
                    && book.buff(buff.buff_id).is_some_and(|look| look.framehold)
                    && let Some((_, held)) = self.actors.get_mut(obj_id)
                {
                    *held = true;
                }
                if admitted
                    && let Some(entry) = self
                        .native
                        .of(*obj_id)
                        .and_then(|list| {
                            list.iter().find(|entry| {
                                entry.buff.index == buff.index && entry.buff.buff_id == buff.buff_id
                            })
                        })
                        .cloned()
                {
                    if let Some(old) = previous {
                        self.poses.remove(&(*obj_id, old, entry.buff.buff_id));
                    }
                    let look = book.buff(buff.buff_id).unwrap_or_default();
                    // Native ragdoll/knockdown branches precede the manager's
                    // gliding predicate, which gates only ordinary starts.
                    let allowed = !self.held(*obj_id)
                        && !look.framehold
                        && (!self.gliding(*obj_id, book) || look.ragdoll || look.knock_down);
                    self.poses
                        .insert((*obj_id, entry.buff.index, entry.buff.buff_id), allowed);
                    change = BuffChange::Created {
                        obj: *obj_id,
                        previous,
                        entry,
                    };
                }
            }
            Event::BuffRemoved { obj_id, index } => {
                let removed = self
                    .native
                    .of(*obj_id)
                    .and_then(|list| {
                        list.iter()
                            .enumerate()
                            .filter(|(_, entry)| entry.buff.index == *index)
                            .min_by_key(|(i, entry)| (order(entry.buff.kind), *i))
                    })
                    .filter(|(_, entry)| {
                        entry.buff.kind != BuffKind::Unknown
                            && self.native.rules.contains_key(&entry.buff.buff_id)
                    })
                    .map(|(_, entry)| entry.clone());
                self.native.apply(event);
                if let Some(look) = removed
                    .as_ref()
                    .and_then(|entry| book.buff(entry.buff.buff_id))
                {
                    if look.framehold
                        && let Some((_, held)) = self.actors.get_mut(obj_id)
                    {
                        *held = false;
                    }
                    if look.end_anim.as_deref().is_some_and(skills::is_stop_clip) {
                        self.released.push(*obj_id);
                    }
                }
                if let Some(entry) = removed {
                    self.poses
                        .remove(&(*obj_id, entry.buff.index, entry.buff.buff_id));
                    change = BuffChange::Removed {
                        obj: *obj_id,
                        entry,
                    };
                }
            }
            Event::UnitsRemoved(ids) => {
                self.native.apply(event);
                self.released.retain(|id| !ids.contains(id));
                self.poses.retain(|(id, _, _), _| !ids.contains(id));
                for id in ids {
                    self.actors.remove(id);
                    if self.own == Some(*id) {
                        self.own = None;
                    }
                }
            }
            Event::CharacterSelected(_)
            | Event::LeftWorld { .. }
            | Event::Disconnected { .. }
            | Event::Kicked { .. } => {
                self.native.apply(event);
                self.actors.clear();
                self.poses.clear();
                self.own = None;
                self.released.clear();
            }
            // These are host completeness diagnostics, not native manager
            // erase operations. Retain known runtime instances so a later
            // decoded removal can still release its hold. The canonical
            // NativeBuffs analytical view continues to report unknown history.
            Event::GameDecodeFailed { .. } | Event::UnhandledGamePacket { .. } => {}
            _ => self.native.apply(event),
        }
        change
    }
}

pub(super) fn observe(
    mut events: MessageReader<NetEvent>,
    mut holds: ResMut<Frameholds>,
    book: Option<Res<SkillBook>>,
    world: Option<Res<super::super::world_switch::CurrentWorld>>,
    mut last_world: Local<Option<String>>,
    screen: Option<Res<State<crate::Screen>>>,
) {
    let Some(book) = book else {
        events.clear();
        return;
    };
    // SkillBook already opens the static database synchronously. Use the same
    // proven rule loader as the HUD, before consuming any permission receipt.
    // This does not depend on the asynchronous HUD catalog becoming ready.
    holds.prepare(&book);
    if let Some(world) = world {
        if last_world.as_ref().is_some_and(|old| old != &world.0) {
            let removed: Vec<_> = holds
                .actors
                .keys()
                .copied()
                .filter(|id| Some(*id) != holds.own)
                .collect();
            holds.apply(&Event::UnitsRemoved(removed), &book);
        }
        *last_world = Some(world.0.clone());
    }
    for NetEvent(event) in events.read() {
        if !screen
            .as_ref()
            .is_some_and(|s| matches!(s.get(), crate::Screen::Loading | crate::Screen::InGame))
        {
            holds.apply(event, &book);
        }
    }
}

#[allow(clippy::type_complexity)]
pub(super) fn sync_after_receipts(
    commands: Commands,
    holds: ResMut<Frameholds>,
    actors: Query<
        (
            Entity,
            Option<&RemoteUnit>,
            Option<&LocalPlayer>,
            Option<&mut Animator>,
            Option<&mut super::super::units::Asleep>,
            Option<&mut Locomotion>,
        ),
        Or<(With<RemoteUnit>, With<LocalPlayer>)>,
    >,
) {
    sync(commands, holds, actors);
}

/// Visual eviction/parking does not change logical permission. A new render
/// tree receives the retained bit rather than replaying remaining buff rows.
#[allow(clippy::type_complexity)]
pub(super) fn sync(
    mut commands: Commands,
    mut holds: ResMut<Frameholds>,
    mut actors: Query<
        (
            Entity,
            Option<&RemoteUnit>,
            Option<&LocalPlayer>,
            Option<&mut Animator>,
            Option<&mut super::super::units::Asleep>,
            Option<&mut Locomotion>,
        ),
        Or<(With<RemoteUnit>, With<LocalPlayer>)>,
    >,
) {
    for (entity, remote, local, animator, asleep, loco) in &mut actors {
        let Some(id) = local
            .map(|actor| actor.obj_id)
            .or(remote.map(|actor| actor.obj_id))
        else {
            continue;
        };
        if let Some(mut animator) = animator {
            animator.set_framehold(holds.held(id));
        }
        if let Some(mut asleep) = asleep {
            asleep.animator_mut().set_framehold(holds.held(id));
        }
        if holds.released.contains(&id) {
            // An authored end stop is independent of whether a start pose was
            // shown. Rows without an end action only release the hold.
            commands.entity(entity).try_remove::<Acting>();
            if let Some(mut loco) = loco {
                loco.invalidate();
            }
        }
    }
    holds.released.clear();
}

#[cfg(test)]
mod tests;
