//! Native live-object strings at the Lua boundary. Private HUD snapshots keep
//! their existing decimal strings; persistent character/item IDs are unrelated.
use mlua::{Lua, Table, Value};

pub(super) fn format(id: u32) -> String {
    format!("{id:x}")
}

/// XlAtoUI323301bcf0 /3301ac60, radix16: ASCII leading whitespace,
/// optional0x, prefix conversion and wrapping32-bit arithmetic. The signed-byte
/// lookup's read-only negative indices are zero, including UTF-8 bytes>=0x80.
fn parse(bytes: &[u8]) -> u32 {
    let mut input = bytes;
    while matches!(input.first(), Some(b' ' | b'\t' | b'\r' | b'\n' | b'\x0b' | b'\x0c')) {
        input = &input[1..];
    }
    if input.starts_with(b"0x") || input.starts_with(b"0X") {
        input = &input[2..];
    }
    let mut id = 0_u32;
    for &byte in input {
        let digit = match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            b'A'..=b'F' => byte - b'A' + 10,
            0x80..=0xff => 0,
            _ => break,
        };
        id = id.wrapping_mul(16).wrapping_add(u32::from(digit));
    }
    id
}

/// The actual ById bindings require string5; numbers/bools do not reach parsing.
pub(super) fn value(value: &Value) -> Option<u32> {
    let Value::String(text) = value else { return None };
    let id = parse(&text.as_bytes());
    (id != 0).then_some(id)
}

pub(super) fn string(value: &str) -> Option<u32> {
    let id = parse(value.as_bytes());
    (id != 0).then_some(id)
}

pub(super) fn from_private(value: &str) -> Option<String> {
    value.parse::<u32>().ok().filter(|id| *id != 0).map(format)
}

/// These two used host-only functions explicitly cross the private decimal
/// snapshot domain. They are not additional X2 APIs or radix guessing.
pub(super) fn install(lua: &Lua, hud: &Table) -> mlua::Result<()> {
    hud.raw_set(
        "script_unit_id",
        lua.create_function(|_, value: Value| {
            let Value::String(text) = value else { return Ok(None) };
            Ok(text.to_str().ok().and_then(|s| from_private(&s)))
        })?,
    )?;
    hud.raw_set(
        "private_unit_id",
        lua.create_function(|_, input: Value| Ok(value(&input).map(|id| id.to_string())))?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_prefix_and_wrapping_controls_are_not_decimal_inference() {
        for (input, expected) in [
            ("64", 100),
            ("100", 256),
            ("10", 16),
            ("0X64", 100),
            (" \t\r\n64", 100),
            ("64z", 100),
            ("+64", 0),
            ("-64", 0),
            ("ffffffff", u32::MAX),
            ("100000000", 0),
            ("100000064", 100),
            ("fffffffff", u32::MAX),
            ("0x64suffix", 100),
            ("64\0ff", 100),
            ("漢64", 100),
            ("\u{200b}64", 100),
        ] {
            assert_eq!(parse(input.as_bytes()), expected, "{input:?}");
        }
        assert_eq!(format(100), "64");
        assert_eq!(format(256), "100");
    }

    #[test]
    fn public_byid_type_checks_precede_private_adapter_lookup() -> mlua::Result<()> {
        let lua = Lua::new();
        let hud = lua.create_table()?;
        install(&lua, &hud)?;
        lua.globals().set("HUD", hud)?;
        lua.load(
            r#"
          assert(HUD.private_unit_id('100')=='256')
          assert(HUD.private_unit_id('64')=='100')
          assert(HUD.private_unit_id(100)==nil and HUD.private_unit_id(false)==nil)
          assert(HUD.private_unit_id('player')==nil)
          assert(HUD.script_unit_id('100')=='64' and HUD.script_unit_id('256')=='100')
          assert(HUD.script_unit_id('0')==nil)
        "#,
        )
        .exec()
    }

    #[test]
    fn pure_digit_public_getters_roundtrip_without_private_decimal_collisions() -> Result<(), Box<dyn std::error::Error>> {
        use super::super::api::{HudData, Shared};
        use aa_ui::UiRuntime;
        let mut rt = UiRuntime::new("en_us", 1280.0, 720.0)?;
        let shared: Shared = std::rc::Rc::new(std::cell::RefCell::new(HudData::default()));
        super::super::api::install(&mut rt, &shared)?;
        rt.lua
            .load(
                r#"
          HUD.state.unit_id='256'; HUD.state.name='Own';
          HUD.target={unit_id='100',name='Other',detail='npc',grade='elite'}
          assert(X2Unit:GetUnitId('player')=='100')
          assert(X2Unit:GetUnitId('target')=='64')
          assert(X2Unit:GetUnitId('100')==nil and X2Unit:GetUnitId(256)==nil)
          assert(X2Unit:GetTargetUnitId('player')=='64')
          assert(X2Unit:GetUnitNameById(X2Unit:GetUnitId('player'))=='Own')
          assert(X2Unit:GetUnitNameById(X2Unit:GetTargetUnitId('player'))=='Other')
          assert(X2Unit:GetUnitNameById(100)==nil)
          assert(X2Unit:GetUnitGradeById('64')=='elite' and X2Unit:GetUnitGradeById('100')==nil)
        "#,
            )
            .exec()?;
        Ok(())
    }
}
