//! Byte-level packet reader / writer (the 1.2 `PacketStream`).
//!
//! Everything is little-endian. Strings and byte blobs are prefixed by a 16-bit length, "bc" is a
//! 24-bit unsigned integer (object ids) and positions are the 9-byte packed world coordinate.

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WireError {
    #[error("wanted {wanted} bytes at offset {pos} but the packet has only {len}")]
    Eof { pos: usize, wanted: usize, len: usize },
    #[error("invalid packet: {0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, WireError>;

/// A cursor over a received packet body.
#[derive(Clone)]
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    pub fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    /// The bytes not read yet.
    pub fn rest(&self) -> &'a [u8] {
        &self.buf[self.pos..]
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.remaining() < n {
            return Err(WireError::Eof {
                pos: self.pos,
                wanted: n,
                len: self.buf.len(),
            });
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut a = [0u8; N];
        a.copy_from_slice(self.take(N)?);
        Ok(a)
    }

    pub fn skip(&mut self, n: usize) -> Result<()> {
        self.take(n).map(|_| ())
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    pub fn i8(&mut self) -> Result<i8> {
        Ok(self.u8()? as i8)
    }
    /// The 1.2 `ReadBoolean`: exactly `1` is true.
    pub fn bool(&mut self) -> Result<bool> {
        Ok(self.u8()? == 1)
    }
    pub fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.array()?))
    }
    pub fn i16(&mut self) -> Result<i16> {
        Ok(i16::from_le_bytes(self.array()?))
    }
    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }
    pub fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.array()?))
    }
    pub fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.array()?))
    }
    pub fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_le_bytes(self.array()?))
    }
    pub fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_le_bytes(self.array()?))
    }

    /// 24-bit unsigned integer ("bc"), used for object ids.
    pub fn bc(&mut self) -> Result<u32> {
        let b = self.array::<3>()?;
        Ok(u32::from(b[0]) | u32::from(b[1]) << 8 | u32::from(b[2]) << 16)
    }

    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        self.take(n)
    }

    /// A blob prefixed by an `i16` length (negative counts as empty).
    pub fn blob(&mut self) -> Result<&'a [u8]> {
        let n = self.i16()?.max(0) as usize;
        self.take(n)
    }

    /// A UTF-8 string prefixed by an `i16` length; NULs are trimmed.
    pub fn string(&mut self) -> Result<String> {
        Ok(String::from_utf8_lossy(self.blob()?).trim_matches('\0').to_string())
    }

    /// A "pisc" group of `n` (at most 4) integers: one header byte with two bits per value
    /// (`0` = `u8`, `1` = `u16`, `2` = 24-bit `bc`, `3` = `u32`), then the values. Unused entries
    /// of the result are 0.
    pub fn pisc(&mut self, n: usize) -> Result<[i64; 4]> {
        if n > 4 {
            return Err(WireError::Invalid(format!("pisc group of {n} values")));
        }
        let header = self.u8()?;
        let mut out = [0i64; 4];
        for (i, v) in out.iter_mut().enumerate().take(n) {
            *v = match (header >> (i * 2)) & 3 {
                0 => i64::from(self.u8()?),
                1 => i64::from(self.u16()?),
                2 => i64::from(self.bc()?),
                _ => i64::from(self.u32()?),
            };
        }
        Ok(out)
    }

    /// Looks at the next byte without consuming it.
    pub fn peek_u8(&self) -> Result<u8> {
        self.buf.get(self.pos).copied().ok_or(WireError::Eof { pos: self.pos, wanted: 1, len: self.buf.len() })
    }

    /// A 9-byte packed world position.
    pub fn position(&mut self) -> Result<Pos> {
        let b = self.array::<9>()?;
        Ok(decode_position(&b))
    }

    /// A position as two fixed-point `i64` (`<< 32`, 1/4096 m) plus a float height, as used by the
    /// character list.
    pub fn long_position(&mut self) -> Result<Pos> {
        let x = self.i64()?;
        let y = self.i64()?;
        let z = self.f32()?;
        Ok(Pos::new(long_to_coord(x), long_to_coord(y), z))
    }
}

