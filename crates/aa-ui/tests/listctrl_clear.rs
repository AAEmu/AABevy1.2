//! Native392a5750 selection-before-clear contract, including original dungeon ALB consumers.
//! CPU UiRuntime only; native oracle and all earlier evidence remain frozen.
#[path = "../examples/fake/mod.rs"]
#[allow(dead_code)]
mod fake;

use aa_ui::{
    UiRuntime, input, lua,
    tree::{Id, ListKey},
};

fn run(ui: &UiRuntime, code: &str) {
    ui.exec("@listctrl_clear", code).unwrap();
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
}

fn id(ui: &UiRuntime, expression: &str) -> Id {
    lua::value_id(&ui.lua, &ui.lua.load(expression).eval().unwrap()).unwrap()
}

fn setup(image: bool) -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    run(
        &ui,
        r#"
        host=UIParent:CreateWidget('window','clearHost','UIParent')
        host:SetExtent(400,300);host:AddAnchor('TOPLEFT',UIParent,20,30);host:Show(true)
        list=host:CreateChildWidget('listctrl','list',0,true)
        list:SetExtent(300,240);list:AddAnchor('TOPLEFT',host,0,0)
        list:InsertColumn(300,0);list:SetColumnHeight(0);list:InsertRows(3,false)
        list:CreateOveredImage()
        list:InsertData(101,1,'old first');list:InsertData(202,1,'old second')
        events={}
        function onClear(self,index,double)
          table.insert(events,index..':'..tostring(double))
        end
        list:SetHandler('OnSelChanged',onClear)
    "#,
    );
    if image {
        run(&ui, "list:CreateSelectedImage()");
    }
    run(&ui, "list:Select(1,false)");
    let list = id(&ui, "list");
    ui.lua
        .globals()
        .set(
            "assertOldData",
            ui.lua
                .create_function(move |lua, ()| {
                    let u = lua::ui(lua);
                    let l = u.tree.widget(list).unwrap().list.as_ref().unwrap();
                    assert_eq!(
                        l.keys,
                        [Some(ListKey::Num(101.)), Some(ListKey::Num(202.)), None]
                    );
                    assert_eq!(l.has_data, [true, true, false]);
                    assert_eq!(l.selected, -1);
                    assert!(l.selected_image.is_some_and(|img| !u.tree.get(img).shown));
                    for (subs, &occupied) in l.sub_items.iter().zip(&l.has_data) {
                        assert!(subs.iter().all(|&s| u.tree.get(s).shown == occupied));
                    }
                    Ok(())
                })
                .unwrap(),
        )
        .unwrap();
    ui
}

fn assert_empty(ui: &UiRuntime, count: usize) {
    let u = lua::ui(&ui.lua);
    let l = u
        .tree
        .widget(id(ui, "list"))
        .unwrap()
        .list
        .as_ref()
        .unwrap();
    assert_eq!(l.rows.len(), count);
    assert_eq!(l.keys, vec![None; count]);
    assert_eq!(l.has_data, vec![false; count]);
    for (&row, subs) in l.rows.iter().zip(&l.sub_items) {
        assert!(u.tree.get(row).shown);
        assert!(subs.iter().all(|&s| !u.tree.get(s).shown));
    }
}

