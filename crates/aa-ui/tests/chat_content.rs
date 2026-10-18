//! Native message-frame corner offsets and their independent update lifetime.

#[path = "../examples/fake/auto_clip.rs"]
mod auto_clip;
#[path = "../examples/fake/mod.rs"]
#[allow(dead_code)]
mod fake;

use aa_ui::{
    UiRuntime, input, lua, render,
    tree::{Point, Rect},
};

fn run(ui: &UiRuntime, code: &str) {
    ui.exec("@chat_content", code).unwrap();
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
fn rect(ui: &UiRuntime, id: u32) -> Rect {
    lua::ui(&ui.lua).tree.rect(id)
}
fn setup() -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    run(
        &ui,
        r#"
        AAUI={ChatTabs=function() return {{id=0,name='All'},{id=1,name='Party'},{id=2,name='Trade'}} end}
        window=UIParent:CreateWidget('chatwindow','contentLayout','UIParent')
        window:SetExtent(300,150);window:AddAnchor('TOPLEFT',UIParent,100,100)
        window:SetTabAreaInset(7,3,10,4);window:SetChatWindowId(0)
        window:Show(true)
    "#,
    );
    ui
}

#[test]
fn default_content_uses_full_width_independent_of_tab_insets() {
    let ui = setup();
    for tab in tabs(&ui) {
        assert_eq!(
            rect(&ui, tab.message),
            Rect {
                x: 100.,
                y: 120.,
                w: 300.,
                h: 130.
            }
        );
    }
    run(
        &ui,
        "window:SetTabAreaInset(18,5,80,9);window:SetWidth(430)",
    );
    for tab in tabs(&ui) {
        assert_eq!(
            rect(&ui, tab.message),
            Rect {
                x: 100.,
                y: 120.,
                w: 430.,
                h: 130.
            }
        );
    }
}

#[test]
fn signed_offsets_update_hidden_frames_and_preserve_native_content_callback_lifetime() {
    let ui = setup();
    run(&ui, "window:SetContentOffset(5,7,-9,-11)");
    for tab in tabs(&ui) {
        assert_eq!(
            rect(&ui, tab.message),
            Rect {
                x: 105.,
                y: 127.,
                w: 286.,
                h: 112.
            }
        );
    }
    let hidden = tabs(&ui)[1].message;
    assert!(!lua::ui(&ui.lua).tree.get(hidden).shown);
    run(
        &ui,
        "window:SetTabAreaHeight(50);window:SetTabWidth(110);window:SetTabAreaInset(4,5,6,7)",
    );
    for tab in tabs(&ui) {
        assert_eq!(rect(&ui, tab.message).y, 127.);
    }
    // Selection runs ordinary tab layout, not the independent content callback.
    let button = tabs(&ui)[1].button;
    let r = {
        let u = lua::ui(&ui.lua);
        u.animations.widget_rect(&u.tree, button)
    };
    let p = (r.x + r.w * 0.5, r.y + r.h * 0.5);
    assert_eq!(input::pick(&ui.lua, p.0, p.1), Some(button));
    let mut mouse = Default::default();
    input::update(&ui.lua, &mut mouse, Some(p), true, false, false, 0.);
    input::update(&ui.lua, &mut mouse, Some(p), false, true, false, 0.);
    assert_eq!(
        lua::ui(&ui.lua)
            .tree
            .widget(id(&ui, "window"))
            .unwrap()
            .chat
            .as_ref()
            .unwrap()
            .selected,
        1
    );
    for tab in tabs(&ui) {
        assert_eq!(rect(&ui, tab.message).y, 127.);
    }
    // The same explicit offset setter refreshes against today's area height.
    run(&ui, "window:SetContentOffset(5,7,-9,-11)");
    for tab in tabs(&ui) {
        assert_eq!(
            rect(&ui, tab.message),
            Rect {
                x: 105.,
                y: 157.,
                w: 286.,
                h: 82.
            }
        );
    }
    run(&ui, "window:SetContentOffset(-3,-4,9,11)");
    for tab in tabs(&ui) {
        assert_eq!(
            rect(&ui, tab.message),
            Rect {
                x: 97.,
                y: 146.,
                w: 312.,
                h: 115.
            }
        );
    }
}

