//! Direct drawable methods share the native text-style state and renderer.

#[path = "../examples/fake/drawable_text.rs"]
mod drawable_text;
#[path = "../examples/fake/mod.rs"]
#[allow(dead_code)]
mod fake;

use aa_ui::{UiRuntime, lua, tree::Align};

fn run(ui: &UiRuntime, source: &str) {
    ui.exec("@drawable_text_test", source).unwrap();
}

fn id(ui: &UiRuntime, name: &str) -> u32 {
    lua::value_id(&ui.lua, &ui.lua.load(name).eval().unwrap()).unwrap()
}

#[test]
fn direct_and_style_methods_address_identical_text_state_and_rendered_quads() {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    run(
        &ui,
        r#"
        owner=UIParent:CreateWidget('emptywidget','textOwner','UIParent')
        owner:SetExtent(200,60);owner:AddAnchor('TOPLEFT',UIParent,100,100);owner:Show(true)
        text=owner:CreateTextDrawable('font/arial.ttf',18,'artwork')
        text:SetText('12');text:SetExtent(200,60);text:AddAnchor('TOPLEFT',owner,0,0)
        text:SetAlign(5);text:SetColor(0.2,0.7,0.9,0.6);text:SetShadow(true);text:SetOutline(true)
    "#,
    );
    let direct = aa_ui::render::build_quads(&mut lua::ui(&ui.lua), None);
    assert!(!direct.is_empty());
    let tid = id(&ui, "text");
    let style = lua::ui(&ui.lua).tree.get(tid).text().unwrap().style.clone();
    assert_eq!(style.align, Align::Right);
    assert_eq!(style.color, [0.2, 0.7, 0.9, 0.6]);
    assert!(style.shadow && style.outline);
    run(
        &ui,
        r#"
        text:SetAlign(0);text:SetColor(1,1,1,1);text:SetShadow(false);text:SetOutline(false)
        text.style:SetAlign(5);text.style:SetColor(0.2,0.7,0.9,0.6)
        text.style:SetShadow(true);text.style:SetOutline(true)
    "#,
    );
    let styled = aa_ui::render::build_quads(&mut lua::ui(&ui.lua), None);
    assert_eq!(direct.len(), styled.len());
    for (a, b) in direct.iter().zip(&styled) {
        assert_eq!(a.pos, b.pos);
        assert_eq!(a.uv, b.uv);
        assert_eq!(a.color, b.color);
    }
    assert!(direct.iter().any(|q| q.color == [0.2, 0.7, 0.9, 0.6]));
}

#[test]
fn each_native_alignment_reaches_text_and_direct_color_does_not_recolor_images() {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    run(
        &ui,
        r#"
        owner=UIParent:CreateWidget('emptywidget','styleOwner','UIParent')
        text=owner:CreateTextDrawable('font/arial.ttf',18,'artwork')
        image=owner:CreateColorDrawable(1,1,1,1,'background')
        text:SetColor(0.1,0.2,0.3,0.4);image:SetColor(0.8,0.7,0.6,0.5)
    "#,
    );
    let tid = id(&ui, "text");
    let iid = id(&ui, "image");
    for align in 0..9 {
        run(&ui, &format!("text:SetAlign({align})"));
        assert_eq!(
            lua::ui(&ui.lua).tree.get(tid).text().unwrap().style.align,
            Align::from_i(align)
        );
    }
    assert_eq!(
        lua::ui(&ui.lua).tree.get(iid).drawable().unwrap().color,
        [0.8, 0.7, 0.6, 0.5]
    );
}

#[test]
fn original_action_slot_text_uses_direct_center_alignment_and_native_font_selection() {
    let mut ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    drawable_text::install(&mut ui);
    lua::tick(&ui.lua, 100.0);
    for i in 1..=3 {
        let tid = id(&ui, &format!("nativeTextSlots[{i}].reqPointText"));
        let u = lua::ui(&ui.lua);
        let style = &u.tree.get(tid).text().unwrap().style;
        assert_eq!(style.align, Align::Center);
        assert!(u.tree.get(tid).shown);
        assert_eq!(u.tree.get(tid).text().unwrap().text, i.to_string());
        assert_eq!(
            style.font,
            ui.lua.load("FONT_PATH.SNAIL").eval::<String>().unwrap()
        );
    }
    assert!(
        !lua::ui(&ui.lua)
            .stub_calls
            .contains_key("drawable:text:SetAlign")
    );
}
