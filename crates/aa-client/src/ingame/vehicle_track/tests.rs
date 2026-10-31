use super::*;
use aa_net::wire::Reader;

fn sample(time: u32, x: f32, y: f32, yaw: f32, ship: bool) -> VehicleSample {
    VehicleSample {
        time,
        pos: Vec3::new(x, y, 0.0),
        rot: Quat::from_rotation_z(yaw),
        steering: 0.0,
        throttle: 0.0,
        zone: 1,
        ship,
    }
}

/// A cart driving along +x at `speed` m/s, reported every 100 ms from `start_ms`.
fn drive(
    track: &mut VehicleTrack,
    start_ms: u32,
    speed: f32,
    seconds: f64,
    dt: f64,
) -> Vec<VehiclePose> {
    let (mut out, mut now, mut sent) = (Vec::new(), 0.0f64, 0u32);
    while now < seconds {
        while f64::from(start_ms + sent * 100) / 1000.0 <= now + 0.0001 {
            let x = speed * (sent as f32) * 0.1;
            track.push(now, sample(start_ms + sent * 100, x, 0.0, 0.0, false));
            sent += 1;
        }
        out.push(track.sample(now));
        now += dt;
    }
    out
}

#[test]
fn a_steady_vehicle_is_played_back_a_delay_behind() {
    let mut track = VehicleTrack::new(Vec3::ZERO, Quat::IDENTITY);
    let poses = drive(&mut track, 0, 5.0, 3.0, 1.0 / 60.0);
    // Drop the first second (the clock sync settles) and look at the frame steps.
    let tail = &poses[60..];
    let steps: Vec<f32> = tail
        .windows(2)
        .map(|w| w[1].pos.distance(w[0].pos))
        .collect();
    let (min, max) = steps
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), s| (lo.min(*s), hi.max(*s)));
    // 5 m/s at 60 fps is 83 mm a frame; no stall, no jump.
    assert!(min > 0.06 && max < 0.11, "steps {min} .. {max}");
    // The playback runs DELAY behind the sender: at local time t the sample shown is at t-0.2.
    let shown_speed = (tail[tail.len() - 1].pos.x - tail[0].pos.x) / (tail.len() as f32 / 60.0);
    assert!((shown_speed - 5.0).abs() < 0.1, "shown {shown_speed} m/s");
    let last_local = (60 + tail.len() - 1) as f32 / 60.0;
    let expected = 5.0 * (last_local - DELAY as f32);
    assert!(
        (tail[tail.len() - 1].pos.x - expected).abs() < 0.15,
        "{} vs {expected}",
        tail[tail.len() - 1].pos.x
    );
}

#[test]
fn a_vehicle_holds_when_the_reports_stop() {
    let mut track = VehicleTrack::new(Vec3::ZERO, Quat::IDENTITY);
    let mut now = 0.0f64;
    for i in 0..10u32 {
        now = f64::from(i as u16) * 0.1;
        track.push(
            now,
            sample(1000 + i * 100, f32::from(i as u16) * 0.5, 0.0, 0.0, false),
        );
    }
    // The reports stop at x = 4.5; it keeps playing the buffered path, then holds (no
    // extrapolation, FUN_390b2420/FUN_390ad150).
    let mut pose = track.sample(now);
    for _ in 0..120 {
        now += 1.0 / 60.0;
        pose = track.sample(now);
    }
    assert!((pose.pos.x - 4.5).abs() < 1e-4, "held at {}", pose.pos.x);
    assert!(
        pose.pos.x <= 4.5 + 1e-3,
        "extrapolated past the newest sample"
    );
}

#[test]
fn the_short_way_round_is_taken() {
    let mut track = VehicleTrack::new(Vec3::ZERO, Quat::IDENTITY);
    track.push(0.0, sample(1000, 0.0, 0.0, 170f32.to_radians(), true));
    track.push(0.1, sample(1100, 0.0, 0.0, -170f32.to_radians(), true));
    let pose = track.sample(0.35);
    let forward = pose.rot * Vec3::Y;
    let yaw = forward.x.atan2(forward.y);
    assert!(
        yaw.abs() > 175f32.to_radians(),
        "yaw {yaw} rad took the long way"
    );
}

#[test]
fn a_large_clock_jump_re_bases_and_clears_the_queue() {
    let mut track = VehicleTrack::new(Vec3::ZERO, Quat::IDENTITY);
    track.push(0.0, sample(1000, 0.0, 0.0, 0.0, true));
    let outcome = track.push(0.1, sample(600_000, 50.0, 0.0, 0.0, true));
    assert_eq!(outcome, Pushed::Rebased);
    assert_eq!(track.len(), 1);
}