#[test]
fn selected_image_clear_delivers_before_data_changes_and_repeats_when_empty() {
    let ui = setup(true);
    run(
        &ui,
        r#"
        list:SetHandler('OnSelChanged',function(self,index,double)
          assert(index==0 and double==false and self:GetSelectedIdx()==0)
          assertOldData();onClear(self,index,double)
        end)
        list:DeleteAllDatas()
        assert(table.concat(events,'|')=='0:false')
        list:SetHandler('OnSelChanged',onClear)
        list:DeleteAllDatas();list:DeleteAllDatas()
        assert(table.concat(events,'|')=='0:false|0:false|0:false')
        list:DeleteRows();list:DeleteAllDatas()
        assert(#events==4 and list:GetSelectedIdx()==0)
    "#,
    );
    assert_empty(&ui, 0);
}

#[test]
fn no_selected_image_preserves_selection_and_suppresses_clear_notification() {
    let ui = setup(false);
    run(
        &ui,
        r#"
        assert(list:GetSelectedIdx()==2)
        list:DeleteAllDatas();list:DeleteAllDatas()
        assert(list:GetSelectedIdx()==2 and #events==0)
    "#,
    );
    assert_empty(&ui, 3);
}

#[test]
fn image_guard_does_not_depend_on_visibility_or_enabled_state() {
    let ui = setup(true);
    run(
        &ui,
        r#"
        list:Show(false);list:Enable(false)
        list:SetHandler('OnSelChanged',function(self,index,double)
          assertOldData();onClear(self,index,double)
        end)
        list:DeleteAllDatas()
        assert(list:GetSelectedIdx()==0 and table.concat(events,'|')=='0:false')
    "#,
    );
    assert_empty(&ui, 3);
}

#[test]
fn callback_appended_rows_and_inserted_keys_are_cleared_from_current_state() {
    let ui = setup(true);
    run(
        &ui,
        r#"
        list:SetHandler('OnSelChanged',function(self,index,double)
          assertOldData();onClear(self,index,double)
          self:InsertRows(2,false)
          self:InsertData(303,1,'reentered third')
          self:InsertData(404,1,'reentered fourth')
          self:InsertData('fifth',1,'reentered fifth')
        end)
        list:DeleteAllDatas()
        assert(#events==1 and #list.items==5 and list:GetSelectedIdx()==0)
        assert(list.items[3].subItems[1]:GetText()=='reentered third')
        assert(list.items[4].subItems[1]:GetText()=='reentered fourth')
        assert(list.items[5].subItems[1]:GetText()=='reentered fifth')
    "#,
    );
    assert_empty(&ui, 5);
}

#[test]
fn callback_replaces_rows_and_reselection_survives_later_data_clear() {
    let ui = setup(true);
    let old_row = id(&ui, "list.items[1]");
    run(
        &ui,
        r#"
        list:SetHandler('OnSelChanged',function(self,index,double)
          assertOldData();onClear(self,index,double)
          self:DeleteRows();self:InsertRows(2,false)
          self:InsertData(909,1,'replacement')
          self:Select(1,false)
        end)
        list:DeleteAllDatas()
        assert(#events==1 and list:GetSelectedIdx()==2)
    "#,
    );
    assert_empty(&ui, 2);
    let u = lua::ui(&ui.lua);
    assert!(!u.tree.get(old_row).alive);
    let l = u
        .tree
        .widget(id(&ui, "list"))
        .unwrap()
        .list
        .as_ref()
        .unwrap();
    let image = u.tree.get(l.selected_image.unwrap());
    assert!(image.shown);
    assert_eq!(image.anchors.len(), 2);
    assert!(image.anchors.iter().all(|a| a.target == l.rows[1]));
}

#[test]
fn bounded_nested_clear_notifies_twice_and_outer_clear_consumes_later_insert() {
    let ui = setup(true);
    run(
        &ui,
        r#"
        list:SetHandler('OnSelChanged',function(self,index,double)
          assertOldData();onClear(self,index,double)
          if #events==1 then
            self:DeleteAllDatas()
            self:InsertData(303,1,'after nested clear')
          end
        end)
        list:DeleteAllDatas()
        assert(table.concat(events,'|')=='0:false|0:false')
        assert(list:GetSelectedIdx()==0)
        assert(list.items[1].subItems[1]:GetText()=='after nested clear')
    "#,
    );
    assert_empty(&ui, 3);
}

#[test]
fn receiver_retirement_during_callback_skips_remaining_data_mutation() {
    let ui = setup(true);
    let list = id(&ui, "list");
    ui.lua
        .globals()
        .set(
            "finishRetirement",
            ui.lua
                .create_function(|lua, ()| {
                    lua::tick(lua, 0.);
                    Ok(())
                })
                .unwrap(),
        )
        .unwrap();
    run(
        &ui,
        r#"
        list:SetHandler('OnSelChanged',function(self,index,double)
          assertOldData();onClear(self,index,double)
          self:EnableHidingIsRemove(true);self:Show(false);finishRetirement()
        end)
        list:DeleteAllDatas()
        assert(#events==1)
        list:DeleteAllDatas()
        assert(#events==1)
    "#,
    );
    let u = lua::ui(&ui.lua);
    assert!(!u.tree.get(list).alive);
    let l = u.tree.widget(list).unwrap().list.as_ref().unwrap();
    assert_eq!(
        l.keys,
        [Some(ListKey::Num(101.)), Some(ListKey::Num(202.)), None]
    );
    assert_eq!(l.has_data, [true, true, false]);
    assert!(l.sub_items[0].iter().all(|&s| u.tree.get(s).shown));
}

#[test]
fn hover_is_preserved_and_standalone_clear_selection_still_suppresses_repeats() {
    let ui = setup(true);
    let list = id(&ui, "list");
    let row = id(&ui, "list.items[1]");
    let rect = lua::ui(&ui.lua).tree.rect(row);
    let mut mouse = input::MouseState::default();
    input::update(
        &ui.lua,
        &mut mouse,
        Some((rect.x + 10., rect.y + 10.)),
        false,
        false,
        false,
        0.,
    );
    let (overed, image, anchors) = {
        let u = lua::ui(&ui.lua);
        let l = u.tree.widget(list).unwrap().list.as_ref().unwrap();
        let image = l.overed_image.unwrap();
        assert_eq!(l.overed, 0);
        assert!(u.tree.get(image).shown);
        (l.overed, image, u.tree.get(image).anchors.clone())
    };
    run(
        &ui,
        r#"
        list:DeleteAllDatas();list:ClearSelection();list:ClearSelection()
        assert(#events==1)
        list:Select(0,false);list:ClearSelection();list:ClearSelection()
        assert(table.concat(events,'|')=='0:false|0:false')
    "#,
    );
    assert_empty(&ui, 3);
    let u = lua::ui(&ui.lua);
    let l = u.tree.widget(list).unwrap().list.as_ref().unwrap();
    assert_eq!(l.overed, overed);
    assert!(u.tree.get(image).shown);
    assert_eq!(u.tree.get(image).anchors, anchors);
}

#[test]
fn unchanged_original_dungeon_fill_and_scroll_wrapper_clear_stale_key_and_highlight() {
    let mut ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    fake::install(&mut ui, "en_us", &fake::default_worlds());
    fake::install_ingame(&mut ui);
    ui.run_stage(aa_ui::Stage::World);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    let handlers: mlua::Table = lua::reg(&ui.lua).handlers.raw_get(0).unwrap();
    let callbacks: mlua::Table = handlers.raw_get("__ev_SYS_INDUN_STAT_UPDATED").unwrap();
    assert_eq!(callbacks.raw_len(), 1);
    let event: mlua::Function = callbacks.raw_get(1).unwrap();
    ui.lua.globals().set("originalDungeonEvent", event).unwrap();
    run(
        &ui,
        r#"
        X2.GetSystemDungeonStateInfo=function() return dungeonInfos end
        X2.GetSystemDungeonName=function() return 'Clear regression dungeon' end
        X2.EnterSystemDungeon=function() error('unexpected dungeon entry') end
        dungeonInfos={{instance_id=701,channel_index=1,members=1,current=1,restrict=10}}
        originalDungeonEvent()
        window=channelSelectWnd
        assert(window~=nil)
        frame=window.scrollListCtrl;list=frame.listCtrl
        list:Select(0)
        assert(frame:GetSelectedDataKey()==701 and frame:GetSelectedDataIndex()==1)
        assert(window.leftButton:IsEnabled())
        dungeonInfos={{instance_id=909,channel_index=2,members=2,current=2,restrict=10}}
        window:FillData()
        assert(list:GetSelectedIdx()==0)
        assert(frame:GetSelectedDataKey()==nil and frame:GetSelectedDataIndex()==0)
        assert(not window.leftButton:IsEnabled())
        assert(frame:GetDataByViewIndex(1,1)==string.format('%s - %d',GetCommonText('dimension'),2))
        assert(frame:GetDataCount()==1)
    "#,
    );
    let list = id(&ui, "list");
    {
        let u = lua::ui(&ui.lua);
        let l = u.tree.widget(list).unwrap().list.as_ref().unwrap();
        assert!(!u.tree.get(l.selected_image.unwrap()).shown);
        assert_eq!(l.keys[0], Some(ListKey::Num(1.)));
        assert_eq!(
            l.has_data,
            [
                true, false, false, false, false, false, false, false, false, false
            ]
        );
    }
    run(
        &ui,
        r#"
        list:Select(0)
        assert(frame:GetSelectedDataKey()==909 and window.leftButton:IsEnabled())
        dungeonInfos=nil;window:FillData()
        assert(list:GetSelectedIdx()==0 and frame:GetSelectedDataIndex()==0)
        assert(frame:GetSelectedDataKey()==nil and frame:GetDataCount()==0)
        assert(not window.leftButton:IsEnabled())
    "#,
    );
    assert_empty(&ui, 10);
    let u = lua::ui(&ui.lua);
    let l = u.tree.widget(list).unwrap().list.as_ref().unwrap();
    assert!(!u.tree.get(l.selected_image.unwrap()).shown);
}
