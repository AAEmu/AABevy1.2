use super::*;
use crate::game::{ClientMsg, ServerMsg};
fn fold(s: &mut Battlefield, e: BattlefieldEvent) {
    s.apply(&Event::Battlefield(e));
}
fn request(s: &mut Battlefield, id: u32, c: u8) {
    fold(s, BattlefieldEvent::Requested(BattlefieldCommand::Apply { battlefield_id: id, corps: c }));
}
fn applied(id: u32, c: u8) -> BattlefieldEvent {
    BattlefieldEvent::Applied {
        battlefield_id: id,
        corps: c,
        error: 0,
    }
}
fn queue(id: u32) -> BattlefieldEvent {
    BattlefieldEvent::QueueState(BattlefieldQueue {
        battlefield_id: id,
        corps_size: 3,
        counts: [9, 1, 9],
    })
}
fn check(op: u16, body: &[u8], e: BattlefieldEvent) {
    let mut w = Writer::new();
    w.raw(&[0xdd, 1, 0, 1]).u16(op).raw(body);
    assert_eq!(ServerMsg::decode(&w.finish()).unwrap(), ServerMsg::Battlefield(e.clone()));
    for cut in 0..body.len() {
        assert!(BattlefieldEvent::read(op, &mut Reader::new(&body[..cut])).is_err(), "{op:x} cut{cut}");
    }
    let mut w = Writer::new();
    w.u16(0).u16(op).raw(body).u16(0).u16(sc::SCOnOffSnowPacket).bool(true);
    let raw = w.finish();
    let n = raw.len() as u16;
    let mut w = Writer::new();
    w.raw(&[0xdd, 4]).u16(2).u8(1).u16(n).u16(!n).raw(&raw);
    assert_eq!(
        ServerMsg::decode(&w.finish()).unwrap(),
        ServerMsg::Batch(vec![ServerMsg::Battlefield(e), ServerMsg::Snow(true)])
    );
}
#[test]
fn queue_wire_goldens_all_truncations_and_dd04_alignment() {
    check(
        0x18b,
        &[7, 0, 0, 0, 2, 0x34, 0x12],
        BattlefieldEvent::Applied {
            battlefield_id: 7,
            corps: 2,
            error: 0x1234,
        },
    );
    check(0x18c, &[0x34, 0x12, 1, 0], BattlefieldEvent::Cancelled { raw: 0x11234 });
    check(0x18d, &[7, 0, 0, 0, 3, 9, 0, 1, 0, 9, 0], queue(7));
    let i = BattlefieldInvitation {
        zone_id: 8,
        instance_id: 9,
        ruleset_id: 10,
        corps: 1,
        qualified_id: 0x0102030405060708,
    };
    check(
        0x18e,
        &[8, 0, 0, 0, 9, 0, 0, 0, 10, 0, 0, 0, 1, 8, 7, 6, 5, 4, 3, 2, 1],
        BattlefieldEvent::Invited(i),
    );
    check(
        0x18f,
        &[8, 0, 0, 0, 9, 0, 0, 0, 10, 0, 0, 0, 1],
        BattlefieldEvent::InviteTimedOut {
            zone_id: 8,
            instance_id: 9,
            ruleset_id: 10,
            corps: 1,
        },
    );
}
#[test]
fn commands_have_exact_native_wire_and_validation() {
    let c = BattlefieldCommand::Apply {
        battlefield_id: 0x01020304,
        corps: 2,
    };
    assert_eq!(c.opcode(), 0xe1);
    assert_eq!(ClientMsg::Battlefield(c).encode_body(), [4, 3, 2, 1, 2]);
    assert_eq!(BattlefieldCommand::Cancel.opcode(), 0xe2);
    assert!(ClientMsg::Battlefield(BattlefieldCommand::Cancel).encode_body().is_empty());
    assert!(BattlefieldCommand::Apply { battlefield_id: 0, corps: 0 }.validate().is_err());
    assert!(BattlefieldCommand::Apply { battlefield_id: 7, corps: 3 }.validate().is_err());
}
#[test]
fn native_default_and_matching_receipts_do_not_invent_population() {
    let mut s = Battlefield::default();
    assert_eq!(s.joined_infos(), (0, 0, -1));
    fold(&mut s, queue(7));
    fold(&mut s, applied(7, 2));
    assert!(s.applied.is_none());
    request(&mut s, 7, 2);
    assert_eq!(s.joined_infos(), (0, 0, -1));
    fold(&mut s, applied(7, 1));
    assert!(s.applied.is_none());
    fold(&mut s, applied(8, 2));
    assert!(s.applied.is_none());
    fold(&mut s, applied(7, 2));
    assert_eq!(s.joined_infos(), (0, 0, 2));
    assert!(s.queue.is_none());
    fold(&mut s, queue(8));
    assert!(s.queue.is_none());
    fold(&mut s, queue(7));
    assert_eq!(s.joined_infos(), (6, 3, 2));
    fold(&mut s, BattlefieldEvent::Requested(BattlefieldCommand::Cancel));
    assert!(s.cancel_pending);
    assert_eq!(s.joined_infos(), (6, 3, 2));
    fold(&mut s, BattlefieldEvent::Cancelled { raw: 0xffff_ffff });
    assert_eq!(s, Battlefield::default());
    fold(&mut s, queue(7));
    assert!(s.queue.is_none());
    request(&mut s, 8, 0);
    fold(
        &mut s,
        BattlefieldEvent::Applied {
            battlefield_id: 8,
            corps: 0,
            error: 91,
        },
    );
    assert_eq!(s.last_error, Some(91));
    assert!(s.applied.is_none());
}
#[test]
fn invitation_rejects_wrong_corps_stops_queue_updates_and_timeout_matches_all_fields() {
    let mut s = Battlefield::default();
    request(&mut s, 7, 2);
    fold(&mut s, applied(7, 2));
    fold(&mut s, queue(7));
    let mut i = BattlefieldInvitation {
        zone_id: 8,
        instance_id: 9,
        ruleset_id: 10,
        corps: 2,
        qualified_id: 11,
    };
    fold(&mut s, BattlefieldEvent::Invited(i));
    assert!(s.invitation.is_none());
    i.corps = 1;
    fold(&mut s, BattlefieldEvent::Invited(i));
    assert_eq!(s.invitation, Some(i));
    fold(
        &mut s,
        BattlefieldEvent::QueueState(BattlefieldQueue {
            battlefield_id: 7,
            corps_size: 99,
            counts: [0; 3],
        }),
    );
    assert_eq!(s.joined_infos(), (6, 3, 2));
    for (z, n, r, c) in [(0, 9, 10, 1), (8, 0, 10, 1), (8, 9, 0, 1), (8, 9, 10, 0)] {
        fold(
            &mut s,
            BattlefieldEvent::InviteTimedOut {
                zone_id: z,
                instance_id: n,
                ruleset_id: r,
                corps: c,
            },
        );
        assert!(s.invitation.is_some());
    }
    fold(
        &mut s,
        BattlefieldEvent::InviteTimedOut {
            zone_id: 8,
            instance_id: 9,
            ruleset_id: 10,
            corps: 1,
        },
    );
    assert!(s.invitation.is_none());
}
#[test]
fn lifecycle_worldstate_and_failed_sends_keep_default_received_state() {
    for e in [
        Event::CharacterSelected(Box::default()),
        Event::LeftWorld { target: 0 },
        Event::Disconnected { message: "test".into() },
        Event::Kicked {
            reason: 1,
            message: "test".into(),
        },
    ] {
        let mut w = crate::state::WorldState::default();
        w.apply(&Event::Battlefield(BattlefieldEvent::Requested(BattlefieldCommand::Apply {
            battlefield_id: 7,
            corps: 0,
        })));
        w.apply(&Event::Battlefield(applied(7, 0)));
        w.apply(&Event::Battlefield(queue(7)));
        assert!(w.battlefield.queue.is_some());
        w.apply(&e);
        assert_eq!(w.battlefield, Battlefield::default());
    }
    let mut s = Battlefield::default();
    fold(
        &mut s,
        BattlefieldEvent::RequestRejected {
            command: BattlefieldCommand::Apply { battlefield_id: 7, corps: 2 },
            message: "not sent".into(),
        },
    );
    assert_eq!(s.joined_infos(), (0, 0, -1));
    assert!(s.requesting.is_none());
}

