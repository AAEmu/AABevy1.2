//! Pinned native392a3e00 mouse branches through actual UiRuntime picking/dispatch.
//! Original wrapper and dungeon-selection ALB prototypes run unchanged; no GPU/network.
use aa_ui::{UiRuntime, input, lua, tree::Id};

fn run(ui: &UiRuntime, code: &str) {
    ui.exec("@listctrl_doubleclick", code).unwrap();
    lua::tick(&ui.lua, 0.);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
}

fn id(ui: &UiRuntime, expression: &str) -> Id {
    let value = ui.lua.load(expression).eval().unwrap();
    lua::value_id(&ui.lua, &value).unwrap()
}

fn setup() -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    run(
        &ui,
        r#"
        host=UIParent:CreateWidget('window','doubleHost','UIParent')
        host:SetExtent(400,300);host:AddAnchor('TOPLEFT',UIParent,20,30)
        host:Show(true)
        list=host:CreateChildWidget('listctrl','list',0,true)
        list:SetExtent(300,240);list:AddAnchor('TOPLEFT',host,0,0)
        list:InsertColumn(140,2);list:InsertColumn(160,2)
        list:SetColumnHeight(0);list:InsertRows(3,false)
        list:CreateSelectedImage();list:CreateOveredImage()
        list:InsertData(101,1,'first');list:InsertData(202,1,'second')
        events={};clicks=0
        list:SetHandler('OnSelChanged',function(self,index,double)
          table.insert(events,index..':'..tostring(double))
        end)
        list:SetHandler('OnClick',function() clicks=clicks+1 end)
        "#,
    );
    ui
}

fn point(ui: &UiRuntime, row: usize) -> (f32, f32) {
    let u = lua::ui(&ui.lua);
    let rect = u.tree.rect(id(ui, &format!("list.items[{row}]")));
    (rect.x + rect.w / 2., rect.y + rect.h / 2.)
}

fn click(ui: &UiRuntime, mouse: &mut input::MouseState, row: usize, time: f64) {
    lua::ui(&ui.lua).time_ms = time;
    let cursor = Some(point(ui, row));
    input::update(&ui.lua, mouse, cursor, true, false, false, 0.);
    input::update(&ui.lua, mouse, cursor, false, true, false, 0.);
}

fn events(ui: &UiRuntime) -> String {
    ui.lua.load("table.concat(events,'|')").eval().unwrap()
}

fn selected(ui: &UiRuntime) -> i32 {
    ui.lua.load("list:GetSelectedIdx()").eval().unwrap()
}

fn assert_keys(ui: &UiRuntime) {
    use aa_ui::tree::ListKey;
    let u = lua::ui(&ui.lua);
    let list = u
        .tree
        .widget(id(ui, "list"))
        .unwrap()
        .list
        .as_ref()
        .unwrap();
    assert_eq!(
        list.keys,
        [Some(ListKey::Num(101.)), Some(ListKey::Num(202.)), None]
    );
    assert_eq!(list.has_data, [true, true, false]);
}

#[test]
fn native_false_default_and_boolean_setter_do_not_change_selection_or_keys() {
    let ui = setup();
    let use_double = || {
        lua::ui(&ui.lua)
            .tree
            .widget(id(&ui, "list"))
            .unwrap()
            .list
            .as_ref()
            .unwrap()
            .use_double_click
    };
    assert!(!use_double());
    run(
        &ui,
        "list:Select(1,false);assert(select('#',list:SetUseDoubleClick(true))==0)",
    );
    assert!(use_double());
    assert_eq!(selected(&ui), 2);
    run(&ui, "assert(select('#',list:SetUseDoubleClick(false))==0)");
    assert!(!use_double());
    assert_eq!(selected(&ui), 2);
    assert!(events(&ui).is_empty());
    assert_keys(&ui);
}

#[test]
fn default_rapid_and_slow_repeats_toggle_without_double_notification() {
    let ui = setup();
    let mut mouse = input::MouseState::default();
    click(&ui, &mut mouse, 1, 1000.);
    assert_eq!(selected(&ui), 1);
    click(&ui, &mut mouse, 1, 1100.);
    assert_eq!(selected(&ui), 0);
    {
        let u = lua::ui(&ui.lua);
        let list = u
            .tree
            .widget(id(&ui, "list"))
            .unwrap()
            .list
            .as_ref()
            .unwrap();
        assert!(!u.tree.get(list.selected_image.unwrap()).shown);
        assert_eq!(list.overed, 0);
    }
    click(&ui, &mut mouse, 2, 2000.);
    click(&ui, &mut mouse, 2, 3000.);
    assert_eq!(selected(&ui), 0);
    assert_eq!(events(&ui), "1:false|0:false|2:false|0:false");
    run(&ui, "assert(clicks==4)");
    assert_keys(&ui);
}

