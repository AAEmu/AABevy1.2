//! Recursive native records and actual original crafting category input.
#[path = "../examples/fake/mod.rs"]
#[allow(dead_code)]
mod fake;
use aa_ui::{UiRuntime, lua};

fn exec(ui: &UiRuntime, s: &str) {
    ui.exec("@listbox_tree", s).unwrap();
}
fn eval<T: mlua::FromLuaMulti>(ui: &UiRuntime, s: &str) -> T {
    ui.lua.load(s).eval().unwrap()
}
fn id(ui: &UiRuntime, s: &str) -> aa_ui::tree::Id {
    lua::value_id(&ui.lua, &eval(ui, s)).unwrap()
}
fn click(ui: &UiRuntime, mouse: &mut aa_ui::input::MouseState, p: (f32, f32)) {
    aa_ui::input::update(&ui.lua, mouse, Some(p), true, false, false, 0.0);
    aa_ui::input::update(&ui.lua, mouse, Some(p), false, true, false, 0.0);
}
fn row(ui: &UiRuntime, i: usize) -> (f32, f32) {
    eval(
        ui,
        &format!(
            "local x,y=list:GetOffset();local l,t=list:GetInset();return x+l+40,y+t+list:GetHeight()*({i}+.5)"
        ),
    )
}
fn setup() -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    exec(
        &ui,
        r#"
        list=UIParent:CreateWidget('listbox','treeTest','UIParent')
        list:AddAnchor('TOPLEFT',UIParent,60,70);list:SetExtent(320,140)
        list:SetInset(5,7,5,7);list:SetHeight(8);list.itemStyle:SetFontSize(14);list:Show(true)
        events={};list:SetHandler('OnSelChanged',function(self) table.insert(events,'sel:'..self:GetSelectedIndex()) end)
        list:SetHandler('OnListboxToggled',function(self) table.insert(events,'toggle:'..self:GetSelectedIndex()) end)
        list:SetItemTrees({{text='A',value=100,opened=true,child={
            {text='a',value=11},{text='B',value=200,opened=true,child={{text='b',value=12}}}}},
            {text='Z',value=20}})
    "#,
    );
    ui
}