/// Builds a packet body.
#[derive(Default, Clone, Debug)]
pub struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn finish(self) -> Vec<u8> {
        self.buf
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.buf
    }

    pub fn u8(&mut self, v: u8) -> &mut Self {
        self.buf.push(v);
        self
    }
    pub fn i8(&mut self, v: i8) -> &mut Self {
        self.u8(v as u8)
    }
    pub fn bool(&mut self, v: bool) -> &mut Self {
        self.u8(u8::from(v))
    }
    pub fn u16(&mut self, v: u16) -> &mut Self {
        self.raw(&v.to_le_bytes())
    }
    pub fn i16(&mut self, v: i16) -> &mut Self {
        self.raw(&v.to_le_bytes())
    }
    pub fn u32(&mut self, v: u32) -> &mut Self {
        self.raw(&v.to_le_bytes())
    }
    pub fn i32(&mut self, v: i32) -> &mut Self {
        self.raw(&v.to_le_bytes())
    }
    pub fn u64(&mut self, v: u64) -> &mut Self {
        self.raw(&v.to_le_bytes())
    }
    pub fn i64(&mut self, v: i64) -> &mut Self {
        self.raw(&v.to_le_bytes())
    }
    pub fn f32(&mut self, v: f32) -> &mut Self {
        self.raw(&v.to_le_bytes())
    }
    pub fn bc(&mut self, v: u32) -> &mut Self {
        self.raw(&v.to_le_bytes()[..3])
    }
    pub fn raw(&mut self, bytes: &[u8]) -> &mut Self {
        self.buf.extend_from_slice(bytes);
        self
    }
    /// A blob prefixed by a `u16` length.
    pub fn blob(&mut self, bytes: &[u8]) -> &mut Self {
        self.u16(bytes.len() as u16).raw(bytes)
    }
    pub fn string(&mut self, s: &str) -> &mut Self {
        self.blob(s.as_bytes())
    }
    /// The server's `WritePisc`: up to four integers behind one header byte (see
    /// [`Reader::pisc`]); values above `0xFF` use the wider forms.
    pub fn pisc(&mut self, values: &[i64]) -> &mut Self {
        debug_assert!(values.len() <= 4);
        let mut header = 0u8;
        let mut body = Writer::new();
        for (i, &v) in values.iter().enumerate() {
            if v <= 0xFF {
                body.u8(v as u8);
            } else if v <= 0xFFFF {
                header |= 1 << (i * 2);
                body.u16(v as u16);
            } else if v <= 0xFF_FFFF {
                header |= 1 << (i * 2 + 1);
                body.bc(v as u32);
            } else {
                header |= 3 << (i * 2);
                body.u32(v as u32);
            }
        }
        self.u8(header).raw(&body.buf)
    }
    pub fn position(&mut self, p: Pos) -> &mut Self {
        self.raw(&encode_position(p))
    }
    pub fn long_position(&mut self, p: Pos) -> &mut Self {
        self.i64(coord_to_long(p.x)).i64(coord_to_long(p.y)).f32(p.z)
    }
}

/// A world position in meters (x east, y north, z up).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pos {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Pos {
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    pub fn distance(self, o: Pos) -> f32 {
        let (dx, dy, dz) = (self.x - o.x, self.y - o.y, self.z - o.z);
        (dx * dx + dy * dy + dz * dz).sqrt()
    }
}

/// Fixed point (`<< 32`, 1/4096 m) to meters.
pub fn long_to_coord(v: i64) -> f32 {
    (v >> 32) as f32 / 4096.0
}

/// Meters to fixed point (`<< 32`, 1/4096 m).
pub fn coord_to_long(v: f32) -> i64 {
    ((v * 4096.0) as i64) << 32
}

/// Decodes the 9-byte packed position: x / y are 24-bit fixed point (1/512 m) with their sign in the
/// top bits of byte 8, z is 22-bit fixed point over 4196 m starting at -100.
pub fn decode_position(v: &[u8; 9]) -> Pos {
    let raw = |a: u8, b: u8, c: u8| 8 * (i64::from(a) | i64::from(b) << 8 | i64::from(c) << 16);
    let mut x = raw(v[0], v[1], v[2]) << 32;
    let mut y = raw(v[3], v[4], v[5]) << 32;
    if v[8] & 0x80 != 0 {
        x = x.wrapping_neg();
    }
    if v[8] & 0x40 != 0 {
        y = y.wrapping_neg();
    }
    let z_raw = u64::from(v[6]) | u64::from(v[7]) << 8 | u64::from(v[8] & 0x3F) << 16;
    let z = z_raw as f64 * 0.000_000_238_418_58 * 4196.0 - 100.0;
    let z = ((z * 10_000.0).round_ties_even() / 10_000.0) as f32;
    Pos::new(long_to_coord(x), long_to_coord(y), z)
}