#[test]
fn enabled_double_notifies_existing_selection_without_touching_highlight() {
    let ui = setup();
    run(&ui, "list:SetUseDoubleClick(true)");
    let mut mouse = input::MouseState::default();
    click(&ui, &mut mouse, 1, 1000.);
    // The callback changes selection after picking but before native list dispatch.
    // The native double branch must report that selection, not select hovered row 1.
    run(
        &ui,
        "list:SetHandler('OnClick',function() list:Select(1,false) end)",
    );
    click(&ui, &mut mouse, 1, 1100.);
    assert_eq!(selected(&ui), 2);
    assert_eq!(events(&ui), "1:false|2:true");
    run(&ui, "list:SetHandler('OnClick',nil)");
    let highlight = {
        let u = lua::ui(&ui.lua);
        let list = u
            .tree
            .widget(id(&ui, "list"))
            .unwrap()
            .list
            .as_ref()
            .unwrap();
        list.selected_image.unwrap()
    };
    // Direct edits are a sentinel: a double notification must leave them intact.
    let before = {
        let mut u = lua::ui(&ui.lua);
        u.tree.get_mut(highlight).shown = false;
        u.tree.get_mut(highlight).width = 73.;
        u.tree.get(highlight).clone()
    };
    click(&ui, &mut mouse, 1, 1200.);
    assert_eq!(lua::ui(&ui.lua).tree.get(highlight), &before);
    assert_eq!(selected(&ui), 2);
    assert_eq!(events(&ui), "1:false|2:true|2:true");
    // An ordinary enabled repeat keeps the row selected and reports false.
    click(&ui, &mut mouse, 1, 2000.);
    click(&ui, &mut mouse, 1, 3000.);
    assert_eq!(selected(&ui), 1);
    assert_eq!(events(&ui), "1:false|2:true|2:true|1:false|1:false");
    assert_keys(&ui);
}

#[test]
fn callback_flag_changes_apply_at_native_entry_and_no_selection_double_reports_zero() {
    let ui = setup();
    let mut mouse = input::MouseState::default();
    click(&ui, &mut mouse, 1, 1000.);
    run(
        &ui,
        "list:SetHandler('OnClick',function() list:SetUseDoubleClick(true);list:ClearSelection() end)",
    );
    click(&ui, &mut mouse, 1, 1100.);
    assert_eq!(selected(&ui), 0);
    assert_eq!(events(&ui), "1:false|0:false|0:true");
    run(&ui, "list:SetHandler('OnClick',nil)");
    click(&ui, &mut mouse, 1, 2000.);
    run(
        &ui,
        "list:SetHandler('OnClick',function() list:SetUseDoubleClick(false) end)",
    );
    click(&ui, &mut mouse, 1, 2100.);
    assert_eq!(selected(&ui), 0);
    assert_eq!(events(&ui), "1:false|0:false|0:true|1:false|0:false");
    assert_keys(&ui);
}

#[test]
fn right_empty_disabled_hidden_and_retired_controls_do_not_gain_selection() {
    let ui = setup();
    run(&ui, "list:SetUseDoubleClick(true)");
    let mut mouse = input::MouseState::default();
    let p = Some(point(&ui, 1));
    input::update(&ui.lua, &mut mouse, p, false, false, true, 0.);
    assert_eq!(selected(&ui), 0);
    click(&ui, &mut mouse, 3, 1000.);
    run(&ui, "list:Enable(false)");
    click(&ui, &mut mouse, 1, 2000.);
    run(&ui, "list:Enable(true);host:Show(false)");
    click(&ui, &mut mouse, 1, 3000.);
    assert!(events(&ui).is_empty());
    assert_eq!(selected(&ui), 0);
    // Reentrant frame delivery retires the native subtree before click dispatch
    // resumes. Use the supported hide/removal lifecycle, not a synthetic kill.
    ui.lua
        .globals()
        .set(
            "retireList",
            ui.lua
                .create_function(|lua, _: mlua::MultiValue| {
                    lua.load("list:EnableHidingIsRemove(true);list:Show(false)")
                        .exec()?;
                    lua::tick(lua, 0.);
                    Ok(())
                })
                .unwrap(),
        )
        .unwrap();
    run(&ui, "host:Show(true);list:SetHandler('OnClick',retireList)");
    click(&ui, &mut mouse, 1, 4000.);
    assert!(events(&ui).is_empty());
    assert!(!lua::ui(&ui.lua).tree.get(id(&ui, "list")).alive);
}

