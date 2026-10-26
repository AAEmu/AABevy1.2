//! World level state around the player: the zone, the game clock, weather, the chat channels and
//! log, and the selected target.

use std::collections::VecDeque;
use web_time::Instant;

use crate::model::{chat_type, ChatMessage, Unit};
use crate::session::Event;
use crate::state::Units;

/// Hours of the 24 hour game clock, with the speed it advances.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimeOfDay {
    /// The hour (0..24) when the packet arrived.
    pub hour: f32,
    /// Game hours per real second (the server's clock runs 6 game seconds per real second,
    /// 0.001666 hours; a day is 4 real hours).
    pub speed: f32,
    pub received: Instant,
}

impl TimeOfDay {
    /// The server's clock speed, used until a packet states another.
    pub const DEFAULT_SPEED: f32 = 0.001_666_6;

    /// The hour now, advanced from when the packet arrived.
    pub fn hour_now(&self) -> f32 {
        (self.hour + self.speed * self.received.elapsed().as_secs_f32()).rem_euclid(24.0)
    }
}

resource! {
    /// Where the player is and what the sky does.
    ///
    /// The player's current zone group follows chat channels: `zone_group_id` follows the zone
    /// ("shout") channel's `sub_type`. The sub zone ("Desiree Peak" on the minimap header) is not
    /// sent at all: the client finds it from the position in the `sub_zones` table
    /// (`linked_zone_group_id`, `x`, `y`, `w`, `h`).
    pub struct WorldInfo {
        /// The zone key of the character state (`zones.zone_key`); `zones.group_id` is the zone
        /// group (`zone_groups.display_text` / `name`).
        pub zone_key: u32,
        /// The current zone group, from the zone chat channel (None until it is joined).
        pub zone_group_id: Option<u32>,
        pub instance_id: u32,
        pub time: Option<TimeOfDay>,
        pub snow: bool,
    }
}

impl WorldInfo {
    /// The game hour now (0..24), if the server has sent the clock.
    pub fn hour_now(&self) -> Option<f32> {
        self.time.map(|t| t.hour_now())
    }

    pub fn apply(&mut self, e: &Event) -> bool {
        match e {
            Event::CharacterSelected(cs) => {
                *self = Self {
                    zone_key: cs.character.zone_id,
                    instance_id: cs.instance_id,
                    ..Self::default()
                };
            }
            Event::TimeOfDay { hour, speed } => {
                let speed = speed.or(self.time.map(|t| t.speed)).unwrap_or(TimeOfDay::DEFAULT_SPEED);
                self.time = Some(TimeOfDay { hour: *hour, speed, received: Instant::now() });
            }
            Event::Snow(on) => self.snow = *on,
            Event::ChatChannelJoined { chat_type: chat_type::SHOUT, sub_type, .. } => {
                self.zone_group_id = Some(*sub_type as u32);
            }
            Event::ChatChannelLeft { chat_type: chat_type::SHOUT, sub_type, .. }
                if self.zone_group_id == Some(*sub_type as u32) =>
            {
                self.zone_group_id = None;
            }
            Event::LeftWorld { .. } | Event::Disconnected { .. } => {
                if *self == Self::default() {
                    return false;
                }
                *self = Self::default();
            }
            _ => return false,
        }
        true
    }
}

impl PartialEq for WorldInfo {
    fn eq(&self, o: &Self) -> bool {
        self.zone_key == o.zone_key
            && self.zone_group_id == o.zone_group_id
            && self.instance_id == o.instance_id
            && self.time == o.time
            && self.snow == o.snow
    }
}

/// A chat channel the player is in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatChannel {
    pub chat_type: i16,
    /// The zone group id for zone channels.
    pub sub_type: i16,
    pub faction_id: u32,
    /// The name (user channels), empty for the server's channels.
    pub name: String,
}

/// How many chat lines [`Chat::log`] keeps.
pub const CHAT_LOG_LIMIT: usize = 500;

