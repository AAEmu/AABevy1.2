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
