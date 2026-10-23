//! Sound tool: survey the banks, list events, resolve event paths, write WAVs, play events.
//!
//! ```text
//! cargo run -p aa-audio --release --example sound -- --survey [--prefix sounds/x2skills] [--threads 4]
//! cargo run -p aa-audio --release --example sound -- --fevs
//! cargo run -p aa-audio --release --example sound -- --list x2skills [--filter fire_arrow]
//! cargo run -p aa-audio --release --example sound -- --resolve data/extract/aa-audio/db_sound_paths.txt
//! cargo run -p aa-audio --release --example sound -- --wav sounds/x2skills:wild:fire_arrow_form shots/aa-audio/skill_hit.wav [--seconds 8]
//! cargo run -p aa-audio --release --example sound -- --wav sounds/x2skills/x2skills_bank571.fsb:0 out.wav
//! cargo run -p aa-audio --release --example sound -- --wav sounds/x2skills:wild:fire_arrow_form out.wav --reverb x2_cave_1[:weight]
//! cargo run -p aa-audio --release --example sound -- --reverbs
//! cargo run -p aa-audio --release --example sound -- --event sounds/x2interface:interface:button_default --sound [--volume 0.3] [--seconds 4]
//! ```
//!
//! `--event` only plays with `--sound` (and stays short and quiet by default): without it the event is
//! resolved and planned but the engine is muted.

use std::collections::{BTreeMap, HashSet};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use aa_audio::fev::{Body, Fev, Wave};
use aa_audio::library::{PickState, Rng, distance_gain, initial_values, layer_gain, plan};
use aa_audio::{Engine, Fsb5, Library, Pcm, Settings, Volumes, decode};
use bevy::math::Vec3;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
    let has = |flag: &str| args.iter().any(|a| a == flag);
    if has("--survey") {
        survey(&value("--prefix").unwrap_or_else(|| "sounds/".into()), value("--threads").and_then(|t| t.parse().ok()).unwrap_or(4));
    } else if has("--loopstats") {
        let pak = aa_pak::Pak::global();
        let mut m: BTreeMap<(String, u32), (usize, String)> = BTreeMap::new();
        for name in pak.list("sounds/").filter(|n| n.ends_with(".fev")).map(str::to_string).collect::<Vec<_>>() {
            let f = Fev::parse(&pak.read(&name).unwrap()).unwrap();
            for e in &f.events {
                for i in e.instances() {
                    let r = m.entry((e.category.clone(), i.loop_word)).or_insert((0, e.path()));
                    r.0 += 1;
                }
            }
        }
        for ((cat, w), (n, ex)) in m {
            println!("{cat:28} loop word {w:#x}: {n:6} e.g. {ex}");
        }
    } else if has("--params") {
        let pak = aa_pak::Pak::global();
        let mut m: BTreeMap<String, (usize, usize, String)> = BTreeMap::new();
        let (mut simple, mut distance_only, mut other, mut total) = (0, 0, 0, 0);
        for name in pak.list("sounds/").filter(|n| n.ends_with(".fev")).map(str::to_string).collect::<Vec<_>>() {
            let f = Fev::parse(&pak.read(&name).unwrap()).unwrap();
            for e in &f.events {
                total += 1;
                match &e.body {
                    Body::Simple(_) => simple += 1,
                    Body::Complex { parameters, .. } => {
                        if parameters.iter().all(|p| p.name == "distance") {
                            distance_only += 1;
                        } else {
                            other += 1;
                        }
                        for p in parameters {
                            if p.velocity != 0.0 || p.seek_speed != 0.0 {
                                println!("velocity {} seek {} flags {:#x} range {}..{} {} {}", p.velocity, p.seek_speed, p.flags, p.min, p.max, p.name, e.path());
                            }
                            let r = m.entry(p.name.clone()).or_insert((0, 0, e.path()));
                            r.0 += 1;
                            if p.flags & 0x8 != 0 {
                                r.1 += 1;
                            }
                        }
                    }
                }
            }
        }
        println!("events {total}: simple {simple}, complex with only distance {distance_only}, complex with other parameters {other}");
        let mut v: Vec<_> = m.into_iter().collect();
        v.sort_by_key(|(_, (n, _, _))| std::cmp::Reverse(*n));
        for (name, (n, loops, ex)) in v.into_iter().take(40) {
            println!("{name:24} {n:5} (flag 8: {loops}) e.g. {ex}");
        }
    } else if has("--features") {
        features();
    } else if has("--fevs") {
        fevs();
    } else if has("--reverbs") {
        reverbs();
    } else if let Some(sub) = value("--users") {
        users(&sub);
    } else if let Some(project) = value("--list") {
        list(&project, value("--filter").as_deref());
    } else if let Some(file) = value("--resolve") {
        resolve(&file);
    } else if let Some(i) = args.iter().position(|a| a == "--wav") {
        let (Some(what), Some(out)) = (args.get(i + 1), args.get(i + 2)) else {
            return eprintln!("--wav <event or bank.fsb:index> <out.wav>");
        };
        let params = params(&args);
        let reverb = value("--reverb").map(|r| match r.split_once(':') {
            Some((n, w)) => (n.to_string(), w.parse().unwrap_or(1.0)),
            None => (r, 1.0),
        });
        wav(what, out, value("--seconds").and_then(|s| s.parse().ok()), value("--distance").and_then(|s| s.parse().ok()), &params, reverb);
    } else if let Some(path) = value("--event") {
        event(&path, has("--sound"), value("--volume").and_then(|s| s.parse().ok()).unwrap_or(0.3), value("--seconds").and_then(|s| s.parse().ok()).unwrap_or(4.0), value("--at"), &params(&args));
    } else if let Some(path) = value("--music") {
        music(&path, has("--sound"), value("--volume").and_then(|s| s.parse().ok()).unwrap_or(0.3), value("--seconds").and_then(|s| s.parse().ok()).unwrap_or(6.0));
    } else {
        eprintln!("usage: sound --survey | --fevs | --list <project> | --resolve <paths.txt> | --wav <event|bank.fsb:index> <out.wav> | --event <path> --sound");
    }
}

