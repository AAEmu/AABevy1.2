//! Decodes a pak DDS to an uncompressed TGA for inspection:
//! `cargo run -p aa-cry --example dds_dump -- <pak path> <out.tga> [mip] [face]`
//! Prints the format, size, mips and CryEngine extension data.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: dds_dump <pak path> <out.tga> [mip] [face]");
        return;
    }
    let mip: u32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0);
    let face: u32 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(0);
    let bytes = aa_cry::read(&args[1]).expect("read");
    let dds = aa_cry::texture::Dds::parse(&bytes).expect("parse");
    println!(
        "{}: {:?} {}x{}x{} mips={} faces={} attached={:?} avg={:?}",
        args[1],
        dds.format,
        dds.width,
        dds.height,
        dds.depth,
        dds.mips,
        dds.faces,
        dds.attached.as_ref().map(|a| (a.format, a.width, a.height)),
        dds.average_color
    );
    let (w, h) = ((dds.width >> mip).max(1), (dds.height >> mip).max(1));
    write_tga(&args[2], w, h, &dds.decode_rgba8(face, mip));
    if let Some(a) = &dds.attached {
        let p = args[2].replace(".tga", "_attached.tga");
        write_tga(&p, a.width, a.height, &a.decode_rgba8(0, 0));
    }
}

fn write_tga(path: &str, w: u32, h: u32, rgba: &[u8]) {
    let mut out = vec![0u8, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    out.extend_from_slice(&(w as u16).to_le_bytes());
    out.extend_from_slice(&(h as u16).to_le_bytes());
    out.extend_from_slice(&[32, 0x28]); // 32 bpp, top-left origin, 8 alpha bits
    for p in rgba.chunks_exact(4) {
        out.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
    }
    if let Some(dir) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::write(path, out).expect("write");
}
