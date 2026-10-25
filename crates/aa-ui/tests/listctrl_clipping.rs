//! Original EnableScroll marker/full-rectangle contract through actual paint and picking.
//! CPU only: no GPU, live original client or game service.
#[path = "../examples/fake/mod.rs"]
#[allow(dead_code)]
mod fake;

use aa_ui::{
    UiRuntime, input, lua, render,
    tree::{Id, Rect},
};

const ART: [f32; 4] = [0.17, 0.31, 0.53, 1.];
const OWN: [f32; 4] = [0.67, 0.23, 0.41, 1.];
const SELECTED: [f32; 4] = [0.12, 0.78, 0.34, 1.];
const OVERED: [f32; 4] = [0.81, 0.42, 0.19, 1.];

fn run(ui: &UiRuntime, code: &str) {
    ui.exec("@listctrl_clipping", code).unwrap();
    lua::tick(&ui.lua, 0.);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
}

fn id(ui: &UiRuntime, expression: &str) -> Id {
    lua::value_id(&ui.lua, &ui.lua.load(expression).eval().unwrap()).unwrap()
}

fn colored(ui: &UiRuntime, color: [f32; 4]) -> Vec<render::Quad> {
    render::build_quads(&mut lua::ui(&ui.lua), None)
        .into_iter()
        .filter(|q| q.color == color && q.pos[2] > q.pos[0] && q.pos[3] > q.pos[1])
        .collect()
}

fn bounds(rect: Rect, scale: f32) -> [f32; 4] {
    [
        rect.x * scale,
        rect.y * scale,
        rect.right() * scale,
        rect.bottom() * scale,
    ]
}

fn near(actual: [f32; 4], expected: [f32; 4]) {
    for (a, b) in actual.into_iter().zip(expected) {
        assert!((a - b).abs() < 0.001, "{actual:?} != {expected:?}");
    }
}

fn setup() -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    run(
        &ui,
        r#"
        host=UIParent:CreateWidget('window','clipHost','UIParent')
        UIParent:SetUIScale(1)
        host:SetExtent(500,400);host:AddAnchor('TOPLEFT',UIParent,0,0);host:Show(true)
        list=host:CreateChildWidget('listctrl','list',0,true)
        list:SetExtent(200,160);list:AddAnchor('TOPLEFT',host,100,100)
        list:InsertColumn(240,2);list:SetColumnHeight(30);list:InsertRows(2,false)
        entered=0;clicked=0;selections=0
        list:SetHandler('OnSelChanged',function() selections=selections+1 end)
        function edge(name,x,y)
          local button=list.items[2].subItems[1]:CreateChildWidget('button',name,0,true)
          button:SetExtent(20,20);button:AddAnchor('TOPLEFT',list,x,y)
          button:SetHandler('OnEnter',function() entered=entered+1 end)
          button:SetHandler('OnClick',function() clicked=clicked+1 end)
          local art=button:CreateColorDrawable(0.17,0.31,0.53,1,'artwork')
          art:AddAnchor('TOPLEFT',button,0,0);art:AddAnchor('BOTTOMRIGHT',button,0,0)
          button.art=art
          return button
        end
        left=edge('left',-10,60);right=edge('right',190,60)
        top=edge('top',60,-10);bottom=edge('bottom',60,150)
    "#,
    );
    ui
}

