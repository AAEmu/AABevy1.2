//! The session against a scripted login server that answers with the frames the real server's own
//! tests produce (`aaemu-login` `packets/l2c.rs`).

use std::time::Duration;

use aa_net::{Client, Command, Event, Phase};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

/// Reads one `[u16 len][payload]` frame and returns the opcode.
async fn read_opcode(sock: &mut tokio::net::TcpStream) -> (u16, Vec<u8>) {
    let mut len = [0u8; 2];
    sock.read_exact(&mut len).await.unwrap();
    let mut payload = vec![0u8; u16::from_le_bytes(len) as usize];
    sock.read_exact(&mut payload).await.unwrap();
    (u16::from_le_bytes([payload[0], payload[1]]), payload)
}

async fn next(client: &mut Client) -> Event {
    tokio::time::timeout(Duration::from_secs(5), client.recv())
        .await
        .expect("timed out")
        .expect("session ended")
}

#[tokio::test]
async fn login_then_world_list_then_cookie() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        // CARequestAuthTrion -> ACJoinResponse + ACAuthResponse.
        let (op, payload) = read_opcode(&mut sock).await;
        assert_eq!(op, 0x004);
        let text = String::from_utf8_lossy(&payload).to_string();
        assert!(text.contains("<username>bevytest</username>"), "{text}");
        assert!(!text.contains("hunter2"), "the password must be hashed");
        sock.write_all(&hex("0c00000000000601000002000000")).await.unwrap();
        sock.write_all(&hex(
            "29000300040302012000000000000000000000000000000000000000000000000000000000000000000006",
        ))
        .await
        .unwrap();
        // CAListWorld -> ACWorldList.
        let (op, _) = read_opcode(&mut sock).await;
        assert_eq!(op, 0x00b);
        sock.write_all(&hex(concat!(
            "76000800020105004141456d750002050054c3a97374010200000000000000000002",
            "14000000020a0000000300426f6201021000000000000000000000000000000000000000000000000000",
            "0700000001ddccbbaa0500c39c6ec3af0401100000000000000000000000000000000000000000000000",
            "0000"
        )))
        .await
        .unwrap();
        // CAEnterWorld(2) -> queue position, then the cookie.
        let (op, payload) = read_opcode(&mut sock).await;
        assert_eq!(op, 0x00c);
        assert_eq!(*payload.last().unwrap(), 2);
        sock.write_all(&hex("0a0009000200030005000200"))
            .await
            .unwrap();
        sock.write_all(&hex("1e000a004d0000000100007fd7040100007fd7040100007fd7040100007fd704"))
            .await
            .unwrap();
        // Keep the socket open until the client is dropped.
        let mut rest = Vec::new();
        let _ = sock.read_to_end(&mut rest).await;
    });

    let mut client = Client::spawn_on(&tokio::runtime::Handle::current());
    client.send(Command::Login {
        addr,
        user: "bevytest".into(),
        password: "hunter2".into(),
    });
    assert!(matches!(next(&mut client).await, Event::PhaseChanged(Phase::Authenticating)));
    assert!(matches!(next(&mut client).await, Event::PhaseChanged(Phase::WorldSelect)));
    let Event::LoginSucceeded(account) = next(&mut client).await else { panic!("no LoginSucceeded") };
    assert_eq!((account.account_id, account.max_characters, account.extra_characters_per_world), (0x0102_0304, 6, 2));
    assert!(account.pre_select_period);
    let Event::WorldList(list) = next(&mut client).await else { panic!("no WorldList") };
    assert_eq!(list.worlds.len(), 2);
    assert_eq!(list.character_count(2), 1);

    client.send(Command::EnterWorld { world_id: 2 });
    assert!(matches!(next(&mut client).await, Event::PhaseChanged(Phase::Queued)));
    let Event::WorldQueue(q) = next(&mut client).await else { panic!("no WorldQueue") };
    assert_eq!((q.world_id, q.my_turn, q.normal_length), (2, 3, 5));
    let ev = loop {
        match next(&mut client).await {
            Event::ReadyToConnect { world_id, cookie } => break (world_id, cookie),
            Event::PhaseChanged(_) | Event::WorldQueue(_) => {}
            other => panic!("unexpected {other:?}"),
        }
    };
    assert_eq!(ev.0, 2);
    assert_eq!(ev.1.cookie, 77);
    assert_eq!(ev.1.addr.to_string(), "127.0.0.1:1239");

    drop(client);
    server.await.unwrap();
}

#[tokio::test]
async fn bad_password_is_reported_and_the_session_stays_usable() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let _ = read_opcode(&mut sock).await;
        sock.write_all(&hex("07000c000100000000")).await.unwrap();
        let mut rest = Vec::new();
        let _ = sock.read_to_end(&mut rest).await;
    });
    let mut client = Client::spawn_on(&tokio::runtime::Handle::current());
    client.send(Command::Login { addr, user: "x".into(), password: "y".into() });
    loop {
        match next(&mut client).await {
            Event::LoginFailed { message } => {
                assert!(message.contains("wrong user name or password"), "{message}");
                break;
            }
            Event::PhaseChanged(_) => {}
            other => panic!("unexpected {other:?}"),
        }
    }
    // A second login attempt to a closed port fails cleanly instead of hanging.
    client.send(Command::Login { addr: "127.0.0.1:1".into(), user: "x".into(), password: "y".into() });
    loop {
        match next(&mut client).await {
            Event::LoginFailed { message } => {
                assert!(message.contains("cannot reach"), "{message}");
                break;
            }
            Event::PhaseChanged(_) => {}
            other => panic!("unexpected {other:?}"),
        }
    }
}
