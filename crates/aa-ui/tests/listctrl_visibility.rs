//! Native InsertRows shows cells before data refresh, independently of row occupancy.
//! Actual UiRuntime, CPU paint traversal/quads and input; no GPU or game services.
use aa_ui::{UiRuntime, input, lua, render, tree::Id};

const ART: [f32; 4] = [0.17, 0.31, 0.53, 1.];
const LABEL: [f32; 4] = [0.91, 0.82, 0.13, 1.];

fn run(ui: &UiRuntime, code: &str) {
    ui.exec("@listctrl_visibility", code).unwrap();
    lua::tick(&ui.lua, 0.);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
}

fn id(ui: &UiRuntime, expression: &str) -> Id {
    let value = ui.lua.load(expression).eval().unwrap();
    lua::value_id(&ui.lua, &value).unwrap()
}

fn cell(row: usize, col: usize) -> String {
    format!("list.items[{row}].subItems[{col}]")
}

fn setup(types: &[u8], rows: usize) -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    run(
        &ui,
        r#"
        host=UIParent:CreateWidget('window','manualListHost','UIParent')
        host:SetExtent(620,440);host:AddAnchor('TOPLEFT',UIParent,20,30);host:Show(true)
        list=host:CreateChildWidget('listctrl','list',0,true)
        list:SetExtent(600,360);list:AddAnchor('TOPLEFT',host,0,0);list:SetColumnHeight(0)
        entered=0;clicked=0;selected=0
        list:SetHandler('OnSelChanged',function() selected=selected+1 end)
        function decorate(row)
          for _,sub in ipairs(list.items[row].subItems) do
            local label=sub:CreateChildWidget('label','manualLabel',0,true)
            label:SetExtent(160,16);label:AddAnchor('TOPLEFT',sub,6,18)
            label.style:SetFont('ui/font/yd_ygo540.ttf',13)
            label.style:SetColor(0.91,0.82,0.13,1);label:SetText('Manual guard')
            local button=sub:CreateChildWidget('button','manualButton',0,true)
            button:SetExtent(60,20);button:AddAnchor('TOPLEFT',sub,80,36)
            button:SetHandler('OnEnter',function() entered=entered+1 end)
            button:SetHandler('OnClick',function() clicked=clicked+1 end)
            local art=sub:CreateColorDrawable(0.17,0.31,0.53,1,'artwork')
            art:SetExtent(10,10);art:AddAnchor('TOPLEFT',sub,3,3);sub.manualArt=art
          end
        end
        "#,
    );
    for ty in types {
        run(
            &ui,
            &format!("list:InsertColumn({}, {ty})", 600 / types.len()),
        );
    }
    run(
        &ui,
        &format!("list:InsertRows({rows},false);for i=1,{rows} do decorate(i) end"),
    );
    ui
}

fn occupancy(ui: &UiRuntime) -> Vec<bool> {
    lua::ui(&ui.lua)
        .tree
        .widget(id(ui, "list"))
        .unwrap()
        .list
        .as_ref()
        .unwrap()
        .has_data
        .clone()
}

fn assert_cells(ui: &UiRuntime, expected: &[bool], columns: usize) {
    let u = lua::ui(&ui.lua);
    let list = u
        .tree
        .widget(id(ui, "list"))
        .unwrap()
        .list
        .as_ref()
        .unwrap();
    assert_eq!(list.rows.len(), expected.len());
    for (&row, (&shown, subs)) in list.rows.iter().zip(expected.iter().zip(&list.sub_items)) {
        assert!(u.tree.get(row).shown, "row widgets stay shown");
        assert_eq!(subs.len(), columns);
        for &sub in subs {
            assert_eq!(u.tree.get(sub).shown, shown);
            assert_eq!(u.tree.visible(sub), shown);
        }
    }
}

fn quads(ui: &UiRuntime) -> Vec<render::Quad> {
    render::build_quads(&mut lua::ui(&ui.lua), None)
}

fn colored(quads: &[render::Quad], color: [f32; 4]) -> usize {
    quads.iter().filter(|q| q.color == color).count()
}

fn point(ui: &UiRuntime, target: Id) -> (f32, f32) {
    let u = lua::ui(&ui.lua);
    let r = u.animations.widget_rect(&u.tree, target);
    (r.x + r.w / 2., r.y + r.h / 2.)
}

