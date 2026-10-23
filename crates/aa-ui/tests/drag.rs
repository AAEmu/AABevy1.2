//! Drag and drop: `RegisterForDrag` sources, `OnDragStart` / `OnDragReceive` / `OnDragStop`, the native slot
//! pick up and drop through the `AAUI` hooks, and what the pointer holds (`X2Cursor`).

use aa_ui::UiRuntime;
use aa_ui::input::{MouseState, update};

fn runtime() -> UiRuntime {
    UiRuntime::new("en_us", 1920.0, 1080.0).expect("runtime")
}

fn eval<T: mlua::FromLua>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().expect(code)
}

struct Pointer {
    mouse: MouseState,
}

impl Pointer {
    fn new() -> Pointer {
        Pointer {
            mouse: MouseState::default(),
        }
    }

    fn at(&mut self, ui: &UiRuntime, x: f32, y: f32) {
        update(
            &ui.lua,
            &mut self.mouse,
            Some((x, y)),
            false,
            false,
            false,
            0.0,
        );
    }

    fn press(&mut self, ui: &UiRuntime, x: f32, y: f32) {
        update(
            &ui.lua,
            &mut self.mouse,
            Some((x, y)),
            true,
            false,
            false,
            0.0,
        );
    }

    fn release(&mut self, ui: &UiRuntime, x: f32, y: f32) {
        update(
            &ui.lua,
            &mut self.mouse,
            Some((x, y)),
            false,
            true,
            false,
            0.0,
        );
    }

    fn right_click(&mut self, ui: &UiRuntime, x: f32, y: f32) {
        update(
            &ui.lua,
            &mut self.mouse,
            Some((x, y)),
            false,
            false,
            true,
            0.0,
        );
    }

    /// Press at `from`, move to `to` in a few steps, release.
    fn drag(&mut self, ui: &UiRuntime, from: (f32, f32), to: (f32, f32)) {
        self.press(ui, from.0, from.1);
        for i in 1..=4 {
            let t = i as f32 / 4.0;
            self.at(
                ui,
                from.0 + (to.0 - from.0) * t,
                from.1 + (to.1 - from.1) * t,
            );
        }
        self.release(ui, to.0, to.1);
    }
}

