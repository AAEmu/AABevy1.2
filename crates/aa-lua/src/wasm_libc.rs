//! Browser build only: `strtod`/`strtoul` for Lua's number parsing (`lua_str2number`, hex
//! literals), so Lua does not link WASI libc's float scanner (see `wasm/shim.c`).
use std::ffi::{c_char, c_ulong};

fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// Bytes of the NUL-terminated string at `s`.
unsafe fn bytes<'a>(s: *const c_char) -> &'a [u8] {
    unsafe { std::ffi::CStr::from_ptr(s) }.to_bytes()
}

/// Length of the longest prefix of `b` that C's `strtod` would read, and its value.
fn parse_double(b: &[u8]) -> Option<(usize, f64)> {
    let mut i = 0;
    while i < b.len() && is_space(b[i]) {
        i += 1;
    }
    let start = i;
    let neg = matches!(b.get(i), Some(b'-'));
    if matches!(b.get(i), Some(b'+' | b'-')) {
        i += 1;
    }
    let rest = &b[i..];
    let lower = |n: usize| rest.get(..n).map(|p| p.to_ascii_lowercase());
    let sign = if neg { -1.0 } else { 1.0 };
    if lower(8).as_deref() == Some(b"infinity") {
        return Some((i + 8, sign * f64::INFINITY));
    }
    if lower(3).as_deref() == Some(b"inf") {
        return Some((i + 3, sign * f64::INFINITY));
    }
    if lower(3).as_deref() == Some(b"nan") {
        return Some((i + 3, f64::NAN));
    }
    if matches!(lower(2).as_deref(), Some(b"0x")) && rest.get(2).is_some_and(u8::is_ascii_hexdigit) {
        let mut j = 2;
        let mut v = 0f64;
        while let Some(d) = rest.get(j).and_then(|c| (*c as char).to_digit(16)) {
            v = v * 16.0 + f64::from(d);
            j += 1;
        }
        return Some((i + j, sign * v));
    }
    let digits = |from: usize| rest[from..].iter().take_while(|c| c.is_ascii_digit()).count();
    let int = digits(0);
    let mut j = int;
    let mut frac = 0;
    if rest.get(j) == Some(&b'.') {
        frac = digits(j + 1);
        j += 1 + frac;
    }
    if int + frac == 0 {
        return None;
    }
    if matches!(rest.get(j), Some(b'e' | b'E')) {
        let k = j + 1 + usize::from(matches!(rest.get(j + 1), Some(b'+' | b'-')));
        let e = digits(k);
        if e > 0 {
            j = k + e;
        }
    }
    let text = std::str::from_utf8(&b[start..i + j]).ok()?;
    Some((i + j, text.parse::<f64>().ok()?))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn aa_lua_strtod(s: *const c_char, end: *mut *mut c_char) -> f64 {
    let (n, v) = parse_double(unsafe { bytes(s) }).unwrap_or((0, 0.0));
    if !end.is_null() {
        unsafe { *end = s.add(n).cast_mut() };
    }
    v
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn aa_lua_strtoul(s: *const c_char, end: *mut *mut c_char, base: i32) -> c_ulong {
    let b = unsafe { bytes(s) };
    let mut i = 0;
    while i < b.len() && is_space(b[i]) {
        i += 1;
    }
    let neg = b.get(i) == Some(&b'-');
    if matches!(b.get(i), Some(b'+' | b'-')) {
        i += 1;
    }
    let mut base = base as u32;
    let hex_prefix = matches!(b.get(i..i + 2), Some(b"0x" | b"0X")) && b.get(i + 2).is_some_and(u8::is_ascii_hexdigit);
    if (base == 0 || base == 16) && hex_prefix {
        i += 2;
        base = 16;
    } else if base == 0 {
        base = if b.get(i) == Some(&b'0') { 8 } else { 10 };
    }
    let first = i;
    let mut v: c_ulong = 0;
    let mut overflow = false;
    while let Some(d) = b.get(i).and_then(|c| (*c as char).to_digit(base)) {
        match v.checked_mul(base as c_ulong).and_then(|x| x.checked_add(d as c_ulong)) {
            Some(x) => v = x,
            None => overflow = true,
        }
        i += 1;
    }
    if !end.is_null() {
        let n = if i == first { 0 } else { i };
        unsafe { *end = s.add(n).cast_mut() };
    }
    if overflow {
        c_ulong::MAX
    } else if neg {
        v.wrapping_neg()
    } else {
        v
    }
}

#[cfg(test)]
mod tests {
    use super::parse_double;

    #[test]
    fn strtod_prefixes() {
        assert_eq!(parse_double(b" 12.5 "), Some((5, 12.5)));
        assert_eq!(parse_double(b"0x1F"), Some((4, 31.0)));
        assert_eq!(parse_double(b"1e"), Some((1, 1.0)));
        assert_eq!(parse_double(b"-.5e2x"), Some((5, -50.0)));
        assert_eq!(parse_double(b"5."), Some((2, 5.0)));
        assert_eq!(parse_double(b"abc"), None);
        assert_eq!(parse_double(b"."), None);
    }
}