/// `--features`: which FEV features the events use (envelope kinds, cones, doppler, spawning, oscillators).
fn features() {
    let pak = aa_pak::Pak::global();
    let mut env: BTreeMap<(u32, String), (usize, String)> = BTreeMap::new();
    let (mut total, mut doppler, mut cone, mut spawn_events, mut osc, mut null) = (0, 0, 0, 0, 0, 0);
    let mut templates = 0;
    let mut doppler_3d = 0;
    let mut playlists = 0;
    let mut params_used: BTreeMap<String, usize> = BTreeMap::new();
    let mut curves: BTreeMap<u32, (usize, String)> = BTreeMap::new();
    for name in pak.list("sounds/").filter(|n| n.ends_with(".fev")).map(str::to_string).collect::<Vec<_>>() {
        let f = Fev::parse(&pak.read(&name).unwrap()).unwrap();
        templates += f.templates.iter().filter(|t| t.spawn_time_max_ms > 0).count();
        for e in &f.events {
            total += 1;
            if e.doppler > 0.0 {
                doppler += 1;
                if e.is_3d() {
                    doppler_3d += 1;
                }
            }
            if e.is_3d() && (e.cone_inside < 360.0 - 0.5 || e.cone_outside < 360.0 - 0.5) {
                cone += 1;
                println!("  cone event {} inside {} outside {} outside volume {}", e.path(), e.cone_inside, e.cone_outside, e.cone_outside_volume);
            }
            if let Body::Complex { layers, .. } = &e.body {
                for l in layers {
                    for en in &l.envelopes {
                        for point in &en.points {
                            let entry = curves.entry(point.curve).or_insert((0, format!("{}:{}", f.name, e.path())));
                            entry.0 += 1;
                        }
                        let r = env.entry((en.flags, en.dsp.clone())).or_insert((0, e.path()));
                        r.0 += 1;
                        if (r.0 <= 2 || en.dsp.starts_with("FMOD")) && std::env::var("AA_ENV").is_ok() {
                            println!("  {:#06x} {:#x} {} dsp {:?}#{} param {} {:?}", en.flags, en.flags2, e.path(), en.dsp, en.dsp_param, en.parameter, en.points);
                        }
                    }
                }
            }
            if let Body::Complex { layers, parameters, .. } = &e.body {
                for l in layers {
                    let ctl = parameters.get(l.control_parameter as usize).map(|p| p.name.as_str()).unwrap_or("?");
                    let on: Vec<&str> = l.envelopes.iter().filter_map(|en| parameters.get(en.parameter as usize)).map(|p| p.name.as_str()).collect();
                    for (kind, name) in std::iter::once(("control", ctl)).chain(on.iter().map(|n| ("envelope", *n))) {
                        if !matches!(name, "distance" | "camera_distance" | "(distance)" | "daylight" | "time_loop") {
                            *params_used.entry(format!("{kind} on {name}")).or_insert(0) += 1;
                        }
                    }
                }
            }
            for i in e.instances() {
                if i.loops() && let Some(d) = f.sound_defs.get(i.sound_def) {
                    let playable = d.waves.iter().filter(|w| matches!(w, Wave::Bank { .. })).count();
                    if d.waves.len() > 1 {
                        playlists += 1;
                        if std::env::var("AA_ENV").is_ok() && playlists % 25 == 1 {
                            println!("  looping playlist {} ({} waves, {} playable, template {:?}) in {} cat {}", d.name, d.waves.len(), playable, f.templates[d.template].play_mode(), e.path(), e.category);
                        }
                    }
                }
            }
            let mut spawns = false;
            for i in e.instances() {
                if let Some(d) = f.sound_defs.get(i.sound_def) {
                    let t = &f.templates[d.template];
                    spawns |= t.spawn_time_max_ms > 0;
                    for w in &d.waves {
                        match w {
                            Wave::Oscillator { .. } => {
                                osc += 1;
                                println!("  oscillator in {}", e.path());
                            }
                            Wave::Null { .. } => null += 1,
                            _ => {}
                        }
                    }
                }
            }
            spawn_events += usize::from(spawns);
            if spawns && std::env::var("AA_ENV").is_ok() {
                println!("  spawning event {} intensity {} (+-{}) one_shot {}", e.path(), e.spawn_intensity, e.spawn_intensity_random, e.one_shot());
                for i in e.instances() {
                    if let Some(d) = f.sound_defs.get(i.sound_def) {
                        let t = &f.templates[d.template];
                        if t.spawn_time_max_ms > 0 {
                            println!("     def {} spawn {}..{} max {} loop_word {} waves {}", d.name, t.spawn_time_min_ms, t.spawn_time_max_ms, t.max_spawned, i.loop_word, d.waves.len());
                        }
                    }
                }
            }
        }
    }
    println!("events {total}: doppler > 0 {doppler} ({doppler_3d} are 3D), cone narrower than 360 (3D) {cone}, spawning templates {templates} in {spawn_events} events, oscillator waves {osc}, silent waves {null}");
    println!("looping instances whose sound definition has several waves: {playlists}");
    for (what, n) in params_used {
        println!("layers with {what}: {n}");
    }
    for ((flags, dsp), (n, ex)) in env {
        println!("envelope flags {flags:#06x} dsp {dsp:?}: {n:6} e.g. {ex}");
    }
    for (shape, (n, ex)) in curves {
        println!("envelope point shape {shape:#x}: {n:6} e.g. {ex}");
    }
}