#[test]
fn native_cancel_is_explicit_two_byte_signed_source_contract() {
    let mut r = Reader::new(&[0xff, 0xff, 0x44, 0x33]);
    assert_eq!(BattlefieldEvent::read_native_cancel_error(&mut r).unwrap(), -1);
    assert_eq!(r.u16().unwrap(), 0x3344);
    assert!(BattlefieldEvent::read_native_cancel_error(&mut Reader::new(&[0])).is_err());
    assert!(BattlefieldEvent::read(0x18c, &mut Reader::new(&[0, 0])).is_err());
}

#[test]
fn successful_cancel_write_blocks_late_admission_and_queue_mutation_without_faking_clear() {
    let mut s = Battlefield::default();
    request(&mut s, 7, 2);
    fold(&mut s, BattlefieldEvent::Requested(BattlefieldCommand::Cancel));
    fold(&mut s, applied(7, 2));
    assert!(s.applied.is_none());
    let mut s = Battlefield::default();
    request(&mut s, 7, 2);
    fold(&mut s, applied(7, 2));
    fold(&mut s, queue(7));
    fold(&mut s, BattlefieldEvent::Requested(BattlefieldCommand::Cancel));
    fold(
        &mut s,
        BattlefieldEvent::QueueState(BattlefieldQueue {
            battlefield_id: 7,
            corps_size: 10,
            counts: [0; 3],
        }),
    );
    assert_eq!(s.joined_infos(), (6, 3, 2));
    fold(&mut s, BattlefieldEvent::Cancelled { raw: 0 });
    assert_eq!(s.joined_infos(), (0, 0, -1));
}


