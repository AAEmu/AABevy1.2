//! Read-only access to the ArcheAge 1.2 `game_pak` archive.
//!
//! Layout (little endian): `[file data][file table][pad to 0x200][header 0x200]`. The header and
//! every 0x150-byte file table entry are AES-128-CBC encrypted on their own with a zero IV and the
//! XLGames key. File data is stored raw (no compression, no encryption).
//!
//! [`Pak::open`] also opens the compressed pak written by `aa-bake` (see [`baked`]); both behave
//! the same through this API. In the browser (wasm32) the pak is a v2 baked pak fetched over HTTP:
//! see [`web`], whose `open` must run before [`Pak::global`].
//!
//! Paths are matched case-insensitively with `/` or `\` separators. CryEngine paths are relative
//! to the `game` folder (`objects/characters/...`), pak paths are not (`game/objects/...`): both
//! forms resolve.
//!
//! ```no_run
//! let pak = aa_pak::Pak::global();
//! let toc = pak.read("scriptsbin/x2ui/loginstage/character_create/toc.g").unwrap();
//! ```

use std::collections::HashMap;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use aes::Aes128;
use aes::cipher::generic_array::GenericArray;
use aes::cipher::{BlockDecrypt, KeyInit};

pub mod baked;
pub mod mesh;
#[cfg(target_arch = "wasm32")]
pub mod web;
#[cfg(not(target_arch = "wasm32"))]
pub mod write;

/// The 1.2 client's `game_pak` when the `AA_PAK` environment variable is not set (relative to the working
/// directory).
pub const DEFAULT_PAK: &str = "game_pak";

const KEY: [u8; 16] = [
    0x32, 0x1F, 0x2A, 0xEE, 0xAA, 0x58, 0x4A, 0xB4, 0x9A, 0x6C, 0x9E, 0x09, 0xD5, 0x9E, 0x9C, 0x6F,
];
const HEADER_SIZE: u64 = 0x200;
const ENTRY_SIZE: usize = 0x150;
const NAME_SIZE: usize = 0x108;

/// A folder of loose files that replace (or add to) pak files, mirroring pak paths
/// (`<dir>/worlds/main_world/cells/015_015/client/terrain/heightmap.dat`): the world editor saves
/// its edits there. Set with the `AA_PAK_OVERLAY` environment variable; unset means no overlay.
fn overlay_dir() -> Option<&'static Path> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| std::env::var_os("AA_PAK_OVERLAY").map(PathBuf::from).filter(|p| p.is_dir())).as_deref()
}

/// The overlay file standing in for `path`, if there is one.
fn overlay_file(path: &str) -> Option<PathBuf> {
    let dir = overlay_dir()?;
    let n = normalize(path);
    let n = n.strip_prefix("game/").unwrap_or(&n);
    let file = dir.join(n);
    file.is_file().then_some(file)
}

/// Location of one file inside the pak.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub offset: u64,
    /// Size of the file itself.
    pub size: u64,
    /// Bytes it takes in the pak (`size` unless compressed).
    pub packed: u64,
    /// [`baked::STORED`] for the original pak.
    pub codec: u8,
    /// v2 baked pak: the solid block holding the file (`offset`/`packed` are then its place
    /// inside the decompressed block, `codec` the transform left to undo).
    pub block: Option<u32>,
}

/// An open `game_pak`. Cheap to share: reads use positioned I/O and take `&self`.
pub struct Pak {
    path: PathBuf,
    /// The pak file (native).
    file: Option<File>,
    /// Blocks over HTTP (browser).
    #[cfg(target_arch = "wasm32")]
    remote: Option<web::Remote>,
    /// Normalised (lowercase, `/`) name -> entry.
    entries: HashMap<String, Entry>,
    /// Original names in file-table order.
    names: Vec<String>,
    /// v2 baked pak only.
    blocks: Vec<baked::Block>,
    /// Recently decompressed blocks, newest last.
    cache: Mutex<Vec<(u32, Arc<Vec<u8>>)>>,
}

