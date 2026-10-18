//! Original394245f0 period decomposition, distinct from calendar dates and caller clamps.
use mlua::{Lua, Table};

pub(super) fn table(lua: &Lua, mut seconds: u64) -> mlua::Result<Table> {
    let out = lua.create_table()?;
    for (name, unit) in [
        ("year", 31_104_000),
        ("month", 2_592_000),
        ("day", 86_400),
        ("hour", 3600),
        ("minute", 60),
    ] {
        let quotient = seconds / unit;
        let field = quotient as i32;
        out.raw_set(name, f64::from(field as f32))?;
        // Native checks the signed low DWORD, but subtracts the complete unsigned quotient.
        if field > 0 {
            seconds = seconds.wrapping_sub(quotient.wrapping_mul(unit));
        }
    }
    out.raw_set("second", f64::from(seconds as i32 as f32))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matches_all_18_executed_original_x86_period_cases() -> mlua::Result<()> {
        let oracle: serde_json::Value = serde_json::from_str(include_str!("period_cases.json")).unwrap();
        let lua = Lua::new();
        assert_eq!(oracle["cases"].as_array().unwrap().len(), 18);
        for case in oracle["cases"].as_array().unwrap() {
            let seconds = case["seconds"].as_i64().unwrap();
            let result = table(&lua, seconds as u64)?;
            let mut count = 0;
            for pair in result.clone().pairs::<String, f64>() {
                pair?;
                count += 1;
            }
            assert_eq!(count, 6);
            for (name, expected) in case["period"].as_object().unwrap() {
                let expected = f64::from(expected.as_i64().unwrap() as i32 as f32);
                assert_eq!(result.raw_get::<f64>(name.as_str())?, expected, "seconds={seconds}, field={name}");
            }
        }
        Ok(())
    }
}
