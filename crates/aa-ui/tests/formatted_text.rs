//! Actual-native signed numeric token goldens and the unchanged original character control.
#[path = "../examples/native_dishonor_text.rs"]
#[allow(dead_code)]
mod original;
#[path = "../examples/native_honor_living_text.rs"]
#[allow(dead_code)]
mod rewards;
use aa_ui::{
    UiRuntime, lua,
    text::{self, FontLib, INLINE_ICONS},
};
fn skull(value: &str) -> String {
    format!("{value}\u{e006}")
}

#[test]
fn native_dishonor_decoder_accepts_optional_terminator_and_canonical_wrapping_i64() {
    for (source, value) in [
        ("|d1234", "1234"),
        ("|d1234;", "1234"),
        ("|D1234;", "1234"),
        ("|d0;", "0"),
        ("|d-1234;", "-1234"),
        ("|dabc", "0"),
        ("|d12suffix", "12"),
        ("|d0x10;", "0"),
        ("|d\t\n-123;", "-123"),
        ("|d9223372036854775807", "9223372036854775807"),
        ("|d-9223372036854775808", "-9223372036854775808"),
        ("|d9223372036854775808", "-9223372036854775808"),
        ("|d18446744073709551616;", "0"),
        ("|d12|h34;", "12"),
        ("|d12\0Tail", "12"),
    ] {
        assert_eq!(text::expand_codes(source), skull(value), "{source:?}");
    }
    assert_eq!(text::expand_codes("|d;"), "");
    assert_eq!(text::expand_codes("|d"), "");
    assert_eq!(text::expand_codes("A|d12;B"), format!("A{}B", skull("12")));
    assert_eq!(
        text::expand_codes("|d  +12tail;X"),
        format!("{}X", skull("12"))
    );
}

#[test]
fn native_low_byte_numeric_quirks_do_not_use_rust_unicode_number_classification() {
    for (source, value) in [
        ("|d\u{131}\u{132};", "12"),
        ("|d\u{a0}123;", "123"),
        ("|d\u{feff}12;", "12"),
        ("|d1\u{e9}2;", "102"),
        ("|d\u{100}12;", "0"),
    ] {
        assert_eq!(text::expand_codes(source), skull(value), "{source:?}");
    }
}

#[test]
fn dishonor_drawing_preserves_full_source_color_links_and_literal_edit_text() {
    let source = "|cFF0000FFCrime |D001234;|r |kLiora,0;";
    assert_eq!(
        text::expand_codes(source),
        format!("|cFF0000FFCrime {}|r Liora", skull("1234"))
    );
    assert_eq!(text::expand_codes("||d1234;"), "||d1234;");
    // Native honor/vocation numeric tokens keep their distinct icon indices.
    assert_eq!(text::expand_codes("|h12;|l3;"), "12\u{e004}3\u{e005}");
    let mut fonts = FontLib::new("en_us");
    let draw = fonts.layout("default", 13., source, None, 0., false);
    assert_eq!(draw.icons.len(), 1);
    assert_eq!(draw.icons[0].icon, 6);
    assert_eq!(draw.links.len(), 1);
    assert_eq!(INLINE_ICONS[6], [62., 30., 16., 20.]);
    let literal = fonts.layout_edit("default", 13., "|d1234", None, 0.);
    assert!(literal.icons.is_empty());
}

#[test]
fn native_honor_living_goldens_share_signed_wrapping_numeric_and_eof_rules() {
    for (token, icon) in [('h', '\u{e004}'), ('H', '\u{e004}'), ('l', '\u{e005}'), ('L', '\u{e005}')] {
        for (body, value) in [
            ("001234", "1234"), ("-0", "0"), ("+00042", "42"), ("abc", "0"),
            ("12suffix", "12"), ("  +12tail", "12"), ("\t\n-123", "-123"),
            ("9223372036854775808", "-9223372036854775808"),
            ("18446744073709551616", "0"), ("12|d34", "12"), ("12\n34", "12"),
            ("\u{131}\u{132}", "12"), ("\u{a0}123", "123"), ("\u{feff}12", "12"),
            ("1\u{e9}2", "102"), ("\u{100}12", "0"), ("12\0Tail", "12"),
        ] {
            for terminator in ["", ";"] {
                let source = format!("|{token}{body}{terminator}");
                assert_eq!(text::expand_codes(&source), format!("{value}{icon}"), "{source:?}");
            }
        }
        assert_eq!(text::expand_codes(&format!("|{token}")), "");
        assert_eq!(text::expand_codes(&format!("|{token};")), "");
        let escaped = format!("||{token}1234;");
        assert_eq!(text::expand_codes(&escaped), escaped);
    }
}

