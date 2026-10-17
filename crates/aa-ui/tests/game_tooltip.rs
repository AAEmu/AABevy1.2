//! `gametooltip:AddLine` returns the 0-based index of the line it added (windowless test): tooltip.lua
//! keeps it (`lastLineIndex`, `firstLineIndex = lastLineIndex + 1`) to attach spacing and the right
//! hand column to a line.

use aa_ui::UiRuntime;

#[test]
fn add_line_returns_the_line_index() {
    let ui = UiRuntime::new("en_us", 1920.0, 1080.0).expect("runtime");
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
    let eval = |code: &str| -> i64 { ui.lua.load(code).eval().expect(code) };
    assert_eq!(eval("first"), 0);
    assert_eq!(eval("second"), 1);
    assert_eq!(eval("again"), 0);
}
