//! `SetTooltipData(unit)`: the unit frame tooltip's `|pN;` placeholders follow the unit's values.
use aa_ui::{UiRuntime, lua};

fn exec(ui: &UiRuntime, s: &str) {
    ui.exec("@unit_tooltip", s).unwrap()
}

fn line(ui: &UiRuntime) -> String {
    let id = lua::value_id(&ui.lua, &ui.lua.load("return tip").eval().unwrap()).unwrap();
    let u = lua::ui(&ui.lua);
    u.tree.widget(id).unwrap().tip.as_ref().unwrap().lines[0].left.text.clone()
}

#[test]
fn placeholders_take_the_bound_units_values_and_follow_changes() {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    exec(
        &ui,
        r#"
        vals = { 'Bob', 720, 800, 90, 5, 10, 50, 1, 2, 3, 4 }
        function AAUI_TooltipUnitValues(unit)
            if unit ~= 'target' then return nil end
            return vals
        end
        tip = UIParent:CreateWidget('gametooltip', 'unitTip', 'UIParent')
        tip:SetTooltipData('target')
        tip:AddLine('|p1; |p2;/|p3; (|p4;%) |p10;|p11;', '', 13, 'left', 0, 0)
        tip:Show(true)
    "#,
    );
    assert_eq!(line(&ui), "Bob 720/800 (90%) 34");
    exec(&ui, "vals[2] = 400; vals[4] = 50");
    lua::tick(&ui.lua, 16.0);
    assert_eq!(line(&ui), "Bob 400/800 (50%) 34");
    // a unit without data keeps the codes
    exec(&ui, "tip:SetTooltipData('nobody')");
    assert!(line(&ui).contains("|p2;"));
}
