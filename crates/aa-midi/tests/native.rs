use aa_midi::mml::{self, Error, Voice};

#[test]
fn original_x86_compiler_fixtures() {
    let mut checked = 0;
    for line in include_str!("native.tsv")
        .lines()
        .filter(|l| !l.starts_with('#'))
    {
        let fields: Vec<_> = line.split('\t').collect();
        assert_eq!(fields.len(), 7);
        let text = fields[0].replace("\\0", "\0");
        let voice = Voice {
            program: fields[3].parse().unwrap(),
            channel: fields[4].parse().unwrap(),
            octave: fields[5].parse().unwrap(),
        };
        let result = mml::compile(
            &text,
            voice,
            fields[1].parse().unwrap(),
            fields[2].parse().unwrap(),
        );
        match fields[6] {
            "Code" => assert!(
                matches!(result, Err(Error::Code(_))),
                "{text:?}: {result:?}"
            ),
            "Failure" => assert_eq!(result, Err(Error::Empty), "{text:?}"),
            // Native accepts an invalid, trackless header; the recovered backend rejects it.
            "4d546864000000060000000001f4" => assert_eq!(result, Err(Error::Empty), "{text:?}"),
            expected => {
                let bytes = result.unwrap_or_else(|e| panic!("{text:?}: {e}"));
                let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
                assert_eq!(hex, expected, "{text:?}");
            }
        }
        checked += 1;
    }
    assert_eq!(checked, 54);
}

#[test]
fn original_converter_and_track_boundaries() {
    let mut checked = 0;
    for line in include_str!("native_limits.tsv")
        .lines()
        .filter(|line| !line.starts_with('#'))
    {
        let fields: Vec<_> = line.split('\t').collect();
        assert_eq!(fields.len(), 5);
        let name = fields[0];
        let mut text = String::new();
        for run in fields[1].split_whitespace() {
            let (code, count) = run.split_once('*').unwrap();
            let c = char::from_u32(u32::from_str_radix(code, 16).unwrap()).unwrap();
            text.extend(std::iter::repeat_n(c, count.parse::<usize>().unwrap()));
        }
        assert_eq!(
            text.chars().count(),
            fields[2].parse::<usize>().unwrap(),
            "{name}"
        );
        let result = mml::compile(&text, Voice::melodic(73), 1.0, 1);
        if fields[4] == "TooLong" {
            assert_eq!(result, Err(Error::TooLong), "{name}");
        } else {
            assert_eq!(fields[4], "Native", "{name}");
            match fields[3] {
                "Code" => assert!(matches!(result, Err(Error::Code(_))), "{name}: {result:?}"),
                expected => {
                    let bytes = result.unwrap_or_else(|e| panic!("{name}: {e}"));
                    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
                    assert_eq!(hex, expected, "{name}");
                    // The larger accepted scores remain valid SMF for downstream playback.
                    aa_midi::smf::parse(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
                }
            }
        }
        checked += 1;
    }
    assert_eq!(checked, 27);
    // The original editor/save bound remains available to callers, independently.
    assert_eq!(mml::MAX_CHARACTERS, 1000);
    let three = [
        "C".to_owned() + &" ".repeat(998),
        "D".to_owned() + &" ".repeat(998),
        "E".to_owned() + &" ".repeat(998),
    ]
    .join(",");
    // aa-client's wrapper truncates first; the lower-level compiler rejects unconverted excess.
    assert_eq!(
        mml::compile(&(three + ",D"), Voice::melodic(73), 1.0, 1),
        Err(Error::TooLong)
    );
}

#[test]
fn compiler_bounds_and_parameters() {
    let voice = Voice::melodic(0);
    for text in [
        "C".repeat(1001),
        format!("C{}", ".".repeat(100)),
        format!("L1{} C", ".".repeat(70)),
    ] {
        assert_eq!(mml::compile(&text, voice, 1.0, 1), Err(Error::TooLong));
    }
    // Three individually bounded tracks can exceed the final 32-KiB output limit.
    let prefix = format!("L1{}", ".".repeat(18));
    let tracks = [
        format!("{prefix}{}", "C".repeat(999 - prefix.len())),
        "D".repeat(999),
        "E".repeat(999),
    ];
    assert_eq!(
        mml::compile(&tracks.join(","), voice, 1.0, 1),
        Err(Error::TooLong)
    );
    for quality in [f32::NAN, f32::INFINITY, -1.0, 1.01] {
        assert!(matches!(
            mml::compile("C", voice, quality, 1),
            Err(Error::InvalidParameter(_))
        ));
    }
    assert!(matches!(
        mml::compile("C", Voice::melodic(128), 1.0, 1),
        Err(Error::InvalidParameter(_))
    ));
}
