//! Read-only original mounted-information metadata over receipt-local objects.
//! Motion, equipment actions and mode-action construction remain separate ports.

mod equipment;

use std::collections::HashMap;
use std::time::Duration;

use aa_net::{
    Event,
    model::{Attachment, Equipment, Movement, Unit, UnitKind},
    slave::SlaveEvent,
};
use mlua::{Lua, MultiValue, Table};
use rusqlite::Connection;
use web_time::Instant;

use super::api::Shared;

/// Safety bound for a `CSBindSlave` the server neither accepts nor refuses. Not a 1.2 value: the
/// native client keeps its pending timeline until a reply, but a lost reply must not block every
/// later click with 0x13.
pub(super) const BIND_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a replicated velocity may stand in for the slave's speed. Not a 1.2 value: the native
/// check reads a live engine getter (`vtable+0xa4`), while a stopped slave sends no more movement,
/// so the last report must not keep refusing the unbind.
pub(super) const SPEED_STALE_AFTER: Duration = Duration::from_millis(500);

#[derive(Default)]
pub(super) struct Tables {
    rows: HashMap<u32, Descriptor>,
    equipment: equipment::Tables,
}

struct Descriptor {
    name: String,
    kind: &'static str,
    customizing: Option<u32>,
}

impl Tables {
    pub(super) fn load(db: &Connection) -> rusqlite::Result<Self> {
        let mut rows = HashMap::new();
        let mut query = db.prepare(
            "SELECT s.id,coalesce(nullif(l.en_us,''),s.name,''),s.slave_kind_id,s.slave_customizing_id \
             FROM slaves s LEFT JOIN localized_texts l ON l.tbl_name='slaves' \
             AND l.tbl_column_name='name' AND l.idx=s.id",
        )?;
        for row in query.query_map([], |r| {
            Ok((
                r.get::<_, u32>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<u32>>(2)?,
                r.get::<_, Option<u32>>(3)?,
            ))
        })? {
            let (id, name, kind, customizing) = row?;
            rows.insert(
                id,
                Descriptor {
                    name,
                    kind: kind_name(kind.unwrap_or(0)),
                    customizing,
                },
            );
        }
        Ok(Self {
            rows,
            equipment: equipment::Tables::load(db)?,
        })
    }
}

fn kind_name(kind: u32) -> &'static str {
    match kind {
        1 => "big_sailing_ship",
        2 => "small_sailing_ship",
        3 => "speedboat",
        4 => "boat",
        5 => "tank",
        6 => "machine",
        7 => "siege_weapon",
        8 => "slave_equipment",
        9 => "fishboat",
        _ => "invalid slave_kind",
    }
}

#[derive(Clone, Copy, Default)]
enum Link {
    #[default]
    Unknown,
    Detached,
    Attached(Attachment),
}

struct Actor {
    name: Vec<u8>,
    slave_template: Option<u32>,
    slave_tl: Option<u16>,
    /// Latest replicated unit-movement velocity, in (m/s)^2, and when it arrived. The native
    /// unbind check reads an engine getter (`vtable+0xa4`, `FUN_39345aa0`); this source is
    /// unverified.
    speed_squared: f32,
    speed_at: Option<Instant>,
    link: Link,
    equipment: Box<Equipment>,
}

pub(super) fn movement_speed_squared(movement: Option<&Movement>) -> f32 {
    movement.map_or(0.0, |m| {
        // Type 2 (land vehicle) velocities use the 30/32767 codec (the codec's VEL_SCALE_VEHICLE
        // is the writer's 1/30); unit movements use 1/1024.
        let scale = if m.move_type == 2 {
            aa_net::vehicle::VEL_VEHICLE / 32767.0
        } else {
            1.0 / 1024.0
        };
        let [x, y, z] = m.velocity;
        (f32::from(x).powi(2) + f32::from(y).powi(2) + f32::from(z).powi(2)) * scale * scale
    })
}

fn c_name(name: &str) -> Vec<u8> {
    let bytes = name
        .as_bytes()
        .split(|b| *b == 0)
        .next()
        .unwrap_or_default();
    bytes[..bytes.len().min(1024)].to_vec()
}

fn snapshot_link(unit: &Unit) -> Link {
    if let Some(link) = unit.attached_to {
        Link::Attached(link)
    } else {
        // Non-character archive fields do not establish a player's primary
        // relation. Absent typed state stays unknown until an attach/detach.
        Link::Unknown
    }
}

