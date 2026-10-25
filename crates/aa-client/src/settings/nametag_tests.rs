//! The unchanged original affiliation callbacks against the real account option bridge.
use super::*;
use aa_lua::chunk::{Chunk, Format};

fn callback(lua: &Lua, parent: usize, child: usize) -> Result<mlua::Function, Box<dyn std::error::Error>> {
    let bytes = aa_pak::Pak::global().read("scriptsbin/x2ui/option/interface_option.alb")?;
    let original = aa_lua::chunk::parse(&bytes)?;
    let proto = original.main.protos[parent].protos[child].clone();
    assert_eq!(proto.num_upvalues, 0, "the original callback has no captured host context");
    let bytes = aa_lua::chunk::write(
        &Chunk {
            format: Format::ALB,
            main: proto,
        },
        Format::HOST,
    )?;
    Ok(lua.load(&bytes).into_function()?)
}

#[test]
fn original_affiliation_callbacks_apply_cancel_and_persist_to_a_fresh_runtime() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::temp_dir().join(format!("aa-client-nametag-{}.json", std::process::id()));
    let open = || -> Result<aa_ui::UiRuntime, Box<dyn std::error::Error>> {
        let mut rt = aa_ui::UiRuntime::new("en_us", 1280.0, 720.0)?;
        let bridge = std::rc::Rc::new(std::cell::RefCell::new(crate::hud::api::HudData::default()));
        crate::hud::api::install(&mut rt, &bridge)?;
        reopen_fixture(&rt, path.clone())?;
        install_options(&rt.lua)?;
        rt.load_base_libs();
        Ok(rt)
    };
    let rt = open()?;
    let init = callback(&rt.lua, 20, 0)?;
    let save = callback(&rt.lua, 20, 1)?;
    rt.lua
        .load(
            r#"
        assert(OPTION_ITEM_NAME_TAG_FACTION_SELECTION==52)
        X2Option:SetItemFloatValue(52,0)
        selected=1
        radio={Check=function(_,value,notify) selected=value;assert(not notify) end,GetChecked=function()return selected end}
    "#,
        )
        .exec()?;
    let radio: Table = rt.lua.globals().get("radio")?;
    init.call::<()>(radio.clone())?;
    assert_eq!(rt.lua.globals().get::<i32>("selected")?, 1);
    // The exact original Save instructions subtract one before calling X2NameTag.
    rt.lua.globals().set("selected", 2)?;
    save.call::<()>(radio.clone())?;
    assert_eq!(rt.lua.load("return X2Option:GetOptionItemValue(52)").eval::<i32>()?, 1);
    assert_eq!(
        rt.lua.load("return NET.option_get(52)").eval::<i32>()?,
        1,
        "the actual option store changes, not a separate fake table"
    );
    rt.lua.load("X2Option:Save(); ToggleOptionFrame(true)").exec()?;
    rt.lua.globals().set("selected", 1)?;
    save.call::<()>(radio)?;
    rt.lua.load("assert(X2Option:GetOptionItemValue(52)==0); optionFrame:Cancel(); ToggleOptionFrame(false); assert(X2Option:GetOptionItemValue(52)==1)").exec()?;
    drop(rt);
    let reopened = open()?;
    assert_eq!(reopened.lua.load("return X2Option:GetOptionItemValue(52)").eval::<i32>()?, 1);
    assert_eq!(Store::open(path.clone()).data["options"]["n:52"].as_f64(), Some(1.0));
    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
fn faction_setter_rejects_nonnumbers_and_uses_native_float32_integer_conversion() -> Result<(), Box<dyn std::error::Error>> {
    let mut rt = aa_ui::UiRuntime::new("en_us", 1280.0, 720.0)?;
    let bridge = std::rc::Rc::new(std::cell::RefCell::new(crate::hud::api::HudData::default()));
    crate::hud::api::install(&mut rt, &bridge)?;
    reopen_fixture(
        &rt,
        std::env::temp_dir().join(format!("aa-nametag-adapter-{}.json", std::process::id())),
    )?;
    install_options(&rt.lua)?;
    rt.lua
        .load(
            r#"
        X2NameTag:SetNameTagFactionSelection(1)
        for _,v in ipairs({false,true,'2',{},function()end}) do
            assert(select('#',X2NameTag:SetNameTagFactionSelection(v))==0)
            assert(X2Option:GetOptionItemValue(52)==1)
        end
        X2NameTag:SetNameTagFactionSelection()
        assert(X2Option:GetOptionItemValue(52)==1)
        for _,row in ipairs({{1.9,1},{-1.9,-1},{-0.9,0},{0.99999999,1},{1/0,-2147483648},{0/0,-2147483648},{2147483647,-2147483648}}) do
            X2NameTag:SetNameTagFactionSelection(row[1])
            assert(X2Option:GetOptionItemValue(52)==row[2])
        end
    "#,
        )
        .exec()?;
    Ok(())
}
