//! Original tab layout's SSE floor, clamp order and geometry callbacks.

#[path = "../examples/fake/auto_clip.rs"]
mod auto_clip;
#[path = "../examples/fake/chat_tab_layout.rs"]
mod chat_tab_layout;
#[path = "../examples/fake/mod.rs"]
#[allow(dead_code)]
mod fake;

use aa_ui::{UiRuntime, lua, render};

fn run(ui: &UiRuntime, code: &str) {
    ui.exec("@chat_tab_layout_test", code).unwrap();
}

fn id(ui: &UiRuntime, name: &str) -> u32 {
    lua::value_id(&ui.lua, &ui.lua.load(name).eval().unwrap()).unwrap()
}

fn tabs(ui: &UiRuntime) -> Vec<lua::chat::ChatTab> {
    lua::ui(&ui.lua)
        .tree
        .widget(id(ui, "window"))
        .unwrap()
        .chat
        .as_ref()
        .unwrap()
        .tabs
        .clone()
}

fn widths(ui: &UiRuntime) -> Vec<f32> {
    tabs(ui)
        .iter()
        .map(|tab| lua::ui(&ui.lua).tree.size(tab.button).0)
        .collect()
}

fn setup() -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    run(
        &ui,
        r#"
        AAUI={ChatTabs=function() return {{id=0,name='All'},{id=1,name='Party'},{id=2,name='Trade'}} end}
        window=UIParent:CreateWidget('chatwindow','nativeLayout','UIParent')
        window:SetExtent(300,150);window:AddAnchor('TOPLEFT',UIParent,100,100)
        window:GetAddButton():SetExtent(22,22)
        window:SetChatWindowId(0)
    "#,
    );
    ui
}

#[test]
fn native_defaults_fixed_mode_and_explicit_auto_toggle() {
    let ui = setup();
    assert_eq!(widths(&ui), [50.; 3]);
    run(
        &ui,
        r#"
        window:SetTabWidth(500);window:SetTabAreaInset(0,0,10,0)
        window:UseAddTabButton(true)
    "#,
    );
    assert_eq!(widths(&ui), [500.; 3]); // Fixed mode deliberately ignores available width/min.
    run(&ui, "window:UseAutoResizingTabButtonMode(true)");
    assert_eq!(widths(&ui), [89.; 3]); // Actual native oracle: floor((300-22-10)/3).
    run(&ui, "window:UseAddTabButton(false)");
    assert_eq!(widths(&ui), [96.; 3]);
    run(
        &ui,
        "window:UseAutoResizingTabButtonMode(false);window:SetWidth(80)",
    );
    assert_eq!(widths(&ui), [500.; 3]);
}

#[test]
fn native_minimum_first_handles_reversed_limits_and_negative_setters() {
    let ui = setup();
    run(
        &ui,
        r#"
        window:UseAutoResizingTabButtonMode(true)
        window:SetMinTabWidth(90);window:SetTabWidth(20);window:SetWidth(50)
    "#,
    );
    assert_eq!(widths(&ui), [90.; 3]);
    run(&ui, "window:SetWidth(600)");
    assert_eq!(widths(&ui), [20.; 3]);
    run(
        &ui,
        "window:SetTabWidth(-1);window:SetMinTabWidth(-4);window:SetTabAreaHeight(-3)",
    );
    assert_eq!(widths(&ui), [0.; 3]);
    let u = lua::ui(&ui.lua);
    let c = u
        .tree
        .widget(id(&ui, "window"))
        .unwrap()
        .chat
        .as_ref()
        .unwrap();
    assert_eq!(
        (c.tab_width, c.min_tab_width, c.tab_area_height),
        (0., 0., 0.)
    );
}

#[test]
fn local_width_uses_f32_floor_boundary_and_positive_zero() {
    let ui = setup();
    ui.lua
        .globals()
        .set("below", f32::from_bits(300f32.to_bits() - 1))
        .unwrap();
    run(
        &ui,
        "window:SetTabWidth(500);window:UseAutoResizingTabButtonMode(true);window:SetWidth(below)",
    );
    assert_eq!(widths(&ui), [99.; 3]);
    run(&ui, "window:SetWidth(300)");
    assert_eq!(widths(&ui), [100.; 3]);
    // The native local-extent getter is independent of framebuffer/UI scale.
    lua::ui(&ui.lua).ui_scale = 1.25;
    run(&ui, "window:SetWidth(below)");
    assert_eq!(widths(&ui), [99.; 3]);
    run(&ui, "window:SetWidth(-0)");
    assert!(widths(&ui).iter().all(|w| w.to_bits() == 0));
}

