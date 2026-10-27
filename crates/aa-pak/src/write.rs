//! Writing the ArcheAge 1.2 `game_pak` format: build a new pak ([`PakWriter`]) or patch an existing
//! one in place ([`patch`]), the way the original patcher and AAEmu's `AAPak` tool do.
//!
//! # Layout (verified against `<client>/game_pak`, 24,877,137,920 bytes, 218,066 files)
//!
//! ```text
//! [file data, each file at a 0x200-aligned offset, zero padded]   starts at 0
//! [file table: (files + extras) entries of 0x150 bytes]           0x200 aligned, right after the data
//! [zero bytes up to the next 0x200 boundary]                      (464 bytes in the real pak)
//! [header, 0x200 bytes]                                           the last 0x200 bytes of the file
//! ```
//!
//! **Header.** Only the first 0x20 bytes are encrypted (AES-128-CBC, zero IV, key [`super::KEY`]);
//! the other 0x1E0 bytes are stored as plain zeros. Decrypted:
//!
//! | off | type | value |
//! |---|---|---|
//! | 0x00 | `[u8;4]` | magic `WIBO` |
//! | 0x04 | u32 | 0 |
//! | 0x08 | u32 | file count (218,066) |
//! | 0x0C | u32 | extra ("free block") count (5) |
//! | 0x10 | 16 bytes | 0 |
//!
//! **Table.** Files first, then the extras. Each 0x150-byte entry is encrypted on its own
//! (AES-128-CBC, zero IV). The table is not sorted, either by name or by offset. Decrypted entry:
//!
//! | off | type | field | notes |
//! |---|---|---|---|
//! | 0x000 | `[u8;0x108]` | name | latin-1 (all ASCII here), NUL padded, lowercase, `/` separators, e.g. `game/ui/hud.dds`, `bin32/zlib1.dll` |
//! | 0x108 | i64 | offset | absolute, always 0x200 aligned |
//! | 0x110 | i64 | size | data length |
//! | 0x118 | i64 | size again | equal to `size` in all 218,071 entries (no compression in 1.2) |
//! | 0x120 | i32 | padding | reserved bytes after the data: `(size + padding) % 0x200 == 0` always. Usually `< 0x200`. It is larger (743 entries, up to 16 MB) where a smaller file was later written into an older file's slot |
//! | 0x124 | `[u8;16]` | MD5 | MD5 of the data. All zero for the 213,538 files of the base build (May-July 2014); set (and verified correct) on the 4,533 files that later patches wrote |
//! | 0x134 | u32 | 0 | always 0 in 1.2 (AAPak calls it `dummy1`; newer clients use 0x80000000) |
//! | 0x138 | i64 | create time | Windows FILETIME (100 ns since 1601, UTC) |
//! | 0x140 | i64 | modify time | FILETIME, `>= create`; equal on 150,009 entries |
//! | 0x148 | u64 | 0 | always 0 (`dummy2`) |
//!
//! **Extras** are free blocks left by deleted or moved files. Name `__unused__`, `offset` the block,
//! `size` = size again = the whole block (old `size + padding`), padding 0, MD5 and times 0. The
//! real pak has 5 (one of them empty: offset 0, size 0).
//!
//! The data region is gap-free: sorted by offset, every file starts at the previous file's
//! `offset + size + padding`, except for the 4 512-byte extras; the last file ends exactly where the
//! table starts. Padding bytes are zeros.
//!
//! # Patching (as the patcher / `AAPak` do)
//!
//! * The new data fits in the old entry's `size + padding`: it is written in place, and the padding
//!   grows to keep the slot.
//! * It does not fit: the old slot becomes an extra (`__unused__`). The new data goes into the
//!   first extra with `size >= new size` (padding = the rest of the block), or is appended where
//!   the table was (padding to 0x200).
//! * The table and the header are written again after the data, and the file is truncated or
//!   extended to the new end.
//! * Replaced entries keep their create time. Modify time is now. New entries get now for both.
//!   The MD5 is always set.
//!
//! A patch overwrites the old table before the new one is complete. [`patch_with_undo`] first
//! writes an undo file (the old table and header, plus the old bytes of every slot that is
//! overwritten in place). [`undo`] puts the pak back exactly as it was.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Read, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use aes::Aes128;
use aes::cipher::generic_array::GenericArray;
use aes::cipher::{BlockEncrypt, KeyInit};
use md5::{Digest, Md5};