#[test]
fn scripted_select_keeps_existing_repeated_fire_index_and_suppression_contract() {
    let ui = setup();
    for enabled in [false, true] {
        run(
            &ui,
            &format!("list:SetUseDoubleClick({enabled});events={{}}"),
        );
        run(&ui, "list:Select(0);list:Select(0);list:Select(1,false)");
        assert_eq!(selected(&ui), 2);
        assert_eq!(events(&ui), "1:false|1:false");
        run(&ui, "list:ClearSelection();list:ClearSelection()");
        assert_eq!(selected(&ui), 0);
        assert_eq!(events(&ui), "1:false|1:false|0:false");
        assert_keys(&ui);
    }
}

// Serialization conversion only; each selected prototype retains the original instructions.
fn original(ui: &UiRuntime, file: &str, params: u8, markers: &[&str]) -> mlua::Function {
    use aa_lua::chunk::{self, Chunk, Constant, Format};
    let bytes = aa_pak::Pak::global().read(file).unwrap();
    let chunk = chunk::parse(&bytes).unwrap();
    let mut found = Vec::new();
    chunk.main.walk(&mut |p| {
        if p.num_params == params
            && markers.iter().all(|m| {
                p.constants
                    .iter()
                    .any(|c| matches!(c, Constant::String(s) if s == m.as_bytes()))
            })
        {
            found.push(p.clone());
        }
    });
    assert_eq!(
        found.len(),
        1,
        "{file} {markers:?}: {} matches",
        found.len()
    );
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
fn original_page_and_scroll_wrappers_forward_flag_and_dungeon_double_activation() {
    for (wrapper, offset) in [
        ("pagelistctrl", "pageIndex"),
        ("scrolllistctrl", "topDataIndex"),
    ] {
        let ui = setup();
        let file = format!("scriptsbin/x2ui/components/{wrapper}.alb");
        ui.lua
            .globals()
            .set(
                "originalSetDouble",
                original(&ui, &file, 2, &["useDoubleClick", "SetUseDoubleClick"]),
            )
            .unwrap();
        ui.lua
            .globals()
            .set(
                "originalSelection",
                original(&ui, &file, 3, &["selectedDataKey", "SelChangedProc"]),
            )
            .unwrap();
        ui.lua
            .globals()
            .set(
                "originalDungeonSelection",
                original(
                    &ui,
                    "scriptsbin/x2ui/indunchannelselect/indun_channel_select.alb",
                    5,
                    &["leftButtonLeftClickFunc", "IsEnabled"],
                ),
            )
            .unwrap();
        run(
            &ui,
            &format!(
                r#"
          frame={{listCtrl=list,dataCount=2,datas={{{{101}},{{202}}}},{offset}=1}}
          function frame:GetSelectedRowViewIndex(i) return i end
          window={{leftButton=host:CreateChildWidget('button','enterDungeon',0,true)}}
          window.leftButton:Enable(false)
          activations=0
          info={{leftButtonLeftClickFunc=function() activations=activations+1 end}}
          ROWDATA_COLUMN_OFFSET=1
          assert(debug.setupvalue(originalSelection,1,frame)=='frame')
          assert(debug.setupvalue(originalDungeonSelection,1,window)=='window')
          assert(debug.setupvalue(originalDungeonSelection,2,info)=='info')
          frame.SelChangedProc=originalDungeonSelection
          list:SetHandler('OnSelChanged',originalSelection)
          frame.SetUseDoubleClick=originalSetDouble
          frame:SetUseDoubleClick(true)
          assert(frame.useDoubleClick==true)
        "#
            ),
        );
        let mut mouse = input::MouseState::default();
        click(&ui, &mut mouse, 1, 1000.);
        run(
            &ui,
            "assert(frame.selectedDataKey==101 and frame.selectedDataIndex==1);assert(window.leftButton:IsEnabled());assert(activations==0)",
        );
        click(&ui, &mut mouse, 1, 1100.);
        run(
            &ui,
            "assert(frame.selectedDataKey==101 and frame.selectedDataIndex==1);assert(activations==1)",
        );
        run(&ui, "frame:SetUseDoubleClick(false)");
        click(&ui, &mut mouse, 1, 1200.);
        assert_eq!(selected(&ui), 0);
        run(
            &ui,
            "assert(frame.selectedDataKey==nil and frame.selectedDataIndex==0);assert(not window.leftButton:IsEnabled());assert(activations==1)",
        );
        assert_keys(&ui);
    }
}
