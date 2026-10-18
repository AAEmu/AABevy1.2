//! Executed native 392c50f0 float boundaries and separate generic-button drawing.
use aa_ui::{UiRuntime, lua, render::Tex};

#[path = "../examples/native_cooldown_caption.rs"]
#[allow(dead_code)]
mod original;

fn eval<T: mlua::FromLua>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().unwrap()
}

fn counts(ui: &UiRuntime) -> (usize, usize) {
    let quads = aa_ui::render::build_quads(&mut lua::ui(&ui.lua), None);
    (
        quads.iter().filter(|q| q.corners.is_some()).count(),
        quads
            .iter()
            .filter(|q| q.corners.is_none() && q.tex == Tex::Atlas)
            .count(),
    )
}

fn slot(remain: f32, total: f32) -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    ui.lua.globals().set("remaining", remain).unwrap();
    ui.lua.globals().set("total", total).unwrap();
    ui.exec(
        "@caption_slot",
        r#"
        events={}
        AAUI.SkillContent=function() return {type='skill',id=1,
            cooldown={remain=remaining,total=total}} end
        slot=UIParent:CreateWidget('slot','captionSlot','UIParent')
        slot:SetExtent(42,42);slot:AddAnchor('CENTER',UIParent,0,0);slot:Show(true)
        slot:SetHandler('OnContentUpdated',function(self,event) table.insert(events,event) end)
        slot:EstablishSkill(1)
    "#,
    )
    .unwrap();
    lua::flush(&ui.lua);
    ui
}

#[test]
fn native_f32_arithmetic_has_strict_unit_boundaries_and_no_day_suffix() {
    // Expected strings come from actual 392c67e0/392c50f0 + XL formatting, 37-group oracle.
    for (ms, expected) in [
        (0., ""),
        (1., "1s"),
        (199., "1s"),
        (200., "1s"),
        (999., "1s"),
        (999.9999, "1s"),
        (1000., "2s"),
        (1000.0001, "2s"),
        (1500., "2s"),
        (59000., "60s"),
        (59999., "60s"),
        (59999.99609375, "61s"),
        (60000., "2m"),
        (60000.01, "2m"),
        (60999., "2m"),
        (3599000., "60m"),
        (3599999.75, "61m"),
        (3600000., "2h"),
        (3600000.25, "2h"),
        (86399000., "24h"),
        (86400000., "25h"),
        (4294967295., "1194h"),
        (f64::NAN, "-214748"),
        (f64::INFINITY, "-214748"),
    ] {
        assert_eq!(lua::slot::time_label(ms), expected, "{ms}");
    }
}

#[test]
fn slot_total_200_is_inclusive_and_short_own_timers_keep_captions() {
    let below = slot(100., 199.99998);
    assert_eq!(counts(&below), (0, 0));
    let at = slot(100., 200.);
    let c = counts(&at);
    assert!(c.0 > 0 && c.1 > 0, "{c:?}");
    let short = slot(1200., 1500.);
    assert!(counts(&short).1 > 0);
    lua::tick(&short.lua, 0.);
    let id = lua::value_id(&short.lua, &eval(&short, "slot")).unwrap();
    assert_eq!(
        lua::ui(&short.lua)
            .tree
            .widget(id)
            .unwrap()
            .cooldown
            .as_ref()
            .unwrap()
            .drawn
            .1,
        "2s"
    );
    // A short duration is deliberately not treated as evidence of native +311 GCD state.
}

#[test]
fn option_changes_current_and_later_slots_without_rebinding_or_callbacks() {
    let ui = slot(8000., 12000.);
    let id = lua::value_id(&ui.lua, &eval(&ui, "slot")).unwrap();
    let before = lua::ui(&ui.lua)
        .tree
        .widget(id)
        .unwrap()
        .cooldown
        .clone()
        .unwrap();
    let event_count = eval::<usize>(&ui, "#events");
    let on = counts(&ui);
    lua::slot::set_cooldown_numbers(&ui.lua, false);
    lua::slot::set_cooldown_numbers(&ui.lua, false);
    assert_eq!(counts(&ui), (on.0, 0));
    assert_eq!(
        lua::ui(&ui.lua)
            .tree
            .widget(id)
            .unwrap()
            .cooldown
            .as_ref()
            .unwrap(),
        &before
    );
    ui.exec("@rebind", "slot:EstablishSkill(1)").unwrap();
    assert_eq!(counts(&ui).1, 0);
    let after_bind = eval::<usize>(&ui, "#events");
    assert!(after_bind > event_count);
    lua::slot::set_cooldown_numbers(&ui.lua, true);
    assert_eq!(counts(&ui), on);
    assert_eq!(eval::<usize>(&ui, "#events"), after_bind);
    lua::tick(&ui.lua, 8100.);
    assert_eq!(counts(&ui), (0, 0));
    assert_eq!(
        eval::<usize>(
            &ui,
            "local n=0;for _,v in ipairs(events) do if v=='skill_cooldown_done' then n=n+1 end end;return n"
        ),
        1
    );
}

