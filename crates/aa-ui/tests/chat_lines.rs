//! Native divider anchors retain authored height and follow the selected tab.

#[path = "../examples/fake/auto_clip.rs"]
mod auto_clip;
#[path = "../examples/fake/mod.rs"]
#[allow(dead_code)]
mod fake;

use aa_ui::{UiRuntime, input, lua, render, tree::Rect};

fn run(ui: &UiRuntime, code: &str) {
    ui.exec("@chat_lines", code).unwrap();
}
fn id(ui: &UiRuntime, name: &str) -> u32 {
    lua::value_id(&ui.lua, &ui.lua.load(name).eval().unwrap()).unwrap()
}
fn data(ui: &UiRuntime) -> lua::chat::ChatData {
    lua::ui(&ui.lua)
        .tree
        .widget(id(ui, "window"))
        .unwrap()
        .chat
        .clone()
        .unwrap()
}
fn rect(ui: &UiRuntime, id: u32) -> Rect {
    lua::ui(&ui.lua).tree.rect(id)
}
fn setup() -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    run(
        &ui,
        r#"
        AAUI={ChatTabs=function() return {{id=0,name='All'},{id=1,name='Party'},{id=2,name='Trade'}} end}
        window=UIParent:CreateWidget('chatwindow','nativeLines','UIParent')
        window:SetExtent(300,150);window:AddAnchor('TOPLEFT',UIParent,100,100)
        window:SetTabAreaHeight(30);window:SetTabWidth(80);window:SetGap(4)
        window:SetOffset(3);window:SetTabAreaInset(5,2,7,0)
        left=window:GetLeftLineDrawable();right=window:GetRightLineDrawable()
        left:SetExtent(0,3);right:SetExtent(0,5)
        window:SetChatWindowId(0);window:Show(true)
    "#,
    );
    ui
}
fn click(ui: &UiRuntime, button: u32) {
    let r = {
        let u = lua::ui(&ui.lua);
        u.animations.widget_rect(&u.tree, button)
    };
    let p = (r.x + r.w * 0.5, r.y + r.h * 0.5);
    assert_eq!(input::pick(&ui.lua, p.0, p.1), Some(button));
    let mut mouse = Default::default();
    input::update(&ui.lua, &mut mouse, Some(p), true, false, false, 0.);
    input::update(&ui.lua, &mut mouse, Some(p), false, true, false, 0.);
}

#[test]
fn native_constructor_dividers_start_hidden_at_one_pixel_without_original_lua() {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    run(
        &ui,
        "window=UIParent:CreateWidget('chatwindow','nativeLineDefaults','UIParent')",
    );
    let c = data(&ui);
    for line in [c.left_line, c.right_line] {
        let u = lua::ui(&ui.lua);
        assert_eq!(u.tree.get(line).height, 1.);
        assert!(!u.tree.get(line).shown);
    }
    run(&ui, "window:SetChatWindowId(0)");
    for line in [c.left_line, c.right_line] {
        assert_eq!(rect(&ui, line).h, 1.);
    }
}

#[test]
fn signed_offsets_and_selection_keep_authored_heights_and_content_anchors() {
    let ui = setup();
    let c = data(&ui);
    assert_eq!(
        rect(&ui, c.left_line),
        Rect {
            x: 100.,
            y: 127.,
            w: 8.,
            h: 3.
        }
    );
    assert_eq!(
        rect(&ui, c.right_line),
        Rect {
            x: 188.,
            y: 125.,
            w: 212.,
            h: 5.
        }
    );
    let content = lua::ui(&ui.lua).tree.get(c.tabs[0].message).anchors.clone();
    run(
        &ui,
        "window:SetLeftLineOffset(-9);window:SetRightLineOffset(13)",
    );
    assert_eq!(
        rect(&ui, c.left_line),
        Rect {
            x: 91.,
            y: 127.,
            w: 17.,
            h: 3.
        }
    );
    assert_eq!(
        rect(&ui, c.right_line),
        Rect {
            x: 188.,
            y: 125.,
            w: 225.,
            h: 5.
        }
    );
    click(&ui, c.tabs[1].button);
    assert_eq!(data(&ui).selected, 1);
    assert_eq!(
        rect(&ui, c.left_line),
        Rect {
            x: 91.,
            y: 127.,
            w: 101.,
            h: 3.
        }
    );
    assert_eq!(
        rect(&ui, c.right_line),
        Rect {
            x: 272.,
            y: 125.,
            w: 141.,
            h: 5.
        }
    );
    assert_eq!(
        lua::ui(&ui.lua).tree.get(c.tabs[0].message).anchors,
        content
    );
    run(
        &ui,
        "window:SetTabAreaInset(5,2,7,4);window:SetTabAreaHeight(50)",
    );
    // Bottom-left anchor precedence: left follows area bottom, right follows
    // selected-tab bottom. The one-unit offsets do not overwrite line height.
    assert_eq!(rect(&ui, c.left_line).y, 147.);
    assert_eq!(rect(&ui, c.right_line).y, 141.);
    assert_eq!(
        lua::ui(&ui.lua).tree.get(c.tabs[0].message).anchors,
        content
    );
}