#[derive(Default)]
pub(super) struct View {
    config: aa_net::state::GameConfig,
    character: Option<u32>,
    player: Option<u32>,
    actors: HashMap<u32, Actor>,
    bound: Option<u32>,
    /// A CSBindSlave sent for `(object, timeline)` that has no acceptance or refusal yet, and when.
    pending_bind: Option<(u32, u16)>,
    pending_bind_since: Option<Instant>,
    /// The local player's own unit died and has not been resurrected.
    player_dead: bool,
    /// Native passenger walk-off throttle (`FUN_39362170`, 1500 ms).
    last_walk_off: Option<Instant>,
    pending: Vec<(u32, Attachment)>,
    /// The cart this client drives and its controller speed: no `UnitMoved` reports it.
    pub(super) local_cart: Option<(u32, f32)>,
}

impl View {
    fn reset_objects(&mut self) {
        // InitialConfig belongs to the connection, not to one character/world.
        let config = std::mem::take(&mut self.config);
        *self = Self {
            config,
            ..Default::default()
        };
    }

    fn load(&mut self, unit: &Unit) {
        if unit.obj_id == 0 || self.actors.contains_key(&unit.obj_id) {
            return;
        }
        let (slave_template, slave_tl) = match unit.kind {
            UnitKind::Slave {
                tl_id, template_id, ..
            } => (Some(template_id), Some(tl_id)),
            _ => (None, None),
        };
        let now = Instant::now();
        self.actors.insert(
            unit.obj_id,
            Actor {
                name: c_name(&unit.name),
                slave_template,
                slave_tl,
                speed_squared: movement_speed_squared(unit.movement.as_ref()),
                speed_at: unit.movement.as_ref().map(|_| now),
                link: snapshot_link(unit),
                equipment: Box::new(unit.equipment.clone()),
            },
        );
        let mut ready = Vec::new();
        self.pending.retain(|(child, link)| {
            if link.obj_id == unit.obj_id {
                ready.push((*child, *link));
                false
            } else {
                true
            }
        });
        for (child, link) in ready {
            self.attach(child, link);
        }
    }

    fn attach(&mut self, child: u32, link: Attachment) {
        if link.obj_id == 0 || !self.actors.contains_key(&child) {
            return;
        }
        if self.actors.contains_key(&link.obj_id) {
            self.actors.get_mut(&child).unwrap().link = Link::Attached(link);
        } else if !self
            .pending
            .iter()
            .any(|(id, old)| *id == child && old.obj_id == link.obj_id)
        {
            // The native queue suppresses duplicate parent/child requests;
            // a later point for the same missing pair does not replace the first.
            self.pending.push((child, link));
        }
    }

    fn slave(&self, id: u32) -> Option<&Actor> {
        self.actors
            .get(&id)
            .filter(|actor| actor.slave_template.is_some())
    }

    fn selected(&self) -> Option<(u32, &Actor)> {
        let player = self.actors.get(&self.player?)?;
        match player.link {
            Link::Unknown => return None,
            Link::Attached(link) => {
                if let Some(actor) = self.slave(link.obj_id) {
                    return Some((link.obj_id, actor));
                }
            }
            Link::Detached => {}
        }
        let id = self.bound?;
        Some((id, self.slave(id)?))
    }

    /// The player's received attachment, when the mounted view has one.
    pub(super) fn player_attachment(&self) -> Option<Attachment> {
        match self.actors.get(&self.player?)?.link {
            Link::Attached(link) => Some(link),
            Link::Unknown | Link::Detached => None,
        }
    }

    /// Whether the player sits on a unit that is not a slave (a mate: riding a mount).
    pub(super) fn riding_mate(&self) -> bool {
        self.player_attachment().is_some_and(|link| self.slave(link.obj_id).is_none())
    }

    /// The slave SCSlaveBound marked as the player's, with its timeline id.
    pub(super) fn bound_slave(&self) -> Option<(u32, u16)> {
        let id = self.bound?;
        Some((id, self.slave(id)?.slave_tl?))
    }

