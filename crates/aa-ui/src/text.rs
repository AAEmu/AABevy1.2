//! Fonts, text measurement and glyph rasterisation.
//!
//! The client draws text with FreeType at integer pixel sizes and integer pen positions. We do
//! the same with `swash` (hinted outlines, 8-bit coverage): advances are rounded to whole pixels,
//! so measured widths (`GetTextWidth`) and drawn text agree exactly. Pairs of the font's `kern`
//! table move the pen as FreeType's `FT_Get_Kerning(FT_KERNING_DEFAULT)` does: scaled, cut down
//! by `ppem / 25` below 25 px and rounded to whole pixels ("Vo" is 1 px tighter at 13 px:
//! "Vocation Badges" in reference/ingame/character_info).
//!
//! Logical font paths (`ui/font/yd_ygo540.ttf`) map to a fallback chain per locale through
//! `fonts/fonts.g` (primary font + fonts that cover listed Unicode ranges).
//!
//! *Image fonts* are bitmap fonts of `ui/image_text.g` (the locale's copy first): a block
//! `infomation` with `name characterInfo`, `size 37`, `width 29`, `height 37` and one
//! `glyph ( key 1, x 0, y 465, insetLeft 0, insetRight 14)` per character, cut from
//! `texturepath ui/image_text.dds`. The scripts select them by name
//! (`style:SetFont("characterInfo", 37)`: the level digits of the character window; `actionBar`:
//! the menu bar badges; `combat`, `arrest`). A glyph advances `width - insetLeft - insetRight`
//! (the level label "1" is 15 px wide in reference/ingame/character_info) and is drawn in the
//! texture's own colours, scaled by `size / font size`.

use std::collections::HashMap;

use swash::scale::{Render, ScaleContext, Source};
use swash::zeno::Format;
use swash::{CacheKey, FontRef};

pub const DEFAULT_FONT: &str = "ui/font/yd_ygo540.ttf";

/// Side of the square glyph atlas (RGBA8, white with coverage in alpha).
pub const ATLAS_SIZE: u32 = 2048;

struct Face {
    data: Vec<u8>,
    offset: u32,
    key: CacheKey,
    /// Horizontal kerning pairs (left glyph, right glyph) -> font units, from `kern`.
    kern: HashMap<(u16, u16), i16>,
    units_per_em: u16,
}

/// Format 0 subtables of a TrueType `kern` table (version 0) that kern horizontally.
fn parse_kern(t: &[u8]) -> HashMap<(u16, u16), i16> {
    let u16_at = |o: usize| t.get(o..o + 2).map(|b| u16::from_be_bytes([b[0], b[1]]));
    let mut out = HashMap::new();
    if u16_at(0) != Some(0) {
        return out;
    }
    let n = u16_at(2).unwrap_or(0);
    let mut sub = 4usize;
    for _ in 0..n {
        let (Some(len), Some(cov)) = (u16_at(sub + 2), u16_at(sub + 4)) else {
            break;
        };
        // Format 0, horizontal, not minimum values, not cross-stream.
        if cov >> 8 == 0 && cov & 0b111 == 0b001 {
            let pairs = u16_at(sub + 6).unwrap_or(0) as usize;
            for i in 0..pairs {
                let o = sub + 14 + 6 * i;
                if let (Some(l), Some(r), Some(v)) = (u16_at(o), u16_at(o + 2), u16_at(o + 4)) {
                    out.insert((l, r), v as i16);
                }
            }
        }
        sub += len as usize;
    }
    out
}

/// A kerning value in whole pixels at `ppem`, as FreeType's `FT_KERNING_DEFAULT` gives it.
fn kern_px(v: i16, ppem: u16, upem: u16) -> f32 {
    if upem == 0 {
        return 0.0;
    }
    let mut x = (v as f64 * ppem as f64 * 64.0 / upem as f64).round() as i64;
    if ppem < 25 {
        x = (x as f64 * ppem as f64 / 25.0).round() as i64;
    }
    (((x + 32) & !63) / 64) as f32
}

impl Face {
    fn font(&self) -> FontRef<'_> {
        FontRef {
            data: &self.data,
            offset: self.offset,
            key: self.key,
        }
    }
}

