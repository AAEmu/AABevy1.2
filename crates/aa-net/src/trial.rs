//! Received trial wait cache and every received producer of its defendant callback gate.
//! Does not implement elapsed trial clocks, invitations, votes or automatic CS072.
mod data;
mod progress;
pub use progress::{TrialHistory, TrialRulingStatus};
pub use data::{TrialCrimeRecord, TrialDefendantData, TrialRecords};
use crate::{
    session::{Event, Phase},
    wire::{Reader, Result, WireError},
};
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrialEvent {
    JuryOkCount{count:i32,total:i32},
    VerdictCount{count:i32,total:i32},
    RulingStatus(TrialRulingStatus),
    History(TrialHistory),
    DefendantData(TrialDefendantData),
    Records(TrialRecords),
    PhaseChanged { trial_id:u32,phase:u8,current_jury:i32,remaining_time_ms:u32 },
    SummonJury { trial_id: u32, court: u32, jury_number: i32 },
    SummonDefendant { trial_id: u32 },
    Waiting { order: u32, sentence_time_ms: i32 },
    JuryWaiting { count: i32, total: i32, sentence_time_ms: i32 },
    RulingClosed,
    AudienceJoined { trial_id: u32, obj_id: u32, name: Vec<u8> },
    AudienceLeft { obj_id: u32, name: Vec<u8> },
    Canceled { trial_id: u32 },
}
impl TrialEvent {
    pub fn handles(op: u16) -> bool {
        matches!(op, 0x173 | 0x175 | 0x176 | 0x177 | 0x178 | 0x179 | 0x17a | 0x17b | 0x17c | 0x17d | 0x17e | 0x17f | 0x180 | 0x181 | 0x183)
    }
    /// Native and active lane bodies agree; consume complete raw names, including NULs.
    pub fn read(op: u16, r: &mut Reader) -> Result<Self> {
        Ok(match op {
            0x179 => Self::JuryOkCount{count:r.i32()?,total:r.i32()?},
            0x17a => Self::VerdictCount{count:r.i32()?,total:r.i32()?},
            0x17d => Self::RulingStatus(TrialRulingStatus::read(r)?),
            0x181 => Self::History(TrialHistory::read(r)?),
            0x176 => Self::DefendantData(TrialDefendantData::read(r)?),
            0x177 => Self::Records(TrialRecords::read(r)?),
            0x178 => Self::PhaseChanged{trial_id:r.u32()?,phase:r.u8()?,current_jury:r.i32()?,remaining_time_ms:r.u32()?},
            0x173 => Self::SummonJury {
                trial_id: r.u32()?,
                court: r.u32()?,
                jury_number: r.i32()?,
            },
            0x175 => Self::SummonDefendant { trial_id: r.u32()? },
            0x17b => Self::Waiting {
                order: r.u32()?,
                sentence_time_ms: r.i32()?,
            },
            0x17c => Self::JuryWaiting {
                count: r.i32()?,
                total: r.i32()?,
                sentence_time_ms: r.i32()?,
            },
            0x17e => Self::RulingClosed,
            0x17f => Self::AudienceJoined {
                trial_id: r.u32()?,
                obj_id: r.bc()?,
                name: r.blob()?.to_vec(),
            },
            0x180 => Self::AudienceLeft {
                obj_id: r.bc()?,
                name: r.blob()?.to_vec(),
            },
            0x183 => Self::Canceled { trial_id: r.u32()? },
            _ => return Err(WireError::Invalid("unknown trial wait opcode".into())),
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrialNotice {
    /// JURY_OK_COUNT(count,total), both signed numericf32.
    JuryOkCount{count:i32,total:i32},
    /// VERDICT_COUNT(count,total), both signed numericf32.
    VerdictCount{count:i32,total:i32},
    /// RULING_STATUS(count,total,type,unsigned sentence bits), after cache assignment.
    RulingStatus(TrialRulingStatus),
    /// UPDATE_INGAME_BEAUTYSHOP_STATUS(), emitted when native trial availability changes.
    BeautyshopStatus,
    /// TRIAL_STATUS(phase,current jury), before phase cache admission.
    Status{phase:u8,current_jury:i32},
    /// SHOW_CRIME_RECORDS(phase).
    ShowCrimeRecords{phase:u8},
    /// SHOW_VERDICTS(minutes for 20%,50%,80%,100%,120%).
    Verdicts{minutes:[i32;5]},
    /// Event141: SHOW_DEPENDANT_WAIT_TRIAL(order as i32 then f32, sentence bits as u32 then f32).
    Waiting { order: u32, sentence_time_ms: i32 },
    /// Event142: SHOW_DEPENDANT_WAIT_JURY(signed count,total, unsigned sentence bits).
    JuryWaiting { count: i32, total: i32, sentence_time_ms: i32 },
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "bevy", derive(bevy::ecs::resource::Resource))]
pub struct Trials {
    /// Actual PlayerSpawn object, never inferred from an audience row or persistent ID.
    pub local_obj_id: Option<u32>,
    /// Native byte2 is true only while this captured defendant context is present.
    /// Retained compatibility projection; trial_id/is_jury are separate native fields.
    pub defendant_context: Option<u32>,
    pub trial_id:u32,
    pub is_jury:bool,
    pub phase:u8,
    /// Raw last received phase duration; this resource does not advance a trial clock.
    pub remaining_time_ms:u32,
    /// Last coherent details projection; invalidated when native resets defendant identity.
    pub defendant:Option<TrialDefendantData>,
    pub records:Vec<TrialCrimeRecord>,
    pub records_received:Option<i32>,
    pub records_total:Option<i32>,
    pub records_raw_unknown:Option<u32>,
    /// Native count+100 can be known before its separate total+104.
    pub current_jury:Option<i32>,
    pub jury_ok_count:Option<i32>,
    pub jury_ok_total:Option<i32>,
    pub verdict_counts:Option<[i32;2]>,
    pub ruling:Option<TrialRulingStatus>,
    /// Native byte3 is uninitialized in constructor; context adoption initializes false.
    pub has_ruling:Option<bool>,
    pub history:Option<TrialHistory>,
    /// Cached [murder,assault,theft,etc], refreshed by176, not each177 append.
    pub crime_kind_counts:Option<[i32;4]>,
    /// Native constructor does not initialize these wait fields. Unknown until a receipt.
    pub waiting_order: Option<u32>,
    pub jury_counts: Option<[i32; 2]>,
    /// Preserve raw bits: native defaultSentenceTime and wait callbacks convert UNSIGNED.
    pub sentence_time_ms: Option<i32>,
    pub last_receipt: Option<TrialEvent>,
}
impl Trials {
    pub fn is_defendant(&self) -> bool {
        self.defendant_context.is_some()
    }
    pub fn apply(&mut self, e: &Event) -> bool {
        self.apply_with_notices(e).0
    }
    /// Use instead of apply when dispatching wait callbacks. Other trial UI callbacks are not synthesized.
    pub fn apply_with_notices(&mut self, e: &Event) -> (bool, Vec<TrialNotice>) {
        let before = self.clone();
        let mut notices = Vec::new();
        match e {
            Event::PlayerSpawn(spawn) => self.local_obj_id = Some(spawn.obj_id),
            Event::UnitsRemoved(ids) => {
                if self.local_obj_id.is_some_and(|id| ids.contains(&id)) {
                    self.local_obj_id = None
                }
            }
            Event::CharacterSelected(_)
            | Event::LoginSucceeded(_)
            | Event::LeftWorld { .. }
            | Event::Disconnected { .. }
            | Event::Kicked { .. }
            | Event::PhaseChanged(Phase::Authenticating | Phase::Connecting) => *self = Self::default(),
            Event::Trial(receipt) => {
                self.last_receipt = Some(receipt.clone());
                match receipt {
                    TrialEvent::JuryOkCount{count,total} => {
                        self.jury_ok_count=Some(*count);self.jury_ok_total=Some(*total);
                        notices.push(TrialNotice::JuryOkCount{count:*count,total:*total});
                    }
                    TrialEvent::VerdictCount{count,total} => {
                        self.verdict_counts=Some([*count,*total]);
                        if let Some(status)=self.ruling.as_mut(){status.count=*count;status.total=*total;}
                        notices.push(TrialNotice::VerdictCount{count:*count,total:*total});
                    }
                    TrialEvent::RulingStatus(status) => {
                        self.verdict_counts=Some([status.count,status.total]);self.ruling=Some(*status);self.has_ruling=Some(true);
                        notices.push(TrialNotice::RulingStatus(*status));
                    }
                    TrialEvent::History(history) => self.history=Some(history.clone()),
                    TrialEvent::Waiting { order, sentence_time_ms } => {
                        self.waiting_order = Some(*order);
                        self.jury_counts = Some([0, 0]);
                        self.current_jury=Some(0);
                        self.sentence_time_ms = Some(*sentence_time_ms);
                        notices.push(TrialNotice::Waiting {
                            order: *order,
                            sentence_time_ms: *sentence_time_ms,
                        });
                    }
                    TrialEvent::JuryWaiting {
                        count,
                        total,
                        sentence_time_ms,
                    } => {
                        self.jury_counts = Some([*count, *total]);
                        self.current_jury=Some(*count);
                        self.sentence_time_ms = Some(*sentence_time_ms);
                        if self.is_defendant() {
                            notices.push(TrialNotice::JuryWaiting {
                                count: *count,
                                total: *total,
                                sentence_time_ms: *sentence_time_ms,
                            });
                        }
                    }
                    TrialEvent::SummonDefendant { trial_id } => {
                        self.adopt_trial(*trial_id,&mut notices);self.defendant_context=Some(*trial_id);self.is_jury=false;
                    }
                    TrialEvent::SummonJury { trial_id,.. } => {
                        self.adopt_trial(*trial_id,&mut notices);self.defendant_context=None;self.is_jury=true;
                    }
                    TrialEvent::RulingClosed | TrialEvent::Canceled { .. } => self.reset_role(&mut notices),
                    TrialEvent::DefendantData(data) => {
                        if data.name.len()<=128 {
                            self.defendant=Some(data.clone());self.sentence_time_ms=Some(data.sentence_time_ms);
                            let mut counts=[0i32;4];for record in &self.records {
                                let i=match record.crime_kind {3=>0,2=>1,4=>2,_=>3};counts[i]=counts[i].wrapping_add(1);
                            }
                            self.crime_kind_counts=Some(counts);
                            notices.push(TrialNotice::ShowCrimeRecords{phase:self.phase});
                            if !self.is_defendant()&&self.is_jury&&self.trial_id!=0 {
                                if let Some(count)=self.current_jury {notices.push(TrialNotice::Status{phase:self.phase,current_jury:count});}
                                notices.push(TrialNotice::ShowCrimeRecords{phase:self.phase});
                            }
                        }
                    }
                    TrialEvent::Records(data) => {
                        if data.native_safe() {
                            if !self.is_jury&&!self.is_defendant() {
                                self.adopt_trial(data.trial_id,&mut notices);
                                if data.trial_id==0 {return (*self!=before,notices)}
                            }
                            self.records_raw_unknown=Some(data.raw_unknown);
                            self.records_total=Some(data.total_count);
                            self.records_received=self.records_received.map(|n|n.wrapping_add(data.count));
                            self.records.extend(data.records.iter().cloned());
                        }
                    }
                    TrialEvent::PhaseChanged{trial_id,phase,current_jury,remaining_time_ms} => {
                        notices.push(TrialNotice::Status{phase:*phase,current_jury:*current_jury});
                        if *trial_id==self.trial_id || (!self.is_jury&&!self.is_defendant()) {
                            self.adopt_trial(*trial_id,&mut notices);
                            self.phase=*phase;self.remaining_time_ms=*remaining_time_ms;self.current_jury=Some(*current_jury);
                            let total=self.jury_counts.map(|v|v[1]);self.jury_counts=total.map(|v|[*current_jury,v]);
                            match *phase {
                                2 => {
                                    if self.is_defendant() {if let (Some(total),Some(sentence_time_ms))=(total,self.sentence_time_ms) {
                                        notices.push(TrialNotice::JuryWaiting{count:*current_jury,total,sentence_time_ms});
                                    }}
                                    notices.push(TrialNotice::ShowCrimeRecords{phase:*phase});
                                }
                                3|4 => notices.push(TrialNotice::ShowCrimeRecords{phase:*phase}),
                                5 if self.is_jury => {if let Some(ms)=self.sentence_time_ms {notices.push(TrialNotice::Verdicts{minutes:verdict_minutes(ms as u32)});}},
                                _=>{}
                            }
                        }
                    }
                    TrialEvent::AudienceJoined { trial_id, obj_id, name } => {
                        // Native fallback39ade6a8 is0 when client has no local actor.
                        if name.len() <= 128 && *obj_id == self.local_obj_id.unwrap_or(0) {
                            self.adopt_trial(*trial_id,&mut notices);
                        }
                    }
                    TrialEvent::AudienceLeft { obj_id, name } => {
                        if name.len() <= 128 && *obj_id == self.local_obj_id.unwrap_or(0) {
                            self.reset_role(&mut notices)
                        }
                    }
                }
            }
            _ => {}
        }
        (*self != before, notices)
    }
    fn adopt_trial(&mut self,id:u32,notices:&mut Vec<TrialNotice>){
        if self.trial_id==id{return}
        self.trial_id=id;self.defendant_context=None;self.is_jury=false;
        self.defendant=None;self.records.clear();self.records_received=Some(0);self.records_total=Some(0);
        self.jury_ok_count=Some(0);self.verdict_counts=Some([0,0]);self.has_ruling=Some(false);self.crime_kind_counts=Some([0;4]);
        if let Some(status)=self.ruling.as_mut(){status.count=0;status.total=0;}
        notices.push(TrialNotice::BeautyshopStatus);
    }
    fn reset_role(&mut self,notices:&mut Vec<TrialNotice>){
        self.trial_id=0;self.defendant_context=None;self.is_jury=false;self.phase=0;self.remaining_time_ms=0;
        if let Some(data)=self.defendant.as_mut(){data.judge_obj_id=0;}
        notices.push(TrialNotice::BeautyshopStatus);
    }

}

/// Native39080201..303: each f32 product narrows, except the100% integer division.
fn verdict_minutes(ms:u32)->[i32;5]{
    let n=ms as f32;let minute=f32::from_bits(0x378bcf65);
    let cvtt=|x:f32| if x>=2147483648.0 || x< -2147483648.0 || !x.is_finite(){i32::MIN}else{x as i32};
    [cvtt((n*f32::from_bits(0x3e4ccccd))*minute),cvtt((n*0.5)*minute),cvtt((n*f32::from_bits(0x3f4ccccd))*minute),(ms/60000) as i32,cvtt((n*f32::from_bits(0x3f99999a))*minute)]
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{game::ServerMsg, session::PlayerSpawn, wire::Writer};
    fn fold(s: &mut Trials, e: TrialEvent) -> Vec<TrialNotice> {
        s.apply_with_notices(&Event::Trial(e)).1
    }
    fn own(s: &mut Trials, id: u32) {
        s.apply(&Event::PlayerSpawn(Box::new(PlayerSpawn {
            obj_id: id,
            state: Default::default(),
            unit: crate::state::tests::unit(id),
        })));
    }
    #[test]
    fn native_lane_layouts_every_truncation_and_raw_dd04_following_packet() {
        let rows = [
            (
                0x173,
                vec![1, 0, 0, 0, 255, 255, 255, 255, 0, 0, 0, 128],
                TrialEvent::SummonJury {
                    trial_id: 1,
                    court: u32::MAX,
                    jury_number: i32::MIN,
                },
            ),
            (0x175, vec![255, 255, 255, 255], TrialEvent::SummonDefendant { trial_id: u32::MAX }),
            (
                0x17b,
                vec![0, 0, 0, 128, 255, 255, 255, 255],
                TrialEvent::Waiting {
                    order: 0x80000000,
                    sentence_time_ms: -1,
                },
            ),
            (
                0x17c,
                vec![255, 255, 255, 255, 0, 0, 0, 128, 255, 255, 255, 127],
                TrialEvent::JuryWaiting {
                    count: -1,
                    total: i32::MIN,
                    sentence_time_ms: i32::MAX,
                },
            ),
            (0x17e, vec![], TrialEvent::RulingClosed),
            (
                0x17f,
                vec![1, 0, 0, 0, 0x56, 0x34, 0x12, 3, 0, b'A', 0, 255],
                TrialEvent::AudienceJoined {
                    trial_id: 1,
                    obj_id: 0x123456,
                    name: vec![b'A', 0, 255],
                },
            ),
            (
                0x180,
                vec![0x56, 0x34, 0x12, 3, 0, b'A', 0, 255],
                TrialEvent::AudienceLeft {
                    obj_id: 0x123456,
                    name: vec![b'A', 0, 255],
                },
            ),
            (0x183, vec![255, 255, 255, 255], TrialEvent::Canceled { trial_id: u32::MAX }),
        ];
        let mut batch = Writer::new();
        for (op, body, expected) in &rows {
            assert!(TrialEvent::handles(*op));
            let mut r = Reader::new(body);
            assert_eq!(TrialEvent::read(*op, &mut r).unwrap(), *expected);
            assert!(r.is_empty());
            for n in 0..body.len() {
                assert!(TrialEvent::read(*op, &mut Reader::new(&body[..n])).is_err(), "{op:x}/{n}");
            }
            batch.u16(0).u16(*op).raw(body);
        }
        assert!(!TrialEvent::handles(0x174));
        assert!(TrialEvent::read(0x174, &mut Reader::new(&[])).is_err());
        batch.u16(0).u16(0x1bc).u32(99);
        use std::io::Write;
        let mut compressor = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        compressor.write_all(&batch.finish()).unwrap();
        let mut packet = Writer::new();
        packet.u8(0xdd).u8(crate::game::LEVEL_COMPRESSED).u16(9).raw(&compressor.finish().unwrap());
        let ServerMsg::Batch(actual) = ServerMsg::decode(&packet.finish()).unwrap() else {
            panic!()
        };
        assert_eq!(actual.len(), 9);
        for (i, (_, _, expected)) in rows.iter().enumerate() {
            assert!(matches!(&actual[i],ServerMsg::Trial(e) if e==expected));
        }
        assert!(matches!(actual[8], ServerMsg::SetBreath { breath: 99 }));
    }
    #[test]
    fn original_x86_wait_cache_and_callback_oracle_contract() {
        // trial_wait_oracle.py executes original3907c320/3907c3f0; Lua dispatch recording only mocked.
        let mut s = Trials::default();
        assert_eq!((s.waiting_order, s.jury_counts, s.sentence_time_ms), (None, None, None));
        assert_eq!(
            fold(
                &mut s,
                TrialEvent::Waiting {
                    order: 0x80000000,
                    sentence_time_ms: -1
                }
            ),
            vec![TrialNotice::Waiting {
                order: 0x80000000,
                sentence_time_ms: -1
            }]
        );
        assert_eq!((s.waiting_order, s.jury_counts, s.sentence_time_ms), (Some(0x80000000), Some([0, 0]), Some(-1)));
        assert_eq!((0x80000000u32 as i32 as f32, (-1i32) as u32 as f32), (-2147483648.0, 4294967296.0));
        assert!(
            fold(
                &mut s,
                TrialEvent::JuryWaiting {
                    count: -1,
                    total: i32::MAX,
                    sentence_time_ms: -1
                }
            )
            .is_empty()
        );
        assert_eq!(s.jury_counts, Some([-1, i32::MAX]), "gate suppresses only callback, not cache write");
        fold(&mut s, TrialEvent::SummonDefendant { trial_id: 7 });
        for _ in 0..2 {
            assert_eq!(
                fold(
                    &mut s,
                    TrialEvent::JuryWaiting {
                        count: i32::MIN,
                        total: i32::MIN,
                        sentence_time_ms: i32::MIN
                    }
                ),
                vec![TrialNotice::JuryWaiting {
                    count: i32::MIN,
                    total: i32::MIN,
                    sentence_time_ms: i32::MIN
                }]
            );
        }
        assert_eq!((i32::MIN as f32, (i32::MIN as u32) as f32), (-2147483648.0, 2147483648.0));
        let wait = (s.waiting_order, s.jury_counts, s.sentence_time_ms);
        fold(&mut s, TrialEvent::Canceled { trial_id: 999 });
        assert!(!s.is_defendant(), "native cancellation does not compare provided trial ID");
        assert_eq!(
            (s.waiting_order, s.jury_counts, s.sentence_time_ms),
            wait,
            "native role reset retains initialized wait fields"
        );
        assert!(
            fold(
                &mut s,
                TrialEvent::JuryWaiting {
                    count: 1,
                    total: 2,
                    sentence_time_ms: 3
                }
            )
            .is_empty()
        );
    }
    #[test]
    fn all_role_gate_mutators_use_actual_local_object_and_source_context() {
        let mut s = Trials::default();
        own(&mut s, 42);
        for clear in [
            TrialEvent::SummonJury {
                trial_id: 8,
                court: 0,
                jury_number: 0,
            },
            TrialEvent::RulingClosed,
            TrialEvent::Canceled { trial_id: 0 },
        ] {
            fold(&mut s, TrialEvent::SummonDefendant { trial_id: 7 });
            fold(&mut s, clear);
            assert!(!s.is_defendant());
        }
        fold(&mut s, TrialEvent::SummonDefendant { trial_id: 7 });
        fold(
            &mut s,
            TrialEvent::AudienceJoined {
                trial_id: 8,
                obj_id: 99,
                name: vec![],
            },
        );
        assert_eq!(s.defendant_context, Some(7));
        fold(
            &mut s,
            TrialEvent::AudienceJoined {
                trial_id: 7,
                obj_id: 42,
                name: vec![],
            },
        );
        assert_eq!(s.defendant_context, Some(7));
        fold(&mut s, TrialEvent::AudienceLeft { obj_id: 99, name: vec![] });
        assert_eq!(s.defendant_context, Some(7));
        fold(
            &mut s,
            TrialEvent::AudienceJoined {
                trial_id: 8,
                obj_id: 42,
                name: vec![],
            },
        );
        assert!(!s.is_defendant());
        fold(&mut s, TrialEvent::SummonDefendant { trial_id: 0 });
        assert!(s.is_defendant(), "native flag can be true even with raw trial0");
        fold(&mut s, TrialEvent::AudienceLeft { obj_id: 42, name: vec![] });
        assert!(!s.is_defendant());
        fold(&mut s, TrialEvent::SummonDefendant { trial_id: 7 });
        s.apply(&Event::UnitsRemoved(vec![42]));
        assert!(s.is_defendant());
        fold(&mut s, TrialEvent::AudienceLeft { obj_id: 42, name: vec![] });
        assert!(s.is_defendant(), "removed numeric actor is no longer local");
        fold(&mut s, TrialEvent::AudienceLeft { obj_id: 0, name: vec![] });
        assert!(!s.is_defendant(), "native no-actor comparison uses source invalidID0");
    }
    #[test]
    fn malformed_native_names_remain_raw_and_lifecycle_never_leaks_context() {
        let mut s = Trials::default();
        own(&mut s, 42);
        fold(&mut s, TrialEvent::SummonDefendant { trial_id: 7 });
        let mut w = Writer::new();
        w.bc(42).blob(&vec![255; 129]);
        let raw = TrialEvent::read(0x180, &mut Reader::new(&w.finish())).unwrap();
        fold(&mut s, raw.clone());
        assert_eq!(s.last_receipt, Some(raw));
        assert!(s.is_defendant(), "safe cache refuses oversize native128-byte buffer input");
        for e in [
            Event::CharacterSelected(Box::default()),
            Event::LeftWorld { target: 0 },
            Event::Disconnected { message: String::new() },
            Event::Kicked {
                reason: 0,
                message: String::new(),
            },
        ] {
            fold(&mut s, TrialEvent::SummonDefendant { trial_id: 7 });
            s.apply(&e);
            assert_eq!(s, Trials::default());
        }
        let mut world = crate::state::WorldState::default();
        world.apply(&Event::Trial(TrialEvent::Waiting { order: 2, sentence_time_ms: 3 }));
        assert_eq!(world.trials.jury_counts, Some([0, 0]));
    }
}
