//! What the app knows about the server, built by folding [`Event`]s: the world list, the character
//! list, the local player and the units and doodads in view.
//!
//! Each piece is a small struct with `apply(&Event)`; the Bevy plugin registers them as separate
//! resources, [`WorldState`] bundles them for everyone else.

use std::collections::HashMap;

use crate::login::{AccountInfo, WorldList, WorldQueue};
use crate::model::{Character, Doodad, Unit};
use crate::session::{Event, Phase, PlayerSpawn};

macro_rules! resource {
    ($(#[$m:meta])* pub struct $name:ident $body:tt) => {
        $(#[$m])*
        #[derive(Clone, Debug, Default)]
        #[cfg_attr(feature = "bevy", derive(bevy::ecs::resource::Resource))]
        pub struct $name $body
    };
}

mod stats;
pub use stats::{BuffEntry, Buffs, PlayerStats, LABOR_MAX, LABOR_MAX_PREMIUM};

resource! {
    /// The login server's worlds and the account's characters on them, plus the queue position.
    pub struct Worlds {
        pub list: WorldList,
        pub queue: Option<WorldQueue>,
        pub account: Option<AccountInfo>,
    }
}

impl Worlds {
    /// Returns true when the data changed.
    pub fn apply(&mut self, e: &Event) -> bool {
        match e {
            Event::LoginSucceeded(a) => self.account = Some(*a),
            Event::WorldList(l) => {
                self.list = l.clone();
                self.queue = None;
            }
            Event::WorldQueue(q) => self.queue = Some(*q),
            Event::ReadyToConnect { .. } | Event::EnterWorldCancelled | Event::EnterWorldDenied { .. } => {
                self.queue = None
            }
            Event::Disconnected { .. } | Event::LoginFailed { .. } => *self = Self::default(),
            _ => return false,
        }
        true
    }
}

resource! {
    /// The characters on the connected world.
    pub struct Characters {
        pub list: Vec<Character>,
        /// Congestion per race (index = race id), for character creation.
        pub race_congestion: [u8; 9],
    }
}

impl Characters {
    pub fn apply(&mut self, e: &Event) -> bool {
        match e {
            Event::CharacterList(l) => self.list = l.clone(),
            Event::RaceCongestion(c) => self.race_congestion = *c,
            Event::CharacterDeleteCancelled { character_id, restored: true } => {
                let Some(c) = self.list.iter_mut().find(|c| c.id == *character_id) else { return false };
                (c.delete_request_time, c.delete_time) = (0, 0);
            }
            Event::Disconnected { .. } => *self = Self::default(),
            _ => return false,
        }
        true
    }

    pub fn by_id(&self, id: u32) -> Option<&Character> {
        self.list.iter().find(|c| c.id == id)
    }
}

resource! {
    /// The local player, once it is in the world.
    pub struct Player {
        pub spawn: Option<PlayerSpawn>,
    }
}

impl Player {
    pub fn apply(&mut self, e: &Event) -> bool {
        match e {
            Event::PlayerSpawn(s) => self.spawn = Some((**s).clone()),
            Event::LeftWorld { .. } | Event::Disconnected { .. } => {
                if self.spawn.take().is_none() {
                    return false;
                }
            }
            _ => return false,
        }
        true
    }
}

resource! {
    /// Units in view, by object id (the local player is in [`Player`], not here).
    pub struct Units {
        pub by_id: HashMap<u32, Unit>,
    }
}

impl Units {
    pub fn apply(&mut self, e: &Event) -> bool {
        match e {
            Event::UnitSpawned(u) => {
                self.by_id.insert(u.obj_id, (**u).clone());
            }
            Event::UnitMoved { obj_id, movement } => {
                let Some(u) = self.by_id.get_mut(obj_id) else { return false };
                u.pos = movement.pos;
                if movement.move_type <= 1 {
                    u.rotation[2] = movement.yaw();
                }
                u.movement = Some(movement.clone());
            }
            Event::UnitsRemoved(ids) => {
                for id in ids {
                    self.by_id.remove(id);
                }
            }
            Event::UnitPoints { obj_id, hp, mp } => {
                let Some(u) = self.by_id.get_mut(obj_id) else { return false };
                (u.hp, u.mp) = (*hp, *mp);
                u.max_hp = u.max_hp.max(*hp);
                u.max_mp = u.max_mp.max(*mp);
            }
            Event::LevelChanged { obj_id, level } => {
                let Some(u) = self.by_id.get_mut(obj_id) else { return false };
                u.level = *level;
            }
            Event::UnitFactionChanged { obj_id, faction_id, .. } => {
                let Some(u) = self.by_id.get_mut(obj_id) else { return false };
                u.faction_id = *faction_id;
            }
            Event::LeftWorld { .. } | Event::Disconnected { .. } => {
                if self.by_id.is_empty() {
                    return false;
                }
                self.by_id.clear();
            }
            _ => return false,
        }
        true
    }

    pub fn get(&self, obj_id: u32) -> Option<&Unit> {
        self.by_id.get(&obj_id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Unit> {
        self.by_id.values()
    }

    pub fn characters(&self) -> impl Iterator<Item = &Unit> {
        self.by_id.values().filter(|u| u.is_character())
    }

    pub fn npcs(&self) -> impl Iterator<Item = &Unit> {
        self.by_id.values().filter(|u| u.is_npc())
    }
}

resource! {
    /// Doodads in view, by object id.
    pub struct Doodads {
        pub by_id: HashMap<u32, Doodad>,
    }
}

impl Doodads {
    pub fn apply(&mut self, e: &Event) -> bool {
        match e {
            Event::DoodadsSpawned(v) => {
                for d in v {
                    self.by_id.insert(d.obj_id, d.clone());
                }
            }
            Event::DoodadsRemoved(ids) => {
                for id in ids {
                    self.by_id.remove(id);
                }
            }
            Event::DoodadPhaseChanged { obj_id, func_group_id } => {
                let Some(d) = self.by_id.get_mut(obj_id) else { return false };
                d.func_group_id = *func_group_id;
            }
            Event::LeftWorld { .. } | Event::Disconnected { .. } => {
                if self.by_id.is_empty() {
                    return false;
                }
                self.by_id.clear();
            }
            _ => return false,
        }
        true
    }
}

resource! {
    /// The session phase (see [`Phase`]).
    pub struct Status {
        pub phase: Phase,
        /// Last connection or login failure, for display.
        pub last_error: Option<String>,
    }
}

impl Status {
    pub fn apply(&mut self, e: &Event) -> bool {
        match e {
            Event::PhaseChanged(p) => self.phase = *p,
            Event::LoginFailed { message } | Event::ConnectFailed { message } | Event::Disconnected { message } => {
                self.last_error = Some(message.clone())
            }
            Event::LoginSucceeded(_) | Event::LobbyReady => self.last_error = None,
            _ => return false,
        }
        true
    }
}

/// All of the above in one value.
#[derive(Clone, Debug, Default)]
pub struct WorldState {
    pub worlds: Worlds,
    pub characters: Characters,
    pub player: Player,
    pub units: Units,
    pub doodads: Doodads,
    pub status: Status,
    pub stats: PlayerStats,
    pub buffs: Buffs,
}

impl WorldState {
    pub fn apply(&mut self, e: &Event) {
        self.worlds.apply(e);
        self.characters.apply(e);
        self.player.apply(e);
        self.units.apply(e);
        self.doodads.apply(e);
        self.status.apply(e);
        self.stats.apply(e);
        self.buffs.apply(e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Equipment, ModelParams, Movement, UnitKind};
    use crate::wire::Pos;

    pub(crate) fn unit(obj_id: u32) -> Unit {
        Unit {
            obj_id,
            name: format!("u{obj_id}"),
            kind: UnitKind::Npc { template_id: 1, owner_id: 0 },
            owner_name: String::new(),
            pos: Pos::new(1.0, 2.0, 3.0),
            rotation: [0.0; 3],
            scale: 1.0,
            level: 1,
            model_id: 1,
            equipment: Equipment::default(),
            model_params: ModelParams::default(),
            hp: 10,
            mp: 0,
            race_gender: 0,
            movement: None,
            faction_id: 0,
            expedition_id: 0,
            appellation: 0,
            flags: 0,
            skills: Vec::new(),
            passive_buffs: Vec::new(),
            ability_exp: Vec::new(),
            active_abilities: Vec::new(),
            buffs: Vec::new(),
            max_hp: 10,
            max_mp: 0,
        }
    }

    #[test]
    fn units_follow_spawn_move_points_and_removal() {
        let mut s = WorldState::default();
        s.apply(&Event::UnitSpawned(Box::new(unit(5))));
        s.apply(&Event::UnitSpawned(Box::new(unit(6))));
        assert_eq!(s.units.by_id.len(), 2);
        s.apply(&Event::UnitMoved {
            obj_id: 5,
            movement: Movement::unit(Pos::new(9.0, 8.0, 7.0), 0.0, 1),
        });
        assert!((s.units.get(5).unwrap().pos.x - 9.0).abs() < 1e-6);
        s.apply(&Event::UnitPoints { obj_id: 5, hp: 3, mp: 4 });
        assert_eq!(s.units.get(5).unwrap().hp, 3);
        s.apply(&Event::UnitsRemoved(vec![5]));
        assert!(s.units.get(5).is_none() && s.units.get(6).is_some());
        // Leaving the world clears the view.
        s.apply(&Event::LeftWorld { target: 1 });
        assert!(s.units.by_id.is_empty());
        // A move for an unknown unit changes nothing.
        assert!(!s.units.apply(&Event::UnitMoved {
            obj_id: 99,
            movement: Movement::unit(Pos::default(), 0.0, 1),
        }));
    }

    #[test]
    fn a_cancelled_delete_clears_the_timer() {
        let mut s = Characters::default();
        let pending = Character { id: 3, name: "A".into(), delete_request_time: 100, delete_time: 160, ..Default::default() };
        s.list.push(pending);
        assert!(s.by_id(3).unwrap().delete_requested());
        // status 4 (unknown character) restores nothing
        assert!(!s.apply(&Event::CharacterDeleteCancelled { character_id: 3, restored: false }));
        assert!(s.by_id(3).unwrap().delete_requested());
        assert!(s.apply(&Event::CharacterDeleteCancelled { character_id: 3, restored: true }));
        let c = s.by_id(3).unwrap();
        assert!(!c.delete_requested());
        assert_eq!(c.delete_time, 0);
        // a cancel for a character that is not listed changes nothing
        assert!(!s.apply(&Event::CharacterDeleteCancelled { character_id: 9, restored: true }));
    }

    #[test]
    fn worlds_characters_and_status() {
        let mut s = WorldState::default();
        s.apply(&Event::PhaseChanged(Phase::WorldSelect));
        assert_eq!(s.status.phase, Phase::WorldSelect);
        s.apply(&Event::CharacterList(vec![Character { id: 3, name: "A".into(), ..Default::default() }]));
        assert_eq!(s.characters.by_id(3).unwrap().name, "A");
        s.apply(&Event::LoginFailed { message: "bad".into() });
        assert_eq!(s.status.last_error.as_deref(), Some("bad"));
        s.apply(&Event::Disconnected { message: "gone".into() });
        assert!(s.characters.list.is_empty());
    }
}