/// One link of a fallback chain: a face and the code point ranges it is used for
/// (empty = everything not claimed by a ranged face).
struct ChainLink {
    face: usize,
    ranges: Vec<(u32, u32)>,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct GlyphKey {
    face: u16,
    size: u16,
    glyph: u16,
    outline: bool,
}

/// A rasterised glyph in the atlas.
#[derive(Clone, Copy, Debug, Default)]
pub struct Glyph {
    /// Atlas pixel rectangle.
    pub ax: u32,
    pub ay: u32,
    pub w: u32,
    pub h: u32,
    /// Offset of the bitmap's top-left from the pen position on the baseline (y down).
    pub left: i32,
    pub top: i32,
}

/// One laid-out glyph, relative to the text block's top-left.
#[derive(Clone, Copy, Debug)]
pub struct PlacedGlyph {
    pub x: f32,
    pub y: f32,
    pub line: u16,
    pub glyph: Glyph,
    pub color: Option<[f32; 4]>,
}

/// An inline icon in laid out text (money coins, loyalty token): `x`, `y` = top left in the
/// block, `icon` indexes [`INLINE_ICONS`].
#[derive(Clone, Copy, Debug)]
pub struct PlacedIcon {
    pub x: f32,
    pub y: f32,
    pub line: u16,
    pub icon: usize,
}

/// Inline icons of the value codes, in `ui/common/money_window.dds`: (x, y, w, h) texels, drawn
/// 1 px after the text and centred on the line (the honor medal hangs over the line's top:
/// `reference/ingame/character_info`, Honor Points and Vocation Badges).
pub const INLINE_ICON_TEXTURE: &str = "ui/common/money_window.dds";
pub const INLINE_ICONS: [[f32; 4]; 6] = [
    [0.0, 29.0, 16.0, 16.0],   // gold coin
    [16.0, 29.0, 16.0, 16.0],  // silver coin
    [32.0, 29.0, 16.0, 16.0],  // copper coin
    [206.0, 12.0, 16.0, 16.0], // loyalty token (BM mileage)
    [47.0, 31.0, 16.0, 20.0],  // honor points medal (`|h`)
    [189.0, 12.0, 17.0, 16.0], // vocation badge sprout (`|l`, living points)
];
/// First private-use code point standing for an inline icon in expanded text.
const ICON_BASE: u32 = 0xE000;
const ICON_ADVANCE: f32 = 16.0;

/// A glyph of an image font: `src` = texel rectangle (x, y, w, h) in `LaidOutText::image_texture`,
/// drawn at `x, y` (top left in the block) with size `w, h`.
#[derive(Clone, Copy, Debug)]
pub struct PlacedImage {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub line: u16,
    pub src: [f32; 4],
}

/// Inputs of a cached layout (sizes as `f32` bits).
#[derive(Debug)]
struct LayoutKey {
    font: String,
    text: String,
    size: u32,
    max_width: Option<u32>,
    line_space: u32,
    outline: bool,
    edit: bool,
}

const LAYOUT_CACHE_MAX: usize = 8192;

/// A bitmap font of `ui/image_text.g`.
#[derive(Clone, Debug)]
struct ImageFont {
    texture: String,
    size: f32,
    width: f32,
    height: f32,
    /// Character -> (x, y, insetLeft, insetRight) in texels.
    glyphs: HashMap<char, [f32; 4]>,
}

#[derive(Clone, Debug, Default)]
pub struct LaidOutText {
    pub glyphs: Vec<PlacedGlyph>,
    pub icons: Vec<PlacedIcon>,
    /// Glyphs of an image font, all from `image_texture`.
    pub images: Vec<PlacedImage>,
    pub image_texture: Option<String>,
    pub width: f32,
    pub height: f32,
    pub line_widths: Vec<f32>,
    pub line_height: f32,
    /// Character-boundary caret positions for literal edit layouts, including trailing lines.
    pub carets: Vec<TextCaret>,
    /// Literal character ranges for visual edit lines, excluding a wrapping separator/newline.
    pub line_ranges: Vec<(usize, usize)>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TextCaret {
    pub x: f32,
    pub line: usize,
}

pub struct FontLib {
    faces: Vec<Face>,
    face_by_path: HashMap<String, usize>,
    chains: HashMap<String, Vec<ChainLink>>,
    scale: ScaleContext,
    glyphs: HashMap<GlyphKey, Option<Glyph>>,
    /// RGBA8 atlas pixels; `dirty` when changed since the last upload.
    pub atlas: Vec<u8>,
    pub atlas_dirty: bool,
    shelf_x: u32,
    shelf_y: u32,
    shelf_h: u32,
    advances: HashMap<(u16, u16, u16), f32>,
    /// Finished layouts by their inputs: the HUD lays out the same texts every frame (each tree
    /// change rebuilds every text block, natives measure). Glyphs never leave the atlas, so a
    /// layout stays valid; the cache is emptied when it grows past `LAYOUT_CACHE_MAX`.
    layouts: HashMap<u64, Vec<(LayoutKey, LaidOutText)>>,
    layouts_len: usize,
    /// Image fonts by lower-case name.
    image_fonts: HashMap<String, ImageFont>,
}

impl FontLib {
    /// Load `fonts/fonts.g` for `locale` (e.g. `en_us`).
    pub fn new(locale: &str) -> FontLib {
        let mut lib = FontLib {
            faces: Vec::new(),
            face_by_path: HashMap::new(),
            chains: HashMap::new(),
            scale: ScaleContext::new(),
            glyphs: HashMap::new(),
            atlas: vec![0; (ATLAS_SIZE * ATLAS_SIZE * 4) as usize],
            atlas_dirty: true,
            shelf_x: 1,
            shelf_y: 1,
            shelf_h: 0,
            advances: HashMap::new(),
            layouts: HashMap::new(),
            layouts_len: 0,
            image_fonts: HashMap::new(),
        };
        // A white texel at (0,0) for solid quads.
        lib.atlas[..4].copy_from_slice(&[255, 255, 255, 255]);
        match aa_pak::Pak::global().read_string("fonts/fonts.g") {
            Ok(text) => lib.parse_fonts_g(&text, locale),
            Err(e) => bevy::log::warn!("aa-ui: fonts/fonts.g: {e}"),
        }
        let image_text = [
            format!("ui/{locale}/image_text.g"),
            "ui/image_text.g".to_string(),
        ]
        .into_iter()
        .find_map(|p| aa_pak::Pak::global().read_string(&p).ok());
        if let Some(text) = image_text {
            lib.image_fonts = parse_image_fonts(&text);
        }
        lib
    }

