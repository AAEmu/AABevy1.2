//! Lua values the client hands to the UI scripts (own file so the examples can include it too).

/// A Lua number argument. Lua 5.1 numbers are doubles; `mlua::Integer` is 32 bits on wasm32, so
/// there ids go through `Value::Number` (exact below 2^53).
pub(crate) fn lua_int(n: i64) -> mlua::Value {
    #[cfg(not(target_arch = "wasm32"))]
    return mlua::Value::Integer(n);
    #[cfg(target_arch = "wasm32")]
    mlua::Value::Number(n as f64)
}