#[test]
fn a_ship_drops_a_stale_report_within_a_second() {
    let mut track = VehicleTrack::new(Vec3::ZERO, Quat::IDENTITY);
    track.push(0.0, sample(10_000, 0.0, 0.0, 0.0, true));
    assert_eq!(
        track.push(0.1, sample(9_500, 1.0, 0.0, 0.0, true)),
        Pushed::Stale
    );
    assert_eq!(track.len(), 1);
    // More than a second older is not a duplicate (the original's wrap guard): it is let
    // through to the clock, which re-bases on the negative deviation.
    assert_eq!(
        track.push(0.2, sample(8_900, 2.0, 0.0, 0.0, true)),
        Pushed::Rebased
    );
}

#[test]
fn a_land_vehicle_keeps_an_out_of_order_report() {
    // The stale check is ship-only (FUN_390b2f30); FUN_390b2e30 queues carts directly.
    let mut track = VehicleTrack::new(Vec3::ZERO, Quat::IDENTITY);
    track.push(0.0, sample(10_000, 0.0, 0.0, 0.0, false));
    assert_eq!(
        track.push(0.1, sample(9_500, 1.0, 0.0, 0.0, false)),
        Pushed::Rebased
    );
}

#[test]
fn a_ship_zone_change_restarts_the_sync() {
    let mut track = VehicleTrack::new(Vec3::ZERO, Quat::IDENTITY);
    let mut first = sample(1000, 0.0, 0.0, 0.0, true);
    first.zone = 7;
    track.push(0.0, first);
    let mut second = sample(1100, 1.0, 0.0, 0.0, true);
    second.zone = 8;
    assert_eq!(track.push(0.1, second), Pushed::ZoneReset);
    assert_eq!(track.len(), 1);
}

#[test]
fn a_teleport_over_twenty_metres_snaps() {
    let mut track = VehicleTrack::new(Vec3::ZERO, Quat::IDENTITY);
    track.push(0.0, sample(1000, 0.0, 0.0, 0.0, true));
    track.push(0.1, sample(2000, 100.0, 0.0, 0.0, true));
    // At play 1300 ms the interpolation is at x = 30, which is > 20 m from the drawn 0: snap to
    // the newest sample instead of gliding across the gap.
    let pose = track.sample(1.5);
    assert!(
        (pose.pos.x - 100.0).abs() < 1e-4,
        "did not snap: {}",
        pose.pos.x
    );
    assert_eq!(track.len(), 1);
}

#[test]
fn five_seconds_without_a_sample_freeze_the_pose() {
    let mut track = VehicleTrack::new(Vec3::ZERO, Quat::IDENTITY);
    track.push(0.0, sample(1000, 0.0, 0.0, 0.0, true));
    track.push(0.1, sample(1100, 1.0, 0.0, 0.0, true));
    let pose = track.sample(0.35);
    let after = track.sample(5.5);
    assert_eq!(pose, after, "the timeout must leave the last pose");
}

#[test]
fn no_sample_keeps_the_announced_pose() {
    let mut track = VehicleTrack::new(Vec3::new(1.0, 2.0, 3.0), Quat::from_rotation_z(0.5));
    let pose = track.sample(10.0);
    assert_eq!(pose.pos, Vec3::new(1.0, 2.0, 3.0));
    assert_eq!(pose.rot, Quat::from_rotation_z(0.5));
    assert_eq!(pose.steering, 0.0);
}

