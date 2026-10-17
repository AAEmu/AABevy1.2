//! Feed native slot contents through the original client's action-slot bytecode.

use aa_ui::UiRuntime;

fn eval<T: mlua::FromLua>(ui: &UiRuntime, source: &str) -> T {
    ui.lua.load(source).eval().expect(source)
}

fn color(ui: &UiRuntime) -> Vec<f32> {
    eval::<mlua::Table>(ui, "slot.stateImage:GetColor()")
        .sequence_values::<f32>()
        .collect::<mlua::Result<_>>()
        .unwrap()
}

#[test]
fn original_slot_script_dims_unusable_skills_and_colors_learning_states() {
    let ui = UiRuntime::new("en_us", 1920.0, 1080.0).unwrap();
    // Only the cosmetic helpers the action-slot constructor expects; its state logic
    // and all native widgets, drawables, slot binding and event delivery are real.
    ui.exec("@helpers", r#"
        ICON_SIZE = {DEFAULT = 42}
        TEXTURE_PATH = {HUD = "ui/hud.dds", SKILL = "ui/skill.dds"}
        FONT_PATH = {DEFAULT = "font_main", SNAIL = "font_main"}
        FONT_SIZE = {SMALL = 12, XLARGE = 18, XXLARGE = 20}
        FONT_COLOR = {SOFT_RED = {1, 0, 0, 1}, RED = {1, 0, 0, 1}, WHITE = {1, 1, 1, 1}}
        function ApplyTextColor(widget, color) widget.style:SetColor(unpack(color)) end
        function ConvertColor(value) return value / 255 end
        function HideTooltip() end
        function CreatePackIcon(slot)
          slot.packIcon = slot:CreateColorDrawable(1, 1, 1, 0, "artwork")
          slot.SetPackInfo = function() end
        end
        function CreateDyeingIcon(slot) slot.dyeingIcon = slot:CreateColorDrawable(1, 1, 1, 0, "artwork") end
        function CreateLookIcon(slot) slot.lookIcon = slot:CreateColorDrawable(1, 1, 1, 0, "artwork") end
        content = {type = "skill", id = 1, canUse = true, tooltip = {name = "First skill"}}
        AAUI.SlotContent = function() return content end
    "#).unwrap();
    aa_lua::run_alb(&ui.lua, "scriptsbin/x2ui/components/action_slot.alb").unwrap();
    ui.exec(
        "@slot",
        r#"
        NotifyEmptyActionBarSlotCount = function() end
        CancelAutoRegisteredEffect = function() end
        window = UIParent:CreateWidget("window", "stateWindow", "UIParent")
        window:SetExtent(200, 100)
        window:Show(true)
        slot = CreateActionSlot(window, "stateSlot", ISLOT_ACTION, 1)
    "#,
    )
    .unwrap();
    assert_eq!(color(&ui), vec![1.0, 1.0, 1.0, 0.0]);
    assert_eq!(eval::<String>(&ui, "slot:GetTooltip().name"), "First skill");

    ui.exec("@unusable", "content.canUse = false").unwrap();
    aa_ui::lua::slot::refresh_slots(&ui.lua).unwrap();
    assert_eq!(color(&ui), vec![0.0, 0.0, 0.0, 0.75]);

    ui.exec(
        "@unlearned",
        "content.learned = false; content.canLearn = false",
    )
    .unwrap();
    aa_ui::lua::slot::refresh_slots(&ui.lua).unwrap();
    let red = color(&ui);
    assert!((red[0] - 144.0 / 255.0).abs() < 0.001 && red[3] == 0.6);

    ui.exec("@learnable", "content.canLearn = true").unwrap();
    aa_ui::lua::slot::refresh_slots(&ui.lua).unwrap();
    let cyan = color(&ui);
    assert!((cyan[1] - 198.0 / 255.0).abs() < 0.001 && cyan[3] == 0.4);

    ui.exec(
        "@passive",
        r#"
        content = {type = "buff", id = 2, learned = false, canLearn = false, reqPoints = 5,
                   tooltip = {name = "Passive"}}
    "#,
    )
    .unwrap();
    aa_ui::lua::slot::refresh_slots(&ui.lua).unwrap();
    assert!(eval::<bool>(&ui, "slot.reqPointText:IsVisible()"));
    let points = aa_ui::lua::value_id(&ui.lua, &eval::<mlua::Value>(&ui, "slot.reqPointText")).unwrap();
    assert_eq!(aa_ui::lua::ui(&ui.lua).tree.get(points).text().unwrap().text, "5");
    assert_eq!(eval::<String>(&ui, "slot:GetTooltip().name"), "Passive");

    ui.exec("@clear", "content = nil").unwrap();
    aa_ui::lua::slot::refresh_slots(&ui.lua).unwrap();
    assert!(eval::<bool>(
        &ui,
        "slot:IsEmpty() and slot:GetTooltip() == nil and not slot.reqPointText:IsVisible()"
    ));
    assert_eq!(color(&ui), vec![1.0, 1.0, 1.0, 0.0]);
}