fn invite() -> BattlefieldInvitation {
    BattlefieldInvitation { zone_id: 8, instance_id: 9, ruleset_id: 10, corps: 1, qualified_id: 0x0102030405060708 }
}
fn invited_state() -> Battlefield {
    let mut s = Battlefield::default();
    request(&mut s, 7, 2);
    fold(&mut s, applied(7, 2));
    fold(&mut s, BattlefieldEvent::Invited(invite()));
    s
}
fn joined() -> BattlefieldJoined {
    BattlefieldJoined { zone_id: 8, instance_id: 9, corps: 1, victory_score: -5,
        victory_kill_count: 0x1234, victory_kill_corps_head: 0xabcdef12,
        phase_times_ms: [-1, 2000, 3000], factions: [101, 102], killstreak_skills: std::array::from_fn(|i| i as u32 + 300) }
}
#[test]
fn invitation_replies_match_native_accept_and_next_room_wire() {
    for accept in [true, false] {
        let c = invited_state().respond_to_invitation(invite(), accept).unwrap();
        let mut w = Writer::new(); c.write(&mut w);
        let mut expected = vec![u8::from(accept)];
        expected.extend_from_slice(&if accept { invite().qualified_id } else { 0 }.to_le_bytes());
        assert_eq!(w.finish(), expected);
        assert_eq!(c.opcode(), 0xe3);
        let mut bad = invite(); bad.corps = 2;
        assert!(BattlefieldCommand::RespondToInvitation { invitation: bad, accept }.validate().is_err());
    }
}
#[test]
fn joined_native_lane_fixed_body_retains_all_fields_truncations_and_dd04_tail() {
    let j = joined();
    let mut w = Writer::new();
    w.u32(j.zone_id).u32(j.instance_id).u8(j.corps).i32(j.victory_score).u16(j.victory_kill_count).u32(j.victory_kill_corps_head);
    for t in j.phase_times_ms { w.i32(t); }
    for f in j.factions { w.u32(f); }
    for skill in j.killstreak_skills { w.u32(skill); }
    let body = w.finish();
    assert_eq!(body.len(), 135);
    check(0x190, &body, BattlefieldEvent::Joined(j));
}
#[test]
fn captured_invite_replies_and_joined_snapshots_reject_stale_or_unsent_contexts() {
    let mut s = invited_state();
    for field in 0..5 {
        let mut stale = invite();
        match field { 0 => stale.zone_id+=1, 1=>stale.instance_id+=1, 2=>stale.ruleset_id+=1, 3=>stale.corps=0, _=>stale.qualified_id+=1 }
        assert!(s.respond_to_invitation(stale, true).is_err());
    }
    fold(&mut s, BattlefieldEvent::Joined(joined()));
    assert!(s.joined.is_none());
    let reply = s.respond_to_invitation(invite(), true).unwrap();
    fold(&mut s, BattlefieldEvent::RequestRejected { command: reply, message: "no lane".into() });
    assert!(s.response_pending.is_none());
    assert!(s.respond_to_invitation(invite(), true).is_ok());
    fold(&mut s, BattlefieldEvent::Requested(reply));
    assert_eq!(s.invitation, Some(invite()));
    assert!(s.respond_to_invitation(invite(), true).is_err());
    for field in 0..3 {
        let mut stale = joined();
        match field { 0=>stale.zone_id+=1, 1=>stale.instance_id+=1, _=>stale.corps=0 }
        fold(&mut s, BattlefieldEvent::Joined(stale));
        assert!(s.joined.is_none());
    }
    fold(&mut s, BattlefieldEvent::InviteTimedOut { zone_id:8,instance_id:9,ruleset_id:10,corps:1 });
    assert!(s.response_pending.is_some()); // Native accepted state4 ignores invitation timeout.
    fold(&mut s, BattlefieldEvent::Joined(joined()));
    assert_eq!(s.joined, Some(joined()));
    assert!(s.invitation.is_none());
    assert!(s.response_pending.is_none());
    assert_eq!(s.joined_infos().2,1);
}
#[test]
fn next_room_requeue_is_received_only_and_session_reset_clears_join() {
    let mut s = invited_state();
    let reply=s.respond_to_invitation(invite(),false).unwrap();
    fold(&mut s,BattlefieldEvent::Requested(reply));
    assert_eq!(s.invitation,Some(invite()));
    fold(&mut s,applied(99,2)); assert!(s.response_pending.is_some());
    fold(&mut s,applied(7,1)); assert!(s.response_pending.is_some());
    fold(&mut s,applied(7,2)); // Lane declines into same field ANY queue via real Applied.
    assert!(s.response_pending.is_none()); assert!(s.invitation.is_none());
    assert_eq!(s.applied,Some((7,2)));
    for event in [Event::CharacterSelected(Box::default()),Event::Kicked{reason:0,message:String::new()},Event::LeftWorld{target:0},Event::Disconnected{message:String::new()}] {
        let mut s=invited_state(); let reply=s.respond_to_invitation(invite(),true).unwrap();
        fold(&mut s,BattlefieldEvent::Requested(reply)); fold(&mut s,BattlefieldEvent::Joined(joined()));
        s.apply(&event); assert_eq!(s,Battlefield::default());
    }
}


