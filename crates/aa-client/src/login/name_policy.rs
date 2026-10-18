//! Original en_us name policies and typed X2Util adapters shared by create and HUD stages.
//! Well-formed UTF-8 follows the executed Xl/X2 source oracle. Malformed input is refused;
//! reproducing the native decoder's unchecked reads is outside this adapter's contract.
use mlua::{Lua, MultiValue, Table, Value};
mod case;
use case::{ALPHA, LATIN_UPPER, TO_LOWER, TO_UPPER};

// local min/max, English min/max, flags. Native policy6 aliases character policy0.
fn policy(kind: i32) -> Option<[usize; 5]> {
    Some(match kind {
        0 | 1 | 6 => [2, 26, 2, 26, 0],
        2 => [2, 32, 3, 32, 5],
        3 => [2, 26, 2, 26, 4],
        4 => [2, 10, 2, 10, 5],
        5 => [3, 32, 3, 32, 5],
        _ => return None,
    })
}

fn space(c: u32) -> bool {
    matches!(c, 9..=13 | 32)
}

fn contains(c: u32, ranges: &[(u32, u32)]) -> bool {
    ranges.iter().any(|&(a, b)| (a..=b).contains(&c))
}

fn mapped(c: u32, ranges: &[(u32, u32, u32, i32)]) -> u32 {
    ranges
        .iter()
        .find_map(|&(a, b, step, delta)| (c >= a && c <= b && (c - a) % step == 0).then(|| (c as i32 + delta) as u32))
        .unwrap_or(c)
}

fn cstr(bytes: &[u8]) -> &[u8] {
    bytes.split(|byte| *byte == 0).next().unwrap_or_default()
}

pub(crate) fn valid_bytes(bytes: &[u8], kind: i32) -> bool {
    // Native strings terminate before any invalid UTF-8 in an ignored suffix.
    std::str::from_utf8(cstr(bytes)).is_ok_and(|text| valid(text, kind))
}

fn valid(text: &str, kind: i32) -> bool {
    let Some(p) = policy(kind) else { return false };
    let text = text.split('\0').next().unwrap_or_default();
    let raw = text.as_bytes();
    if raw.first() == Some(&b'.') {
        return false;
    }
    let english = raw.first().is_some_and(u8::is_ascii_alphabetic);
    let (min, max) = if english { (p[2], p[3]) } else { (p[0], p[1]) };
    // XlStringLen counts a leading BOM, but the subsequent decoder strips one BOM.
    if !(min..=max).contains(&text.chars().count()) {
        return false;
    }
    let decoded = text.strip_prefix('\u{feff}').unwrap_or(text);
    let count = decoded.chars().count();
    if decoded.chars().next().is_some_and(|c| space(c as u32)) {
        return false;
    }
    // Original33009257 indexes ORIGINAL UTF-8 bytes with the DECODED scalar count.
    if count > 0 && space(raw[count - 1] as u32) {
        return false;
    }
    for (i, c) in decoded.chars().map(u32::from).enumerate() {
        if space(c) {
            if p[4] & 1 == 0 {
                return false;
            }
        } else {
            if !matches!(c, 0x20..=0x7f | 0xa0..=0x17f) {
                return false;
            }
            if i > 0 && p[4] & 4 == 0 && contains(c, LATIN_UPPER) {
                return false;
            }
        }
    }
    true
}

fn normalize(text: &str, mix_case: bool) -> String {
    let text = text.split('\0').next().unwrap_or_default();
    let mut codes: Vec<u32> = text
        .strip_prefix('\u{feff}')
        .unwrap_or(text)
        .chars()
        .take(32)
        .map(u32::from)
        .collect();
    if !mix_case && !codes.is_empty() {
        if space(codes[0]) {
            return String::new();
        }
        if contains(codes[0], ALPHA) {
            codes[0] = mapped(codes[0], TO_UPPER);
            for c in &mut codes[1..] {
                *c = mapped(*c, TO_LOWER);
            }
        }
    }
    codes
        .into_iter()
        .map(|c| char::from_u32(c).expect("native case maps contain valid scalars"))
        .collect()
}

fn integer(value: &Value) -> Option<i32> {
    let n = match value {
        Value::Integer(n) => *n as f32,
        Value::Number(n) => *n as f32,
        _ => return None,
    };
    Some(if !n.is_finite() || !(-2147483648.0..2147483648.0).contains(&n) {
        i32::MIN // Native CVTTSS2SI, including NaN/overflow; Rust saturates instead.
    } else {
        n as i32
    })
}

fn boolean(value: &Value) -> Option<bool> {
    match value {
        Value::Boolean(b) => Some(*b),
        Value::Integer(n) => Some(*n != 0),
        Value::Number(n) => Some(*n != 0.), // NaN is true in the original typed adapter.
        _ => None,
    }
}

pub(crate) fn install(lua: &Lua) -> mlua::Result<()> {
    let util: Table = lua.globals().get("X2Util")?;
    util.set(
        "GetNamePolicyInfo",
        lua.create_function(|lua, args: MultiValue| {
            let Some(p) = args.get(1).and_then(integer).and_then(policy) else {
                return Ok(MultiValue::new());
            };
            let out = lua.create_table()?;
            for (key, value) in [
                ("local_min", p[0]),
                ("local_max", p[1]),
                ("eng_min", p[2]),
                ("eng_max", p[3]),
                ("limit_space_cnt", 0),
            ] {
                out.set(key, value as f64)?;
            }
            for (key, bit) in [
                ("is_allow_space", 1),
                ("is_allow_mix_lang", 2),
                ("is_allow_mix_case", 4),
                ("is_allow_special_letter", 16),
            ] {
                out.set(key, p[4] & bit != 0)?;
            }
            Ok(MultiValue::from_iter([Value::Table(out)]))
        })?,
    )?;
    util.set(
        "IsValidName",
        lua.create_function(|_, args: MultiValue| {
            let (Some(Value::String(text)), Some(kind)) = (args.get(1), args.get(2).and_then(integer)) else {
                return Ok(MultiValue::new());
            };
            Ok(MultiValue::from_iter([Value::Boolean(valid_bytes(&text.as_bytes(), kind))]))
        })?,
    )?;
    util.set(
        "CheckAvailableName",
        lua.create_function(|lua, args: MultiValue| {
            let (Some(Value::String(text)), Some(mix_case)) = (args.get(1), args.get(2).and_then(boolean)) else {
                return Ok(MultiValue::new());
            };
            let bytes = text.as_bytes();
            let Ok(text) = std::str::from_utf8(cstr(&bytes)) else {
                return Ok(MultiValue::new());
            };
            Ok(MultiValue::from_iter([Value::String(
                lua.create_string(normalize(text, mix_case))?,
            )]))
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests;
