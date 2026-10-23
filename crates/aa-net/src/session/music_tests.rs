use super::*;
use crate::music::tests::{
    ITEM, dd01, dd04, fixture, lane, lane_body, server_music, server_note, upload,
};
use crate::{MusicCommand, NoticeEvent};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

fn active() -> (Session, UnboundedReceiver<Event>) {
    let (evt, events) = unbounded_channel();
    let mut s = Session::new(evt, ClientOptions::default());
    s.phase = Phase::InWorld;
    s.selected = 42;
    s.self_obj = 7;
    let mut header = crate::model::CharacterState::default();
    header.character.id = 42;
    s.selected_state = Some(header);
    (s, events)
}

async fn peer(s: &mut Session) -> TcpStream {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let (conn, accepted) = tokio::join!(GameConn::connect(&address), listener.accept());
    s.game = Some(conn.unwrap());
    accepted.unwrap().0
}

async fn read_frame(peer: &mut TcpStream) -> Vec<u8> {
    tokio::time::timeout(Duration::from_secs(2), async {
        let length = peer.read_u16_le().await.unwrap();
        let mut bytes = vec![0; length as usize];
        peer.read_exact(&mut bytes).await.unwrap();
        bytes
    })
    .await
    .unwrap()
}

fn commands() -> Vec<(&'static str, MusicCommand)> {
    vec![
        ("upload_unicode", upload()),
        (
            "midi_bytes",
            MusicCommand::CacheMidi {
                midi: b"MThd\0\xff".to_vec(),
            },
        ),
        (
            "request_01",
            MusicCommand::RequestNotes {
                note_id: ITEM,
                is_tooltip: false,
                show_buff_only: true,
            },
        ),
        ("end", MusicCommand::End),
    ]
}

#[tokio::test]
async fn music_commands_forward_original_frames_without_synthetic_replies_or_playback() {
    let (mut s, mut events) = active();
    let mut socket = peer(&mut s).await;
    for (counter, (name, command)) in commands().into_iter().enumerate() {
        s.command(Command::Music {
            character_id: 42,
            command: command.clone(),
        })
        .await;
        let (opcode, body) = fixture(name);
        let mut expected = dd01(opcode, &body);
        expected[3] = counter as u8;
        let bytes = read_frame(&mut socket).await;
        assert_eq!(bytes, expected);
        assert!(
            matches!(events.try_recv(), Ok(Event::MusicRequested { character_id:42, command:actual }) if actual == command)
        );
        assert!(
            events.try_recv().is_err(),
            "upload/cache/end never synthesize native notices or item/skill receipts"
        );
        assert_eq!((s.selected, s.self_obj, s.phase), (42, 7, Phase::InWorld));
        // Feed the actual forwarded bodies into the lane reader implementations.
        let mut reader = lane::PacketStream::from_bytes(bytes[6..].to_vec());
        match command {
            MusicCommand::UploadNotes { .. } => {
                let request = lane::CSSaveUserMusicNotesPacket::read(&mut reader).unwrap();
                assert_eq!(
                    (request.item_id, request.value2, request.song_size),
                    (ITEM, 0xff02, 9)
                );
                assert_eq!(request.song, "C 도😀");
            }
            MusicCommand::CacheMidi { midi } => {
                assert_eq!(
                    lane::CSSendUserMusicPacket::read(&mut reader).unwrap().data,
                    midi
                );
            }
            MusicCommand::RequestNotes { .. } => {
                let request = lane::CSRequestMusicNotesPacket::read(&mut reader).unwrap();
                assert_eq!((request.id64, request.value2), (ITEM, 256));
            }
            MusicCommand::End => {
                lane::CSEndMusicPacket::read(&mut reader).unwrap();
            }
        }
        assert_eq!(reader.remaining(), 0);
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(20), socket.read_u8())
            .await
            .is_err(),
        "no automatic skill, pause, or End request"
    );
}