/// Decompressed v2 blocks kept around (`AA_PAK_CACHE_MB`, default 256).
fn cache_budget() -> usize {
    static MB: OnceLock<usize> = OnceLock::new();
    // the browser decompresses on its one thread and holds the packed blocks too (`web`): a bigger share for
    // decompressed blocks there, or loading the world decompresses half of its blocks twice
    const DEFAULT_MB: usize = if cfg!(target_arch = "wasm32") { 640 } else { 256 };
    *MB.get_or_init(|| std::env::var("AA_PAK_CACHE_MB").ok().and_then(|v| v.parse().ok()).unwrap_or(DEFAULT_MB)) << 20
}

impl Pak {
    /// The process-wide pak (`AA_PAK` or [`DEFAULT_PAK`]). Panics if it cannot be opened.
    pub fn global() -> &'static Pak {
        #[cfg(target_arch = "wasm32")]
        return web::global();
        #[cfg(not(target_arch = "wasm32"))]
        static PAK: OnceLock<Pak> = OnceLock::new();
        #[cfg(not(target_arch = "wasm32"))]
        PAK.get_or_init(|| {
            let path = std::env::var("AA_PAK").unwrap_or_else(|_| DEFAULT_PAK.to_string());
            Pak::open(&path).unwrap_or_else(|e| panic!("cannot open game_pak {path}: {e}"))
        })
    }

    pub fn open(path: impl AsRef<Path>) -> io::Result<Pak> {
        let path = path.as_ref().to_path_buf();
        let file = File::open(&path)?;
        let len = file.metadata()?.len();
        if len >= baked::FOOTER_SIZE {
            let mut footer = [0u8; baked::FOOTER_SIZE as usize];
            read_exact_at(&file, &mut footer, len - baked::FOOTER_SIZE)?;
            if &footer[..8] == baked::MAGIC || &footer[..8] == baked::MAGIC2 {
                let u64_at = |o: usize| u64::from_le_bytes(footer[o..o + 8].try_into().unwrap());
                let mut index = vec![0u8; u64_at(16) as usize];
                read_exact_at(&file, &mut index, u64_at(8))?;
                let v2 = &footer[..8] == baked::MAGIC2;
                return Self::from_index(path, Some(file), None, &index, u64_at(24) as usize, v2);
            }
        }
        let cipher = Aes128::new(GenericArray::from_slice(&KEY));

        let mut header = vec![0u8; HEADER_SIZE as usize];
        read_exact_at(&file, &mut header, len - HEADER_SIZE)?;
        decrypt_cbc(&cipher, &mut header);
        let u32_at = |b: &[u8], o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        let (kind, files, extra) = if &header[0..4] == b"WIBO" {
            ('A', u32_at(&header, 8), u32_at(&header, 12))
        } else if &header[8..12] == b"IDEJ" {
            ('B', u32_at(&header, 12), u32_at(&header, 0))
        } else if &header[0..4] == b"ZERO" {
            ('F', u32_at(&header, 8), u32_at(&header, 12))
        } else {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "unknown game_pak header"));
        };

        let count = (files + extra) as usize;
        let table_len = (count * ENTRY_SIZE) as u64;
        let mut table_at = len - HEADER_SIZE - table_len;
        table_at -= table_at % HEADER_SIZE;
        let mut table = vec![0u8; count * ENTRY_SIZE];
        read_exact_at(&file, &mut table, table_at)?;

        let mut entries = HashMap::with_capacity(files as usize);
        let mut names = Vec::with_capacity(files as usize);
        for (i, raw) in table.as_chunks_mut::<ENTRY_SIZE>().0.iter_mut().enumerate() {
            let is_file = if kind == 'B' { i >= extra as usize } else { i < files as usize };
            if !is_file {
                continue;
            }
            decrypt_cbc(&cipher, raw);
            let i64_at = |o: usize| i64::from_le_bytes(raw[o..o + 8].try_into().unwrap()) as u64;
            let (name_at, offset, size) = match kind {
                'A' => (0, i64_at(0x108), i64_at(0x110)),
                'B' => (0x20, i64_at(0x18), i64_at(0x130)),
                _ => (8, i64_at(0x110), i64_at(0x118)),
            };
            let name = &raw[name_at..name_at + NAME_SIZE];
            let name = &name[..name.iter().position(|&b| b == 0).unwrap_or(NAME_SIZE)];
            let name: String = name.iter().map(|&b| b as char).collect(); // latin-1
            entries.insert(normalize(&name), Entry { offset, size, packed: size, codec: baked::STORED, block: None });
            names.push(name);
        }
        Ok(Pak {
            path,
            file: Some(file),
            #[cfg(target_arch = "wasm32")]
            remote: None,
            entries,
            names,
            blocks: Vec::new(),
            cache: Mutex::default(),
        })
    }

    /// A baked pak from its packed (zstd) index; `v2` selects the solid-block index format.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn from_index(
        path: PathBuf,
        file: Option<File>,
        remote: Option<web::Remote>,
        index: &[u8],
        raw_len: usize,
        v2: bool,
    ) -> io::Result<Pak> {
        let mut pak = Self::parse_index(path, file, index, raw_len, v2)?;
        pak.remote = remote;
        Ok(pak)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn from_index(path: PathBuf, file: Option<File>, _remote: Option<()>, index: &[u8], raw_len: usize, v2: bool) -> io::Result<Pak> {
        Self::parse_index(path, file, index, raw_len, v2)
    }

    fn parse_index(path: PathBuf, file: Option<File>, index: &[u8], raw_len: usize, v2: bool) -> io::Result<Pak> {
        let index = baked::unzstd(index, raw_len)?;
        let mut entries = HashMap::new();
        let mut names = Vec::new();
        let mut blocks = Vec::new();
        if v2 {
            let (b, records) = baked::decode_index2(&index)?;
            blocks = b;
            for r in records {
                let e = Entry { offset: r.offset, size: r.size, packed: r.len, codec: r.codec, block: Some(r.block) };
                entries.insert(normalize(&r.name), e);
                names.push(r.name);
            }
        } else {
            for r in baked::decode_index(&index)? {
                let e = Entry { offset: r.offset, size: r.size, packed: r.packed, codec: r.codec, block: None };
                entries.insert(normalize(&r.name), e);
                names.push(r.name);
            }
        }
        Ok(Pak {
            path,
            file,
            #[cfg(target_arch = "wasm32")]
            remote: None,
            entries,
            names,
            blocks,
            cache: Mutex::default(),
        })
    }

    fn file(&self) -> io::Result<&File> {
        self.file.as_ref().ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "pak has no local file"))
    }

    /// `len` bytes at `offset` inside stored block `b`.
    fn stored_range(&self, b: u32, offset: u64, len: u64) -> io::Result<Vec<u8>> {
        #[cfg(target_arch = "wasm32")]
        if let Some(r) = &self.remote {
            let block = r.get(b)?;
            return block
                .get(offset as usize..(offset + len) as usize)
                .map(<[u8]>::to_vec)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "range outside its stored block"));
        }
        let mut buf = vec![0u8; len as usize];
        read_exact_at(self.file()?, &mut buf, self.blocks[b as usize].offset + offset)?;
        Ok(buf)
    }

    /// The packed bytes of block `b`.
    fn packed_block(&self, b: u32) -> io::Result<Arc<Vec<u8>>> {
        #[cfg(target_arch = "wasm32")]
        if let Some(r) = &self.remote {
            return r.get(b);
        }
        let block = self.blocks[b as usize];
        let mut packed = vec![0u8; block.packed as usize];
        read_exact_at(self.file()?, &mut packed, block.offset)?;
        Ok(Arc::new(packed))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Number of files.
    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// All file names as stored (e.g. `game/ui/hud.dds`), in file-table order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.names.iter().map(String::as_str)
    }

    /// Names whose normalised form starts with the normalised `prefix` (also tried under `game/`).
    pub fn list<'a>(&'a self, prefix: &str) -> impl Iterator<Item = &'a str> + 'a {
        let p = normalize(prefix);
        let g = format!("game/{p}");
        self.names.iter().map(String::as_str).filter(move |n| {
            let n = normalize(n);
            n.starts_with(&p) || n.starts_with(&g)
        })
    }

    pub fn entry(&self, path: &str) -> Option<Entry> {
        let n = normalize(path);
        self.entries.get(&n).or_else(|| self.entries.get(&format!("game/{n}"))).copied()
    }

    pub fn exists(&self, path: &str) -> bool {
        self.entry(path).is_some() || overlay_file(path).is_some()
    }

    /// Whether reading `paths` will not wait on the network: always natively; in the browser, whether their blocks
    /// are in memory, and the missing ones start downloading in the background. Work that knows its files can wait
    /// for this (see `aa_cry::workers::Workers::spawn_after`) instead of blocking the page on each read.
    pub fn ready<'a>(&self, paths: impl IntoIterator<Item = &'a str>) -> bool {
        #[cfg(target_arch = "wasm32")]
        if self.remote.is_some() {
            let mut blocks: Vec<u32> = paths.into_iter().filter_map(|p| self.entry(p)?.block).collect();
            blocks.sort_unstable();
            blocks.dedup();
            return web::want(&blocks);
        }
        let _ = paths;
        true
    }

    /// Whole file contents.
    pub fn read(&self, path: &str) -> io::Result<Vec<u8>> {
        if let Some(file) = overlay_file(path) {
            return std::fs::read(file);
        }
        #[cfg(target_arch = "wasm32")]
        web::reading(path);
        let e = self.entry(path).ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, format!("not in game_pak: {path}"))
        })?;
        if let Some(b) = e.block {
            let block = self.blocks[b as usize];
            if block.kind == baked::BLOCK_STORED {
                return baked::decode_inner(e.codec, &self.stored_range(b, e.offset, e.packed)?);
            }
            let raw = self.block(b)?;
            let inner = raw
                .get(e.offset as usize..(e.offset + e.packed) as usize)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "baked v2 entry outside its block"))?;
            return baked::decode_inner(e.codec, inner);
        }
        let mut buf = vec![0u8; e.packed as usize];
        read_exact_at(self.file()?, &mut buf, e.offset)?;
        if e.codec == baked::STORED {
            return Ok(buf);
        }
        baked::unpack(e.codec, &buf, e.size as usize)
    }

    /// The bytes of a file as stored (compressed, transformed) in an original or v1 baked pak.
    pub fn read_packed(&self, path: &str) -> io::Result<Vec<u8>> {
        let e = self.entry(path).ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, format!("not in game_pak: {path}"))
        })?;
        if e.block.is_some() {
            return Err(io::Error::new(io::ErrorKind::Unsupported, "read_packed on a v2 baked pak"));
        }
        let mut buf = vec![0u8; e.packed as usize];
        read_exact_at(self.file()?, &mut buf, e.offset)?;
        Ok(buf)
    }

    /// A decompressed v2 block, from the cache when it was used recently.
    fn block(&self, b: u32) -> io::Result<Arc<Vec<u8>>> {
        {
            let mut cache = self.cache.lock().unwrap();
            if let Some(pos) = cache.iter().position(|c| c.0 == b) {
                let hit = cache.remove(pos);
                let data = hit.1.clone();
                cache.push(hit);
                return Ok(data);
            }
        }
        let block = self.blocks[b as usize];
        let packed = self.packed_block(b)?;
        #[cfg(target_arch = "wasm32")]
        let t0 = js_sys::Date::now();
        let raw = Arc::new(baked::unzstd(&packed, block.raw as usize)?);
        #[cfg(target_arch = "wasm32")]
        web::note_unzstd(b, raw.len(), js_sys::Date::now() - t0);
        let mut cache = self.cache.lock().unwrap();
        cache.push((b, raw.clone()));
        let mut total: usize = cache.iter().map(|c| c.1.len()).sum();
        while total > cache_budget() && cache.len() > 1 {
            total -= cache.remove(0).1.len();
        }
        Ok(raw)
    }

    /// Fills `buf` from `start` bytes into the file. A plain positioned read for stored files;
    /// a compressed file is decoded whole first.
    pub fn read_at(&self, path: &str, start: u64, buf: &mut [u8]) -> io::Result<()> {
        let e = self.entry(path).ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, format!("not in game_pak: {path}"))
        })?;
        if start + buf.len() as u64 > e.size {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        if e.codec == baked::STORED {
            match e.block {
                None => return read_exact_at(self.file()?, buf, e.offset + start),
                Some(b) if self.blocks[b as usize].kind == baked::BLOCK_STORED => {
                    buf.copy_from_slice(&self.stored_range(b, e.offset + start, buf.len() as u64)?);
                    return Ok(());
                }
                Some(_) => {}
            }
        }
        let all = self.read(path)?;
        buf.copy_from_slice(&all[start as usize..start as usize + buf.len()]);
        Ok(())
    }

    /// Whole file as text (UTF-8, BOM stripped; invalid bytes replaced).
    pub fn read_string(&self, path: &str) -> io::Result<String> {
        let b = self.read(path)?;
        let b = b.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&b);
        Ok(String::from_utf8_lossy(b).into_owned())
    }
}