resource! {
    /// Joined chat channels and the most recent chat lines (all types: say, whispers, system
    /// text, channel messages), oldest first.
    pub struct Chat {
        pub channels: Vec<ChatChannel>,
        pub log: VecDeque<ChatMessage>,
        /// Latest received handshake configuration; no local throttling is inferred from it.
        pub spam_delay: Option<crate::ChatSpamDelay>,
    }
}

impl Chat {
    pub fn apply(&mut self, e: &Event) -> bool {
        match e {
            Event::ChatSpamDelay(config) => {
                if self.spam_delay.as_ref() == Some(config) {
                    return false;
                }
                self.spam_delay = Some(config.clone());
            }
            Event::ChatChannelJoined { chat_type, sub_type, faction_id, name } => {
                let ch = ChatChannel {
                    chat_type: *chat_type,
                    sub_type: *sub_type,
                    faction_id: *faction_id,
                    name: name.clone(),
                };
                if self.channels.contains(&ch) {
                    return false;
                }
                self.channels.push(ch);
            }
            Event::ChatChannelLeft { chat_type, sub_type, faction_id } => {
                let before = self.channels.len();
                self.channels.retain(|c| !(c.chat_type == *chat_type && c.sub_type == *sub_type && c.faction_id == *faction_id));
                return self.channels.len() != before;
            }
            Event::Chat(m) => {
                if self.log.len() >= CHAT_LOG_LIMIT {
                    self.log.pop_front();
                }
                self.log.push_back(m.clone());
            }
            Event::LeftWorld { .. } => {
                if self.channels.is_empty() && self.spam_delay.is_none() {
                    return false;
                }
                self.channels.clear();
                self.spam_delay = None;
            }
            Event::Disconnected { .. } => {
                if self.channels.is_empty() && self.log.is_empty() && self.spam_delay.is_none() {
                    return false;
                }
                self.channels.clear();
                self.log.clear();
                self.spam_delay = None;
            }
            _ => return false,
        }
        true
    }
}

resource! {
    /// The local player's target.
    pub struct Target {
        pub obj_id: Option<u32>,
        own_obj_id: u32,
    }
}

