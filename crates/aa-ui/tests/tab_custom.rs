//! `tab:AddNewTab(text, selectedButton, unselectedButton, window)`: the bag's tab bar, built from
//! buttons and a window the script made itself (windowless test).

use aa_ui::UiRuntime;

fn runtime() -> UiRuntime {
    UiRuntime::new("en_us", 1920.0, 1080.0).expect("runtime")
}

fn eval<T: mlua::FromLua>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().expect(code)
}

const SETUP: &str = r#"
    tab = UIParent:CreateWidget("tab", "testTab", "UIParent")
    tab:SetExtent(400, 200)
    tab:AddAnchor("TOPLEFT", "UIParent", 100, 50)
    tab:Show(true)
    tab:SetGap(0)
    tab:SetOffset(0)
    tab:SetCorner("TOPLEFT")
    win = UIParent:CreateWidget("emptywidget", "testTabWindow", tab)
    win:SetExtent(0, 0)
    sel, unsel = {}, {}
    for i = 1, 3 do
        sel[i] = UIParent:CreateWidget("button", "testSel" .. i, tab)
        sel[i]:SetExtent(40, 30)
        unsel[i] = UIParent:CreateWidget("button", "testUnsel" .. i, tab)
        unsel[i]:SetExtent(40, 30)
    end
    changed = {}
    tab:SetHandler("OnTabChanged", function(self, i) changed[#changed + 1] = i end)
    for i = 1, 3 do
        ret = tab:AddNewTab("tab" .. i, sel[i], unsel[i], win)
    end
"#;

#[test]
fn add_new_tab_lays_out_the_scripts_buttons() {
    let ui = runtime();
    ui.exec("@setup", SETUP).unwrap();
    // The window comes back, the buttons line up from the corner: 40 px apart, under the tab's top.
    assert!(eval::<bool>(&ui, "ret == win"));
    assert_eq!(eval::<f64>(&ui, "sel[1]:GetOffset()"), 100.0);
    assert_eq!(eval::<f64>(&ui, "unsel[2]:GetOffset()"), 140.0);
    assert_eq!(eval::<f64>(&ui, "unsel[3]:GetOffset()"), 180.0);
    // The shared window sits below the button row (the script anchors its dividers to it at y = -24).
    assert_eq!(eval::<f64>(&ui, "select(2, win:GetOffset())"), 80.0);
    // The first tab is selected: its selected button shows, the others show unselected buttons.
    assert!(eval::<bool>(&ui, "sel[1]:IsVisible() and not unsel[1]:IsVisible()"));
    assert!(eval::<bool>(&ui, "unsel[2]:IsVisible() and not sel[2]:IsVisible()"));
    // Each tab shows its window when selected and hides it otherwise, in tab order: the window
    // all three share ends up hidden (the bag's `tabWindow` in reference/ingame/bag).
    assert!(eval::<bool>(&ui, "not win:IsVisible()"));
    // The text goes to both buttons of a tab.
    assert_eq!(eval::<String>(&ui, "sel[2]:GetText()"), "tab2");
    assert_eq!(eval::<String>(&ui, "unsel[2]:GetText()"), "tab2");
}

#[test]
fn add_new_tab_selection_and_removal() {
    let ui = runtime();
    ui.exec("@setup", SETUP).unwrap();
    ui.exec("@select", "tab:SelectTab(3)").unwrap();
    assert_eq!(eval::<i64>(&ui, "tab:GetSelectedTab()"), 3);
    assert_eq!(eval::<i64>(&ui, "changed[#changed]"), 3);
    assert!(eval::<bool>(&ui, "sel[3]:IsVisible() and unsel[1]:IsVisible() and not sel[1]:IsVisible()"));
    // With the last tab selected, the shared window shows.
    assert!(eval::<bool>(&ui, "win:IsVisible()"));
    ui.exec("@select2", "tab:SelectTab(2)").unwrap();
    assert!(eval::<bool>(&ui, "not win:IsVisible()"));
    // RemoveAllTabs forgets the tabs but leaves the script's widgets alive (it reuses them).
    ui.exec("@clear", "tab:RemoveAllTabs()").unwrap();
    assert_eq!(eval::<i64>(&ui, "tab:GetActivateTabCount()"), 0);
    ui.exec("@again", "tab:AddNewTab('again', sel[1], unsel[1], win)").unwrap();
    assert_eq!(eval::<i64>(&ui, "tab:GetActivateTabCount()"), 1);
    assert_eq!(eval::<f64>(&ui, "sel[1]:GetOffset()"), 100.0);
    assert_eq!(eval::<String>(&ui, "sel[1]:GetText()"), "again");
}