    /// Lay out `text` in an image font (line breaks at `\n`; characters without a glyph are
    /// skipped).
    fn layout_image(f: &ImageFont, size: f32, text: &str, line_space: f32) -> LaidOutText {
        let scale = if f.size > 0.0 { size / f.size } else { 1.0 };
        let line_h = (f.height * scale).round();
        let mut out = LaidOutText {
            line_height: line_h,
            image_texture: Some(f.texture.clone()),
            ..Default::default()
        };
        let plain = strip_colors(&expand_codes(text));
        let lines: Vec<&str> = plain.split('\n').collect();
        for (li, line) in lines.iter().enumerate() {
            let y = li as f32 * (line_h + line_space);
            let mut x = 0.0f32;
            for ch in line.chars().filter(|&c| c != '\r') {
                let Some(&[gx, gy, il, ir]) = f.glyphs.get(&ch) else {
                    continue;
                };
                let w = (f.width - il - ir).max(0.0);
                let dw = (w * scale).round();
                out.images.push(PlacedImage {
                    x,
                    y,
                    w: dw,
                    h: line_h,
                    line: li as u16,
                    src: [gx + il, gy, w, f.height],
                });
                x += dw;
            }
            out.line_widths.push(x);
        }
        out.width = out.line_widths.iter().copied().fold(0.0, f32::max);
        let n = lines.len() as f32;
        out.height = n * line_h + (n - 1.0) * line_space;
        out
    }

    fn parse_fonts_g(&mut self, text: &str, locale: &str) {
        let mut in_locale = false;
        let mut logical: Option<String> = None;
        let mut chain: Vec<ChainLink> = Vec::new();
        let flush =
            |lib: &mut FontLib, logical: &mut Option<String>, chain: &mut Vec<ChainLink>| {
                if let Some(l) = logical.take() {
                    lib.chains.insert(l, std::mem::take(chain));
                }
            };
        for raw in text.lines() {
            let line = raw.trim_end();
            if line.trim().is_empty() {
                continue;
            }
            let indent = line.len() - line.trim_start().len();
            let item = line.trim().trim_start_matches('\u{feff}');
            match indent {
                0 => {
                    if in_locale {
                        flush(self, &mut logical, &mut chain);
                    }
                    in_locale = item.eq_ignore_ascii_case(locale);
                }
                _ if !in_locale => {}
                1..=5 => {
                    flush(self, &mut logical, &mut chain);
                    logical = Some(aa_pak::normalize(item));
                }
                6..=9 => {
                    let path = aa_pak::normalize(item);
                    let path = path.strip_prefix("game/").unwrap_or(&path).to_string();
                    if let Some(face) = self.face(&path) {
                        chain.push(ChainLink {
                            face,
                            ranges: Vec::new(),
                        });
                    }
                }
                _ => {
                    let nums: Vec<u32> = item
                        .trim_matches(|c| c == '(' || c == ')')
                        .split(',')
                        .filter_map(|s| s.trim().parse().ok())
                        .collect();
                    if let (Some(link), [a, b]) = (chain.last_mut(), nums.as_slice()) {
                        link.ranges.push((*a, *b));
                    }
                }
            }
        }
        if in_locale {
            flush(self, &mut logical, &mut chain);
        }
    }

    fn face(&mut self, path: &str) -> Option<usize> {
        if let Some(&i) = self.face_by_path.get(path) {
            return Some(i);
        }
        let data = match aa_pak::Pak::global().read(path) {
            Ok(d) => d,
            Err(e) => {
                bevy::log::warn!("aa-ui: font {path}: {e}");
                return None;
            }
        };
        let (offset, key, kern, units_per_em) = {
            let f = FontRef::from_index(&data, 0)?;
            let kern = f
                .table(swash::tag_from_bytes(b"kern"))
                .map(parse_kern)
                .unwrap_or_default();
            (f.offset, f.key, kern, f.metrics(&[]).units_per_em)
        };
        let i = self.faces.len();
        self.faces.push(Face {
            data,
            offset,
            key,
            kern,
            units_per_em,
        });
        self.face_by_path.insert(path.to_string(), i);
        Some(i)
    }

    /// Fallback chain of a logical font path; unknown paths load the file itself.
    fn chain_for(&mut self, font: &str) -> Vec<(usize, Vec<(u32, u32)>)> {
        let key = aa_pak::normalize(font);
        if !self.chains.contains_key(&key) {
            let chain = match self.face(&key) {
                Some(face) => vec![ChainLink {
                    face,
                    ranges: Vec::new(),
                }],
                None => match self.chains.get(DEFAULT_FONT) {
                    Some(c) => c
                        .iter()
                        .map(|l| ChainLink {
                            face: l.face,
                            ranges: l.ranges.clone(),
                        })
                        .collect(),
                    None => Vec::new(),
                },
            };
            self.chains.insert(key.clone(), chain);
        }
        self.chains[&key]
            .iter()
            .map(|l| (l.face, l.ranges.clone()))
            .collect()
    }

