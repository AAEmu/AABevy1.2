//! Tests against the real game data. They need the pak (`AA_PAK` or `D:/AA1.2/game_pak`):
//! `cargo test -p aa-audio -- --ignored`.

use aa_audio::fev::{Body, Fev, Rolloff, Wave};
use aa_audio::library::{PickState, Rng, initial_values, layer_gain, plan};
use aa_audio::{Fsb5, Library, decode};

#[test]
#[ignore = "needs the game pak"]
fn vorbis_bank_header_and_subsound_decode() {
    let pak = aa_pak::Pak::global();
    let name = "sounds/x2interface/x2interface_bank00.fsb";
    let name = if pak.exists(name) { name.to_string() } else { pak.list("sounds/x2interface/").find(|n| n.ends_with(".fsb")).unwrap().to_string() };
    let bytes = pak.read(&name).unwrap();
    let fsb = Fsb5::parse(&bytes).unwrap();
    assert_eq!(fsb.codec, aa_audio::fsb5::Codec::Vorbis, "{name}");
    assert!(!fsb.subsounds.is_empty());
    let sub = &fsb.subsounds[0];
    // the Vorbis chunk names a setup header the crate knows
    let crc = sub.vorbis_crc.expect("Vorbis chunk");
    assert!(aa_audio::vorbis::setup_header(crc).is_some(), "setup {crc:#x}");
    let pcm = decode(&fsb, &bytes, 0).unwrap();
    assert_eq!(pcm.frames(), sub.samples as usize, "decoded length equals the header's");
    assert_eq!(pcm.channels, sub.channels);
    assert!(pcm.rms() > 1e-4, "not silent");
}

#[test]
#[ignore = "needs the game pak"]
fn mpeg_voice_subsound_decodes() {
    let pak = aa_pak::Pak::global();
    let name = pak.list("sounds/x2voice_system/en_us/").find(|n| n.ends_with(".fsb")).expect("a voice bank").to_string();
    let bytes = pak.read(&name).unwrap();
    let fsb = Fsb5::parse(&bytes).unwrap();
    assert_eq!(fsb.codec, aa_audio::fsb5::Codec::Mpeg);
    let pcm = decode(&fsb, &bytes, 0).unwrap();
    let want = fsb.subsounds[0].samples as f64;
    assert!((pcm.frames() as f64 / want - 1.0).abs() < 0.05, "{} vs {want}", pcm.frames());
    assert!(pcm.rms() > 1e-4);
}

#[test]
#[ignore = "needs the game pak"]
fn fev_voice_system_parses_completely() {
    let pak = aa_pak::Pak::global();
    let fev = Fev::parse(&pak.read("sounds/x2voice_system/x2voice_system.fev").unwrap()).unwrap();
    assert_eq!(fev.name, "x2voice_system");
    assert_eq!(fev.events.len(), 17);
    assert_eq!(fev.banks.len(), 9);
    assert!(fev.events.iter().all(|e| e.is_simple() && !e.is_3d()));
    assert!(fev.categories.paths().iter().any(|p| p == "master/system_voice"));
    // every wave points into a bank of the project
    for def in &fev.sound_defs {
        for w in &def.waves {
            if let Wave::Bank { bank, .. } = w {
                assert!((*bank as usize) < fev.banks.len());
            }
        }
    }
}

#[test]
#[ignore = "needs the game pak"]
fn every_fev_in_the_pak_parses() {
    let pak = aa_pak::Pak::global();
    let names: Vec<String> = pak.list("sounds/").filter(|n| n.ends_with(".fev")).map(str::to_string).collect();
    assert_eq!(names.len(), 69);
    for n in names {
        Fev::parse(&pak.read(&n).unwrap()).unwrap_or_else(|e| panic!("{n}: {e}"));
    }
}

#[test]
#[ignore = "needs the game pak"]
fn original_pan_and_highpass_inventory() {
    use std::collections::{BTreeMap, BTreeSet};
    let pak = aa_pak::Pak::global();
    let (mut files, mut pan, mut distance) = (0, 0, 0);
    let mut events = BTreeSet::new();
    let mut projects = BTreeMap::new();
    let mut highpass = Vec::new();
    for name in pak.list("sounds/").filter(|n| n.ends_with(".fev")) {
        files += 1;
        let fev = Fev::parse(&pak.read(name).unwrap()).unwrap();
        for ev in &fev.events {
            let Body::Complex { layers, parameters, .. } = &ev.body else { continue };
            for (li, layer) in layers.iter().enumerate() {
                let mut layer_pan = 0;
                for env in &layer.envelopes {
                    if env.flags & 0x400 != 0 {
                        assert!(ev.is_3d() && !env.points.is_empty(), "{name}:{} layer {li}", ev.path());
                        assert_eq!(env.flags, 0x404);
                        let param = &parameters[env.parameter as usize].name;
                        if param == "distance" {
                            distance += 1;
                        } else {
                            assert_eq!((fev.name.as_str(), ev.path(), param.as_str()),
                                ("x2eventsounds", "weather/origin_continent_effect_weather".into(), "param01"));
                        }
                        pan += 1;
                        layer_pan += 1;
                        events.insert((name.to_owned(), ev.path()));
                        *projects.entry(fev.name.clone()).or_insert(0) += 1;
                    }
                    if env.dsp == "FMOD Highpass" {
                        assert_eq!((env.flags, env.dsp_param, env.points.len()), (4, 0, 1));
                        highpass.push(format!("{}:{}", fev.name, ev.path()));
                    }
                }
                assert!(layer_pan <= 1, "the original inventory has one pan curve per layer");
            }
        }
    }
    highpass.sort();
    assert_eq!((files, pan, events.len(), projects.len(), distance), (69, 666, 655, 18, 665));
    assert_eq!(highpass, ["x2ambience:login_stage/countryside02", "x2ambience:login_stage/mar03",
        "x2ambience:world2_day/countryside02", "x2ambience:world2_night/wind_night1", "x2weapons:x2weapons/shield_metal"]);
    eprintln!("original pan inventory: files={files} references={pan} file/events={} projects={} distance={distance} per_project={projects:?}; deferred high-pass={highpass:?}", events.len(), projects.len());
}

