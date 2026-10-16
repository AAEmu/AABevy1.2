//! `aa-pak ls <prefix>` | `aa-pak cat <path>` | `aa-pak x <prefix> <outdir>`
use std::io::Write;

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let pak = aa_pak::Pak::global();
    match args.as_slice() {
        ["ls", prefix] => {
            let mut out = std::io::stdout().lock();
            for n in pak.list(prefix) {
                writeln!(out, "{n}\t{}", pak.entry(n).unwrap().size)?;
            }
        }
        ["cat", path] => std::io::stdout().write_all(&pak.read(path)?)?,
        ["x", prefix, outdir] => {
            let mut count = 0;
            for n in pak.list(prefix) {
                let dst = std::path::Path::new(outdir).join(n);
                std::fs::create_dir_all(dst.parent().unwrap())?;
                std::fs::write(dst, pak.read(n)?)?;
                count += 1;
            }
            eprintln!("extracted {count} files to {outdir}");
        }
        _ => eprintln!(
            "usage: aa-pak ls <prefix> | cat <path> | x <prefix> <outdir>   (AA_PAK overrides the pak)"
        ),
    }
    Ok(())
}