    /// Face and glyph id for a character.
    fn lookup(&self, chain: &[(usize, Vec<(u32, u32)>)], ch: char) -> Option<(usize, u16)> {
        let c = ch as u32;
        // Ranged links take their ranges, the unranged (primary) link the rest.
        for (face, ranges) in chain {
            if !ranges.is_empty() && ranges.iter().any(|&(a, b)| c >= a && c <= b) {
                let g = self.faces[*face].font().charmap().map(c);
                if g != 0 {
                    return Some((*face, g));
                }
            }
        }
        for (face, ranges) in chain {
            if ranges.is_empty() {
                let g = self.faces[*face].font().charmap().map(c);
                if g != 0 {
                    return Some((*face, g));
                }
            }
        }
        for (face, _) in chain {
            let g = self.faces[*face].font().charmap().map(c);
            if g != 0 {
                return Some((*face, g));
            }
        }
        chain.first().map(|(f, _)| (*f, 0))
    }

    fn advance(&mut self, face: usize, size: u16, glyph: u16) -> f32 {
        let key = (face as u16, size, glyph);
        if let Some(&a) = self.advances.get(&key) {
            return a;
        }
        let a = self.faces[face]
            .font()
            .glyph_metrics(&[])
            .scale(size as f32)
            .advance_width(glyph)
            .round();
        self.advances.insert(key, a);
        a
    }

    /// Whether the configured font chain has every visible character (combat extraStyle).
    pub(crate) fn supports(&mut self, font: &str, text: &str) -> bool {
        if let Some(f) = self.image_fonts.get(&font.to_ascii_lowercase()) {
            return text.chars().all(|c| c.is_whitespace() || f.glyphs.contains_key(&c));
        }
        let chain = self.chain_for(font);
        text.chars().all(|c| c.is_whitespace() || self.lookup(&chain, c).is_some_and(|(_, glyph)| glyph != 0))
    }

    /// Ascent (rounded) and line height for a font size.
    pub fn metrics(&mut self, font: &str, size: f32) -> (f32, f32) {
        if let Some(f) = self.image_fonts.get(&font.to_ascii_lowercase()) {
            let h = (f.height * if f.size > 0.0 { size / f.size } else { 1.0 }).round();
            return (h, h);
        }
        let chain = self.chain_for(font);
        let px = size.round().max(1.0);
        match chain.first() {
            Some((face, _)) => {
                // The client's line height is the font's ascent + descent, rounded (yd_ygo540:
                // 13 px -> 16, 15 -> 18, 18 -> 22; yoon_snail_b 15 -> 19; checked against
                // GetTextHeight in reference/*/ui.json). The baseline sits at the ascent rounded
                // up, as FreeType grid-fits it: LibreBaskerville at 18 px (17.46) puts the window
                // titles 1 px lower than rounding would (reference/ingame/character_info, bag).
                let m = self.faces[*face].font().metrics(&[]).scale(px);
                (m.ascent.ceil(), (m.ascent + m.descent).round())
            }
            None => (px * 0.8, (px * 1.2).round()),
        }
    }

    /// Width of the longest line of `text` (colour codes stripped).
    pub fn measure(&mut self, font: &str, size: f32, text: &str) -> f32 {
        self.layout_size(font, size, text, None, 0.0).0
    }

    /// Lay out `text` from the top-left of its block: lines break at `\n` and, with
    /// `max_width`, at spaces (or anywhere for words longer than a line). Glyphs are rasterised
    /// into the atlas as needed.
    pub fn layout(
        &mut self,
        font: &str,
        size: f32,
        text: &str,
        max_width: Option<f32>,
        line_space: f32,
        outline: bool,
    ) -> LaidOutText {
        self.layout_ref(font, size, text, max_width, line_space, outline, false)
            .clone()
    }

    /// Literal edit text, preserving trailing lines and character boundaries for picking,
    /// selection and wrapped caret navigation. Shares the normal glyph and layout caches.
    pub fn layout_edit(
        &mut self,
        font: &str,
        size: f32,
        text: &str,
        max_width: Option<f32>,
        line_space: f32,
    ) -> LaidOutText {
        self.layout_ref(font, size, text, max_width, line_space, false, true)
            .clone()
    }

    /// Width and height of the laid out text, without copying the layout.
    pub fn layout_size(
        &mut self,
        font: &str,
        size: f32,
        text: &str,
        max_width: Option<f32>,
        line_space: f32,
    ) -> (f32, f32) {
        let lo = self.layout_ref(font, size, text, max_width, line_space, false, false);
        (lo.width, lo.height)
    }