#[test]
fn current_caption_reads_localized_templates_and_native_byte_budget() {
    let ui = slot(8000., 12000.);
    let mut texts = aa_ui::uitext::UiTexts::new();
    texts.insert((98, "second_initial".into()), "$1 sec".into());
    ui.set_ui_texts(std::sync::Arc::new(texts)).unwrap();
    lua::tick(&ui.lua, 0.);
    let id = lua::value_id(&ui.lua, &eval(&ui, "slot")).unwrap();
    assert_eq!(
        lua::ui(&ui.lua)
            .tree
            .widget(id)
            .unwrap()
            .cooldown
            .as_ref()
            .unwrap()
            .drawn
            .1,
        "9 sec"
    );
    let mut texts = aa_ui::uitext::UiTexts::new();
    texts.insert((98, "second_initial".into()), "seconds $1".into());
    ui.set_ui_texts(std::sync::Arc::new(texts)).unwrap();
    lua::tick(&ui.lua, 0.);
    assert_eq!(
        lua::ui(&ui.lua)
            .tree
            .widget(id)
            .unwrap()
            .cooldown
            .as_ref()
            .unwrap()
            .drawn
            .1,
        "seconds"
    );
    for (template, expected) in [("$1éééé", "9ééé"), ("$1éé€", "9éé")] {
        let mut texts = aa_ui::uitext::UiTexts::new();
        texts.insert((98, "second_initial".into()), template.into());
        ui.set_ui_texts(std::sync::Arc::new(texts)).unwrap();
        lua::tick(&ui.lua, 0.);
        assert_eq!(
            lua::ui(&ui.lua)
                .tree
                .widget(id)
                .unwrap()
                .cooldown
                .as_ref()
                .unwrap()
                .drawn
                .1,
            expected
        );
    }
}

#[test]
fn every_generic_cooldown_class_keeps_its_mask_without_a_caption() {
    for class in [
        "cooldownbutton",
        "cooldownactionbutton",
        "cooldownconstantbutton",
        "cooldowninventorybutton",
    ] {
        let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
        ui.lua.globals().set("class", class).unwrap();
        ui.exec(
            "@generic",
            r#"
            button=UIParent:CreateWidget(class,'generic','UIParent')
            button:SetExtent(42,42);button:Show(true)
            button:SetCoolDown(8000,12000)
        "#,
        )
        .unwrap();
        let on = counts(&ui);
        assert!(on.0 > 0, "{class}");
        assert_eq!(on.1, 0, "{class}");
        lua::slot::set_cooldown_numbers(&ui.lua, false);
        assert_eq!(counts(&ui), on, "{class}");
        ui.exec("@generic_day", "button:SetCoolDown(86400000,90000000)")
            .unwrap();
        assert_eq!(counts(&ui).1, 0, "{class}");
        ui.exec("@ordinary_text", "button:SetText('OK')").unwrap();
        let ordinary = counts(&ui);
        assert!(ordinary.1 > 0, "{class} retains ordinary button text");
        lua::slot::set_cooldown_numbers(&ui.lua, true);
        assert_eq!(counts(&ui), ordinary);
    }
}

#[test]
fn original_action_slot_click_and_equipment_identity_survive_caption_options() {
    let mut ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    original::install(&mut ui, true);
    ui.exec(
        "@click_probe",
        r#"
        uses=0
        AAUI.UseSlot=function(kind,index,button)
            assert(kind==ISLOT_ACTION and index==1 and button=='LeftButton');uses=uses+1
        end
    "#,
    )
    .unwrap();
    lua::tick(&ui.lua, 0.);
    let slot = lua::value_id(&ui.lua, &eval(&ui, "captionSlots[1]")).unwrap();
    let button = lua::value_id(&ui.lua, &eval(&ui, "nativeCooldownButtons[1]")).unwrap();
    let before = lua::ui(&ui.lua).tree.widget(button).unwrap().clone();
    lua::slot::set_cooldown_numbers(&ui.lua, false);
    lua::slot::set_cooldown_numbers(&ui.lua, true);
    let rect = lua::ui(&ui.lua).tree.rect(slot);
    let p = (rect.x + rect.w / 2., rect.y + rect.h / 2.);
    assert_eq!(aa_ui::input::pick(&ui.lua, p.0, p.1), Some(slot));
    let mut mouse = aa_ui::input::MouseState::default();
    aa_ui::input::update(&ui.lua, &mut mouse, Some(p), true, false, false, 0.);
    aa_ui::input::update(&ui.lua, &mut mouse, Some(p), false, true, false, 0.);
    assert_eq!(eval::<usize>(&ui, "uses"), 1);
    let u = lua::ui(&ui.lua);
    let after = u.tree.widget(button).unwrap();
    assert_eq!(before.button, after.button);
    assert_eq!(before.cooldown, after.cooldown);
    assert_eq!(before.slot, after.slot);
    assert_eq!(
        u.tree
            .widget(slot)
            .unwrap()
            .cooldown
            .as_ref()
            .unwrap()
            .drawn
            .1,
        "2s"
    );
}