#[test]
fn honor_living_use_source_rectangles_and_preserve_colors_links_and_literal_edit() {
    let mut fonts = FontLib::new("en_us");
    let source = "|cFF0000FFHonor |H0012;|r |kLiora,0; Vocation |L0042";
    assert_eq!(text::expand_codes(source), "|cFF0000FFHonor 12\u{e004}|r Liora Vocation 42\u{e005}");
    let draw = fonts.layout("default", 13., source, None, 0., false);
    assert_eq!(draw.icons.iter().map(|icon| icon.icon).collect::<Vec<_>>(), [4,5]);
    assert_eq!(draw.links.len(), 1);
    assert_eq!(INLINE_ICONS[4], [48.,31.,14.,21.]);
    assert_eq!(INLINE_ICONS[5], [190.,13.,17.,15.]);
    assert!(fonts.layout_edit("default", 13., "|h12;|L42", None, 0.).icons.is_empty());
}

#[test]
fn original_character_and_quest_rewards_keep_source_and_refresh_on_original_events() {
    let mut ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    rewards::install(&mut ui);
    ui.exec("@points_refresh", "fixtureHonor='00042';fixtureLiving='00083'").unwrap();
    lua::fire_ui_event(&ui.lua, "PLAYER_HONOR_POINT", vec![]);
    lua::fire_ui_event(&ui.lua, "PLAYER_LIVING_POINT", vec![]);
    ui.exec("@verify_point_source", "assert(originalHonor:GetText()=='|h00042;' and originalLiving:GetText()=='|l00083;')").unwrap();
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
}

#[test]
fn original_character_dishonor_control_keeps_source_and_refreshes_on_original_event() {
    let mut ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    let label = original::install(&mut ui);
    assert!(lua::ui(&ui.lua).tree.visible(label));
    assert_eq!(
        lua::ui(&ui.lua).tree.get(label).text().unwrap().text,
        "|d1234"
    );
    ui.exec("@crime_refresh", "fixtureCrimeRecord=5678")
        .unwrap();
    lua::fire_ui_event(&ui.lua, "CRIME_REPORTED", Vec::new());
    assert_eq!(
        lua::ui(&ui.lua).tree.get(label).text().unwrap().text,
        "|d5678"
    );
    assert_eq!(text::expand_codes("|d5678"), skull("5678"));
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
}

#[test]
fn native_plain_numeric_token_uses_wrapping_i64_without_an_icon_or_required_terminator() {
    for (body, expected) in [
        ("001234", "1234"), ("-0", "0"), ("+00042", "42"), ("abc", "0"),
        ("12suffix", "12"), ("\t\n-123", "-123"),
        ("9223372036854775808", "-9223372036854775808"),
        ("18446744073709551616", "0"), ("12|d34", "12"), ("12\n34", "12"),
        ("\u{131}\u{132}", "12"), ("\u{a0}123", "123"), ("\u{feff}12", "12"),
        ("1\u{e9}2", "102"), ("\u{100}12", "0"), ("12\0Tail", "12"),
    ] {
        for terminator in ["", ";"] {
            let source = format!("|,{body}{terminator}");
            assert_eq!(text::expand_codes(&source), expected, "{source:?}");
        }
    }
    assert_eq!(text::expand_codes("|,"), "");
    assert_eq!(text::expand_codes("|,;"), "");
    assert_eq!(text::expand_codes("||,0012;"), "||,0012;");
    assert_eq!(text::expand_codes("|cFF0000FF|,0012;|r / |,0042;"), "|cFF0000FF12|r / 42");
    let mut fonts = FontLib::new("en_us");
    let formatted = fonts.layout("default", 13., "|,001234", None, 0., false).clone();
    let plain = fonts.layout("default", 13., "1234", None, 0., false);
    assert_eq!(formatted.width, plain.width);
    assert!(formatted.icons.is_empty());
    assert!(fonts.layout_edit("default", 13., "|,001234", None, 0.).width > formatted.width);
}

#[test]
fn original_crime_plain_numeric_control_retains_raw_markup_through_refresh() {
    let mut ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    original::install(&mut ui);
    let crime = lua::ui(&ui.lua).tree.find("character.window.crimePoint").unwrap();
    let raw = lua::ui(&ui.lua).tree.get(crime).text().unwrap().text.clone();
    assert!(raw.contains("|,80;"));
    assert!(!text::expand_codes(&raw).contains("|,"));
    ui.exec("@crime_numeric_refresh", "X2Player.GetCrimeInfo=function() return {crimePoint=1234,crimeRecord=1234} end").unwrap();
    lua::fire_ui_event(&ui.lua, "CRIME_REPORTED", vec![]);
    let raw = lua::ui(&ui.lua).tree.get(crime).text().unwrap().text.clone();
    assert!(raw.contains("|,1234;"));
    assert!(text::expand_codes(&raw).ends_with("1234|r / 50"));
}