#[test]
fn recursive_input_visible_viewport_and_native_root_selection_operations() {
    let ui = setup();
    exec(
        &ui,
        r#"
        assert(list:ItemCount()==2 and list:GetOpenedItemCount()==5)
        local infos=list:GetViewItemsInfo();assert(#infos==5 and infos[1].value==100 and infos[4].value==12)
        assert(infos[1].opened and infos[3].opened and infos[2].opened==false and infos[1].text==nil)
        local h=list:GetHeight();list:SetExtent(320,h*2.49+14)
        assert(list:GetViewItemCount()==2 and #list:GetViewItemsInfo()==2 and list:GetMaxTop()==3)
        list:SetTop(2);infos=list:GetViewItemsInfo();assert(infos[1].value==200 and infos[2].value==12)
        list:Select(1);assert(list:GetSelectedIndex()==-1 and #events==0)
        list:InitializeSelect(0);assert(list:GetSelectedIndex()==0 and list:GetSelectedValue()==100)
        assert(list:GetOpenedItemCount()==2 and #events==1 and events[1]=='sel:0')
        list:ClearAllSelected();assert(list:GetSelectedIndex()==-1 and list:GetSelectedValue()==nil)
        list:SetTop(0);assert(list:GetMaxTop()==0 and #list:GetViewItemsInfo()==2)
        list:SetItemTrees({{text='First'},{text=99},{text='Not appended'}})
        assert(list:ItemCount()==1 and list:GetViewItemsInfo()[1].value==0)
        list:SetItemTrees(nil);assert(list:ItemCount()==1)
        list:ClearItem();list:AppendItem('Flat again',91);list:Select(0);assert(list:GetSelectedValue()==91)
    "#,
    );
}

#[test]
fn parent_toggle_selection_event_order_hidden_leaf_and_callback_replacement() {
    let ui = setup();
    let mut mouse = Default::default();
    exec(&ui, "list:EnableSelectParent(false)");
    click(&ui, &mut mouse, row(&ui, 1));
    exec(
        &ui,
        "assert(list:GetSelectedIndex()==1 and list:GetSelectedValue()==11)",
    );
    click(&ui, &mut mouse, row(&ui, 0));
    exec(
        &ui,
        r#"
        assert(list:GetSelectedIndex()==1 and list:GetSelectedValue()==11 and list:GetOpenedItemCount()==2)
        assert(table.concat(events,',')=='sel:1,toggle:-1')
        -- Native text getter indexes roots, unlike the recursively selected value.
        assert(list:GetSelectedText()=='Z')
    "#,
    );
    click(&ui, &mut mouse, row(&ui, 0));
    exec(
        &ui,
        "assert(list:GetSelectedValue()==11 and list:GetOpenedItemCount()==5);list:EnableSelectParent(true)",
    );
    click(&ui, &mut mouse, row(&ui, 0)); // Closing a selectable parent clears selection.
    exec(
        &ui,
        "assert(list:GetSelectedIndex()==-1 and list:GetSelectedValue()==nil and events[#events]=='sel:-1')",
    );
    exec(
        &ui,
        "list:SetHandler('OnListboxToggled',function(self) self:ClearItem();self:AppendItem('Replacement',77) end)",
    );
    click(&ui, &mut mouse, row(&ui, 0));
    exec(
        &ui,
        "assert(list:ItemCount()==1 and list:GetSelectedIndex()==-1 and list:GetSelectedValue()==nil)",
    );
}

#[test]
fn authored_child_style_and_open_closed_image_geometry_follow_visible_rows() {
    let ui = setup();
    exec(
        &ui,
        r#"
        list:UseChildStyle(true);list.childStyle:SetFontSize(11);list.childStyle:SetColor(1,0,0,1)
        list:SetTreeTypeIndent(true,20)
        opened=list:CreateOpenedImageDrawable('ui/button/grid.dds');opened:SetCoords(0,0,21,14);opened:SetExtent(21,14)
        closed=list:CreateClosedImageDrawable('ui/button/grid.dds');closed:SetCoords(44,0,13,21);closed:SetExtent(13,21)
    "#,
    );
    let quads = aa_ui::render::build_quads(&mut lua::ui(&ui.lua), None);
    let icons: Vec<_> = quads
        .iter()
        .filter(|q| matches!(&q.tex, aa_ui::render::Tex::Image(p) if p.ends_with("ui/button/grid.dds")))
        .collect();
    assert_eq!(icons.len(), 2);
    let h = eval::<f32>(&ui, "list:GetHeight()");
    assert!((icons[0].pos[1] - (77. + h * 0.5 - 7.)).abs() < 0.01);
    assert!((icons[1].pos[0] - (65. + 25. - 10.5)).abs() < 0.01);
    let red: Vec<_> = quads
        .iter()
        .filter(|q| q.color == [1., 0., 0., 1.])
        .collect();
    assert!(!red.is_empty());
    assert!(red.iter().all(|q| q.pos[0] >= 100.));
    exec(&ui, "list:InitializeSelect(0)");
    let quads = aa_ui::render::build_quads(&mut lua::ui(&ui.lua), None);
    let icons: Vec<_> = quads
        .iter()
        .filter(|q| matches!(&q.tex, aa_ui::render::Tex::Image(p) if p.ends_with("ui/button/grid.dds")))
        .collect();
    assert_eq!(icons.len(), 1);
    assert!((icons[0].pos[2] - icons[0].pos[0] - 13.).abs() < 0.01);
}

#[test]
fn original_crafting_group_category_toggle_counts_scroll_and_leaf_selection() {
    let mut ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    fake::install(&mut ui, "en_us", &fake::default_worlds());
    fake::install_ingame(&mut ui);
    ui.exec(
        "@craft_data",
        include_str!("../examples/fixtures/crafting_tree.lua"),
    )
    .unwrap();
    ui.run_stage(aa_ui::Stage::World);
    fake::enter_world(&mut ui);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    exec(
        &ui,
        r#"
        craft=CreateCraftingWindowRenewal('nativeCraftTree','UIParent');craft:AddAnchor('CENTER',UIParent,0,0)
        craft:FillCraft(false);craft:Show(true)
        group=craft.abilityGroupPage.buttons[1]
        cat=craft.detailPage.categoryList;list=cat.content;counts=cat.countList
    "#,
    );
    let mut mouse = Default::default();
    let r = lua::ui(&ui.lua).tree.rect(id(&ui, "group"));
    click(&ui, &mut mouse, (r.x + r.w * 0.5, r.y + r.h * 0.5));
    exec(
        &ui,
        "assert(craft.detailPage:IsVisible() and list:ItemCount()==3 and list:GetOpenedItemCount()==3 and counts:ItemCount()==3)",
    );
    click(&ui, &mut mouse, row(&ui, 0));
    exec(
        &ui,
        "assert(list:GetOpenedItemCount()==27 and counts:ItemCount()==#list:GetViewItemsInfo() and list:GetSelectedIndex()==-1)",
    );
    click(&ui, &mut mouse, row(&ui, 1));
    exec(
        &ui,
        "assert(list:GetSelectedValue()==10101 and CRAFT_TREE_SELECTED[#CRAFT_TREE_SELECTED]==10101 and craft.detailPage.highRankFrame.combobox:GetSelectedIndex()==1)",
    );
    exec(
        &ui,
        "cat.scroll.vs:SetValue(4,true);assert(list:GetTop()==4 and list:GetViewItemsInfo()[1].value==10104 and counts:ItemCount()==#list:GetViewItemsInfo())",
    );
    click(&ui, &mut mouse, row(&ui, 0));
    exec(
        &ui,
        "assert(list:GetSelectedIndex()==4 and list:GetSelectedValue()==10104 and CRAFT_TREE_SELECTED[#CRAFT_TREE_SELECTED]==10104);cat.scroll.vs:SetValue(0,true)",
    );
    let calls = eval::<usize>(&ui, "#CRAFT_TREE_SELECTED");
    click(&ui, &mut mouse, row(&ui, 0));
    exec(
        &ui,
        "assert(list:GetOpenedItemCount()==3 and counts:ItemCount()==3 and list:GetSelectedIndex()==4 and list:GetSelectedValue()==10104)",
    );
    assert_eq!(eval::<usize>(&ui, "#CRAFT_TREE_SELECTED"), calls);
}