#[test]
fn all_column_types_and_mixed_manual_cells_reach_effective_paint_without_data() {
    for types in [&[0][..], &[1][..], &[2][..], &[0, 1, 2][..]] {
        let ui = setup(types, 6);
        assert_cells(&ui, &[true; 6], types.len());
        assert_eq!(occupancy(&ui), vec![false; 6]);
        {
            let u = lua::ui(&ui.lua);
            let l = u
                .tree
                .widget(id(&ui, "list"))
                .unwrap()
                .list
                .as_ref()
                .unwrap();
            assert!(l.keys.iter().all(Option::is_none));
        }
        let painted = quads(&ui);
        assert_eq!(colored(&painted, ART), 6 * types.len());
        for row in 1..=6 {
            for col in 1..=types.len() {
                let expression = cell(row, col);
                let sub = id(&ui, &expression);
                let label = id(&ui, &format!("{expression}.manualLabel"));
                let button = id(&ui, &format!("{expression}.manualButton"));
                let art = id(&ui, &format!("{expression}.manualArt"));
                let u = lua::ui(&ui.lua);
                for order in [u.tree.paint_order(), u.tree.fading_paint_order()] {
                    let pos = |target| order.iter().position(|&id| id == target).unwrap();
                    assert!(pos(id(&ui, "host")) < pos(id(&ui, "list")));
                    assert!(pos(id(&ui, &format!("list.items[{row}]"))) < pos(sub));
                    assert!(pos(sub) < pos(label) && pos(sub) < pos(button));
                }
                assert!(u.tree.visible(art));
                let r = u.tree.rect(sub);
                let scale = u.ui_scale;
                assert!(
                    painted.iter().any(|q| q.color == LABEL
                        && q.pos[0] >= r.x * scale
                        && q.pos[1] >= r.y * scale
                        && q.pos[2] <= (r.x + r.w) * scale
                        && q.pos[3] <= (r.y + r.h) * scale),
                    "types={types:?}, row={row}, col={col}"
                );
            }
        }
    }
}

#[test]
fn child_show_and_drawable_visibility_do_not_bypass_a_hidden_cell_or_ancestor() {
    for ancestor in ["list.items[1].subItems[1]", "host"] {
        let ui = setup(&[2], 1);
        let expression = cell(1, 1);
        let button = id(&ui, &format!("{expression}.manualButton"));
        let p = point(&ui, button);
        run(
            &ui,
            &format!(
                "{ancestor}:Show(false);{expression}.manualButton:Show(true);{expression}.manualArt:SetVisible(true)"
            ),
        );
        {
            let u = lua::ui(&ui.lua);
            assert!(!u.tree.get(id(&ui, ancestor)).shown);
            for suffix in ["manualLabel", "manualButton", "manualArt"] {
                let child = id(&ui, &format!("{expression}.{suffix}"));
                assert!(u.tree.get(child).shown);
                assert!(!u.tree.visible(child));
                assert!(!u.tree.paint_order().contains(&child));
                assert!(!u.tree.fading_paint_order().contains(&child));
            }
        }
        let hidden = quads(&ui);
        assert_eq!(colored(&hidden, ART), 0);
        assert_eq!(colored(&hidden, LABEL), 0);
        assert_ne!(input::pick(&ui.lua, p.0, p.1), Some(button));
        assert_eq!(occupancy(&ui), [false]);
        run(&ui, &format!("{ancestor}:Show(true)"));
        assert_eq!(colored(&quads(&ui), ART), 1);
        assert!(colored(&quads(&ui), LABEL) > 0);
        assert_eq!(input::pick(&ui.lua, p.0, p.1), Some(button));
        assert_eq!(occupancy(&ui), [false]);
    }
}

#[test]
fn manual_descendant_button_receives_hover_and_click_without_row_selection() {
    let ui = setup(&[2], 3);
    let button = id(&ui, "list.items[2].subItems[1].manualButton");
    let p = point(&ui, button);
    assert_eq!(input::pick(&ui.lua, p.0, p.1), Some(button));
    let mut mouse = input::MouseState::default();
    input::update(&ui.lua, &mut mouse, Some(p), false, false, false, 0.);
    assert_eq!(lua::ui(&ui.lua).hover, Some(button));
    input::update(&ui.lua, &mut mouse, Some(p), true, false, false, 0.);
    input::update(&ui.lua, &mut mouse, Some(p), false, true, false, 0.);
    run(
        &ui,
        "assert(entered==1 and clicked==1 and selected==0);assert(list:GetSelectedIdx()==0)",
    );
    assert_eq!(occupancy(&ui), [false; 3]);
    let u = lua::ui(&ui.lua);
    let l = u
        .tree
        .widget(id(&ui, "list"))
        .unwrap()
        .list
        .as_ref()
        .unwrap();
    assert!(l.keys.iter().all(Option::is_none));
    assert_eq!((l.selected, l.overed), (-1, -1));
}