    /// The slave the player leaves with `DiscardSiegeWeapon`: the seated one (the attachment wins,
    /// as for the boarding checks), else the SCSlaveBound fallback, with its timeline id.
    pub(super) fn discard_slave(&self) -> Option<(u32, u16)> {
        if let Some(link) = self.player_attachment()
            && let Some(actor) = self.slave(link.obj_id)
        {
            return Some((link.obj_id, actor.slave_tl?));
        }
        self.bound_slave()
    }

    fn clear_pending_bind(&mut self) {
        self.pending_bind = None;
        self.pending_bind_since = None;
    }

    /// The player left a seat (a detach receipt): drop its queued links and end the bound
    /// fallback when it named the seat's slave.
    fn detach(&mut self, obj_id: u32) {
        self.pending.retain(|(child, _)| *child != obj_id);
        if self.player == Some(obj_id) {
            self.clear_pending_bind();
        }
        if let Some(actor) = self.actors.get_mut(&obj_id) {
            if self.player == Some(obj_id)
                && let Link::Attached(link) = actor.link
                && self.bound == Some(link.obj_id)
            {
                // Conservative invalidation: the native continuation also
                // tests a manager-validity member whose typed map is held.
                // A fresh Bound receipt can re-establish the fallback. The
                // driver and passenger exits both end the bound relation.
                self.bound = None;
            }
            actor.link = Link::Detached;
        }
    }

    /// Whether a bind is still awaiting its reply, retiring it after [`BIND_TIMEOUT`] of silence.
    pub(super) fn bind_pending(&mut self, now: Instant) -> bool {
        if self
            .pending_bind_since
            .is_some_and(|since| now.saturating_duration_since(since) >= BIND_TIMEOUT)
        {
            self.clear_pending_bind();
        }
        self.pending_bind.is_some()
    }

    pub(super) fn note_bind(&mut self, obj_id: u32, tl_id: u16) {
        self.pending_bind = Some((obj_id, tl_id));
        self.pending_bind_since = Some(Instant::now());
    }

    pub(super) fn player_dead(&self) -> bool {
        self.player_dead
    }

    /// Latest replicated velocity of `obj_id`, squared (m/s)^2, or 0 once it is older than
    /// [`SPEED_STALE_AFTER`]; `None` when the object is not loaded. The cart this client drives
    /// uses its controller speed instead.
    pub(super) fn speed_squared(&self, obj_id: u32, now: Instant) -> Option<f32> {
        let actor = self.actors.get(&obj_id)?;
        if let Some((_, speed)) = self.local_cart.filter(|(id, _)| *id == obj_id) {
            return Some(speed * speed);
        }
        Some(
            if actor
                .speed_at
                .is_some_and(|at| now.saturating_duration_since(at) > SPEED_STALE_AFTER)
            {
                0.0
            } else {
                actor.speed_squared
            },
        )
    }

    /// Starts the native passenger walk-off throttle (`_DAT_3a169948`, 1500 ms).
    pub(super) fn walk_off_throttle(&mut self, now: Instant) -> bool {
        if self.last_walk_off.is_some_and(|last| {
            now.saturating_duration_since(last) < super::slave::WALK_OFF_THROTTLE
        }) {
            return false;
        }
        self.last_walk_off = Some(now);
        true
    }
}