#[test]
fn world_ready_requires_current_successfully_sent_acceptance_and_keeps_received_state() {
    let mut s=invited_state();
    assert!(s.world_ready(invite()).is_err());
    let decline=s.respond_to_invitation(invite(),false).unwrap();
    fold(&mut s,BattlefieldEvent::Requested(decline));
    assert!(s.world_ready(invite()).is_err());
    let mut s=invited_state();
    let accept=s.respond_to_invitation(invite(),true).unwrap();
    fold(&mut s,BattlefieldEvent::Requested(accept));
    let mut stale=invite(); stale.qualified_id+=1;
    assert!(s.world_ready(stale).is_err());
    let ready=s.world_ready(invite()).unwrap();
    let mut w=Writer::new();ready.write(&mut w);
    assert_eq!(ready.opcode(),0xe4);
    assert_eq!(w.finish(),invite().qualified_id.to_le_bytes());
    let before=s.clone();
    fold(&mut s,BattlefieldEvent::Requested(ready));
    assert_eq!(s,before); // Not a join receipt or an invented native state transition.
    assert!(s.world_ready(invite()).is_ok()); // Native remains state4; explicit retry allowed.
    fold(&mut s,BattlefieldEvent::Requested(BattlefieldCommand::Cancel));
    assert!(s.world_ready(invite()).is_err());
    fold(&mut s,BattlefieldEvent::Cancelled{raw:0});
    assert!(s.world_ready(invite()).is_err());
}


