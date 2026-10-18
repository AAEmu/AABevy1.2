//! Native internal chat tab-area clipping, independent of content and public parents.

#[path = "../examples/fake/auto_clip.rs"]
mod auto_clip;
#[path = "../examples/fake/mod.rs"]
#[allow(dead_code)]
mod fake;

use aa_ui::{UiRuntime, input, lua, render};
fn run(ui: &UiRuntime, s: &str) {
    ui.exec("@chat_clip", s).unwrap();
}
fn id(ui: &UiRuntime, s: &str) -> u32 {
    lua::value_id(&ui.lua, &ui.lua.load(s).eval().unwrap()).unwrap()
}
fn setup() -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    run(
        &ui,
        r#"
        AAUI={ChatTabs=function() return {{id=0,name='All'},{id=1,name='Party'},{id=2,name='Trade'}} end}
        window=UIParent:CreateWidget('chatwindow','tabClip','UIParent')
        window:SetExtent(200,100);window:AddAnchor('TOPLEFT',UIParent,100,100)
        window:SetTabAreaHeight(30);window:SetTabWidth(120)
        window:SetChatWindowId(0);window:UseAddTabButton(true);window:Show(true)
    "#,
    );
    let c = lua::ui(&ui.lua)
        .tree
        .widget(id(&ui, "window"))
        .unwrap()
        .chat
        .clone()
        .unwrap();
    let tabs = ui.lua.create_table().unwrap();
    for (i, t) in c.tabs.iter().enumerate() {
        tabs.raw_set(
            i + 1,
            lua::object_table(&ui.lua, t.button).unwrap().unwrap(),
        )
        .unwrap();
    }
    ui.lua.globals().set("tabs", tabs).unwrap();
    ui.lua
        .globals()
        .set(
            "message",
            lua::object_table(&ui.lua, c.tabs[0].message)
                .unwrap()
                .unwrap(),
        )
        .unwrap();
    run(
        &ui,
        r#"
        for i,tab in ipairs(tabs) do
            tab:SetText('')
            local art=tab:CreateColorDrawable(i==1 and 1 or 0,i==2 and 1 or 0,i==3 and 1 or 0,1,'artwork')
            art:AddAnchor('TOPLEFT',tab,0,0);art:AddAnchor('BOTTOMRIGHT',tab,0,0)
        end
        add=window:GetAddButton();addArt=add:CreateColorDrawable(1,1,0,1,'artwork')
        addArt:AddAnchor('TOPLEFT',add,0,0);addArt:AddAnchor('BOTTOMRIGHT',add,0,0)
    "#,
    );
    ui
}
fn colored(ui: &UiRuntime, color: [f32; 3]) -> Vec<render::Quad> {
    render::build_quads(&mut lua::ui(&ui.lua), None)
        .into_iter()
        .filter(|q| q.color[..3] == color)
        .collect()
}
fn nonzero(q: &render::Quad) -> bool {
    q.pos[2] > q.pos[0] && q.pos[3] > q.pos[1]
}

#[test]
fn overflowing_fixed_tabs_and_add_button_are_clipped_and_not_pickable_outside_inclusive_edges() {
    let ui = setup();
    assert_eq!(colored(&ui, [0., 1., 0.])[0].pos, [220., 100., 300., 130.]);
    assert!(colored(&ui, [0., 0., 1.]).iter().all(|q| !nonzero(q)));
    assert!(colored(&ui, [1., 1., 0.]).iter().all(|q| !nonzero(q)));
    assert_eq!(input::pick(&ui.lua, 300., 115.), Some(id(&ui, "tabs[2]")));
    assert_eq!(
        input::pick(&ui.lua, f32::from_bits(300f32.to_bits() + 1), 115.),
        None
    );
    assert_eq!(input::pick(&ui.lua, 350., 115.), None);
    run(&ui, "window:SetOffset(-40)");
    assert_eq!(colored(&ui, [1., 0., 0.])[0].pos, [100., 100., 180., 130.]);
    assert_eq!(input::pick(&ui.lua, 100., 115.), Some(id(&ui, "tabs[1]")));
    assert_eq!(
        input::pick(&ui.lua, f32::from_bits(100f32.to_bits() - 1), 115.),
        None
    );
}