use super::{ENTRY_SIZE, HEADER_SIZE, KEY, NAME_SIZE, decrypt_cbc, normalize, read_exact_at};

/// File data and the table are aligned to this.
pub const ALIGN: u64 = 0x200;
/// Name of the free-block entries.
pub const UNUSED: &str = "__unused__";
const MAGIC: &[u8; 4] = b"WIBO";
/// Bytes of the header that are encrypted; the rest of the 0x200 are plain zeros.
const HEADER_CRYPT: usize = 0x20;
const UNDO_MAGIC: &[u8; 8] = b"AAPKUND1";

/// One decrypted file-table entry. See the module docs for the layout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileInfo {
    pub name: String,
    pub offset: u64,
    pub size: u64,
    /// Second size field, equal to `size` in 1.2.
    pub size_dup: u64,
    pub padding: u32,
    pub md5: [u8; 16],
    pub dummy1: u32,
    pub create_time: i64,
    pub modify_time: i64,
    pub dummy2: u64,
}

impl FileInfo {
    /// The bytes this entry owns: `size + padding`.
    pub fn reserved(&self) -> u64 {
        self.size + self.padding as u64
    }

    fn free_block(offset: u64, len: u64) -> FileInfo {
        FileInfo {
            name: UNUSED.into(),
            offset,
            size: len,
            size_dup: len,
            padding: 0,
            md5: [0; 16],
            dummy1: 0,
            create_time: 0,
            modify_time: 0,
            dummy2: 0,
        }
    }

    pub fn decode(raw: &[u8; ENTRY_SIZE]) -> FileInfo {
        let u64_at = |o: usize| u64::from_le_bytes(raw[o..o + 8].try_into().unwrap());
        let u32_at = |o: usize| u32::from_le_bytes(raw[o..o + 4].try_into().unwrap());
        let name = &raw[..NAME_SIZE];
        let name = &name[..name.iter().position(|&b| b == 0).unwrap_or(NAME_SIZE)];
        FileInfo {
            name: name.iter().map(|&b| b as char).collect(),
            offset: u64_at(0x108),
            size: u64_at(0x110),
            size_dup: u64_at(0x118),
            padding: u32_at(0x120),
            md5: raw[0x124..0x134].try_into().unwrap(),
            dummy1: u32_at(0x134),
            create_time: u64_at(0x138) as i64,
            modify_time: u64_at(0x140) as i64,
            dummy2: u64_at(0x148),
        }
    }

    pub fn encode(&self) -> io::Result<[u8; ENTRY_SIZE]> {
        let mut raw = [0u8; ENTRY_SIZE];
        if self.name.chars().count() >= NAME_SIZE || self.name.chars().any(|c| c as u32 > 0xFF || c == '\0') {
            return Err(invalid(format!("pak name too long or not latin-1: {}", self.name)));
        }
        for (b, c) in raw.iter_mut().zip(self.name.chars()) {
            *b = c as u8;
        }
        raw[0x108..0x110].copy_from_slice(&self.offset.to_le_bytes());
        raw[0x110..0x118].copy_from_slice(&self.size.to_le_bytes());
        raw[0x118..0x120].copy_from_slice(&self.size_dup.to_le_bytes());
        raw[0x120..0x124].copy_from_slice(&self.padding.to_le_bytes());
        raw[0x124..0x134].copy_from_slice(&self.md5);
        raw[0x134..0x138].copy_from_slice(&self.dummy1.to_le_bytes());
        raw[0x138..0x140].copy_from_slice(&self.create_time.to_le_bytes());
        raw[0x140..0x148].copy_from_slice(&self.modify_time.to_le_bytes());
        raw[0x148..0x150].copy_from_slice(&self.dummy2.to_le_bytes());
        Ok(raw)
    }
}

/// The decrypted file table of a pak.
#[derive(Clone, Debug)]
pub struct Table {
    pub files: Vec<FileInfo>,
    /// Free blocks (`__unused__`).
    pub extras: Vec<FileInfo>,
    /// Where the table starts = where the file data ends.
    pub offset: u64,
}

