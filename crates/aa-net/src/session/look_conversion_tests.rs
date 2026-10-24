use super::*;
use crate::{
    item_effect::CapturedBagItem,
    items::ItemTask,
    look_conversion::tests::{ACTOR, detail_receipt, fixture, request, skin_receipt},
    look_conversion::{ItemLookOperation, ItemLookRequest},
    wire::Writer,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

const OPERATIONS: [ItemLookOperation; 3] = [
    ItemLookOperation::Convert,
    ItemLookOperation::Cleanup,
    ItemLookOperation::Skinize,
];

fn snapshot(request: &ItemLookRequest) -> InvenChunk {
    let mut items = vec![None; 10];
    for captured in request.captures() {
        items[usize::from(captured.slot)] = Some(captured.item.clone());
    }
    InvenChunk {
        container: 2,
        chunks: 1,
        start_chunk: 0,
        items,
        truncated: false,
    }
}

fn active(request: &ItemLookRequest) -> (Session, UnboundedReceiver<Event>) {
    let (tx, mut events) = unbounded_channel();
    let mut s = Session::new(tx, ClientOptions::default());
    let mut spawn = crate::native_item_records::tests::spawn();
    spawn.obj_id = ACTOR;
    spawn.unit.obj_id = ACTOR;
    spawn.unit.kind = UnitKind::Character { char_id: 42 };
    spawn.state.character.id = 42;
    s.phase = Phase::InWorld;
    s.selected = 42;
    s.self_obj = ACTOR;
    s.selected_state = Some(spawn.state.clone());
    s.emit(Event::CharacterSelected(Box::new(spawn.state.clone())));
    s.emit(Event::PlayerSpawn(Box::new(spawn)));
    s.emit(Event::InventoryInit {
        bag_slots: 10,
        bank_slots: 10,
    });
    s.emit(Event::InventoryChunk(snapshot(request)));
    drain(&mut events);
    (s, events)
}

fn command(request: &ItemLookRequest) -> Command {
    Command::ItemLook {
        character_id: 42,
        request: request.clone(),
    }
}

fn drain(events: &mut UnboundedReceiver<Event>) {
    while events.try_recv().is_ok() {}
}

fn rejected(events: &mut UnboundedReceiver<Event>, request: &ItemLookRequest) {
    assert!(
        matches!(events.try_recv(), Ok(Event::ItemLookRejected { character_id: 42, request: actual, message }) if actual == *request && !message.is_empty())
    );
    assert!(events.try_recv().is_err());
}

async fn peer(s: &mut Session) -> TcpStream {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let (conn, accepted) = tokio::join!(GameConn::connect(&address), listener.accept());
    s.game = Some(conn.unwrap());
    accepted.unwrap().0
}

async fn read_frame(socket: &mut TcpStream) -> Vec<u8> {
    tokio::time::timeout(Duration::from_secs(2), async {
        let length = socket.read_u16_le().await.unwrap();
        let mut bytes = vec![0; usize::from(length)];
        socket.read_exact(&mut bytes).await.unwrap();
        bytes
    })
    .await
    .unwrap()
}

async fn silent(socket: &mut TcpStream) {
    assert!(
        tokio::time::timeout(Duration::from_millis(15), socket.read_u8())
            .await
            .is_err()
    );
}

async fn sent(
    s: &mut Session,
    events: &mut UnboundedReceiver<Event>,
    socket: &mut TcpStream,
    request: &ItemLookRequest,
    counter: u8,
) {
    s.command(command(request)).await;
    assert!(
        matches!(events.try_recv(), Ok(Event::ItemLookRequested { character_id: 42, request: actual }) if actual == *request)
    );
    assert!(events.try_recv().is_err());
    let expected = game::ClientMsg::ItemLook {
        self_obj_id: ACTOR,
        request: request.clone(),
    }
    .encode(counter);
    assert_eq!(read_frame(socket).await, expected);
}

fn refreshed(s: &Session, previous: &ItemLookRequest) -> ItemLookRequest {
    let records = s.received_native_records.borrow();
    let capture = |c: &CapturedBagItem| CapturedBagItem {
        slot: c.slot,
        item: records.item(2, c.slot).unwrap().clone(),
    };
    let source = capture(previous.source());
    let target = capture(previous.target());
    match previous.operation() {
        ItemLookOperation::Convert => ItemLookRequest::convert(target, source),
        ItemLookOperation::Cleanup => {
            ItemLookRequest::cleanup(previous.skill_id().unwrap(), source, target)
        }
        ItemLookOperation::Skinize => {
            ItemLookRequest::skinize(previous.skill_id().unwrap(), source, target)
        }
    }
    .unwrap()
}

async fn receive(s: &mut Session, socket: &mut TcpStream, frame: &[u8]) {
    socket.write_u16_le(frame.len() as u16).await.unwrap();
    socket.write_all(frame).await.unwrap();
    let message = tokio::time::timeout(Duration::from_secs(2), s.game.as_mut().unwrap().recv())
        .await
        .unwrap();
    s.game_msg(message).await;
}

#[tokio::test]
async fn look_conversion_sends_native_frames_without_mutation_or_success_or_retry() {
    for operation in OPERATIONS {
        let request = request(operation);
        let (mut s, mut events) = active(&request);
        let records = s.received_native_records.borrow().clone();
        let bag = s.received_inventory.borrow().bag.clone();
        let mut socket = peer(&mut s).await;
        s.command(command(&request)).await;
        let mut expected = vec![0xdd, 1, 0, 0];
        expected.extend_from_slice(
            &(if operation == ItemLookOperation::Convert {
                0x49u16
            } else {
                0x52u16
            })
            .to_le_bytes(),
        );
        expected.extend_from_slice(fixture(operation));
        assert_eq!(read_frame(&mut socket).await, expected);
        assert!(
            matches!(events.try_recv(), Ok(Event::ItemLookRequested { character_id: 42, request: actual }) if actual == request)
        );
        assert!(events.try_recv().is_err());
        assert_eq!(*s.received_native_records.borrow(), records);
        assert_eq!(s.received_inventory.borrow().bag, bag);
        silent(&mut socket).await;
        let mut error = Writer::new();
        error.i16(737).i16(737).u32(29880).u8(1);
        receive(
            &mut s,
            &mut socket,
            &crate::look_conversion::tests::dd01(0x10a, &error.finish()),
        )
        .await;
        assert!(matches!(
            events.try_recv(),
            Ok(Event::ServerError { code: 737 })
        ));
        assert!(events.try_recv().is_err());
        assert_eq!(*s.received_native_records.borrow(), records);
        silent(&mut socket).await;
        s.game.as_mut().unwrap().shutdown().await;
    }
}

#[tokio::test]
async fn look_conversion_guards_all_phases_character_headers_owner_actor_and_socket() {
    for operation in OPERATIONS {
        let request = request(operation);
        for phase in [
            Phase::Idle,
            Phase::Authenticating,
            Phase::WorldSelect,
            Phase::Queued,
            Phase::ReadyToConnect,
            Phase::Connecting,
            Phase::CharacterSelect,
            Phase::EnteringWorld,
            Phase::Leaving,
        ] {
            let (mut s, mut events) = active(&request);
            s.phase = phase;
            let mut socket = peer(&mut s).await;
            s.command(command(&request)).await;
            rejected(&mut events, &request);
            silent(&mut socket).await;
        }
        for case in 0..8 {
            let (mut s, mut events) = active(&request);
            match case {
                0 => s.selected = 0,
                1 => s.selected = 43,
                2 => s.self_obj = 0,
                3 => s.self_obj = 0x1000000,
                4 => s.selected_state = None,
                5 => s.selected_state.as_mut().unwrap().character.id = 43,
                6 => *s.received_native_records.borrow_mut() = Default::default(),
                _ => {
                    let mut spawn = crate::native_item_records::tests::spawn();
                    spawn.obj_id = ACTOR;
                    spawn.unit.obj_id = ACTOR;
                    s.emit(Event::PlayerSpawn(Box::new(spawn)));
                    s.emit(Event::InventoryInit {
                        bag_slots: 10,
                        bank_slots: 10,
                    });
                    s.emit(Event::InventoryChunk(snapshot(&request)));
                    // The items still match, but their actual provider owner is47.
                    assert!(
                        request
                            .recheck_native(&s.received_native_records.borrow())
                            .is_ok()
                    );
                }
            }
            drain(&mut events);
            let mut socket = peer(&mut s).await;
            s.command(command(&request)).await;
            rejected(&mut events, &request);
            silent(&mut socket).await;
        }
        let (mut s, mut events) = active(&request);
        s.command(command(&request)).await; // Exact records, absent socket.
        rejected(&mut events, &request);
        let mut socket = peer(&mut s).await;
        for character_id in [0, 43] {
            s.command(Command::ItemLook {
                character_id,
                request: request.clone(),
            })
            .await;
            assert!(
                matches!(events.try_recv(), Ok(Event::ItemLookRejected { character_id: got, request: actual, .. }) if got == character_id && actual == request)
            );
            assert!(events.try_recv().is_err());
            silent(&mut socket).await;
        }
        // Native owner is unchanged and no unrelated optional subsystem gate was added.
        sent(&mut s, &mut events, &mut socket, &request, 0).await;
    }
}

#[tokio::test]
async fn look_conversion_rechecks_both_full_records_and_never_uses_legacy_snapshot() {
    for operation in OPERATIONS {
        let request = request(operation);
        for member in 0..2 {
            for field in 0..14 {
                let (mut s, mut events) = active(&request);
                let mut chunk = snapshot(&request);
                let capture = if member == 0 {
                    request.source()
                } else {
                    request.target()
                };
                let item = chunk.items[usize::from(capture.slot)].as_mut().unwrap();
                match field {
                    0 => item.id ^= 1 << 60,
                    1 => item.template_id ^= 1,
                    2 => item.grade ^= 1,
                    3 => item.flags ^= 8,
                    4 => item.count -= 1,
                    5 => item.made_unit_id ^= 1,
                    6 => item.create_time += 1,
                    7 => item.lifespan_mins += 1,
                    8 => item.world_id ^= 1,
                    9 => item.unsecure_time += 1,
                    10 => item.unpack_time += 1,
                    11 => item.detail_payload.as_mut().unwrap()[5] ^= 1,
                    12 => item.detail = crate::model::ItemDetail::None,
                    _ => chunk.items[usize::from(capture.slot)] = None,
                }
                s.emit(Event::InventoryChunk(chunk));
                // Supply the old exact record through legacy state. It cannot authorize a send.
                s.received_inventory.borrow_mut().bag[usize::from(capture.slot)] =
                    Some(capture.item.clone());
                drain(&mut events);
                let mut socket = peer(&mut s).await;
                s.command(command(&request)).await;
                rejected(&mut events, &request);
                silent(&mut socket).await;
            }
        }
    }
}

#[tokio::test]
async fn look_conversion_received_native_rows_allow_explicit_refresh_and_snapshot_repair_only() {
    for operation in OPERATIONS {
        let request = request(operation);
        let (mut s, mut events) = active(&request);
        let mut socket = peer(&mut s).await;
        let slot = request.target().slot;
        // Unknown ordinary framed opcode does not by itself lose native certainty.
        s.game_msg(Ok(game::ServerMsg::Unhandled {
            level: 1,
            opcode: 0xfff,
            len: 20,
        }))
        .await;
        drain(&mut events);
        sent(&mut s, &mut events, &mut socket, &request, 0).await;
        let received = if operation == ItemLookOperation::Skinize {
            skin_receipt(slot, 999)
        } else {
            detail_receipt(slot, 999, 8002)
        };
        for _ in 0..2 {
            receive(&mut s, &mut socket, &received).await;
            assert!(
                matches!(events.try_recv(), Ok(Event::ItemsChanged(changes)) if changes.reason == if operation == ItemLookOperation::Skinize { 113 } else { 111 })
            );
            assert!(events.try_recv().is_err());
            silent(&mut socket).await;
        }
        // Original Update9/Flags10 use the declared physical record despite packetID999.
        assert!(
            request
                .recheck_native(&s.received_native_records.borrow())
                .is_err()
        );
        s.command(command(&request)).await;
        rejected(&mut events, &request);
        silent(&mut socket).await;
        let after = refreshed(&s, &request);
        sent(&mut s, &mut events, &mut socket, &after, 1).await;
        let target = after.target();
        // Count ignores wire IDs; full native capture must reject while the legacy copy differs.
        s.emit(Event::ItemsChanged(ItemTasks {
            reason: 40,
            tasks: vec![ItemTask::Count {
                container: 2,
                slot,
                item_id: 999,
                template_id: 999,
                delta: -1,
            }],
            ..Default::default()
        }));
        assert_eq!(
            s.received_native_records
                .borrow()
                .item(2, slot)
                .unwrap()
                .count,
            target.item.count - 1
        );
        drain(&mut events);
        s.command(command(&after)).await;
        rejected(&mut events, &after);
        let after_count = refreshed(&s, &after);
        sent(&mut s, &mut events, &mut socket, &after_count, 2).await;
        // Unsupported/synthetic detail retires the individual row, not its sibling.
        s.emit(Event::ItemsChanged(ItemTasks {
            tasks: vec![ItemTask::Update {
                container: 2,
                slot,
                item_id: 999,
                detail: crate::model::ItemDetail::None,
                detail_payload: None,
                raw_detail_block: None,
            }],
            ..Default::default()
        }));
        assert!(s.received_native_records.borrow().item(2, slot).is_none());
        assert!(
            s.received_native_records
                .borrow()
                .item(2, after_count.source().slot)
                .is_some()
        );
        drain(&mut events);
        s.command(command(&after_count)).await;
        rejected(&mut events, &after_count);
        silent(&mut socket).await;
        s.emit(Event::InventoryChunk(snapshot(&request)));
        drain(&mut events);
        sent(&mut s, &mut events, &mut socket, &request, 3).await;
        // Init clears certainty even if legacy memory can still supply old items.
        s.emit(Event::InventoryInit {
            bag_slots: 10,
            bank_slots: 10,
        });
        drain(&mut events);
        s.command(command(&request)).await;
        rejected(&mut events, &request);
        silent(&mut socket).await;
    }
}

#[tokio::test]
async fn look_conversion_decode_gap_bad_snapshot_and_actor_loss_stay_unqualified() {
    for operation in OPERATIONS {
        let request = request(operation);
        for case in 0..6 {
            let (mut s, mut events) = active(&request);
            let mut socket = peer(&mut s).await;
            match case {
                0 => {
                    s.game_msg(Err(NetError::Wire(crate::wire::WireError::Invalid(
                        "look conversion gap".into(),
                    ))))
                    .await
                }
                1 => s.emit(Event::UnhandledGamePacket {
                    level: 4,
                    opcode: 0xfff,
                    len: 1,
                }),
                2 => {
                    let mut chunk = snapshot(&request);
                    chunk.truncated = true;
                    s.emit(Event::InventoryChunk(chunk));
                }
                3 => s.emit(Event::UnitsRemoved(vec![ACTOR])),
                4 => s.emit(Event::ItemsChanged(ItemTasks {
                    truncated: true,
                    ..Default::default()
                })),
                _ => s.emit(Event::UnhandledGamePacket {
                    level: 1,
                    opcode: 0x93,
                    len: 1,
                }),
            }
            // Same-owner snapshots/Init/spawn cannot erase a factual decode-history loss.
            let mut spawn = crate::native_item_records::tests::spawn();
            spawn.obj_id = ACTOR;
            spawn.unit.obj_id = ACTOR;
            spawn.unit.kind = UnitKind::Character { char_id: 42 };
            spawn.state.character.id = 42;
            s.emit(Event::PlayerSpawn(Box::new(spawn)));
            s.emit(Event::InventoryInit {
                bag_slots: 10,
                bank_slots: 10,
            });
            s.emit(Event::InventoryChunk(snapshot(&request)));
            drain(&mut events);
            s.command(command(&request)).await;
            rejected(&mut events, &request);
            silent(&mut socket).await;
        }
    }
}

#[tokio::test]
async fn look_conversion_exact_bag_slot_extent_and_shutdown_transport_are_enforced() {
    for operation in OPERATIONS {
        let original = request(operation);
        let mut source = original.source().clone();
        source.slot = 129;
        let target = original.target().clone();
        let request = match operation {
            ItemLookOperation::Convert => ItemLookRequest::convert(target, source.clone()),
            ItemLookOperation::Cleanup => ItemLookRequest::cleanup(23457, source.clone(), target),
            ItemLookOperation::Skinize => ItemLookRequest::skinize(23741, source.clone(), target),
        }
        .unwrap();
        let (mut s, mut events) = active(&original);
        let mut socket = peer(&mut s).await;
        s.command(command(&request)).await; // Matching ID at its old slot is insufficient.
        rejected(&mut events, &request);
        silent(&mut socket).await;
        s.emit(Event::InventoryInit {
            bag_slots: 130,
            bank_slots: 130,
        });
        s.emit(Event::InventoryChunk(snapshot(&original)));
        let mut last = InvenChunk {
            container: 3,
            chunks: 1,
            start_chunk: 12,
            items: vec![None; 10],
            truncated: false,
        };
        last.items[9] = Some(source.item.clone());
        s.emit(Event::InventoryChunk(last.clone()));
        drain(&mut events);
        s.command(command(&request)).await; // A complete matching BANK record is insufficient.
        rejected(&mut events, &request);
        silent(&mut socket).await;
        last.container = 2;
        s.emit(Event::InventoryChunk(last));
        drain(&mut events);
        assert!(
            request
                .recheck_native(&s.received_native_records.borrow())
                .is_ok()
        );
        sent(&mut s, &mut events, &mut socket, &request, 0).await;
        s.emit(Event::InventoryExpanded {
            container: 2,
            slots: 120,
        });
        drain(&mut events);
        s.command(command(&request)).await;
        rejected(&mut events, &request);
        silent(&mut socket).await;
        s.emit(Event::InventoryExpanded {
            container: 2,
            slots: 130,
        });
        drain(&mut events);
        s.game.as_mut().unwrap().shutdown().await;
        s.command(command(&request)).await;
        let mut rejection = false;
        while let Ok(event) = events.try_recv() {
            match event {
                Event::ItemLookRejected {
                    character_id: 42,
                    request: actual,
                    ..
                } => {
                    assert_eq!(actual, request);
                    rejection = true;
                }
                Event::ItemLookRequested { .. }
                | Event::ItemsChanged(_)
                | Event::SkillStarted(_)
                | Event::SkillFired(_) => panic!("unexpected optimistic event: {event:?}"),
                _ => {}
            }
        }
        assert!(rejection);
    }
}
