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
    /// The native unit kind byte (`+0x410`): 2 is a slave, 5 a mate.
    is_slave: bool,
    slave_template: Option<u32>,
    /// `UnitKind::Mate.owner_id`, the character the mate belongs to (the native `+0x420`).
    mate_owner: Option<u32>,
    slave_tl: Option<u16>,
    /// `UnitKind::Slave.id`: 7 siege weapon, 8 ship cannon/equipment. A cart is 6.
    slave_kind: Option<u32>,
    /// `Unit.model_id`: the model whose +0x2a0 slot gates the driver's 0x67 unbind refusal (a
    /// wheeled-simulation vehicle flag or a ship's 0x198-byte ship object pointer,
    /// `hud/slave.rs`).
    model_id: u32,
    /// Latest replicated unit-movement velocity, in (m/s)^2, and when it arrived. The native
    /// unbind check reads the model's entity velocity (`vtable+0xa4`, `FUN_39345970`); for a
    /// replicated slave this is the same pose the client moves the model with.
    speed_squared: f32,
    /// Forward speed along the movement's facing, in m/s (negative in reverse), and the vehicle
    /// yaw rate in rad/s. `FUN_394aabd0` / `FUN_394aae90` read the local controller's live
    /// getters; these are the replicated stand-in for a remote slave.
    forward_speed: f32,
    turn_rate: f32,
    speed_at: Option<Instant>,
    link: Link,
    equipment: Box<Equipment>,
}

/// The cart this client drives: its controller speed (m/s, signed) and yaw rate (rad/s).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct LocalCart {
    pub obj_id: u32,
    pub speed: f32,
    pub turn_rate: f32,
}

pub(super) fn movement_speed_squared(movement: Option<&Movement>) -> f32 {
    // `Movement::velocity_mps` owns the codec: 30/32767 for type-2 land vehicles and type-3
    // ships, 50/32767 for a type-5 transfer, 1/1024 for unit movements. Reading type 3 at
    // 1/1024 would make a real 4.8 m/s ship read 5.12 and falsely refuse the unbind.
    movement.map_or(0.0, |m| {
        let [x, y, z] = m.velocity_mps();
        x * x + y * y + z * z
    })
}