impl Table {
    /// Encrypted table, zero padding and header: everything from [`Table::offset`] to the end.
    pub fn tail(&self) -> io::Result<Vec<u8>> {
        let cipher = Aes128::new(GenericArray::from_slice(&KEY));
        let count = self.files.len() + self.extras.len();
        let mut out = Vec::with_capacity(count * ENTRY_SIZE + 2 * HEADER_SIZE as usize);
        for e in self.files.iter().chain(&self.extras) {
            let mut raw = e.encode()?;
            encrypt_cbc(&cipher, &mut raw);
            out.extend_from_slice(&raw);
        }
        out.resize(align(out.len() as u64) as usize, 0);
        out.extend_from_slice(&header(self.files.len(), self.extras.len()));
        Ok(out)
    }
}

/// Reads and decrypts the whole table of a `WIBO` pak (the 1.2 format).
pub fn read_table(path: impl AsRef<Path>) -> io::Result<Table> {
    let file = File::open(path)?;
    read_table_from(&file, file.metadata()?.len())
}

fn read_table_from(file: &File, len: u64) -> io::Result<Table> {
    if len < HEADER_SIZE {
        return Err(invalid("file too small for a game_pak".into()));
    }
    let cipher = Aes128::new(GenericArray::from_slice(&KEY));
    let mut h = [0u8; HEADER_CRYPT];
    read_exact_at(file, &mut h, len - HEADER_SIZE)?;
    decrypt_cbc(&cipher, &mut h);
    if &h[0..4] != MAGIC {
        return Err(invalid("not a WIBO (1.2) game_pak; only that format can be written".into()));
    }
    let files = u32::from_le_bytes(h[8..12].try_into().unwrap()) as usize;
    let extras = u32::from_le_bytes(h[12..16].try_into().unwrap()) as usize;
    let table_len = ((files + extras) * ENTRY_SIZE) as u64;
    let end = (len - HEADER_SIZE).checked_sub(table_len).ok_or_else(|| invalid("bad game_pak table size".into()))?;
    let offset = end - end % ALIGN;
    let mut table = vec![0u8; table_len as usize];
    read_exact_at(file, &mut table, offset)?;
    let mut all: Vec<FileInfo> = table
        .as_chunks_mut::<ENTRY_SIZE>()
        .0
        .iter_mut()
        .map(|raw| {
            decrypt_cbc(&cipher, raw);
            FileInfo::decode(raw)
        })
        .collect();
    let extras = all.split_off(files);
    Ok(Table { files: all, extras, offset })
}

/// Builds a new pak from `(name, bytes)` entries. Names are stored normalised (lowercase, `/`);
/// adding a name twice keeps the last bytes.
#[derive(Default)]
pub struct PakWriter {
    files: Vec<(String, Vec<u8>)>,
    index: HashMap<String, usize>,
}

impl PakWriter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, name: &str, data: Vec<u8>) -> &mut Self {
        let name = normalize(name);
        match self.index.get(&name) {
            Some(&i) => self.files[i].1 = data,
            None => {
                self.index.insert(name.clone(), self.files.len());
                self.files.push((name, data));
            }
        }
        self
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Writes the pak (creating or truncating `path`).
    pub fn write(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let now = filetime(SystemTime::now());
        let mut out = BufWriter::with_capacity(1 << 20, File::create(path)?);
        let mut table = Table { files: Vec::with_capacity(self.files.len()), extras: Vec::new(), offset: 0 };
        for (name, data) in &self.files {
            let size = data.len() as u64;
            let padding = align(size) - size;
            out.write_all(data)?;
            write_zeros(&mut out, padding)?;
            table.files.push(new_entry(name.clone(), table.offset, data, padding as u32, now, now));
            table.offset += size + padding;
        }
        out.write_all(&table.tail()?)?;
        out.into_inner().map_err(|e| e.into_error())?.sync_all()
    }
}

/// What [`patch`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PatchReport {
    /// Existing entries rewritten in their own slot.
    pub in_place: usize,
    /// Entries written into a free block (`__unused__` extra).
    pub reused: usize,
    /// Entries appended after the old data.
    pub appended: usize,
    /// How many of the entries were new names (the rest replaced existing ones).
    pub new_names: usize,
    /// Data bytes written.
    pub bytes: u64,
    pub old_len: u64,
    pub new_len: u64,
}

/// Replaces (same name, case-insensitive) or adds `files` in the pak at `pak_path`, in place.
/// See the module docs for the slot rules.
pub fn patch(pak_path: impl AsRef<Path>, files: &[(String, Vec<u8>)]) -> io::Result<PatchReport> {
    patch_with_undo(pak_path, files, None::<&Path>)
}

