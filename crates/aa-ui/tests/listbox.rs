//! Native row indices, metrics, input, and original shared dropdown bytecode.

#[path = "../examples/fake/mod.rs"]
#[allow(dead_code)]
mod fake;

use aa_ui::{UiRuntime, lua};

fn eval<T: mlua::FromLuaMulti>(ui: &UiRuntime, s: &str) -> T {
    ui.lua.load(s).eval().expect(s)
}
fn id(ui: &UiRuntime, name: &str) -> aa_ui::tree::Id {
    lua::value_id(&ui.lua, &eval(ui, name)).unwrap()
}
fn setup() -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1920.0, 1080.0).unwrap();
    ui.exec("@listbox",r#"
        list=UIParent:CreateWidget('listbox','nativeListbox','UIParent')
        list:SetExtent(200,100); list:AddAnchor('TOPLEFT',UIParent,20,30)
        list:SetHeight(8); list:SetInset(5,7,5,7); list.itemStyle:SetFontSize(14)
        list:Show(true); events={}
        list:SetHandler('OnSelChanged',function(self) table.insert(events,self:GetSelectedIndex()) end)
    "#).unwrap();
    ui
}
fn click(ui: &UiRuntime, mouse: &mut aa_ui::input::MouseState, p: (f32, f32)) {
    aa_ui::input::update(&ui.lua, mouse, Some(p), true, false, false, 0.0);
    aa_ui::input::update(&ui.lua, mouse, Some(p), false, true, false, 0.0);
}
fn row(ui: &UiRuntime, name: &str, index: usize) -> (f32, f32) {
    let (x, y): (f32, f32) = eval(
        ui,
        &format!(
            "local x,y={name}:GetOffset(); local l,t={name}:GetInset(); return x+l+12,y+t+{name}:GetHeight()*({index}+0.5)"
        ),
    );
    (x, y)
}