#[tokio::test]
async fn every_music_variant_obeys_phase_character_header_actor_and_socket_guards() {
    let (mut s, mut events) = active();
    let mut socket = peer(&mut s).await;
    for (_, command) in commands() {
        for phase in [
            Phase::Idle,
            Phase::CharacterSelect,
            Phase::EnteringWorld,
            Phase::Leaving,
        ] {
            s.phase = phase;
            s.command(Command::Music {
                character_id: 42,
                command: command.clone(),
            })
            .await;
            assert!(matches!(
                events.try_recv(),
                Ok(Event::MusicRequestRejected { .. })
            ));
        }
        s.phase = Phase::InWorld;
        for character_id in [0, 41] {
            s.command(Command::Music {
                character_id,
                command: command.clone(),
            })
            .await;
            assert!(matches!(
                events.try_recv(),
                Ok(Event::MusicRequestRejected { .. })
            ));
        }
        for bad in 0..4 {
            match bad {
                0 => s.selected = 0,
                1 => s.self_obj = 0,
                2 => s.selected_state = None,
                _ => {
                    let mut header = crate::model::CharacterState::default();
                    header.character.id = 41;
                    s.selected_state = Some(header);
                }
            }
            s.command(Command::Music {
                character_id: 42,
                command: command.clone(),
            })
            .await;
            assert!(matches!(
                events.try_recv(),
                Ok(Event::MusicRequestRejected { .. })
            ));
            s.selected = 42;
            s.self_obj = 7;
            let mut header = crate::model::CharacterState::default();
            header.character.id = 42;
            s.selected_state = Some(header);
        }
    }
    for command in [
        MusicCommand::CacheMidi { midi: vec![] },
        MusicCommand::CacheMidi {
            midi: vec![0; 32769],
        },
        MusicCommand::UploadNotes {
            item_id: 1,
            slot_type: 3,
            slot: 0,
            title: "T".into(),
            notes: "C".into(),
        },
        MusicCommand::UploadNotes {
            item_id: 1,
            slot_type: 2,
            slot: 0,
            title: "T".into(),
            notes: "C\0D".into(),
        },
    ] {
        s.command(Command::Music {
            character_id: 42,
            command,
        })
        .await;
        assert!(matches!(
            events.try_recv(),
            Ok(Event::MusicRequestRejected { .. })
        ));
    }
    assert!(events.try_recv().is_err());
    assert!(
        tokio::time::timeout(Duration::from_millis(20), socket.read_u8())
            .await
            .is_err()
    );
    for (_, command) in commands() {
        for closed in [false, true] {
            let (mut fresh, mut events) = active();
            if closed {
                let _socket = peer(&mut fresh).await;
                fresh.game.as_mut().unwrap().shutdown().await;
            }
            fresh
                .command(Command::Music {
                    character_id: 42,
                    command: command.clone(),
                })
                .await;
            let mut rejected = false;
            while let Ok(event) = events.try_recv() {
                assert!(!matches!(
                    event,
                    Event::MusicRequested { .. } | Event::Notice(_)
                ));
                rejected |= matches!(event, Event::MusicRequestRejected { .. });
            }
            assert!(rejected, "missing/closed socket must not claim a write");
        }
    }
}

#[tokio::test]
async fn scripted_lane_music_writers_reach_session_events_in_order_and_keep_batch_alignment() {
    let (mut s, mut events) = active();
    let mut socket = peer(&mut s).await;
    let midi = vec![0xa5; 32768];
    let pause = lane_body(&lane::SCPauseUserMusicPacket { obj_id: 0x030201 });
    let frames = [
        dd01(0x1e0, &server_note()),
        dd04(&[
            (0xb4, vec![1]),
            (0x1df, server_music(midi.clone())),
            (0x1e1, pause.clone()),
            (0x1e1, pause),
            (0xb4, vec![0]),
        ]),
        dd01(
            0x1e0,
            &lane_body(&lane::SCUserNoteLoadedPacket { song: None }),
        ),
    ];
    for frame in frames {
        socket.write_u16_le(frame.len() as u16).await.unwrap();
        socket.write_all(&frame).await.unwrap();
        let message = tokio::time::timeout(Duration::from_secs(2), s.game.as_mut().unwrap().recv())
            .await
            .unwrap();
        s.game_msg(message).await;
    }
    assert!(
        matches!(events.try_recv(), Ok(Event::Notice(NoticeEvent::UserNoteLoaded(Some(note)))) if note.song_id==7 && note.notes=="C 도😀")
    );
    assert!(matches!(events.try_recv(), Ok(Event::Snow(true))));
    assert!(
        matches!(events.try_recv(), Ok(Event::Notice(NoticeEvent::UserMusic { obj_id:0x030201, author, midi:bytes })) if author=="한글" && bytes==midi)
    );
    for _ in 0..2 {
        assert!(matches!(
            events.try_recv(),
            Ok(Event::Notice(NoticeEvent::UserMusicPaused {
                obj_id: 0x030201
            }))
        ));
    }
    assert!(matches!(events.try_recv(), Ok(Event::Snow(false))));
    assert!(matches!(
        events.try_recv(),
        Ok(Event::Notice(NoticeEvent::UserNoteLoaded(None)))
    ));
    assert!(events.try_recv().is_err());
    assert_eq!((s.selected, s.self_obj, s.phase), (42, 7, Phase::InWorld));
    assert!(
        tokio::time::timeout(Duration::from_millis(20), socket.read_u8())
            .await
            .is_err(),
        "no automatic playback completion"
    );
}
