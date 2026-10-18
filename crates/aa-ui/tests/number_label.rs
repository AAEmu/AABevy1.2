#[path = "../examples/native_number_labels.rs"]
#[allow(dead_code)]
mod original;

use aa_ui::{UiRuntime, lua};

fn setup() -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    run(
        &ui,
        "label=UIParent:CreateWidget('label','number','UIParent');label:SetText('old')",
    );
    ui
}
fn run(ui: &UiRuntime, code: &str) {
    ui.exec("@number_label", code).unwrap();
}
fn display(ui: &UiRuntime) -> String {
    let u = lua::ui(&ui.lua);
    u.tree
        .get(u.tree.find("number").unwrap())
        .text()
        .unwrap()
        .text
        .clone()
}
fn raw(ui: &UiRuntime) -> Vec<u8> {
    ui.lua
        .load("return label:GetText()")
        .eval::<mlua::LuaString>()
        .unwrap()
        .as_bytes()
        .to_vec()
}
fn input(ui: &UiRuntime, value: &[u8]) {
    ui.lua
        .globals()
        .set("input", ui.lua.create_string(value).unwrap())
        .unwrap();
    run(ui, "assert(select('#',label:SetText(input))==0)");
}

#[test]
fn typed_flag_only_changes_future_strings_and_nonlabels_ignore_it() {
    let ui = setup();
    run(
        &ui,
        r#"
        assert(select('#',label:SetNumberOnly(true))==0);assert(label:GetText()=='old')
        label:SetText('1234');assert(label:GetText()=='1,234')
        for _,v in ipairs({'false',{},function()end}) do label:SetNumberOnly(v) end
        label:SetNumberOnly();label:SetNumberOnly(nil);label:SetText('5678')
        assert(label:GetText()=='5,678')
        label:SetNumberOnly(false);assert(label:GetText()=='5,678')
        label:SetText('5678');assert(label:GetText()=='5678')
        for _,v in ipairs({1,-1,0/0}) do
            label:SetNumberOnly(v);label:SetText('1234');assert(label:GetText()=='1,234')
        end
        label:SetNumberOnly(0);label:SetText('1234');assert(label:GetText()=='1234')
        local button=UIParent:CreateWidget('button','control','UIParent')
        assert(button.SetNumberOnly==nil)
        label.SetNumberOnly(button,true);button:SetText(1234);assert(button:GetText()=='1234')
    "#,
    );
}

#[test]
fn label_settext_requires_an_actual_string_with_zero_results() {
    let ui = setup();
    run(
        &ui,
        r#"
        for _,v in ipairs({1234,true,{},function()end}) do
            assert(select('#',label:SetText(v))==0);assert(label:GetText()=='old')
        end
        label:SetText();label:SetText(nil);assert(label:GetText()=='old')
        label:SetText('1234','ignored');assert(select('#',label:GetText())==1)
        assert(label:GetText()=='1234')
    "#,
    );
}

#[test]
fn grouping_matches_native_bytes_and_gettext_retains_the_raw_model() {
    let ui = setup();
    run(&ui, "label:SetNumberOnly(true)");
    for (source, stored, rendered) in [
        ("1234", b"1,234".as_slice(), "1,234"),
        ("-123", b"-,123".as_slice(), "-,123"),
        ("abcdefg", b"a,bcd,efg".as_slice(), "a,bcd,efg"),
        ("1,234", b"1,,234".as_slice(), "1,,234"),
        ("éé", b"".as_slice(), ""),
        ("ééé", b"\xc3\xa9\xc3,\xa9\xc3\xa9".as_slice(), "é"),
        ("Aéé", b"A\xc3,\xa9\xc3\xa9".as_slice(), "A"),
        ("友友", "友,友".as_bytes(), "友,友"),
        ("友人A", b"".as_slice(), ""),
    ] {
        input(&ui, source.as_bytes());
        assert_eq!(raw(&ui), stored, "{source}");
        assert_eq!(display(&ui), rendered, "{source}");
        run(&ui, "label:SetText(label:GetText())"); // Existing commas are grouped again.
    }
    input(&ui, b"1234\0ignored");
    assert_eq!(raw(&ui), b"1,234");
}

#[test]
fn native_model_limits_count_the_grouped_prefix_and_preserve_rejected_text() {
    let ui = setup();
    input(&ui, &vec![b'1'; 511]);
    assert_eq!(raw(&ui).len(), 511);
    input(&ui, &vec![b'2'; 512]);
    assert_eq!(raw(&ui), vec![b'1'; 511]);
    run(&ui, "label:SetNumberOnly(true)");
    input(&ui, &vec![b'1'; 384]);
    assert_eq!(raw(&ui).len(), 511);
    let prior = raw(&ui);
    input(&ui, &vec![b'2'; 385]);
    assert_eq!(raw(&ui), prior);
    run(&ui, "label:SetNumberOnly(false)");
    let mut long = b"A\x80".to_vec();
    long.extend(vec![b'B'; 2100]);
    input(&ui, &long);
    assert_eq!(raw(&ui), long[..2048]);
    assert_eq!(display(&ui), "A");
}

#[test]
fn bom_and_native_structural_decoding_preserve_the_explicit_scalar_boundary() {
    let ui = setup();
    input(&ui, b"\xef\xbb\xbfAb");
    assert_eq!(display(&ui), "Ab");
    input(&ui, b"\xef\xbb\xbfA\xc3,B");
    assert_eq!(display(&ui), "Aì");
    input(&ui, b"\xe0\x80\x81");
    assert_eq!(display(&ui), "\u{1}");
    let prior = raw(&ui);
    for bytes in [b"\xed\xa0\x80".as_slice(), b"\xf4\xbf\xbf\xbf".as_slice()] {
        input(&ui, bytes);
        assert_eq!(raw(&ui), prior);
        assert_eq!(display(&ui), "\u{1}");
    }
    input(&ui, b"\x80bad");
    assert!(raw(&ui).is_empty());
    assert!(display(&ui).is_empty());
}

#[test]
fn original_currency_and_item_stack_constructors_use_the_native_label_state() {
    let mut ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    original::install(&mut ui);
    run(
        &ui,
        r#"
        money:Update('1000000005678');assert(money.goldLabel:GetText()=='100,000,000')
        stackButton:SetStack('1000000');assert(stackButton.stack:GetText()=='1,000,000')
        stackButton:SetStack('');assert(stackButton.stack:GetText()=='')
    "#,
    );
    run(
        &ui,
        "clicks=0;stackButton:SetHandler('OnClick',function() clicks=clicks+1 end)",
    );
    let id = lua::value_id(&ui.lua, &ui.lua.load("return stackButton").eval().unwrap()).unwrap();
    let rect = lua::ui(&ui.lua).tree.rect(id);
    let p = (rect.x + rect.w / 2., rect.y + rect.h / 2.);
    assert_eq!(aa_ui::input::pick(&ui.lua, p.0, p.1), Some(id));
    let mut mouse = aa_ui::input::MouseState::default();
    aa_ui::input::update(&ui.lua, &mut mouse, Some(p), true, false, false, 0.);
    aa_ui::input::update(&ui.lua, &mut mouse, Some(p), false, true, false, 0.);
    run(&ui, "assert(clicks==1)");
}