/// [`patch`], first writing an undo file (if given) that [`undo`] can apply.
pub fn patch_with_undo(
    pak_path: impl AsRef<Path>,
    files: &[(String, Vec<u8>)],
    undo_path: Option<impl AsRef<Path>>,
) -> io::Result<PatchReport> {
    let file = OpenOptions::new().read(true).write(true).open(pak_path)?;
    let old_len = file.metadata()?.len();
    let mut table = read_table_from(&file, old_len)?;
    let old_table_at = table.offset;
    let now = filetime(SystemTime::now());

    // the last bytes per name win
    let mut wanted: Vec<(String, &[u8])> = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    for (name, data) in files {
        let n = normalize(name);
        match seen.get(&n) {
            Some(&i) => wanted[i].1 = data,
            None => {
                seen.insert(n.clone(), wanted.len());
                wanted.push((n, data));
            }
        }
    }

    let mut index: HashMap<String, usize> =
        table.files.iter().enumerate().map(|(i, f)| (normalize(&f.name), i)).collect();
    let mut report = PatchReport { old_len, ..Default::default() };
    // (offset, data, zero fill end)
    let mut writes: Vec<(u64, &[u8], u64)> = Vec::new();
    for (name, data) in wanted {
        let size = data.len() as u64;
        report.bytes += size;
        let old = index.get(&name).copied();
        if let Some(i) = old {
            let f = &mut table.files[i];
            if size <= f.reserved() {
                let end = f.offset + f.reserved();
                *f = FileInfo {
                    padding: (end - f.offset - size) as u32,
                    modify_time: now.max(f.create_time),
                    ..new_entry(f.name.clone(), f.offset, data, 0, f.create_time, now)
                };
                writes.push((f.offset, data, align(f.offset + size).min(end)));
                report.in_place += 1;
                continue;
            }
        }
        // a new slot: the first free block that fits, else the end of the data
        let (offset, padding) =
            match table.extras.iter().position(|e| e.name == UNUSED && e.size > 0 && size <= e.size) {
                Some(j) => {
                    let e = table.extras.remove(j);
                    report.reused += 1;
                    (e.offset, e.size - size)
                }
                None => {
                    let at = table.offset;
                    let padding = align(size) - size;
                    table.offset += size + padding;
                    report.appended += 1;
                    (at, padding)
                }
            };
        let padding = u32::try_from(padding).map_err(|_| invalid(format!("padding too large for {name}")))?;
        let zero_end = align(offset + size).min(offset + size + padding as u64);
        writes.push((offset, data, zero_end));
        match old {
            Some(i) => {
                let f = &table.files[i];
                table.extras.push(FileInfo::free_block(f.offset, f.reserved()));
                let create = f.create_time;
                table.files[i] = new_entry(f.name.clone(), offset, data, padding, create, now.max(create));
            }
            None => {
                index.insert(name.clone(), table.files.len());
                table.files.push(new_entry(name, offset, data, padding, now, now));
                report.new_names += 1;
            }
        }
    }

    let tail = table.tail()?;
    report.new_len = table.offset + tail.len() as u64;

    if let Some(undo_path) = undo_path {
        // the old table + header, and the old bytes of every slot below it that gets overwritten
        let mut regions = vec![(old_table_at, old_len - old_table_at)];
        regions.extend(
            writes.iter().filter(|w| w.0 < old_table_at).map(|&(at, _, end)| (at, end.min(old_table_at) - at)),
        );
        let mut out = BufWriter::new(File::create(undo_path)?);
        out.write_all(UNDO_MAGIC)?;
        out.write_all(&old_len.to_le_bytes())?;
        out.write_all(&(regions.len() as u32).to_le_bytes())?;
        for (at, len) in regions {
            let mut buf = vec![0u8; len as usize];
            read_exact_at(&file, &mut buf, at)?;
            out.write_all(&at.to_le_bytes())?;
            out.write_all(&len.to_le_bytes())?;
            out.write_all(&buf)?;
        }
        out.into_inner().map_err(|e| e.into_error())?.sync_all()?;
    }

    for &(at, data, zero_end) in &writes {
        write_all_at(&file, data, at)?;
        let zeros = zero_end - at - data.len() as u64;
        if zeros > 0 {
            write_all_at(&file, &vec![0u8; zeros as usize], at + data.len() as u64)?;
        }
    }
    write_all_at(&file, &tail, table.offset)?;
    file.set_len(report.new_len)?;
    file.sync_all()?;
    Ok(report)
}

