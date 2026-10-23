use super::*;
use crate::game::{ClientMsg, ServerMsg};
use crate::notice::{NoticeEvent, UserNote};
use crate::wire::Reader;
use std::io::Write;

#[path = "../../tests/fixtures/music/lane.rs"]
pub(crate) mod lane;
use lane::Marshal;

pub(crate) const ITEM: u64 = 0x0506070801020304;

pub(crate) fn upload() -> MusicCommand {
    MusicCommand::UploadNotes {
        item_id: ITEM,
        slot_type: container::INVENTORY,
        slot: 255,
        title: "한글".into(),
        notes: "C 도😀".into(),
    }
}

pub(crate) fn fixture(name: &str) -> (u16, Vec<u8>) {
    let line = include_str!("../../tests/fixtures/music/native.tsv")
        .lines()
        .find(|line| line.split('\t').next() == Some(name))
        .unwrap();
    let mut fields = line.split('\t');
    fields.next();
    let opcode = u16::from_str_radix(fields.next().unwrap(), 16).unwrap();
    let hex = fields.next().unwrap();
    let bytes = (0..hex.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&hex[at..at + 2], 16).unwrap())
        .collect();
    (opcode, bytes)
}

pub(crate) fn dd01(opcode: u16, body: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0xdd, 1, 0, 0];
    bytes.extend(opcode.to_le_bytes());
    bytes.extend(body);
    bytes
}

pub(crate) fn dd04(packets: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let mut raw = Vec::new();
    for (opcode, body) in packets {
        raw.extend([0, 0]);
        raw.extend(opcode.to_le_bytes());
        raw.extend(body);
    }
    let mut compressor =
        flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
    compressor.write_all(&raw).unwrap();
    let mut bytes = vec![0xdd, 4];
    bytes.extend((packets.len() as u16).to_le_bytes());
    bytes.extend(compressor.finish().unwrap());
    bytes
}

pub(crate) fn lane_body(packet: &impl Marshal) -> Vec<u8> {
    let mut stream = lane::PacketStream::new();
    packet.write(&mut stream);
    stream.into_bytes()
}

pub(crate) fn server_music(midi: Vec<u8>) -> Vec<u8> {
    lane_body(&lane::SCSendUserMusicPacket {
        obj_id: 0x030201,
        author: "한글".into(),
        midi,
    })
}

pub(crate) fn server_note() -> Vec<u8> {
    lane_body(&lane::SCUserNoteLoadedPacket {
        song: Some(lane::SongData {
            id: 7,
            title: "한글".into(),
            song: "C 도😀".into(),
        }),
    })
}

#[test]
fn writers_match_executed_original_constructors_and_serializers() {
    let mut ascii = upload();
    if let MusicCommand::UploadNotes {
        slot, title, notes, ..
    } = &mut ascii
    {
        *slot = 0;
        *title = "Song".into();
        *notes = "T120 C".into();
    }
    let mut cases = vec![
        ("upload_ascii", ascii),
        ("upload_unicode", upload()),
        (
            "midi_bytes",
            MusicCommand::CacheMidi {
                midi: b"MThd\0\xff".to_vec(),
            },
        ),
        ("end", MusicCommand::End),
    ];
    for (name, is_tooltip, show_buff_only) in [
        ("request_00", false, false),
        ("request_10", true, false),
        ("request_01", false, true),
        ("request_11", true, true),
    ] {
        cases.push((
            name,
            MusicCommand::RequestNotes {
                note_id: ITEM,
                is_tooltip,
                show_buff_only,
            },
        ));
    }
    for (name, command) in cases {
        command.validate().unwrap();
        let (opcode, expected) = fixture(name);
        let msg = ClientMsg::Music(command);
        assert_eq!(msg.opcode(), opcode, "{name}");
        assert_ne!(msg.name(), "?");
        assert_eq!(msg.encode_body(), expected, "{name}");
        let mut frame = vec![0xdd, 1, 0, 37];
        frame.extend(opcode.to_le_bytes());
        frame.extend(expected);
        assert_eq!(msg.encode(37), frame, "{name}");
    }
    let midi = MusicCommand::CacheMidi {
        midi: vec![0xa5; MAX_MIDI_BYTES],
    };
    assert!(midi.validate().is_ok());
    assert_eq!(
        ClientMsg::Music(midi).encode_body(),
        include_bytes!("../../tests/fixtures/music/native-midi-max.body")
    );
    let upload = MusicCommand::UploadNotes {
        item_id: 1,
        slot_type: 2,
        slot: 255,
        title: "T".repeat(96),
        notes: "C".repeat(12000),
    };
    assert!(upload.validate().is_ok());
    assert_eq!(
        ClientMsg::Music(upload).encode_body(),
        include_bytes!("../../tests/fixtures/music/native-upload-max.body")
    );
}

