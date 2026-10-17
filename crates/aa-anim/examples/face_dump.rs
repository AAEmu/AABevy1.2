//! Face texture diagnostics: `cargo run -p aa-anim --example face_dump -- <pak path> ...` prints
//! size and format of DDS files (and whether they exist).

fn main() {
    let pak = aa_pak::Pak::global();
    for p in std::env::args().skip(1) {
        let key = aa_pak::normalize(&p);
        if let Some(prefix) = key.strip_suffix('*') {
            for n in pak.names().map(aa_pak::normalize).filter(|n| n.starts_with(prefix)) {
                println!("{n}");
            }
            continue;
        }
        match aa_cry::read(&key).ok().and_then(|b| aa_cry::texture::Dds::parse(&b).ok().map(|d| (d.width, d.height))) {
            Some((w, h)) => println!("{key}: {w}x{h}"),
            None => println!("{key}: missing / unreadable"),
        }
    }
}