#[test]
fn all_four_manual_descendant_edges_clip_paint_and_pointer_without_hiding_cells() {
    let ui = setup();
    let buttons = ["left", "right", "top", "bottom"].map(|s| id(&ui, s));
    let points = [(95., 165.), (305., 165.), (165., 95.), (165., 265.)];
    assert_eq!(colored(&ui, ART).len(), 4);
    for (button, p) in buttons.into_iter().zip(points) {
        assert_eq!(input::pick(&ui.lua, p.0, p.1), Some(button));
    }
    run(&ui, "assert(select('#',list:EnableScroll(true))==0)");
    let list = id(&ui, "list");
    let scale = ui.ui_scale();
    let clip = lua::ui(&ui.lua).tree.rect(list);
    let quads = colored(&ui, ART);
    assert_eq!(quads.len(), 4);
    for (q, button) in quads.iter().zip(buttons) {
        let r = lua::ui(&ui.lua).tree.rect(button).intersect(&clip);
        near(q.pos, bounds(r, scale));
    }
    let mut mouse = input::MouseState::default();
    for (button, outside) in buttons.into_iter().zip(points) {
        assert_ne!(input::pick(&ui.lua, outside.0, outside.1), Some(button));
        input::update(&ui.lua, &mut mouse, Some(outside), true, false, false, 0.);
        input::update(&ui.lua, &mut mouse, Some(outside), false, true, false, 0.);
    }
    run(&ui, "assert(entered==0 and clicked==0 and selections==0)");
    let inside = [(105., 165.), (295., 165.), (165., 105.), (165., 255.)];
    for (button, p) in buttons.into_iter().zip(inside) {
        assert_eq!(input::pick(&ui.lua, p.0, p.1), Some(button));
        input::update(&ui.lua, &mut mouse, Some(p), true, false, false, 0.);
        input::update(&ui.lua, &mut mouse, Some(p), false, true, false, 0.);
    }
    run(&ui, "assert(entered==4 and clicked==4 and selections==0)");
    let u = lua::ui(&ui.lua);
    let data = u.tree.widget(list).unwrap().list.as_ref().unwrap();
    assert_eq!(data.has_data, [false, false]);
    assert_eq!(data.keys, [None, None]);
    assert_eq!((data.overed, data.selected), (-1, -1));
    assert!(data.sub_items.iter().flatten().all(|&s| u.tree.visible(s)));
}

#[test]
fn full_list_header_area_and_oversize_column_share_the_same_boundary() {
    let ui = setup();
    run(
        &ui,
        r#"
        list:EnableScroll(true)
        header=list.column[1]
        headerArt=header:CreateColorDrawable(0.67,0.23,0.41,1,'artwork')
        headerArt:AddAnchor('TOPLEFT',header,0,0);headerArt:AddAnchor('BOTTOMRIGHT',header,0,0)
    "#,
    );
    near(
        colored(&ui, OWN)[0].pos,
        bounds(
            Rect {
                x: 100.,
                y: 100.,
                w: 200.,
                h: 30.,
            },
            ui.ui_scale(),
        ),
    );
    assert_eq!(input::pick(&ui.lua, 290., 115.), Some(id(&ui, "header")));
    assert_ne!(input::pick(&ui.lua, 310., 115.), Some(id(&ui, "header")));
    // The manual top button lies above the row area but inside the list's header.
    assert_eq!(input::pick(&ui.lua, 165., 105.), Some(id(&ui, "top")));
    run(&ui, "list:SetColumnHeight(70)");
    near(
        colored(&ui, OWN)[0].pos,
        bounds(
            Rect {
                x: 100.,
                y: 100.,
                w: 200.,
                h: 70.,
            },
            ui.ui_scale(),
        ),
    );
    assert_eq!(input::pick(&ui.lua, 165., 105.), Some(id(&ui, "top")));
    run(&ui, "list:EnableScroll(false)");
    assert_eq!(input::pick(&ui.lua, 310., 115.), Some(id(&ui, "header")));
}

