//! Lua 5.1 binary chunk reader/writer.
//!
//! The client's `.alb` files are plain `luac` 5.1 output built for a 32-bit host whose `lua_Number`
//! is a 4-byte `float`: header `1b 4c 75 61 51 00 01 04 04 04 04 00` (little endian, int 4,
//! size_t 4, Instruction 4, lua_Number 4, not integral). A 64-bit host Lua (size_t 8, double)
//! refuses that header, so [`convert`] parses the chunk and writes it back in the host layout.
//! The instruction stream uses the stock 5.1 opcode numbering and is copied as is.

use anyhow::{Context, Result, bail, ensure};

/// `ESC "Lua"`.
pub const SIGNATURE: &[u8; 4] = b"\x1bLua";

/// The sizes recorded in a 5.1 chunk header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    pub little_endian: bool,
    pub size_int: u8,
    pub size_size_t: u8,
    pub size_instruction: u8,
    pub size_number: u8,
    pub integral: bool,
}

impl Format {
    /// The `.alb` layout (32-bit client with `float` numbers).
    pub const ALB: Format = Format {
        little_endian: true,
        size_int: 4,
        size_size_t: 4,
        size_instruction: 4,
        size_number: 4,
        integral: false,
    };

    /// The layout the linked Lua 5.1 expects (`double` numbers, native `size_t`).
    pub const HOST: Format = Format {
        little_endian: cfg!(target_endian = "little"),
        size_int: 4,
        size_size_t: std::mem::size_of::<usize>() as u8,
        size_instruction: 4,
        size_number: 8,
        integral: false,
    };
}

