//! Native Window background picking and the original character-info/login consumers.
//! CPU UiRuntime only; no OS window, game endpoint or GPU.
#[path = "../examples/fake/mod.rs"]
#[allow(dead_code)]
mod fake;

use aa_ui::{
    UiRuntime, input, lua,
    tree::{Id, Layer},
};

fn runtime() -> UiRuntime {
    UiRuntime::new("en_us", 1280., 720.).unwrap()
}

fn run(ui: &UiRuntime, code: &str) {
    ui.exec("@window_pick", code).unwrap();
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
}

fn id(ui: &UiRuntime, expr: &str) -> Id {
    lua::value_id(&ui.lua, &ui.lua.load(expr).eval().unwrap()).unwrap()
}

fn click(ui: &UiRuntime, mouse: &mut input::MouseState, x: f32, y: f32) {
    input::update(&ui.lua, mouse, Some((x, y)), true, false, false, 0.);
    input::update(&ui.lua, mouse, Some((x, y)), false, true, false, 0.);
}

fn setup(ui: &UiRuntime) {
    run(
        ui,
        r#"
        under=UIParent:CreateWidget('button','under','UIParent')
        under:AddAnchor('TOPLEFT',UIParent,40,40);under:SetExtent(300,200);under:Show(true)
        low=UIParent:CreateWidget('window','low','UIParent')
        low:AddAnchor('TOPLEFT',UIParent,50,50);low:SetExtent(200,150);low:Show(true)
        high=UIParent:CreateWidget('window','high','UIParent')
        high:AddAnchor('TOPLEFT',UIParent,50,50);high:SetExtent(200,150);high:Show(true)
    "#,
    );
}

#[test]
fn fresh_backgrounds_take_hits_and_false_to_true_restores_overlapping_order() {
    let ui = runtime();
    setup(&ui);
    assert_eq!(input::pick(&ui.lua, 60., 60.), Some(id(&ui, "high")));
    run(&ui, "high:EnableWindowPick(false)");
    assert_eq!(input::pick(&ui.lua, 60., 60.), Some(id(&ui, "low")));
    run(&ui, "low:EnableWindowPick(false)");
    assert_eq!(input::pick(&ui.lua, 60., 60.), Some(id(&ui, "under")));
    run(&ui, "high:EnableWindowPick(true)");
    assert_eq!(input::pick(&ui.lua, 60., 60.), Some(id(&ui, "high")));
    run(&ui, "low:EnableWindowPick(true);low:Raise()");
    assert_eq!(input::pick(&ui.lua, 60., 60.), Some(id(&ui, "low")));
    run(&ui, "low:Show(false)");
    assert_eq!(input::pick(&ui.lua, 60., 60.), Some(id(&ui, "high")));
    run(&ui, "high:Clickable(false)");
    assert_eq!(input::pick(&ui.lua, 60., 60.), Some(id(&ui, "under")));
    run(&ui, "high:EnableWindowPick(true)");
    assert_eq!(input::pick(&ui.lua, 60., 60.), Some(id(&ui, "under")));
}

#[test]
fn layers_outrank_creation_and_false_windows_expose_buttons_below() {
    let ui = runtime();
    setup(&ui);
    run(
        &ui,
        "low:SetUILayer('hud');high:SetUILayer('normal');high:Raise()",
    );
    assert_eq!(
        lua::ui(&ui.lua).tree.widget(id(&ui, "low")).unwrap().layer,
        Layer::Hud
    );
    assert_eq!(input::pick(&ui.lua, 60., 60.), Some(id(&ui, "low")));
    run(&ui, "low:EnableWindowPick(false)");
    assert_eq!(input::pick(&ui.lua, 60., 60.), Some(id(&ui, "high")));
    run(&ui, "high:EnableWindowPick(false)");
    assert_eq!(input::pick(&ui.lua, 60., 60.), Some(id(&ui, "under")));
    run(
        &ui,
        r#"
        clicks=0;enters=0
        under:SetHandler('OnClick',function()clicks=clicks+1 end)
        under:SetHandler('OnEnter',function()enters=enters+1 end)
    "#,
    );
    click(&ui, &mut input::MouseState::default(), 60., 60.);
    assert_eq!(
        ui.lua
            .load("return clicks,enters")
            .eval::<(i64, i64)>()
            .unwrap(),
        (1, 1)
    );
}