fn active_state() -> Battlefield {
    let mut s=invited_state();
    let reply=s.respond_to_invitation(invite(),true).unwrap();
    fold(&mut s,BattlefieldEvent::Requested(reply));
    fold(&mut s,BattlefieldEvent::Joined(joined())); s
}
fn start() -> BattlefieldStart { BattlefieldStart {zone_id:8,instance_id:9,now_ms:123,start_ms:20000} }
fn end() -> BattlefieldEnd {
    BattlefieldEnd {zone_id:8,instance_id:9,reason:3,teams:std::array::from_fn(|i| BattlefieldTeamResult {
        state:i as u8,faction_id:100+i as u32,score:-123,total_kills:-2,total_deaths:7,
        members:vec![BattlefieldMemberResult {character_id:77,name:"??".into(),score:-9,bonus:10,bonus_set:0xfedcba98,killstreak:2,kills:3,assists:4,deaths:5}]
    })}
}
#[test]
fn later_phase_wire_goldens_truncations_and_tail_alignment() {
    let a=start(); let mut w=Writer::new();
    w.u32(a.zone_id).u32(a.instance_id).u32(a.now_ms).u32(a.start_ms);
    check(0x191,&w.finish(),BattlefieldEvent::Started(a));
    let e=end(); let mut w=Writer::new();
    w.u32(e.zone_id).u32(e.instance_id).u8(e.reason);
    for t in &e.teams {
        w.u8(t.state).u32(t.faction_id).i32(t.score).i16(t.total_kills).i16(t.total_deaths).u8(t.members.len() as u8);
        for m in &t.members {w.u32(m.character_id).string(&m.name).i32(m.score).i32(m.bonus).u32(m.bonus_set).u16(m.killstreak).u16(m.kills).u16(m.assists).u16(m.deaths);}
    }
    check(0x192,&w.finish(),BattlefieldEvent::Ended(e));
}
#[test]
fn phases_require_matching_identity_clock_and_received_reset() {
    let mut s=active_state(); assert!(s.in_instant_game()); assert_eq!(s.joined_ruleset_id,Some(10));
    assert!(s.validate_command(BattlefieldCommand::Leave).is_err());
    fold(&mut s,BattlefieldEvent::Ended(end())); assert!(s.result.is_none()); assert_eq!(s.last_end,Some(end()));
    let mut stale=start(); stale.instance_id+=1;
    fold(&mut s,BattlefieldEvent::Started(stale)); assert!(s.start.is_none());
    fold(&mut s,BattlefieldEvent::Started(start())); assert_eq!(s.phase,Some(BattlefieldPhase::Opening));
    assert!(!s.advance_server_time(13999)); assert!(s.advance_server_time(14000));
    assert!(!s.advance_server_time(14001));
    let mut stale=end(); stale.zone_id+=1;
    fold(&mut s,BattlefieldEvent::Ended(stale)); assert!(s.result.is_none());
    fold(&mut s,BattlefieldEvent::Ended(end())); assert_eq!(s.phase,Some(BattlefieldPhase::Ending));
    assert!(s.in_instant_game()); assert!(s.validate_command(BattlefieldCommand::Leave).is_ok());
    fold(&mut s,BattlefieldEvent::Requested(BattlefieldCommand::Leave)); assert!(s.in_instant_game());
    assert_eq!(BattlefieldCommand::Leave.opcode(),0xe5);
    assert!(ClientMsg::Battlefield(BattlefieldCommand::Leave).encode_body().is_empty());
    let load=Event::LoadInstance(crate::world_msg::InstanceLoad {raw_id:1,zone_id:2,pos:Default::default(),angles:[0.;3]});
    s.apply(&load); assert_eq!(s,Battlefield::default());
    let mut accepted=invited_state(); let reply=accepted.respond_to_invitation(invite(),true).unwrap();
    fold(&mut accepted,BattlefieldEvent::Requested(reply)); accepted.apply(&load);
    assert!(accepted.world_ready(invite()).is_ok());
}
#[test]
fn lane_result_count_preserves_all_255_members_instead_of_native_inline_clamp() {
    let mut w=Writer::new(); w.u32(8).u32(9).u8(0);
    for _ in 0..2 {
        w.u8(0).u32(1).i32(0).i16(0).i16(0).u8(255);
        for id in 0..255 {w.u32(id).string("").i32(0).i32(0).u32(0).u16(0).u16(0).u16(0).u16(0);}
    }
    let body=w.finish();
    let BattlefieldEvent::Ended(e)=BattlefieldEvent::read(0x192,&mut Reader::new(&body)).unwrap() else {panic!()};
    assert_eq!(e.teams[0].members.len(),255); assert_eq!(e.teams[1].members[254].character_id,254);
    assert!(BattlefieldEvent::read(0x192,&mut Reader::new(&body[..body.len()-1])).is_err());
}