#[test]
fn original_fixtures_pass_actual_lane_request_readers_with_no_tail() {
    let bytes = ClientMsg::Music(upload()).encode_body();
    let mut stream = lane::PacketStream::from_bytes(bytes.clone());
    let saved = lane::CSSaveUserMusicNotesPacket::read(&mut stream).unwrap();
    assert_eq!(
        (saved.song_size, saved.item_id, saved.value2),
        (9, ITEM, 0xff02)
    );
    assert_eq!(
        (saved.title.as_str(), saved.song.as_str()),
        ("한글", "C 도😀")
    );
    assert_eq!(stream.remaining(), 0);
    assert_eq!(&bytes[..4], &9u32.to_le_bytes()); // UTF-8 bytes; only 4 Unicode scalars.
    assert_eq!(&bytes[12..14], &[2, 255]); // wire bag2, physical slot255, no +1.
    assert_eq!(&bytes[14..22], &[6, 0, 0xed, 0x95, 0x9c, 0xea, 0xb8, 0x80]);
    assert_eq!(&bytes[22..24], &[10, 0]); // note blob includes NUL, outer count excludes it.
    assert_eq!(bytes.last(), Some(&0));
    for n in 0..bytes.len() {
        assert!(
            lane::CSSaveUserMusicNotesPacket::read(&mut lane::PacketStream::from_bytes(
                bytes[..n].to_vec()
            ))
            .is_err()
        );
    }
    for (name, expected) in [
        ("request_00", 0),
        ("request_10", 1),
        ("request_01", 256),
        ("request_11", 257),
    ] {
        let bytes = fixture(name).1;
        let mut stream = lane::PacketStream::from_bytes(bytes.clone());
        let request = lane::CSRequestMusicNotesPacket::read(&mut stream).unwrap();
        assert_eq!(
            (request.id64, request.value2, stream.remaining()),
            (ITEM, expected, 0)
        );
        for n in 0..bytes.len() {
            assert!(
                lane::CSRequestMusicNotesPacket::read(&mut lane::PacketStream::from_bytes(
                    bytes[..n].to_vec()
                ))
                .is_err()
            );
        }
    }
    for bytes in [
        fixture("midi_bytes").1,
        include_bytes!("../../tests/fixtures/music/native-midi-max.body").to_vec(),
    ] {
        let mut stream = lane::PacketStream::from_bytes(bytes.clone());
        let request = lane::CSSendUserMusicPacket::read(&mut stream).unwrap();
        assert_eq!(request.song_size as usize, request.data.len());
        assert_eq!(&request.data, &bytes[6..]);
        assert_eq!(stream.remaining(), 0);
        for n in 0..bytes.len() {
            assert!(
                lane::CSSendUserMusicPacket::read(&mut lane::PacketStream::from_bytes(
                    bytes[..n].to_vec()
                ))
                .is_err()
            );
        }
    }
    lane::CSEndMusicPacket::read(&mut lane::PacketStream::from_bytes(Vec::new())).unwrap();
}

#[test]
fn upload_and_midi_bounds_reject_empty_nul_and_oversize_without_truncation() {
    for bytes in [vec![], vec![0; MAX_MIDI_BYTES + 1]] {
        assert!(MusicCommand::CacheMidi { midi: bytes }.validate().is_err());
    }
    assert!(MusicCommand::CacheMidi { midi: vec![0] }.validate().is_ok()); // transport is not a SMF parser.
    for (title, notes) in [
        ("".into(), "C".into()),
        ("T".into(), "".into()),
        ("T\0x".into(), "C".into()),
        ("T".into(), "C\0D".into()),
        ("한".repeat(33), "C".into()),
        ("T".into(), "도".repeat(4001)),
    ] {
        let request = MusicCommand::UploadNotes {
            item_id: ITEM,
            slot_type: 2,
            slot: 255,
            title,
            notes,
        };
        assert!(request.validate().is_err());
    }
    for (item_id, slot_type) in [(0, 2), (1, 1), (1, 3), (1, 0), (1, 255)] {
        assert!(
            MusicCommand::UploadNotes {
                item_id,
                slot_type,
                slot: 0,
                title: "T".into(),
                notes: "C".into()
            }
            .validate()
            .is_err()
        );
    }
    // Native lower-level serializers permit empty and truncate C strings; the Lua save path
    // rejects empty input. Our typed upload rejects embedded NUL instead of changing the text.
    assert_eq!(fixture("upload_empty_native").1.last(), Some(&0));
    assert_eq!(fixture("upload_cstring_native").1.last(), Some(&0));
    assert_eq!(fixture("midi_empty_native").1, [0; 6]);
    for note_id in [0, u64::MAX] {
        assert!(
            MusicCommand::RequestNotes {
                note_id,
                is_tooltip: false,
                show_buff_only: false
            }
            .validate()
            .is_ok()
        );
    }
}