#[test]
fn ancestor_false_cannot_be_overridden_and_nonwindow_descendants_keep_input() {
    let ui = runtime();
    setup(&ui);
    run(
        &ui,
        r#"
        low:Show(false);high:EnableWindowPick(false)
        bridge=high:CreateChildWidget('emptywidget','bridge',0,true)
        bridge:AddAnchor('TOPLEFT',high,0,0);bridge:SetExtent(200,150)
        child=bridge:CreateChildWidget('window','child',0,true)
        child:AddAnchor('TOPLEFT',high,0,0);child:SetExtent(200,150)
        child:EnableWindowPick(true)
        button=child:CreateChildWidget('button','button',0,true)
        button:AddAnchor('TOPLEFT',child,20,20);button:SetExtent(30,30)
        clicks=0;button:SetHandler('OnClick',function()clicks=clicks+1 end)
        edit=child:CreateChildWidget('editbox','edit',0,true)
        edit:AddAnchor('TOPLEFT',child,80,20);edit:SetExtent(80,25)
    "#,
    );
    assert_eq!(input::pick(&ui.lua, 60., 60.), Some(id(&ui, "under")));
    assert_eq!(input::pick(&ui.lua, 75., 75.), Some(id(&ui, "button")));
    let mut mouse = input::MouseState::default();
    click(&ui, &mut mouse, 75., 75.);
    assert_eq!(ui.lua.load("return clicks").eval::<i64>().unwrap(), 1);
    click(&ui, &mut mouse, 135., 75.);
    assert_eq!(ui.focus(), Some(id(&ui, "edit")));
    assert!(ui.type_text("native input"));
    assert_eq!(
        ui.lua
            .load("return edit:GetText()")
            .eval::<String>()
            .unwrap(),
        "native input"
    );
    run(&ui, "high:EnableWindowPick(true)");
    assert_eq!(input::pick(&ui.lua, 60., 60.), Some(id(&ui, "child")));
    run(&ui, "child:EnableWindowPick(false)");
    assert_eq!(input::pick(&ui.lua, 60., 60.), Some(id(&ui, "high")));
    assert_eq!(input::pick(&ui.lua, 75., 75.), Some(id(&ui, "button")));
}

#[test]
fn hidden_clipped_outside_and_retired_subtrees_keep_their_existing_gates() {
    let ui = runtime();
    setup(&ui);
    run(
        &ui,
        r#"
        low:Show(false);high:EnableWindowPick(false)
        child=high:CreateChildWidget('window','child',0,true)
        child:AddAnchor('TOPLEFT',high,200,0);child:SetExtent(100,100)
        button=child:CreateChildWidget('button','button',0,true)
        button:AddAnchor('TOPLEFT',child,0,0);button:SetExtent(30,30)
    "#,
    );
    assert_eq!(input::pick(&ui.lua, 255., 55.), Some(id(&ui, "button")));
    run(&ui, "high:EnableScroll(true)");
    assert_ne!(input::pick(&ui.lua, 255., 55.), Some(id(&ui, "button")));
    run(&ui, "high:EnableScroll(false);child:Show(false)");
    assert_eq!(input::pick(&ui.lua, 255., 55.), Some(id(&ui, "under")));
    run(&ui, "child:Show(true);high:Show(false)");
    assert_eq!(input::pick(&ui.lua, 255., 55.), Some(id(&ui, "under")));
    run(
        &ui,
        "high:Show(true);child:EnableHidingIsRemove(true);child:Show(false)",
    );
    lua::tick(&ui.lua, 0.);
    run(&ui, "child:EnableWindowPick(true)");
    assert!(!lua::ui(&ui.lua).tree.get(id(&ui, "child")).alive);
    assert_eq!(input::pick(&ui.lua, 255., 55.), Some(id(&ui, "under")));
}

#[test]
fn window_boolean_method_is_separate_from_enablepick_and_grid_stub() {
    let ui = runtime();
    setup(&ui);
    run(
        &ui,
        r#"
        high:EnableWindowPick(false)
        for _,v in ipairs({0,1,'true','false',{}})do
          assert(select('#',high:EnableWindowPick(v))==0)
        end
        assert(select('#',high:EnableWindowPick())==0)
        assert(select('#',high:EnableWindowPick(nil))==0)
    "#,
    );
    assert_eq!(input::pick(&ui.lua, 60., 60.), Some(id(&ui, "low")));
    run(
        &ui,
        "high:EnableWindowPick(true,'ignored');high:EnablePick(false)",
    );
    assert_eq!(input::pick(&ui.lua, 60., 60.), Some(id(&ui, "low")));
    run(&ui, "high:EnableWindowPick(false);high:EnablePick(true)");
    assert_eq!(input::pick(&ui.lua, 60., 60.), Some(id(&ui, "low")));
    run(&ui, "high:EnableWindowPick(true)");
    assert_eq!(input::pick(&ui.lua, 60., 60.), Some(id(&ui, "high")));
    run(
        &ui,
        r#"
        grid=UIParent:CreateWidget('grid','heldGrid','UIParent')
        grid:EnableWindowPick(false)
    "#,
    );
    assert!(
        ui.stub_calls()
            .iter()
            .any(|(n, c)| n == "grid:EnableWindowPick" && *c == 1)
    );
    assert!(
        !ui.stub_calls()
            .iter()
            .any(|(n, _)| n == "window:EnableWindowPick")
    );
}