#[test]
fn zero_based_select_getters_repeat_clear_and_native_negative_selection() {
    let ui = setup();
    ui.exec("@items",r#"
        assert(list:GetSelectedIndex()==-1)
        assert(select('#',list:GetSelectedText())==0 and select('#',list:GetSelectedValue())==0)
        list:AppendItem('Alpha',11); list:AppendItem('Beta',22); list:AppendItem('Gamma',33)
        assert(list:ItemCount()==3 and list:GetOpenedItemCount()==3)
        list:Select(0); assert(list:GetSelectedValue()==11 and list:GetSelectedText()=='Alpha')
        list:Select(0); assert(#events==1)
        list:Select(2); assert(#events==2 and list:GetSelectedValue()==33)
        list:Select(99); assert(#events==2 and list:GetSelectedIndex()==2)
        list:ClearAllSelected(); assert(list:GetSelectedIndex()==2 and list:GetSelectedValue()==nil)
        list:Select(-1); assert(#events==3 and list:GetSelectedIndex()==-1)
        list:InitializeSelect(1); assert(#events==4 and list:GetSelectedValue()==22)
        list:SetTop(99); list:ClearItem()
        assert(list:ItemCount()==0 and list:GetSelectedIndex()==-1 and list:GetTop()==0 and #events==4)
    "#).unwrap();
    ui.exec(
        "@selected_idle",
        "list:AppendItem('Alpha',11); list:Select(0)",
    )
    .unwrap();
    lua::ui(&ui.lua).tree.settle();
    ui.exec("@repeat_idle", "list:Select(0)").unwrap();
    assert!(!lua::ui(&ui.lua).tree.changed());
    ui.exec("@integer_value", "list:AppendItem('Exact native i32',16777217); list:Select(1); assert(list:GetSelectedValue()==16777217)").unwrap();
}

#[test]
fn row_padding_does_not_resize_widget_and_top_uses_rounded_view_rows() {
    let ui = setup();
    let h = eval::<f32>(&ui, "list:GetHeight()");
    assert!(h > 8.0);
    let extent = lua::ui(&ui.lua).tree.rect(id(&ui, "list"));
    assert_eq!(extent.h, 100.0);
    ui.exec(
        "@metrics",
        r#"
        local h=list:GetHeight()
        list:SetHeight(-1); assert(list:GetHeight()==h)
        list:SetExtent(200,2.49*h+14); assert(list:GetViewItemCount()==2)
        list:SetExtent(200,2.51*h+14); assert(list:GetViewItemCount()==3)
        for i=1,12 do list:AppendItem('Row '..i,i) end
        assert(list:GetMaxTop()==9)
        list:SetTop(999); assert(list:GetTop()==9)
        list:ScrollDown(); assert(list:GetTop()==9)
        list:ScrollUp(); assert(list:GetTop()==8)
        list:SetTop(-10); assert(list:GetTop()==0)
    "#,
    )
    .unwrap();
    lua::ui(&ui.lua).tree.settle();
    ui.exec(
        "@idle",
        "list:SetTop(0); list:SetHeight(8); list:UseConsistentTextColor(true)",
    )
    .unwrap();
    assert!(!lua::ui(&ui.lua).tree.changed());
}

#[test]
fn mouse_notifies_same_row_after_onclick_and_handlers_can_clear_storage() {
    let ui = setup();
    ui.exec("@mouse",r#"
        list:AppendItem('Alpha',11); list:AppendItem('Beta',22)
        before={}; list:SetHandler('OnClick',function(self) table.insert(before,self:GetSelectedIndex()) end)
    "#).unwrap();
    let mut mouse = Default::default();
    let p = row(&ui, "list", 1);
    let picked = aa_ui::input::pick(&ui.lua, p.0, p.1);
    assert_eq!(
        picked,
        Some(id(&ui, "list")),
        "point={p:?}, picked={:?}",
        picked.map(|i| lua::ui(&ui.lua).tree.get(i).name.clone())
    );
    click(&ui, &mut mouse, p);
    click(&ui, &mut mouse, row(&ui, "list", 1));
    assert_eq!(eval::<String>(&ui, "table.concat(before,',')"), "-1,1");
    assert_eq!(eval::<String>(&ui, "table.concat(events,',')"), "1,1");
    ui.exec(
        "@clear_callback",
        "list:SetHandler('OnSelChanged',function(self) self:ClearItem() end)",
    )
    .unwrap();
    click(&ui, &mut mouse, row(&ui, "list", 0));
    assert_eq!(eval::<usize>(&ui, "list:ItemCount()"), 0);
}

#[test]
fn stored_text_limit_colors_hover_and_rendering_clip_to_authored_view() {
    let ui = setup();
    ui.exec(
        "@look",
        r#"
        list:SetTextLimit(5); list:AppendItem('abcdefgh',11,1,0,0,1)
        list:Select(0); assert(list:GetSelectedText()=='abcd...')
        list:UseConsistentTextColor(false)
        for i=1,20 do list:AppendItem('More text '..i,i) end
    "#,
    )
    .unwrap();
    let quads = aa_ui::render::build_quads(&mut lua::ui(&ui.lua), None);
    assert!(quads.iter().any(|q| q.color == [1.0, 0.0, 0.0, 1.0]));
    for q in quads
        .iter()
        .filter(|q| q.pos[2] > q.pos[0] && q.pos[3] > q.pos[1])
    {
        assert!(
            q.pos[0] >= 25.0 && q.pos[1] >= 37.0 && q.pos[2] <= 215.0 && q.pos[3] <= 123.0,
            "{:?}",
            q.pos
        );
    }
    let mut mouse = Default::default();
    aa_ui::input::update(
        &ui.lua,
        &mut mouse,
        Some(row(&ui, "list", 0)),
        false,
        false,
        false,
        0.0,
    );
    lua::ui(&ui.lua).tree.settle();
    aa_ui::input::update(
        &ui.lua,
        &mut mouse,
        Some(row(&ui, "list", 1)),
        false,
        false,
        false,
        0.0,
    );
    assert!(lua::ui(&ui.lua).tree.changed());
    lua::ui(&ui.lua).tree.settle();
    let generation = lua::ui(&ui.lua).tree.generation;
    aa_ui::input::update(
        &ui.lua,
        &mut mouse,
        Some(row(&ui, "list", 1)),
        false,
        false,
        false,
        0.0,
    );
    assert!(!lua::ui(&ui.lua).tree.changed());
    assert_eq!(lua::ui(&ui.lua).tree.generation, generation);
}

#[test]
fn original_world_dropdown_opens_selects_by_mouse_cleans_up_and_scrolls() {
    let mut ui = UiRuntime::new("en_us", 1920.0, 1080.0).unwrap();
    fake::install(&mut ui, "en_us", &fake::default_worlds());
    fake::install_ingame(&mut ui);
    ui.run_stage(aa_ui::Stage::World);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    ui.exec(
        "@original_combo",
        r#"
        frame=CreateEmptyWindow('nativeComboFrame',UIParent)
        frame:SetExtent(300,200); frame:AddAnchor('TOPLEFT',UIParent,260,130); frame:Show(true)
        combo=CreateComboBox(frame); combo:SetWidth(180); combo:AddAnchor('TOPLEFT',frame,10,10)
        combo.dropdownItem={'One','Two','Three','Four','Five','Six'}; combo.limitItemCount=3
        combo:Select(1); picked={}
        combo.SelectedProc=function(self,index) table.insert(picked,index) end
    "#,
    )
    .unwrap();
    let mut mouse = Default::default();
    let r = lua::ui(&ui.lua).tree.rect(id(&ui, "combo"));
    let point = (r.x + 25.0, r.y + 10.0);
    click(&ui, &mut mouse, point);
    ui.exec("@dropdown", "dropdown=combo:GetAttachedWidget(); list=dropdown.list; assert(dropdown:IsVisible()); assert(list:ItemCount()==6)").unwrap();
    let p = row(&ui, "list", 1);
    let list = id(&ui, "list");
    let u = lua::ui(&ui.lua);
    let base = u.tree.rect(list);
    let view = u.animations.widget_rect(&u.tree, list);
    drop(u);
    let picked = aa_ui::input::pick(&ui.lua, p.0, p.1);
    assert_eq!(
        picked,
        Some(list),
        "point={p:?} base={base:?} view={view:?} picked={:?}",
        picked.map(|i| lua::ui(&ui.lua).tree.get(i).name.clone())
    );
    click(&ui, &mut mouse, row(&ui, "list", 1));
    ui.exec(
        "@selected",
        r#"
        assert(combo:GetSelectedIndex()==2 and combo:GetText()=='Two' and picked[1]==2,
            string.format('index=%s text=%s picked=%s native=%s rows=%s visible=%s',tostring(combo:GetSelectedIndex()),combo:GetText(),tostring(picked[1]),tostring(list:GetSelectedIndex()),tostring(list:ItemCount()),tostring(dropdown:IsVisible())))
        assert(not dropdown:IsVisible() and list:ItemCount()==0)
    "#,
    )
    .unwrap();
    click(&ui, &mut mouse, point);
    assert_eq!(eval::<usize>(&ui, "list:ItemCount()"), 6);
    click(&ui, &mut mouse, row(&ui, "list", 1)); // Selected row still closes and notifies.
    assert_eq!(eval::<usize>(&ui, "#picked"), 2);
    assert!(!eval::<bool>(&ui, "dropdown:IsVisible()"));
    click(&ui, &mut mouse, point);
    aa_ui::input::update(
        &ui.lua,
        &mut mouse,
        Some(row(&ui, "list", 1)),
        false,
        false,
        false,
        -1.0,
    );
    assert_eq!(eval::<i32>(&ui, "list:GetTop()"), 1);
    click(&ui, &mut mouse, row(&ui, "list", 2));
    assert_eq!(eval::<i32>(&ui, "combo:GetSelectedIndex()"), 4);
}

#[test]
fn original_options_resolution_dropdown_mouse_selection_and_reopen() {
    let mut ui = UiRuntime::new("en_us", 1920.0, 1080.0).unwrap();
    fake::install(&mut ui, "en_us", &fake::default_worlds());
    fake::install_ingame(&mut ui);
    ui.run_stage(aa_ui::Stage::World);
    fake::enter_world(&mut ui);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    ui.exec(
        "@original_options",
        r#"
        options=CreateOptionFrame(); options:Show(true); options:Init(); options:SelectSubFrame(1)
        screen=options.contentWindow.pageWindow.subFrames[1]
        combo=screen.content.optionFrames[1].optionControl
        assert(#combo.dropdownItem==4)
    "#,
    )
    .unwrap();
    let mut mouse = Default::default();
    let r = lua::ui(&ui.lua).tree.rect(id(&ui, "combo"));
    let point = (r.x + 20.0, r.y + r.h * 0.5);
    click(&ui, &mut mouse, point);
    ui.exec("@options_open", "dropdown=combo:GetAttachedWidget(); list=dropdown.list; assert(dropdown:IsVisible() and list:ItemCount()==4)").unwrap();
    click(&ui, &mut mouse, row(&ui, "list", 1));
    ui.exec(
        "@options_selection",
        r#"
        assert(combo:GetSelectedIndex()==2 and combo:GetText()=='1600 x 900')
        assert(not dropdown:IsVisible() and list:ItemCount()==0)
        saved={}; AAUI.OptionChanged=function(id,value) saved[id]=value end
        combo:Save()
        assert(saved[OPTION_ITEM_SCREEN_RESOLUTION_WIDTH]==1600)
        assert(saved[OPTION_ITEM_SCREEN_RESOLUTION_HEIGHT]==900)
    "#,
    )
    .unwrap();
    click(&ui, &mut mouse, point);
    assert_eq!(eval::<i32>(&ui, "list:GetSelectedIndex()"), 1);
    assert_eq!(eval::<usize>(&ui, "list:ItemCount()"), 4);
    click(&ui, &mut mouse, row(&ui, "list", 1));
    assert!(!eval::<bool>(&ui, "dropdown:IsVisible()"));
}