#[test]
fn list_owned_background_art_overlay_and_selection_images_clip_too() {
    let ui = setup();
    run(
        &ui,
        r#"
        for _,layer in ipairs({'background','artwork','overlay'}) do
          local art=list:CreateColorDrawable(0.67,0.23,0.41,1,layer)
          art:SetExtent(220,180);art:AddAnchor('TOPLEFT',list,-10,-10)
        end
        selected=list:CreateSelectedImage();overed=list:CreateOveredImage()
        list:InsertData(101,1,'first');list:InsertData(202,1,'second');list:Select(1,false)
        for _,image in ipairs({selected,overed}) do
          image:SetTexture('ui/common/hud.dds');image:SetCoords(462,233,3,3)
          image:RemoveAllAnchors();image:SetExtent(220,40)
          image:AddAnchor('TOPLEFT',list,-10,140);image:Show(true)
        end
        selected:SetColor(0.12,0.78,0.34,1);overed:SetColor(0.81,0.42,0.19,1)
    "#,
    );
    let original_uv = colored(&ui, SELECTED)[0].uv;
    run(&ui, "list:EnableScroll(true)");
    let list = id(&ui, "list");
    let scale = ui.ui_scale();
    let clip = bounds(lua::ui(&ui.lua).tree.rect(list), scale);
    assert_eq!(colored(&ui, OWN).len(), 3);
    for q in colored(&ui, OWN) {
        near(q.pos, clip);
    }
    let expected = bounds(
        Rect {
            x: 100.,
            y: 240.,
            w: 200.,
            h: 20.,
        },
        ui.ui_scale(),
    );
    for color in [SELECTED, OVERED] {
        let q = colored(&ui, color);
        assert_eq!(q.len(), 1);
        near(q[0].pos, expected);
    }
    let cropped_uv = colored(&ui, SELECTED)[0].uv;
    assert!(
        cropped_uv[0] > original_uv[0]
            && cropped_uv[2] < original_uv[2]
            && cropped_uv[3] < original_uv[3]
    );
    run(
        &ui,
        "assert(list:GetSelectedIdx()==2);assert(selections==0);list:EnableScroll(false)",
    );
    near(
        colored(&ui, SELECTED)[0].pos,
        bounds(
            Rect {
                x: 90.,
                y: 240.,
                w: 220.,
                h: 40.,
            },
            ui.ui_scale(),
        ),
    );
}

#[test]
fn native_clip_edges_are_inclusive_while_ordinary_hit_and_scroll_bounds_are_retained() {
    let ui = setup();
    run(&ui, "list:EnableScroll(true)");
    for (name, p, outside) in [
        (
            "left",
            (100., 165.),
            (f32::from_bits(100f32.to_bits() - 1), 165.),
        ),
        (
            "right",
            (300., 165.),
            (f32::from_bits(300f32.to_bits() + 1), 165.),
        ),
        (
            "top",
            (165., 100.),
            (165., f32::from_bits(100f32.to_bits() - 1)),
        ),
        (
            "bottom",
            (165., 260.),
            (165., f32::from_bits(260f32.to_bits() + 1)),
        ),
    ] {
        assert_eq!(input::pick(&ui.lua, p.0, p.1), Some(id(&ui, name)));
        assert_ne!(
            input::pick(&ui.lua, outside.0, outside.1),
            Some(id(&ui, name))
        );
    }
    // Child hit rectangles remain half-open even within a ListCtrl.
    assert_ne!(input::pick(&ui.lua, 110., 165.), Some(id(&ui, "left")));
    run(&ui, "host:EnableScroll(true);host:SetWidth(300)");
    assert_ne!(input::pick(&ui.lua, 300., 165.), Some(id(&ui, "right")));
    run(&ui, "host:EnableScroll(false)");
    assert_eq!(input::pick(&ui.lua, 300., 165.), Some(id(&ui, "right")));
}

#[test]
fn ancestor_clips_hidden_owners_and_siblings_survive_list_toggle() {
    let ui = setup();
    run(
        &ui,
        r#"
        host:SetExtent(290,250);host:EnableScroll(true);list:EnableScroll(true)
        sibling=host:CreateChildWidget('button','sibling',0,true)
        sibling:SetExtent(20,20);sibling:AddAnchor('TOPLEFT',host,70,160)
    "#,
    );
    assert!(
        colored(&ui, ART)
            .iter()
            .all(|q| q.pos[2] <= 290. * ui.ui_scale() && q.pos[3] <= 250. * ui.ui_scale())
    );
    assert_eq!(input::pick(&ui.lua, 75., 165.), Some(id(&ui, "sibling")));
    run(&ui, "list:EnableScroll(false)");
    assert_eq!(input::pick(&ui.lua, 95., 165.), Some(id(&ui, "left")));
    assert_ne!(input::pick(&ui.lua, 305., 165.), Some(id(&ui, "right")));
    run(
        &ui,
        "list:EnableScroll(true);list.items[2].subItems[1]:Show(false)",
    );
    assert!(colored(&ui, ART).is_empty());
    assert_ne!(input::pick(&ui.lua, 105., 165.), Some(id(&ui, "left")));
    run(&ui, "list:EnableScroll(false);list:EnableScroll(true)");
    assert!(colored(&ui, ART).is_empty());
    run(&ui, "list.items[2].subItems[1]:Show(true);host:Show(false)");
    assert!(colored(&ui, ART).is_empty());
    assert_eq!(input::pick(&ui.lua, 105., 165.), None);
}