#[test]
fn data_refresh_sweeps_every_column_and_preserves_existing_key_updates() {
    for types in [&[0][..], &[1][..], &[2][..], &[0, 1, 2][..]] {
        let ui = setup(types, 3);
        run(&ui, "list:InsertData(99,1,'first')");
        assert_eq!(occupancy(&ui), [true, false, false]);
        assert_cells(&ui, &[true, false, false], types.len());
        assert_eq!(colored(&quads(&ui), ART), types.len());
        run(
            &ui,
            "list.items[3].subItems[1]:Show(true);list:InsertData(99,1,'updated')",
        );
        assert_cells(&ui, &[true, false, false], types.len());
        assert_eq!(occupancy(&ui), [true, false, false]);
        run(&ui, "list:InsertData(101,1,'second')");
        assert_cells(&ui, &[true, true, false], types.len());
        assert_eq!(colored(&quads(&ui), ART), 2 * types.len());
    }
}

#[test]
fn appended_cells_show_until_unknown_delete_or_delete_all_refreshes_them() {
    let ui = setup(&[0, 1, 2], 3);
    run(
        &ui,
        "list:InsertData(99,1,'first');list:InsertRows(1,false);decorate(4)",
    );
    assert_cells(&ui, &[true, false, false, true], 3);
    assert_eq!(occupancy(&ui), [true, false, false, false]);
    assert_eq!(colored(&quads(&ui), ART), 6);
    run(&ui, "list:DeleteData(999)");
    assert_cells(&ui, &[true, false, false, false], 3);
    assert_eq!(occupancy(&ui), [true, false, false, false]);
    assert_eq!(colored(&quads(&ui), ART), 3);
    run(&ui, "list:DeleteAllDatas()");
    assert_cells(&ui, &[false; 4], 3);
    assert_eq!(occupancy(&ui), [false; 4]);
    assert_eq!(colored(&quads(&ui), ART), 0);
    run(
        &ui,
        "list:InsertRows(1,false);decorate(5);list:DeleteData(999)",
    );
    assert_cells(&ui, &[false; 5], 3);
    run(&ui, "list:InsertRows(1,false);decorate(6)");
    assert_cells(&ui, &[false, false, false, false, false, true], 3);
    run(&ui, "list:DeleteAllDatas()");
    assert_cells(&ui, &[false; 6], 3);
    assert_eq!(colored(&quads(&ui), ART), 0);
}

#[test]
fn delete_key_and_index_keep_existing_holes_and_refresh_other_cells() {
    let ui = setup(&[0, 1, 2], 4);
    run(
        &ui,
        "list:InsertData(101,1,'first');list:InsertData(202,1,'second');list.items[4].subItems[1]:Show(true);list:DeleteData(101)",
    );
    assert_eq!(occupancy(&ui), [false, true, false, false]);
    assert_cells(&ui, &[false, true, false, false], 3);
    run(
        &ui,
        "list:InsertData(303,1,'reuse');list:DeleteDataByIndex(2);list:InsertRows(1,false);decorate(5)",
    );
    assert_eq!(occupancy(&ui), [true, false, false, false, false]);
    assert_cells(&ui, &[true, false, false, false, true], 3);
    run(&ui, "list:DeleteDataByIndex(0);list:DeleteDataByIndex(6)");
    assert_cells(&ui, &[true, false, false, false, true], 3);
    run(&ui, "list:DeleteDataByIndex(1)");
    assert_cells(&ui, &[false; 5], 3);
}

#[test]
fn resizing_and_header_layout_preserve_manual_and_data_visibility_states() {
    let ui = setup(&[0, 1, 2], 6);
    let original = id(&ui, "list.items[2].subItems[3]");
    run(
        &ui,
        "list.items[2].subItems[3]:Show(false);list:SetExtent(610,420);list:SetColumnHeight(14);list:SetItemLineGap(2)",
    );
    assert_eq!(id(&ui, "list.items[2].subItems[3]"), original);
    assert_eq!(occupancy(&ui), [false; 6]);
    for row in 1..=6 {
        for col in 1..=3 {
            assert_eq!(
                lua::ui(&ui.lua).tree.get(id(&ui, &cell(row, col))).shown,
                (row, col) != (2, 3)
            );
        }
    }
    assert_eq!(colored(&quads(&ui), ART), 17);
    run(
        &ui,
        "list:InsertData(99,1,'first');list:InsertRows(1,false);decorate(7);list:SetExtent(620,490);list:SetColumnHeight(0)",
    );
    assert_cells(&ui, &[true, false, false, false, false, false, true], 3);
    assert_eq!(
        occupancy(&ui),
        [true, false, false, false, false, false, false]
    );
    assert_eq!(id(&ui, "list.items[2].subItems[3]"), original);
    assert_eq!(colored(&quads(&ui), ART), 6);
}
