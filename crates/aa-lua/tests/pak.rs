//! Checks against the real `game_pak` (`AA_PAK` or `<client>/game_pak`).

use aa_lua::{Stage, chunk, toc_files};
use aa_pak::Pak;

fn albs() -> Vec<String> {
    let mut v: Vec<String> = Pak::global()
        .list("scriptsbin/")
        .filter(|n| n.to_ascii_lowercase().ends_with(".alb"))
        .map(str::to_string)
        .collect();
    v.sort();
    v
}

fn tocs() -> Vec<String> {
    let mut v: Vec<String> = Pak::global()
        .list("scriptsbin/")
        .filter(|n| n.to_ascii_lowercase().ends_with("/toc.g"))
        .map(str::to_string)
        .collect();
    v.sort();
    v
}

/// Every `.alb` parses, uses only stock 5.1 opcodes, round-trips, converts and compiles.
#[test]
fn every_alb_converts_and_loads() {
    let pak = Pak::global();
    let lua = aa_lua::new_lua().unwrap();
    let files = albs();
    assert!(files.len() >= 961, "only {} .alb files found", files.len());
    let mut failures = Vec::new();
    for name in &files {
        let bytes = pak.read(name).unwrap();
        let parsed = match chunk::parse(&bytes) {
            Ok(c) => c,
            Err(e) => {
                failures.push(format!("{name}: parse: {e:#}"));
                continue;
            }
        };
        assert_eq!(parsed.format, chunk::Format::ALB, "{name}");
        // Writing back in the original layout reproduces the file byte for byte.
        assert_eq!(chunk::write(&parsed, chunk::Format::ALB).unwrap(), bytes, "{name} round trip");
        let mut bad_op = None;
        parsed.main.walk(&mut |p| {
            if let Some(&i) = p.code.iter().find(|&&i| (i & 0x3f) as usize >= chunk::OPCODES.len()) {
                bad_op = Some(i & 0x3f);
            }
        });
        if let Some(op) = bad_op {
            failures.push(format!("{name}: opcode {op} out of the 5.1 range"));
        }
        if let Err(e) = aa_lua::load_alb(&lua, name) {
            failures.push(format!("{name}: {e:#}"));
        }
    }
    assert!(failures.is_empty(), "{} of {} failed:\n{}", failures.len(), files.len(), failures.join("\n"));
}

/// Every entry of every `toc.g` resolves to a file in the pak.
#[test]
fn every_toc_entry_exists() {
    let pak = Pak::global();
    let all = tocs();
    assert!(all.len() >= 56, "only {} toc.g files found", all.len());
    let mut missing = Vec::new();
    let mut n = 0;
    for toc in &all {
        for f in toc_files(toc).unwrap() {
            n += 1;
            if !pak.exists(&f) {
                missing.push(format!("{toc}: {f}"));
            }
        }
    }
    assert!(missing.is_empty(), "{} of {n} toc entries missing:\n{}", missing.len(), missing.join("\n"));
}

#[test]
fn stage_lists_resolve() {
    let pak = Pak::global();
    for stage in Stage::ALL {
        let scripts = stage.scripts().unwrap();
        assert!(!scripts.is_empty(), "{stage:?}");
        for s in &scripts {
            assert!(pak.exists(s), "{stage:?}: {s} missing");
        }
    }
    let ws = Stage::WorldSelect.scripts().unwrap();
    assert_eq!(ws.last().unwrap(), "scriptsbin/x2ui/loginstage/world_select/world_select.alb");
    assert!(ws.contains(&"scriptsbin/x2ui/loginstage/baselib.alb".to_string()));
    let world = Stage::World.units();
    assert!(world.iter().any(|u| u == "scriptsbin/x2ui/hud/toc.g"));
    assert!(!world.iter().any(|u| u.contains("loginstage")));
}

/// Without any native API the scripts still run until they touch it, and errors name the file.
#[test]
fn run_reports_script_errors() {
    let lua = aa_lua::new_lua().unwrap();
    // Pure data file: runs fine.
    aa_lua::run_alb(&lua, "scriptsbin/x2ui/baselib/color.alb").unwrap();
    let err = aa_lua::run_toc(&lua, "scriptsbin/x2ui/loginstage/world_select/toc.g").unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("scriptsbin/x2ui/loginstage/world_select/world_select.alb"), "{msg}");
}

/// Float constants come out as the decimal literal (0.7, not 0.69999998807907).
#[test]
fn numbers_widen_cleanly() {
    let lua = aa_lua::new_lua().unwrap();
    let f = lua.load("return 0.7, 1/3, 2^24").into_function().unwrap();
    let dumped = f.dump(false);
    // Re-encode the host chunk with float numbers, as the client's compiler would have.
    let c = chunk::parse(&dumped).unwrap();
    let alb = chunk::write(&c, chunk::Format::ALB).unwrap();
    let back = aa_lua::load_alb_bytes(&lua, &alb, "test").unwrap();
    let (a, b, c): (f64, f64, f64) = back.call(()).unwrap();
    assert_eq!(a, 0.7);
    assert!((b - 1.0 / 3.0).abs() < 1e-7);
    assert_eq!(c, 16777216.0);
}
