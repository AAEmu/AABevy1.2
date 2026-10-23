//! `grid` cells, row heights and selection, and `OnBoundChanged`, driven from Lua without a window.

use aa_ui::UiRuntime;

fn runtime() -> UiRuntime {
    UiRuntime::new("en_us", 1920.0, 1080.0).expect("runtime")
}

fn eval<T: mlua::FromLua>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().expect(code)
}

fn tick(ui: &UiRuntime) {
    aa_ui::lua::tick(&ui.lua, 16.0);
}

#[test]
fn grid_places_cells_and_sums_rows() {
    let ui = runtime();
    ui.exec(
        "@setup",
        r#"
        grid = UIParent:CreateWidget("grid", "testGrid", "UIParent")
        grid:AddAnchor("TOPLEFT", "UIParent", 100, 50)
        grid:SetExtent(300, 200)
        grid:SetColCount(2)
        grid:SetColWidth(40, 1)
        grid:SetColWidth(150, 2)
        own = UIParent:CreateWidget("emptywidget", "own", grid)
        own:SetExtent(10, 12)
        fit = UIParent:CreateWidget("emptywidget", "fit", grid)
        -- withoutExtent: the widget keeps its size; otherwise the grid sizes it to the cell.
        grid:SetItem(own, 1, 1, true, 0, true)
        grid:SetItem(fit, 2, 2, true, 0, false)
        grid:SetRowHeight(30, 1)
        grid:SetRowHeight(20, 2)
        grid:SetItemInset(1, 1, 20, 5, 0, 0)
        grid:SetItemInset(2, 2, 5, 4, 3, 2)
        "#,
    )
    .unwrap();
    tick(&ui);
    assert_eq!(eval::<f64>(&ui, "grid:GetAdjustRowHeight()"), 50.0);
    assert_eq!(eval::<i64>(&ui, "grid:GetRowCount()"), 2);
    assert_eq!(eval::<f64>(&ui, "grid:GetRowHeight(2)"), 20.0);
    // `own` sits at the cell corner plus the inset and keeps its size.
    assert_eq!(eval::<f64>(&ui, "own:GetOffset()"), 120.0);
    assert_eq!(eval::<f64>(&ui, "select(2, own:GetOffset())"), 55.0);
    assert_eq!(eval::<f64>(&ui, "own:GetWidth()"), 10.0);
    // `fit` got the cell (column 2 is 150 wide, row 2 is 20 high) inside its insets.
    assert_eq!(eval::<f64>(&ui, "fit:GetOffset()"), 145.0);
    assert_eq!(eval::<f64>(&ui, "select(2, fit:GetOffset())"), 84.0);
    assert_eq!(eval::<f64>(&ui, "fit:GetWidth()"), 142.0);
    assert_eq!(eval::<f64>(&ui, "fit:GetHeight()"), 14.0);
}

#[test]
fn grid_row_grows_with_a_taller_widget() {
    // A folder that opens is taller than its cell: the row (and the rows below) follow.
    let ui = runtime();
    ui.exec(
        "@setup",
        r#"
        grid = UIParent:CreateWidget("grid", "testGrid", "UIParent")
        grid:SetExtent(200, 100)
        grid:SetColCount(1)
        grid:SetColWidth(200, 1)
        a = UIParent:CreateWidget("emptywidget", "a", grid)
        b = UIParent:CreateWidget("emptywidget", "b", grid)
        grid:SetItem(a, 1, 1, false, 0, false)
        grid:SetItem(b, 2, 1, false, 0, false)
        grid:SetRowHeight(25, 1)
        grid:SetRowHeight(25, 2)
        "#,
    )
    .unwrap();
    tick(&ui);
    assert_eq!(eval::<f64>(&ui, "a:GetHeight()"), 25.0);
    assert_eq!(eval::<f64>(&ui, "grid:GetAdjustRowHeight()"), 50.0);
    ui.exec("@grow", "a:SetHeight(80)").unwrap();
    assert_eq!(eval::<f64>(&ui, "grid:GetAdjustRowHeight()"), 105.0);
    tick(&ui);
    assert_eq!(eval::<f64>(&ui, "select(2, b:GetOffset())"), 80.0);
    ui.exec("@shrink", "a:SetHeight(25)").unwrap();
    tick(&ui);
    assert_eq!(eval::<f64>(&ui, "grid:GetAdjustRowHeight()"), 50.0);
    assert_eq!(eval::<f64>(&ui, "select(2, b:GetOffset())"), 25.0);
}

#[test]
fn grid_selection_follows_clicks_and_raises_the_handler() {
    let ui = runtime();
    ui.exec(
        "@setup",
        r#"
        grid = UIParent:CreateWidget("grid", "testGrid", "UIParent")
        grid:SetExtent(200, 100)
        grid:SetColCount(1)
        grid:SetColWidth(200, 1)
        grid:SetSelectedLine(true)
        grid:SetSelectedColor(0.3, 0.6, 1, 0.3)
        a = UIParent:CreateWidget("emptywidget", "a", grid)
        b = UIParent:CreateWidget("emptywidget", "b", grid)
        grid:SetItem(a, 1, 1, false, 0, false)
        grid:SetItem(b, 2, 1, false, 0, false)
        grid:SetRowHeight(20, 1)
        grid:SetRowHeight(20, 2)
        changes = 0
        grid:SetHandler("OnSelChanged", function(self) changes = changes + 1 end)
        grid:LineSelect(2)
        grid:LineSelect(2)
        "#,
    )
    .unwrap();
    // The same row twice is one change.
    assert_eq!(eval::<i64>(&ui, "changes"), 1);
    assert_eq!(eval::<i64>(&ui, "grid:GetSelectedRowIndex()"), 2);
    ui.exec("@release", "grid:ReleaseSelect()").unwrap();
    assert_eq!(eval::<i64>(&ui, "grid:GetSelectedRowIndex()"), 0);
    assert_eq!(eval::<i64>(&ui, "changes"), 1);
}

#[test]
fn bound_changed_runs_in_the_size_setter_even_for_an_identical_size_not_first_layout() {
    let ui = runtime();
    ui.exec(
        "@setup",
        r#"
        w = UIParent:CreateWidget("emptywidget", "boundWidget", "UIParent")
        w:SetExtent(100, 50)
        calls = 0
        w:SetHandler("OnBoundChanged", function(self) calls = calls + 1 end)
        "#,
    )
    .unwrap();
    tick(&ui);
    assert_eq!(eval::<i64>(&ui, "calls"), 0);
    tick(&ui);
    assert_eq!(eval::<i64>(&ui, "calls"), 0);
    ui.exec("@resize", "w:SetWidth(100);assert(calls==1);w:SetWidth(120);assert(calls==2)").unwrap();
    tick(&ui);
    assert_eq!(eval::<i64>(&ui, "calls"), 2);
}
