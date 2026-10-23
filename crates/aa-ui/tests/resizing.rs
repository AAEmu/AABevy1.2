//! Native border interception, directional limits and unchanged quest notifier integration.

#[path = "../examples/fake/mod.rs"]
#[allow(dead_code)]
mod fake;
#[path = "../examples/fake/resizing.rs"]
mod resizing;

use aa_ui::{UiRuntime, input, lua};

fn run(ui: &UiRuntime, source: &str) {
    ui.exec("@resize_test", source).unwrap();
}
fn step(ui: &UiRuntime, mouse: &mut input::MouseState, at: (f32, f32), down: bool, up: bool) {
    input::update(&ui.lua, mouse, Some(at), down, up, false, 0.0);
}
fn setup(ui: &UiRuntime) {
    run(
        ui,
        r#"
        w=UIParent:CreateWidget('window','resizeWindow','UIParent')
        w:SetExtent(200,100);w:AddAnchor('TOPLEFT',UIParent,100,100)
        w:Show(true)
        w:SetMinResizingExtent(100,50);w:SetMaxResizingExtent(300,200)
        w:SetResizingBorderSize(10,10,10,10);w:UseResizing(true)
        clicks=0;downs=0
        w:SetHandler('OnClick',function() clicks=clicks+1 end)
        w:SetHandler('OnMouseDown',function() downs=downs+1 end)
    "#,
    );
}

#[test]
fn eight_native_edges_preserve_opposite_edges_and_consume_the_click() {
    for (from, to, expected) in [
        ((105., 150.), (80., 150.), [75., 100., 225., 100.]),
        ((295., 150.), (330., 150.), [100., 100., 235., 100.]),
        ((200., 105.), (200., 80.), [100., 75., 200., 125.]),
        ((200., 195.), (200., 230.), [100., 100., 200., 135.]),
        ((105., 105.), (80., 80.), [75., 75., 225., 125.]),
        ((295., 105.), (330., 80.), [100., 75., 235., 125.]),
        ((105., 195.), (80., 230.), [75., 100., 225., 135.]),
        ((295., 195.), (330., 230.), [100., 100., 235., 135.]),
    ] {
        let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
        setup(&ui);
        let mut mouse = input::MouseState::default();
        step(&ui, &mut mouse, from, true, false);
        step(&ui, &mut mouse, to, false, false);
        step(&ui, &mut mouse, to, false, true);
        let u = lua::ui(&ui.lua);
        let r = u.tree.rect(u.tree.find("resizeWindow").unwrap());
        assert_eq!([r.x, r.y, r.w, r.h], expected);
        drop(u);
        run(&ui, "assert(downs==0 and clicks==0)");
        step(&ui, &mut mouse, (800., 800.), false, false);
        let u = lua::ui(&ui.lua);
        assert_eq!(u.tree.rect(u.tree.find("resizeWindow").unwrap()), r);
    }
}

#[test]
fn clamped_capture_waits_for_reentry_and_stops_explicitly() {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    setup(&ui);
    let mut mouse = input::MouseState::default();
    step(&ui, &mut mouse, (295., 150.), true, false);
    step(&ui, &mut mouse, (500., 150.), false, false);
    run(&ui, "assert(w:GetWidth()==300)");
    step(&ui, &mut mouse, (450., 150.), false, false);
    run(&ui, "assert(w:GetWidth()==300)");
    step(&ui, &mut mouse, (390., 150.), false, false);
    run(&ui, "assert(w:GetWidth()==240);w:StopMovingOrSizing()");
    step(&ui, &mut mouse, (600., 150.), false, false);
    run(&ui, "assert(w:GetWidth()==240)");
    // Explicit sizing is independent of the automatic enable flag.
    run(&ui, "w:UseResizing(false);w:StartSizing('RIGHT')");
    step(&ui, &mut mouse, (650., 150.), false, false);
    run(&ui, "assert(w:GetWidth()==290);w:StopMovingOrSizing()");
}

#[test]
fn border_pixels_and_resolved_widget_scale_are_independent() {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    setup(&ui);
    run(&ui, "UIParent:SetUIScale(2)");
    let mut mouse = input::MouseState::default();
    step(&ui, &mut mouse, (107., 150.), true, false);
    step(&ui, &mut mouse, (107., 150.), false, true);
    run(&ui, "assert(downs==1 and clicks==1)");
    step(&ui, &mut mouse, (103., 150.), true, false);
    step(&ui, &mut mouse, (80., 150.), false, false);
    step(&ui, &mut mouse, (80., 150.), false, true);
    run(&ui, "assert(downs==1 and clicks==1 and w:GetWidth()==223)");

    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    setup(&ui);
    run(&ui, "w:SetScale(0.5)");
    let mut mouse = input::MouseState::default();
    step(&ui, &mut mouse, (248., 150.), true, false);
    step(&ui, &mut mouse, (268., 150.), false, false);
    step(&ui, &mut mouse, (268., 150.), false, true);
    run(
        &ui,
        "assert(w:GetWidth()==240);local x,y=w:GetEffectiveOffset();assert(x==150 and y==125)",
    );
}

#[test]
fn original_tracker_uses_authored_bottom_border_limits_and_guide() {
    let mut ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    resizing::install(&mut ui);
    resizing::drag_bottom(&ui);
    run(&ui, "assert(questContext.notifierBack:IsVisible())");
    let u = lua::ui(&ui.lua);
    let id = u.tree.find("questContext.notifierBack").unwrap();
    let w = u.tree.widget(id).unwrap();
    assert_eq!(w.resizing.border, [0., 0., 0., 10.]);
    assert!(w.drawables.iter().any(|&id| {
        u.tree.get(id).shown
            && u.tree.get(id).drawable().is_some_and(|d| {
                d.texture.as_deref() == Some("ui/hud/chat_guide.dds") && d.inset == [7., 7., 8., 8.]
            })
    }));
}

#[test]
fn covered_chat_fix_does_not_install_a_global_parent_resize_policy() {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    setup(&ui);
    run(&ui, r#"
        child=w:CreateChildWidget('emptywidget','coveredGenericEdge',0,true)
        child:SetExtent(200,100);child:AddAnchor('TOPLEFT',w,0,0);child:Show(true)
        childDowns=0;child:SetHandler('OnMouseDown',function() childDowns=childDowns+1 end)
    "#);
    let mut mouse = input::MouseState::default();
    step(&ui, &mut mouse, (105., 150.), true, false);
    step(&ui, &mut mouse, (85., 150.), false, true);
    run(&ui, "assert(childDowns==1 and downs==0 and w:GetWidth()==200);local x,y=w:GetEffectiveOffset();assert(x==100 and y==100)");
}
