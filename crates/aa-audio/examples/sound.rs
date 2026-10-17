//! Sound tool: survey the banks, list events, write WAVs, play events.
//!
//! ```text
//! cargo run -p aa-audio --release --example sound -- --survey [--prefix sounds/x2skills] [--threads 4]
//! ```

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use aa_audio::{Fsb5, decode};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
    if args.iter().any(|a| a == "--survey") {
        survey(&value("--prefix").unwrap_or_else(|| "sounds/".into()), value("--threads").and_then(|t| t.parse().ok()).unwrap_or(4));
        return;
    }
    eprintln!("usage: sound --survey | --list <fev> | --wav <event|bank:index> <out.wav> | --event <path> [--sound]");
}

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