#[test]
fn empty_tab_vector_stores_offsets_for_later_insertion_without_touching_children() {
    let ui = setup();
    let before: Vec<_> = tabs(&ui)
        .into_iter()
        .map(|t| (t.message, rect(&ui, t.message)))
        .collect();
    lua::ui(&ui.lua)
        .tree
        .widget_mut(id(&ui, "window"))
        .unwrap()
        .chat
        .as_mut()
        .unwrap()
        .tabs
        .clear();
    run(&ui, "window:SetContentOffset(5,7,-9,-11)");
    for (id, r) in before {
        assert_eq!(rect(&ui, id), r);
    }
    // Same-ID assignment is a native no-op; assign another configured window to rebuild.
    run(&ui, "window:SetChatWindowId(1)");
    for tab in tabs(&ui) {
        assert_eq!(
            rect(&ui, tab.message),
            Rect {
                x: 105.,
                y: 127.,
                w: 286.,
                h: 112.
            }
        );
    }
}

#[test]
fn top_offset_addition_remains_f32_before_anchor_storage() {
    let ui = setup();
    run(
        &ui,
        "window:SetTabAreaHeight(16777216);window:SetContentOffset(0,1,0,0)",
    );
    for tab in tabs(&ui) {
        let u = lua::ui(&ui.lua);
        let a = &u.tree.get(tab.message).anchors[0];
        assert_eq!(
            (a.point, a.rel, a.x, a.y),
            (Point::TopLeft, Point::TopLeft, 0., 16777216.)
        );
    }
}

#[test]
fn original_message_full_width_keeps_boundary_link_on_one_line_and_pickable_after_resize() {
    let mut ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    auto_clip::install(&mut ui);
    run(&ui, "window=nativeClipChat");
    let msg = tabs(&ui)[0].message;
    ui.lua
        .globals()
        .set("message", lua::object_table(&ui.lua, msg).unwrap().unwrap())
        .unwrap();
    for width in [430., 345.] {
        run(&ui, &format!("window:SetWidth({width})"));
        let (font, size, space, inset, scale) = {
            let u = lua::ui(&ui.lua);
            let t = u.tree.get(msg).text().unwrap();
            (
                t.style.font.clone(),
                t.style.size,
                t.line_space,
                t.inset,
                u.ui_scale,
            )
        };
        assert_eq!(rect(&ui, msg).w, width);
        let available = (width - inset[0] - inset[2]) * scale;
        // This real-font link fits the corrected original width but would wrap
        // under the previous extra ten-unit right subtraction.
        let name = (1..200)
            .map(|n| "N".repeat(n))
            .find(|s| {
                let w = lua::ui(&ui.lua).fonts.measure(&font, size * scale, s);
                w > available - 10. * scale && w <= available
            })
            .unwrap();
        let raw = format!("|k{name},0;");
        ui.lua.globals().set("raw", raw.as_str()).unwrap();
        run(&ui, "message:Clear();message:AddMessage(raw)");
        let lo = lua::ui(&ui.lua).fonts.layout(
            &font,
            size * scale,
            &raw,
            Some(available),
            space * scale,
            false,
        );
        assert_eq!(lo.line_widths.len(), 1);
        assert_eq!(lo.links.len(), 1);
        let r = rect(&ui, msg);
        let link = &lo.links[0];
        let ox = ((r.x + inset[0]) * scale).round();
        let oy = ((r.bottom() - inset[3]) * scale - lo.height).round();
        let p = (
            (ox + (link.rect[0] + link.rect[2]) * 0.5) / scale,
            (oy + (link.rect[1] + link.rect[3]) * 0.5) / scale,
        );
        assert_eq!(input::pick(&ui.lua, p.0, p.1), Some(msg));
        let mut mouse = Default::default();
        input::update(&ui.lua, &mut mouse, Some(p), false, false, false, 0.);
        let info = ui
            .lua
            .load("message:GetLinkInfoOnCursor()")
            .eval::<mlua::Table>()
            .unwrap();
        assert_eq!(info.get::<String>("linkType").unwrap(), "character");
        assert_eq!(info.get::<String>("characterName").unwrap(), name);
        assert!(!render::build_quads(&mut lua::ui(&ui.lua), None).is_empty());
    }
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
}
