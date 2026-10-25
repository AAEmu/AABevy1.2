use super::*;
use crate::game::{ClientMsg, LEVEL_GAME};

#[test]
fn hanging_native_fixed_requests_match_original_archive_outputs_and_all_counters() {
    let mut count = 0;
    for line in include_str!("../../../tests/fixtures/hanging-outbound/native.tsv")
        .lines()
        .filter(|line| !line.starts_with('#'))
    {
        let columns: Vec<_> = line.split('\t').collect();
        assert_eq!(columns.len(), 6);
        let opcode = u16::from_str_radix(columns[1], 16).unwrap();
        let body: Vec<_> = (0..columns[2].len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&columns[2][i..i + 2], 16).unwrap())
            .collect();
        let actor = columns[3].parse().unwrap();
        let target = columns[4].parse().unwrap();
        let reason = columns[5].parse().unwrap();
        let request = match opcode {
            0x0cb => HangingRequest::hang(actor, target).unwrap(),
            0x0cc => HangingRequest::unhang(actor, reason).unwrap(),
            _ => panic!("unexpected hanging fixture opcode"),
        };
        assert_eq!(request.unit_obj_id(), actor);
        assert_eq!(request.target_obj_id(), (opcode == 0x0cb).then_some(target));
        assert_eq!(request.reason(), (opcode == 0x0cc).then_some(reason));
        let message = ClientMsg::Hanging(request);
        assert_eq!(message.level(), LEVEL_GAME);
        assert_eq!(message.opcode(), opcode);
        assert_eq!(
            message.name(),
            if opcode == 0x0cb {
                "CSHangPacket"
            } else {
                "CSUnhangPacket"
            }
        );
        assert_eq!(message.encode_body(), body, "{}", columns[0]);
        assert_eq!(body.len(), if opcode == 0x0cb { 6 } else { 7 });
        for counter in 0..=255 {
            let mut expected = vec![0xdd, 1, 0, counter];
            expected.extend(opcode.to_le_bytes());
            expected.extend_from_slice(&body);
            assert_eq!(message.encode(counter), expected, "{}", columns[0]);
        }
        count += 1;
    }
    assert_eq!(count, 33);
}

#[test]
fn hanging_checked_construction_rejects_zero_and_unrepresentable_ids_without_truncating() {
    for bad in [0, 0x0100_0000, 0x0100_0001, 0xff11_2233, u32::MAX] {
        assert!(HangingRequest::hang(bad, 1).is_err());
        assert!(HangingRequest::hang(1, bad).is_err());
        assert!(HangingRequest::unhang(bad, 0).is_err());
    }
    // Equal representable IDs are not a packet-format error. Native admission
    // remains outside this codec, including whether this target can be used.
    for id in [1, 0x112233, 0x00ff_ffff] {
        assert!(HangingRequest::hang(id, id).is_ok());
        for reason in [i32::MIN, -1, 0, 1, i32::MAX] {
            assert_eq!(
                HangingRequest::unhang(id, reason).unwrap().reason(),
                Some(reason)
            );
        }
    }
}

#[tokio::test]
async fn hanging_game_connection_writes_exact_length_prefixed_frames_without_a_reply() {
    use crate::net::GameConn;
    use std::time::Duration;
    use tokio::{io::AsyncReadExt, net::TcpListener};

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let (connection, accepted) = tokio::join!(GameConn::connect(&address), listener.accept());
    let mut connection = connection.unwrap();
    let mut socket = accepted.unwrap().0;
    let cases: [(HangingRequest, &[u8]); 2] = [
        (
            HangingRequest::hang(0x112233, 0xaabbcc).unwrap(),
            &[
                0x0c, 0, 0xdd, 1, 0, 0, 0xcb, 0, 0x33, 0x22, 0x11, 0xcc, 0xbb, 0xaa,
            ],
        ),
        (
            HangingRequest::unhang(0xffffff, i32::MIN).unwrap(),
            &[
                0x0d, 0, 0xdd, 1, 0, 1, 0xcc, 0, 0xff, 0xff, 0xff, 0, 0, 0, 0x80,
            ],
        ),
    ];
    for (request, expected) in cases {
        connection.send(&ClientMsg::Hanging(request)).await.unwrap();
        let mut bytes = vec![0; expected.len()];
        tokio::time::timeout(Duration::from_secs(2), socket.read_exact(&mut bytes))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(bytes, expected);
    }
    // The peer sends no acknowledgement. The codec supplies no extra request.
    assert!(
        tokio::time::timeout(Duration::from_millis(20), socket.read_u8())
            .await
            .is_err()
    );
    connection.shutdown().await;
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), socket.read(&mut [0; 1]))
            .await
            .unwrap()
            .unwrap(),
        0
    );
}
