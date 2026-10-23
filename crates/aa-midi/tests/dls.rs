use aa_midi::dls::{Dls, Error};

fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut d = id.to_vec();
    d.extend((body.len() as u32).to_le_bytes());
    d.extend(body);
    if !body.len().is_multiple_of(2) {
        d.push(0);
    }
    d
}
fn list(kind: &[u8; 4], parts: &[Vec<u8>]) -> Vec<u8> {
    let mut d = kind.to_vec();
    for p in parts {
        d.extend(p);
    }
    chunk(b"LIST", &d)
}
fn sample(unity: u16, loops: u32, start: u32, length: u32) -> Vec<u8> {
    let mut d = 20u32.to_le_bytes().to_vec();
    d.extend(unity.to_le_bytes());
    d.extend((-7i16).to_le_bytes());
    d.extend((-60i32 * 65536).to_le_bytes());
    d.extend(0u32.to_le_bytes());
    d.extend(loops.to_le_bytes());
    if loops != 0 {
        for v in [16u32, 0, start, length] {
            d.extend(v.to_le_bytes());
        }
    }
    d
}
fn bank(encoding: u16, channels: u16, bits: u16, loop_length: u32) -> Vec<u8> {
    let mut fmt = encoding.to_le_bytes().to_vec();
    fmt.extend(channels.to_le_bytes());
    fmt.extend(24000u32.to_le_bytes());
    fmt.extend(48000u32.to_le_bytes());
    fmt.extend((bits / 8).to_le_bytes());
    fmt.extend(bits.to_le_bytes());
    let wave = list(
        b"wave",
        &[
            chunk(b"data", &[0, 128, 255, 127]), // deliberately before fmt; order is immaterial
            chunk(b"fmt ", &fmt),
            chunk(b"wsmp", &sample(60, 0, 0, 0)),
        ],
    );
    let pool = list(b"wvpl", &[wave.clone(), wave.clone()]);
    let mut cues = 12u32.to_le_bytes().to_vec(); // extended header, not the usual eight bytes
    cues.extend(2u32.to_le_bytes());
    cues.extend([0u8; 4]);
    cues.extend((wave.len() as u32).to_le_bytes());
    cues.extend(0u32.to_le_bytes());
    let region = list(
        b"rgn ",
        &[
            chunk(b"rgnh", &[0, 0, 127, 0, 0, 0, 127, 0, 0, 0, 3, 0]),
            chunk(b"wlnk", &[0; 12]),
            chunk(b"wsmp", &sample(69, 1, 0, loop_length)),
        ],
    );
    let mut insh = 1u32.to_le_bytes().to_vec();
    insh.extend(0u32.to_le_bytes());
    insh.extend(73u32.to_le_bytes());
    let instruments = list(
        b"lins",
        &[list(
            b"ins ",
            &[chunk(b"insh", &insh), list(b"lrgn", &[region])],
        )],
    );
    let mut body = b"DLS ".to_vec();
    body.extend(chunk(b"colh", &1u32.to_le_bytes()));
    body.extend(instruments);
    body.extend(chunk(b"ptbl", &cues));
    body.extend(pool);
    chunk(b"RIFF", &body)
}

#[test]
fn cues_extended_header_region_override_and_pcm_formats() {
    let b = Dls::parse(&bank(1, 1, 16, 2)).unwrap();
    let r = &b.instruments[0].regions[0];
    assert_eq!(r.wave, 1); // first cue points at the second wave
    assert_eq!(r.unity, 69);
    assert_eq!(r.fine_tune, -7);
    assert_eq!(r.gain_db, -6.0);
    assert_eq!(r.looped, Some((0, 2)));
    assert_eq!(r.key_group, 3);
    assert_eq!(&*b.waves[0].pcm, &[-32768, 32767]);
    let b = Dls::parse(&bank(1, 1, 8, 4)).unwrap();
    assert_eq!(&*b.waves[0].pcm, &[-32768, 0, 32512, -256]);
}

#[test]
fn malformed_bank_fields_and_unsupported_formats_are_errors() {
    for data in [bank(3, 1, 16, 2), bank(1, 2, 16, 2), bank(1, 1, 24, 2)] {
        assert!(matches!(Dls::parse(&data), Err(Error::Unsupported(_))));
    }
    assert!(Dls::parse(&bank(1, 1, 16, 0)).is_err());
    assert!(Dls::parse(&bank(1, 1, 16, 3)).is_err());
    let data = bank(1, 1, 16, 2);
    for n in 0..data.len() {
        assert!(Dls::parse(&data[..n]).is_err(), "prefix {n}");
    }
    for (tag, field, value) in [
        (b"ptbl", 4, u32::MAX),
        (b"wlnk", 8, 99),
        (b"wsmp", 0, 1),
        (b"insh", 0, 5),
    ] {
        let mut changed = data.clone();
        let at = changed.windows(4).position(|p| p == tag).unwrap() + 8 + field;
        changed[at..at + 4].copy_from_slice(&value.to_le_bytes());
        assert!(Dls::parse(&changed).is_err(), "{tag:?}");
    }
}

#[test]
#[ignore = "reads the original bank through aa-pak"]
fn original_bank_structure_and_program_selection() {
    let bytes = aa_pak::Pak::global().read("sounds/default.dls").unwrap();
    assert_eq!(bytes.len(), 3_440_660);
    let b = Dls::parse(&bytes).unwrap();
    assert_eq!(b.waves.len(), 495);
    assert_eq!(b.instruments.len(), 235);
    assert_eq!(
        b.instruments.iter().map(|i| i.regions.len()).sum::<usize>(),
        1498
    );
    assert_eq!(b.waves.iter().filter(|w| w.looped.is_some()).count(), 439);
    for program in 0..=127 {
        assert_eq!(b.instrument(program, false).unwrap().bank, 0);
    }
    assert_eq!(b.instrument(35, true).unwrap().program, 32);
    for i in &b.instruments {
        for r in &i.regions {
            assert!(r.wave < b.waves.len());
            if let Some((s, l)) = r.looped {
                assert!(l > 0 && (s + l) as usize <= b.waves[r.wave].pcm.len());
            }
        }
    }
    eprintln!(
        "original DLS: {} instruments, {} waves, 1498 regions, 439 wave loops",
        b.instruments.len(),
        b.waves.len()
    );
}
