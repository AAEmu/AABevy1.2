//! Executed native callback cache points: native-trial-phase-oracle and jury-summon-reply-oracle.
use super::*;
fn fold(s: &mut Trials, e: TrialEvent) {
    s.apply(&Event::Trial(e));
}
fn row(id: u32) -> TrialCrimeRecord {
    TrialCrimeRecord {
        id,
        victim_id: 0,
        victim_name: vec![],
        reporter_id: 0,
        reporter_name: vec![],
        victim_faction_id: 148,
        victim_expedition_id: 0,
        crime_kind: 3,
        doodad_template_id: 0,
        used_skill_id: 0,
        position_x: 0,
        position_y: 0,
        position_z_bits: 0,
        message: vec![],
        report_time: 0,
    }
}
fn seed() -> Trials {
    let mut s = Trials::default();
    fold(
        &mut s,
        TrialEvent::Records(TrialRecords {
            trial_id: 7,
            raw_unknown: 0,
            total_count: 2,
            count: 2,
            records: vec![row(11), row(12)],
        }),
    );
    fold(
        &mut s,
        TrialEvent::DefendantData(TrialDefendantData {
            defendant_id: 9,
            name: b"old\0tail".to_vec(),
            race: 1,
            faction_id: 148,
            trial_id: 999,
            sentence_time_ms: -1,
            judge_obj_id: 3,
        }),
    );
    fold(
        &mut s,
        TrialEvent::JuryWaiting {
            count: 99,
            total: 6,
            sentence_time_ms: -1,
        },
    );
    fold(
        &mut s,
        TrialEvent::History(TrialHistory {
            player_id: 99,
            crime_point: 1,
            arrest: 2,
            accept_guilty: 3,
            accept_trial: 4,
            not_guilty: 5,
            guilty: 6,
            total_seconds: 7,
            current_seconds: 8,
            crime_record_count: 9,
            bot_report: 10,
        }),
    );
    fold(
        &mut s,
        TrialEvent::PhaseChanged {
            trial_id: 7,
            phase: 1,
            current_jury: 99,
            remaining_time_ms: 123,
        },
    );
    s
}
// These are the corresponding original GetCrimeData/record getter reads, not final-cache aliases.
fn name(s: &Trials) -> Option<&[u8]> {
    s.defendant.as_ref().map(|d| d.name.split(|b| *b == 0).next().unwrap())
}
fn sentence(s: &Trials) -> Option<f32> {
    s.sentence_time_ms.map(|v| (v as u32) as f32)
}
fn total_records(s: &Trials) -> Option<f32> {
    s.records_raw_unknown
        .and_then(|kind| s.history.as_ref().map(|h| (if kind == 0 { h.crime_record_count } else { h.bot_report }) as f32))
}
fn ids(s: &Trials) -> Vec<u32> {
    s.records.iter().map(|r| r.id).collect()
}
#[test]
fn phase_status_reads_old_getters_beautyshop_reads_adopted_context_and_records_read_final() {
    let mut s = seed();
    let event = Event::Trial(TrialEvent::PhaseChanged {
        trial_id: 8,
        phase: 2,
        current_jury: 5,
        remaining_time_ms: 456,
    });
    let (changed, dispatch) = s.apply_with_dispatch(&event);
    assert!(changed);
    assert_eq!(dispatch.len(), 3);
    let (notice, before) = &dispatch[0];
    assert_eq!(*notice, TrialNotice::Status { phase: 2, current_jury: 5 });
    assert_eq!(
        (before.trial_id, before.phase, before.current_jury, before.remaining_time_ms),
        (7, 1, Some(99), 123)
    );
    assert_eq!(ids(before), [11, 12]);
    assert_eq!(name(before), Some(&b"old"[..]));
    assert_eq!(before.crime_kind_counts, Some([2, 0, 0, 0]));
    assert_eq!(before.records_total, Some(2));
    assert_eq!((sentence(before), total_records(before)), (Some(4294967296.0), Some(9.0)));
    let (notice, adopted) = &dispatch[1];
    assert_eq!(*notice, TrialNotice::BeautyshopStatus);
    assert_eq!(
        (adopted.trial_id, adopted.phase, adopted.current_jury, adopted.remaining_time_ms),
        (8, 1, Some(99), 123)
    );
    assert!(!adopted.is_jury);
    assert!(!adopted.is_defendant());
    assert!(ids(adopted).is_empty());
    assert_eq!(name(adopted), None);
    assert_eq!(adopted.crime_kind_counts, Some([0; 4]));
    assert_eq!(adopted.records_total, Some(0));
    assert_eq!(total_records(adopted), Some(9.0));
    let (notice, after) = &dispatch[2];
    assert_eq!(*notice, TrialNotice::ShowCrimeRecords { phase: 2 });
    assert_eq!((after.phase, after.current_jury, after.remaining_time_ms), (2, Some(5), 456));
    assert_eq!(*after, s);
}
#[test]
fn changed_summon_beautyshop_getcrime_role_is_before_native_final_jury_assignment() {
    let mut s = seed();
    fold(&mut s, TrialEvent::SummonDefendant { trial_id: 7 });
    let (_, dispatch) = s.apply_with_dispatch(&Event::Trial(TrialEvent::SummonJury {
        trial_id: 8,
        court: 1,
        jury_number: 2,
    }));
    assert_eq!(dispatch.len(), 1);
    let (notice, at_callback) = &dispatch[0];
    assert_eq!(*notice, TrialNotice::BeautyshopStatus);
    assert_eq!(at_callback.trial_id, 8);
    assert!(!at_callback.is_jury);
    assert!(!at_callback.is_defendant());
    assert!(ids(at_callback).is_empty());
    assert!(s.is_jury);
    assert!(!s.is_defendant());
    assert_eq!(s.trial_id, 8);
    let data = TrialDefendantData {
        defendant_id: 10,
        name: b"new".to_vec(),
        race: 1,
        faction_id: 148,
        trial_id: 999,
        sentence_time_ms: 120000,
        judge_obj_id: 3,
    };
    let (_, updated) = s.apply_with_dispatch(&Event::Trial(TrialEvent::DefendantData(data)));
    assert_eq!(
        updated.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>(),
        [
            TrialNotice::ShowCrimeRecords { phase: 1 },
            TrialNotice::Status { phase: 1, current_jury: 99 },
            TrialNotice::ShowCrimeRecords { phase: 1 }
        ]
    );
    for (_, view) in updated {
        assert!(view.is_jury);
        assert_eq!(name(&view), Some(&b"new"[..]));
        assert_eq!(sentence(&view), Some(120000.0));
    }
}
#[test]
fn admitted_and_refused_active_phase_getters_follow_native_status_then_assignment() {
    let mut s = seed();
    fold(&mut s, TrialEvent::SummonDefendant { trial_id: 7 });
    let (_, rejected) = s.apply_with_dispatch(&Event::Trial(TrialEvent::PhaseChanged {
        trial_id: 8,
        phase: 2,
        current_jury: 5,
        remaining_time_ms: 456,
    }));
    assert_eq!(rejected.len(), 1);
    assert_eq!(rejected[0].1.trial_id, 7);
    assert_eq!(rejected[0].1.phase, 1);
    assert_eq!(ids(&rejected[0].1), [11, 12]);
    assert!(s.is_defendant());
    assert_eq!(s.phase, 1);
    let (_, mut accepted) = s.apply_with_dispatch(&Event::Trial(TrialEvent::PhaseChanged {
        trial_id: 7,
        phase: 2,
        current_jury: 5,
        remaining_time_ms: 456,
    }));
    assert_eq!(accepted.len(), 3);
    assert_eq!(accepted[0].1.phase, 1);
    assert_eq!(accepted[0].1.current_jury, Some(99));
    assert_eq!(
        accepted[1].0,
        TrialNotice::JuryWaiting {
            count: 5,
            total: 6,
            sentence_time_ms: -1
        }
    );
    assert_eq!(accepted[1].1.phase, 2);
    assert_eq!(accepted[1].1.current_jury, Some(5));
    assert!(accepted[1].1.is_defendant());
    assert_eq!(ids(&accepted[1].1), [11, 12]);
    assert_eq!(accepted[2].1, s);
    accepted[2].1.records.clear();
    assert_eq!(ids(&s), [11, 12], "callback view is an owned projection, not canonical state mutation");
}
#[test]
fn existing_apply_paths_keep_final_state_and_notices_without_snapshot_collection() {
    let mut plain = seed();
    let mut notices = plain.clone();
    let mut staged = plain.clone();
    for event in [
        Event::Trial(TrialEvent::SummonJury {
            trial_id: 8,
            court: 1,
            jury_number: 2,
        }),
        Event::Trial(TrialEvent::PhaseChanged {
            trial_id: 8,
            phase: 3,
            current_jury: 2,
            remaining_time_ms: 999,
        }),
        Event::Trial(TrialEvent::Canceled { trial_id: 999 }),
        Event::Snow(true),
    ] {
        let a = plain.apply(&event);
        let (b, ns) = notices.apply_with_notices(&event);
        let (c, ds) = staged.apply_with_dispatch(&event);
        assert_eq!((a, b), (c, c));
        assert_eq!(plain, notices);
        assert_eq!(plain, staged);
        assert_eq!(ns, ds.into_iter().map(|(n, _)| n).collect::<Vec<_>>());
    }
}