/// Update before Lua can query this receipt; do not read frame-final aa-net resources.
pub(super) fn receive(state: &Shared, event: &Event) {
    let mut data = state.borrow_mut();
    let view = &mut data.mounted;
    view.config.apply(event);
    match event {
        Event::CharacterSelected(selected) => {
            view.reset_objects();
            view.character = Some(selected.character.id);
        }
        Event::PlayerSpawn(spawn) => {
            if view
                .character
                .is_some_and(|id| id != spawn.state.character.id)
            {
                view.reset_objects();
            }
            view.character = Some(spawn.state.character.id);
            view.player = Some(spawn.obj_id);
            view.player_dead = spawn.unit.dead;
            view.load(&spawn.unit);
        }
        Event::UnitSpawned(unit) => view.load(unit),
        Event::OwnUnitSnapshot(unit) if view.player == Some(unit.obj_id) => view.load(unit),
        Event::Slave(SlaveEvent::Bound { master_id, obj_id })
            if view.character == Some(*master_id) && view.slave(*obj_id).is_some() =>
        {
            view.bound = Some(*obj_id);
            view.clear_pending_bind();
        }
        // RequestRejected is emitted by SlaveCommand::validate, not by the server.
        Event::Slave(SlaveEvent::RequestRejected {
            command: aa_net::slave_control::SlaveCommand::Bind { tl_id },
            ..
        }) if view
            .pending_bind
            .is_some_and(|(_, pending)| pending == *tl_id) =>
        {
            view.clear_pending_bind();
        }
        // SCErrorMsg for a refused CSBindSlave: SLAVE_CANNOT_BIND, SLAVE_ALREADY_HAS_MASTER,
        // SLAVE_UNBIND_FIRST (the server can also stay silent, which BIND_TIMEOUT covers).
        Event::ServerError { code } if matches!(*code, 0x60 | 0x61 | 0x66) => {
            view.clear_pending_bind();
        }
        Event::Slave(SlaveEvent::Removed { obj_id, .. }) => {
            if view.bound == Some(*obj_id) {
                view.bound = None;
            }
            if view
                .pending_bind
                .is_some_and(|(pending, _)| pending == *obj_id)
            {
                view.clear_pending_bind();
            }
        }
        Event::Slave(SlaveEvent::Despawn { obj_id }) => {
            if view.bound == Some(*obj_id) {
                view.bound = None;
            }
            if view
                .pending_bind
                .is_some_and(|(pending, _)| pending == *obj_id)
            {
                view.clear_pending_bind();
            }
        }
        Event::Slave(SlaveEvent::Escaped(escape)) => {
            if view.bound == Some(escape.obj_id) {
                view.bound = None;
            }
            if view
                .pending_bind
                .is_some_and(|(pending, _)| pending == escape.obj_id)
            {
                view.clear_pending_bind();
            }
        }
        Event::UnitMoved { obj_id, movement } => {
            if let Some(actor) = view.actors.get_mut(obj_id) {
                actor.speed_squared = movement_speed_squared(Some(movement));
                actor.speed_at = Some(Instant::now());
            }
        }
        Event::UnitAttached {
            obj_id,
            mount,
            point,
            reason,
        } => {
            view.attach(
                *obj_id,
                Attachment {
                    obj_id: *mount,
                    point: *point,
                    reason: *reason,
                },
            );
        }
        Event::UnitDetached { obj_id, .. } => view.detach(*obj_id),
        Event::UnitDeath { obj_id } if view.player == Some(*obj_id) => {
            view.player_dead = true;
            view.clear_pending_bind();
        }
        Event::UnitPoints { obj_id, hp, .. } if view.player == Some(*obj_id) && *hp > 0 => {
            view.player_dead = false;
        }
        Event::Resurrected { obj_id, .. } if view.player == Some(*obj_id) => {
            view.player_dead = false;
            // `Player::apply` ends the seat on resurrection even without a UnitDetached
            // receipt (`aa-net/src/state.rs:193`); keep the cached link and the bound
            // fallback in step or the old cart refuses boarding another with 0x66.
            view.detach(*obj_id);
            view.bound = None;
        }
        Event::UnitNameChanged { obj_id, name } => {
            if let Some(actor) = view.actors.get_mut(obj_id) {
                actor.name = c_name(name);
            }
        }
        Event::EquipmentChanged { obj_id, items } => {
            if let Some(actor) = view.actors.get_mut(obj_id) {
                for (slot, item) in items {
                    if let Some(current) = actor.equipment.slots.get_mut(usize::from(*slot)) {
                        *current = item.clone();
                    }
                }
            }
        }
        Event::UnitsRemoved(ids) => {
            for id in ids {
                view.actors.remove(id);
            }
            view.pending.retain(|(child, _)| !ids.contains(child));
            if view
                .pending_bind
                .is_some_and(|(obj_id, _)| ids.contains(&obj_id))
            {
                view.clear_pending_bind();
            }
            if view.bound.is_some_and(|id| ids.contains(&id)) {
                view.bound = None;
            }
            if view.player.is_some_and(|id| ids.contains(&id)) {
                view.player = None;
            }
        }
        Event::ReadyToConnect { .. }
        | Event::LoginSucceeded(_)
        | Event::LoginFailed { .. }
        | Event::LeftWorld { .. }
        | Event::Disconnected { .. }
        | Event::Kicked { .. }
        | Event::PhaseChanged(aa_net::Phase::Connecting | aa_net::Phase::Authenticating) => {
            view.reset_objects();
        }
        _ => {}
    }
}

