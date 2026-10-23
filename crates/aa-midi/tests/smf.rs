use aa_midi::smf::{self, Error, Kind};

fn file(format: u16, division: u16, tracks: &[&[u8]]) -> Vec<u8> {
    let mut d = b"MThd\0\0\0\x06".to_vec();
    d.extend(format.to_be_bytes());
    d.extend((tracks.len() as u16).to_be_bytes());
    d.extend(division.to_be_bytes());
    for track in tracks {
        d.extend(b"MTrk");
        d.extend((track.len() as u32).to_be_bytes());
        d.extend(*track);
    }
    d
}

#[test]
fn shared_tempo_map_and_stable_simultaneous_events() {
    let tempo = [
        0, 0xff, 0x51, 3, 7, 0xa1, 0x20, 0x83, 0x60, 0xff, 0x51, 3, 0x0f, 0x42, 0x40, 0x83, 0x60,
        0xff, 0x2f, 0,
    ];
    let notes = [
        0, 0x90, 60, 100, 0x83, 0x60, 0x80, 60, 0, 0, 0x90, 62, 90, 0x83, 0x60, 0x80, 62, 0, 0,
        0xff, 0x2f, 0,
    ];
    let s = smf::parse(&file(1, 480, &[&tempo, &notes])).unwrap();
    assert_eq!(
        s.events.iter().map(|e| e.ms).collect::<Vec<_>>(),
        [0, 500, 500, 1500]
    );
    assert_eq!(s.events[1].kind, Kind::NoteOff { note: 60 });
    assert_eq!(s.length_ms(), 1500);
}

#[test]
fn running_status_velocity_zero_and_end_silence() {
    let t = [0, 0x92, 60, 100, 100, 60, 0, 100, 0xff, 0x2f, 0];
    let s = smf::parse(&file(0, 100, &[&t])).unwrap();
    assert_eq!(s.events[1].ms, 500);
    assert_eq!(s.events[1].channel, 2);
    assert_eq!(s.events[1].kind, Kind::NoteOff { note: 60 });
    assert_eq!(s.length_ms(), 1000);
}

#[test]
fn fractional_time_is_integrated_before_rounding() {
    // Three ticks at 1/3 ms each, across two tempo boundaries.
    let t = [
        0, 0xff, 0x51, 3, 0, 3, 0xe8, 1, 0xff, 0x51, 3, 0, 3, 0xe8, 1, 0xff, 0x51, 3, 0, 3, 0xe8,
        1, 0x90, 60, 1, 0, 0xff, 0x2f, 0,
    ];
    let s = smf::parse(&file(0, 3, &[&t])).unwrap();
    assert_eq!(s.events[0].ms, 1);
}

#[test]
fn rejects_invalid_headers_and_event_encodings() {
    let end = [0, 0xff, 0x2f, 0];
    for d in [
        file(2, 500, &[&end]),
        file(0, 0, &[&end]),
        file(0, 500, &[]),
        file(0, 500, &[&end, &end]),
    ] {
        assert!(smf::parse(&d).is_err());
    }
    assert_eq!(smf::parse(&file(0, 0x8001, &[&end])), Err(Error::Smpte));
    for track in [
        vec![0, 60, 100],                              // no running status
        vec![0, 0x90, 128, 1],                         // status in data
        vec![0xff, 0xff, 0xff, 0xff, 0],               // overlong delta
        vec![0, 0xf0, 10, 0],                          // truncated sysex
        vec![0, 0xff, 1, 10, 0],                       // truncated metadata
        vec![0, 0xff, 0x51, 3, 0, 0, 0],               // invalid tempo
        vec![0, 0xff, 0x2f, 1, 0],                     // invalid end
        vec![0, 0x90, 60, 100],                        // missing end
        vec![0, 0x90, 60, 1, 0, 0xff, 1, 0, 0, 60, 0], // meta clears running status
        vec![0, 0xf1],
    ] {
        assert!(smf::parse(&file(0, 500, &[&track])).is_err(), "{track:?}");
    }
}

#[test]
fn every_truncated_prefix_is_rejected() {
    let d = file(
        0,
        500,
        &[&[0, 0x90, 60, 1, 0x83, 0x74, 0x80, 60, 0, 0, 0xff, 0x2f, 0]],
    );
    for n in 0..d.len() {
        assert!(smf::parse(&d[..n]).is_err(), "prefix {n}");
    }
    let mut d = d;
    d[7] = 5;
    assert!(smf::parse(&d).is_err());
}

#[test]
fn resource_and_duration_limits_are_errors() {
    assert_eq!(smf::parse(&vec![0; smf::MAX_BYTES + 1]), Err(Error::Limit));
    let mut track = Vec::new();
    for _ in 0..smf::MAX_EVENTS {
        track.extend([0, 0xd0, 0]);
    }
    track.extend([0, 0xff, 0x2f, 0]);
    assert_eq!(smf::parse(&file(0, 500, &[&track])), Err(Error::Limit));
    assert_eq!(
        smf::parse(&file(0, 1, &[&[0xff, 0xff, 0xff, 0x7f, 0xff, 0x2f, 0]])),
        Err(Error::Limit)
    );
}
