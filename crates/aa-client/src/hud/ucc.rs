//! The `X2Ucc` API `ucc.alb` needs to open its window.
//!
//! `DoodadFuncStampMaker` raises `OPEN_EMBLEM_UPLOAD_UI(objId)`; the original handler builds the
//! pattern/sample lists from `X2Ucc.GetPatternTypeNumbers`/`GetPatternKind`/`GetPatternIconPath`
//! and the user fg list from `GetFgUserCount`/`GetFgUserPath` before it shows the window.
//! This client has no UCC pattern catalog, so the lists are empty and the window opens with
//! nothing to pick. Uploading (`UploadColors`, `UploadEmblem`) is not wired: record what is
//! still missing in `docs/aa-client-doodads.md`.

use mlua::{Lua, MultiValue, Table, Value};

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let ucc: Table = lua.globals().get("X2Ucc")?;
    ucc.set(
        "GetPatternTypeNumbers",
        lua.create_function(|lua, _: MultiValue| lua.create_table())?,
    )?;
    ucc.set(
        "GetPatternKind",
        lua.create_function(|_, _: MultiValue| Ok(Value::Nil))?,
    )?;
    ucc.set(
        "GetPatternIconPath",
        lua.create_function(|_, _: MultiValue| Ok(String::new()))?,
    )?;
    ucc.set(
        "GetPatternPath",
        lua.create_function(|_, _: MultiValue| Ok(String::new()))?,
    )?;
    ucc.set(
        "GetFgUserCount",
        lua.create_function(|_, _: MultiValue| Ok(0i64))?,
    )?;
    ucc.set(
        "GetFgUserPath",
        lua.create_function(|_, _: MultiValue| Ok(String::new()))?,
    )?;
    ucc.set(
        "GetUccUserDirectoryPath",
        lua.create_function(|_, _: MultiValue| Ok(String::new()))?,
    )?;
    ucc.set(
        "GetMakeUccConsumeInfo",
        lua.create_function(|_, _: MultiValue| Ok(MultiValue::new()))?,
    )?;
    ucc.set(
        "UploadColors",
        lua.create_function(|_, _: MultiValue| Ok(()))?,
    )?;
    ucc.set(
        "UploadEmblem",
        lua.create_function(|_, _: MultiValue| Ok(()))?,
    )?;
    Ok(())
}
