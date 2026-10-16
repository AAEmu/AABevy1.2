//! A tiny bounds-checked little endian reader for the cell files.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadError;

pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Reader { data, pos: 0 }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    pub fn seek(&mut self, pos: usize) -> Result<(), ReadError> {
        if pos > self.data.len() {
            return Err(ReadError);
        }
        self.pos = pos;
        Ok(())
    }

    pub fn skip(&mut self, n: usize) -> Result<(), ReadError> {
        self.bytes(n).map(|_| ())
    }

    /// `BinaryReader.ReadBytes` semantics: stops at the end of the data without failing.
    pub fn skip_clamped(&mut self, n: usize) {
        self.pos = (self.pos + n).min(self.data.len());
    }

    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8], ReadError> {
        let end = self.pos.checked_add(n).ok_or(ReadError)?;
        let s = self.data.get(self.pos..end).ok_or(ReadError)?;
        self.pos = end;
        Ok(s)
    }

    pub fn u8(&mut self) -> Result<u8, ReadError> {
        Ok(self.bytes(1)?[0])
    }

    pub fn u16(&mut self) -> Result<u16, ReadError> {
        Ok(u16::from_le_bytes(self.bytes(2)?.try_into().unwrap()))
    }

    pub fn u32(&mut self) -> Result<u32, ReadError> {
        Ok(u32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }

    pub fn i32(&mut self) -> Result<i32, ReadError> {
        Ok(i32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }

    pub fn f32(&mut self) -> Result<f32, ReadError> {
        Ok(f32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }
}

/// `u32` at `at` (little endian); 0 when out of range.
pub fn u32_at(data: &[u8], at: usize) -> u32 {
    data.get(at..at + 4).map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap()))
}

pub fn f32_at(data: &[u8], at: usize) -> f32 {
    f32::from_bits(u32_at(data, at))
}

pub fn i32_at(data: &[u8], at: usize) -> i32 {
    u32_at(data, at) as i32
}

pub fn u16_at(data: &[u8], at: usize) -> u16 {
    data.get(at..at + 2).map_or(0, |b| u16::from_le_bytes(b.try_into().unwrap()))
}

/// A NUL terminated latin-1 string from a fixed size field.
pub fn cstr(data: &[u8]) -> String {
    let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
    data[..end].iter().map(|&b| b as char).collect()
}
