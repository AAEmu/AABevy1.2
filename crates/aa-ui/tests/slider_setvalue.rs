//! Primary pre-edit RED: original Sound Init explicitly requests same-value notifications.
//! This observes handler/data application, not audio/backend or live-navigation visual parity.
#[path = "../examples/fake/slider_setvalue.rs"]
mod original;

use aa_ui::UiRuntime;
use std::sync::{Arc, Mutex};

type Trace = (String, i64, Option<f64>);

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
        !["runtime error", "stack traceback", "attempt to ", " handler of "]
            .iter()
            .any(|marker| text.contains(marker)),
        "ADMISSION {phase}: original script/handler error log: {text}"
    );
}

#[test]
fn complete_original_sound_init_applies_unchanged_volumes_through_original_handlers() {
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
        clean(&ui, &logs, "complete original constructor/providers/handlers");
        let initial: Vec<[f64; 5]> =
            eval(&ui, "return sliderSetValueProbe.initial");
        assert_eq!(initial.len(), 5, "ADMISSION: five original Sound channels");
        let ids = [35_i64, 36, 37, 39, 40];
        for (row, id) in initial.iter().zip(ids) {
            assert_eq!(row[0], id as f64, "ADMISSION: original constructor/channel order");
            assert_eq!((row[1], row[2]), (10., 1.), "ADMISSION: constructor10,false / actual option1");
            assert!(row[3] > 0. && row[4] > 0., "ADMISSION: actual original bounds");
        }
        original::init(&ui);
        clean(&ui, &logs, "original Init and real handler/provider call observation");
        let final_values: Vec<[f64; 3]> = eval(&ui, "return sliderSetValueProbe.final");
        let trace_rows: mlua::Table = eval(&ui, "return sliderSetValueProbe.trace");
        let trace: Vec<Trace> = trace_rows.sequence_values::<mlua::Table>().map(|row| {
            let row = row.unwrap();
            (row.raw_get(1).unwrap(), row.raw_get(2).unwrap(), row.raw_get(3).unwrap())
        }).collect();
        let writes: Vec<[f64; 2]> = eval(&ui, "return sliderSetValueProbe.channelWrites");
        let provider_returns: Vec<[f64; 2]> =
            eval(&ui, "return sliderSetValueProbe.channelProviderReturns");
        let all_writes: Vec<[f64; 2]> = eval(&ui, "return sliderSetValueProbe.allWrites");
        let all_provider_returns: Vec<[f64; 2]> =
            eval(&ui, "return sliderSetValueProbe.allProviderReturns");
        assert_eq!(final_values, ids.map(|id| [id as f64, 10., 1.]).to_vec(), "ADMISSION: all unchanged finite values");
        assert_eq!(trace.iter().filter(|r| r.0 == "Init").count(), 5, "ADMISSION: all five real Init functions ran");
        println!("ORIGINAL_SOUND_SETVALUE initial={initial:?} final={final_values:?}");
        println!("ORIGINAL_SOUND_SETVALUE trace={trace:?} channel_writes={writes:?} provider_returns={provider_returns:?} all_writes={all_writes:?} all_provider_returns={all_provider_returns:?}");
        // First behavior assertion: native true must run each actual original handler,
        // even though requested10 equals constructor10. Current production predicts0.
        let expected = ids.map(|id| [id as f64, 1.]).to_vec();
        assert_eq!(writes, expected, "NATIVE SetValue(10,true): five original SetVolume/provider applications in Sound Init order");
        assert_eq!(provider_returns, expected, "actual existing storage provider completed each request");
        let mut expected_trace = Vec::new();
        for id in ids {
            expected_trace.extend([
                ("Init".into(), id, None),
                ("OnSliderChanged".into(), id, None),
                ("SetVolume".into(), id, Some(10.)),
                ("SetOptionItemValue".into(), id, Some(1.)),
                ("provider_set".into(), id, Some(1.)),
                ("provider_return".into(), id, Some(1.)),
            ]);
        }
        assert_eq!(trace, expected_trace, "original handler -> SetVolume -> provider order; nested false call does not recurse");
        assert_eq!(all_writes, expected, "quality's false Init and unchanged checkbox controls are not the five volume applications");
        assert_eq!(all_provider_returns, expected);
    });
}