#[test]
fn records_adoption_callback_precedes_new_type_headers_and_rows() {
    let mut s = seed();
    let event = Event::Trial(TrialEvent::Records(TrialRecords {
        trial_id: 8,
        raw_unknown: 1,
        total_count: 3,
        count: 1,
        records: vec![row(77)],
    }));
    let (_, dispatch) = s.apply_with_dispatch(&event);
    assert_eq!(dispatch.len(), 1);
    let (notice, view) = &dispatch[0];
    assert_eq!(*notice, TrialNotice::BeautyshopStatus);
    assert_eq!(
        (view.trial_id, view.records_raw_unknown, view.records_total, view.records_received),
        (8, Some(0), Some(0), Some(0))
    );
    assert!(view.records.is_empty());
    assert_eq!(total_records(view), Some(9.0));
    assert_eq!(
        (s.trial_id, s.records_raw_unknown, s.records_total, s.records_received),
        (8, Some(1), Some(3), Some(1))
    );
    assert_eq!(ids(&s), [77]);
    assert_eq!(total_records(&s), Some(10.0));
}


#[test]
fn close_and_audience_native_dispatch_has_pre_and_post_reset_getters() {
    let mut s=seed();s.local_obj_id=Some(42);s.defendant_context=Some(7);
    let (_,ds)=s.apply_with_dispatch(&Event::Trial(TrialEvent::AudienceJoined{trial_id:8,obj_id:42,name:b"name\0tail".to_vec()}));
    assert_eq!(ds.iter().map(|(n,_)|n.clone()).collect::<Vec<_>>(),[TrialNotice::AudienceJoined{name:b"name\0tail".to_vec()},TrialNotice::BeautyshopStatus]);
    assert_eq!((ds[0].1.trial_id,ds[0].1.is_defendant(),ids(&ds[0].1)),(7,true,vec![11,12]));
    assert_eq!((ds[1].1.trial_id,ds[1].1.is_defendant(),ids(&ds[1].1)),(8,false,vec![]));
    let (_,ds)=s.apply_with_dispatch(&Event::Trial(TrialEvent::AudienceLeft{obj_id:42,name:vec![255,0,1]}));
    assert_eq!(ds.iter().map(|(n,_)|n.clone()).collect::<Vec<_>>(),[TrialNotice::Closed,TrialNotice::BeautyshopStatus,TrialNotice::AudienceLeft{name:vec![255,0,1]}]);
    assert_eq!(ds.iter().map(|(_,v)|v.trial_id).collect::<Vec<_>>(),[8,0,0]);
    let (_,ds)=s.apply_with_dispatch(&Event::Trial(TrialEvent::AudienceLeft{obj_id:42,name:vec![]}));
    assert_eq!(ds.iter().map(|(n,_)|n.clone()).collect::<Vec<_>>(),[TrialNotice::BeautyshopStatus,TrialNotice::AudienceLeft{name:vec![]}]);
    for (e,tail) in [(TrialEvent::RulingClosed,vec![TrialNotice::CloseTrialTasks,TrialNotice::RulingClosed,TrialNotice::Closed]),(TrialEvent::Canceled{trial_id:999},vec![TrialNotice::CloseTrialTasks,TrialNotice::Closed,TrialNotice::Canceled])] {
        s=seed();s.defendant_context=Some(7);let(_,ds)=s.apply_with_dispatch(&Event::Trial(e));
        assert_eq!(ds[0].0,TrialNotice::BeautyshopStatus);assert_eq!(ds[1..].iter().map(|(n,_)|n.clone()).collect::<Vec<_>>(),tail);
        for (_,v) in ds {assert_eq!((v.trial_id,v.is_defendant(),v.phase),(0,false,0));assert_eq!(ids(&v),[11,12]);}
    }
}

#[test]
fn audience_nonlocal_duplicate_and_oversize_raw_receipts_are_distinct() {
    let mut s=seed();s.local_obj_id=Some(42);
    for e in [TrialEvent::AudienceJoined{trial_id:8,obj_id:99,name:b"other".to_vec()},TrialEvent::AudienceLeft{obj_id:99,name:b"other".to_vec()}] {
        for _ in 0..2 {let(_,ds)=s.apply_with_dispatch(&Event::Trial(e.clone()));assert_eq!(ds.len(),1);assert_eq!(ds[0].1.trial_id,7);assert_eq!(ids(&ds[0].1),[11,12]);}
    }
    for e in [TrialEvent::AudienceJoined{trial_id:8,obj_id:42,name:vec![0;129]},TrialEvent::AudienceLeft{obj_id:42,name:vec![0;129]}] {
        let(_,ds)=s.apply_with_dispatch(&Event::Trial(e.clone()));assert!(ds.is_empty());assert_eq!(s.trial_id,7);assert_eq!(s.last_receipt,Some(e));
    }
}