#[test]
fn scroll_scale_and_resize_keep_paint_and_pick_at_the_current_list_boundary() {
    let ui = setup();
    run(
        &ui,
        r#"
        right.art:SetColor(0.67,0.23,0.41,1)
        list:ChangeChildAnchorByScrollValue('horz',20)
        UIParent:SetUIScale(0.93);host:SetScale(0.9);list:SetScale(1.25)
    "#,
    );
    let list = id(&ui, "list");
    let right = id(&ui, "right");
    for width in [200., 180., 210.] {
        run(&ui, &format!("list:SetWidth({width})"));
        run(&ui, "list:EnableScroll(false)");
        let raw = colored(&ui, OWN)[0].pos;
        run(&ui, "list:EnableScroll(true)");
        let u = lua::ui(&ui.lua);
        let clip = u.animations.widget_rect(&u.tree, list);
        let button = u.animations.widget_rect(&u.tree, right);
        let area = button.intersect(&clip);
        // Ordinary geometry snaps before clipping; compare against its actual
        // unclipped paint and the transformed, snapped scroll boundary.
        let clip_px = bounds(clip, u.ui_scale).map(f32::round);
        let expected = [
            raw[0].max(clip_px[0]),
            raw[1].max(clip_px[1]),
            raw[2].min(clip_px[2]),
            raw[3].min(clip_px[3]),
        ];
        let p = (area.x + area.w / 2., area.y + area.h / 2.);
        assert_eq!(u.tree.clip_rect(list), Some(u.tree.rect(list)));
        drop(u);
        let qs = colored(&ui, OWN);
        if area.w > 0. {
            assert!(
                qs.iter().any(|q| q
                    .pos
                    .into_iter()
                    .zip(expected)
                    .all(|(a, b)| (a - b).abs() < 0.001)),
                "{expected:?}"
            );
            assert_eq!(input::pick(&ui.lua, p.0, p.1), Some(right));
        }
        let edge = clip.right();
        assert_ne!(input::pick(&ui.lua, edge + 1., button.y + 1.), Some(right));
    }
}

fn original_world() -> UiRuntime {
    let mut ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    fake::install(&mut ui, "en_us", &fake::default_worlds());
    fake::install_ingame(&mut ui);
    ui.run_stage(aa_ui::Stage::World);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    ui
}

