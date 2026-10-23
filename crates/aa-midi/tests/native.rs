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
fn compiler_bounds_and_parameters() {
    let voice = Voice::melodic(0);
    for text in [
        "C".repeat(1001),
        format!("C{}", ".".repeat(100)),
        format!("L1{} C", ".".repeat(70)),
    ] {
        assert_eq!(mml::compile(&text, voice, 1.0, 1), Err(Error::TooLong));
    }
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