/// Puts a pak back to the state saved by [`patch_with_undo`].
pub fn undo(pak_path: impl AsRef<Path>, undo_path: impl AsRef<Path>) -> io::Result<()> {
    let mut log = Vec::new();
    File::open(undo_path)?.read_to_end(&mut log)?;
    let bad = || invalid("not an aa-pak undo file".into());
    if log.len() < 20 || &log[..8] != UNDO_MAGIC {
        return Err(bad());
    }
    let u64_at = |o: usize| log.get(o..o + 8).map(|b| u64::from_le_bytes(b.try_into().unwrap()));
    let old_len = u64_at(8).ok_or_else(bad)?;
    let count = u32::from_le_bytes(log[16..20].try_into().unwrap());
    let file = OpenOptions::new().write(true).open(pak_path)?;
    let mut at = 20usize;
    for _ in 0..count {
        let (offset, len) = (u64_at(at).ok_or_else(bad)?, u64_at(at + 8).ok_or_else(bad)? as usize);
        let bytes = log.get(at + 16..at + 16 + len).ok_or_else(bad)?;
        write_all_at(&file, bytes, offset)?;
        at += 16 + len;
    }
    file.set_len(old_len)?;
    file.sync_all()
}

/// MD5 of `data`, as stored in the table.
pub fn md5(data: &[u8]) -> [u8; 16] {
    Md5::digest(data).into()
}

/// Windows FILETIME (100 ns ticks since 1601-01-01 UTC).
pub fn filetime(t: SystemTime) -> i64 {
    const EPOCH_DIFF: i64 = 116_444_736_000_000_000;
    let unix = match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_nanos() as i64 / 100,
        Err(e) => -(e.duration().as_nanos() as i64 / 100),
    };
    EPOCH_DIFF + unix
}

fn new_entry(name: String, offset: u64, data: &[u8], padding: u32, create: i64, modify: i64) -> FileInfo {
    FileInfo {
        name,
        offset,
        size: data.len() as u64,
        size_dup: data.len() as u64,
        padding,
        md5: md5(data),
        dummy1: 0,
        create_time: create,
        modify_time: modify,
        dummy2: 0,
    }
}

fn header(files: usize, extras: usize) -> [u8; HEADER_SIZE as usize] {
    let mut h = [0u8; HEADER_SIZE as usize];
    h[0..4].copy_from_slice(MAGIC);
    h[8..12].copy_from_slice(&(files as u32).to_le_bytes());
    h[12..16].copy_from_slice(&(extras as u32).to_le_bytes());
    let cipher = Aes128::new(GenericArray::from_slice(&KEY));
    encrypt_cbc(&cipher, &mut h[..HEADER_CRYPT]);
    h
}

fn align(n: u64) -> u64 {
    n.div_ceil(ALIGN) * ALIGN
}

