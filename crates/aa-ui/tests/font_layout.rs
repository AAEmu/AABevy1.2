//! Disconfirming controls: the sample quest overlap is permitted by original font/column data.
use aa_ui::text::{DEFAULT_FONT, FontLib};
#[test]
fn original_quest_sample_native_advances_do_not_justify_padding_or_font_substitution() {
    let mut font = FontLib::new("en_us");
    // Retained FreeType2.1.10, actual fonts.g Flareserif 821, rich wrapper39273c00.
    // These do not claim Swash/FreeType coverage or absolute-origin parity.
    for (size, ascent, height) in [
        (11., 11., 13.),
        (12., 12., 15.),
        (13., 13., 16.),
        (15., 15., 18.),
    ] {
        assert_eq!(font.metrics(DEFAULT_FONT, size), (ascent, height));
    }
    assert_eq!(
        font.measure(DEFAULT_FONT, 12., "Old forests conceal distant"),
        140.
    );
    assert_eq!(font.measure(DEFAULT_FONT, 11., "In-Progress"), 54.);
    let right_start = 210. * 0.85 - 54.;
    assert_eq!(140. - right_start, 15.5);
    assert_eq!(font.measure(DEFAULT_FONT, 15., "Old forests conceal"), 129.);
    assert_eq!(font.measure(DEFAULT_FONT, 13., "In-Progress"), 65.);
    assert_eq!(210. - 65. - 129., 16.);
}