#[test]
#[ignore = "needs the game pak"]
fn original_playback_policy_survey() {
    let pak = aa_pak::Pak::global();
    let mut counts = [0usize; 6];
    for name in pak.list("sounds/").filter(|n| n.ends_with(".fev")) {
        let fev = Fev::parse(&pak.read(name).unwrap()).unwrap();
        for e in &fev.events {
            counts[e.playback_behaviour as usize] += 1;
            if matches!(e.path().as_str(), "quest_elf/stone_mark_beam" | "prefab/mining_machinery_idle") {
                eprintln!("policy {} max {} {name}:{} volume {} fade {}/{} category {} instance gains {:?}",
                    e.playback_behaviour, e.max_playbacks, e.path(), e.volume, e.fade_in_ms, e.fade_out_ms,
                    e.category, e.instances().iter().map(|i| f32::from_bits(i.raw[5])).collect::<Vec<_>>());
            }
        }
    }
    eprintln!("original playback policies 0..5: {counts:?}");
    assert_eq!(counts, [0, 1514, 0, 476, 7152, 320]);
}

#[test]
#[ignore = "needs the game pak"]
fn skill_event_resolves_and_plans() {
    let lib = Library::global();
    let ev = lib.event("Sounds/X2Skills:Wild:Fire_Arrow_Form").expect("event");
    assert_eq!(ev.event().name, "fire_arrow_form");
    assert!(ev.event().is_3d());
    assert_eq!(ev.event().rolloff(), Rolloff::Custom);
    assert_eq!(ev.event().category, "master/skill");
    let values = initial_values(ev.event(), Some(5.0), &[]);
    let planned = plan(&ev, &values, &mut PickState::default(), &mut Rng::new(3));
    assert_eq!(planned.len(), 2, "two layers");
    // the custom rolloff is a volume curve on the distance parameter: loud near, silent at the end
    assert!(layer_gain(ev.event(), &values, 0) > 0.5);
    let far = initial_values(ev.event(), Some(20.0), &[]);
    assert!(layer_gain(ev.event(), &far, 0) < 0.01);
    for p in &planned {
        let bank = lib.bank(&ev.project, p.bank).unwrap();
        let pcm = lib.decode(&bank, p.index as usize).unwrap();
        assert!(pcm.frames() > 1000);
    }
    // an event the project lacks and a project the pak lacks both resolve to nothing
    assert!(lib.event("sounds/x2skills:monster_skill:ground_launch_end").is_none());
    assert!(lib.event("sounds/x2voice_prologue:system:6170").is_none());
}

#[test]
#[ignore = "needs the game pak"]
fn ui_event_is_simple_and_2d() {
    let lib = Library::global();
    let ev = lib.event("sounds/x2interface:login_stage:button_login").expect("event");
    assert!(matches!(ev.event().body, Body::Simple(_)));
    let planned = plan(&ev, &[], &mut PickState::default(), &mut Rng::new(1));
    assert_eq!(planned.len(), 1);
    assert!(!planned[0].looping, "a button click plays once");
}

