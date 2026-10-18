//! Expected values are captured native Xl/X2 output, independent of this Rust translation.
use super::*;
use serde_json::Value as Json;

fn cases() -> Json {
    serde_json::from_str(include_str!("native_cases.json")).unwrap()
}

fn runtime() -> mlua::Result<Lua> {
    let lua = Lua::new();
    lua.globals().set("X2Util", lua.create_table()?)?;
    install(&lua)?;
    Ok(lua)
}

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

fn assert_value(actual: &Value, expected: &Json) -> mlua::Result<()> {
    match expected["type"].as_str().unwrap() {
        "boolean" => {
            let Value::Boolean(actual) = actual else {
                panic!("expected boolean, got {actual:?}")
            };
            assert_eq!(*actual, expected["value"].as_bool().unwrap());
        }
        "number" => {
            let n = match actual {
                Value::Number(n) => *n,
                Value::Integer(n) => *n as f64,
                _ => panic!("expected number, got {actual:?}"),
            };
            assert_eq!(n, expected["value"].as_f64().unwrap());
        }
        "string" => {
            let Value::String(s) = actual else {
                panic!("expected string, got {actual:?}")
            };
            assert_eq!(s.as_bytes().as_ref(), unhex(expected["hex"].as_str().unwrap()));
        }
        "table" => {
            let Value::Table(t) = actual else {
                panic!("expected table, got {actual:?}")
            };
            let fields = expected["fields"].as_object().unwrap();
            assert_eq!(t.clone().pairs::<Value, Value>().count(), fields.len());
            for (key, expected) in fields {
                assert_value(&t.get::<Value>(key.as_str())?, expected)?;
            }
        }
        other => panic!("unsupported native fixture type {other}"),
    }
    Ok(())
}

#[test]
fn validation_matches_448_executed_native_cases_through_typed_lua_adapter() -> mlua::Result<()> {
    let lua = runtime()?;
    let util: Table = lua.globals().get("X2Util")?;
    let f: mlua::Function = util.get("IsValidName")?;
    let cases = cases();
    let rows = cases["valid"].as_array().unwrap();
    assert_eq!(rows.len(), 448);
    for row in rows {
        let text = row["text"].as_str().unwrap();
        let kind = row["kind"].as_i64().unwrap() as i32;
        let expected = row["valid"].as_bool().unwrap();
        assert_eq!(valid_bytes(text.as_bytes(), kind), expected, "kind{kind}: {text:?}");
        let result: MultiValue = f.call((util.clone(), text, kind))?;
        assert_eq!(result.len(), 1);
        assert!(
            matches!(result[0], Value::Boolean(value) if value == expected),
            "kind{kind}: {text:?}"
        );
    }
    Ok(())
}

#[test]
fn normalization_matches_58_executed_native_cases_with_punctuation_bom_and_case_maps() -> mlua::Result<()> {
    let lua = runtime()?;
    let util: Table = lua.globals().get("X2Util")?;
    let f: mlua::Function = util.get("CheckAvailableName")?;
    let cases = cases();
    let rows = cases["normal"].as_array().unwrap();
    assert_eq!(rows.len(), 58);
    for row in rows {
        let text = row["input"].as_str().unwrap();
        let mix = row["mix_case"].as_bool().unwrap();
        let expected = unhex(row["output_hex"].as_str().unwrap());
        let actual: MultiValue = f.call((util.clone(), text, mix))?;
        assert_eq!(actual.len(), 1);
        let Value::String(s) = &actual[0] else {
            panic!("expected one string")
        };
        assert_eq!(s.as_bytes().as_ref(), expected, "mix{mix}: {text:?}");
    }
    Ok(())
}

#[test]
fn name_policy_and_wrapper_arity_match_native_typed_conversion_goldens() -> mlua::Result<()> {
    let lua = runtime()?;
    let data = cases();
    for (kind, fields) in data["policies"].as_object().unwrap() {
        let actual: Table = lua.load(format!("return X2Util:GetNamePolicyInfo({kind})")).eval()?;
        assert_eq!(actual.clone().pairs::<Value, Value>().count(), 9);
        for (key, expected) in fields.as_object().unwrap() {
            let actual: Value = actual.get(key.as_str())?;
            if let Some(b) = expected.as_bool() {
                assert!(matches!(actual, Value::Boolean(value) if value == b));
            } else {
                assert_value(&actual, &serde_json::json!({"type":"number","value":expected}))?;
            }
        }
    }
    let rows = data["wrappers"].as_array().unwrap();
    assert_eq!(rows.len(), 31);
    for row in rows {
        let actual: MultiValue = lua.load(row["call"].as_str().unwrap()).eval()?;
        assert_eq!(actual.len(), row["arity"].as_u64().unwrap() as usize, "{}", row["name"]);
        for (actual, expected) in actual.iter().zip(row["values"].as_array().unwrap()) {
            assert_value(actual, expected)?;
        }
    }
    Ok(())
}

#[test]
fn case_helpers_match_native_domain_and_do_not_use_rust_unicode_case_folding() {
    // Four little-endian u32 native outputs per scalar: isAlpha,isUpper,toLower,toUpper.
    // The validator only needs isUpper within its accepted Latin range through0x17f.
    let raw = include_bytes!("native_helpers.bin");
    assert_eq!(raw.len(), 2048 * 4 * 4);
    for (c, words) in raw.chunks_exact(16).enumerate() {
        let values: Vec<_> = words.chunks_exact(4).map(|n| u32::from_le_bytes(n.try_into().unwrap())).collect();
        let c = c as u32;
        assert_eq!(u32::from(contains(c, ALPHA)), values[0], "alpha U+{c:04X}");
        if c <= 0x17f {
            assert_eq!(u32::from(contains(c, LATIN_UPPER)), values[1], "upper U+{c:04X}");
        }
        assert_eq!(mapped(c, TO_LOWER), values[2], "lower U+{c:04X}");
        assert_eq!(mapped(c, TO_UPPER), values[3], "upper mapping U+{c:04X}");
    }
    assert_eq!(normalize("ſAbC", false), "ſabc");
    assert_eq!(normalize("中AbC", false), "中AbC");
    assert_eq!(normalize("\u{800}AbC", false), "\u{800}AbC");
}

#[test]
fn malformed_utf8_refusal_preserves_c_string_prefix_and_never_substitutes_replacement_characters() -> mlua::Result<()> {
    let lua = runtime()?;
    let util: Table = lua.globals().get("X2Util")?;
    let validate: mlua::Function = util.get("IsValidName")?;
    let normalize: mlua::Function = util.get("CheckAvailableName")?;
    for bytes in [b"Ab\xff".as_slice(), b"\xc0\xafAb", b"\xed\xa0\x80Ab", b"Ab\xc2"] {
        let text = lua.create_string(bytes)?;
        assert!(!validate.call::<bool>((util.clone(), text.clone(), 0))?);
        assert!(normalize.call::<MultiValue>((util.clone(), text, false))?.is_empty());
    }
    let text = lua.create_string(b"aBc\0\xff")?;
    assert!(validate.call::<bool>((util.clone(), text.clone(), 2))?);
    let normalized: mlua::LuaString = normalize.call((util, text, false))?;
    assert_eq!(normalized.as_bytes().as_ref(), b"Abc");
    Ok(())
}
