//! Received bounty bulletin and settlement reports. No payment or reward optimism.
use crate::{
    session::{Event, Phase},
    wire::{Reader, Result, WireError},
};
mod commands;
pub use commands::BountyCommand;
use std::collections::BTreeMap;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BountyRow {
    pub player_id: u32,
    pub name: Vec<u8>,
    pub level: u8,
    pub zone_id: i32,
    pub x: i64,
    pub y: i64,
    pub z_bits: u32,
    pub money: u64,
    pub crime_points: i16,
    pub raw_is_live: u8,
    pub race: u8,
    pub faction_id: u32,
    pub arrest_count: i32,
    pub accept_guilty_count: i32,
    pub accept_trial_count: i32,
    /// First verdict counter on the wire. Native calls it not-guilty; lane writer calls it guilty.
    pub first_verdict_count: i32,
    /// Second verdict counter on the wire. Native calls it guilty; lane writer calls it not-guilty.
    pub second_verdict_count: i32,
    pub seconds: i32,
}
impl BountyRow {
    fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            player_id: r.u32()?,
            name: r.blob()?.to_vec(),
            level: r.u8()?,
            zone_id: r.i32()?,
            x: r.i64()?,
            y: r.i64()?,
            z_bits: r.u32()?,
            money: r.u64()?,
            crime_points: r.i16()?,
            raw_is_live: r.u8()?,
            race: r.u8()?,
            faction_id: r.u32()?,
            arrest_count: r.i32()?,
            accept_guilty_count: r.i32()?,
            accept_trial_count: r.i32()?,
            first_verdict_count: r.i32()?,
            second_verdict_count: r.i32()?,
            seconds: r.i32()?,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BountyEvent {
    UnitMoney { obj_id: u32, money: i64 },
    List { total: u32, declared_count: u8, rows: Vec<BountyRow> },
    SetPermitted { kind: u32, name: Vec<u8>, money: i64, raw_allowed: u8 },
    SetDone { kind: u32, name: Vec<u8>, money: i64, error: i16 },
    Paid { hunter: Vec<u8>, arrested: Vec<u8>, money: i64 },
}
impl BountyEvent {
    pub fn handles(op: u16) -> bool {
        matches!(op, 0xbb | 0x1d9 | 0x1da | 0x1db | 0x1dc)
    }
    pub fn read(op: u16, r: &mut Reader) -> Result<Self> {
        Ok(match op {
            0xbb => Self::UnitMoney {
                obj_id: r.bc()?,
                money: r.i64()?,
            },
            0x1d9 => {
                let total = r.u32()?;
                let declared_count = r.u8()?;
                let mut rows = Vec::with_capacity(declared_count as usize);
                for _ in 0..declared_count {
                    rows.push(BountyRow::read(r)?);
                }
                Self::List { total, declared_count, rows }
            }
            0x1da => Self::SetPermitted {
                kind: r.u32()?,
                name: r.blob()?.to_vec(),
                money: r.i64()?,
                raw_allowed: r.u8()?,
            },
            0x1db => Self::SetDone {
                kind: r.u32()?,
                name: r.blob()?.to_vec(),
                money: r.i64()?,
                error: r.i16()?,
            },
            0x1dc => Self::Paid {
                hunter: r.blob()?.to_vec(),
                arrested: r.blob()?.to_vec(),
                money: r.i64()?,
            },
            _ => return Err(WireError::Invalid("unknown bounty opcode".into())),
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BountyNotice {
    /// TOGGLE_BOUNTY_BULLETIN(), after complete chunk assignment, including duplicate receipts.
    ToggleBulletin,
    /// SET_BOUNTY(kind,name,signed-low32-money-as-f32).
    Set { kind: u32, name: Vec<u8>, money: i64 },
    /// SET_BOUNTY_DONE(kind,name,signed-low32-money-as-f32,1).
    Done { kind: u32, name: Vec<u8>, money: i64 },
    /// BOUNTY_PAID(hunter,arrested,signed-low32-money-as-f32,1).
    Paid { hunter: Vec<u8>, arrested: Vec<u8>, money: i64 },
    /// SET_BOUNTY_DONE(runtimeID,actorName,money,localNumericFlag), only after existing actor assignment.
    UnitMoney { obj_id: u32, name: String, money: i64, is_local: bool },
    /// Native local error path, not a successful settlement.
    Error(i32),
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "bevy", derive(bevy::ecs::resource::Resource))]
pub struct Bounties {
    /// Native ordered unsigned persistent-ID cache. Last duplicate row wins.
    pub by_id: BTreeMap<u32, BountyRow>,
    /// Full latest raw receipt; forwarded Events remain the complete ordered receipt channel.
    pub last_receipt: Option<BountyEvent>,
}
impl Bounties {
    pub fn apply(&mut self, event: &Event) -> bool {
        self.apply_with_notices(event).0
    }
    /// Fold once, then dispatch returned notices; no intermediate map callbacks occur in these handlers.
    pub fn apply_with_notices(&mut self, event: &Event) -> (bool, Vec<BountyNotice>) {
        let mut notices = Vec::new();
        match event {
            Event::LoginSucceeded(_) | Event::Disconnected { .. } | Event::Kicked { .. } | Event::PhaseChanged(Phase::Authenticating | Phase::Connecting) => {
                let changed = !self.by_id.is_empty() || self.last_receipt.is_some();
                *self = Self::default();
                return (changed, notices);
            }
            Event::Bounty(receipt) => {
                let mut changed = self.last_receipt.as_ref() != Some(receipt);
                self.last_receipt = Some(receipt.clone());
                match receipt {
                    BountyEvent::List { total, declared_count, rows } => {
                        // Native inline reader has only twenty rows but callback trusts count. Refuse its unsafe domain.
                        if *declared_count <= 20 && rows.len() == *declared_count as usize && rows.iter().all(|r| r.name.len() <= 128) {
                            for row in rows {
                                changed |= self.by_id.get(&row.player_id) != Some(row);
                                self.by_id.insert(row.player_id, row.clone());
                            }
                            if *total as usize == self.by_id.len() {
                                notices.push(BountyNotice::ToggleBulletin);
                            }
                        }
                    }
                    BountyEvent::SetPermitted {
                        kind,
                        name,
                        money,
                        raw_allowed,
                    } if name.len() <= 128 => {
                        if *raw_allowed == 0 {
                            notices.push(BountyNotice::Error(584))
                        } else {
                            notices.push(BountyNotice::Set {
                                kind: *kind,
                                name: name.clone(),
                                money: *money,
                            })
                        }
                    }
                    BountyEvent::SetDone { kind, name, money, error } if name.len() <= 128 => {
                        if *error != 0 {
                            notices.push(BountyNotice::Error(*error as i32))
                        } else {
                            notices.push(BountyNotice::Done {
                                kind: *kind,
                                name: name.clone(),
                                money: *money,
                            })
                        }
                    }
                    BountyEvent::Paid { hunter, arrested, money } if hunter.len() <= 128 && arrested.len() <= 128 => notices.push(BountyNotice::Paid {
                        hunter: hunter.clone(),
                        arrested: arrested.clone(),
                        money: *money,
                    }),
                    _ => {}
                }
                return (changed, notices);
            }
            _ => {}
        }
        (false, notices)
    }
    /// Build the shared14f callback only from the actual matching existing actor.
    pub fn unit_money_notice(receipt: &BountyEvent, unit: &crate::model::Unit, local_obj_id: Option<u32>) -> Option<BountyNotice> {
        let BountyEvent::UnitMoney { obj_id, money } = receipt else { return None };
        (unit.obj_id == *obj_id && unit.bounty_money == Some(*money)).then(|| BountyNotice::UnitMoney {
            obj_id: *obj_id,
            name: unit.name.clone(),
            money: *money,
            is_local: local_obj_id == Some(*obj_id),
        })
    }
    /// Native display conversion deliberately ignores the high half, including for negative amounts.
    pub fn displayed_money(money: i64) -> f32 {
        money as i32 as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{game::ServerMsg, wire::Writer};
    fn row(id: u32) -> BountyRow {
        BountyRow {
            player_id: id,
            name: b"N\0tail".to_vec(),
            level: 55,
            zone_id: -1,
            x: i64::MIN,
            y: i64::MAX,
            z_bits: 0x7fc01234,
            money: u64::MAX,
            crime_points: i16::MIN,
            raw_is_live: 255,
            race: 8,
            faction_id: 100,
            arrest_count: -1,
            accept_guilty_count: 2,
            accept_trial_count: 3,
            first_verdict_count: 4,
            second_verdict_count: 5,
            seconds: i32::MIN,
        }
    }
    fn write_row(w: &mut Writer, r: &BountyRow) {
        w.u32(r.player_id)
            .blob(&r.name)
            .u8(r.level)
            .i32(r.zone_id)
            .i64(r.x)
            .i64(r.y)
            .u32(r.z_bits)
            .u64(r.money)
            .i16(r.crime_points)
            .u8(r.raw_is_live)
            .u8(r.race)
            .u32(r.faction_id)
            .i32(r.arrest_count)
            .i32(r.accept_guilty_count)
            .i32(r.accept_trial_count)
            .i32(r.first_verdict_count)
            .i32(r.second_verdict_count)
            .i32(r.seconds);
    }
    #[test]
    fn wire_all_fields_truncation_and_dd04_alignment() {
        let mut list = Writer::new();
        list.u32(1).u8(1);
        write_row(&mut list, &row(u32::MAX));
        let cases = [
            (0x1d9, list.finish()),
            (0xbb, vec![0x56, 0x34, 0x12, 0, 0, 0, 0, 0, 0, 0, 0x80]),
            (0x1da, vec![1, 0, 0, 0, 1, 0, b'N', 1, 0, 0, 0, 0, 0, 0, 0, 1]),
            (0x1db, vec![1, 0, 0, 0, 1, 0, b'N', 1, 0, 0, 0, 0, 0, 0, 0, 0x48, 2]),
            (0x1dc, vec![1, 0, b'H', 1, 0, b'A', 1, 0, 0, 0, 0, 0, 0, 0]),
        ];
        let mut batch = Writer::new();
        for (op, body) in &cases {
            let mut r = Reader::new(body);
            let got = BountyEvent::read(*op, &mut r).unwrap();
            assert!(r.is_empty());
            if let BountyEvent::List { rows, .. } = got {
                assert_eq!(rows, vec![row(u32::MAX)]);
            }
            for n in 0..body.len() {
                assert!(BountyEvent::read(*op, &mut Reader::new(&body[..n])).is_err(), "{op:x}/{n}");
            }
            batch.u16(0).u16(*op).raw(body);
        }
        batch.u16(0).u16(0x1bc).u32(17);
        use std::io::Write;
        let mut z = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(&batch.finish()).unwrap();
        let mut p = Writer::new();
        p.u8(0xdd).u8(crate::game::LEVEL_COMPRESSED).u16(4).raw(&z.finish().unwrap());
        let ServerMsg::Batch(events) = ServerMsg::decode(&p.finish()).unwrap() else {
            panic!()
        };
        assert_eq!(events.len(), 6);
        assert!(matches!(events[5], ServerMsg::SetBreath { breath: 17 }));
        for e in &events[..5] {
            assert!(matches!(e, ServerMsg::Bounty(_)));
        }
    }
    #[test]
    fn native_map_last_duplicate_order_completion_retention_and_transport_isolation() {
        let mut s = Bounties::default();
        let mut newer = row(7);
        newer.seconds = 99;
        let event = Event::Bounty(BountyEvent::List {
            total: 2,
            declared_count: 3,
            rows: vec![row(u32::MAX), row(7), newer.clone()],
        });
        assert_eq!(s.apply_with_notices(&event).1, vec![BountyNotice::ToggleBulletin]);
        assert_eq!(s.by_id.keys().copied().collect::<Vec<_>>(), vec![7, u32::MAX]);
        assert_eq!(s.by_id[&7], newer);
        assert_eq!(s.apply_with_notices(&event).1, vec![BountyNotice::ToggleBulletin]);
        s.apply(&Event::CharacterSelected(Box::default()));
        s.apply(&Event::LeftWorld { target: 0 });
        assert_eq!(s.by_id.len(), 2);
        let invalid = Event::Bounty(BountyEvent::List {
            total: 21,
            declared_count: 21,
            rows: vec![row(9); 21],
        });
        assert!(s.apply_with_notices(&invalid).1.is_empty());
        assert!(!s.by_id.contains_key(&9));
        assert_eq!(s.last_receipt, if let Event::Bounty(e) = invalid { Some(e) } else { None });
        s.apply(&Event::PhaseChanged(Phase::Connecting));
        assert_eq!(s, Bounties::default());
    }
    #[test]
    fn errors_duplicate_notifications_and_executed_low32_display_oracle() {
        let mut s = Bounties::default();
        let good = Event::Bounty(BountyEvent::SetDone {
            kind: 1,
            name: b"N\0tail".to_vec(),
            money: 0x100000001,
            error: 0,
        });
        for _ in 0..2 {
            assert_eq!(
                s.apply_with_notices(&good).1,
                vec![BountyNotice::Done {
                    kind: 1,
                    name: b"N\0tail".to_vec(),
                    money: 0x100000001
                }]
            );
        }
        assert_eq!(
            s.apply_with_notices(&Event::Bounty(BountyEvent::SetPermitted {
                kind: 1,
                name: vec![],
                money: 0,
                raw_allowed: 0
            }))
            .1,
            vec![BountyNotice::Error(584)]
        );
        assert_eq!(
            s.apply_with_notices(&Event::Bounty(BountyEvent::SetDone {
                kind: 1,
                name: vec![],
                money: 0,
                error: -1
            }))
            .1,
            vec![BountyNotice::Error(-1)]
        );
        assert_eq!(Bounties::displayed_money(0x100000001), 1.0);
        assert_eq!(Bounties::displayed_money(0x80000000), -2147483648.0);
    }
    #[test]
    fn actor_money_only_existing_matching_identity_before_callback() {
        let mut world = crate::state::WorldState::default();
        let receipt = Event::Bounty(BountyEvent::UnitMoney { obj_id: 7, money: i64::MIN });
        world.apply(&receipt);
        assert!(world.units.by_id.is_empty());
        world.apply(&Event::UnitSpawned(Box::new(crate::state::tests::unit(7))));
        world.apply(&receipt);
        let unit = &world.units.by_id[&7];
        assert_eq!(unit.bounty_money, Some(i64::MIN));
        let Event::Bounty(raw) = &receipt else { unreachable!() };
        assert!(matches!(
            Bounties::unit_money_notice(raw, unit, Some(7)),
            Some(BountyNotice::UnitMoney { is_local: true, .. })
        ));
        assert!(Bounties::unit_money_notice(raw, &crate::state::tests::unit(8), Some(7)).is_none());
        world.apply(&Event::UnitsRemoved(vec![7]));
        world.apply(&receipt);
        assert!(world.units.by_id.is_empty());
        world.apply(&Event::UnitSpawned(Box::new(crate::state::tests::unit(7))));
        assert_eq!(world.units.by_id[&7].bounty_money, None);
        world.apply(&Event::PlayerSpawn(Box::new(crate::session::PlayerSpawn {
            obj_id: 7,
            state: crate::model::CharacterState::default(),
            unit: crate::state::tests::unit(7),
        })));
        world.apply(&receipt);
        assert_eq!(world.player.spawn.as_ref().unwrap().unit.bounty_money, Some(i64::MIN));
        assert_eq!(
            world.player.spawn.as_ref().unwrap().state.bounty_money,
            0,
            "runtimeactor receipt does not rewrite selectedheader"
        );
    }
}