// Select unchanged original instructions, converting only Lua serialization widths.
fn original(ui: &UiRuntime, file: &str, marker: &str) -> mlua::Function {
    use aa_lua::chunk::{self, Chunk, Constant, Format};
    let bytes = aa_pak::Pak::global().read(file).unwrap();
    let chunk = chunk::parse(&bytes).unwrap();
    let mut found = Vec::new();
    chunk.main.walk(&mut |p| {
        if marker.split('+').all(|m| {
            p.constants
                .iter()
                .any(|c| matches!(c,Constant::String(s)if s==m.as_bytes()))
        }) {
            found.push(p.clone());
        }
    });
    assert_eq!(found.len(), 1, "{marker}: {} matches", found.len());
    let bytes = chunk::write(
        &Chunk {
            format: Format::HOST,
            main: found.remove(0),
        },
        Format::HOST,
    )
    .unwrap();
    ui.lua.load(bytes).into_function().unwrap()
}

#[test]
fn unchanged_character_info_updatehud_restores_background_and_retains_equipment_buttons() {
    let ui = runtime();
    setup(&ui);
    run(
        &ui,
        r#"
        low:Show(false)
        title=high:CreateChildWidget('emptywidget','title',0,true)
        popup=high:CreateChildWidget('window','popup',0,true)
        popup:AddAnchor('TOPLEFT',high,400,0);popup:SetExtent(30,30)
        popup.titleBar=title
        button=high:CreateChildWidget('button','equipmentButton',0,true)
        button:AddAnchor('TOPLEFT',high,20,20);button:SetExtent(30,30)
        popupBtn=high:CreateChildWidget('button','popupBtn',0,true)
        characterInfo={mainWindow=high,popupWindow=popup,popupBtn=popupBtn,subtitle={},popupSubtitle={}}
        began={};equippedItem={Begin=function(self,on)table.insert(began,on)end}
        tooltipSet=0;tooltipRelease=0
        function SetTooltipHandlers()tooltipSet=tooltipSet+1 end
        function ReleaseTooltipHandlers()tooltipRelease=tooltipRelease+1 end
        function HideTooltip()end
    "#,
    );
    const FILE: &str = "scriptsbin/x2ui/chracterinfo/character_info.alb";
    ui.lua
        .load("return characterInfo.mainWindow")
        .eval::<mlua::Table>()
        .unwrap()
        .set(
            "ReleaseHandlers",
            original(&ui, FILE, "ReleaseHandler+titleBar+OnDragStop"),
        )
        .unwrap();
    ui.lua
        .globals()
        .set(
            "updateHud",
            original(&ui, FILE, "EnableWindowPick+ReleaseHandlers"),
        )
        .unwrap();
    run(&ui, "updateHud(high,false)");
    assert_eq!(input::pick(&ui.lua, 60., 60.), Some(id(&ui, "under")));
    assert_eq!(input::pick(&ui.lua, 75., 75.), Some(id(&ui, "button")));
    run(&ui, "updateHud(high,true)");
    assert_eq!(input::pick(&ui.lua, 60., 60.), Some(id(&ui, "high")));
    assert_eq!(input::pick(&ui.lua, 75., 75.), Some(id(&ui, "button")));
    run(
        &ui,
        "assert(began[1]==false and began[2]==true and tooltipSet==2 and tooltipRelease==2)",
    );
}

#[test]
fn unchanged_login_constructor_leaves_game_layer_backdrop_transparent_to_input() {
    let mut ui = runtime();
    fake::install(&mut ui, "en_us", &fake::default_worlds());
    run(
        &ui,
        r#"
        local previous=Console.GetAttribute
        function Console:GetAttribute(name)
          if name=='cl_account' then return '' end
          return previous(self,name)
        end
    "#,
    );
    ui.run_stage(aa_ui::Stage::Login);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    run(
        &ui,
        r#"
        loginFloor=UIParent:CreateWidget('window','loginFloor','UIParent')
        loginFloor:SetUILayer('background');loginFloor:SetExtent(20,20);loginFloor:Show(true)
        loginUnder=loginFloor:CreateChildWidget('button','loginUnder',0,true)
        loginUnder:SetExtent(20,20)
        loginUnder:AddAnchor('TOPLEFT',UIParent,0,0);loginUnder:Show(true)
    "#,
    );
    let back = id(&ui, "login.backMain");
    assert!(!lua::ui(&ui.lua).tree.widget(back).unwrap().window_pick);
    assert_eq!(input::pick(&ui.lua, 5., 5.), Some(id(&ui, "loginUnder")));
    assert!(
        !ui.stub_calls()
            .iter()
            .any(|(n, _)| n == "window:EnableWindowPick")
    );
}
