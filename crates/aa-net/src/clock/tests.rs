use super::*;
use std::time::Duration;
// Original AddSample + actual original comparator and CRT qsort executed independently in local Unicorn.
type OracleRow = ([u32; 3], [(i32, i32); 5], usize, u32, i32, u32);
fn verify(rows: &[OracleRow]) {
    let mut k = NativeTimeKeeper::default();
    for (step, (sample, pairs, index, anchor, mean, mapped)) in rows.iter().enumerate() {
        k.add_sample(sample[0], sample[1], sample[2]);
        assert_eq!(k.pairs.map(|p| (p.rtt, p.local)), *pairs, "step {step} pairs");
        assert_eq!(
            (k.index, k.anchor, k.local_mean, k.mapped_time(sample[2])),
            (*index, *anchor, *mean, *mapped),
            "step {step} state"
        );
    }
}
#[test]
fn native_oracle_odd_first_and_outlier() {
    verify(&[
        (
            [1000, 5000, 1011],
            [(11, 1005), (11, 1005), (11, 1005), (11, 1005), (11, 1005)],
            0,
            5000,
            1005,
            5006,
        ),
        (
            [2000, 6000, 2016],
            [(16, 1008), (11, 1005), (11, 1005), (11, 1005), (11, 1005)],
            1,
            5000,
            1005,
            6011,
        ),
        (
            [3000, 7000, 3010],
            [(16, 1008), (10, 1005), (11, 1005), (11, 1005), (11, 1005)],
            2,
            5000,
            1005,
            7005,
        ),
        (
            [4000, 8000, 4012],
            [(16, 1008), (10, 1005), (12, 1006), (11, 1005), (11, 1005)],
            3,
            5000,
            1005,
            8007,
        ),
        (
            [5000, 9000, 5200],
            [(16, 1008), (10, 1005), (12, 1006), (200, 1100), (11, 1005)],
            4,
            5000,
            1005,
            9195,
        ),
        (
            [6000, 10000, 6009],
            [(16, 1008), (10, 1005), (12, 1006), (200, 1100), (9, 1004)],
            0,
            5000,
            1005,
            10004,
        ),
    ]);
}
#[test]
fn native_oracle_mixed_threshold() {
    verify(&[
        ([100, 10000, 110], [(10, 105), (10, 105), (10, 105), (10, 105), (10, 105)], 0, 10000, 105, 10005),
        ([200, 10100, 214], [(14, 107), (10, 105), (10, 105), (10, 105), (10, 105)], 1, 10000, 105, 10109),
        ([300, 10200, 308], [(14, 107), (8, 104), (10, 105), (10, 105), (10, 105)], 2, 10000, 105, 10203),
        ([400, 10300, 412], [(14, 107), (8, 104), (12, 106), (10, 105), (10, 105)], 3, 10000, 105, 10307),
        ([500, 10400, 518], [(14, 107), (8, 104), (12, 106), (18, 109), (10, 105)], 4, 10000, 106, 10412),
        ([600, 10500, 616], [(14, 107), (8, 104), (12, 106), (18, 109), (16, 108)], 0, 10000, 107, 10509),
    ]);
}
#[test]
fn native_oracle_f32_boundary() {
    verify(&[
        (
            [16777210, 500000, 16777221],
            [(11, 16777214), (11, 16777214), (11, 16777214), (11, 16777214), (11, 16777214)],
            0,
            500000,
            16777214,
            500007,
        ),
        (
            [16777310, 500100, 16777321],
            [(11, 16777214), (11, 16777214), (11, 16777214), (11, 16777214), (11, 16777214)],
            1,
            500000,
            16777214,
            500107,
        ),
    ]);
}
#[test]
fn native_oracle_unsigned_wrap() {
    verify(&[
        ([4294967280, 1000, 4], [(20, -6), (20, -6), (20, -6), (20, -6), (20, -6)], 0, 1000, -6, 1010),
        ([32, 1048, 48], [(16, -8), (20, -6), (20, -6), (20, -6), (20, -6)], 1, 1000, -6, 1054),
    ]);
}
#[test]
fn native_oracle_zero_anchor() {
    verify(&[
        ([100, 0, 110], [(10, 105), (10, 105), (10, 105), (10, 105), (10, 105)], 0, 0, 105, 5),
        ([200, 1000, 220], [(20, 210), (20, 210), (20, 210), (20, 210), (20, 210)], 0, 1000, 210, 1010),
    ]);
}
#[test]
fn native_oracle_signed_square_overflow() {
    verify(&[
        ([100, 1000, 110], [(10, 105), (10, 105), (10, 105), (10, 105), (10, 105)], 0, 1000, 105, 1005),
        (
            [200, 1100, 100200],
            [(100000, 50100), (10, 105), (10, 105), (10, 105), (10, 105)],
            1,
            1000,
            105,
            101095,
        ),
        (
            [300, 1200, 200300],
            [(100000, 50100), (200000, 100100), (10, 105), (10, 105), (10, 105)],
            2,
            1000,
            105,
            201195,
        ),
        (
            [400, 1300, 300400],
            [(100000, 50100), (200000, 100100), (300000, 150100), (10, 105), (10, 105)],
            3,
            1000,
            0,
            301400,
        ),
        (
            [500, 1400, 400500],
            [(100000, 50100), (200000, 100100), (300000, 150100), (400000, 200100), (10, 105)],
            4,
            1000,
            0,
            401500,
        ),
    ]);
}
#[test]
fn native_oracle_nontransitive_rtt() {
    verify(&[
        ([0, 1000, 10], [(10, 5), (10, 5), (10, 5), (10, 5), (10, 5)], 0, 1000, 5, 1005),
        (
            [0, 1100, 2147483648],
            [(-2147483648, -2147483648), (10, 5), (10, 5), (10, 5), (10, 5)],
            1,
            1000,
            5,
            2147484643,
        ),
        (
            [0, 1200, 2147483647],
            [(-2147483648, -2147483648), (2147483647, 1073741568), (10, 5), (10, 5), (10, 5)],
            2,
            1000,
            5,
            2147484642,
        ),
        (
            [0, 1300, 4294967295],
            [(-2147483648, -2147483648), (2147483647, 1073741568), (-1, -300), (10, 5), (10, 5)],
            3,
            1000,
            -357914126,
            357915125,
        ),
        (
            [0, 1400, 0],
            [(-2147483648, -2147483648), (2147483647, 1073741568), (-1, -300), (0, -400), (10, 5)],
            4,
            1000,
            536870634,
            3758097662,
        ),
    ]);
}
fn sample(at: Instant) -> ClockSample {
    ClockSample {
        pong: ProxyPong {
            tm: 1,
            when: 2,
            elapsed: 0,
            remote: 5000000,
            local: 1000,
            world: 5000,
        },
        local_receive_ms: 1011,
        received_at: at,
        local_origin: at - Duration::from_millis(1011),
    }
}
#[test]
fn receive_anchor_accounts_for_late_ui_delivery_and_connection_lifecycle() {
    let at = Instant::now();
    let mut c = ServerClock::default();
    assert_eq!(c.mapped_at(at), None);
    c.apply(&Event::ClockSample(sample(at)));
    assert_eq!(c.mapped_at(at), Some(5006));
    assert_eq!(c.mapped_at(at + Duration::from_millis(200)), Some(5206));
    let cloned = c.clone();
    assert_eq!(cloned.mapped_at(at + Duration::from_millis(200)), Some(5206));
    assert!(!c.apply(&Event::CharacterSelected(Box::default())));
    assert!(c.mapped_at(at).is_some());
    for e in [
        Event::PhaseChanged(Phase::Connecting),
        Event::ConnectFailed { message: String::new() },
        Event::Disconnected { message: String::new() },
        Event::Kicked {
            reason: 0,
            message: String::new(),
        },
    ] {
        let mut c = cloned.clone();
        assert!(c.apply(&e));
        assert_eq!(c, ServerClock::default());
        assert_eq!(c.mapped_at(at), None);
    }
}
#[test]
fn native_init_keeps_mean_and_pairs_but_resource_never_exposes_an_unsampled_clock() {
    let mut k = NativeTimeKeeper::default();
    k.add_sample(1000, 5000, 1011);
    let pairs = k.pairs;
    let mean = k.local_mean;
    k.init();
    assert_eq!(k.anchor, 0);
    assert_eq!(k.index, 0);
    assert_eq!(k.pairs, pairs);
    assert_eq!(k.local_mean, mean);
    assert_eq!(cvtt(2147483648.0), i32::MIN);
    assert_eq!(cvtt(f32::NAN), i32::MIN);
    assert_eq!(cvtt(-3.9), -3);
}

