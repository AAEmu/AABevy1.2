use super::*;
use crate::{
    battlefield::Battlefield,
    game::{MAGIC, ServerMsg},
    opcodes::sc,
    wire::Writer,
};
fn fixture(message: Vec<u8>) -> ScheduledEventStarted {
    ScheduledEventStarted {
        id: 0x10203040,
        values: std::array::from_fn(|i| if i == 10 { -123 } else { i as i32 - 7 }),
        changes: 0xffe00005,
        priority: 0xfe,
        start_date: 2,
        period_date: 3,
        hour: 25,
        minute: 61,
        period_minutes: -9,
        week_day: 0xff,
        open_message: message,
    }
}
fn body(s: &ScheduledEventStarted) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(s.id);
    for v in s.values {
        w.i32(v);
    }
    w.u32(s.changes)
        .u8(s.priority)
        .u8(s.start_date)
        .u8(s.period_date)
        .u8(s.hour)
        .u8(s.minute)
        .i32(s.period_minutes)
        .u8(s.week_day)
        .u16(s.open_message.len() as u16)
        .raw(&s.open_message);
    w.finish()
}
#[test]
fn native_complete_schema_golden_retains_raw_fields_and_compressed_packet_tail() {
    let s = fixture(b"native".to_vec());
    let b = body(&s);
    assert_eq!(b.len(), 104 + 6);
    assert_eq!(&b[44..48], &(-123i32).to_le_bytes());
    let mut w = Writer::new();
    w.raw(&[MAGIC, 1, 0, 1]).u16(sc::SCScheduledEventStartedPacket).raw(&b);
    assert_eq!(ServerMsg::decode(&w.finish()).unwrap(), ServerMsg::ScheduledEventStarted(s.clone()));
    for cut in 0..b.len() {
        assert!(ScheduledEventStarted::read_native(&mut Reader::new(&b[..cut])).is_err(), "cut {cut}");
    }
    let mut w = Writer::new();
    w.u16(0)
        .u16(sc::SCScheduledEventStartedPacket)
        .raw(&b)
        .u16(0)
        .u16(sc::SCOnOffSnowPacket)
        .bool(true);
    let raw = w.finish();
    let n = raw.len() as u16;
    let mut w = Writer::new();
    w.raw(&[MAGIC, 4]).u16(2).u8(1).u16(n).u16(!n).raw(&raw);
    assert_eq!(
        ServerMsg::decode(&w.finish()).unwrap(),
        ServerMsg::Batch(vec![ServerMsg::ScheduledEventStarted(s), ServerMsg::Snow(true)])
    );
}
#[test]
fn open_message_bound_counts_wire_bytes_and_preserves_non_utf8_and_nuls() {
    for len in [0, 126, 127] {
        let s = fixture(vec![0xff; len]);
        assert_eq!(ScheduledEventStarted::read_native(&mut Reader::new(&body(&s))).unwrap(), s);
    }
    let s = fixture(vec![b'x'; 128]);
    assert!(ScheduledEventStarted::read_native(&mut Reader::new(&body(&s))).is_err());
    let s = fixture(vec![0, b'x', 0xff, 0]);
    let actual = ScheduledEventStarted::read_native(&mut Reader::new(&body(&s))).unwrap();
    assert_eq!(actual.open_message, s.open_message);
}
#[test]
fn receipts_are_not_an_active_merge_policy_and_ratio_changes_only_from_actual_start_or_reset() {
    let mut history = ScheduledEvents::default();
    let mut bf = Battlefield::default();
    assert_eq!(bf.my_corps_kill_death_ratio, 0);
    let event = Event::ScheduledEventStarted(fixture(vec![]));
    assert!(history.apply(&event));
    bf.apply(&event);
    assert_eq!(bf.my_corps_kill_death_ratio, -123);
    assert!(!history.apply(&event));
    assert_eq!(history.starts[&0x10203040].changes, 0xffe00005);
    let mut other = fixture(vec![]);
    other.id = 1;
    other.values[10] = i32::MIN;
    other.changes = 0;
    let event = Event::ScheduledEventStarted(other);
    history.apply(&event);
    bf.apply(&event);
    assert_eq!(history.starts.len(), 2);
    assert_eq!(bf.my_corps_kill_death_ratio, i32::MIN); // No invented changes-bit gate.
    bf.apply(&Event::Battlefield(crate::battlefield::BattlefieldEvent::Cancelled { raw: 0 }));
    assert_eq!(bf.my_corps_kill_death_ratio, 0);
    assert_eq!(history.starts.len(), 2);
    for reset in [
        Event::CharacterSelected(Box::default()),
        Event::Kicked {
            reason: 0,
            message: String::new(),
        },
        Event::Disconnected { message: String::new() },
        Event::LeftWorld { target: 0 },
    ] {
        let mut h = history.clone();
        assert!(h.apply(&reset));
        assert!(h.starts.is_empty());
    }
    let mut world = crate::state::WorldState::default();
    world.apply(&Event::ScheduledEventStarted(fixture(vec![])));
    assert_eq!(world.battlefield.my_corps_kill_death_ratio, -123);
    assert_eq!(world.scheduled_events.starts.len(), 1);
}