/// A constant in a function's constant table.
#[derive(Clone, Debug, PartialEq)]
pub enum Constant {
    Nil,
    Bool(bool),
    Number(f64),
    /// Raw bytes (the scripts are UTF-8, but Lua strings are byte strings).
    String(Vec<u8>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct LocVar {
    pub name: Vec<u8>,
    pub start_pc: i32,
    pub end_pc: i32,
}

/// One function prototype (`Proto` in lobject.h).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Proto {
    /// `None` = inherited from the parent (or `=?` for the main function).
    pub source: Option<Vec<u8>>,
    pub line_defined: i32,
    pub last_line_defined: i32,
    pub num_upvalues: u8,
    pub num_params: u8,
    pub is_vararg: u8,
    pub max_stack_size: u8,
    pub code: Vec<u32>,
    pub constants: Vec<Constant>,
    pub protos: Vec<Proto>,
    pub line_info: Vec<i32>,
    pub loc_vars: Vec<LocVar>,
    pub upvalue_names: Vec<Vec<u8>>,
}

/// A parsed chunk: its header format and main function.
#[derive(Clone, Debug, PartialEq)]
pub struct Chunk {
    pub format: Format,
    pub main: Proto,
}

impl Proto {
    /// This function and all nested ones, depth first.
    pub fn walk(&self, f: &mut impl FnMut(&Proto)) {
        f(self);
        for p in &self.protos {
            p.walk(f);
        }
    }
}

/// Parse a 5.1 binary chunk in any of the supported layouts (int 4, size_t 4/8, number 4/8).
pub fn parse(data: &[u8]) -> Result<Chunk> {
    ensure!(data.len() >= 12 && &data[0..4] == SIGNATURE, "not a Lua binary chunk");
    ensure!(data[4] == 0x51, "not Lua 5.1 bytecode (version byte {:#04x})", data[4]);
    ensure!(data[5] == 0, "unknown bytecode format {}", data[5]);
    let format = Format {
        little_endian: data[6] == 1,
        size_int: data[7],
        size_size_t: data[8],
        size_instruction: data[9],
        size_number: data[10],
        integral: data[11] != 0,
    };
    ensure!(format.little_endian, "big-endian chunks are not supported");
    ensure!(format.size_int == 4, "sizeof(int) = {} unsupported", format.size_int);
    ensure!(matches!(format.size_size_t, 4 | 8), "sizeof(size_t) = {} unsupported", format.size_size_t);
    ensure!(format.size_instruction == 4, "sizeof(Instruction) = {} unsupported", format.size_instruction);
    ensure!(matches!(format.size_number, 4 | 8), "sizeof(lua_Number) = {} unsupported", format.size_number);
    let mut r = Reader { data, pos: 12, format };
    let main = r.function(0).context("reading main function")?;
    ensure!(r.pos == data.len(), "{} trailing bytes after chunk", data.len() - r.pos);
    Ok(Chunk { format, main })
}

/// Serialise a chunk in `format`.
pub fn write(chunk: &Chunk, format: Format) -> Result<Vec<u8>> {
    ensure!(format.little_endian && format.size_int == 4 && format.size_instruction == 4, "unsupported target format");
    let mut w = Writer { out: Vec::with_capacity(4096), format };
    w.out.extend_from_slice(SIGNATURE);
    w.out.extend_from_slice(&[
        0x51,
        0,
        1,
        format.size_int,
        format.size_size_t,
        format.size_instruction,
        format.size_number,
        format.integral as u8,
    ]);
    w.function(&chunk.main)?;
    Ok(w.out)
}

/// Convert `.alb` bytes to a chunk the host Lua can `load`. When the main function has no source
/// name (stripped chunks), `source` (e.g. `@scriptsbin/x2ui/hud/clock.lua`) is recorded instead so
/// error messages and tracebacks name the file.
pub fn convert(data: &[u8], source: Option<&str>) -> Result<Vec<u8>> {
    let mut chunk = parse(data)?;
    if let Some(src) = source
        && chunk.main.source.as_ref().is_none_or(|s| s.is_empty())
    {
        chunk.main.source = Some(src.as_bytes().to_vec());
    }
    write(&chunk, Format::HOST)
}

/// `f32` constant -> `f64` via its shortest decimal form, so a literal written as `0.1` in the
/// source becomes the double `0.1` rather than `0.100000001490116`.
pub fn widen_f32(v: f32) -> f64 {
    if !v.is_finite() || v == v.trunc() {
        return v as f64;
    }
    format!("{v}").parse::<f64>().unwrap_or(v as f64)
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    format: Format,
}

impl Reader<'_> {
    fn bytes(&mut self, n: usize) -> Result<&[u8]> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.data.len());
        let Some(end) = end else {
            bail!("unexpected end of chunk at {:#x} (need {n} bytes)", self.pos);
        };
        let b = &self.data[self.pos..end];
        self.pos = end;
        Ok(b)
    }
    fn byte(&mut self) -> Result<u8> {
        Ok(self.bytes(1)?[0])
    }
    fn int(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }
    fn count(&mut self) -> Result<usize> {
        let n = self.int()?;
        ensure!(n >= 0 && (n as usize) <= self.data.len(), "bad count {n} at {:#x}", self.pos - 4);
        Ok(n as usize)
    }
    fn size_t(&mut self) -> Result<u64> {
        Ok(match self.format.size_size_t {
            4 => u32::from_le_bytes(self.bytes(4)?.try_into().unwrap()) as u64,
            _ => u64::from_le_bytes(self.bytes(8)?.try_into().unwrap()),
        })
    }
    fn number(&mut self) -> Result<f64> {
        Ok(match self.format.size_number {
            4 => {
                let b = self.bytes(4)?.try_into().unwrap();
                if self.format.integral { i32::from_le_bytes(b) as f64 } else { widen_f32(f32::from_le_bytes(b)) }
            }
            _ => {
                let b = self.bytes(8)?.try_into().unwrap();
                if self.format.integral { i64::from_le_bytes(b) as f64 } else { f64::from_le_bytes(b) }
            }
        })
    }
    /// `size_t` length including the trailing NUL; 0 = NULL string.
    fn string(&mut self) -> Result<Option<Vec<u8>>> {
        let n = self.size_t()? as usize;
        if n == 0 {
            return Ok(None);
        }
        let b = self.bytes(n)?;
        ensure!(b[n - 1] == 0, "string at {:#x} is not NUL terminated", self.pos - n);
        Ok(Some(b[..n - 1].to_vec()))
    }
    fn function(&mut self, depth: usize) -> Result<Proto> {
        ensure!(depth < 200, "functions nested too deeply");
        let source = self.string()?;
        let line_defined = self.int()?;
        let last_line_defined = self.int()?;
        let num_upvalues = self.byte()?;
        let num_params = self.byte()?;
        let is_vararg = self.byte()?;
        let max_stack_size = self.byte()?;
        let n = self.count()?;
        let code = self.bytes(n * 4)?.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect();
        let n = self.count()?;
        let mut constants = Vec::with_capacity(n);
        for _ in 0..n {
            constants.push(match self.byte()? {
                0 => Constant::Nil,
                1 => Constant::Bool(self.byte()? != 0),
                3 => Constant::Number(self.number()?),
                4 => Constant::String(self.string()?.unwrap_or_default()),
                t => bail!("bad constant type {t} at {:#x}", self.pos - 1),
            });
        }
        let n = self.count()?;
        let mut protos = Vec::with_capacity(n);
        for i in 0..n {
            protos.push(self.function(depth + 1).with_context(|| format!("nested function {i}"))?);
        }
        let n = self.count()?;
        let line_info = (0..n).map(|_| self.int()).collect::<Result<_>>()?;
        let n = self.count()?;
        let mut loc_vars = Vec::with_capacity(n);
        for _ in 0..n {
            let name = self.string()?.unwrap_or_default();
            loc_vars.push(LocVar { name, start_pc: self.int()?, end_pc: self.int()? });
        }
        let n = self.count()?;
        let upvalue_names = (0..n).map(|_| self.string().map(Option::unwrap_or_default)).collect::<Result<_>>()?;
        Ok(Proto {
            source,
            line_defined,
            last_line_defined,
            num_upvalues,
            num_params,
            is_vararg,
            max_stack_size,
            code,
            constants,
            protos,
            line_info,
            loc_vars,
            upvalue_names,
        })
    }
}

