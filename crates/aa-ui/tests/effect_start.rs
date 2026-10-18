//! Native effect visibility writes and actual low-health/ordinary-slot consumers.
use aa_ui::{UiRuntime, lua};
#[path = "../examples/fake/mod.rs"]
#[allow(dead_code)]
mod fake;
fn eval<T: mlua::FromLuaMulti>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().expect(code)
}
fn id(ui: &UiRuntime, name: &str) -> u32 {
    lua::value_id(&ui.lua, &eval(ui, name)).unwrap()
}
fn setup() -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    ui.exec(
        "@effect_setup",
        r#"
        host=UIParent:CreateWidget('window','effectStartHost','UIParent')
        host:SetExtent(200,100);host:Show(true)
        e=host:CreateEffectDrawable('ui/common/hud.dds','overlay')
        e:SetExtent(42,42);e:SetCoords(462,233,3,3);e:AddAnchor('CENTER',host,0,0)
        e:SetEffectPriority(1,'alpha',0.5,0.5)
        e:SetEffectInitialColor(1,1,1,1,0);e:SetEffectFinalColor(1,1,1,1,1)
    "#,
    )
    .unwrap();
    lua::tick(&ui.lua, 0.);
    ui
}
#[test]
fn original_low_health_warning_restarts_after_recovery_for_menu_and_player_frame() {
    let mut ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    fake::install(&mut ui, "en_us", &fake::default_worlds());
    fake::install_ingame(&mut ui);
    ui.run_stage(aa_ui::Stage::World);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    let player = lua::ui(&ui.lua).tree.find("playerFrame").unwrap();
    ui.lua
        .globals()
        .raw_set(
            "effectPlayerFixture",
            lua::object_table(&ui.lua, player).unwrap().unwrap(),
        )
        .unwrap();
    ui.exec("@original_health_sequence",r#"
        PlayerLifeAlertEffectVisibleHandler(true)
        for i=1,2 do
            FAKE.player.hp=FAKE.player.maxHp;effectPlayerFixture:UpdateAll()
            assert(not main_menu_bar.life_alert_effect:IsVisible(),'healthy menu border still marked visible')
            assert(not main_menu_bar.life_alert_effect_2:IsVisible())
            assert(not effectPlayerFixture.effect_texture:IsVisible())
            FAKE.player.hp=FAKE.player.maxHp/5;effectPlayerFixture:UpdateAll()
            assert(main_menu_bar.life_alert_effect:IsVisible() and main_menu_bar.life_alert_effect:IsRunning())
            assert(main_menu_bar.life_alert_effect_2:IsVisible() and effectPlayerFixture.effect_texture:IsVisible())
        end
    "#).unwrap();
}
#[test]
fn stop_hides_and_preserves_sample_and_configuration_for_a_real_restart() {
    let ui = setup();
    ui.exec("@start", "e:SetStartEffect(true)").unwrap();
    lua::tick(&ui.lua, 250.);
    let sample = lua::ui(&ui.lua).animations.sample(id(&ui, "e"));
    assert_eq!(sample.color.unwrap()[3], 0.5);
    ui.exec(
        "@stop",
        "assert(select('#',e:SetStartEffect(false))==0);assert(not e:IsVisible())",
    )
    .unwrap();
    lua::tick(&ui.lua, 500.);
    assert_eq!(lua::ui(&ui.lua).animations.sample(id(&ui, "e")), sample);
    ui.exec("@restart", "e:SetStartEffect(true);assert(e:IsVisible())")
        .unwrap();
    assert_eq!(
        lua::ui(&ui.lua)
            .animations
            .sample(id(&ui, "e"))
            .color
            .unwrap()[3],
        0.
    );
    lua::tick(&ui.lua, 250.);
    assert_eq!(lua::ui(&ui.lua).animations.sample(id(&ui, "e")), sample);
}
#[test]
fn typed_bool_wrappers_do_not_default_or_coerce_and_keep_zero_result_arity() {
    let ui = setup();
    ui.exec(
        "@strict_effect",
        r#"
        e:SetStartEffect(false)
        for _,call in ipairs({function()return e:SetStartEffect()end,
            function()return e:SetStartEffect(nil)end,function()return e:SetStartEffect('true')end,
            function()return e:SetStartEffect({})end})do
            assert(select('#',call())==0);assert(not e:IsVisible())
        end
        for _,value in ipairs({true,-2,5e-324,0/0,1/0})do
            assert(select('#',e:SetStartEffect(value))==0);assert(e:IsVisible())
            assert(select('#',e:SetStartEffect(0,'ignored'))==0);assert(not e:IsVisible())
        end
        image=host:CreateImageDrawable('ui/common/hud.dds','overlay');image:SetVisible(true)
        assert(select('#',e.SetStartEffect(image,false))==0);assert(image:IsVisible())
    "#,
    )
    .unwrap();
}
#[test]
fn original_action_slot_helpers_stop_every_effect_and_restart_without_replacing_art() {
    let mut ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    fake::install(&mut ui, "en_us", &fake::default_worlds());
    fake::install_ingame(&mut ui);
    ui.run_stage(aa_ui::Stage::World);
    assert!(ui.errors.is_empty());
    ui.exec("@original_slot_effects",r#"
        effectHost=UIParent:CreateWidget('emptywidget','originalEffectHost','UIParent')
        effectHost:SetExtent(200,100);effectHost:Show(true)
        effectSlot=CreateActionSlot(effectHost,'nativeEffectStartSlot',ISLOT_ACTION,1)
        effectSlot:Show(true)
        local effects={effectSlot.auto_attack_effect_bg,effectSlot.auto_attack_effect_arrow,
            effectSlot.combo_action_for_me_effect,effectSlot.combo_action_for_target_effect,
            effectSlot.toggleSkillEffectBg,effectSlot.toggleSkillEffectLine}
        for i=1,2 do
            effectSlot:Anim_auto_attack_effect(true);effectSlot:Anim_combo_aciton_for_me_effect(true)
            effectSlot:Anim_combo_aciton_for_target_effect(true);effectSlot:AnimToggleSkill(true)
            for _,effect in ipairs(effects)do assert(effect:IsVisible())end
            effectSlot.auto_attack=false;effectSlot.toggle=false;effectSlot:ResetAllAnimEffect()
            for _,effect in ipairs(effects)do assert(not effect:IsVisible())end
        end
    "#).unwrap();
}