#[test]
fn descendants_clip_but_message_caret_and_other_window_children_remain_outside_area() {
    let ui = setup();
    run(
        &ui,
        r#"
        child=tabs[1]:CreateChildWidget('button','overflowChild',0,true)
        child:SetExtent(40,60);child:AddAnchor('TOPLEFT',tabs[1],-20,-10)
        childArt=child:CreateColorDrawable(0,1,1,1,'artwork')
        childArt:AddAnchor('TOPLEFT',child,0,0);childArt:AddAnchor('BOTTOMRIGHT',child,0,0)
        body=message:CreateColorDrawable(1,0,1,1,'artwork')
        body:AddAnchor('TOPLEFT',message,0,0);body:AddAnchor('BOTTOMRIGHT',message,0,0)
        sibling=window:CreateChildWidget('button','outsideArea',0,true)
        sibling:SetExtent(20,20);sibling:AddAnchor('TOPLEFT',window,-30,50)
        siblingArt=sibling:CreateColorDrawable(0.5,0.5,0.5,1,'artwork')
        siblingArt:AddAnchor('TOPLEFT',sibling,0,0);siblingArt:AddAnchor('BOTTOMRIGHT',sibling,0,0)
    "#,
    );
    assert_eq!(colored(&ui, [0., 1., 1.])[0].pos, [100., 100., 120., 130.]);
    assert_eq!(colored(&ui, [1., 0., 1.])[0].pos, [100., 130., 300., 200.]);
    assert_eq!(colored(&ui, [0.5, 0.5, 0.5])[0].pos, [70., 150., 90., 170.]);
    assert_eq!(input::pick(&ui.lua, 90., 110.), None);
    assert_eq!(input::pick(&ui.lua, 110., 125.), Some(id(&ui, "child")));
    assert_eq!(input::pick(&ui.lua, 75., 155.), Some(id(&ui, "sibling")));
    run(
        &ui,
        "assert(tabs[1]:GetParent()==window);assert(child:GetParent()==tabs[1]);assert(message:GetParent()==window)",
    );
}

#[test]
fn divider_ranges_clip_after_rotation_without_clipping_the_owners_caret() {
    let ui = setup();
    let tab = id(&ui, "tabs[2]");
    let r = lua::ui(&ui.lua).tree.rect(tab);
    let mut mouse = Default::default();
    let p = (r.x + 20., r.y + 10.);
    input::update(&ui.lua, &mut mouse, Some(p), true, false, false, 0.);
    input::update(&ui.lua, &mut mouse, Some(p), false, true, false, 0.);
    run(
        &ui,
        r#"
        window:SetLeftLineOffset(-20);window:SetRightLineOffset(25)
        left=window:GetLeftLineDrawable();left:SetTexture('ui/common/hud.dds')
        left:SetCoords(462,233,3,3);left:SetHeight(20);left:SetRotation(35)
        caret=window:GetCaretDrawable();caret:SetTexture('ui/common/hud.dds')
        caret:SetCoords(462,233,3,3);caret:SetExtent(10,10)
        caret:AddAnchor('TOPLEFT',window,190,50);caret:Show(true)
    "#,
    );
    let quads = render::build_quads(&mut lua::ui(&ui.lua), None);
    let triangles: Vec<_> = quads.iter().filter(|q| q.corners.is_some()).collect();
    assert!(!triangles.is_empty());
    for q in triangles {
        for [x, y] in q.corners.unwrap() {
            assert!(x >= 100. && x <= 300. && y >= 100. && y <= 130.);
        }
    }
    assert!(quads.iter().any(|q| q.pos == [290., 150., 300., 160.]));
}