    fn layout_ref(
        &mut self,
        font: &str,
        size: f32,
        text: &str,
        max_width: Option<f32>,
        line_space: f32,
        outline: bool,
        edit: bool,
    ) -> &LaidOutText {
        use std::hash::{Hash, Hasher};
        let mw = max_width.map(f32::to_bits);
        let (sz, ls) = (size.to_bits(), line_space.to_bits());
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (font, sz, text, mw, ls, outline, edit).hash(&mut h);
        let hash = h.finish();
        let same = |k: &LayoutKey| {
            k.font == font
                && k.text == text
                && (k.size, k.max_width, k.line_space, k.outline, k.edit) == (sz, mw, ls, outline, edit)
        };
        if let Some(i) = self
            .layouts
            .get(&hash)
            .and_then(|v| v.iter().position(|(k, _)| same(k)))
        {
            return &self.layouts[&hash][i].1;
        }
        let lo = self.layout_uncached(font, size, text, max_width, line_space, outline, edit);
        if self.layouts_len >= LAYOUT_CACHE_MAX {
            self.layouts.clear();
            self.layouts_len = 0;
        }
        let key = LayoutKey {
            font: font.to_string(),
            text: text.to_string(),
            size: sz,
            max_width: mw,
            line_space: ls,
            outline,
            edit,
        };
        let v = self.layouts.entry(hash).or_default();
        v.push((key, lo));
        self.layouts_len += 1;
        &v[v.len() - 1].1
    }

    fn layout_uncached(
        &mut self,
        font: &str,
        size: f32,
        text: &str,
        max_width: Option<f32>,
        line_space: f32,
        outline: bool,
        edit: bool,
    ) -> LaidOutText {
        if let Some(f) = self.image_fonts.get(&font.to_ascii_lowercase()) {
            // Bitmap glyphs have no outline of their own.
            if outline {
                return LaidOutText::default();
            }
            return Self::layout_image(f, size, text, line_space);
        }
        let chain = self.chain_for(font);
        let px = size.round().max(1.0) as u16;
        let (ascent, line_h) = self.metrics(font, size);

        // 1. Characters with advance and colour; `None` face = line break.
        struct Item {
            index: usize,
            ch: char,
            face: usize,
            gid: u16,
            adv: f32,
            color: Option<[f32; 4]>,
        }
        let mut items: Vec<Item> = Vec::new();
        // A last line that is empty or only blanks does not count: the client gives
        // `"We fear no death."\r\n\n` 2 lines and `"Claw to earth, ... fur to fire."\r\n\n `
        // (wrapped in two) 3 lines (GetTextHeight in reference/character_create/step1_*).
        let mut expanded = if edit { text.to_string() } else { expand_codes(text) };
        if !edit && let Some(nl) = expanded.rfind('\n')
            && expanded[nl + 1..]
                .chars()
                .all(|c| c == ' ' || c == '\t' || c == '\r')
        {
            expanded.truncate(nl);
            if expanded.ends_with('\r') {
                expanded.pop();
            }
        }
        let chars: Vec<char> = expanded.chars().collect();
        let mut color: Option<[f32; 4]> = None;
        let mut i = 0;
        while i < chars.len() {
            let ch = chars[i];
            if !edit && ch == '|' && i + 1 < chars.len() {
                match chars[i + 1] {
                    'c' | 'C' if i + 10 <= chars.len() => {
                        let hex: String = chars[i + 2..i + 10].iter().collect();
                        if let Ok(v) = u32::from_str_radix(&hex, 16) {
                            let ch = |s: u32| ((v >> s) & 255) as f32 / 255.0;
                            color = Some([ch(16), ch(8), ch(0), ch(24)]);
                            i += 10;
                            continue;
                        }
                    }
                    'r' | 'R' => {
                        color = None;
                        i += 2;
                        continue;
                    }
                    _ => {}
                }
            }
            i += 1;
            if ch == '\r' {
                continue;
            }
            if ch == '\n' {
                items.push(Item {
                    index: i - 1,
                    ch,
                    face: usize::MAX,
                    gid: 0,
                    adv: 0.0,
                    color,
                });
                continue;
            }
            if (ICON_BASE..ICON_BASE + INLINE_ICONS.len() as u32).contains(&(ch as u32)) {
                // Icon plus 1 px on each side (matches the loyalty token width at login).
                let adv = (ICON_ADVANCE * size / 13.0).round().max(ICON_ADVANCE) + 2.0;
                items.push(Item {
                    index: i - 1,
                    ch,
                    face: ICON_FACE,
                    gid: (ch as u32 - ICON_BASE) as u16,
                    adv,
                    color,
                });
                continue;
            }
            let Some((face, gid)) = self.lookup(&chain, ch) else {
                continue;
            };
            let adv = self.advance(face, px, gid);
            // Kerning moves this glyph towards the previous one of the same face.
            if let Some(prev) = items.last_mut().filter(|p| p.face == face) {
                let f = &self.faces[face];
                if let Some(&v) = f.kern.get(&(prev.gid, gid)) {
                    prev.adv += kern_px(v, px, f.units_per_em);
                }
            }
            items.push(Item {
                index: i - 1,
                ch,
                face,
                gid,
                adv,
                color,
            });
        }

        // 2. Greedy line breaking: lines are index ranges into `items`.
        let mut lines: Vec<(usize, usize)> = Vec::new();
        let mut start = 0;
        let mut x = 0.0f32;
        let mut last_space: Option<usize> = None;
        let mut k = 0;
        while k < items.len() {
            let it = &items[k];
            if it.face == usize::MAX {
                lines.push((start, k));
                start = k + 1;
                x = 0.0;
                last_space = None;
                k += 1;
                continue;
            }
            if let Some(maxw) = max_width
                && it.ch != ' '
                && x + it.adv > maxw
                && k > start
            {
                let brk = match last_space {
                    Some(sp) if sp > start => sp,
                    _ => k,
                };
                lines.push((start, brk));
                // Skip the space we broke at.
                start = if brk < items.len() && items[brk].ch == ' ' {
                    brk + 1
                } else {
                    brk
                };
                x = items[start..k].iter().map(|i| i.adv).sum();
                last_space = None;
                continue;
            }
            if it.ch == ' ' {
                last_space = Some(k);
            }
            x += it.adv;
            k += 1;
        }
        lines.push((start, items.len()));

        // 3. Place glyphs.
        let mut out = LaidOutText {
            line_height: line_h,
            ..Default::default()
        };
        let mut carets = if edit { vec![None; chars.len()+1] } else { Vec::new() };
        for (li, &(a, b)) in lines.iter().enumerate() {
            let y = li as f32 * (line_h + line_space) + ascent;
            let mut x = 0.0f32;
            // Trailing spaces do not count towards the line width.
            let mut width = 0.0f32;
            if edit {
                let first = items.get(a).map_or(chars.len(), |it| it.index);
                carets[first] = Some(TextCaret { x: 0.0, line: li });
                out.line_ranges.push((first,items.get(b).map_or(chars.len(),|it|it.index)));
            }
            for it in &items[a..b] {
                if edit { carets[it.index] = Some(TextCaret { x, line: li }); }
                if it.face == ICON_FACE {
                    let [_, _, cw, ch] = INLINE_ICONS[it.gid as usize];
                    let ih = (cw * size / 13.0).round().max(cw) * ch / cw;
                    let top = li as f32 * (line_h + line_space) + ((line_h - ih) / 2.0).round();
                    out.icons.push(PlacedIcon {
                        x: x + 1.0,
                        y: top,
                        line: li as u16,
                        icon: it.gid as usize,
                    });
                    width = x + it.adv;
                } else if it.ch != ' ' {
                    if let Some(g) = self.glyph(it.face, px, it.gid, outline) {
                        out.glyphs.push(PlacedGlyph {
                            x,
                            y,
                            line: li as u16,
                            glyph: g,
                            color: it.color,
                        });
                    }
                    width = x + it.adv;
                }
                x += it.adv;
                if edit { carets[it.index+1] = Some(TextCaret { x, line: li }); }
            }
            if edit && let Some(end) = items.get(b) {
                carets[end.index] = Some(TextCaret { x, line: li });
            }
            out.line_widths.push(if edit { x } else { width });
        }
        if edit {
            let mut previous = TextCaret::default();
            out.carets = carets.into_iter().map(|p| { previous = p.unwrap_or(previous); previous }).collect();
        }
        out.width = out.line_widths.iter().copied().fold(0.0, f32::max);
        let n = lines.len() as f32;
        out.height = n * line_h + (n - 1.0) * line_space;
        out
    }