fn public_value_control() -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    ui.exec("@public_value_control", r#"
        valueControl=UIParent:CreateWidget('slider','publicValueControl','UIParent')
        valueControl:EnableHidingIsRemove(false)
        valueControl:Show(true)
        valueControl:SetMinMaxValues(0,10)
        valueControl:SetValue(4,false)
        valueCalls={}
        valueControl:SetHandler('OnSliderChanged',function(self,value)
            valueCalls[#valueCalls+1]={value,self:GetValue()}
        end)
    "#).unwrap();
    ui
}

#[test]
fn explicit_boolean_values_commit_and_notify_with_exact_same_changed_and_clamped_counts() {
    for (before, request, notify, expected) in [
        (4., 4., true, 4.),
        (4., 7.5, true, 7.5),
        (10., 40., true, 10.),
        (4., 7.5, false, 7.5),
        (4., 4., false, 4.),
    ] {
        let ui = public_value_control();
        ui.exec("@public_value_case", &format!(
            "valueControl:SetValue({before},false);valueCalls={{}};assert(select('#',valueControl:SetValue({request},{notify}))==0)"
        )).unwrap();
        assert_eq!(eval::<f32>(&ui, "valueControl:GetValue()"), expected);
        let calls: Vec<[f32; 2]> = eval(&ui, "valueCalls");
        let expected_calls = if notify { vec![[expected, expected]] } else { Vec::new() };
        assert_eq!(calls, expected_calls, "before={before}, request={request}, explicit notify={notify}");
        assert!(aa_ui::lua::ui(&ui.lua).pending.is_empty());
    }
}

#[test]
fn explicit_public_values_are_programmatic_for_hidden_and_disabled_sliders() {
    for (visible, enabled) in [(true, true), (false, true), (true, false), (false, false)] {
        let ui = public_value_control();
        ui.exec("@public_value_flags", &format!(
            "valueControl:Show({visible});valueControl:Enable({enabled});valueControl:SetValue(4,true);valueControl:SetValue(7.5,false)"
        )).unwrap();
        assert_eq!(eval::<Vec<[f32; 2]>>(&ui, "valueCalls"), vec![[4., 4.]]);
        assert_eq!(eval::<f32>(&ui, "valueControl:GetValue()"), 7.5);
        assert_eq!(eval::<bool>(&ui, "valueControl:IsVisible()"), visible);
        assert_eq!(eval::<bool>(&ui, "valueControl:IsEnabled()"), enabled);
    }
}

#[test]
fn callback_retirement_keeps_live_setters_until_existing_tick_then_dead_guard_stops_them() {
    let ui = public_value_control();
    let object: mlua::Value = eval(&ui, "valueControl");
    let id = aa_ui::lua::value_id(&ui.lua, &object).unwrap();
    ui.exec("@public_value_retirement", r#"
        valueControl:EnableHidingIsRemove(true)
        valueControl:SetHandler('OnSliderChanged',function(self,value)
            valueCalls[#valueCalls+1]={value,self:GetValue()}
            self:Show(false)
        end)
        valueControl:SetValue(4,true)
        valueControl:SetValue(7.5,true)
    "#).unwrap();
    assert_eq!(eval::<Vec<[f32; 2]>>(&ui, "valueCalls"), vec![[4., 4.], [7.5, 7.5]]);
    // Existing hide completion retires after callbacks; hiding is not immediate death.
    assert!(aa_ui::lua::ui(&ui.lua).tree.get(id).alive);
    aa_ui::lua::tick(&ui.lua, 0.);
    assert!(!aa_ui::lua::ui(&ui.lua).tree.get(id).alive);
    ui.exec("@public_value_dead", "assert(select('#',valueControl:SetValue(9,true))==0)").unwrap();
    assert_eq!(eval::<Vec<[f32; 2]>>(&ui, "valueCalls"), vec![[4., 4.], [7.5, 7.5]]);
    assert_eq!(aa_ui::lua::ui(&ui.lua).tree.widget(id).unwrap().slider.as_ref().unwrap().value, 7.5);
    assert!(aa_ui::lua::ui(&ui.lua).pending.is_empty());
}

#[test]
fn finite_value_reentry_observes_committed_state_before_outer_lua_resumes() {
    let ui = public_value_control();
    ui.exec("@public_value_reentry", r#"
        reentryTrace={}
        valueControl:SetHandler('OnSliderChanged',function(self,value)
            reentryTrace[#reentryTrace+1]=string.format('%g/%g',value,self:GetValue())
            if value==4 then
                self:SetValue(7.5,true)
                reentryTrace[#reentryTrace+1]='inner-returned'
            else
                self:SetValue(value,false)
            end
        end)
        valueControl:SetValue(4,true)
        reentryTrace[#reentryTrace+1]='outer-returned'
    "#).unwrap();
    assert_eq!(eval::<String>(&ui, "table.concat(reentryTrace,',')"),
        "4/4,7.5/7.5,inner-returned,outer-returned");
    assert_eq!(eval::<f32>(&ui, "valueControl:GetValue()"), 7.5);
    assert!(aa_ui::lua::ui(&ui.lua).pending.is_empty());
}