#[test]
fn native_anchors_follow_direct_tab_and_drawable_changes_without_tab_relayout() {
    let ui = setup();
    let c = data(&ui);
    ui.lua
        .globals()
        .set(
            "tab",
            lua::object_table(&ui.lua, c.tabs[0].button)
                .unwrap()
                .unwrap(),
        )
        .unwrap();
    run(&ui, "tab:SetWidth(100)");
    assert_eq!(rect(&ui, c.right_line).x, 208.);
    run(&ui, "left:SetHeight(7);right:SetHeight(9)");
    assert_eq!(rect(&ui, c.left_line).y, 123.);
    assert_eq!(rect(&ui, c.right_line).y, 121.);
    run(
        &ui,
        "tab:RemoveAllAnchors();tab:AddAnchor('TOPLEFT',window,60,10)",
    );
    assert_eq!(
        rect(&ui, c.left_line),
        Rect {
            x: 100.,
            y: 123.,
            w: 60.,
            h: 7.
        }
    );
    assert_eq!(
        rect(&ui, c.right_line),
        Rect {
            x: 260.,
            y: 129.,
            w: 140.,
            h: 9.
        }
    );
    // Explicit window geometry restores the ordinary tab anchors, retaining
    // the custom drawable height and configured line offsets.
    run(&ui, "window:SetWidth(350)");
    assert_eq!(rect(&ui, c.left_line).h, 7.);
    assert_eq!(rect(&ui, c.right_line).h, 9.);
}

#[test]
fn no_selected_tab_hides_lines_and_offset_setters_retain_signed_f32() {
    let ui = setup();
    let window = id(&ui, "window");
    let c = data(&ui);
    lua::ui(&ui.lua)
        .tree
        .widget_mut(window)
        .unwrap()
        .chat
        .as_mut()
        .unwrap()
        .selected = usize::MAX;
    let anchors = lua::ui(&ui.lua).tree.get(c.left_line).anchors.clone();
    run(
        &ui,
        "window:SetLeftLineOffset(-0.1000000001);window:SetRightLineOffset(0.1000000001)",
    );
    assert_eq!(data(&ui).line_offset, [-0.1, 0.1]);
    assert!(!lua::ui(&ui.lua).tree.get(c.left_line).shown);
    assert!(!lua::ui(&ui.lua).tree.get(c.right_line).shown);
    for tab in &c.tabs {
        assert!(!lua::ui(&ui.lua).tree.get(tab.message).shown);
    }
    assert_eq!(lua::ui(&ui.lua).tree.get(c.left_line).anchors, anchors);
    lua::ui(&ui.lua)
        .tree
        .widget_mut(window)
        .unwrap()
        .chat
        .as_mut()
        .unwrap()
        .tabs
        .clear();
    run(&ui, "window:SetRightLineOffset(-8)");
    assert!(!lua::ui(&ui.lua).tree.get(c.left_line).shown);
    run(&ui, "window:SetChatWindowId(0)");
    assert!(lua::ui(&ui.lua).tree.get(c.left_line).shown);
    assert!(lua::ui(&ui.lua).tree.get(c.right_line).shown);
    assert_eq!(data(&ui).line_offset, [-0.1, -8.]);
}

#[test]
fn original_dividers_follow_physical_selection_and_render_with_original_one_pixel_extent() {
    let mut ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    auto_clip::install(&mut ui);
    run(
        &ui,
        "window=nativeClipChat;selectedCallbacks=0;oldSelected=OnSelectedChatTab;OnSelectedChatTab=function(button,selected) selectedCallbacks=selectedCallbacks+1;return oldSelected(button,selected) end;window:SetLeftLineOffset(-8);window:SetRightLineOffset(12);assert(selectedCallbacks==0);HideTooltip()",
    );
    let c = data(&ui);
    click(&ui, c.tabs[1].button);
    assert_eq!(data(&ui).selected, 1);
    run(&ui, "assert(selectedCallbacks>0)");
    run(&ui, "HideTooltip()");
    lua::tick(&ui.lua, 0.);
    let quads = render::build_quads(&mut lua::ui(&ui.lua), None);
    for line in [c.left_line, c.right_line] {
        let r = rect(&ui, line);
        let scale = lua::ui(&ui.lua).ui_scale;
        let window = rect(&ui, id(&ui, "window"));
        assert_eq!(r.h, 1.);
        let expected = [
            (r.x * scale).round().max(window.x * scale),
            (r.y * scale).round().max(window.y * scale),
            (r.right() * scale).round().min(window.right() * scale),
            (r.bottom() * scale)
                .round()
                .min((window.y + c.tab_area_height) * scale),
        ];
        assert!(
            quads.iter().any(|q| q.pos == expected),
            "missing native line {expected:?}"
        );
    }
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
}
