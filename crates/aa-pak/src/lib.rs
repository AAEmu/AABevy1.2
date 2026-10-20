//! Read-only access to the ArcheAge 1.2 `game_pak` archive.
//!
//! Layout (little endian): `[file data][file table][pad to 0x200][header 0x200]`. The header and
//! every 0x150-byte file table entry are AES-128-CBC encrypted on their own with a zero IV and the
//! XLGames key. File data is stored raw (no compression, no encryption).
//!
//! [`Pak::open`] also opens the compressed pak written by `aa-bake` (see [`baked`]); both behave
//! the same through this API.
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
use std::sync::OnceLock;

use aes::Aes128;
use aes::cipher::generic_array::GenericArray;
use aes::cipher::{BlockDecrypt, KeyInit};

pub mod baked;

/// Where the 1.2 client lives on this machine; override with the `AA_PAK` environment variable.
pub const DEFAULT_PAK: &str = "D:/AA1.2/game_pak";

const KEY: [u8; 16] = [
    0x32, 0x1F, 0x2A, 0xEE, 0xAA, 0x58, 0x4A, 0xB4, 0x9A, 0x6C, 0x9E, 0x09, 0xD5, 0x9E, 0x9C, 0x6F,
];
const HEADER_SIZE: u64 = 0x200;
const ENTRY_SIZE: usize = 0x150;
const NAME_SIZE: usize = 0x108;

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
}

/// An open `game_pak`. Cheap to share: reads use positioned I/O and take `&self`.
pub struct Pak {
    path: PathBuf,
    file: File,
    /// Normalised (lowercase, `/`) name -> entry.
    entries: HashMap<String, Entry>,
    /// Original names in file-table order.
    names: Vec<String>,
}

impl Pak {
    /// The process-wide pak (`AA_PAK` or [`DEFAULT_PAK`]). Panics if it cannot be opened.
    pub fn global() -> &'static Pak {
        static PAK: OnceLock<Pak> = OnceLock::new();
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
            if &footer[..8] == baked::MAGIC {
                return Self::open_baked(path, file, &footer);
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
            entries.insert(normalize(&name), Entry { offset, size, packed: size, codec: baked::STORED });
            names.push(name);
        }
        Ok(Pak { path, file, entries, names })
    }

    fn open_baked(path: PathBuf, file: File, footer: &[u8]) -> io::Result<Pak> {
        let u64_at = |o: usize| u64::from_le_bytes(footer[o..o + 8].try_into().unwrap());
        let mut index = vec![0u8; u64_at(16) as usize];
        read_exact_at(&file, &mut index, u64_at(8))?;
        let index = zstd::bulk::decompress(&index, u64_at(24) as usize)?;
        let records = baked::decode_index(&index)?;
        let mut entries = HashMap::with_capacity(records.len());
        let mut names = Vec::with_capacity(records.len());
        for r in records {
            let e = Entry { offset: r.offset, size: r.size, packed: r.packed, codec: r.codec };
            entries.insert(normalize(&r.name), e);
            names.push(r.name);
        }
        Ok(Pak { path, file, entries, names })
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
        self.entry(path).is_some()
    }

    /// Whole file contents.
    pub fn read(&self, path: &str) -> io::Result<Vec<u8>> {
        let e = self.entry(path).ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, format!("not in game_pak: {path}"))
        })?;
        let mut buf = vec![0u8; e.packed as usize];
        read_exact_at(&self.file, &mut buf, e.offset)?;
        if e.codec == baked::STORED {
            return Ok(buf);
        }
        baked::unpack(e.codec, &buf, e.size as usize)
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
            return read_exact_at(&self.file, buf, e.offset + start);
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

    /// Needs the real client (`AA_PAK` or `D:/AA1.2/game_pak`).
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