#[test]
fn the_clock_correction_runs_every_frame_with_a_dead_band() {
    let mut track = VehicleTrack::new(Vec3::ZERO, Quat::IDENTITY);
    track.push(0.0, sample(1000, 0.0, 0.0, 0.0, true));
    // A huge jump re-bases: deviation 0, queue cleared.
    assert_eq!(
        track.push(0.1, sample(600_000, 0.0, 0.0, 0.0, true)),
        Pushed::Rebased
    );
    assert_eq!(track.clock().1, 0.0);
    // One frame corrects by 20 % of the 0.29 s excess (not a per-packet step).
    track.sample(0.2);
    let first = track.clock().1;
    assert!(
        (first - 0.058).abs() < 1e-3,
        "one frame corrected to {first}"
    );
    // Each later frame corrects again; inside the dead band the step is 1 ms.
    let mut last = first;
    let mut now = 0.3;
    for _ in 0..30 {
        track.sample(now);
        let deviation = track.clock().1;
        assert!(deviation >= last, "the deviation went backwards");
        assert!(
            deviation - last <= 0.058 + 1e-6,
            "a frame stepped more than 20 %"
        );
        last = deviation;
        now += 0.016;
    }
    assert!(
        last > 0.25,
        "frames did not correct toward the delay: {last}"
    );
    // From far below the band the step is the 20 % excess; near it, exactly 1 ms.
    let mut track = VehicleTrack::new(Vec3::ZERO, Quat::IDENTITY);
    track.push(0.0, sample(1000, 0.0, 0.0, 0.0, true));
    track.push(0.1, sample(600_000, 0.0, 0.0, 0.0, true));
    let mut now = 0.2;
    let mut previous = 0.0;
    while track.clock().1 < DELAY - DEAD_BAND {
        previous = track.clock().1;
        track.sample(now);
        now += 0.016;
    }
    let before = track.clock().1;
    track.sample(now);
    let step = track.clock().1 - before;
    assert!(
        step > 0.0 && step <= NUDGE + 1e-9,
        "the in-band step was {step}"
    );
    assert!(before >= previous);
}

#[test]
fn a_far_newest_sample_snaps_before_the_playback_reaches_it() {
    let mut track = VehicleTrack::new(Vec3::ZERO, Quat::IDENTITY);
    track.push(0.0, sample(1000, 0.0, 0.0, 0.0, false));
    // The new sample is 100 m away and its time is still ahead of the playback clock: the
    // drawn entity is compared with the model position before sampling (FUN_390b0040).
    track.push(0.1, sample(5000, 100.0, 0.0, 0.0, false));
    let pose = track.sample(0.1);
    assert!(
        (pose.pos.x - 100.0).abs() < 1e-4,
        "did not snap immediately: {}",
        pose.pos.x
    );
}

/// The recorded S6/S7 cart drive (case c_left): the server rebroadcasts the driver's type-2
/// body in `SCOneUnitMovement`, so these are received vehicle packets. Every frame must move
/// (no hold inside the stream) and stay near the recorded path.
#[test]
fn the_recorded_cart_drive_is_played_without_stalls_or_jumps() {
    let mut rows = Vec::new();
    for line in include_str!("../../../tests/fixtures/remote-vehicle/cart_c_left.tsv").lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let (ms, hex) = line.split_once('\t').unwrap();
        let bytes: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        let mut r = Reader::new(&bytes);
        assert_eq!(r.bc().unwrap(), 33993);
        let movement = Movement::read(&mut r).unwrap();
        assert!(r.is_empty());
        rows.push((ms.parse::<u32>().unwrap(), movement));
    }
    assert!(rows.len() > 40);
    let first = rows[0].1.clone();
    let mut track = VehicleTrack::new(
        Vec3::new(first.pos.x, first.pos.y, first.pos.z),
        Quat::from_array(first.rotation_quat().expect("type-2 rotation")),
    );
    let mut poses = Vec::new();
    let mut sent = 0;
    let mut now = 0.0f64;
    let last_ms = rows.last().unwrap().0;
    while now < f64::from(last_ms) / 1000.0 + 1.0 {
        while sent < rows.len() && f64::from(rows[sent].0) / 1000.0 <= now {
            let sample = VehicleSample::from_movement(&rows[sent].1).expect("type-2 body");
            track.push(now, sample);
            sent += 1;
        }
        poses.push(track.sample(now));
        now += 1.0 / 60.0;
    }
    // Before the recorded brake (the last samples are the cart standing still) every frame
    // moves; nowhere does a frame jump across a gap.
    let moving = (5.0 * 60.0) as usize;
    let steps: Vec<f32> = poses[90..moving]
        .windows(2)
        .map(|w| w[1].pos.distance(w[0].pos))
        .collect();
    let (min, max) = steps
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), s| (lo.min(*s), hi.max(*s)));
    assert!(
        min > 0.0005,
        "a frame stalled while the recorded cart moved: {min}"
    );
    assert!(max < 0.2, "a frame jumped: {max}");
    let end = poses[poses.len() - 1].pos;
    let last = &rows.last().unwrap().1;
    let recorded_end = Vec3::new(last.pos.x, last.pos.y, last.pos.z);
    assert!(
        end.distance(recorded_end) < 0.05,
        "ended at {end:?}, recorded {recorded_end:?}"
    );
}
