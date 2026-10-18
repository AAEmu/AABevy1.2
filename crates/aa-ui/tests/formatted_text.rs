//! Actual-native signed numeric token goldens and the unchanged original character control.
#[path = "../examples/native_dishonor_text.rs"]
#[allow(dead_code)]
mod original;
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
    // Existing honor/vocation behavior remains a separate compatibility boundary.
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