#[test]
fn plain_worldstate_uses_received_clock_before_folding_battlefield_updates() {
    use crate::battlefield::{BattlefieldPhase, BattlefieldStart};
    let at = Instant::now();
    let mut world = crate::state::WorldState::default();
    world.battlefield.phase = Some(BattlefieldPhase::Opening);
    world.battlefield.start = Some(BattlefieldStart {
        zone_id: 8,
        instance_id: 9,
        now_ms: 0,
        start_ms: 6000,
    });
    world.apply(&Event::ClockSample(sample(at)));
    assert!(world.server_clock.mapped_now().is_some());
    assert_eq!(world.battlefield.phase, Some(BattlefieldPhase::Playing));
    world.apply(&Event::Disconnected { message: String::new() });
    assert_eq!(world.server_clock.mapped_now(), None);
}

#[test]
fn fresh_local_timer_preserves_fractional_origin_across_ui_delivery() {
    let at = Instant::now();
    let mut captured = sample(at);
    captured.local_origin = at - Duration::from_micros(1_011_900);
    let mut clock = ServerClock::default();
    clock.apply(&Event::ClockSample(captured));
    assert_eq!(clock.mapped_at(at), Some(5006));
    // The source local timer crosses its next millisecond before a full millisecond elapses after receipt.
    assert_eq!(clock.mapped_at(at + Duration::from_micros(200)), Some(5007));
}