#[test]
fn unchanged_original_scroll_wrapper_enables_clipping_and_keeps_data_selection_contracts() {
    let ui = original_world();
    run(
        &ui,
        r#"
        UIParent:SetUIScale(1)
        host=UIParent:CreateWidget('window','originalScrollHost','UIParent')
        host:SetExtent(500,400);host:Show(true)
        frame=CreateScrollListCtrl(host,'originalClipScroll',0)
        frame:SetExtent(260,160);frame:AddAnchor('TOPLEFT',UIParent,100,100)
        frame:SetColumnHeight(30)
        frame:InsertColumn('Title',300,LCCIT_WINDOW,function(sub,data,on) sub.value=data end,nil,nil,
          function(parent,row,col,sub)
            local button=sub:CreateChildWidget('button','overflow',0,true)
            button:SetExtent(20,20);button:AddAnchor('TOPLEFT',parent.listCtrl,-10,50)
            local art=button:CreateColorDrawable(0.17,0.31,0.53,1,'artwork')
            art:AddAnchor('TOPLEFT',button,0,0);art:AddAnchor('BOTTOMRIGHT',button,0,0)
          end)
        frame:InsertRows(2,false);frame:CreateSelectedImage();frame:SetUseDoubleClick(true)
        frame:InsertData(101,1,'first');frame:InsertData(202,1,'second');frame:UpdateView()
        list=frame.listCtrl;list:Select(0)
        assert(frame:GetSelectedDataKey()==101 and frame:GetSelectedDataIndex()==1)
        assert(list.items[1].subItems[1].value=='first')
        assert(list.items[2].subItems[1].value=='second')
    "#,
    );
    let list = id(&ui, "list");
    assert!(lua::ui(&ui.lua).tree.widget(list).unwrap().clip);
    let clip = lua::ui(&ui.lua).tree.rect(list);
    assert_eq!(colored(&ui, ART).len(), 2);
    for q in colored(&ui, ART) {
        near(
            q.pos,
            bounds(
                Rect {
                    x: clip.x,
                    y: clip.y + 50.,
                    w: 10.,
                    h: 20.,
                },
                ui.ui_scale(),
            ),
        );
    }
    let button = id(&ui, "list.items[2].subItems[1].overflow");
    assert_ne!(
        input::pick(&ui.lua, clip.x - 5., clip.y + 55.),
        Some(button)
    );
    assert_eq!(
        input::pick(&ui.lua, clip.x + 5., clip.y + 55.),
        Some(button)
    );
    run(&ui, "list:EnableScroll(false)");
    assert_eq!(
        input::pick(&ui.lua, clip.x - 5., clip.y + 55.),
        Some(button)
    );
    run(
        &ui,
        "list:EnableScroll(true);assert(frame:GetSelectedDataKey()==101);assert(list:GetSelectedIdx()==1)",
    );
    let u = lua::ui(&ui.lua);
    let data = u.tree.widget(list).unwrap().list.as_ref().unwrap();
    assert!(data.use_double_click);
    assert_eq!(data.has_data, [true, true]);
    assert_eq!(
        data.keys,
        [
            Some(aa_ui::tree::ListKey::Num(1.)),
            Some(aa_ui::tree::ListKey::Num(2.))
        ]
    );
}

#[test]
fn unchanged_original_achievement_objective_explicit_false_allows_overflow() {
    let ui = original_world();
    aa_lua::run_alb(&ui.lua, "scriptsbin/x2ui/achievement/content_detail.alb").unwrap();
    run(
        &ui,
        r#"
        host=UIParent:CreateWidget('window','objectiveClipHost','UIParent')
        UIParent:SetUIScale(1)
        host:SetExtent(400,200);host:AddAnchor('TOPLEFT',UIParent,100,100);host:Show(true)
        local function upvalue(f,name)
          for i=1,32 do
            local key,value=debug.getupvalue(f,i)
            if key==name then return value end
            if key==nil then break end
          end
          error('missing original upvalue '..name)
        end
        local body=upvalue(CreateAchievementDetailWnd,'CreateBodySectionWnd')
        local originalObjective=upvalue(body,'CreateAchievementObjectiveWnd')
        objectives=originalObjective('objectives',host)
        objectives:SetExtent(200,100);objectives:AddAnchor('TOPLEFT',host,0,0)
        list=objectives.objectiveCtrl;list:SetHeight(80);list:InsertRows(1,false)
        overflow=list.items[1].subItems[1]:CreateChildWidget('button','overflow',0,true)
        overflow:SetExtent(20,20);overflow:AddAnchor('TOPLEFT',list,-10,40)
        art=overflow:CreateColorDrawable(0.17,0.31,0.53,1,'artwork')
        art:AddAnchor('TOPLEFT',overflow,0,0);art:AddAnchor('BOTTOMRIGHT',overflow,0,0)
    "#,
    );
    let list = id(&ui, "list");
    assert!(!lua::ui(&ui.lua).tree.widget(list).unwrap().clip);
    let r = lua::ui(&ui.lua).tree.rect(list);
    let button = id(&ui, "overflow");
    assert_eq!(input::pick(&ui.lua, r.x - 5., r.y + 45.), Some(button));
    near(
        colored(&ui, ART)[0].pos,
        bounds(
            Rect {
                x: r.x - 10.,
                y: r.y + 40.,
                w: 20.,
                h: 20.,
            },
            ui.ui_scale(),
        ),
    );
    run(&ui, "list:EnableScroll(true)");
    assert_ne!(input::pick(&ui.lua, r.x - 5., r.y + 45.), Some(button));
    run(&ui, "list:EnableScroll(false)");
    assert_eq!(input::pick(&ui.lua, r.x - 5., r.y + 45.), Some(button));
}