/// `--param name=value` (repeatable).
fn params(args: &[String]) -> Vec<(String, f32)> {
    args.iter()
        .enumerate()
        .filter(|(_, a)| *a == "--param")
        .filter_map(|(i, _)| args.get(i + 1)?.split_once('='))
        .filter_map(|(n, v)| Some((n.to_string(), v.parse().ok()?)))
        .collect()
}

// ---------------------------------------------------------------------------------------- survey

#[derive(Default)]
struct Stats {
    banks: usize,
    subsounds: usize,
    ok: usize,
    audio_seconds: f64,
    decode_seconds: f64,
    silent: usize,
    ratio_sum: f64,
    ratio_min: f64,
    ratio_max: f64,
    off_ratio: usize,
    loud_rms_sum: f64,
    loud: usize,
    bytes: usize,
    failures: BTreeMap<String, (usize, String)>,
}

fn survey(prefix: &str, threads: usize) {
    let pak = aa_pak::Pak::global();
    let names: Vec<String> = pak.list(prefix).filter(|n| n.ends_with(".fsb")).map(str::to_string).collect();
    let next = AtomicUsize::new(0);
    let stats: Mutex<BTreeMap<String, Stats>> = Mutex::new(BTreeMap::new());
    let started = Instant::now();
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(name) = names.get(i) else { break };
                    let bytes = pak.read(name).unwrap();
                    let fsb = match Fsb5::parse(&bytes) {
                        Ok(f) => f,
                        Err(e) => {
                            let mut all = stats.lock().unwrap();
                            let s = all.entry("unparsed".into()).or_default();
                            s.failures.entry(e.to_string()).or_insert((0, name.clone())).0 += 1;
                            continue;
                        }
                    };
                    let mut local = Stats { ratio_min: f64::MAX, banks: 1, bytes: bytes.len(), ..Default::default() };
                    for (idx, sub) in fsb.subsounds.iter().enumerate() {
                        local.subsounds += 1;
                        let t = Instant::now();
                        let result = decode(&fsb, &bytes, idx);
                        let took = t.elapsed().as_secs_f64();
                        match result {
                            Ok(pcm) => {
                                local.ok += 1;
                                local.decode_seconds += took;
                                local.audio_seconds += pcm.seconds() as f64;
                                let ratio = pcm.frames() as f64 / sub.samples.max(1) as f64;
                                local.ratio_sum += ratio;
                                local.ratio_min = local.ratio_min.min(ratio);
                                local.ratio_max = local.ratio_max.max(ratio);
                                if !(0.99..=1.01).contains(&ratio) {
                                    local.off_ratio += 1;
                                }
                                let rms = pcm.rms();
                                if rms < 1e-4 {
                                    local.silent += 1;
                                    if std::env::var_os("AA_SHOW_SILENT").is_some() {
                                        println!("silent: {name}:{idx} {:?} {:.2} s peak {}", sub.name, pcm.seconds(), pcm.samples.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0));
                                    }
                                } else {
                                    local.loud += 1;
                                    local.loud_rms_sum += rms as f64;
                                }
                            }
                            Err(e) => {
                                local.failures.entry(e.to_string()).or_insert((0, format!("{name}:{idx}"))).0 += 1;
                            }
                        }
                    }
                    let mut all = stats.lock().unwrap();
                    let key = format!("{:?}", fsb.codec);
                    let s = all.entry(key).or_insert_with(|| Stats { ratio_min: f64::MAX, ..Default::default() });
                    s.banks += local.banks;
                    s.bytes += local.bytes;
                    s.subsounds += local.subsounds;
                    s.ok += local.ok;
                    s.audio_seconds += local.audio_seconds;
                    s.decode_seconds += local.decode_seconds;
                    s.silent += local.silent;
                    s.ratio_sum += local.ratio_sum;
                    s.ratio_min = s.ratio_min.min(local.ratio_min);
                    s.ratio_max = s.ratio_max.max(local.ratio_max);
                    s.off_ratio += local.off_ratio;
                    s.loud += local.loud;
                    s.loud_rms_sum += local.loud_rms_sum;
                    for (k, (n, ex)) in local.failures {
                        let e = s.failures.entry(k).or_insert((0, ex));
                        e.0 += n;
                    }
                }
            });
        }
    });
    let wall = started.elapsed().as_secs_f64();
    println!("survey of {} banks under {prefix} in {wall:.1} s wall ({threads} threads)", names.len());
    let mut total = (0, 0, 0);
    let stats = stats.into_inner().unwrap();
    for (codec, s) in &stats {
        total.0 += s.subsounds;
        total.1 += s.ok;
        println!(
            "{codec:8} banks {:5} ({:.1} MB) subsounds {:5} decoded {:5} ({:.2} %), audio {:.2} h, ratio decoded/header mean {:.4} min {:.4} max {:.4} (outside 1% : {}), \
             silent (rms < 1e-4) {}, mean rms of the others {:.4}, decode {:.1} s = {:.0} x realtime, {:.2} ms per second of audio",
            s.banks,
            s.bytes as f64 / 1e6,
            s.subsounds,
            s.ok,
            100.0 * s.ok as f64 / s.subsounds.max(1) as f64,
            s.audio_seconds / 3600.0,
            s.ratio_sum / s.ok.max(1) as f64,
            if s.ratio_min == f64::MAX { 0.0 } else { s.ratio_min },
            s.ratio_max,
            s.off_ratio,
            s.silent,
            s.loud_rms_sum / s.loud.max(1) as f64,
            s.decode_seconds,
            s.audio_seconds / s.decode_seconds.max(1e-9),
            1000.0 * s.decode_seconds / s.audio_seconds.max(1e-9),
        );
        for (msg, (n, example)) in &s.failures {
            println!("    {n} x {msg}  (e.g. {example})");
        }
        total.2 += s.failures.values().map(|f| f.0).sum::<usize>();
    }
    println!("total subsounds {} decoded {} ({:.2} %), failures {}", total.0, total.1, 100.0 * total.1 as f64 / total.0.max(1) as f64, total.2);
}

