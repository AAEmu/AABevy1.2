//! Persistent character names received on the stream channel. No expiry or automatic queries.
use crate::session::{Event, Phase};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CharacterName {
    pub character_id: u32,
    pub name: Vec<u8>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "bevy", derive(bevy::ecs::resource::Resource))]
pub struct CharacterNames {
    pub by_id: BTreeMap<u32, Vec<u8>>,
    pub pending: BTreeSet<u32>,
    pub last_receipt: Option<CharacterName>,
}
impl CharacterNames {
    pub fn apply(&mut self, e: &Event) -> bool {
        match e {
            Event::CharacterName(row) => {
                let mut changed = self.last_receipt.as_ref() != Some(row);
                self.last_receipt = Some(row.clone());
                // Native buffer is 129 bytes including terminator. Unsafe oversized receipts stay raw only.
                if row.name.len() <= 128 {
                    changed |= self.pending.remove(&row.character_id);
                    let end = row.name.iter().position(|&b| b == 0).unwrap_or(row.name.len());
                    let name = row.name[..end].to_vec();
                    changed |= self.by_id.get(&row.character_id) != Some(&name);
                    self.by_id.insert(row.character_id, name);
                }
                changed
            }
            Event::CharacterNameRequested { requested_id, .. } => self.pending.insert(*requested_id),
            Event::CharacterNameRequestRejected { .. } => false,
            Event::CharacterNameStreamClosed => {
                let changed = !self.pending.is_empty();
                self.pending.clear();
                changed
            }
            Event::LoginSucceeded(_) | Event::Disconnected { .. } | Event::Kicked { .. } | Event::PhaseChanged(Phase::Connecting | Phase::Authenticating) => {
                let changed = *self != Self::default();
                *self = Self::default();
                changed
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn character_names_raw_duplicates_caps_and_transport_lifetime() {
        let mut s = CharacterNames::default();
        let request = Event::CharacterNameRequested {
            character_id: 42,
            requested_id: 7,
        };
        assert!(s.apply(&request));
        assert!(!s.apply(&request));
        for name in [vec![255, 0, b'X'], vec![b'A'; 128], Vec::new()] {
            let row = CharacterName {
                character_id: 7,
                name: name.clone(),
            };
            assert!(s.apply(&Event::CharacterName(row.clone())));
            assert!(!s.pending.contains(&7));
            assert_eq!(s.last_receipt, Some(row));
            assert_eq!(s.by_id[&7], name.split(|&b| b == 0).next().unwrap());
            assert!(!s.apply(&Event::CharacterName(s.last_receipt.clone().unwrap())));
        }
        s.apply(&request);
        let before = s.by_id.clone();
        let bad = CharacterName {
            character_id: 7,
            name: vec![b'A'; 129],
        };
        assert!(s.apply(&Event::CharacterName(bad.clone())));
        assert_eq!(s.last_receipt, Some(bad));
        assert_eq!(s.by_id, before);
        assert!(s.pending.contains(&7));
        s.apply(&Event::CharacterNameStreamClosed);
        assert!(s.pending.is_empty());
        assert_eq!(s.by_id, before);
        assert!(!s.apply(&Event::LeftWorld { target: 0 }));
        assert_eq!(s.by_id, before);
        assert!(s.apply(&Event::Disconnected { message: String::new() }));
        assert_eq!(s, CharacterNames::default());
    }
}