#[test]
fn scaled_area_and_ancestor_scroll_intersect_in_physical_pixels() {
    let ui = setup();
    run(
        &ui,
        r#"
        UIParent:SetUIScale(0.85);window:SetScale(1.5)
        window:EnableScroll(true)
        child=tabs[2]:CreateChildWidget('button','scaledOverflow',0,true)
        child:SetExtent(100,80);child:AddAnchor('TOPLEFT',tabs[2],0,-20)
        child:SetScale(0.75)
        art=child:CreateColorDrawable(0.3,0.4,0.5,1,'artwork')
        art:AddAnchor('TOPLEFT',child,0,0);art:AddAnchor('BOTTOMRIGHT',child,0,0)
    "#,
    );
    let (area, outer) = {
        let u = lua::ui(&ui.lua);
        let w = id(&ui, "window");
        let r = u.animations.widget_rect(&u.tree, w);
        let s = u.ui_scale;
        let a = [r.x * s, r.y * s, r.x * s + 300., r.y * s + 45.];
        let o = [
            (r.x * s).round(),
            (r.y * s).round(),
            (r.right() * s).round(),
            (r.bottom() * s).round(),
        ];
        (a, o)
    };
    let bounds = [
        area[0].max(outer[0]),
        area[1].max(outer[1]),
        area[2].min(outer[2]),
        area[3].min(outer[3]),
    ];
    let qs = colored(&ui, [0.3, 0.4, 0.5]);
    assert!(qs.iter().any(nonzero));
    for q in qs.into_iter().filter(nonzero) {
        assert!(
            q.pos[0] >= bounds[0]
                && q.pos[1] >= bounds[1]
                && q.pos[2] <= bounds[2]
                && q.pos[3] <= bounds[3]
        );
    }
    assert_eq!(
        input::pick(&ui.lua, (area[2] + 1.) / 0.85, (area[1] + 10.) / 0.85),
        None
    );
}

#[test]
fn zero_tab_guard_retains_area_anchor_height_until_nonempty_layout() {
    let ui = setup();
    let window = id(&ui, "window");
    lua::ui(&ui.lua)
        .tree
        .widget_mut(window)
        .unwrap()
        .chat
        .as_mut()
        .unwrap()
        .tabs
        .clear();
    run(
        &ui,
        r#"
        window:SetTabAreaHeight(70)
        add:RemoveAllAnchors();add:AddAnchor('TOPLEFT',window,0,20)
        add:Show(true)
    "#,
    );
    assert_eq!(colored(&ui, [1., 1., 0.])[0].pos, [100., 120., 120., 130.]);
    run(
        &ui,
        // Same-ID hydration is a native no-op; use a different window to repopulate.
        "window:SetChatWindowId(1);add:RemoveAllAnchors();add:AddAnchor('TOPLEFT',window,0,20);add:Show(true)",
    );
    // Insertion runs the nonempty tab-area layout with today's height70.
    let r = lua::ui(&ui.lua).tree.rect(id(&ui, "add"));
    assert_eq!(r.h, 20.);
    assert_eq!(colored(&ui, [1., 1., 0.])[0].pos, [100., 120., 120., 140.]);
}

