//! Pre-edit original SKILL_UPGRADED consumer through real wheel input and range rebuilding.
#[path = "../examples/fake/slider_range.rs"]
mod original;

use aa_ui::{UiRuntime, input, lua};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
struct LogWriter(Arc<Mutex<Vec<u8>>>);
impl std::io::Write for LogWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn eval<T: mlua::FromLua>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().expect(code)
}

fn clean(ui: &UiRuntime, logs: &Arc<Mutex<Vec<u8>>>, phase: &str) {
    assert!(ui.errors.is_empty(), "ADMISSION {phase}: {:?}", ui.errors);
    let text = String::from_utf8(logs.lock().unwrap().clone()).unwrap();
    assert!(
        ![
            "runtime error",
            "stack traceback",
            "attempt to ",
            "aa-ui: event handler"
        ]
        .iter()
        .any(|marker| text.contains(marker)),
        "ADMISSION {phase}: original script error log: {text}"
    );
}

fn id(ui: &UiRuntime, name: &str) -> u32 {
    lua::value_id(&ui.lua, &eval(ui, name)).expect(name)
}

#[derive(Debug)]
struct Bounds {
    max: f32,
    value: f32,
    frame_y: f32,
    frame_h: f32,
    last_y: f32,
    last_h: f32,
    content_y: f32,
    content_h: f32,
    children: usize,
}
fn bounds(ui: &UiRuntime) -> Bounds {
    let a: [f32; 9] = eval(
        ui,
        r#"
        local f=rangeScroll
        local last=rangeNotifier.children[#rangeNotifier.children]
        local _,max=f.scroll.vs:GetMinMaxValues()
        local _,fy=f:GetOffset();local _,fh=f:GetExtent()
        local _,ly=last:GetOffset();local _,lh=last:GetExtent()
        local _,cy=f.content:GetOffset();local _,ch=f.content:GetExtent()
        return {max,f.scroll.vs:GetValue(),fy,fh,ly,lh,cy,ch,#rangeNotifier.children}
    "#,
    );
    Bounds {
        max: a[0],
        value: a[1],
        frame_y: a[2],
        frame_h: a[3],
        last_y: a[4],
        last_h: a[5],
        content_y: a[6],
        content_h: a[7],
        children: a[8] as usize,
    }
}
fn zero_origin_range(b: &Bounds) -> f32 {
    // Original UpdateScroll's INSET.bottom=10, followed by original ResetScroll's floor.
    // Every child offset/height is queried; no 50px TextBox height or font metric assumption.
    (b.last_y + b.last_h - b.frame_y + 10. - b.frame_h)
        .floor()
        .max(0.)
}
fn scroll_value(ui: &UiRuntime) -> f32 {
    let content = id(ui, "rangeContent");
    lua::ui(&ui.lua).tree.widget(content).unwrap().scroll[1]
}
fn wheel(ui: &UiRuntime, mouse: &mut input::MouseState) {
    let r: [f32; 4] = eval(
        ui,
        "local x,y=rangeContent:GetEffectiveOffset();local w,h=rangeContent:GetEffectiveExtent();return {x,y,w,h}",
    );
    let scale = lua::ui(&ui.lua).ui_scale;
    let point = ((r[0] + r[2] / 2.) / scale, (r[1] + r[3] / 2.) / scale);
    let picked = input::pick(&ui.lua, point.0, point.1)
        .expect("ADMISSION: real physical wheel pointer hits original content subtree");
    let owner = id(ui, "rangeScroll");
    let mut ancestor = Some(picked);
    while ancestor.is_some_and(|n| n != owner) {
        ancestor = lua::ui(&ui.lua).tree.get(ancestor.unwrap()).parent;
    }
    assert_eq!(
        ancestor,
        Some(owner),
        "ADMISSION: original wheel receiver chain"
    );
    input::update(&ui.lua, mouse, Some(point), false, false, false, -1.);
}

#[test]
fn original_upgrade_while_scrolled_preserves_full_range_and_last_entry_reachability() {
    let logs = Arc::new(Mutex::new(Vec::new()));
    let writer = LogWriter(logs.clone());
    let subscriber = bevy::log::tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    bevy::log::tracing::subscriber::with_default(subscriber, || {
        let mut ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
        original::install(&mut ui);
        for _ in 0..12 {
            original::upgrade(&ui);
            original::expose_original(&ui);
            clean(&ui, &logs, "original upgrade constructor/child admission");
            if bounds(&ui).children >= 8 && bounds(&ui).max >= 80. {
                break;
            }
        }
        let initial = bounds(&ui);
        assert!(
            initial.children >= 8 && initial.max >= 80.,
            "ADMISSION: overflow {initial:?}"
        );
        assert_eq!(
            scroll_value(&ui),
            0.,
            "ADMISSION: initial zero scroll origin"
        );
        assert!(
            (initial.max - zero_origin_range(&initial)).abs() < 0.01,
            "ADMISSION: initial original geometry/range {initial:?}"
        );
        let content = id(&ui, "rangeContent");
        assert!(
            lua::ui(&ui.lua).tree.widget(content).unwrap().clip,
            "ADMISSION: actual original viewport clipping enabled"
        );

        let mut mouse = input::MouseState::default();
        wheel(&ui, &mut mouse);
        let old = bounds(&ui);
        let displacement = scroll_value(&ui);
        assert!(
            displacement > 0. && (displacement - old.value).abs() < 0.01,
            "ADMISSION: original wheel/OnSliderChanged did not scroll: d={displacement} {old:?}"
        );
        clean(&ui, &logs, "before decisive next upgrade");
        println!(
            "ADMISSION original SKILL_UPGRADED16064: children={}, max={}, realwheel_value={}, content_displacement={}, script_errors=0",
            old.children, old.max, old.value, displacement
        );

        original::upgrade(&ui);
        original::expose_original(&ui);
        let after = bounds(&ui);
        assert_eq!(
            after.children,
            old.children + 1,
            "ADMISSION: next original distinct child"
        );
        assert_eq!(
            scroll_value(&ui),
            0.,
            "ADMISSION: original final positive-range wrapper restored zero origin"
        );
        let expected = zero_origin_range(&after);
        for _ in 0..64 {
            if bounds(&ui).value >= after.max - 0.01 {
                break;
            }
            wheel(&ui, &mut mouse);
        }
        let bottom = bounds(&ui);
        assert!(
            (bottom.value - after.max).abs() < 0.01,
            "ADMISSION: real wheel reached stored slider maximum: {bottom:?}"
        );
        clean(
            &ui,
            &logs,
            "after original next upgrade and wheel to bottom",
        );
        let last_bottom_in_clip = bottom.last_y + bottom.last_h - bottom.content_y;
        let reachable = last_bottom_in_clip <= bottom.content_h + 1.;
        println!(
            "RANGE_DIAGNOSTICS children={} old_d={} stored_max={} expected_max={} deficit={} last_bottom_in_clip={} clip_height={} reachable={} script_errors=0",
            after.children,
            displacement,
            after.max,
            expected,
            expected - after.max,
            last_bottom_in_clip,
            bottom.content_h,
            reachable
        );
        assert!(
            (after.max - expected).abs() < 0.01 && reachable,
            "BEHAVIOR_RED original upgrade range reset: old_d={displacement}, stored_max={}, expected_max={expected}, deficit={}, last_bottom_in_clip={last_bottom_in_clip}, clip_height={}, reachable={reachable}; before={old:?}, after={after:?}, bottom={bottom:?}",
            after.max,
            expected - after.max,
            bottom.content_h
        );
    });
}

fn finite_reversed_golden(old: f32, expected: [f32; 4]) {
    let ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    ui.exec(
        "@finite_reversed_admission",
        &format!(
            r#"
        golden=UIParent:CreateWidget('slider','finiteGolden','UIParent')
        golden:SetMinMaxValues(0,100);golden:SetValue({old},false)
        changes=0;seenCommitted=false
        golden:SetHandler('OnSliderChanged',function(self,value)
            changes=changes+1
            local lo,hi=self:GetMinMaxValues()
            seenCommitted=lo==0 and hi==10 and self:GetValue()==value
        end)
    "#
        ),
    )
    .expect("ADMISSION: finite reversed initial binding and handler");
    let initial: [f32; 4] = eval(
        &ui,
        "local lo,hi=golden:GetMinMaxValues();return {lo,hi,golden:GetValue(),changes}",
    );
    assert_eq!(
        initial,
        [0., 100., old, 0.],
        "ADMISSION: finite reversed initial state"
    );
    ui.exec("@finite_reversed_call", "golden:SetMinMaxValues(10,0)")
        .expect("ADMISSION: finite reversed native binding call");
    let actual: [f32; 4] = eval(
        &ui,
        "local lo,hi=golden:GetMinMaxValues();return {lo,hi,golden:GetValue(),changes}",
    );
    println!("FINITE_REVERSED_DIAGNOSTICS old={old} actual={actual:?} expected={expected:?}");
    assert_eq!(
        actual, expected,
        "BEHAVIOR_RED finite reversed bounds/value/callback policy old={old}"
    );
    if expected[3] != 0. {
        assert!(
            eval::<bool>(&ui, "seenCommitted"),
            "callback must observe committed normalized bounds/value"
        );
    }
}

#[test]
fn finite_reversed_clamps_old40_to10_and_notifies_once() {
    finite_reversed_golden(40., [0., 10., 10., 1.]);
}

#[test]
fn finite_reversed_retains_old5_without_false_change() {
    finite_reversed_golden(5., [0., 10., 5., 0.]);
}

fn original_ui() -> UiRuntime {
    let mut ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    original::install(&mut ui);
    ui
}
fn bare_range() -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    ui.exec(
        "@range_control",
        r#"
        range=UIParent:CreateWidget('slider','rangeControl','UIParent');range:Show(true)
        range:SetMinMaxValues(0,100);range:SetValue(40,false)
        changes=0;seen={}
        range:SetHandler('OnSliderChanged',function(self,value)
            changes=changes+1
            local lo,hi=self:GetMinMaxValues()
            table.insert(seen,{value,lo,hi,self:GetValue()})
        end)
    "#,
    )
    .unwrap();
    ui
}

#[test]
fn ordered_clamp_policy_and_committed_callback_state() {
    for (old, lo, hi, notify, value, calls) in [
        (40., 0., 10., None, 10., 1.),
        (0., 2., 6., Some(true), 2., 1.),
        (5., 0., 10., Some(true), 5., 0.),
        (5., 5., 5., None, 5., 0.),
        (40., 0., 100., None, 40., 0.),
        (40., 0., 10., Some(false), 10., 0.),
    ] {
        let ui = bare_range();
        ui.exec("@range_old", &format!("range:SetValue({old},false)"))
            .unwrap();
        let suffix = notify.map_or(String::new(), |v| format!(",{v}"));
        let returns: usize = eval(
            &ui,
            &format!("return select('#',range:SetMinMaxValues({lo},{hi}{suffix}))"),
        );
        assert_eq!(returns, 0);
        let actual: [f32; 4] = eval(
            &ui,
            "local a,b=range:GetMinMaxValues();return {a,b,range:GetValue(),changes}",
        );
        assert_eq!(actual, [lo, hi, value, calls]);
        if calls != 0. {
            assert_eq!(eval::<[f32; 4]>(&ui, "seen[1]"), [value, lo, hi, value]);
        }
        ui.exec(
            "@range_no_repeated_change",
            &format!("range:SetMinMaxValues({lo},{hi});range:SetMinMaxValues({lo},{hi},true)"),
        )
        .unwrap();
        assert_eq!(eval::<f32>(&ui, "changes"), calls);
    }
}

#[test]
fn hidden_disabled_programmatic_ranges_and_dead_guard() {
    for (visible, enabled) in [(true, true), (false, true), (true, false), (false, false)] {
        let ui = bare_range();
        ui.exec(
            "@range_flags",
            &format!("range:Show({visible});range:Enable({enabled});range:SetMinMaxValues(0,10)"),
        )
        .unwrap();
        assert_eq!(
            eval::<[f32; 2]>(&ui, "{range:GetValue(),changes}"),
            [10., 1.]
        );
        assert_eq!(eval::<bool>(&ui, "range:IsVisible()"), visible);
        assert_eq!(eval::<bool>(&ui, "range:IsEnabled()"), enabled);
    }
    let ui = bare_range();
    let dead = id(&ui, "range");
    ui.exec(
        "@range_dead",
        "range:EnableHidingIsRemove(true);range:Show(false)",
    )
    .unwrap();
    // Show schedules removal; preserve the existing frame/fade cleanup contract.
    assert!(lua::ui(&ui.lua).tree.get(dead).alive);
    lua::tick(&ui.lua, 0.);
    assert!(!lua::ui(&ui.lua).tree.get(dead).alive);
    ui.exec("@range_after_retirement", "range:SetMinMaxValues(0,0)")
        .unwrap();
    assert_eq!(eval::<usize>(&ui, "changes"), 0);
    assert_eq!(
        lua::ui(&ui.lua)
            .tree
            .widget(dead)
            .unwrap()
            .slider
            .as_ref()
            .unwrap()
            .value,
        40.
    );
    assert!(lua::ui(&ui.lua).pending.is_empty());
}

#[test]
fn range_callbacks_can_disable_hide_retire_or_set_without_stale_delivery() {
    for action in [
        "self:Enable(false)",
        "self:Show(false,50)",
        "self:EnableHidingIsRemove(true);self:Show(false)",
        "self:SetValue(7,false)",
    ] {
        let ui = bare_range();
        let owner = id(&ui, "range");
        ui.exec(
            "@range_callback_action",
            &format!(
                r#"
            range:SetHandler('OnSliderChanged',function(self,value)
                changes=changes+1;local lo,hi=self:GetMinMaxValues()
                seen={{value,lo,hi,self:GetValue()}};{action}
            end)
            range:SetMinMaxValues(0,10)
        "#
            ),
        )
        .unwrap();
        assert_eq!(eval::<[f32; 4]>(&ui, "seen"), [10., 0., 10., 10.]);
        assert_eq!(eval::<usize>(&ui, "changes"), 1);
        lua::tick(&ui.lua, 100.);
        if action.starts_with("self:EnableHiding") {
            assert!(!lua::ui(&ui.lua).tree.get(owner).alive);
        } else {
            assert_eq!(
                eval::<f32>(&ui, "range:GetValue()"),
                if action.contains("SetValue") { 7. } else { 10. }
            );
        }
        assert!(lua::ui(&ui.lua).pending.is_empty());
    }
}

#[test]
fn same_slider_range_reentry_finishes_before_outer_lua_resumes() {
    let ui = bare_range();
    ui.exec(
        "@range_reentry",
        r#"
        trace={}
        range:SetHandler('OnSliderChanged',function(self,value)
            local lo,hi=self:GetMinMaxValues()
            table.insert(trace,string.format('%g/%g/%g/%g',value,lo,hi,self:GetValue()))
            if value==10 then self:SetMinMaxValues(0,5);table.insert(trace,'returned') end
        end)
        range:SetMinMaxValues(0,10)
    "#,
    )
    .unwrap();
    assert_eq!(
        eval::<String>(&ui, "table.concat(trace,',')"),
        "10/0/10/10,5/0/5/5,returned"
    );
    ui.exec("@range_unchanged_reentry", "range:SetMinMaxValues(0,5)")
        .unwrap();
    assert_eq!(eval::<usize>(&ui, "#trace"), 3);
    assert!(lua::ui(&ui.lua).pending.is_empty());
}

#[test]
fn another_range_callback_can_retire_the_outer_receiver() {
    let ui = bare_range();
    let outer = id(&ui, "range");
    ui.exec("@range_cross_retirement",r#"
        peer=UIParent:CreateWidget('slider','rangePeer','UIParent');peer:SetMinMaxValues(0,100);peer:SetValue(40,false)
        trace={}
        peer:SetHandler('OnSliderChanged',function(self,value)
            table.insert(trace,'peer'..value)
            range:EnableHidingIsRemove(true);range:Show(false)
        end)
        range:SetHandler('OnSliderChanged',function(self,value)
            table.insert(trace,'outer'..value);peer:SetMinMaxValues(0,5);table.insert(trace,'returned')
        end)
        range:SetMinMaxValues(0,10);range:SetMinMaxValues(0,0)
    "#).unwrap();
    assert_eq!(
        eval::<String>(&ui, "table.concat(trace,',')"),
        "outer10,peer5,returned,outer0,returned"
    );
    // Hidden is still live until the existing frame cleanup; its second setter is valid.
    assert!(lua::ui(&ui.lua).tree.get(outer).alive);
    lua::tick(&ui.lua, 0.);
    assert!(!lua::ui(&ui.lua).tree.get(outer).alive);
    ui.exec("@outer_after_retirement", "range:SetMinMaxValues(2,3)")
        .unwrap();
    assert_eq!(eval::<usize>(&ui, "#trace"), 5);
    assert_eq!(
        lua::ui(&ui.lua)
            .tree
            .widget(outer)
            .unwrap()
            .slider
            .as_ref()
            .unwrap()
            .value,
        0.
    );
    assert_eq!(eval::<f32>(&ui, "peer:GetValue()"), 5.);
    assert!(lua::ui(&ui.lua).pending.is_empty());
}

#[test]
fn original_hide_removes_singleton_and_upgrade_recreates_original_children() {
    let ui = original_ui();
    original::upgrade(&ui);
    original::expose_original(&ui);
    let old = id(&ui, "rangeNotifier");
    ui.exec("@original_notifier_close", "ToggleSkillNotifyer(false)")
        .unwrap();
    lua::tick(&ui.lua, 0.);
    assert!(!lua::ui(&ui.lua).tree.get(old).alive);
    original::upgrade(&ui);
    original::expose_original(&ui);
    assert_ne!(id(&ui, "rangeNotifier"), old);
    assert_eq!(bounds(&ui).children, 1);
    assert_eq!(scroll_value(&ui), 0.);
}

#[test]
fn original_level_changed_uses_matching_id_and_current_level5_or10() {
    let ui = original_ui();
    ui.exec(
        "@level_supplier",
        "FAKE.player.unitId='40000';FAKE.player.level=4",
    )
    .unwrap();
    ui.fire_event("LEVEL_CHANGED", (5_i64, "40000"));
    assert!(lua::ui(&ui.lua).tree.find("skillInfoNotifier").is_none());
    ui.exec("@level5_supplier", "FAKE.player.level=5").unwrap();
    ui.fire_event("LEVEL_CHANGED", (5_i64, "40001"));
    assert!(lua::ui(&ui.lua).tree.find("skillInfoNotifier").is_none());
    ui.fire_event("LEVEL_CHANGED", (5_i64, "40000"));
    let owner = lua::ui(&ui.lua)
        .tree
        .find("skillInfoNotifier")
        .expect("original level5 notifier");
    let table = lua::object_table(&ui.lua, owner).unwrap().unwrap();
    ui.lua.globals().set("levelNotifier", table).unwrap();
    assert_eq!(eval::<usize>(&ui, "#levelNotifier.children"), 1);
    assert!(eval::<bool>(
        &ui,
        "levelNotifier.children[1].flameIcon~=nil"
    ));
    ui.exec("@level10_supplier", "FAKE.player.level=10")
        .unwrap();
    ui.fire_event("LEVEL_CHANGED", (10_i64, "40000"));
    assert_eq!(eval::<usize>(&ui, "#levelNotifier.children"), 2);
    assert!(ui.errors.is_empty());
}

#[test]
fn page_callback_then_range_clamp_adds_exactly_one_nested_committed_callback() {
    let ui = original_ui();
    ui.exec(
        "@coupled_original_slider",
        r#"
        coupled=CreateSlider('rangeCoupled','UIParent');coupled:SetExtent(300,26)
        coupled:AddAnchor('TOPLEFT',UIParent,100,100);coupled:Show(true)
        coupled:SetMinMaxValues(0,4);coupled:SetValue(2,false);coupled:SetPageStep(1)
        coupledLog={}
        coupled:SetHandler('OnSliderChanged',function(self,value)
            local lo,hi=self:GetMinMaxValues()
            table.insert(coupledLog,{value,lo,hi,self:GetValue()})
            if value==3 then self:SetMinMaxValues(0,1);self:SetMinMaxValues(0,1) end
        end)
    "#,
    )
    .unwrap();
    let r: [f32; 4] = eval(
        &ui,
        "local x,y=coupled:GetEffectiveOffset();local w,h=coupled:GetEffectiveExtent();return {x,y,w,h}",
    );
    let scale = lua::ui(&ui.lua).ui_scale;
    let right = ((r[0] + r[2] - 0.5) / scale, (r[1] + r[3] / 2.) / scale);
    assert_eq!(
        input::pick(&ui.lua, right.0, right.1),
        Some(id(&ui, "coupled"))
    );
    let mut mouse = input::MouseState::default();
    input::update(&ui.lua, &mut mouse, Some(right), true, false, false, 0.);
    assert_eq!(eval::<usize>(&ui, "#coupledLog"), 2);
    assert_eq!(eval::<[f32; 4]>(&ui, "coupledLog[1]"), [3., 0., 4., 3.]);
    assert_eq!(eval::<[f32; 4]>(&ui, "coupledLog[2]"), [1., 0., 1., 1.]);
    input::update(
        &ui.lua,
        &mut mouse,
        Some((right.0 - 100., right.1)),
        false,
        false,
        false,
        0.,
    );
    input::update(&ui.lua, &mut mouse, Some(right), false, true, false, 0.);
    ui.exec("@coupled_unchanged_range", "coupled:SetMinMaxValues(0,1)")
        .unwrap();
    assert_eq!(eval::<usize>(&ui, "#coupledLog"), 2);
}

#[test]
fn visual_original_notifier_scenario_has_reachable_last_entry_at85() {
    let ui = original_ui();
    ui.exec("@visual_scale", "UIParent:SetUIScale(.85)")
        .unwrap();
    let result = original::visible_range_update(&ui);
    assert!((result[1] - result[2]).abs() < 0.01);
    assert!(result[3] <= result[4] + 1.);
}