pub(super) fn dispatch(rt: Option<&aa_ui::UiRuntime>, state: &Shared, event: &Event) {
    receive(state, event);
    equipment::notify(rt, event);
}

pub(super) fn install(lua: &Lua, state: &Shared) -> mlua::Result<()> {
    let namespace: Table = lua.globals().get("X2SiegeWeapon")?;
    let shared = state.clone();
    namespace.set(
        "GetMountedSiegeWeaponInfo",
        lua.create_function(move |lua, _: MultiValue| {
            // Copy the result before allocating a Lua table, keeping reentry free
            // of a held HudData borrow.
            let info = {
                let data = shared.borrow();
                data.mounted.selected().and_then(|(id, actor)| {
                    let descriptor = data.game.get()?.mounted.rows.get(&actor.slave_template?)?;
                    Some((
                        actor.name.clone(),
                        descriptor.name.clone(),
                        descriptor.kind,
                        id,
                    ))
                })
            };
            let Some((name, description, kind, id)) = info else {
                return Ok(None);
            };
            let value = lua.create_table()?;
            value.set("name", lua.create_string(&name)?)?;
            value.set("desc_name", description)?;
            value.set("kind", kind)?;
            value.set("unitId", format!("{id:x}"))?;
            Ok(Some(value))
        })?,
    )?;
    equipment::install(lua, state)?;
    super::slave::install(lua, state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, rc::Rc};

    fn state() -> Shared {
        Rc::new(RefCell::new(super::super::api::HudData::default()))
    }

    fn slave(id: u32, template_id: u32) -> Unit {
        let mut unit = super::super::expansion::fixture::spawn().unit;
        unit.obj_id = id;
        unit.name = format!("Received {id}");
        unit.kind = UnitKind::Slave {
            id: 700,
            tl_id: 8,
            template_id,
            summoner_id: 100,
        };
        unit
    }

    fn own(state: &Shared) {
        let spawn = super::super::expansion::fixture::spawn();
        receive(
            state,
            &Event::CharacterSelected(Box::new(spawn.state.clone())),
        );
        receive(state, &Event::PlayerSpawn(Box::new(spawn)));
    }

    fn selected(state: &Shared) -> Option<u32> {
        state.borrow().mounted.selected().map(|(id, _)| id)
    }

    fn bound(state: &Shared, id: u32) {
        receive(
            state,
            &Event::Slave(SlaveEvent::Bound {
                master_id: 47,
                obj_id: id,
            }),
        );
    }

    #[test]
    fn binding_requires_local_context_a_loaded_slave_and_known_primary_state() {
        let state = state();
        receive(&state, &Event::UnitSpawned(Box::new(slave(20, 1))));
        bound(&state, 20);
        assert_eq!(selected(&state), None);
        own(&state);
        bound(&state, 20);
        receive(&state, &Event::UnitSpawned(Box::new(slave(20, 1))));
        assert!(
            state.borrow().mounted.bound.is_none(),
            "missing object is not a deferred bind"
        );
        receive(
            &state,
            &Event::Slave(SlaveEvent::Bound {
                master_id: 48,
                obj_id: 20,
            }),
        );
        assert!(state.borrow().mounted.bound.is_none());
        bound(&state, 20);
        assert_eq!(
            selected(&state),
            None,
            "unknown player archive is not an empty primary"
        );
        receive(
            &state,
            &Event::UnitDetached {
                obj_id: 100,
                reason: 0,
            },
        );
        assert_eq!(selected(&state), Some(20));
    }

    #[test]
    fn loaded_primary_wins_while_missing_parent_defers_without_clobbering_it() {
        let state = state();
        own(&state);
        receive(&state, &Event::UnitSpawned(Box::new(slave(20, 1))));
        receive(&state, &Event::UnitSpawned(Box::new(slave(30, 2))));
        bound(&state, 20);
        receive(
            &state,
            &Event::UnitAttached {
                obj_id: 100,
                mount: 30,
                point: 2,
                reason: 0,
            },
        );
        assert_eq!(selected(&state), Some(30));
        receive(
            &state,
            &Event::UnitAttached {
                obj_id: 100,
                mount: 40,
                point: 3,
                reason: 0,
            },
        );
        receive(
            &state,
            &Event::UnitAttached {
                obj_id: 100,
                mount: 40,
                point: 7,
                reason: 0,
            },
        );
        assert_eq!(selected(&state), Some(30));
        receive(&state, &Event::UnitSpawned(Box::new(slave(40, 1))));
        assert_eq!(selected(&state), Some(40));
        assert!(matches!(
            state.borrow().mounted.actors[&100].link,
            Link::Attached(Attachment {
                obj_id: 40,
                point: 3,
                reason: 0
            })
        ));
        receive(&state, &Event::UnitsRemoved(vec![40]));
        assert_eq!(
            selected(&state),
            Some(20),
            "unresolved primary permits admitted fallback"
        );
    }

    #[test]
    fn detach_cancels_pending_child_and_conservatively_retires_matching_driver_fallback() {
        let state = state();
        own(&state);
        receive(&state, &Event::UnitSpawned(Box::new(slave(20, 1))));
        bound(&state, 20);
        receive(
            &state,
            &Event::UnitAttached {
                obj_id: 100,
                mount: 20,
                point: 1,
                reason: 0,
            },
        );
        receive(
            &state,
            &Event::UnitAttached {
                obj_id: 100,
                mount: 30,
                point: 2,
                reason: 0,
            },
        );
        receive(
            &state,
            &Event::UnitDetached {
                obj_id: 100,
                reason: 0,
            },
        );
        receive(&state, &Event::UnitSpawned(Box::new(slave(30, 1))));
        assert_eq!(selected(&state), None);
        assert!(state.borrow().mounted.pending.is_empty());
        bound(&state, 20);
        assert_eq!(
            selected(&state),
            Some(20),
            "fresh binding restores known fallback"
        );
    }

    #[test]
    fn duplicate_snapshot_does_not_rename_but_name_receipt_and_context_retirement_apply() {
        let state = state();
        own(&state);
        receive(
            &state,
            &Event::UnitDetached {
                obj_id: 100,
                reason: 0,
            },
        );
        receive(&state, &Event::UnitSpawned(Box::new(slave(20, 1))));
        bound(&state, 20);
        let mut duplicate = slave(20, 2);
        duplicate.name = "Ignored duplicate".into();
        receive(&state, &Event::UnitSpawned(Box::new(duplicate)));
        assert_eq!(state.borrow().mounted.actors[&20].name, b"Received 20");
        receive(
            &state,
            &Event::UnitNameChanged {
                obj_id: 20,
                name: "Current\0suffix".into(),
            },
        );
        assert_eq!(state.borrow().mounted.actors[&20].name, b"Current");
        receive(&state, &Event::PhaseChanged(aa_net::Phase::Connecting));
        assert_eq!(selected(&state), None);
        assert!(state.borrow().mounted.actors.is_empty());
        assert!(state.borrow().mounted.pending.is_empty());
    }

    #[test]
    fn public_getter_uses_primary_descriptor_and_unsigned_lowercase_hex() -> mlua::Result<()> {
        let state = state();
        let mut game = super::super::data::GameData::default();
        game.mounted.rows.insert(
            1,
            Descriptor {
                name: "Class".into(),
                kind: "boat",
                customizing: None,
            },
        );
        assert!(state.borrow().game.0.set(game).is_ok());
        own(&state);
        receive(
            &state,
            &Event::UnitDetached {
                obj_id: 100,
                reason: 0,
            },
        );
        receive(&state, &Event::UnitSpawned(Box::new(slave(0xfedcba98, 1))));
        bound(&state, 0xfedcba98);
        let lua = Lua::new();
        lua.globals().set("X2SiegeWeapon", lua.create_table()?)?;
        install(&lua, &state)?;
        lua.load("local v=X2SiegeWeapon:GetMountedSiegeWeaponInfo(); assert(v.unitId=='fedcba98' and v.desc_name=='Class' and v.kind=='boat')").exec()?;
        receive(&state, &Event::UnitSpawned(Box::new(slave(30, 99))));
        receive(
            &state,
            &Event::UnitAttached {
                obj_id: 100,
                mount: 30,
                point: 2,
                reason: 0,
            },
        );
        lua.load("assert(X2SiegeWeapon:GetMountedSiegeWeaponInfo()==nil)")
            .exec()?;
        Ok(())
    }
}