fn point() -> BattlefieldPoint { BattlefieldPoint {zone_id:8,instance_id:9,corps:1,points:-4,scores:[-30,99],character_name:"source".into()} }
fn skill_enable() -> BattlefieldSkillEnable { BattlefieldSkillEnable {zone_id:8,instance_id:9,killstreak:23,skill_id:323,enabled:true} }
#[test]
fn progress_packet_goldens_signed_index_truncations_and_compressed_tail() {
    let p=point(); let mut w=Writer::new();w.u32(8).u32(9).u8(1).i32(-4).i32(-30).i32(99).string("source");
    check(0x193,&w.finish(),BattlefieldEvent::Point(p));
    for index in [-128,-1,0,23,24,127] {let mut u=skill_enable();u.killstreak=index;let mut w=Writer::new();w.u32(8).u32(9).i8(index).u32(323).bool(true);check(0x195,&w.finish(),BattlefieldEvent::SkillEnable(u));}
}
#[test]
fn scores_are_received_totals_and_enable_requires_matching_slot_identity_phase_and_change() {
    let mut s=active_state();assert_eq!(s.scores,[0;2]);assert_eq!(s.skills_enabled,[false;24]);
    fold(&mut s,BattlefieldEvent::Point(point()));assert_eq!(s.scores,[0;2]);assert_eq!(s.last_point,Some(point()));
    fold(&mut s,BattlefieldEvent::Started(start()));
    assert!(s.accepts_point(&point()));fold(&mut s,BattlefieldEvent::Point(point()));assert_eq!(s.scores,[-30,99]);
    fold(&mut s,BattlefieldEvent::Point(point()));assert_eq!(s.scores,[-30,99]); // overwrite, never add delta
    let mut stale=point();stale.instance_id+=1;fold(&mut s,BattlefieldEvent::Point(stale));assert_eq!(s.scores,[-30,99]);
    fold(&mut s,BattlefieldEvent::SkillEnable(skill_enable()));assert!(!s.skills_enabled[23]);
    s.advance_server_time(14000);
    for field in 0..5 {let mut u=skill_enable();match field {0=>u.zone_id+=1,1=>u.instance_id+=1,2=>u.killstreak=-1,3=>u.killstreak=24,_=>u.skill_id+=1};assert!(!s.accepts_skill_change(&u));fold(&mut s,BattlefieldEvent::SkillEnable(u));assert_eq!(s.skills_enabled,[false;24]);}
    assert!(s.accepts_skill_change(&skill_enable()));fold(&mut s,BattlefieldEvent::SkillEnable(skill_enable()));assert!(s.skills_enabled[23]);assert!(!s.accepts_skill_change(&skill_enable()));
    let mut off=skill_enable();off.enabled=false;fold(&mut s,BattlefieldEvent::SkillEnable(off));assert!(!s.skills_enabled[23]);
    fold(&mut s,BattlefieldEvent::Ended(end()));let mut late=point();late.scores=[123,456];fold(&mut s,BattlefieldEvent::Point(late));assert_eq!(s.scores,[-30,99]);
    s.apply(&Event::Disconnected{message:String::new()});assert_eq!(s,Battlefield::default());
}