struct Writer {
    out: Vec<u8>,
    format: Format,
}

impl Writer {
    fn int(&mut self, v: i32) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }
    fn count(&mut self, n: usize) -> Result<()> {
        self.int(i32::try_from(n).context("count overflows int")?);
        Ok(())
    }
    fn size_t(&mut self, v: u64) {
        match self.format.size_size_t {
            4 => self.out.extend_from_slice(&(v as u32).to_le_bytes()),
            _ => self.out.extend_from_slice(&v.to_le_bytes()),
        }
    }
    fn number(&mut self, v: f64) {
        match (self.format.size_number, self.format.integral) {
            (4, false) => self.out.extend_from_slice(&(v as f32).to_le_bytes()),
            (4, true) => self.out.extend_from_slice(&(v as i32).to_le_bytes()),
            (_, false) => self.out.extend_from_slice(&v.to_le_bytes()),
            (_, true) => self.out.extend_from_slice(&(v as i64).to_le_bytes()),
        }
    }
    fn string(&mut self, s: Option<&[u8]>) {
        match s {
            None => self.size_t(0),
            Some(s) => {
                self.size_t(s.len() as u64 + 1);
                self.out.extend_from_slice(s);
                self.out.push(0);
            }
        }
    }
    fn function(&mut self, p: &Proto) -> Result<()> {
        self.string(p.source.as_deref());
        self.int(p.line_defined);
        self.int(p.last_line_defined);
        self.out.extend_from_slice(&[p.num_upvalues, p.num_params, p.is_vararg, p.max_stack_size]);
        self.count(p.code.len())?;
        for i in &p.code {
            self.out.extend_from_slice(&i.to_le_bytes());
        }
        self.count(p.constants.len())?;
        for k in &p.constants {
            match k {
                Constant::Nil => self.out.push(0),
                Constant::Bool(b) => self.out.extend_from_slice(&[1, *b as u8]),
                Constant::Number(n) => {
                    self.out.push(3);
                    self.number(*n);
                }
                Constant::String(s) => {
                    self.out.push(4);
                    self.string(Some(s));
                }
            }
        }
        self.count(p.protos.len())?;
        for q in &p.protos {
            self.function(q)?;
        }
        self.count(p.line_info.len())?;
        for &l in &p.line_info {
            self.int(l);
        }
        self.count(p.loc_vars.len())?;
        for v in &p.loc_vars {
            self.string(Some(&v.name));
            self.int(v.start_pc);
            self.int(v.end_pc);
        }
        self.count(p.upvalue_names.len())?;
        for u in &p.upvalue_names {
            self.string(Some(u));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// Disassembly (stock Lua 5.1 opcode set, lopcodes.h)

/// Stock Lua 5.1 opcode names, by number.
pub const OPCODES: [&str; 38] = [
    "MOVE", "LOADK", "LOADBOOL", "LOADNIL", "GETUPVAL", "GETGLOBAL", "GETTABLE", "SETGLOBAL", "SETUPVAL",
    "SETTABLE", "NEWTABLE", "SELF", "ADD", "SUB", "MUL", "DIV", "MOD", "POW", "UNM", "NOT", "LEN", "CONCAT",
    "JMP", "EQ", "LT", "LE", "TEST", "TESTSET", "CALL", "TAILCALL", "RETURN", "FORLOOP", "FORPREP",
    "TFORLOOP", "SETLIST", "CLOSE", "CLOSURE", "VARARG",
];

/// Decoded instruction fields (`iABC` / `iABx` / `iAsBx`).
#[derive(Clone, Copy, Debug)]
pub struct Insn(pub u32);

impl Insn {
    pub fn op(self) -> u8 {
        (self.0 & 0x3f) as u8
    }
    pub fn a(self) -> u32 {
        (self.0 >> 6) & 0xff
    }
    pub fn c(self) -> u32 {
        (self.0 >> 14) & 0x1ff
    }
    pub fn b(self) -> u32 {
        (self.0 >> 23) & 0x1ff
    }
    pub fn bx(self) -> u32 {
        self.0 >> 14
    }
    pub fn sbx(self) -> i32 {
        self.bx() as i32 - 131071
    }
}

fn fmt_const(k: &Constant) -> String {
    match k {
        Constant::Nil => "nil".into(),
        Constant::Bool(b) => b.to_string(),
        Constant::Number(n) => n.to_string(),
        Constant::String(s) => format!("{:?}", String::from_utf8_lossy(s)),
    }
}

/// Human-readable listing of a function and its children (like `luac -l -l`).
pub fn disassemble(p: &Proto) -> String {
    let mut out = String::new();
    dis(p, "main", &mut out);
    out
}

fn dis(p: &Proto, name: &str, out: &mut String) {
    use std::fmt::Write;
    let _ = writeln!(
        out,
        "function {name} (lines {}-{}; {} params{}, {} upvalues, {} slots, {} constants, {} functions)",
        p.line_defined,
        p.last_line_defined,
        p.num_params,
        if p.is_vararg != 0 { "+" } else { "" },
        p.num_upvalues,
        p.max_stack_size,
        p.constants.len(),
        p.protos.len()
    );
    let rk = |x: u32| -> String {
        if x & 0x100 != 0 {
            p.constants.get((x & 0xff) as usize).map(fmt_const).unwrap_or_else(|| format!("K?{x}"))
        } else {
            format!("R{x}")
        }
    };
    for (pc, &raw) in p.code.iter().enumerate() {
        let i = Insn(raw);
        let op = OPCODES.get(i.op() as usize).copied().unwrap_or("???");
        let line = p.line_info.get(pc).map(|l| format!("[{l}]")).unwrap_or_default();
        let (a, b, c, bx, sbx) = (i.a(), i.b(), i.c(), i.bx(), i.sbx());
        let k = |n: u32| p.constants.get(n as usize).map(fmt_const).unwrap_or_else(|| format!("K?{n}"));
        let args = match i.op() {
            1 | 5 | 7 => format!("R{a} {}", k(bx)),
            36 => format!("R{a} F{bx}"),
            22 | 31 | 32 => format!("{sbx} -> {}", pc as i32 + 1 + sbx),
            6 | 11 => format!("R{a} R{b} {}", rk(c)),
            9 | 12..=17 | 23..=25 => format!("R{a} {} {}", rk(b), rk(c)),
            4 | 8 => format!("R{a} U{b}"),
            _ => format!("R{a} {b} {c}"),
        };
        let _ = writeln!(out, "  {:4} {line:7} {op:9} {args}", pc + 1);
    }
    for (i, q) in p.protos.iter().enumerate() {
        dis(q, &format!("{name}.F{i}"), out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widen() {
        assert_eq!(widen_f32(0.1), 0.1);
        assert_eq!(widen_f32(0.7), 0.7);
        assert_eq!(widen_f32(1024.0), 1024.0);
        assert_eq!(widen_f32(-3.5), -3.5);
    }

    #[test]
    fn roundtrip_header() {
        let chunk = Chunk { format: Format::ALB, main: Proto { is_vararg: 2, max_stack_size: 2, code: vec![0x0080001e], ..Default::default() } };
        let bytes = write(&chunk, Format::ALB).unwrap();
        assert_eq!(&bytes[..12], b"\x1bLuaQ\x00\x01\x04\x04\x04\x04\x00");
        assert_eq!(parse(&bytes).unwrap(), chunk);
    }
}
