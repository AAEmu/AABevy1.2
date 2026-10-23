//! Recovery regression: each newly handled packet must consume its exact body in DD01 and DD04.
//! Layouts are the lane writers, except the three native corrections documented in aa-net.md.
use aa_net::game::ServerMsg;
use aa_net::opcodes::sc;
use aa_net::wire::Writer;

fn body(write: impl FnOnce(&mut Writer)) -> Vec<u8> {
    let mut w = Writer::new();
    write(&mut w);
    w.finish()
}

fn plain(opcode: u16, bytes: &[u8]) -> Vec<u8> {
    body(|w| {
        w.raw(&[0xdd, 1, 0, 0]).u16(opcode).raw(bytes);
    })
}

fn batch(opcode: u16, bytes: &[u8]) -> Vec<u8> {
    let raw = body(|w| {
        w.u16(0)
            .u16(opcode)
            .raw(bytes)
            .u16(0)
            .u16(sc::SCOnOffSnowPacket)
            .bool(true);
    });
    let len = u16::try_from(raw.len()).unwrap();
    body(|w| {
        w.raw(&[0xdd, 4]).u16(2).u8(1).u16(len).u16(!len).raw(&raw);
    })
}

#[test]
fn recovered_packets_preserve_compressed_batch_alignment() {
    let cases = vec![
        (
            0x007,
            body(|w| {
                w.bool(true)
                    .string("auth")
                    .string("platform")
                    .string("shop");
            }),
            "SCTrionConfig",
        ),
        (
            0x009,
            body(|w| {
                w.bool(false).u8(1).u32(1).u32(2).u8(3).i64(4).i64(5).u8(6);
            }),
            "SCFactionRelationList",
        ),
        (0x048, 7u32.to_le_bytes().to_vec(), "SCLpManaged"),
        (
            0x1a1,
            body(|w| {
                w.u8(1).string("warning");
            }),
            "SCAccountWarned",
        ),
        (
            0x1cd,
            vec![0, 1, 2, 3, 4, 5, 6, 7, 8],
            "SCRefreshInCharacterList",
        ),
        (
            0x1ce,
            body(|w| {
                w.u32(7).u8(1).u8(2);
            }),
            "SCResultRestrictCheck",
        ),
        (0x215, vec![1, 0], "SCAccountAttributeConfig"),
        (0x217, (1..=20).collect(), "SCLevelRestrictionConfig"),
        (
            0x020,
            body(|w| {
                w.u16(1).i32(7);
            }),
            "SCDominionTaxRate",
        ),
        (
            0x0d0,
            body(|w| {
                w.u8(1).string("FF").i32(3000).string("FF0000Notice");
            }),
            "SCNoticeMessage",
        ),
        (
            0x0ec,
            body(|w| {
                w.u16(1).u32(2).u32(3).bool(true);
            }),
            "SCSiegeMember",
        ),
        (
            0x0ed,
            body(|w| {
                w.u16(1).u8(2).u32(3).string("name");
            }),
            "SCSiegeAlert",
        ),
        (
            0x0ed,
            body(|w| {
                w.u16(1).u8(2).u32(0);
            }),
            "SCSiegeAlert",
        ),
        (
            0x0ef,
            body(|w| {
                w.u16(1).i32(123);
            }),
            "SCConflictZoneHonorPointSum",
        ),
        (
            0x16b,
            body(|w| {
                w.u8(1)
                    .u32(2)
                    .u16(3)
                    .u32(4)
                    .u32(5)
                    .bc(6)
                    .i64(7)
                    .i64(8)
                    .f32(9.0)
                    .u32(10);
            }),
            "SCTowerDefList",
        ),
        (
            0x16c,
            body(|w| {
                w.u32(1).u16(2).u32(3);
            }),
            "SCTowerDefStart",
        ),
        (
            0x16d,
            body(|w| {
                w.u32(1).u16(2).u32(3);
            }),
            "SCTowerDefEnd",
        ),
        (
            0x16e,
            body(|w| {
                w.u32(1).u16(2).u32(3).u32(4);
            }),
            "SCTowerDefWaveStart",
        ),
        (
            0x170,
            body(|w| {
                w.bc(0xabcdef).string("Thief").string("Guard");
            }),
            "SCCriminalArrested",
        ),
        (
            0x184,
            body(|w| {
                w.string("Reporter").string("Suspect");
            }),
            "SCBotSuspectReported",
        ),
        (0x198, 7i32.to_le_bytes().to_vec(), "SCProcessingInstance"),
        (
            0x1a7,
            body(|w| {
                w.u32(1).u64(u64::MAX);
            }),
            "SCTutorialSaved",
        ),
        (
            0x1de,
            body(|w| {
                w.u8(0).u32(99);
            }),
            "SCPlaySound",
        ),
        (
            0x1de,
            body(|w| {
                w.u8(1).u32(99).string("voice");
            }),
            "SCPlaySound",
        ),
        (
            0x1de,
            body(|w| {
                w.u8(2).u32(99).bc(0xabcdef);
            }),
            "SCPlaySound",
        ),
        (
            0x1df,
            body(|w| {
                w.i32(3).bc(1).string("Bard").blob(&[2, 3, 4]);
            }),
            "SCSendUserMusic",
        ),
        (
            0x1e0,
            body(|w| {
                w.u32(1).bool(true).i32(3).string("Song").string("abc");
            }),
            "SCUserNoteLoaded",
        ),
        (0x1e1, vec![1, 2, 3], "SCPauseUserMusic"),
        (
            0x1f4,
            body(|w| {
                w.i32(1).u16(2).u16(3).u32(4).i32(95);
            }),
            "SCSpecialtyCurrent",
        ),
        (0x06f, vec![1, 2, 3, 1], "SCActiveWeaponChanged"),
        (
            0x079,
            body(|w| {
                w.bc(1).u8(0).bc(1);
                for n in 0..12 {
                    w.f32(n as f32);
                }
            }),
            "SCUnitImpulse",
        ),
        (
            0x1cb,
            body(|w| {
                w.bc(1).u8(31).raw(&[1, 2, 3, 4, 5, 6, 1, 0, 1, 0]);
            }),
            "SCUnitVisualOptions",
        ),
        (0x1cb, vec![1, 0, 0, 0], "SCUnitVisualOptions"),
        (0x1f5, vec![1, 2, 3, 1], "SCUnitIdleStatus"),
        (
            0x1cf,
            body(|w| {
                w.bool(true);
                for n in 1..=6 {
                    w.u8(n).raw(&[1, 0, 3, 0, 0, 0, 0]);
                }
            }),
            "SCICSMenuList",
        ),
        (
            0x1d0,
            body(|w| {
                w.bool(true)
                    .u16(1)
                    .u32(2)
                    .string("Goods")
                    .raw(&[1, 2, 3, 4])
                    .u32(5)
                    .bool(true)
                    .bool(false)
                    .u8(0)
                    .u16(0)
                    .u8(0)
                    .u32(0)
                    .i64(0)
                    .i64(0)
                    .u8(0)
                    .u32(6)
                    .i32(-1)
                    .u32(7)
                    .u32(8)
                    .u8(9);
            }),
            "SCICSGoodList",
        ),
        (
            0x1d1,
            body(|w| {
                w.bool(true)
                    .u32(1)
                    .u32(2)
                    .u32(3)
                    .u32(4)
                    .u8(5)
                    .bool(true)
                    .u8(0)
                    .i64(0)
                    .u8(0)
                    .u32(6)
                    .u32(7)
                    .u32(8)
                    .u32(9);
            }),
            "SCICSGoodDetail",
        ),
        (0x1d2, vec![], "SCICSCheckTime"),
        (
            0x1d3,
            body(|w| {
                w.bool(true).u8(1).string("Buyer").i32(2);
            }),
            "SCICSBuyResult",
        ),
        (
            0x1d4,
            body(|w| {
                w.i32(1).i32(-1);
            }),
            "SCICSSyncGood",
        ),
        (
            0x1d8,
            body(|w| {
                w.bool(true)
                    .u8(1)
                    .i32(1)
                    .string("Premium")
                    .u16(2)
                    .u8(0)
                    .u8(0)
                    .i32(720)
                    .u8(0)
                    .i32(300)
                    .u32(0)
                    .i32(0)
                    .i32(100);
            }),
            "SCPremiumServiceList",
        ),
        (
            0x096,
            body(|w| {
                w.bc(1).u64(2).u8(2).u8(0).u32(0).u8(30).u32(0);
            }),
            "SCCofferContentsUpdate",
        ),
        (0x081, vec![1, 2, 3, 4, 5, 6], "SCVegetationCutdowning"),
        (
            0x134,
            body(|w| {
                w.string("Bob").i32(100).i32(42);
            }),
            "SCDiceValue",
        ),
        (
            0x058,
            body(|w| {
                w.u32(7).string("Buyer").u8(5);
            }),
            "SCCharBrief",
        ),
        (
            0x0d3,
            body(|w| {
                w.u8(1).raw(&[2; 15]);
                for n in 0..15 {
                    w.f32(n as f32);
                }
                w.string("apply").string("detect");
            }),
            "SCChatSpamDelay",
        ),
    ];
    let mut opcodes = std::collections::BTreeSet::new();
    for (opcode, bytes, name) in cases {
        opcodes.insert(opcode);
        let message = ServerMsg::decode(&plain(opcode, &bytes)).unwrap();
        assert_eq!(message.name(), name, "opcode {opcode:#x}");
        assert!(!matches!(message, ServerMsg::Unhandled { .. }));
        assert_eq!(
            ServerMsg::decode(&batch(opcode, &bytes)).unwrap(),
            ServerMsg::Batch(vec![message, ServerMsg::Snow(true)]),
            "{name}: body consumption"
        );
        for cut in 0..bytes.len() {
            if opcode == 0x1e0 && cut == 0 {
                continue;
            } // native empty unknown-song reply
            assert!(
                ServerMsg::decode(&plain(opcode, &bytes[..cut])).is_err(),
                "{name} cut {cut}"
            );
        }
    }
    assert_eq!(opcodes.len(), 42);
}

