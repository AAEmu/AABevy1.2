//! Native full-text tooltip events and original Lua positioning/lifecycle.
#[path = "../examples/fake/mod.rs"]
#[allow(dead_code)]
mod fake;
use aa_ui::{UiRuntime, lua};
fn exec(ui: &UiRuntime, s: &str) {
    ui.exec("@row_tooltip", s).unwrap()
}
fn eval<T: mlua::FromLuaMulti>(ui: &UiRuntime, s: &str) -> T {
    ui.lua.load(s).eval().unwrap()
}
fn id(ui: &UiRuntime, s: &str) -> aa_ui::tree::Id {
    lua::value_id(&ui.lua, &eval(ui, s)).unwrap()
}
fn point(ui: &UiRuntime, name: &str, row: usize) -> (f32, f32) {
    eval(
        ui,
        &format!(
            "local x,y={name}:GetOffset();local l,t={name}:GetInset();return x+l+12,y+t+{name}:GetHeight()*({row}+.5)"
        ),
    )
}
fn hover(ui: &UiRuntime, m: &mut aa_ui::input::MouseState, p: Option<(f32, f32)>) {
    aa_ui::input::update(&ui.lua, m, p, false, false, false, 0.);
}
#[test]
fn full_row_physical_coordinates_dedup_scroll_and_native_leave_order() {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    exec(
        &ui,
        r#"
        list=UIParent:CreateWidget('listbox','tipTest','UIParent')
        list:AddAnchor('TOPLEFT',UIParent,60,70);list:SetExtent(320,100)
        list:SetInset(5,7,5,7);list:SetHeight(8);list.itemStyle:SetFontSize(14);list:Show(true)
        list:SetListItemStateTextureInset(-15,-2,-10,-1)
        for i=1,12 do list:AppendItem('Row '..i,i) end
        events={};order={}
        list:SetHandler('OnTooltip',function(self,...)
            table.insert(events,{n=select('#',...),...});table.insert(order,'tip:'..select('#',...))
        end)
        list:SetHandler('OnLeave',function() table.insert(order,'leave') end)
        UIParent:SetUIScale(1.25)
    "#,
    );
    let mut mouse = Default::default();
    let p = point(&ui, "list", 0);
    hover(&ui, &mut mouse, Some(p));
    exec(&ui, "assert(#events==0);list:ShowTooltip(true)");
    hover(&ui, &mut mouse, Some(p));
    exec(
        &ui,
        "assert(#events==1 and events[1].n==3 and events[1][1]=='Row 1')",
    );
    let (x, y): (f32, f32) = eval(&ui, "return events[1][2],events[1][3]");
    let h = eval::<f32>(&ui, "list:GetHeight()");
    assert!((x - 380. * 1.25).abs() < 0.01);
    assert!((y - (77. + h + 1.) * 1.25).abs() < 0.01);
    lua::ui(&ui.lua).tree.settle();
    let generation = lua::ui(&ui.lua).tree.generation;
    hover(&ui, &mut mouse, Some(p));
    exec(&ui, "assert(#events==1)");
    assert!(!lua::ui(&ui.lua).tree.changed());
    assert_eq!(lua::ui(&ui.lua).tree.generation, generation);
    exec(
        &ui,
        "list:SetTop(3);assert(#events==2 and events[2][1]=='Row 4')",
    );
    exec(&ui, "list:ShowTooltip(false)");
    hover(&ui, &mut mouse, Some(point(&ui, "list", 1)));
    exec(&ui, "assert(#events==2)");
    hover(&ui, &mut mouse, None);
    exec(
        &ui,
        "assert(events[#events].n==0 and order[#order-1]=='tip:0' and order[#order]=='leave')",
    );
    exec(&ui, "list:ShowTooltip(true)");
    hover(&ui, &mut mouse, Some(p));
    exec(&ui, "assert(events[#events][1]=='Row 4')");
}

#[test]
fn tree_tooltip_follows_opened_preorder_and_callback_replacement_is_safe() {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    exec(
        &ui,
        r#"
        list=UIParent:CreateWidget('listbox','treeTipTest','UIParent')
        list:AddAnchor('TOPLEFT',UIParent,60,70);list:SetExtent(320,100);list:SetHeight(8)
        list:SetItemTrees({{text='Category',opened=true,child={{text='Recipe',value=77}}}})
        list:Show(true);list:ShowTooltip(true);tips={}
        list:SetHandler('OnTooltip',function(self,text)
            if text then table.insert(tips,text) end
            if text=='Recipe' then self:ClearItem();self:AppendItem('Replacement',99) end
        end)
    "#,
    );
    let mut mouse = Default::default();
    hover(&ui, &mut mouse, Some(point(&ui, "list", 1)));
    exec(&ui, "assert(tips[1]=='Recipe' and list:ItemCount()==1)");
    hover(&ui, &mut mouse, Some(point(&ui, "list", 0)));
    exec(&ui, "assert(tips[2]=='Replacement')");
    exec(&ui, "list:SetItemTrees({{text=string.rep('界',200)}})");
    hover(&ui, &mut mouse, Some(point(&ui, "list", 0)));
    exec(&ui, "assert(#tips[3]==511 and string.byte(tips[3],511)==0xe7)");
}

#[test]
fn original_slider_list_shows_full_text_at_native_corner_and_hides_on_leave() {
    let mut ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    fake::install(&mut ui, "en_us", &fake::default_worlds());
    fake::install_ingame(&mut ui);
    ui.run_stage(aa_ui::Stage::World);
    fake::enter_world(&mut ui);
    exec(
        &ui,
        include_str!("../examples/fixtures/listbox_tooltip.lua"),
    );
    let mut mouse = Default::default();
    hover(&ui, &mut mouse, Some(point(&ui, "list", 0)));
    exec(
        &ui,
        "assert(tooltip.window:IsVisible() and tooltip.window:GetLineCount()>=1)",
    );
    let tip = id(&ui, "tooltip.window");
    let u = lua::ui(&ui.lua);
    let lines = &u.tree.widget(tip).unwrap().tip.as_ref().unwrap().lines;
    assert_eq!(
        lines[0].left.text,
        "Restores Health over time while exploring Erenor"
    );
    drop(u);
    exec(&ui, "rows.scroll.vs:SetValue(4,true)");
    let u = lua::ui(&ui.lua);
    assert_eq!(
        u.tree.widget(tip).unwrap().tip.as_ref().unwrap().lines[0]
            .left
            .text,
        "Alchemy recipe 5"
    );
    drop(u);
    hover(&ui, &mut mouse, None);
    exec(&ui, "assert(not tooltip.window:IsVisible())");
}