/// Inverse of [`decode_position`].
pub fn encode_position(p: Pos) -> [u8; 9] {
    let pack = |l: i64| {
        let pre = l >> 31;
        (pre ^ l.wrapping_add(pre).wrapping_add(i64::from(pre < 0))) >> 3
    };
    let rx = pack(coord_to_long(p.x));
    let ry = pack(coord_to_long(p.y));
    let rz = (f64::from((p.z + 100.0) / 4196.0 * 4_194_304.0) + 0.5).floor() as i64;
    let sign = (u8::from(p.y < 0.0) + 2 * u8::from(p.x < 0.0)) << 6;
    [
        (rx >> 32) as u8,
        (rx >> 40) as u8,
        (rx >> 48) as u8,
        (ry >> 32) as u8,
        (ry >> 40) as u8,
        (ry >> 48) as u8,
        rz as u8,
        (rz >> 8) as u8,
        (((rz >> 16) & 0x3F) as u8).wrapping_add(sign),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primitives_roundtrip() {
        let mut w = Writer::new();
        w.u8(1).u16(0x1234).u32(0xDEAD_BEEF).u64(7).bc(0x00AB_CDEF);
        w.string("hello").f32(1.5).bool(true).i16(-2);
        let bytes = w.finish();
        // u16 little endian, then the string length prefix.
        assert_eq!(&bytes[1..3], &[0x34, 0x12]);
        let mut r = Reader::new(&bytes);
        assert_eq!(r.u8().unwrap(), 1);
        assert_eq!(r.u16().unwrap(), 0x1234);
        assert_eq!(r.u32().unwrap(), 0xDEAD_BEEF);
        assert_eq!(r.u64().unwrap(), 7);
        assert_eq!(r.bc().unwrap(), 0x00AB_CDEF);
        assert_eq!(r.string().unwrap(), "hello");
        assert_eq!(r.f32().unwrap(), 1.5);
        assert!(r.bool().unwrap());
        assert_eq!(r.i16().unwrap(), -2);
        assert!(matches!(r.u8(), Err(WireError::Eof { .. })));
    }

    #[test]
    fn strings_trim_nuls_and_are_length_prefixed() {
        let bytes = [4, 0, b'a', b'b', 0, 0];
        assert_eq!(Reader::new(&bytes).string().unwrap(), "ab");
        let mut w = Writer::new();
        w.string("Ab");
        assert_eq!(w.finish(), [2, 0, b'A', b'b']);
    }

    #[test]
    fn bc_is_three_bytes() {
        let mut w = Writer::new();
        w.bc(0x0102_0304);
        assert_eq!(w.finish(), [0x04, 0x03, 0x02]);
    }

    #[test]
    fn position_roundtrip() {
        for p in [
            Pos::new(12345.5, 6789.25, 123.0),
            Pos::new(-100.5, -2000.0, -50.0),
            Pos::new(0.0, 0.0, -100.0),
            Pos::new(1.0, -1.0, 7.5),
        ] {
            let q = decode_position(&encode_position(p));
            assert!((q.x - p.x).abs() < 0.01, "{p:?} -> {q:?}");
            assert!((q.y - p.y).abs() < 0.01, "{p:?} -> {q:?}");
            assert!((q.z - p.z).abs() < 0.01, "{p:?} -> {q:?}");
        }
    }

    #[test]
    fn position_golden_bytes() {
        // Origin at the lowest height encodes to all zeros.
        assert_eq!(encode_position(Pos::new(0.0, 0.0, -100.0)), [0; 9]);
        // x = 1 m is 512 in the 24-bit fixed point (1/512 m), y = 2 m is 1024.
        let b = encode_position(Pos::new(1.0, 2.0, -100.0));
        assert_eq!(&b[..6], &[0, 2, 0, 0, 4, 0]);
        // Negative x sets 0x80 in byte 8, negative y 0x40.
        assert_eq!(encode_position(Pos::new(-1.0, 0.0, -100.0))[8] & 0xC0, 0x80);
        assert_eq!(encode_position(Pos::new(0.0, -1.0, -100.0))[8] & 0xC0, 0x40);
    }

    #[test]
    fn pisc_groups_use_two_bits_per_value() {
        // The server's `write_pisc` forms: 5 (u8), 0x1234 (u16), 0x123456 (bc), 0x12345678 (u32).
        let mut w = Writer::new();
        w.pisc(&[5, 0x1234, 0x12_3456, 0x1234_5678]);
        let bytes = w.finish();
        assert_eq!(
            bytes,
            [0b11_10_01_00, 5, 0x34, 0x12, 0x56, 0x34, 0x12, 0x78, 0x56, 0x34, 0x12]
        );
        let mut r = Reader::new(&bytes);
        assert_eq!(r.pisc(4).unwrap(), [5, 0x1234, 0x12_3456, 0x1234_5678]);
        assert!(r.is_empty());
        // Two values: the header still takes a whole byte; the rest reads as zero.
        let mut w = Writer::new();
        w.pisc(&[0, 300]);
        assert_eq!(w.as_bytes(), [0b0100, 0, 0x2C, 0x01]);
        assert_eq!(Reader::new(w.as_bytes()).pisc(2).unwrap(), [0, 300, 0, 0]);
        assert!(Reader::new(&[0]).pisc(5).is_err());
    }

    #[test]
    fn long_position_matches_fixed_point() {
        let mut w = Writer::new();
        w.long_position(Pos::new(10.5, -3.25, 8.0));
        let bytes = w.finish();
        assert_eq!(bytes.len(), 20);
        let p = Reader::new(&bytes).long_position().unwrap();
        assert_eq!(p, Pos::new(10.5, -3.25, 8.0));
    }
}
