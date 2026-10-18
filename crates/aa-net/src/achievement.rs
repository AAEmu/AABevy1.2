//! Original achievement receipts and a bounded native cache; no reward requests.
use crate::{
    session::{Event, Phase},
    wire::{Reader, Result, WireError},
};
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

pub const NATIVE_ACHIEVEMENT_ROWS: usize = 50;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AchievementProgress {
    pub id: u32,
    /// Exact wire bits, interpreted as signed by native scalar amount operations.
    pub amount: i32,
    /// Raw signed 64-bit timestamp. Lane writes nonnegative Unix seconds; no conversion here.
    pub completed_at: i64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AchievementEvent {
    List { raw_count: i32, rows: Vec<AchievementProgress> },
    Changed { id: u32, amount: i32 },
    Completed { id: u32, completed_at: i64 },
    Reset { id: u32, amount: i32, completed_at: i64 },
    ItemSent { id: u32, by_mail: u8 },
}
impl AchievementEvent {
    pub fn handles(opcode: u16) -> bool {
        (0x1fc..=0x200).contains(&opcode)
    }
    /// Active lane framing consumes every declared row. Native inline storage admits only50.
    pub fn read_lane(opcode: u16, r: &mut Reader) -> Result<Self> {
        Ok(match opcode {
            0x1fc => {
                let raw_count = r.i32()?;
                let count = raw_count.max(0) as usize;
                if count > r.remaining() / 16 {
                    return Err(WireError::Invalid("achievement row count exceeds remaining payload".into()));
                }
                let rows = (0..count)
                    .map(|_| {
                        Ok(AchievementProgress {
                            id: r.u32()?,
                            amount: r.i32()?,
                            completed_at: r.i64()?,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                Self::List { raw_count, rows }
            }
            0x1fd => Self::Changed {
                id: r.u32()?,
                amount: r.i32()?,
            },
            0x1fe => Self::Completed {
                id: r.u32()?,
                completed_at: r.i64()?,
            },
            0x1ff => Self::Reset {
                id: r.u32()?,
                amount: r.i32()?,
                completed_at: r.i64()?,
            },
            0x200 => Self::ItemSent {
                id: r.u32()?,
                by_mail: r.u8()?,
            },
            _ => return Err(WireError::Invalid("unknown achievement opcode".into())),
        })
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AchievementRule {
    /// achievements.complete_num; native uses this only when strictly positive.
    pub complete_num: i32,
    /// Number of loaded achievement_objectives rows for this achievement.
    pub objective_count: u32,
}
impl AchievementRule {
    pub fn completed_amount(self) -> i32 {
        if self.complete_num > 0 {
            self.complete_num
        } else {
            self.objective_count as i32
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AchievementNotice {
    /// COMPLETE_ACHIEVEMENT(id), before Updated, once per admitted nonzero timestamp receipt.
    Completed(u32),
    /// ACHIEVEMENT_UPDATE(), after the cache write.
    Updated,
    /// ACHIEVEMENT_ITEM_SENT(localized descriptor name): root resolves the name from this ID.
    ItemSent(u32),
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "bevy", derive(bevy::ecs::resource::Resource))]
pub struct Achievements {
    pub entries: BTreeMap<u32, AchievementProgress>,
    /// Complete ordered receipts are forwarded through Event::Achievement, not accumulated here.
    pub last_receipt: Option<AchievementEvent>,
    /// Actual received feature byte7 bit7. Unknown never assumes the native absent-game exception.
    pub native_enabled: Option<bool>,
    catalog: Arc<HashMap<u32, AchievementRule>>,
}
impl Achievements {
    /// Static catalog survives lifecycle clears and is shared by snapshot clones. No retroactive fold.
    pub fn set_catalog(&mut self, catalog: HashMap<u32, AchievementRule>) {
        self.catalog = Arc::new(catalog);
    }
    pub fn get(&self, id: u32) -> Option<&AchievementProgress> {
        self.entries.get(&id)
    }
    pub fn apply(&mut self, event: &Event) -> bool {
        self.apply_with_notices(event).0
    }
    /// Use INSTEAD of apply when consuming Lua notices; never fold a receipt twice.
    pub fn apply_with_notices(&mut self, event: &Event) -> (bool, Vec<AchievementNotice>) {
        let mut notices = Vec::new();
        match event {
            Event::InitialConfig(config) => {
                let value = config.feature_bytes.get(7).map(|b| b & 0x80 != 0);
                let changed = value != self.native_enabled;
                self.native_enabled = value;
                return (changed, notices);
            }
            Event::CharacterSelected(_) => {
                let changed = !self.entries.is_empty() || self.last_receipt.is_some();
                self.entries.clear();
                self.last_receipt = None;
                return (changed, notices);
            }
            Event::LoginSucceeded(_) | Event::Disconnected { .. } | Event::Kicked { .. } | Event::PhaseChanged(Phase::Authenticating | Phase::Connecting) => {
                let changed = !self.entries.is_empty() || self.last_receipt.is_some() || self.native_enabled.is_some();
                self.entries.clear();
                self.last_receipt = None;
                self.native_enabled = None;
                return (changed, notices);
            }
            Event::Achievement(receipt) => {
                let mut changed = self.last_receipt.as_ref() != Some(receipt);
                self.last_receipt = Some(receipt.clone());
                match receipt {
                    AchievementEvent::List { raw_count, rows } => {
                        for row in rows.iter().take((*raw_count).max(0) as usize).take(NATIVE_ACHIEVEMENT_ROWS) {
                            if let std::collections::btree_map::Entry::Vacant(entry) = self.entries.entry(row.id) {
                                entry.insert(*row);
                                changed = true;
                            }
                        }
                    }
                    AchievementEvent::ItemSent { id, .. } => {
                        if self.catalog.contains_key(id) {
                            notices.push(AchievementNotice::ItemSent(*id));
                        }
                    }
                    AchievementEvent::Changed { id, amount } | AchievementEvent::Reset { id, amount, completed_at: 0 } => {
                        self.update(*id, *amount, 0, &mut changed, &mut notices)
                    }
                    AchievementEvent::Completed { id, completed_at } | AchievementEvent::Reset { id, completed_at, .. } => {
                        self.update(*id, 0, *completed_at, &mut changed, &mut notices)
                    }
                }
                (changed, notices)
            }
            _ => (false, notices),
        }
    }
    fn update(&mut self, id: u32, amount: i32, completed_at: i64, changed: &mut bool, notices: &mut Vec<AchievementNotice>) {
        if self.native_enabled != Some(true) {
            return;
        }
        let Some(rule) = self.catalog.get(&id) else { return };
        let row = AchievementProgress {
            id,
            amount: if completed_at != 0 { rule.completed_amount() } else { amount },
            completed_at,
        };
        if self.entries.get(&id) != Some(&row) {
            self.entries.insert(id, row);
            *changed = true;
        }
        if completed_at != 0 {
            notices.push(AchievementNotice::Completed(id));
        }
        notices.push(AchievementNotice::Updated);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::Writer;
    fn config(bytes: &[u8]) -> Event {
        Event::InitialConfig(crate::initial_config::InitialConfig {
            host: vec![],
            feature_bytes: bytes.to_vec(),
            raw_candidate_count: 0,
            candidate_codes: vec![],
            initial_lp: 0,
            service_flags: [0; 7],
            second_price_type: 0,
            second_password_max_fail_count: 0,
            idle_kick_time: 0,
        })
    }
    fn receipt(e: AchievementEvent) -> Event {
        Event::Achievement(e)
    }
    fn ready() -> Achievements {
        let mut s = Achievements::default();
        s.set_catalog(
            [
                (
                    7,
                    AchievementRule {
                        complete_num: 12,
                        objective_count: 99,
                    },
                ),
                (
                    8,
                    AchievementRule {
                        complete_num: -1,
                        objective_count: 3,
                    },
                ),
            ]
            .into(),
        );
        s.apply(&config(&[0, 0, 0, 0, 0, 0, 0, 128]));
        s
    }
    #[test]
    fn all_wire_layouts_raw_signed_extremes_and_every_truncation() {
        for timestamp in [i64::MIN, -1, 0, 1, 1_783_456_789, i64::MAX] {
            for amount in [i32::MIN, -1, 0, i32::MAX] {
                let mut list = Writer::new();
                list.i32(1).u32(u32::MAX).i32(amount).i64(timestamp);
                let mut changed = Writer::new();
                changed.u32(u32::MAX).i32(amount);
                let mut complete = Writer::new();
                complete.u32(u32::MAX).i64(timestamp);
                let mut reset = Writer::new();
                reset.u32(u32::MAX).i32(amount).i64(timestamp);
                for (op, body, expected) in [
                    (
                        0x1fc,
                        list.finish(),
                        AchievementEvent::List {
                            raw_count: 1,
                            rows: vec![AchievementProgress {
                                id: u32::MAX,
                                amount,
                                completed_at: timestamp,
                            }],
                        },
                    ),
                    (0x1fd, changed.finish(), AchievementEvent::Changed { id: u32::MAX, amount }),
                    (
                        0x1fe,
                        complete.finish(),
                        AchievementEvent::Completed {
                            id: u32::MAX,
                            completed_at: timestamp,
                        },
                    ),
                    (
                        0x1ff,
                        reset.finish(),
                        AchievementEvent::Reset {
                            id: u32::MAX,
                            amount,
                            completed_at: timestamp,
                        },
                    ),
                ] {
                    let mut r = Reader::new(&body);
                    assert_eq!(AchievementEvent::read_lane(op, &mut r).unwrap(), expected);
                    assert!(r.is_empty());
                    for n in 0..body.len() {
                        assert!(AchievementEvent::read_lane(op, &mut Reader::new(&body[..n])).is_err(), "{op:x}/{n}");
                    }
                }
            }
        }
        for by_mail in [0, 1, 2, 255] {
            let body = [7, 0, 0, 0, by_mail];
            let mut r = Reader::new(&body);
            assert_eq!(
                AchievementEvent::read_lane(0x200, &mut r).unwrap(),
                AchievementEvent::ItemSent { id: 7, by_mail }
            );
            for n in 0..body.len() {
                assert!(AchievementEvent::read_lane(0x200, &mut Reader::new(&body[..n])).is_err());
            }
        }
        assert!(AchievementEvent::read_lane(0x201, &mut Reader::new(&[])).is_err());
        for count in [i32::MIN, -1, 0] {
            let raw = count.to_le_bytes();
            assert_eq!(
                AchievementEvent::read_lane(0x1fc, &mut Reader::new(&raw)).unwrap(),
                AchievementEvent::List {
                    raw_count: count,
                    rows: vec![]
                }
            );
        }
        assert!(AchievementEvent::read_lane(0x1fc, &mut Reader::new(&i32::MAX.to_le_bytes())).is_err());
    }
    #[test]
    fn lane_more_than_fifty_preserves_following_dd04_packet_and_native_first_wins_cache() {
        use std::io::Write;
        let mut body = Writer::new();
        body.i32(52);
        let mut rows = vec![];
        for i in 0..52 {
            let row = AchievementProgress {
                id: if i == 1 { 0 } else { i },
                amount: i as i32,
                completed_at: -(i as i64),
            };
            body.u32(row.id).i32(row.amount).i64(row.completed_at);
            rows.push(row);
        }
        let body = body.finish();
        let mut raw = Writer::new();
        raw.u16(0).u16(0x1fc).raw(&body).u16(0).u16(0x200).u32(7).u8(255);
        let mut compressor = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
        compressor.write_all(&raw.finish()).unwrap();
        let mut packet = Writer::new();
        packet.u8(0xdd).u8(crate::game::LEVEL_COMPRESSED).u16(2).raw(&compressor.finish().unwrap());
        let crate::game::ServerMsg::Batch(messages) = crate::game::ServerMsg::decode(&packet.finish()).unwrap() else {
            panic!()
        };
        assert_eq!(messages.len(), 2);
        let crate::game::ServerMsg::Achievement(list) = &messages[0] else { panic!() };
        assert_eq!(
            list,
            &AchievementEvent::List {
                raw_count: 52,
                rows: rows.clone()
            }
        );
        assert!(matches!(
            messages[1],
            crate::game::ServerMsg::Achievement(AchievementEvent::ItemSent { id: 7, by_mail: 255 })
        ));
        let mut s = Achievements::default();
        assert!(s.apply_with_notices(&receipt(list.clone())).1.is_empty());
        assert_eq!(s.entries.len(), 49);
        assert_eq!(s.entries[&0].amount, 0);
        assert!(!s.entries.contains_key(&50));
        s.apply(&receipt(AchievementEvent::List {
            raw_count: 1,
            rows: vec![AchievementProgress {
                id: 0,
                amount: 99,
                completed_at: 9,
            }],
        }));
        assert_eq!(s.entries[&0].amount, 0);
        s.apply(&receipt(AchievementEvent::List { raw_count: 0, rows: vec![] }));
        assert_eq!(s.entries.len(), 49);
    }
    #[test]
    fn scalar_catalog_gate_completion_normalization_and_repeated_notice_order() {
        let mut s = ready();
        for (id, expected) in [(7, 12), (8, 3)] {
            for timestamp in [-1, 1, i64::MIN, i64::MAX] {
                let e = receipt(AchievementEvent::Reset {
                    id,
                    amount: -500,
                    completed_at: timestamp,
                });
                for _ in 0..2 {
                    let (_, n) = s.apply_with_notices(&e);
                    assert_eq!(n, [AchievementNotice::Completed(id), AchievementNotice::Updated]);
                    assert_eq!(
                        s.entries[&id],
                        AchievementProgress {
                            id,
                            amount: expected,
                            completed_at: timestamp
                        }
                    );
                }
            }
        }
        let (_, n) = s.apply_with_notices(&receipt(AchievementEvent::Changed { id: 7, amount: i32::MIN }));
        assert_eq!(n, [AchievementNotice::Updated]);
        assert_eq!(s.entries[&7].completed_at, 0);
        assert_eq!(s.entries[&7].amount, i32::MIN);
        let (_, n) = s.apply_with_notices(&receipt(AchievementEvent::Completed { id: 7, completed_at: 0 }));
        assert_eq!(n, [AchievementNotice::Updated]);
        assert_eq!(s.entries[&7].amount, 0);
        let (_, n) = s.apply_with_notices(&receipt(AchievementEvent::ItemSent { id: 7, by_mail: 255 }));
        assert_eq!(n, [AchievementNotice::ItemSent(7)]);
        assert_eq!(s.entries[&7].amount, 0);
        for mode in 0..4 {
            let mut s = ready();
            match mode {
                0 => s.native_enabled = None,
                1 => {
                    s.apply(&config(&[0; 8]));
                }
                2 => {
                    s.apply(&config(&[0; 7]));
                }
                _ => s.set_catalog(HashMap::new()),
            };
            let e = AchievementEvent::Completed { id: 7, completed_at: 1 };
            assert!(s.apply_with_notices(&receipt(e.clone())).1.is_empty());
            assert!(s.entries.is_empty());
            assert_eq!(s.last_receipt, Some(e));
        }
        assert!(s.apply_with_notices(&receipt(AchievementEvent::Changed { id: 999, amount: 2 })).1.is_empty());
        assert!(!s.entries.contains_key(&999));
    }
    #[test]
    fn static_catalog_shared_across_clone_and_lifecycle_runtime_clear() {
        let mut s = ready();
        let clone = s.clone();
        assert!(Arc::ptr_eq(&s.catalog, &clone.catalog));
        s.apply(&receipt(AchievementEvent::Completed { id: 7, completed_at: 1 }));
        for event in [
            Event::Disconnected { message: String::new() },
            Event::Kicked {
                reason: 1,
                message: String::new(),
            },
            Event::PhaseChanged(Phase::Connecting),
            Event::PhaseChanged(Phase::Authenticating),
        ] {
            let mut copy = s.clone();
            assert!(copy.apply(&event));
            assert!(copy.entries.is_empty() && copy.last_receipt.is_none());
            assert_eq!(copy.native_enabled, None);
            assert!(Arc::ptr_eq(&s.catalog, &copy.catalog));
        }
        let mut copy = s.clone();
        let (changed, notices) = copy.apply_with_notices(&Event::CharacterSelected(Box::new(crate::model::CharacterState::default())));
        assert!(changed && notices.is_empty() && copy.entries.is_empty() && copy.last_receipt.is_none());
        assert_eq!(copy.native_enabled, Some(true));
        assert!(Arc::ptr_eq(&s.catalog, &copy.catalog));
        // This app isolation boundary is stronger than native global-manager lifetime; no synthetic update notice.
        let mut world = crate::WorldState::default();
        world.achievements = s;
        world.apply(&Event::Disconnected { message: String::new() });
        assert!(world.achievements.entries.is_empty());
    }
}