/// The Summon Info motion of a replicated vehicle movement: forward speed (m/s, signed) and yaw
/// rate (rad/s). `FUN_394aabd0` returns the velocity's length signed by its component along the
/// body's forward axis; `FUN_394aae90` returns the angular velocity's z times 57.29578. For a
/// remote slave the vehicle/ship body carries both.
pub(super) fn movement_motion(movement: &Movement) -> (f32, f32) {
    let [x, y, z] = movement.velocity_mps();
    let (sin, cos) = movement.yaw().sin_cos();
    let along = x * -sin + y * cos;
    let magnitude = (x * x + y * y + z * z).sqrt();
    // The native reduces the forward projection to an integer sign; this stand-in keeps the
    // sign of any nonzero projection, reads 0 across the body, and is negative in reverse.
    let speed = if along == 0.0 {
        0.0
    } else {
        magnitude * along.signum()
    };
    let turn_rate = movement
        .vehicle
        .as_ref()
        .map(|vehicle| vehicle.angular_velocity[2])
        .or_else(|| {
            movement
                .ship
                .as_ref()
                .map(|ship| ship.angular_velocity[2])
        })
        .unwrap_or(0.0);
    (speed, turn_rate)
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
    /// The cart this client drives and its controller motion: no `UnitMoved` reports it.
    pub(super) local_cart: Option<LocalCart>,
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
        let is_slave = matches!(unit.kind, UnitKind::Slave { .. });
        let (slave_template, slave_tl, slave_kind) = match unit.kind {
            UnitKind::Slave {
                id,
                tl_id,
                template_id,
                ..
            } => (Some(template_id), Some(tl_id), Some(id)),
            _ => (None, None, None),
        };
        let mate_owner = match unit.kind {
            UnitKind::Mate { owner_id, .. } => Some(owner_id),
            _ => None,
        };
        let now = Instant::now();
        let (forward_speed, turn_rate) = unit
            .movement
            .as_ref()
            .map(movement_motion)
            .unwrap_or_default();
        self.actors.insert(
            unit.obj_id,
            Actor {
                name: c_name(&unit.name),
                is_slave,
                slave_template,
                mate_owner,
                slave_tl,
                slave_kind,
                model_id: unit.model_id,
                speed_squared: movement_speed_squared(unit.movement.as_ref()),
                forward_speed,
                turn_rate,
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

    /// The slave the local player drives (the driver's seat, attach point 1) and its template id.
    /// A passenger seat, a mate and walking read `None`; the bar itself is built from
    /// [`View::slave_seat`] now, so this is kept for the seat tests.
    #[allow(dead_code)]
    pub(super) fn driver_slave(&self) -> Option<(u32, u32)> {
        self.driver_slave_kind()
            .map(|(obj_id, template, _)| (obj_id, template))
    }

    /// The driver's slave with its `slave_kind_id`: a kind-7/8 slave with a gunner bar uses its
    /// `mount_attached_skills` skill (14372/24501/20161), not the slave's own mount-skill list.
    #[allow(dead_code)]
    pub(super) fn driver_slave_kind(&self) -> Option<(u32, u32, u32)> {
        let link = self.player_attachment()?;
        if !crate::ingame::driver_seat(link.point) {
            return None;
        }
        let actor = self.slave(link.obj_id)?;
        Some((link.obj_id, actor.slave_template?, actor.slave_kind?))
    }

    /// The slave seat the local player sits on, whichever attach point: `FUN_39345f10`
    /// returns the slave for any seat and the point in `local_8` (1 = driver, 2.. = passenger).
    /// The passenger's bar is built from the same seat (`FUN_39204500`).
    pub(super) fn slave_seat(&self) -> Option<(u32, u32, u32, u8)> {
        let link = self.player_attachment()?;
        let actor = self.slave(link.obj_id)?;
        Some((
            link.obj_id,
            actor.slave_template?,
            actor.slave_kind?,
            link.point,
        ))
    }

    /// Whether the player sits on a loaded unit that is not a slave (a mate: riding a mount).
    pub(super) fn riding_mate(&self) -> bool {
        self.player_attachment().is_some_and(|link| {
            self.actors
                .get(&link.obj_id)
                .is_some_and(|actor| actor.slave_template.is_none())
        })
    }

    /// `FUN_392045f0`'s mate function slot: the parent is a mate (kind 5) and
    /// `FUN_3933b1b0(parent) == 0`. That predicate is 1 when the mate's owner is the local
    /// character (the client's other paths use it to act through the player's own mount), so
    /// riding the player's own pet gets no `command_unmount` here. Which native mate field the
    /// spawn's `owner_id` maps to (`+0x420` or another id) is unproven: the port uses the
    /// received `UnitKind::Mate.owner_id`.
    pub(super) fn mate_unmount(&self) -> bool {
        let Some(link) = self.player_attachment() else {
            return false;
        };
        self.actors
            .get(&link.obj_id)
            .and_then(|actor| actor.mate_owner)
            .is_some_and(|owner| self.character != Some(owner))
    }

    /// The slave SCSlaveBound marked as the player's, with its timeline id.
    pub(super) fn bound_slave(&self) -> Option<(u32, u16)> {
        let id = self.bound?;
        Some((id, self.slave(id)?.slave_tl?))
    }

    /// The model of the slave `obj_id` (`Unit.model_id`), for the model-data gates of the
    /// driver's leave (`hud/slave.rs`).
    pub(super) fn slave_model(&self, obj_id: u32) -> Option<u32> {
        self.slave(obj_id).map(|actor| actor.model_id)
    }

    /// The seated slave with its timeline id, for the passenger `FUN_39362170` branch of
    /// `DiscardSiegeWeapon` (no bound master). The native bound branch never uses it: both its
    /// speed check and its packet name the SCSlaveBound slave.
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
        if let Some(cart) = self.local_cart.filter(|cart| cart.obj_id == obj_id) {
            return Some(cart.speed * cart.speed);
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

    /// The player's loaded attachment when it is a slave (the native `+0x410 == 2`), without
    /// needing its template or slave-kind row.
    fn seated_slave(&self) -> Option<(u32, &Actor)> {
        let link = self.player_attachment()?;
        let actor = self.actors.get(&link.obj_id)?;
        actor.is_slave.then_some((link.obj_id, actor))
    }

    /// The native gates both motion getters on the physics entity's class: 3 (living) or 4
    /// (wheeled). Ships are kind 1..=4 or 9 here, so neither branch matches and they read 0.
    fn seat_motion(&self) -> (f32, f32) {
        let Some((obj_id, actor)) = self.seated_slave() else {
            return (0.0, 0.0);
        };
        if actor.slave_kind.is_some_and(|kind| matches!(kind, 1..=4 | 9)) {
            return (0.0, 0.0);
        }
        if let Some(cart) = self.local_cart.filter(|cart| cart.obj_id == obj_id) {
            return (cart.speed, cart.turn_rate);
        }
        if actor
            .speed_at
            .is_some_and(|at| Instant::now().saturating_duration_since(at) > SPEED_STALE_AFTER)
        {
            return (0.0, 0.0);
        }
        (actor.forward_speed, actor.turn_rate)
    }

    /// `X2SiegeWeapon:GetSiegeWeaponSpeed`: the seated slave's forward speed in m/s. The native
    /// `FUN_394aabd0` falls through to number 0 outside its first `FUN_39345f10` branch (this
    /// client's player attachment), never nil, because `slave_info.alb` multiplies the result.
    pub(super) fn siege_weapon_speed(&self) -> f32 {
        self.seat_motion().0
    }

    /// `X2SiegeWeapon:GetSiegeWeaponTurnSpeed`: the seated slave's effective yaw rate in
    /// degrees/s (`FUN_394aae90` multiplies the angular velocity by 57.29578), 0 outside a slave
    /// seat or for a ship.
    pub(super) fn siege_weapon_turn_speed(&self) -> f32 {
        self.seat_motion().1.to_degrees()
    }

    /// Whether `event` takes the player off a slave seat: the native `FUN_393628a0` fires
    /// `SIEGEWEAPON_UNBOUND` (event 98) on that detach, and `FUN_39345dc0` does the same when the
    /// bound slave goes away. `mode_action.alb` F5 hides the Summon Info window on it.
    fn leaves_slave_seat(&self, event: &Event) -> bool {
        let Some((seat, ..)) = self.seated_slave() else {
            return false;
        };
        match event {
            Event::UnitDetached { obj_id, .. } => self.player == Some(*obj_id),
            Event::Slave(SlaveEvent::Removed { obj_id, .. })
            | Event::Slave(SlaveEvent::Despawn { obj_id }) => *obj_id == seat,
            Event::Slave(SlaveEvent::Escaped(escape)) => escape.obj_id == seat,
            Event::UnitsRemoved(ids) => ids.contains(&seat),
            _ => false,
        }
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
                let (forward_speed, turn_rate) = movement_motion(movement);
                actor.speed_squared = movement_speed_squared(Some(movement));
                actor.forward_speed = forward_speed;
                actor.turn_rate = turn_rate;
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
    let unbound = state.borrow().mounted.leaves_slave_seat(event);
    receive(state, event);
    if unbound && let Some(rt) = rt {
        rt.fire_event("SIEGEWEAPON_UNBOUND", ());
    }
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
    let shared = state.clone();
    namespace.set(
        "GetSiegeWeaponSpeed",
        lua.create_function(move |_, _: MultiValue| {
            Ok(shared.borrow().mounted.siege_weapon_speed())
        })?,
    )?;
    let shared = state.clone();
    namespace.set(
        "GetSiegeWeaponTurnSpeed",
        lua.create_function(move |_, _: MultiValue| {
            Ok(shared.borrow().mounted.siege_weapon_turn_speed())
        })?,
    )?;
    // The stock window draws its gear/handbrake/traction block only when this table exists
    // (`FUN_394ab270` needs a vehicle object with `+0x2a0` set). This client has no such
    // controller state, and the captured hauler shows no block, so nil is the honest answer.
    namespace.set(
        "GetVehicleInfo",
        lua.create_function(|_, _: MultiValue| Ok(mlua::Value::Nil))?,
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
    fn only_the_driver_seat_of_a_slave_is_the_slave_bar_source() {
        let state = state();
        own(&state);
        receive(&state, &Event::UnitSpawned(Box::new(slave(20, 60))));
        receive(
            &state,
            &Event::UnitAttached {
                obj_id: 100,
                mount: 20,
                point: 2,
                reason: 0,
            },
        );
        assert_eq!(
            state.borrow().mounted.driver_slave(),
            None,
            "a passenger keeps the normal bar"
        );
        receive(
            &state,
            &Event::UnitAttached {
                obj_id: 100,
                mount: 20,
                point: 1,
                reason: 1,
            },
        );
        assert_eq!(state.borrow().mounted.driver_slave(), Some((20, 60)));
        receive(
            &state,
            &Event::UnitDetached {
                obj_id: 100,
                reason: 0,
            },
        );
        assert_eq!(
            state.borrow().mounted.driver_slave(),
            None,
            "leaving restores the normal bar"
        );
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

    #[test]
    fn summon_info_reads_the_seated_slaves_template_for_type_and_classification() -> mlua::Result<()>
    {
        let state = state();
        let mut game = super::super::data::GameData::default();
        game.mounted.rows.insert(
            146,
            Descriptor {
                name: "Red Farm Hauler".into(),
                kind: "machine",
                customizing: None,
            },
        );
        assert!(state.borrow().game.0.set(game).is_ok());
        own(&state);
        let mut hauler = slave(20, 146);
        hauler.name = "Red Farm Hauler".into();
        receive(&state, &Event::UnitSpawned(Box::new(hauler)));
        receive(
            &state,
            &Event::UnitAttached {
                obj_id: 100,
                mount: 20,
                point: 1,
                reason: 1,
            },
        );
        let lua = Lua::new();
        lua.globals().set("X2SiegeWeapon", lua.create_table()?)?;
        install(&lua, &state)?;
        lua.load(
            r#"
        local v = X2SiegeWeapon:GetMountedSiegeWeaponInfo()
        assert(v.name == 'Red Farm Hauler')
        assert(v.desc_name == 'Red Farm Hauler', 'classification from slaves.name')
        assert(v.kind == 'machine', 'kind key the locale maps to Production Equipment')
        assert(v.unitId == '14')
    "#,
        )
        .exec()?;
        Ok(())
    }

    #[test]
    fn siege_weapon_speed_and_turn_speed_are_live_and_zero_outside_the_seat() -> mlua::Result<()> {
        let state = state();
        own(&state);
        receive(&state, &Event::UnitSpawned(Box::new(slave(20, 146))));
        receive(
            &state,
            &Event::UnitAttached {
                obj_id: 100,
                mount: 20,
                point: 1,
                reason: 1,
            },
        );
        let lua = Lua::new();
        lua.globals().set("X2SiegeWeapon", lua.create_table()?)?;
        install(&lua, &state)?;
        lua.load(
            r#"
        assert(X2SiegeWeapon:GetSiegeWeaponSpeed() == 0)
        assert(X2SiegeWeapon:GetSiegeWeaponTurnSpeed() == 0)
        assert(X2SiegeWeapon:GetVehicleInfo() == nil)
    "#,
        )
        .exec()?;
        state.borrow_mut().mounted.local_cart = Some(LocalCart {
            obj_id: 20,
            speed: 5.0,
            turn_rate: 0.4,
        });
        lua.load(
            r#"
        assert(X2SiegeWeapon:GetSiegeWeaponSpeed() == 5)
        assert(math.abs(X2SiegeWeapon:GetSiegeWeaponTurnSpeed() - 22.918312) < 0.001,
               tostring(X2SiegeWeapon:GetSiegeWeaponTurnSpeed()))
    "#,
        )
        .exec()?;
        receive(
            &state,
            &Event::UnitDetached {
                obj_id: 100,
                reason: 0,
            },
        );
        lua.load(
            "assert(X2SiegeWeapon:GetSiegeWeaponSpeed() == 0); \
             assert(X2SiegeWeapon:GetSiegeWeaponTurnSpeed() == 0)",
        )
        .exec()?;
        Ok(())
    }

    #[test]
    fn siege_weapon_speed_and_turn_speed_are_zero_for_ships() -> mlua::Result<()> {
        let state = state();
        own(&state);
        let mut boat = slave(20, 21);
        if let UnitKind::Slave { id, .. } = &mut boat.kind {
            *id = 2;
        }
        receive(&state, &Event::UnitSpawned(Box::new(boat)));
        receive(
            &state,
            &Event::UnitAttached {
                obj_id: 100,
                mount: 20,
                point: 1,
                reason: 1,
            },
        );
        state.borrow_mut().mounted.local_cart = Some(LocalCart {
            obj_id: 20,
            speed: 5.0,
            turn_rate: 0.4,
        });
        let lua = Lua::new();
        lua.globals().set("X2SiegeWeapon", lua.create_table()?)?;
        install(&lua, &state)?;
        lua.load(
            r#"
        assert(X2SiegeWeapon:GetSiegeWeaponSpeed() == 0)
        assert(X2SiegeWeapon:GetSiegeWeaponTurnSpeed() == 0)
    "#,
        )
        .exec()?;
        Ok(())
    }

    #[test]
    fn leaving_a_slave_seat_is_the_unbind_event() {
        let state = state();
        own(&state);
        receive(&state, &Event::UnitSpawned(Box::new(slave(20, 146))));
        receive(
            &state,
            &Event::UnitAttached {
                obj_id: 100,
                mount: 20,
                point: 1,
                reason: 1,
            },
        );
        assert!(
            state
                .borrow()
                .mounted
                .leaves_slave_seat(&Event::UnitDetached {
                    obj_id: 100,
                    reason: 0,
                })
        );
        assert!(
            state
                .borrow()
                .mounted
                .leaves_slave_seat(&Event::Slave(SlaveEvent::Removed {
                    obj_id: 20,
                    tl_id: 8,
                }))
        );
        assert!(
            !state
                .borrow()
                .mounted
                .leaves_slave_seat(&Event::UnitDetached {
                    obj_id: 101,
                    reason: 0,
                })
        );
    }

    #[test]
    fn siege_weapon_speed_and_turn_speed_follow_replicated_vehicle_motion() -> mlua::Result<()> {
        let state = state();
        own(&state);
        receive(&state, &Event::UnitSpawned(Box::new(slave(20, 146))));
        receive(
            &state,
            &Event::UnitAttached {
                obj_id: 100,
                mount: 20,
                point: 1,
                reason: 1,
            },
        );
        let mut movement = Movement::unit(aa_net::Pos::new(0.0, 0.0, 0.0), 0.0, 1);
        movement.move_type = 2;
        movement.velocity = [0, 5461, 0];
        movement.vehicle_yaw = Some(0.0);
        movement.vehicle = Some(aa_net::model::VehicleMove {
            angular_velocity: [0.0, 0.0, 0.4],
            ..Default::default()
        });
        receive(
            &state,
            &Event::UnitMoved {
                obj_id: 20,
                movement,
            },
        );
        let lua = Lua::new();
        lua.globals().set("X2SiegeWeapon", lua.create_table()?)?;
        install(&lua, &state)?;
        lua.load(
            r#"
        local speed = X2SiegeWeapon:GetSiegeWeaponSpeed()
        assert(math.abs(speed - 5.0) < 0.01, tostring(speed))
        local turn = X2SiegeWeapon:GetSiegeWeaponTurnSpeed()
        assert(math.abs(turn - 22.918312) < 0.001, tostring(turn))
    "#,
        )
        .exec()?;
        Ok(())
    }
}