impl Target {
    /// The targeted unit, if it is in view.
    pub fn unit<'a>(&self, units: &'a Units) -> Option<&'a Unit> {
        units.get(self.obj_id?)
    }

    pub fn apply(&mut self, e: &Event) -> bool {
        match e {
            Event::PlayerSpawn(s) => self.own_obj_id = s.obj_id,
            Event::TargetChanged { obj_id, target_obj_id } if self.own_obj_id != 0 && *obj_id == self.own_obj_id => {
                let new = (*target_obj_id != 0).then_some(*target_obj_id);
                if new == self.obj_id {
                    return false;
                }
                self.obj_id = new;
            }
            Event::UnitsRemoved(ids) => {
                if !self.obj_id.is_some_and(|t| ids.contains(&t)) {
                    return false;
                }
                self.obj_id = None;
            }
            Event::LeftWorld { .. } | Event::Disconnected { .. } => {
                if self.obj_id.is_none() && self.own_obj_id == 0 {
                    return false;
                }
                *self = Self::default();
            }
            _ => return false,
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Character, CharacterState};

    #[test]
    fn the_clock_runs_from_the_packets() {
        let mut w = WorldInfo::default();
        assert!(w.hour_now().is_none());
        assert!(w.apply(&Event::TimeOfDay { hour: 12.0, speed: Some(0.5) }));
        let t = w.time.unwrap();
        assert_eq!((t.hour, t.speed), (12.0, 0.5));
        // A plain time keeps the speed.
        assert!(w.apply(&Event::TimeOfDay { hour: 23.9, speed: None }));
        assert_eq!(w.time.unwrap().speed, 0.5);
        assert!(w.hour_now().unwrap() >= 23.9 || w.hour_now().unwrap() < 0.5);
        // Before any detailed packet the server's default speed applies.
        let mut w = WorldInfo::default();
        w.apply(&Event::TimeOfDay { hour: 6.0, speed: None });
        assert_eq!(w.time.unwrap().speed, TimeOfDay::DEFAULT_SPEED);
        assert!(w.apply(&Event::Snow(true)) && w.snow);
    }

    #[test]
    fn the_zone_follows_the_shout_channel() {
        let mut w = WorldInfo::default();
        let sel = Event::CharacterSelected(Box::new(CharacterState {
            character: Character { zone_id: 179, ..Character::default() },
            instance_id: 3,
            ..CharacterState::default()
        }));
        assert!(w.apply(&sel));
        assert_eq!((w.zone_key, w.instance_id, w.zone_group_id), (179, 3, None));
        // Trade (2) does not name the zone, Shout (1) does.
        assert!(!w.apply(&Event::ChatChannelJoined { chat_type: 2, sub_type: 5, faction_id: 0, name: String::new() }));
        assert!(w.apply(&Event::ChatChannelJoined { chat_type: 1, sub_type: 5, faction_id: 0, name: String::new() }));
        assert_eq!(w.zone_group_id, Some(5));
        // Walking into zone 24: the old channel is left, the new one joined.
        assert!(w.apply(&Event::ChatChannelJoined { chat_type: 1, sub_type: 24, faction_id: 0, name: String::new() }));
        assert!(!w.apply(&Event::ChatChannelLeft { chat_type: 1, sub_type: 5, faction_id: 0 }));
        assert_eq!(w.zone_group_id, Some(24));
    }

    #[test]
    fn chat_channels_and_log() {
        let mut c = Chat::default();
        let join = Event::ChatChannelJoined { chat_type: 1, sub_type: 5, faction_id: 0, name: String::new() };
        assert!(c.apply(&join));
        assert!(!c.apply(&join), "joining twice changes nothing");
        assert!(c.apply(&Event::ChatChannelJoined { chat_type: 15, sub_type: 0, faction_id: 0, name: "Guild".into() }));
        assert_eq!(c.channels.len(), 2);
        assert!(c.apply(&Event::ChatChannelLeft { chat_type: 1, sub_type: 5, faction_id: 0 }));
        assert_eq!(c.channels.len(), 1);
        let msg = |text: &str| ChatMessage {
            chat_type: chat_type::SYSTEM,
            sub_type: 0,
            channel_faction: 0,
            sender_obj_id: 0,
            sender_id: 0,
            sender_race: 0,
            faction_id: 0,
            sender: String::new(),
            message: text.into(),
            ability: 0,
        };
        for i in 0..CHAT_LOG_LIMIT + 3 {
            c.apply(&Event::Chat(msg(&i.to_string())));
        }
        assert_eq!(c.log.len(), CHAT_LOG_LIMIT);
        assert_eq!(c.log.front().unwrap().message, "3");
        assert!(c.apply(&Event::Disconnected { message: String::new() }));
        assert!(c.log.is_empty() && c.channels.is_empty());
    }

    #[test]
    fn the_target_follows_the_player_only() {
        let mut t = Target::default();
        let mut units = Units::default();
        units.apply(&Event::UnitSpawned(Box::new(crate::state::tests::unit(7))));
        // Before the spawn nothing is the player's.
        assert!(!t.apply(&Event::TargetChanged { obj_id: 1, target_obj_id: 7 }));
        t.own_obj_id = 1;
        assert!(!t.apply(&Event::TargetChanged { obj_id: 2, target_obj_id: 7 }));
        assert!(t.apply(&Event::TargetChanged { obj_id: 1, target_obj_id: 7 }));
        assert_eq!(t.obj_id, Some(7));
        assert_eq!(t.unit(&units).unwrap().obj_id, 7);
        assert!(t.apply(&Event::UnitsRemoved(vec![7])));
        assert_eq!(t.obj_id, None);
        assert!(t.apply(&Event::TargetChanged { obj_id: 1, target_obj_id: 9 }));
        assert!(t.apply(&Event::TargetChanged { obj_id: 1, target_obj_id: 0 }));
        assert_eq!(t.obj_id, None);
    }
}