// ------------------------------------------------------------------------------------------ FEVs

fn fevs() {
    let pak = aa_pak::Pak::global();
    let mut names: Vec<&str> = pak.list("sounds/").filter(|n| n.ends_with(".fev")).collect();
    names.sort();
    let (mut ok, mut events, mut simple, mut defs, mut waves) = (0, 0, 0, 0, 0);
    for name in names {
        let bytes = pak.read(name).unwrap();
        match Fev::parse(&bytes) {
            Ok(f) => {
                ok += 1;
                events += f.events.len();
                simple += f.events.iter().filter(|e| e.is_simple()).count();
                defs += f.sound_defs.len();
                waves += f.sound_defs.iter().map(|d| d.waves.len()).sum::<usize>();
                println!(
                    "{name}: {} events ({} simple), {} groups, {} banks, {} templates, {} sound defs, {} reverbs",
                    f.events.len(),
                    f.events.iter().filter(|e| e.is_simple()).count(),
                    f.groups.len(),
                    f.banks.len(),
                    f.templates.len(),
                    f.sound_defs.len(),
                    f.reverbs.len(),
                );
            }
            Err(e) => println!("{name}: ERROR {e}"),
        }
    }
    println!("{ok} FEV files parsed to their last byte: {events} events ({simple} simple), {defs} sound definitions, {waves} waves");
}