fn invalid(msg: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

fn encrypt_cbc(cipher: &Aes128, data: &mut [u8]) {
    let mut prev = [0u8; 16];
    for block in data.as_chunks_mut::<16>().0 {
        for (b, p) in block.iter_mut().zip(prev) {
            *b ^= p;
        }
        cipher.encrypt_block(GenericArray::from_mut_slice(block));
        prev = *block;
    }
}

fn write_zeros(out: &mut impl Write, n: u64) -> io::Result<()> {
    const Z: [u8; ALIGN as usize] = [0; ALIGN as usize];
    let mut n = n;
    while n > 0 {
        let k = n.min(ALIGN);
        out.write_all(&Z[..k as usize])?;
        n -= k;
    }
    Ok(())
}

fn write_all_at(file: &File, mut buf: &[u8], mut offset: u64) -> io::Result<()> {
    while !buf.is_empty() {
        #[cfg(windows)]
        let n = std::os::windows::fs::FileExt::seek_write(file, buf, offset)?;
        #[cfg(unix)]
        let n = std::os::unix::fs::FileExt::write_at(file, buf, offset)?;
        if n == 0 {
            return Err(io::ErrorKind::WriteZero.into());
        }
        buf = &buf[n..];
        offset += n as u64;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::Pak;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aa-pak-write-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    fn bytes(n: usize, seed: u8) -> Vec<u8> {
        (0..n).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed)).collect()
    }

    /// Every file is aligned, owns `size + padding` bytes, hashes right, and no two slots overlap.
    fn check_layout(path: &Path) -> Table {
        let t = read_table(path).unwrap();
        let data = std::fs::read(path).unwrap();
        let mut slots: Vec<(u64, u64)> = Vec::new();
        for f in &t.files {
            assert_eq!(f.offset % ALIGN, 0, "{}", f.name);
            assert_eq!(f.reserved() % ALIGN, 0, "{}", f.name);
            assert_eq!(f.size, f.size_dup);
            let d = &data[f.offset as usize..(f.offset + f.size) as usize];
            assert_eq!(md5(d), f.md5, "md5 of {}", f.name);
            assert!(f.modify_time >= f.create_time);
            slots.push((f.offset, f.reserved()));
        }
        slots.extend(t.extras.iter().map(|e| (e.offset, e.size)));
        slots.sort();
        for w in slots.windows(2) {
            assert!(w[0].0 + w[0].1 <= w[1].0, "overlap {w:?}");
        }
        assert_eq!(t.offset % ALIGN, 0);
        assert!(slots.last().is_none_or(|s| s.0 + s.1 <= t.offset));
        t
    }

    #[test]
    fn writes_and_reads_back() {
        let path = scratch("small.pak");
        let files = [("game/a.txt", bytes(1000, 1)), ("Game\\Sub\\B.dds", bytes(0x200, 2)), ("game/empty", vec![])];
        let mut w = PakWriter::new();
        for (n, d) in &files {
            w.add(n, d.clone());
        }
        w.write(&path).unwrap();

        let pak = Pak::open(&path).unwrap();
        assert_eq!(pak.len(), 3);
        assert_eq!(pak.names().collect::<Vec<_>>(), ["game/a.txt", "game/sub/b.dds", "game/empty"]);
        for (n, d) in &files {
            assert_eq!(&pak.read(n).unwrap(), d, "{n}");
        }
        assert_eq!(pak.read("sub/b.dds").unwrap(), files[1].1);
        let t = check_layout(&path);
        assert_eq!(t.files[0].padding, 24);
        assert_eq!(t.files[1].padding, 0);
        assert_eq!(t.offset, 0x400 + 0x200);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn patches_in_place_reuses_and_appends() {
        let path = scratch("patch.pak");
        let mut w = PakWriter::new();
        w.add("game/a", bytes(1000, 1)).add("game/b", bytes(600, 2)).add("game/c", bytes(5000, 3));
        w.write(&path).unwrap();
        let original = std::fs::read(&path).unwrap();
        let before = read_table(&path).unwrap();

        let files = vec![
            ("game/a".to_string(), bytes(1020, 9)),  // fits its 1024 slot
            ("GAME/B".to_string(), bytes(2000, 8)),  // does not fit 1024: moves to the end
            ("game/new/d".to_string(), bytes(300, 7)), // new, goes into b's freed slot
            ("game/new/e".to_string(), bytes(700, 6)), // new, appended
        ];
        let undo_path = scratch("patch.undo");
        let r = patch_with_undo(&path, &files, Some(&undo_path)).unwrap();
        assert_eq!((r.in_place, r.reused, r.appended, r.new_names), (1, 1, 2, 2), "{r:?}");

        let pak = Pak::open(&path).unwrap();
        assert_eq!(pak.len(), 5);
        for (n, d) in &files {
            assert_eq!(&pak.read(n).unwrap(), d, "{n}");
        }
        assert_eq!(pak.read("game/c").unwrap(), bytes(5000, 3));
        let t = check_layout(&path);
        assert!(t.extras.is_empty(), "b's old slot was reused by d: {:?}", t.extras);
        assert_eq!(t.files[2], before.files[2], "untouched entry kept as is");
        assert_eq!(t.files[0].create_time, before.files[0].create_time);
        assert_eq!(t.files[0].offset, before.files[0].offset);
        assert_eq!(t.files[3].offset, before.files[1].offset);
        assert_eq!(t.files[1].offset, before.offset);
        // the untouched file's bytes are the same on disk
        let c = &before.files[2];
        let now = std::fs::read(&path).unwrap();
        assert_eq!(now[c.offset as usize..][..c.size as usize], original[c.offset as usize..][..c.size as usize]);

        // a second patch that frees a slot nothing reuses leaves an extra
        patch(&path, &[("game/a".into(), bytes(3000, 5))]).unwrap();
        let t = check_layout(&path);
        assert_eq!(t.extras.len(), 1);
        assert_eq!((t.extras[0].offset, t.extras[0].size), (0, 1024));
        assert_eq!(Pak::open(&path).unwrap().read("game/a").unwrap(), bytes(3000, 5));

        // undo the first patch on a fresh copy of the original
        std::fs::write(&path, &original).unwrap();
        patch_with_undo(&path, &files, Some(&undo_path)).unwrap();
        undo(&path, &undo_path).unwrap();
        assert!(std::fs::read(&path).unwrap() == original, "undo restores the pak byte for byte");
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(undo_path);
    }

    /// The real 1.2 table and header re-encode to exactly the same bytes.
    /// Needs the real client (`AA_PAK` or `<client>/game_pak`).
    #[test]
    #[ignore]
    fn real_table_roundtrip() {
        let path = std::env::var("AA_PAK").unwrap_or_else(|_| crate::DEFAULT_PAK.into());
        let Ok(file) = File::open(&path) else { return };
        let len = file.metadata().unwrap().len();
        let t = read_table_from(&file, len).unwrap();
        assert_eq!((t.files.len(), t.extras.len()), (218_066, 5));
        let mut raw = vec![0u8; (len - t.offset) as usize];
        read_exact_at(&file, &mut raw, t.offset).unwrap();
        assert!(t.tail().unwrap() == raw, "re-encoded table + header differ from the real pak");
        assert!(t.files.iter().all(|f| f.size == f.size_dup && f.reserved() % ALIGN == 0 && f.dummy1 == 0));
        assert!(t.extras.iter().all(|e| e.name == UNUSED));
    }

    /// A small pak built from 20 real files reads back byte-identical, and a patched real
    /// heightmap reads back. Needs the real client.
    #[test]
    #[ignore]
    fn real_files_small_pak() {
        let path = std::env::var("AA_PAK").unwrap_or_else(|_| crate::DEFAULT_PAK.into());
        if !Path::new(&path).is_file() {
            return;
        }
        let real = Pak::open(&path).unwrap();
        let hm = "game/worlds/main_world/cells/015_015/client/terrain/heightmap.dat";
        // 20 real files, without the heightmap (the patch adds it)
        let names: Vec<String> = ["game/worlds/main_world/cells/015_015/", "game/scriptsbin/x2ui/loginstage/", "game/ui/icon/"]
            .iter()
            .flat_map(|p| real.list(p).filter(|n| *n != hm).take(7).map(str::to_string).collect::<Vec<_>>())
            .take(20)
            .collect();
        assert_eq!(names.len(), 20);
        let out = scratch("real20.pak");
        let mut w = PakWriter::new();
        let mut originals = Vec::new();
        for n in &names {
            let d = real.read(n).unwrap();
            w.add(n, d.clone());
            originals.push(d);
        }
        w.write(&out).unwrap();
        let small = Pak::open(&out).unwrap();
        assert_eq!(small.names().collect::<Vec<_>>(), names);
        for (n, d) in names.iter().zip(&originals) {
            assert!(&small.read(n).unwrap() == d, "{n}");
        }
        // patch: the first file grows past its slot, a heightmap-sized new file is added
        let grown: Vec<u8> = originals[0].iter().chain(&vec![0xAB; 4096]).copied().collect();
        let hm_bytes = real.read(hm).unwrap();
        let r = patch(&out, &[(names[0].clone(), grown.clone()), (hm.into(), hm_bytes.clone())]).unwrap();
        assert_eq!((r.in_place, r.appended, r.new_names), (0, 2, 1), "{r:?}");
        let small = Pak::open(&out).unwrap();
        assert!(small.read(&names[0]).unwrap() == grown);
        assert!(small.read(hm).unwrap() == hm_bytes);
        for (n, d) in names.iter().zip(&originals).skip(1) {
            assert!(&small.read(n).unwrap() == d, "{n} changed after {r:?}");
        }
        check_layout(&out);
        let _ = std::fs::remove_file(out);
    }
}