#[test]
fn nested_chat_areas_intersect_and_outer_area_also_clips_inner_content() {
    let ui = setup();
    run(
        &ui,
        r#"
        inner=tabs[1]:CreateChildWidget('chatwindow','nestedClip',0,true)
        inner:SetExtent(150,80);inner:AddAnchor('TOPLEFT',tabs[1],-20,10)
        inner:SetTabAreaHeight(40);inner:SetTabWidth(120);inner:SetChatWindowId(7)
    "#,
    );
    let c = lua::ui(&ui.lua)
        .tree
        .widget(id(&ui, "inner"))
        .unwrap()
        .chat
        .clone()
        .unwrap();
    ui.lua
        .globals()
        .set(
            "innerTab",
            lua::object_table(&ui.lua, c.tabs[0].button)
                .unwrap()
                .unwrap(),
        )
        .unwrap();
    ui.lua
        .globals()
        .set(
            "innerBody",
            lua::object_table(&ui.lua, c.tabs[0].message)
                .unwrap()
                .unwrap(),
        )
        .unwrap();
    run(
        &ui,
        r#"
        innerTab:SetText('')
        paint=innerTab:CreateColorDrawable(0.1,0.7,0.4,1,'artwork')
        paint:AddAnchor('TOPLEFT',innerTab,0,0);paint:AddAnchor('BOTTOMRIGHT',innerTab,0,0)
        paintBody=innerBody:CreateColorDrawable(0.8,0.2,0.6,1,'artwork')
        paintBody:AddAnchor('TOPLEFT',innerBody,0,0);paintBody:AddAnchor('BOTTOMRIGHT',innerBody,0,0)
    "#,
    );
    assert_eq!(
        colored(&ui, [0.1, 0.7, 0.4])[0].pos,
        [100., 110., 200., 130.]
    );
    assert!(colored(&ui, [0.8, 0.2, 0.6]).iter().all(|q| !nonzero(q)));
    assert_eq!(input::pick(&ui.lua, 90., 120.), None);
    assert_eq!(input::pick(&ui.lua, 110., 120.), Some(c.tabs[0].button));
}

#[test]
fn original_fixed_tabs_keep_full_tooltip_and_body_after_physical_selection() {
    let mut ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    auto_clip::install(&mut ui);
    run(
        &ui,
        "nativeClipChat:UseAutoResizingTabButtonMode(false);nativeClipChat:SetTabWidth(220);window=nativeClipChat;HideTooltip()",
    );
    let c = lua::ui(&ui.lua)
        .tree
        .widget(id(&ui, "window"))
        .unwrap()
        .chat
        .clone()
        .unwrap();
    let (r, s) = {
        let u = lua::ui(&ui.lua);
        (
            u.animations.widget_rect(&u.tree, c.tabs[1].button),
            u.ui_scale,
        )
    };
    let p = (r.x + 20., r.y + r.h * 0.5);
    assert_eq!(input::pick(&ui.lua, p.0, p.1), Some(c.tabs[1].button));
    let mut mouse = Default::default();
    input::update(&ui.lua, &mut mouse, Some(p), true, false, false, 0.);
    input::update(&ui.lua, &mut mouse, Some(p), false, true, false, 0.);
    run(
        &ui,
        "oldTooltip=SetTooltip;SetTooltip=function(text,owner) tooltipText=text;return oldTooltip(text,owner) end;nativeClipTabs[2]:OnEnter();assert(tooltipText==nativeClipNames[2]);HideTooltip()",
    );
    let wr = {
        let u = lua::ui(&ui.lua);
        u.animations.widget_rect(&u.tree, id(&ui, "window"))
    };
    assert_ne!(
        input::pick(&ui.lua, wr.right() + 2. / s, r.y + 5.),
        Some(c.tabs[1].button)
    );
    assert_eq!(
        lua::ui(&ui.lua)
            .tree
            .get(c.tabs[1].button)
            .text()
            .unwrap()
            .text,
        ui.lua.load("nativeClipNames[2]").eval::<String>().unwrap()
    );
    assert!(!render::build_quads(&mut lua::ui(&ui.lua), None).is_empty());
    run(
        &ui,
        "UIParent:SetUIScale(GetUIScaleValueByOptionWindowValue(90),true)",
    );
    let r = {
        let u = lua::ui(&ui.lua);
        u.animations.widget_rect(&u.tree, c.tabs[1].button)
    };
    let p = (r.x + 20., r.y + r.h * 0.5);
    assert_eq!(input::pick(&ui.lua, p.0, p.1), Some(c.tabs[1].button));
    input::update(&ui.lua, &mut mouse, Some(p), true, false, false, 0.);
    input::update(&ui.lua, &mut mouse, Some(p), false, true, false, 0.);
    assert!(!render::build_quads(&mut lua::ui(&ui.lua), None).is_empty());
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
}