/// `--users <bank>:<index>`: the events whose sound definitions play that subsound (e.g. `x2weapons_sword_bank055:0`).
fn users(sub: &str) {
    let (bank, index) = sub.rsplit_once(':').expect("bank:index");
    let bank = bank.trim_end_matches(".fsb").to_ascii_lowercase();
    let index: u32 = index.parse().expect("index");
    let pak = aa_pak::Pak::global();
    let mut names: Vec<&str> = pak.list("sounds/").filter(|n| n.ends_with(".fev")).collect();
    names.sort();
    for name in names {
        let f = Fev::parse(&pak.read(name).unwrap()).unwrap();
        let defs: HashSet<usize> = f
            .sound_defs
            .iter()
            .enumerate()
            .filter(|(_, d)| d.waves.iter().any(|w| matches!(w, Wave::Bank { bank: b, index: i, .. } if *i == index && f.banks[*b as usize].name.to_ascii_lowercase() == bank)))
            .map(|(i, _)| i)
            .collect();
        for e in &f.events {
            if e.instances().iter().any(|i| defs.contains(&i.sound_def)) {
                println!("{name}: {} (category {}, {})", e.path(), e.category, if e.is_3d() { "3D" } else { "2D" });
            }
        }
    }
}

/// `--reverbs`: the reverb definitions of the projects and how the events send to the reverb.
fn reverbs() {
    let pak = aa_pak::Pak::global();
    let mut names: Vec<&str> = pak.list("sounds/").filter(|n| n.ends_with(".fev")).collect();
    names.sort();
    let mut hist: BTreeMap<(String, String), (usize, String)> = BTreeMap::new();
    let mut env: BTreeMap<u32, (usize, usize, String)> = BTreeMap::new();
    let mut dsps: BTreeMap<(u32, String), (usize, String)> = BTreeMap::new();
    for name in names {
        let f = Fev::parse(&pak.read(name).unwrap()).unwrap();
        for (i, r) in f.reverbs.iter().enumerate() {
            println!("{name}: reverb {i} {r:?}");
        }
        for e in &f.events {
            let cat = e.category.rsplit('/').next().unwrap_or("").to_string();
            let key = (cat, format!("dry {:.3} wet {:.3}", e.reverb_dry, e.reverb_wet));
            let r = hist.entry(key).or_insert((0, e.path()));
            r.0 += 1;
            if let Body::Complex { layers, parameters, .. } = &e.body {
                let mut seen = HashSet::new();
                for l in layers {
                    for en in &l.envelopes {
                        let bits = en.flags & !0x8;
                        if bits != 0 && seen.insert(en.flags) {
                            let on_dist = parameters.get(en.parameter as usize).is_some_and(|p| p.name.contains("distance"));
                            let r = env.entry(en.flags).or_insert((0, 0, e.path()));
                            r.0 += 1;
                            r.1 += usize::from(on_dist);
                            let d = dsps.entry((en.flags, en.dsp.clone())).or_insert((0, String::new()));
                            d.0 += 1;
                            if d.1.is_empty() {
                                d.1 = format!("{} points {:?} param {}", e.path(), en.points.iter().map(|p| (p.x, p.y)).collect::<Vec<_>>(), parameters.get(en.parameter as usize).map_or("?", |p| p.name.as_str()));
                            }
                        }
                    }
                }
            }
        }
    }
    for ((flags, dsp), (n, ex)) in dsps {
        println!("envelope {flags:#x} dsp {dsp:?}: {n} e.g. {ex}");
    }
    for (flags, (n, on_distance, ex)) in env {
        println!("envelope flags {flags:#x}: {n} events ({on_distance} on a distance parameter) e.g. {ex}");
    }
    for ((cat, v), (n, ex)) in hist {
        println!("{cat:22} {v:26} {n:6} e.g. {ex}");
    }
}