#[test]
fn new_scoped_caches_reset_without_resetting_lobby_data_on_spawn() {
    use aa_net::{AccountEvent, Event, HousingEvent, Phase, WorldState};
    let mut state = WorldState::default();
    let brief = Event::Housing(HousingEvent::CharacterBrief {
        character_id: 7,
        name: "Buyer".into(),
        race: 5,
    });
    assert!(state.houses.apply(&brief));
    assert!(!state.houses.apply(&brief));
    assert_eq!(
        state.houses.character_briefs.get(&7),
        Some(&("Buyer".into(), 5))
    );
    let spam = Event::ChatSpamDelay(aa_net::ChatSpamDelay::default());
    assert!(state.chat.apply(&spam));
    assert!(!state.chat.apply(&spam));
    state.apply(&Event::Account(AccountEvent::LpManaged { character_id: 7 }));
    state.apply(&Event::Account(AccountEvent::CharacterListRefreshed {
        race_congestion: [2; 9],
    }));
    state.apply(&Event::CharacterSelected(Box::default()));
    assert_eq!(state.characters.lp_managed, Some(7));
    assert_eq!(state.characters.race_congestion, [2; 9]);
    assert!(state.houses.character_briefs.is_empty());
    state.apply(&spam);
    state.apply(&brief);
    state.apply(&Event::LeftWorld { target: 1 });
    assert!(state.chat.spam_delay.is_none());
    assert!(state.houses.character_briefs.is_empty());
    assert_eq!(state.characters.lp_managed, Some(7));
    state.apply(&Event::PhaseChanged(Phase::Authenticating));
    assert_eq!(state.characters.lp_managed, None);
    assert_eq!(state.characters.race_congestion, [0; 9]);
    state.apply(&spam);
    state.apply(&brief);
    state.apply(&Event::Disconnected {
        message: "lost".into(),
    });
    assert!(state.chat.spam_delay.is_none());
    assert!(state.houses.character_briefs.is_empty());
}