#[test]
#[ignore = "needs the game pak"]
fn layer_curves_follow_the_project() {
    use aa_audio::library::{layer_cutoff, layer_filtered, layer_pitch};
    let lib = Library::global();
    let layers = |ev: &aa_audio::fev::Event| match &ev.body {
        Body::Complex { layers, .. } => layers.len(),
        Body::Simple(_) => 1,
    };

    // a constant DSP low-pass of an ambience bed: 0.80 on the log scale between 10 Hz and 22 kHz is about 4.7 kHz
    let e = lib.event("sounds/x2ambience:world2_day:field_day2").expect("ambience");
    let ev = e.event();
    let values = initial_values(ev, None, &[]);
    let cutoffs: Vec<f32> = (0..layers(ev)).filter(|&l| layer_filtered(ev, l)).filter_map(|l| layer_cutoff(ev, &values, l)).collect();
    assert!(cutoffs.iter().any(|c| (4000.0..5500.0).contains(c)), "{cutoffs:?}");

    // a pitch curve: the single point 0.47 of the reload is a quarter octave down
    let e = lib.event("sounds/x2eventsounds:siege_weapons:palinton_a_reload").expect("reload");
    let ev = e.event();
    let values = initial_values(ev, None, &[]);
    let pitches: Vec<f32> = (0..layers(ev)).map(|l| layer_pitch(ev, &values, l)).collect();
    assert!(pitches.iter().any(|p| (p - 0.84).abs() < 0.02), "{pitches:?}");

    // the distance occlusion of a 3D event: clear at the listener, filtering at the far end
    let e = lib.event("sounds/x2eventsounds:quest:cat_cry_2").expect("cat");
    let ev = e.event();
    assert!(ev.is_3d());
    let far = initial_values(ev, Some(ev.max_distance), &[]);
    let near = initial_values(ev, Some(0.0), &[]);
    assert!(layer_filtered(ev, 0), "the layer has an occlusion curve");
    assert_eq!(layer_cutoff(ev, &near, 0), None);
    assert!(layer_cutoff(ev, &far, 0).is_some_and(|c| c < 18000.0), "{:?}", layer_cutoff(ev, &far, 0));
}

#[test]
#[ignore = "needs the game pak"]
fn original_daylight_time_and_distance_curves_follow_native_shapes() {
    let lib = Library::global();
    let field = lib.event("sounds/x2ambience:world2_day:arche_field_day").unwrap();
    // daylight 0.4 -> 0.55, 0 -> 1: destination flag 1 uses the native smooth curve.
    let daylight = initial_values(field.event(), None, &[("daylight".into(), 0.4375)]);
    assert!((layer_gain(field.event(), &daylight, 0) - 0.15625).abs() < 1e-5);
    // The first time_loop segment ends at x=0.174757, y=0.742049 with flag 8.
    // At a quarter of the segment the inverse cubic weight is 0.4375.
    let time = initial_values(field.event(), None, &[("daylight".into(), 1.0), ("time_loop".into(), 1.5291238)]);
    assert!((layer_gain(field.event(), &time, 0) - 0.8871464).abs() < 1e-5);

    let cat = lib.event("sounds/x2eventsounds:quest:cat_cry_2").unwrap();
    // Distance 3.19578 -> 20 m, gain 0.945017 -> 0: flag 2 is linear.
    let distance = initial_values(cat.event(), Some(7.396835), &[]);
    assert!((layer_gain(cat.event(), &distance, 0) - 0.70876275).abs() < 1e-5);
}

#[test]
#[ignore = "needs the game pak"]
fn spawning_playlists_and_long_loops_are_planned() {
    let lib = Library::global();
    // gusts that spawn while the layer is active: 1 to 3 s apart, three at most
    let e = lib.event("sounds/x2ambience:level_bronzerock:winter_wind").expect("winter wind");
    let values = initial_values(e.event(), None, &[]);
    let (mut pick, mut rng) = (PickState::default(), Rng::new(3));
    let spawning: Vec<_> = (0..4).filter_map(|i| aa_audio::library::start_part(&e, 1, i, &mut pick, &mut rng)).filter_map(|p| p.spawn_ms.map(|s| (s, p.max_spawned))).collect();
    assert!(spawning.iter().all(|&(s, max)| s == (1000, 3000) && max == 3) && !spawning.is_empty(), "{spawning:?} {values:?}");

    // the combat music is a looping playlist of long tracks (they stream)
    let e = lib.event("sounds/x2music:conditions_bgm:battle_normal").expect("battle music");
    let values = initial_values(e.event(), None, &[]);
    let planned = plan(&e, &values, &mut pick, &mut rng);
    assert!(planned.iter().any(|p| p.looping && p.playlist && p.length_ms >= 10_000), "{planned:?}");
}

#[test]
#[ignore = "needs the game pak"]
fn cones_are_inaudible_and_the_only_oscillator_is_an_authoring_test() {
    let pak = aa_pak::Pak::global();
    let mut cones = 0;
    let mut tones = Vec::new();
    for name in pak.list("sounds/").filter(|n| n.ends_with(".fev")).map(str::to_string).collect::<Vec<_>>() {
        let fev = Fev::parse(&pak.read(&name).unwrap()).unwrap();
        // a cone only shapes the sound when the volume outside it differs from the volume inside
        cones += fev.events.iter().filter(|e| e.is_3d() && (e.cone_inside < 359.0 || e.cone_outside < 359.0) && e.cone_outside_volume < 0.99).count();
        for event in &fev.events {
            for instance in event.instances() {
                for wave in &fev.sound_defs[instance.sound_def].waves {
                    if let Wave::Oscillator { kind, rate, .. } = wave {
                        tones.push((fev.name.clone(), event.path(), *kind, f32::from_bits(*rate)));
                    }
                }
            }
        }
    }
    assert_eq!(cones, 0, "cones play no part in the game data");
    assert_eq!(tones, [("x2ambience".into(), "world2_merchant/test".into(), 0, 440.0)]);
}
