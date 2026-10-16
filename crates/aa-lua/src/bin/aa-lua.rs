//! `aa-lua` command line: inspect and convert the client's UI scripts.
//!
//! ```text
//! aa-lua dis <pak path>                 disassemble one .alb (stock 5.1 opcodes)
//! aa-lua toc <toc path>                 list the scripts a toc.g loads, in order
//! aa-lua convert <pak prefix> <outdir>  write host-format chunks (.luac) for a decompiler
//! aa-lua run <toc path>...              run tocs in one state (no native API: expect errors)
//! aa-lua stage <name>                   list the scripts of a screen (login, world_select, ..., world)
//! aa-lua record [json] [md]             API recorder -> data/ui-api.json + docs/ui-api.md
//! ```

use std::path::Path;

use anyhow::{Context, Result, bail};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let pak = aa_pak::Pak::global();
    match args.first().map(String::as_str) {
        Some("dis") if args.len() == 2 => {
            let path = aa_lua::alb_path(&args[1]);
            let chunk = aa_lua::chunk::parse(&pak.read(&path)?)?;
            println!("; {path}: {:?}", chunk.format);
            print!("{}", aa_lua::chunk::disassemble(&chunk.main));
        }
        Some("toc") if args.len() == 2 => {
            for f in aa_lua::toc_files(&args[1])? {
                let mark = if pak.exists(&f) { "" } else { "   (missing)" };
                println!("{f}{mark}");
            }
        }
        Some("convert") if args.len() == 3 => {
            let out = Path::new(&args[2]);
            let mut n = 0;
            for name in pak.list(&args[1]).filter(|n| n.to_ascii_lowercase().ends_with(".alb")) {
                let rel = aa_pak::normalize(name);
                let rel = rel.strip_prefix("game/").unwrap_or(&rel).to_string();
                let host = aa_lua::convert_alb(&pak.read(name)?, &rel)?;
                let dst = out.join(format!("{}.luac", rel.trim_end_matches(".alb")));
                std::fs::create_dir_all(dst.parent().unwrap())?;
                std::fs::write(&dst, host).with_context(|| format!("writing {}", dst.display()))?;
                n += 1;
            }
            eprintln!("converted {n} chunks into {}", out.display());
        }
        Some("run") if args.len() >= 2 => {
            let lua = aa_lua::new_lua().map_err(|e| anyhow::anyhow!("{e}"))?;
            for toc in &args[1..] {
                if let Err(e) = aa_lua::run_toc(&lua, toc) {
                    eprintln!("{e:#}");
                }
            }
        }
        Some("stage") if args.len() == 2 => {
            let stage = aa_lua::Stage::ALL.into_iter().find(|s| s.name() == args[1]).context("unknown stage")?;
            for s in stage.scripts()? {
                println!("{s}");
            }
        }
        Some("record") => {
            let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
            let json = args.get(1).map(Into::into).unwrap_or_else(|| root.join("data/ui-api.json"));
            let md = args.get(2).map(Into::into).unwrap_or_else(|| root.join("docs/ui-api.md"));
            aa_lua::recorder::record_all(&json, &md)?;
            eprintln!("wrote {} and {}", json.display(), md.display());
        }
        _ => bail!("usage: aa-lua dis <path> | toc <toc> | convert <prefix> <outdir> | run <toc>... | stage <name> | record [json] [md]"),
    }
    Ok(())
}
