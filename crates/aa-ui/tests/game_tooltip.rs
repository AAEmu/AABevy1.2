//! `gametooltip` lines (windowless tests): `AddLine` returns the 0-based index of the line it added
//! (tooltip.lua keeps it to attach spacing and the right hand column to a line), the line heights
//! the tooltip scripts size their sections from, and the widget sizing itself to its lines.

use aa_ui::UiRuntime;

fn runtime() -> UiRuntime {
    UiRuntime::new("en_us", 1920.0, 1080.0).expect("runtime")
}

fn eval<T: mlua::FromLua>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().expect(code)
}

#[test]
fn add_line_returns_the_line_index() {
    let ui = runtime();
    ui.exec(
        "@setup",
        r#"
        tip = UIParent:CreateWidget("gametooltip", "testTooltip", "UIParent")
        first = tip:AddLine("name", "", 0, "left", ALIGN_LEFT, 0)
        second = tip:AddLine("Dura 100/100", "", 0, "left", ALIGN_LEFT, 0)
        tip:ClearLines()
        again = tip:AddLine("again", "", 0, "left", ALIGN_LEFT, 0)
        "#,
    )
    .unwrap();
    assert_eq!(eval::<i64>(&ui, "first"), 0);
    assert_eq!(eval::<i64>(&ui, "second"), 1);
    assert_eq!(eval::<i64>(&ui, "again"), 0);
    assert_eq!(eval::<i64>(&ui, "return tip:GetLastLine()"), 0);
}

/// A section of the item tooltip as tooltip.lua builds it: upper and lower space around its lines,
/// the height from the `GetHeightToLastLine` differences (reference/ingame/tooltip_item: 16 per
/// 13 px row, wrapped rows without extra space, an empty line without rows).
#[test]
fn line_heights_and_spaces() {
    let ui = runtime();
    ui.exec(
        "@setup",
        r#"
        tip = UIParent:CreateWidget("gametooltip", "tip", "UIParent")
        tip:SetExtent(230, 20)
        tip:AddAnchor("TOPLEFT", "UIParent", 100, 100)
        tip:SetAutoWordwrap(true)
        tip:SetInset(10, 10, 10, 10)
        tip:SetLineSpace(0)
        _, h0 = tip:GetHeightToLastLine()
        a = tip:AddLine("Untradeable", "", 13, "left", ALIGN_LEFT, 0)
        tip:AttachUpperSpaceLine(a, 5)
        tip:AttachLowerSpaceLine(a, 5)
        only1, h1 = tip:GetHeightToLastLine()
        b = tip:AddLine("Automatically records new locations the first time you visit them. You can then use a Hereafter Stone to return to any location on the list.", "", 13, "left", ALIGN_LEFT, 0)
        tip:AttachUpperSpaceLine(b, 5)
        tip:AttachLowerSpaceLine(b, 5)
        _, h2 = tip:GetHeightToLastLine()
        c = tip:AddLine("", "", 0, "left", ALIGN_LEFT, 0)
        _, h3 = tip:GetHeightToLastLine()
        "#,
    )
    .unwrap();
    assert_eq!(eval::<f64>(&ui, "h0"), 0.0);
    assert_eq!(eval::<f64>(&ui, "h1 - h0"), 26.0);
    assert_eq!(eval::<f64>(&ui, "only1"), 21.0);
    // Four rows at 205 units (the capture's portal section is 64 high).
    assert_eq!(eval::<f64>(&ui, "h2 - h1 - 10"), 64.0);
    assert_eq!(eval::<f64>(&ui, "h3 - h2"), 0.0);
    assert_eq!(eval::<i64>(&ui, "tip:GetLineCount()"), 5);
    // The widget height follows: insets plus the lines.
    assert_eq!(eval::<f64>(&ui, "tip:GetHeight()"), 20.0 + 26.0 + 74.0);
}

/// Without word wrap (the simple text tooltips: `SetTooltip`) the width follows the text too:
/// "Bag (I)" is 58 x 36 in reference/ingame/tooltip_hud.
#[test]
fn unwrapped_tooltip_fits_its_text() {
    let ui = runtime();
    ui.exec(
        "@setup",
        r#"
        tip = UIParent:CreateWidget("gametooltip", "tip", "UIParent")
        tip:SetExtent(250, 20)
        tip:SetAutoWordwrap(false)
        tip:SetInset(10, 10, 10, 10)
        tip:AddLine("Bag |cFFFF9C27(I)", "", 0, "left", ALIGN_LEFT, 0)
        "#,
    )
    .unwrap();
    assert_eq!(eval::<f64>(&ui, "tip:GetWidth()"), 58.0);
    assert_eq!(eval::<f64>(&ui, "tip:GetHeight()"), 36.0);
}