#[test]
fn native_offset_gap_inset_height_and_bottom_aligned_add_button() {
    let ui = setup();
    run(
        &ui,
        r#"
        window:SetTabWidth(500);window:UseAutoResizingTabButtonMode(true)
        window:UseAddTabButton(true);window:SetOffset(3.25);window:SetGap(2.5)
        window:SetTabAreaInset(4,3,11,5)
    "#,
    );
    assert_eq!(widths(&ui), [84.; 3]);
    let ids = tabs(&ui);
    let u = lua::ui(&ui.lua);
    let first = u.tree.rect(ids[0].button);
    let second = u.tree.rect(ids[1].button);
    let last = u.tree.rect(ids[2].button);
    let c = u
        .tree
        .widget(id(&ui, "window"))
        .unwrap()
        .chat
        .as_ref()
        .unwrap();
    let add = u.tree.rect(c.add_button);
    assert_eq!((first.x, first.y, first.h), (107.25, 103., 12.));
    assert_eq!(second.x, first.x + first.w + 2.5);
    assert_eq!(
        (add.x, add.y + add.h),
        (last.x + last.w + 2.5, last.y + last.h)
    );
}

#[test]
fn zero_tab_guard_preserves_child_state_and_anchor_resize_relayouts() {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    run(
        &ui,
        r#"
        window=UIParent:CreateWidget('chatwindow','emptyChat','UIParent')
        assert(window:GetAddButton():GetWidth()==20 and window:GetAddButton():GetHeight()==20)
        window:GetAddButton():SetExtent(22,22)
        window:SetWidth(300);window:SetTabWidth(500)
        window:UseAddTabButton(true);window:UseAutoResizingTabButtonMode(true)
        assert(not window:GetAddButton():IsVisible())
        window:GetAddButton():Show(true)
        window:SetGap(5);window:SetMinTabWidth(42);window:SetWidth(50)
        assert(window:GetAddButton():IsVisible())
        AAUI={ChatTabs=function() return {{id=0,name='All'},{id=1,name='Party'},{id=2,name='Trade'}} end}
        window:SetGap(0);window:SetMinTabWidth(0);window:UseAddTabButton(false)
        window:SetChatWindowId(0)
        owner=UIParent:CreateWidget('emptywidget','chatLayoutOwner','UIParent')
        owner:SetExtent(300,150);owner:AddAnchor('TOPLEFT',UIParent,100,100)
        window:RemoveAllAnchors();window:AddAnchor('TOPLEFT',owner,0,0);window:AddAnchor('TOPRIGHT',owner,0,0)
    "#,
    );
    assert_eq!(widths(&ui), [100.; 3]);
    run(&ui, "owner:SetWidth(299)");
    lua::tick(&ui.lua, 0.);
    assert_eq!(widths(&ui), [99.; 3]);
}

#[test]
fn original_constructor_resize_keeps_cached_unicode_prefix_and_full_tooltip() {
    let mut ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    chat_tab_layout::install(&mut ui);
    let fixed = id(&ui, "fixedChat");
    let fixed_tabs = lua::ui(&ui.lua)
        .tree
        .widget(fixed)
        .unwrap()
        .chat
        .as_ref()
        .unwrap()
        .tabs
        .clone();
    assert!(
        fixed_tabs
            .iter()
        .all(|t| lua::ui(&ui.lua).tree.size(t.button).0 == 140.)
    );
    run(
        &ui,
        "window=nativeClipChat;window:SetTabWidth(500);window:SetWidth(300)",
    );
    render::build_quads(&mut lua::ui(&ui.lua), None);
    let first = tabs(&ui)[0].button;
    let before = lua::ui(&ui.lua)
        .tree
        .get(first)
        .text()
        .unwrap()
        .auto_clip
        .count;
    assert_eq!(widths(&ui), [89.; 3]);
    run(&ui, "window:SetWidth(430)");
    render::build_quads(&mut lua::ui(&ui.lua), None);
    let after = lua::ui(&ui.lua)
        .tree
        .get(first)
        .text()
        .unwrap()
        .auto_clip
        .count;
    assert!(after > before);
    run(
        &ui,
        r#"
        originalTooltip=SetTooltip
        SetTooltip=function(text,owner) tooltipText=text;return originalTooltip(text,owner) end
        nativeClipTabs[1]:OnEnter()
        assert(tooltipText==nativeClipNames[1])
        assert(nativeClipTabs[2]:GetText()==nativeClipNames[2])
    "#,
    );
    run(&ui, "HideTooltip()");
    for window in [id(&ui, "window"), fixed] {
        let target = lua::ui(&ui.lua)
            .tree
            .widget(window)
            .unwrap()
            .chat
            .as_ref()
            .unwrap()
            .tabs[1]
            .button;
        let r = lua::ui(&ui.lua).tree.rect(target);
        let point = (r.x + r.w * 0.5, r.y + r.h * 0.5);
        assert_eq!(aa_ui::input::pick(&ui.lua, point.0, point.1), Some(target));
        let mut mouse = Default::default();
        for pressed in [true, false] {
            aa_ui::input::update_buttons(
                &ui.lua,
                &mut mouse,
                Some(point),
                aa_ui::input::MouseButtons {
                    press: pressed,
                    release: !pressed,
                    ..Default::default()
                },
                0.0,
            );
        }
        assert_eq!(
            lua::ui(&ui.lua)
                .tree
                .widget(window)
                .unwrap()
                .chat
                .as_ref()
                .unwrap()
                .selected,
            1
        );
    }
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
}