/// Lowercase, `/` separators, no leading `./` or `/`.
pub fn normalize(path: &str) -> String {
    let p = path.replace('\\', "/").to_ascii_lowercase();
    let p = p.trim_start_matches("./").trim_start_matches('/');
    p.to_string()
}

fn decrypt_cbc(cipher: &Aes128, data: &mut [u8]) {
    let mut prev = [0u8; 16];
    for block in data.as_chunks_mut::<16>().0 {
        let ct = *block;
        cipher.decrypt_block(GenericArray::from_mut_slice(block));
        for (b, p) in block.iter_mut().zip(prev) {
            *b ^= p;
        }
        prev = ct;
    }
}

fn read_exact_at(file: &File, mut buf: &mut [u8], mut offset: u64) -> io::Result<()> {
    #[cfg(not(any(windows, unix)))]
    {
        let _ = (file, &mut buf, &mut offset);
        return Err(io::Error::new(io::ErrorKind::Unsupported, "no positioned file reads on this target"));
    }
    #[cfg(any(windows, unix))]
    while !buf.is_empty() {
        #[cfg(windows)]
        let n = std::os::windows::fs::FileExt::seek_read(file, buf, offset)?;
        #[cfg(unix)]
        let n = std::os::unix::fs::FileExt::read_at(file, buf, offset)?;
        if n == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        buf = &mut buf[n..];
        offset += n as u64;
    }
    #[cfg(any(windows, unix))]
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_paths() {
        assert_eq!(normalize("Game\\UI\\Hud.dds"), "game/ui/hud.dds");
        assert_eq!(normalize("./objects/a.cgf"), "objects/a.cgf");
    }

    /// Needs the real client (`AA_PAK` or `<client>/game_pak`).
    #[test]
    #[ignore]
    fn reads_real_pak() {
        let pak = Pak::global();
        assert_eq!(pak.len(), 218_066);
        let toc = pak.read_string("scriptsbin/x2ui/loginstage/character_create/toc.g").unwrap();
        assert!(toc.starts_with("## author"), "{}", &toc[..40]);
        let alb = pak.read("GAME\\SCRIPTSBIN\\X2UI\\LOGINSTAGE\\BASELIB.ALB").unwrap();
        assert!(alb.starts_with(b"\x1bLuaQ"));
        assert!(pak.list("ui/login_stage/").count() > 100);
    }
}
