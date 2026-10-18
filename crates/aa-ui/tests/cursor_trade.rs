//! Native cursor kind names and the original trade drag/click guards.

#[path = "../examples/fake/mod.rs"]
#[allow(dead_code)]
mod fake;

use aa_ui::UiRuntime;

#[test]
fn native_names_preserve_internal_kinds_and_do_not_guess_item_stackability_from_count() {
    let ui = UiRuntime::new("en_us", 1920.0, 1080.0).unwrap();
    ui.exec(
        "@cursor_names",
        r#"
        local function check(held,native)
            X2Cursor:SetHeld(held)
            assert(X2Cursor:GetCursorInfo()==native)
            assert(X2Cursor:GetHeld()==held)
        end
        check({kind='bag',slot=3,count=1,uid='9007199254740997'},'Item')
        check({kind='bag',slot=3,count=1,cursorInfo='Item_Id'},'Item_Id')
        check({kind='bag',slot=3,count=1,cursorInfo='Item'},'Item')
        check({kind='equip'},'Item_Id')
        check({kind='skill'},'Spell')
        check({kind='item'},'Item')
        check({kind='action',what='skill'},'Spell')
        check({kind='action',what='item'},'Item')
        check({kind='macro'},'Macro')
        check({kind='unknown'},nil)
        check({kind='bag',cursorInfo='guessed'},nil)
        X2Cursor:SetCursorImage('ui/common/money_window.dds',0,0)
        assert(X2Cursor:GetCursorInfo()==nil and X2Cursor:GetHeld().kind=='image')
        X2Cursor:ClearCursor(); assert(X2Cursor:GetCursorInfo()==nil)
    "#,
    )
    .unwrap();
}

#[test]
fn original_trade_receives_stackable_bag_drag_and_unique_item_click_without_handler_overrides() {
    let mut ui = UiRuntime::new("en_us", 1920.0, 1080.0).unwrap();
    fake::install(&mut ui, "en_us", &fake::default_worlds());
    fake::install_ingame(&mut ui);
    ui.run_stage(aa_ui::Stage::World);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    ui.exec(
        "@original_trade_cursor",
        r#"
        offers={}; locked=false
        X2Trade.IsTradeLocked=function() return locked end
        X2Trade.PutupTradeItem=function(self,slot,count)
            local held=X2Cursor:GetHeld()
            table.insert(offers,{slot=slot,count=count,uid=held.uid,kind=held.kind})
        end
        trade.window:Show(true)
        X2Cursor:SetHeld({kind='bag',cursorInfo='Item',slot=7,count=3,uid='9007199254740997'})
        trade.window.myWindow.slotWindow:OnDragReceive('LeftButton')
        assert(#offers==1 and offers[1].slot==7 and offers[1].count==3)
        assert(offers[1].uid=='9007199254740997' and offers[1].kind=='bag')
        assert(X2Cursor:GetCursorInfo()==nil)
        X2Cursor:SetHeld({kind='bag',cursorInfo='Item_Id',slot=8,count=1,uid='9007199254740999'})
        trade.window.myWindow.slotWindow:OnDragReceive('LeftButton')
        assert(#offers==1 and X2Cursor:GetCursorInfo()=='Item_Id')
        local item=trade.window.myWindow.slotWindow.item[1]
        item:OnClickProc('LeftButton')
        assert(#offers==2 and offers[2].slot==8 and offers[2].count==1)
        assert(offers[2].uid=='9007199254740999' and X2Cursor:GetCursorInfo()==nil)
        locked=true
        X2Cursor:SetHeld({kind='bag',cursorInfo='Item',slot=9,count=2})
        trade.window.myWindow.slotWindow:OnDragReceive('LeftButton')
        assert(#offers==2 and X2Cursor:GetCursorInfo()=='Item')
    "#,
    )
    .unwrap();
}