#[test]
fn proxy_level_descriptor_preserves_wire_values_and_unknown_song_keeps_batch_tail() {
    let descriptor = body(|w| {
        w.string("main_world").u64(u64::MAX).u8(0x80);
    });
    let frame = body(|w| {
        w.raw(&[0xdd, 2, 0x0f, 0]).raw(&descriptor);
    });
    assert_eq!(
        ServerMsg::decode(&frame).unwrap(),
        ServerMsg::GameType(aa_net::proxy::GameType {
            level: "main_world".into(),
            checksum: u64::MAX,
            immersive: 0x80,
        })
    );
    for cut in 4..frame.len() {
        assert!(
            ServerMsg::decode(&frame[..cut]).is_err(),
            "proxy descriptor cut {cut}"
        );
    }
    assert_eq!(
        ServerMsg::decode(&plain(0x1e0, &[])).unwrap(),
        ServerMsg::Notice(aa_net::NoticeEvent::UserNoteLoaded(None))
    );
    // The lane's empty unknown-song reply is self-delimiting only in a standalone frame.
    // An empty body before another DD04 entry is ambiguous: report it, preserve the tail.
    let ServerMsg::Batch(messages) = ServerMsg::decode(&batch(0x1e0, &[])).unwrap() else {
        panic!()
    };
    assert!(matches!(
        messages.as_slice(),
        [ServerMsg::DecodeFailed { .. }, ServerMsg::Snow(true)]
    ));
}