    /// Rasterise (once) and return a glyph.
    fn glyph(&mut self, face: usize, size: u16, glyph: u16, outline: bool) -> Option<Glyph> {
        let key = GlyphKey {
            face: face as u16,
            size,
            glyph,
            outline,
        };
        if let Some(g) = self.glyphs.get(&key) {
            return *g;
        }
        let img = {
            let font = self.faces[face].font();
            let mut scaler = self
                .scale
                .builder(font)
                .size(size as f32)
                .hint(true)
                .build();
            Render::new(&[Source::Outline])
                .format(Format::Alpha)
                .render(&mut scaler, glyph)
        };
        let result = img.and_then(|img| {
            let (mut w, mut h) = (img.placement.width, img.placement.height);
            if w == 0 || h == 0 {
                return None;
            }
            let mut data = img.data;
            let (mut left, mut top) = (img.placement.left, img.placement.top);
            if outline {
                // 1 px dilation of the coverage.
                let (nw, nh) = (w + 2, h + 2);
                let mut d = vec![0u8; (nw * nh) as usize];
                for y in 0..nh as i32 {
                    for x in 0..nw as i32 {
                        let mut m = 0u8;
                        for dy in -1..=1 {
                            for dx in -1..=1 {
                                let sx = x - 1 + dx;
                                let sy = y - 1 + dy;
                                if sx >= 0 && sy >= 0 && (sx as u32) < w && (sy as u32) < h {
                                    m = m.max(data[(sy as u32 * w + sx as u32) as usize]);
                                }
                            }
                        }
                        d[(y as u32 * nw + x as u32) as usize] = m;
                    }
                }
                data = d;
                w = nw;
                h = nh;
                left -= 1;
                top += 1;
            }
            let (ax, ay) = self.alloc(w, h)?;
            for y in 0..h {
                for x in 0..w {
                    let a = data[(y * w + x) as usize];
                    let o = (((ay + y) * ATLAS_SIZE + ax + x) * 4) as usize;
                    self.atlas[o..o + 4].copy_from_slice(&[255, 255, 255, a]);
                }
            }
            self.atlas_dirty = true;
            Some(Glyph {
                ax,
                ay,
                w,
                h,
                left,
                top: -top,
            })
        });
        self.glyphs.insert(key, result);
        result
    }