/// `--list <project> [--filter text]`: dump the events.
fn list(project: &str, filter: Option<&str>) {
    let pak = aa_pak::Pak::global();
    let path = if project.ends_with(".fev") { project.to_string() } else { format!("sounds/{project}/{project}.fev") };
    let f = Fev::parse(&pak.read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))).unwrap();
    println!("project {} : {} groups, {} events, {} banks, {} sound defs", f.name, f.groups.len(), f.events.len(), f.banks.len(), f.sound_defs.len());
    println!("categories: {}", f.categories.paths().join(" "));
    let filter = filter.map(str::to_ascii_lowercase);
    for e in &f.events {
        let p = e.path();
        if filter.as_ref().is_some_and(|t| !p.to_ascii_lowercase().contains(t)) {
            continue;
        }
        println!(
            "{p}  [{}] {} vol {:.2} pitch {:+.2}oct (+-{:.2}) volrand {:.2} prio {} max {} dist {}..{} {:?} oneshot {} fade {}/{} ms cat {} flags {:#x} mode {:#x}",
            if e.is_simple() { "simple" } else { "complex" },
            if e.is_3d() { "3D" } else { "2D" },
            e.volume,
            e.pitch * 4.0,
            e.pitch_random * 4.0,
            e.volume_random,
            e.priority,
            e.max_playbacks,
            e.min_distance,
            e.max_distance,
            e.rolloff(),
            e.one_shot(),
            e.fade_in_ms,
            e.fade_out_ms,
            e.category,
            e.flags,
            e.mode,
        );
        let show = |i: &aa_audio::fev::Instance, indent: &str| {
            let sd = &f.sound_defs[i.sound_def];
            let t = &f.templates[sd.template];
            println!(
                "{indent}instance {}..+{} loop word {} -> sound def {:?} {:?} vol {} pitch {:+.2}oct (+-{:.2}) volmin {} spawn {}..{} ms max {}",
                i.start,
                i.length,
                i.loop_word,
                sd.name,
                t.play_mode(),
                t.volume,
                t.pitch * 4.0,
                t.pitch_random * 4.0,
                t.volume_random_min,
                t.spawn_time_min_ms,
                t.spawn_time_max_ms,
                t.max_spawned
            );
            for w in &sd.waves {
                match w {
                    Wave::Bank { weight, name, bank, index, length_ms } => {
                        println!("{indent}    wave w{weight} {name} = {}:{index} ({length_ms} ms)", f.banks[*bank as usize].name)
                    }
                    other => println!("{indent}    {other:?}"),
                }
            }
        };
        match &e.body {
            Body::Simple(i) => show(i, "    "),
            Body::Complex { layers, parameters, .. } => {
                for p in parameters {
                    println!("    parameter {} {}..{} flags {:#x}", p.name, p.min, p.max, p.flags);
                }
                for (li, l) in layers.iter().enumerate() {
                    let envs: Vec<String> = l.envelopes.iter().map(|e| format!("{:#x}@p{} {} points", e.flags, e.parameter, e.points.len())).collect();
                    println!("    layer {li} control p{} envelopes [{}]", l.control_parameter, envs.join(", "));
                    if std::env::var_os("AA_ENV").is_some() {
                        for envelope in &l.envelopes {
                            println!("        curve {:#x} dsp {:?}#{} parameter {} {:?}", envelope.flags, envelope.dsp, envelope.dsp_param, envelope.parameter, envelope.points);
                        }
                    }
                    for i in &l.instances {
                        show(i, "        ");
                    }
                }
            }
        }
    }
}

// ------------------------------------------------------------------------------------- resolving