#[test]
fn actual_lane_writers_decode_exact_lengths_and_compressed_following_packets() {
    for midi in [vec![], b"MThd\0\xff".to_vec(), vec![0xa5; MAX_MIDI_BYTES]] {
        let body = server_music(midi.clone());
        let expected = NoticeEvent::UserMusic {
            obj_id: 0x030201,
            author: "한글".into(),
            midi,
        };
        let mut reader = Reader::new(&body);
        assert_eq!(NoticeEvent::read(0x1df, &mut reader).unwrap(), expected);
        assert!(reader.is_empty());
        assert_eq!(
            ServerMsg::decode(&dd01(0x1df, &body)).unwrap(),
            ServerMsg::Notice(expected.clone())
        );
        for n in 0..body.len() {
            assert!(
                ServerMsg::decode(&dd01(0x1df, &body[..n])).is_err(),
                "truncated MIDI at {n}"
            );
        }
        let frame = dd04(&[
            (0xb4, vec![1]),
            (0x1df, body),
            (0x1e1, vec![1, 2, 3]),
            (0xb4, vec![0]),
        ]);
        assert_eq!(
            ServerMsg::decode(&frame).unwrap(),
            ServerMsg::Batch(vec![
                ServerMsg::Snow(true),
                ServerMsg::Notice(expected),
                ServerMsg::Notice(NoticeEvent::UserMusicPaused { obj_id: 0x030201 }),
                ServerMsg::Snow(false),
            ])
        );
    }
    let note = server_note();
    assert_eq!(&note[5..9], &5i32.to_le_bytes()); // lane UTF-16 units, not UTF-8 byte count9.
    let expected = ServerMsg::Notice(NoticeEvent::UserNoteLoaded(Some(UserNote {
        song_id: 7,
        tooltip: true,
        title: "한글".into(),
        notes: "C 도😀".into(),
    })));
    assert_eq!(ServerMsg::decode(&dd01(0x1e0, &note)).unwrap(), expected);
    for n in 1..note.len() {
        assert!(
            ServerMsg::decode(&dd01(0x1e0, &note[..n])).is_err(),
            "truncated notes at {n}"
        );
    }
    assert_eq!(
        ServerMsg::decode(&dd04(&[(0x1e0, note), (0xb4, vec![1])])).unwrap(),
        ServerMsg::Batch(vec![expected, ServerMsg::Snow(true)])
    );
    assert_eq!(
        ServerMsg::decode(&dd01(
            0x1e0,
            &lane_body(&lane::SCUserNoteLoadedPacket { song: None })
        ))
        .unwrap(),
        ServerMsg::Notice(NoticeEvent::UserNoteLoaded(None))
    );
}

#[test]
fn malformed_music_counts_fail_and_empty_unknown_note_batch_does_not_invent_a_note() {
    for (outer, inner) in [(32769, 0), (u32::MAX, 0), (0, 1), (1, 0), (1, 2)] {
        let mut body = server_music(vec![]);
        body[..4].copy_from_slice(&outer.to_le_bytes());
        let at = body.len() - 2;
        body[at..].copy_from_slice(&(inner as u16).to_le_bytes());
        body.extend([0, 0]);
        assert!(ServerMsg::decode(&dd01(0x1df, &body)).is_err());
        let ServerMsg::Batch(batch) =
            ServerMsg::decode(&dd04(&[(0x1df, body), (0xb4, vec![1])])).unwrap()
        else {
            panic!()
        };
        assert!(matches!(
            batch.as_slice(),
            [ServerMsg::DecodeFailed { .. }, ServerMsg::Snow(true)]
        ));
    }
    // The lane's unknown-song reply is empty. DD04 has no per-packet length, so a
    // nonfinal empty SC1e0 cannot be distinguished safely from a truncated full reply.
    let ServerMsg::Batch(batch) =
        ServerMsg::decode(&dd04(&[(0x1e0, vec![]), (0xb4, vec![1])])).unwrap()
    else {
        panic!()
    };
    assert!(matches!(
        batch.as_slice(),
        [ServerMsg::DecodeFailed { .. }, ServerMsg::Snow(true)]
    ));
}