    fn alloc(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        if self.shelf_x + w + 1 > ATLAS_SIZE {
            self.shelf_x = 1;
            self.shelf_y += self.shelf_h + 1;
            self.shelf_h = 0;
        }
        if self.shelf_y + h + 1 > ATLAS_SIZE {
            bevy::log::warn!("aa-ui: glyph atlas full");
            return None;
        }
        let p = (self.shelf_x, self.shelf_y);
        self.shelf_x += w + 1;
        self.shelf_h = self.shelf_h.max(h);
        Some(p)
    }
}

/// Item face for inline icons.
const ICON_FACE: usize = usize::MAX - 1;

/// Remove colour codes (`|cAARRGGBB`, `|r`).
fn strip_colors(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '|' && i + 1 < chars.len() {
            match chars[i + 1] {
                'c' | 'C' if i + 10 <= chars.len() => {
                    i += 10;
                    continue;
                }
                'r' | 'R' => {
                    i += 2;
                    continue;
                }
                _ => {}
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Parse `ui/image_text.g`: `texturepath <path>`, then `infomation` blocks (`name`, `size`,
/// `width`, `height`, `fontset` with `glyph ( key K, x X, y Y, insetLeft L, insetRight R)`).
fn parse_image_fonts(text: &str) -> HashMap<String, ImageFont> {
    let mut out = HashMap::new();
    let mut texture = String::new();
    let mut cur: Option<(String, ImageFont)> = None;
    let flush = |cur: &mut Option<(String, ImageFont)>, out: &mut HashMap<String, ImageFont>| {
        if let Some((name, f)) = cur.take()
            && !name.is_empty()
        {
            out.insert(name.to_ascii_lowercase(), f);
        }
    };
    for raw in text.lines() {
        let raw = raw.trim_start_matches('\u{feff}');
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let indented = raw.starts_with(' ') || raw.starts_with('\t');
        let (word, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let rest = rest.trim();
        if !indented {
            flush(&mut cur, &mut out);
            match word {
                "texturepath" => texture = aa_pak::normalize(rest),
                "infomation" => {
                    cur = Some((
                        String::new(),
                        ImageFont {
                            texture: texture.clone(),
                            size: 0.0,
                            width: 0.0,
                            height: 0.0,
                            glyphs: HashMap::new(),
                        },
                    ))
                }
                _ => {}
            }
            continue;
        }
        let Some((name, f)) = cur.as_mut() else {
            continue;
        };
        match word {
            "name" => *name = rest.to_string(),
            "size" => f.size = rest.parse().unwrap_or(0.0),
            "width" => f.width = rest.parse().unwrap_or(0.0),
            "height" => f.height = rest.parse().unwrap_or(0.0),
            "glyph" => {
                let body = rest.trim_start_matches('(').trim_end_matches(')');
                // `key` is a quoted character (`" "`, `"항"`) or a bare one (`1`, `-`).
                let Some(after_key) = body.trim_start().strip_prefix("key") else {
                    continue;
                };
                let after_key = after_key.trim_start();
                let (key, tail) = if let Some(q) = after_key.strip_prefix('"') {
                    match q.find('"') {
                        Some(end) => (q[..end].chars().next(), &q[end + 1..]),
                        None => (None, ""),
                    }
                } else {
                    let end = after_key.find(',').unwrap_or(after_key.len());
                    (after_key[..end].trim().chars().next(), &after_key[end..])
                };
                let Some(key) = key else { continue };
                let mut v = [0.0f32; 4];
                for part in tail.split(',') {
                    let mut it = part.split_whitespace();
                    let (Some(k), Some(n)) = (it.next(), it.next()) else {
                        continue;
                    };
                    let n: f32 = n.parse().unwrap_or(0.0);
                    match k {
                        "x" => v[0] = n,
                        "y" => v[1] = n,
                        "insetLeft" => v[2] = n,
                        "insetRight" => v[3] = n,
                        _ => {}
                    }
                }
                f.glyphs.insert(key, v);
            }
            _ => {}
        }
    }
    flush(&mut cur, &mut out);
    out
}

fn icon_char(i: usize) -> char {
    char::from_u32(ICON_BASE + i as u32).unwrap_or(' ')
}

/// `|m<copper>;`: gold / silver / copper parts with coin icons (`0` shows a copper coin).
fn money_text(body: &str) -> String {
    let Ok(v) = body.trim().parse::<i64>() else {
        return body.to_string();
    };
    let (g, s, c) = (v / 10000, (v / 100) % 100, v % 100);
    let mut parts = Vec::new();
    if g != 0 {
        parts.push(format!("{g}{}", icon_char(0)));
    }
    if s != 0 {
        parts.push(format!("{s}{}", icon_char(1)));
    }
    if c != 0 || parts.is_empty() {
        parts.push(format!("{c}{}", icon_char(2)));
    }
    parts.join(" ")
}

/// Expand the client's inline value codes, keeping colour codes (`|cAARRGGBB`, `|r`):
/// `|,1234;` -> `1234` (the client shows `Labor 85/2000`, no separators at login); other `|x...;` codes (`|b` bold value, `|m`
/// money with coin icons, `|p`, `|u`, ...) show their content (`|b` adds the loyalty token
/// icon, `|h` the honor medal, `|l` the vocation badge, `|m` coins).
pub fn expand_codes(text: &str) -> String {
    if !text.contains('|') {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '|' && i + 1 < chars.len() {
            let k = chars[i + 1];
            let is_value =
                k == ',' || (k.is_ascii_alphabetic() && !matches!(k, 'c' | 'C' | 'r' | 'R'));
            if is_value {
                let end = chars[i + 2..]
                    .iter()
                    .take(64)
                    .position(|&x| x == ';' || x == '|' || x == '\n');
                if let Some(n) = end.filter(|&n| chars[i + 2 + n] == ';') {
                    let body: String = chars[i + 2..i + 2 + n].iter().collect();
                    match k {
                        'm' => out.push_str(&money_text(&body)),
                        'b' | 'h' | 'l' => {
                            out.push_str(&body);
                            out.push(icon_char(match k {
                                'b' => 3,
                                'h' => 4,
                                _ => 5,
                            }));
                        }
                        _ => out.push_str(&body),
                    }
                    i += n + 3;
                    continue;
                }
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

#[allow(dead_code)]
fn group_thousands(s: &str) -> String {
    let (sign, digits) = s.strip_prefix('-').map_or(("", s), |d| ("-", d));
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return s.to_string();
    }
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    format!("{sign}{out}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_codes() {
        assert_eq!(expand_codes("LP |,85;/|,2000;"), "LP 85/2000");
        assert_eq!(expand_codes("|cFFFF0000red|r |p12;"), "|cFFFF0000red|r 12");
        assert_eq!(
            expand_codes("|m10203;"),
            format!("1{} 2{} 3{}", icon_char(0), icon_char(1), icon_char(2))
        );
        assert_eq!(expand_codes("|m0;"), format!("0{}", icon_char(2)));
        assert_eq!(expand_codes("|h0;"), format!("0{}", icon_char(4)));
        assert_eq!(expand_codes("|l12;"), format!("12{}", icon_char(5)));
        assert_eq!(expand_codes("a|b"), "a|b");
    }

    #[test]
    fn kerning_like_freetype() {
        // Flareserif 821 (2048 units): "Vo" -190 is -1 px at 13 px, "ke" -39 is 0, "P." -379
        // is -1 at 13 px and -4 at 30 px (no small-size cut).
        assert_eq!(kern_px(-190, 13, 2048), -1.0);
        assert_eq!(kern_px(-39, 13, 2048), 0.0);
        assert_eq!(kern_px(-379, 13, 2048), -1.0);
        assert_eq!(kern_px(-379, 30, 2048), -6.0);
        assert_eq!(kern_px(115, 13, 2048), 0.0);
        let mut t = vec![0u8, 0, 0, 1, 0, 0, 0, 20, 0, 1, 0, 1, 0, 0, 0, 0, 0, 0];
        t.extend_from_slice(&[0, 57, 0, 82, 0xff, 0x42]);
        assert_eq!(parse_kern(&t).get(&(57, 82)), Some(&-190));
    }

    #[test]
    fn image_font_parse() {
        let g = "\u{feff}texturepath ui/image_text.dds\n        \ninfomation\n    name characterInfo\n    size 37\n    width 29\n    height 37\n    fontset\n        glyph ( key 1, x 0, y 465, insetLeft 0, insetRight 14)\n        glyph ( key \" \", x 30, y 404, insetLeft 0, insetRight 40)\n\nlevel\n    coords ( 290, 465, 45, 21 )\n";
        let fonts = parse_image_fonts(g);
        let f = &fonts["characterinfo"];
        assert_eq!(f.texture, "ui/image_text.dds");
        assert_eq!((f.size, f.width, f.height), (37.0, 29.0, 37.0));
        assert_eq!(f.glyphs[&'1'], [0.0, 465.0, 0.0, 14.0]);
        assert_eq!(f.glyphs[&' '], [30.0, 404.0, 0.0, 40.0]);
        assert_eq!(fonts.len(), 1);
        let lo = FontLib::layout_image(f, 37.0, "1", 0.0);
        assert_eq!((lo.width, lo.height), (15.0, 37.0));
        assert_eq!(lo.images[0].src, [0.0, 465.0, 15.0, 37.0]);
    }

    /// Prints font metrics of the UI fonts (needs the client pak): `cargo test -p aa-ui font_metrics -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn font_metrics() {
        let mut lib = FontLib::new("en_us");
        for font in [DEFAULT_FONT, "ui/font/yoon_snail_b.ttf"] {
            let chain = lib.chain_for(font);
            let Some((face, _)) = chain.first() else {
                continue;
            };
            for px in [11.0f32, 13.0, 15.0, 18.0, 22.0] {
                let m = lib.faces[*face].font().metrics(&[]).scale(px);
                println!(
                    "{font} {px}: ascent {:.2} descent {:.2} leading {:.2} sum {:.2} upem {}",
                    m.ascent,
                    m.descent,
                    m.leading,
                    m.ascent + m.descent + m.leading,
                    m.units_per_em
                );
            }
        }
    }
}