const TWO_BUTTONS: &str = r#"
    log = {}
    local function button(name, x)
      local b = UIParent:CreateWidget("button", name, "UIParent")
      b:AddAnchor("TOPLEFT", "UIParent", x, 100)
      b:SetExtent(40, 40)
      b:Show(true)
      b:RegisterForDrag("LeftButton")
      b:SetHandler("OnClick", function(self, btn) log[#log + 1] = name .. ":click:" .. btn end)
      b:SetHandler("OnDragStart", function(self, btn) log[#log + 1] = name .. ":start:" .. btn end)
      b:SetHandler("OnDragStop", function(self, btn) log[#log + 1] = name .. ":stop:" .. btn end)
      b:SetHandler("OnDragReceive", function(self, btn) log[#log + 1] = name .. ":receive:" .. btn end)
      return b
    end
    a = button("a", 100)
    b = button("b", 300)
"#;

#[test]
fn dragging_between_buttons_sends_start_receive_stop_and_no_click() {
    let ui = runtime();
    ui.exec("@setup", TWO_BUTTONS).unwrap();
    let mut p = Pointer::new();
    // nothing is held: the drop target gets no OnDragReceive (the scripts would pick up what they are not given)
    p.drag(&ui, (120.0, 120.0), (320.0, 120.0));
    assert_eq!(
        eval::<String>(&ui, "table.concat(log, ' ')"),
        "a:start:LeftButton a:stop:LeftButton"
    );
    // holding something (a pick up in OnDragStart): the button under the release receives it
    ui.exec("@hold", "log = {}; a:SetHandler('OnDragStart', function() X2Cursor:SetHeld({ kind = 'thing' }) end)").unwrap();
    p.drag(&ui, (120.0, 120.0), (320.0, 120.0));
    assert_eq!(
        eval::<String>(&ui, "table.concat(log, ' ')"),
        "b:receive:LeftButton a:stop:LeftButton"
    );
    assert!(
        !ui.holding(),
        "nothing took it and the app has no hook: let go of it"
    );
    // with the hook, the app sees what is left
    ui.exec("@hook", "log = {}; AAUI.DropOnNothing = function(overUi) log[#log + 1] = 'nothing:' .. tostring(overUi) end").unwrap();
    p.drag(&ui, (120.0, 120.0), (700.0, 500.0));
    assert_eq!(
        eval::<String>(&ui, "table.concat(log, ' ')"),
        "a:stop:LeftButton nothing:false"
    );
    assert!(ui.holding(), "the hook decides (it did not clear)");
    ui.clear_cursor();
    assert!(!ui.holding());
}

#[test]
fn a_press_that_barely_moves_is_a_click() {
    let ui = runtime();
    ui.exec("@setup", TWO_BUTTONS).unwrap();
    let mut p = Pointer::new();
    p.drag(&ui, (120.0, 120.0), (122.0, 121.0));
    assert_eq!(
        eval::<String>(&ui, "table.concat(log, ' ')"),
        "a:click:LeftButton"
    );
}

#[test]
fn a_child_of_a_source_drags_it_and_other_widgets_do_not_drag() {
    let ui = runtime();
    ui.exec(
        "@setup",
        r#"
        log = {}
        win = UIParent:CreateWidget("window", "dragWin", "UIParent")
        win:AddAnchor("TOPLEFT", "UIParent", 100, 100)
        win:SetExtent(200, 100)
        win:Show(true)
        win:RegisterForDrag("LeftButton")
        win:SetHandler("OnDragStart", function(self, btn) log[#log + 1] = "win:start" end)
        win:SetHandler("OnDragStop", function(self, btn) log[#log + 1] = "win:stop" end)
        child = win:CreateChildWidget("button", "dragChild", 0, true)
        child:AddAnchor("TOPLEFT", win, 10, 10)
        child:SetExtent(50, 20)
        loose = UIParent:CreateWidget("button", "looseBtn", "UIParent")
        loose:AddAnchor("TOPLEFT", "UIParent", 500, 100)
        loose:SetExtent(50, 20)
        loose:Show(true)
        loose:SetHandler("OnDragStart", function() log[#log + 1] = "loose:start" end)
        "#,
    )
    .unwrap();
    let mut p = Pointer::new();
    p.drag(&ui, (120.0, 115.0), (220.0, 215.0));
    p.drag(&ui, (510.0, 110.0), (610.0, 210.0));
    assert_eq!(
        eval::<String>(&ui, "table.concat(log, ' ')"),
        "win:start win:stop"
    );
}

const TWO_SLOTS: &str = r#"
    ISLOT = ISLOT_ACTION
    log = {}
    bar = { [1] = { id = 11, icon = "ui/icon/icon_item_0392.dds" } }
    AAUI.SlotContent = function(ty, idx)
      local s = bar[idx]
      if s == nil then return nil end
      return { type = "skill", id = s.id, icon = s.icon }
    end
    AAUI.PickupSlot = function(info)
      log[#log + 1] = "pickup:" .. info.kind .. ":" .. info.slotType .. ":" .. info.slotIdx
      X2Cursor:SetHeld({ kind = "action", from = info.slotIdx, icon = info.content.icon })
    end
    AAUI.DropSlot = function(info)
      local held = X2Cursor:GetHeld()
      log[#log + 1] = "drop:" .. info.slotIdx .. ":" .. tostring(info.empty) .. ":" .. held.kind .. ":" .. held.from
      X2Cursor:ClearCursor()
    end
    AAUI.DropOnNothing = function(overUi)
      log[#log + 1] = "nothing:" .. tostring(overUi)
      X2Cursor:ClearCursor()
    end
    AAUI.UseSlot = function(ty, idx, btn) log[#log + 1] = "use:" .. idx .. ":" .. btn end
    local function slot(name, x, idx)
      local s = UIParent:CreateWidget("slot", name, "UIParent")
      s:AddAnchor("TOPLEFT", "UIParent", x, 400)
      s:SetExtent(42, 42)
      s:Show(true)
      s:RegisterForDrag("LeftButton")
      s:SetHandler("OnClick", function() end) -- like CreateActionSlot: it takes the mouse
      s:EstablishSlot(ISLOT_ACTION, idx)
      return s
    end
    s1 = slot("slotOne", 100, 1)
    s2 = slot("slotTwo", 200, 2)
"#;

#[test]
fn a_bound_slot_is_picked_up_natively_and_dropped_on_another_slot() {
    let ui = runtime();
    ui.exec("@setup", TWO_SLOTS).unwrap();
    let mut p = Pointer::new();
    p.drag(&ui, (120.0, 420.0), (220.0, 420.0));
    assert_eq!(
        eval::<String>(&ui, "table.concat(log, ' ')"),
        "pickup:slot:254:1 drop:2:true:action:1"
    );
    assert!(!ui.holding());
    // an empty slot has nothing to pick up
    ui.exec("@reset", "log = {}").unwrap();
    p.drag(&ui, (220.0, 420.0), (120.0, 420.0));
    assert_eq!(eval::<String>(&ui, "table.concat(log, ' ')"), "");
}

#[test]
fn dropping_a_slot_on_nothing_asks_the_app() {
    let ui = runtime();
    ui.exec("@setup", TWO_SLOTS).unwrap();
    let mut p = Pointer::new();
    // over the bare world, and over a widget that takes no drops
    p.drag(&ui, (120.0, 420.0), (900.0, 700.0));
    assert_eq!(
        eval::<String>(&ui, "table.concat(log, ' ')"),
        "pickup:slot:254:1 nothing:false"
    );
    ui.exec(
        "@label",
        "log = {}; lab = UIParent:CreateWidget('button', 'plainBtn', 'UIParent'); lab:AddAnchor('TOPLEFT', 'UIParent', 800, 600); lab:SetExtent(60, 30); lab:Show(true)",
    )
    .unwrap();
    p.drag(&ui, (120.0, 420.0), (820.0, 610.0));
    assert_eq!(
        eval::<String>(&ui, "table.concat(log, ' ')"),
        "pickup:slot:254:1 nothing:true"
    );
}

#[test]
fn a_click_on_a_slot_uses_it_or_drops_what_is_held() {
    let ui = runtime();
    ui.exec("@setup", TWO_SLOTS).unwrap();
    let mut p = Pointer::new();
    p.press(&ui, 120.0, 420.0);
    p.release(&ui, 120.0, 420.0);
    assert_eq!(
        eval::<String>(&ui, "table.concat(log, ' ')"),
        "use:1:LeftButton"
    );
    // holding something: the click places it, the slot is not used
    ui.exec(
        "@hold",
        "log = {}; X2Cursor:SetHeld({ kind = 'action', from = 1 })",
    )
    .unwrap();
    p.press(&ui, 220.0, 420.0);
    p.release(&ui, 220.0, 420.0);
    assert_eq!(
        eval::<String>(&ui, "table.concat(log, ' ')"),
        "drop:2:true:action:1"
    );
}

#[test]
fn a_right_click_puts_the_held_thing_back_and_the_cursor_image_is_not_droppable() {
    let ui = runtime();
    ui.exec("@setup", TWO_SLOTS).unwrap();
    let mut p = Pointer::new();
    ui.exec(
        "@hold",
        "log = {}; X2Cursor:SetHeld({ kind = 'bag', slot = 4, count = 7 })",
    )
    .unwrap();
    assert_eq!(eval::<String>(&ui, "X2Cursor:GetCursorInfo()"), "Item");
    assert_eq!(
        eval::<i64>(&ui, "X2Cursor:GetCursorPickedBagItemIndex()"),
        4
    );
    assert_eq!(
        eval::<i64>(&ui, "X2Cursor:GetCursorPickedBagItemAmount()"),
        7
    );
    p.right_click(&ui, 220.0, 420.0);
    assert!(!ui.holding());
    assert!(eval::<bool>(&ui, "X2Cursor:GetCursorInfo() == nil"));
    assert_eq!(
        eval::<String>(&ui, "table.concat(log, ' ')"),
        "",
        "the right click only cleared the cursor"
    );
    // a cursor image (a window being moved) is held but never dropped
    ui.exec(
        "@window",
        r#"
        log = {}
        mover = UIParent:CreateWidget("window", "moverWin", "UIParent")
        mover:AddAnchor("TOPLEFT", "UIParent", 500, 500)
        mover:SetExtent(100, 60)
        mover:Show(true)
        mover:RegisterForDrag("LeftButton")
        mover:SetHandler("OnDragStart", function() X2Cursor:SetCursorImage("ui/cursor/cursor_move.dds", 0, 0) end)
        mover:SetHandler("OnDragStop", function() X2Cursor:ClearCursor() end)
        "#,
    )
    .unwrap();
    p.press(&ui, 520.0, 520.0);
    p.at(&ui, 400.0, 480.0);
    assert!(eval::<bool>(&ui, "X2Cursor:GetCursorInfo() == nil"));
    assert!(!ui.holding());
    p.at(&ui, 220.0, 420.0);
    p.release(&ui, 220.0, 420.0);
    assert_eq!(
        eval::<String>(&ui, "table.concat(log, ' ')"),
        "",
        "no drop with an image cursor"
    );
    assert!(eval::<bool>(&ui, "X2Cursor:GetCursorInfo() == nil"));
}

#[test]
fn a_window_that_comes_up_takes_the_mouse_before_the_ones_below() {
    let ui = runtime();
    // the second window (a tracker the scripts show at once) covers the first; showing the first brings it up
    ui.exec(
        "@setup",
        r#"
        log = {}
        local function window(name)
          local w = UIParent:CreateWidget("window", name, "UIParent")
          w:AddAnchor("TOPLEFT", "UIParent", 100, 100)
          w:SetExtent(200, 200)
          w:SetHandler("OnClick", function() log[#log + 1] = name end)
          return w
        end
        bag = window("bag")
        tracker = window("tracker")
        tracker:Show(true)
        bag:Show(true)
    "#,
    )
    .unwrap();
    let mut p = Pointer::new();
    p.press(&ui, 150.0, 150.0);
    p.release(&ui, 150.0, 150.0);
    assert_eq!(eval::<String>(&ui, "table.concat(log, ' ')"), "bag");
}

#[test]
fn a_receive_only_widget_and_a_native_slot_take_the_pointer() {
    let ui = runtime();
    ui.exec("@setup", TWO_SLOTS).unwrap();
    ui.exec(
        "@target",
        r#"
        target = UIParent:CreateWidget("emptywidget", "receiveOnly", "UIParent")
        target:AddAnchor("TOPLEFT", "UIParent", 400, 400)
        target:SetExtent(42, 42)
        target:Show(true)
        target:SetHandler("OnDragReceive", function()
          log[#log + 1] = "receive"
          X2Cursor:ClearCursor()
        end)
        s1:ReleaseHandler("OnClick")
        "#,
    )
    .unwrap();
    let mut p = Pointer::new();
    p.drag(&ui, (120.0, 420.0), (420.0, 420.0));
    assert_eq!(
        eval::<String>(&ui, "table.concat(log, ' ')"),
        "pickup:slot:254:1 receive"
    );
    ui.exec("@reset", "log = {}").unwrap();
    p.press(&ui, 120.0, 420.0);
    p.release(&ui, 120.0, 420.0);
    assert_eq!(
        eval::<String>(&ui, "table.concat(log, ' ')"),
        "use:1:LeftButton"
    );
}

#[test]
fn disabled_slots_neither_pick_up_nor_accept_a_drop() {
    let ui = runtime();
    ui.exec("@setup", TWO_SLOTS).unwrap();
    ui.exec("@disable", "s1:Enable(false)").unwrap();
    let mut p = Pointer::new();
    p.drag(&ui, (120.0, 420.0), (220.0, 420.0));
    assert_eq!(eval::<String>(&ui, "table.concat(log, ' ')"), "");
    ui.exec("@disable_target", "s1:Enable(true); s2:Enable(false)")
        .unwrap();
    p.drag(&ui, (120.0, 420.0), (220.0, 420.0));
    assert_eq!(
        eval::<String>(&ui, "table.concat(log, ' ')"),
        "pickup:slot:254:1 nothing:true"
    );
    ui.exec(
        "@hold",
        "log = {}; X2Cursor:SetHeld({kind = 'action', from = 1})",
    )
    .unwrap();
    p.press(&ui, 220.0, 420.0);
    p.release(&ui, 220.0, 420.0);
    assert_eq!(eval::<String>(&ui, "table.concat(log, ' ')"), "");
    assert!(ui.holding());
}

#[test]
fn a_script_that_changes_the_hand_consumes_the_slot_click() {
    let ui = runtime();
    ui.exec("@setup", TWO_SLOTS).unwrap();
    ui.exec(
        "@script",
        r#"
        s1:SetHandler("OnClick", function()
          log[#log + 1] = "script"
          if X2Cursor:GetHeld() then X2Cursor:ClearCursor()
          else X2Cursor:SetHeld({kind = 'action', from = 1}) end
        end)
        "#,
    )
    .unwrap();
    let mut p = Pointer::new();
    p.press(&ui, 120.0, 420.0);
    p.release(&ui, 120.0, 420.0);
    assert!(
        ui.holding(),
        "the native drop must not place what this click just picked up"
    );
    p.press(&ui, 120.0, 420.0);
    p.release(&ui, 120.0, 420.0);
    assert!(!ui.holding());
    assert_eq!(
        eval::<String>(&ui, "table.concat(log, ' ')"),
        "script script"
    );
}

#[test]
fn title_bar_movement_keeps_the_window_size_and_releases_outside_the_window() {
    let ui = runtime();
    ui.exec(
        "@titlebar",
        r#"
        win = UIParent:CreateWidget("window", "movingWindow", "UIParent")
        win:AddAnchor("TOPLEFT", "UIParent", 100, 100)
        win:AddAnchor("BOTTOMRIGHT", "UIParent", "TOPLEFT", 300, 200)
        win:Show(true)
        title = win:CreateChildWidget("emptywidget", "title", 0, true)
        title:AddAnchor("TOPLEFT", win, 0, 0)
        title:SetExtent(200, 30)
        title:SetHandler("OnMouseDown", function()
          X2Cursor:SetCursorImage("ui/cursor/cursor_move.dds", 0, 0)
          win:StartMoving()
        end)
        title:SetHandler("OnMouseUp", function() win:StopMovingOrSizing() end)
        "#,
    )
    .unwrap();
    let mut p = Pointer::new();
    p.press(&ui, 120.0, 115.0);
    p.at(&ui, 170.0, 190.0);
    assert_eq!(eval::<f32>(&ui, "win:GetOffset()"), 150.0);
    assert_eq!(eval::<f32>(&ui, "select(2, win:GetOffset())"), 175.0);
    assert_eq!(eval::<f32>(&ui, "win:GetWidth()"), 200.0);
    assert_eq!(eval::<f32>(&ui, "win:GetHeight()"), 100.0);
    p.at(&ui, 120.0, 115.0);
    assert_eq!(eval::<f32>(&ui, "win:GetOffset()"), 100.0);
    p.at(&ui, 170.0, 190.0);
    // Release with no cursor position (outside the OS window): the title gets no OnMouseUp.
    update(&ui.lua, &mut p.mouse, None, false, true, false, 0.0);
    assert!(eval::<bool>(&ui, "X2Cursor:GetCursorInfo() == nil"));
    p.at(&ui, 500.0, 500.0);
    assert_eq!(eval::<f32>(&ui, "win:GetOffset()"), 150.0);
}

#[test]
fn a_title_bar_stop_notifies_once_even_without_motion_and_release_does_not_duplicate_it() {
    let ui = runtime();
    ui.exec(
        "@moved",
        r#"
        win = UIParent:CreateWidget("window", "savedWindow", "UIParent")
        win:AddAnchor("TOPLEFT", "UIParent", 100, 100)
        win:AddAnchor("BOTTOMRIGHT", "UIParent", "TOPLEFT", 300, 200)
        win:Show(true)
        moved = 0
        win:SetHandler("OnMovedPosition", function() moved = moved + 1 end)
        title = win:CreateChildWidget("emptywidget", "title", 0, true)
        title:AddAnchor("TOPLEFT", win, 0, 0)
        title:SetExtent(200, 30)
        title:SetHandler("OnMouseDown", function() win:StartMoving() end)
        title:SetHandler("OnMouseUp", function() win:StopMovingOrSizing() end)
        "#,
    )
    .unwrap();
    let mut p = Pointer::new();
    // Native StopMovingOrSizing notifies even when the title bar moved nothing.
    p.press(&ui, 120.0, 115.0);
    p.release(&ui, 120.0, 115.0);
    assert_eq!(eval::<i32>(&ui, "moved"), 1);
    p.press(&ui, 120.0, 115.0);
    p.at(&ui, 170.0, 190.0);
    assert_eq!(eval::<i32>(&ui, "moved"), 1);
    p.release(&ui, 170.0, 190.0);
    assert_eq!(eval::<i32>(&ui, "moved"), 2);
    assert_eq!(eval::<f32>(&ui, "win:GetOffset()"), 150.0);
    p.release(&ui, 900.0, 900.0);
    p.press(&ui, 900.0, 900.0);
    p.release(&ui, 900.0, 900.0);
    assert_eq!(eval::<i32>(&ui, "moved"), 2);
}

fn moving_windows(ui: &UiRuntime) {
    ui.exec(
        "@moving_windows",
        r#"
        log={}
        win=UIParent:CreateWidget('window','moveReceiver','UIParent')
        win:SetExtent(200,100);win:AddAnchor('TOPLEFT',UIParent,100,100);win:Show(true)
        other=UIParent:CreateWidget('window','otherMoveReceiver','UIParent')
        other:SetExtent(200,100);other:AddAnchor('TOPLEFT',UIParent,600,100);other:Show(true)
        local function moved(self,...)
            assert(select('#',...)==0)
            assert(X2Cursor:GetHeld()==nil)
            table.insert(log,self==win and 'win' or 'other')
        end
        win:SetHandler('OnMovedPosition',moved);other:SetHandler('OnMovedPosition',moved)
        "#,
    )
    .unwrap();
}

#[test]
fn explicit_stop_notifies_its_receiver_without_capture_and_cancels_a_different_capture() {
    let ui = runtime();
    moving_windows(&ui);
    ui.exec(
        "@stop_without_capture",
        "assert(select('#',win:StopMovingOrSizing())==0);win:StopMovingOrSizing()",
    )
    .unwrap();
    assert_eq!(eval::<String>(&ui, "table.concat(log,',')"), "win,win");
    ui.exec("@other_capture", "other:StartMoving();X2Cursor:SetCursorImage('move',0,0);win:Show(false);win:StopMovingOrSizing()").unwrap();
    let mut p = Pointer::new();
    p.at(&ui, 40., 50.);
    p.release(&ui, 40., 50.);
    assert_eq!(eval::<String>(&ui, "table.concat(log,',')"), "win,win,win");
    assert_eq!(eval::<f32>(&ui, "other:GetOffset()"), 600.);
    assert!(!eval::<bool>(&ui, "win:IsVisible()"));
}

#[test]
fn pointer_release_notifies_final_bounds_once_outside_the_os_window_including_returned_motion() {
    let ui = runtime();
    moving_windows(&ui);
    let mut p = Pointer::new();
    p.at(&ui, 120., 115.);
    ui.exec(
        "@capture",
        "win:StartMoving();X2Cursor:SetCursorImage('move',0,0)",
    )
    .unwrap();
    p.at(&ui, 170., 190.);
    assert_eq!(eval::<String>(&ui, "table.concat(log,',')"), "");
    update(&ui.lua, &mut p.mouse, None, false, true, false, 0.);
    assert_eq!(eval::<String>(&ui, "table.concat(log,',')"), "win");
    assert_eq!(eval::<f32>(&ui, "win:GetOffset()"), 150.);
    p.at(&ui, 800., 700.);
    assert_eq!(eval::<f32>(&ui, "win:GetOffset()"), 150.);
    update(&ui.lua, &mut p.mouse, None, false, true, false, 0.);
    assert_eq!(eval::<String>(&ui, "table.concat(log,',')"), "win");
    ui.exec("@capture_return", "win:StartMoving()").unwrap();
    p.at(&ui, 810., 710.);
    p.at(&ui, 800., 700.);
    p.release(&ui, 800., 700.);
    assert_eq!(eval::<String>(&ui, "table.concat(log,',')"), "win,win");
    assert_eq!(eval::<f32>(&ui, "win:GetOffset()"), 150.);
}

#[test]
fn moved_callback_can_read_bounds_and_start_another_capture_without_borrowing_or_losing_it() {
    let ui = runtime();
    moving_windows(&ui);
    ui.exec(
        "@reentrant_move",
        r#"
        win:SetHandler('OnMovedPosition',function(self,...)
            assert(self==win and select('#',...)==0 and X2Cursor:GetHeld()==nil)
            local x,y=self:GetEffectiveOffset();assert(x==150 and y==175)
            table.insert(log,'win')
            self:RemoveAllAnchors();self:AddAnchor('TOPLEFT',UIParent,200,200)
            other:StartMoving();X2Cursor:SetCursorImage('next_move',0,0)
        end)
    "#,
    )
    .unwrap();
    let mut p = Pointer::new();
    p.at(&ui, 120., 115.);
    ui.exec(
        "@capture",
        "win:StartMoving();X2Cursor:SetCursorImage('move',0,0)",
    )
    .unwrap();
    p.at(&ui, 170., 190.);
    update(&ui.lua, &mut p.mouse, None, false, true, false, 0.);
    assert_eq!(eval::<String>(&ui, "table.concat(log,',')"), "win");
    assert_eq!(eval::<String>(&ui, "X2Cursor:GetHeld().icon"), "next_move");
    p.at(&ui, 180., 200.);
    assert_eq!(eval::<f32>(&ui, "other:GetOffset()"), 610.);
    p.release(&ui, 180., 200.);
    assert_eq!(eval::<String>(&ui, "table.concat(log,',')"), "win,other");
    assert_eq!(eval::<f32>(&ui, "win:GetOffset()"), 200.);
}

#[test]
fn moved_callback_can_retire_a_chat_window_and_retirement_before_release_does_not_notify() {
    let ui = runtime();
    ui.exec(
        "@retire_moving_chat",
        r#"
        moved=0
        win=UIParent:CreateWidget('chatwindow','retiringMove','UIParent')
        win:SetExtent(300,180);win:AddAnchor('TOPLEFT',UIParent,100,100);win:Show(true)
        win:SetHandler('OnMovedPosition',function(self)
            moved=moved+1;self:EnableHidingIsRemove(true);self:Show(false)
        end)
        win:StartMoving()
    "#,
    )
    .unwrap();
    let mut p = Pointer::new();
    p.at(&ui, 10., 10.);
    update(&ui.lua, &mut p.mouse, None, false, true, false, 0.);
    aa_ui::lua::tick(&ui.lua, 0.);
    assert_eq!(eval::<i32>(&ui, "moved"), 1);
    p.release(&ui, 20., 20.);
    assert_eq!(eval::<i32>(&ui, "moved"), 1);
    ui.exec(
        "@retire_before_release",
        r#"
        win=UIParent:CreateWidget('chatwindow','retiredBeforeRelease','UIParent')
        win:SetExtent(300,180);win:Show(true)
        win:SetHandler('OnMovedPosition',function() moved=moved+1 end)
        win:StartMoving();X2Cursor:SetCursorImage('move',0,0)
    "#,
    )
    .unwrap();
    assert_eq!(aa_ui::lua::chat::reset_windows(&ui.lua), 1);
    update(&ui.lua, &mut p.mouse, None, false, true, false, 0.);
    assert_eq!(eval::<i32>(&ui, "moved"), 1);
    assert!(eval::<bool>(&ui, "X2Cursor:GetHeld()==nil"));
}

#[test]
fn hidden_resize_cancellation_notifies_once_and_handler_failure_does_not_retry_on_release() {
    let ui = runtime();
    moving_windows(&ui);
    let mut p = Pointer::new();
    p.at(&ui, 300., 150.);
    ui.exec("@sizing", "win:StartSizing('RIGHT')").unwrap();
    p.at(&ui, 330., 150.);
    ui.exec("@hide", "win:Show(false)").unwrap();
    p.at(&ui, 340., 150.);
    assert_eq!(eval::<String>(&ui, "table.concat(log,',')"), "win");
    assert_eq!(eval::<f32>(&ui, "win:GetWidth()"), 230.);
    p.release(&ui, 350., 150.);
    assert_eq!(eval::<String>(&ui, "table.concat(log,',')"), "win");
    ui.exec("@failing_callback", r#"
        win:Show(true);calls=0
        win:SetHandler('OnMovedPosition',function() calls=calls+1;error('expected callback failure') end)
        win:StartMoving()
    "#).unwrap();
    p.at(&ui, 360., 150.);
    p.release(&ui, 360., 150.);
    p.release(&ui, 370., 150.);
    assert_eq!(eval::<i32>(&ui, "calls"), 1);
}