/// Resolves every path of a text file (one per line) and checks the banks the events point at.
fn resolve(file: &str) {
    let lib = Library::global();
    let text = std::fs::read_to_string(file).unwrap_or_else(|e| panic!("{file}: {e}"));
    let mut paths: Vec<String> = text.lines().map(|l| l.trim().to_ascii_lowercase()).filter(|l| !l.is_empty()).collect();
    paths.sort();
    paths.dedup();
    let started = Instant::now();
    let mut outcome: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    let mut absent_projects: BTreeMap<String, usize> = BTreeMap::new();
    let (mut waves, mut bank_missing, mut index_bad, mut name_ok, mut name_other) = (0, 0, 0, 0, 0);
    let mut bad_examples = Vec::new();
    let mut resolved_events = HashSet::new();
    for p in &paths {
        let Some(parsed) = aa_audio::library::parse_path(p) else {
            outcome.entry("malformed path").or_default().push(p.clone());
            continue;
        };
        if lib.project(&parsed.project).is_none() {
            *absent_projects.entry(parsed.project.clone()).or_default() += 1;
            outcome.entry("project FEV not in the pak").or_default().push(p.clone());
            continue;
        }
        let Some(event) = lib.event(p) else {
            outcome.entry("event not in the project").or_default().push(p.clone());
            continue;
        };
        outcome.entry("resolved").or_default().push(p.clone());
        if !resolved_events.insert(event.key()) {
            continue;
        }
        for inst in event.event().instances() {
            for w in &event.project.fev.sound_defs[inst.sound_def].waves {
                let Wave::Bank { name, bank, index, .. } = w else { continue };
                waves += 1;
                match lib.bank(&event.project, *bank) {
                    Err(e) => {
                        bank_missing += 1;
                        if bad_examples.len() < 5 {
                            bad_examples.push(format!("{p}: {e}"));
                        }
                    }
                    Ok(b) => match b.fsb.subsounds.get(*index as usize) {
                        None => index_bad += 1,
                        Some(sub) => {
                            let stem = name.rsplit('/').next().unwrap_or(name).trim_end_matches(".wav").to_ascii_lowercase();
                            match sub.name.as_deref().map(str::to_ascii_lowercase) {
                                Some(n) if n.contains(&stem) || stem.contains(n.trim_end_matches(".wav")) => name_ok += 1,
                                _ => name_other += 1,
                            }
                        }
                    },
                }
            }
        }
    }
    println!("{} distinct paths (case-insensitive) in {:.1} s", paths.len(), started.elapsed().as_secs_f64());
    let show = if std::env::var_os("AA_VERBOSE").is_some() { usize::MAX } else { 3 };
    for (k, v) in &outcome {
        if *k == "resolved" && show == usize::MAX {
            println!("{k:30} {:5}", v.len());
            continue;
        }
        println!("{k:30} {:5}   e.g. {}", v.len(), v.iter().take(show).cloned().collect::<Vec<_>>().join("
                                      "));
    }
    println!("absent projects: {absent_projects:?}");
    println!("{} distinct events resolved; waves checked {waves}: bank missing {bank_missing}, subsound index out of range {index_bad}, subsound name matches the FEV wave name {name_ok}, differs {name_other}", resolved_events.len());
    for e in bad_examples {
        println!("    {e}");
    }
}

// ------------------------------------------------------------------------------------------ WAVs

fn mix(parts: &[(Pcm, f32, f32, bool)], seconds: f32) -> Pcm {
    const RATE: u32 = 44100;
    let frames = (seconds * RATE as f32) as usize;
    let mut out = vec![0f32; frames * 2];
    for (pcm, gain, rate, looping) in parts {
        let ch = pcm.channels.max(1) as usize;
        let n = pcm.frames();
        if n == 0 {
            continue;
        }
        let step = pcm.rate as f64 * *rate as f64 / RATE as f64;
        let mut pos = 0.0f64;
        for o in 0..frames {
            let mut i = pos as usize;
            if i >= n {
                if !*looping {
                    break;
                }
                pos -= n as f64;
                i = pos as usize;
            }
            let frac = (pos - i as f64) as f32;
            let j = if i + 1 < n { i + 1 } else if *looping { 0 } else { i };
            let at = |f: usize, c: usize| pcm.samples[f * ch + c.min(ch - 1)] as f32 / 32768.0;
            for c in 0..2 {
                out[o * 2 + c] += gain * (at(i, c) * (1.0 - frac) + at(j, c) * frac);
            }
            pos += step;
        }
    }
    let peak = out.iter().fold(0f32, |m, v| m.max(v.abs()));
    let scale = if peak > 1.0 { 1.0 / peak } else { 1.0 };
    Pcm { rate: RATE, channels: 2, samples: out.iter().map(|v| (v * scale * 32767.0) as i16).collect(), loop_points: None }
}

/// The sound through the area reverb `name` at `weight`, sent at the event's wet level (what the engine does live), plus
/// the tail.
fn with_reverb(dry: Pcm, name: &str, weight: f32, wet_db: f32) -> Pcm {
    use aa_audio::reverb::{Params, Presets, ReverbCore};
    let presets = Presets::load(aa_pak::Pak::global());
    let preset = presets.get(name).unwrap_or_else(|| panic!("no reverb preset {name}"));
    let params = Params::of(preset, weight);
    println!("reverb {name} x{weight}: decay {:.2} s, hf ratio {:.2}, late {:.3}, early {:.3}, send {wet_db} dB", params.decay, params.hf_ratio, params.late, params.early);
    let mut core = ReverbCore::new(dry.rate);
    core.set_target(params);
    core.snap();
    let send = 10f32.powf(wet_db / 20.0);
    let tail = (params.decay.min(6.0) * dry.rate as f32) as usize;
    let frames = dry.frames() + tail;
    let mut out = Vec::with_capacity(frames * 2);
    for i in 0..frames {
        let (l, r) = if i < dry.frames() { (dry.samples[i * 2] as f32 / 32768.0, dry.samples[i * 2 + 1] as f32 / 32768.0) } else { (0.0, 0.0) };
        let w = core.tick(kira::Frame::new(l * send, r * send));
        out.push(l + w.left);
        out.push(r + w.right);
    }
    let peak = out.iter().fold(0f32, |m, v| m.max(v.abs()));
    let scale = if peak > 1.0 { 1.0 / peak } else { 1.0 };
    Pcm { rate: dry.rate, channels: 2, samples: out.iter().map(|v| (v * scale * 32767.0) as i16).collect(), loop_points: None }
}

fn wav(what: &str, out: &str, seconds: Option<f32>, distance: Option<f32>, params: &[(String, f32)], reverb: Option<(String, f32)>) {
    let lib = Library::global();
    if let Some(parent) = std::path::Path::new(out).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut wet_db = -8.0;
    let pcm = if let Some((bank, index)) = what.rsplit_once(':').filter(|(b, _)| b.ends_with(".fsb")) {
        let bytes = aa_pak::Pak::global().read(bank).unwrap_or_else(|e| panic!("{bank}: {e}"));
        let fsb = Fsb5::parse(&bytes).unwrap();
        let mut pcm = decode(&fsb, &bytes, index.parse().expect("subsound index")).unwrap();
        if let Some(s) = seconds {
            pcm.samples.truncate((s * pcm.rate as f32) as usize * pcm.channels as usize);
        }
        pcm
    } else {
        let event = lib.event(what).unwrap_or_else(|| panic!("no event {what}"));
        let ev = event.event();
        wet_db = ev.reverb_wet;
        let distance = ev.is_3d().then_some(distance.unwrap_or(1.0));
        let values = initial_values(ev, distance, params);
        let planned = plan(&event, &values, &mut PickState::default(), &mut Rng::new(1));
        let mut parts = Vec::new();
        let mut length = 0f32;
        for p in &planned {
            let bank = lib.bank(&event.project, p.bank).unwrap_or_else(|e| panic!("{e}"));
            let pcm = lib.decode(&bank, p.index as usize).unwrap_or_else(|e| panic!("{e}"));
            let gain = p.gain * layer_gain(ev, &values, p.layer) * distance.map_or(1.0, |d| distance_gain(ev, d));
            println!(
                "  layer {} {} = {}:{} gain {:.2} rate {:.3} {:.2} s{}",
                p.layer,
                p.name,
                event.project.fev.banks[p.bank as usize].name,
                p.index,
                gain,
                p.rate,
                pcm.seconds(),
                if p.looping { " loop" } else { "" }
            );
            length = length.max(pcm.seconds() / p.rate);
            parts.push((pcm, gain, p.rate, p.looping));
        }
        let looping = parts.iter().any(|p| p.3);
        let seconds = seconds.unwrap_or(if looping { (length * 2.0).min(30.0) } else { length });
        println!("{what}: {} sounds, {seconds:.2} s{}", parts.len(), if looping { " (looped twice)" } else { "" });
        mix(&parts, seconds)
    };
    let pcm = match &reverb {
        Some((name, weight)) => with_reverb(pcm, name, *weight, wet_db),
        None => pcm,
    };
    std::fs::write(out, pcm.to_wav()).unwrap();
    println!("wrote {out}: {:.2} s, {} Hz, {} ch, rms {:.4}", pcm.seconds(), pcm.rate, pcm.channels, pcm.rms());
}

// ------------------------------------------------------------------------------------- playing

fn event(path: &str, audible: bool, volume: f32, seconds: f32, at: Option<String>, params: &[(String, f32)]) {
    let lib = Library::global();
    let Some(ev) = lib.event(path) else {
        return eprintln!("no event {path}");
    };
    let e = ev.event();
    println!("{} : {} {}, volume {:.2}, {} instance(s), category {}", ev.name(), if e.is_3d() { "3D" } else { "2D" }, if e.one_shot() { "one shot" } else { "may loop" }, e.volume, e.instances().len(), e.category);
    let engine = Engine::start(Settings { audible, ..Settings::default() });
    engine.set_volumes(Volumes { master: volume, ..Volumes::default() });
    let pos = at.map(|s| {
        let v: Vec<f32> = s.split(',').filter_map(|c| c.trim().parse().ok()).collect();
        Vec3::new(*v.first().unwrap_or(&0.0), *v.get(1).unwrap_or(&0.0), *v.get(2).unwrap_or(&0.0))
    });
    if audible {
        println!("playing for at most {seconds} s at master volume {volume}");
    } else {
        println!("muted (pass --sound to hear it)");
    }
    let voice = engine.play_with(path, pos, aa_audio::PlayOptions { params: params.to_vec(), ..Default::default() });
    if engine.wait_idle(Duration::from_secs_f32(seconds)) {
        println!("finished");
    } else {
        engine.stop(voice, true);
        engine.wait_idle(Duration::from_secs(2));
        println!("stopped after {seconds} s");
    }
    println!("{:#?}", engine.stats());
}

fn music(path: &str, audible: bool, volume: f32, seconds: f32) {
    let engine = Engine::start(Settings { audible, ..Settings::default() });
    engine.set_volumes(Volumes { master: volume, ..Volumes::default() });
    if audible {
        println!("streaming {path} for {seconds} s at master volume {volume}");
    } else {
        println!("muted (pass --sound to hear it)");
    }
    engine.play_music(path, Duration::from_millis(1500), Duration::from_millis(1500));
    std::thread::sleep(Duration::from_secs_f32(seconds));
    engine.stop_music(Duration::from_millis(500));
    std::thread::sleep(Duration::from_millis(700));
    println!("{:#?}", engine.stats());
}
