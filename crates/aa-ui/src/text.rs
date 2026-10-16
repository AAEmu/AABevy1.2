//! Fonts, text measurement and glyph rasterisation.
//!
//! The client draws text with FreeType at integer pixel sizes and integer pen positions. We do
//! the same with `swash` (hinted outlines, 8-bit coverage): advances are rounded to whole pixels,
//! so measured widths (`GetTextWidth`) and drawn text agree exactly.
//!
//! Logical font paths (`ui/font/yd_ygo540.ttf`) map to a fallback chain per locale through
//! `fonts/fonts.g` (primary font + fonts that cover listed Unicode ranges).

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

/// Inline icons of the value codes, in `ui/common/money_window.dds`: (x, y, w, h) texels.
pub const INLINE_ICON_TEXTURE: &str = "ui/common/money_window.dds";
pub const INLINE_ICONS: [[f32; 4]; 4] = [
    [0.0, 29.0, 16.0, 16.0],   // gold coin
    [16.0, 29.0, 16.0, 16.0],  // silver coin
    [32.0, 29.0, 16.0, 16.0],  // copper coin
    [206.0, 12.0, 16.0, 16.0], // loyalty token (BM mileage)
];
/// First private-use code point standing for an inline icon in expanded text.
const ICON_BASE: u32 = 0xE000;
const ICON_ADVANCE: f32 = 16.0;

#[derive(Clone, Debug, Default)]
pub struct LaidOutText {
    pub glyphs: Vec<PlacedGlyph>,
    pub icons: Vec<PlacedIcon>,
    pub width: f32,
    pub height: f32,
    pub line_widths: Vec<f32>,
    pub line_height: f32,
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
        };
        // A white texel at (0,0) for solid quads.
        lib.atlas[..4].copy_from_slice(&[255, 255, 255, 255]);
        match aa_pak::Pak::global().read_string("fonts/fonts.g") {
            Ok(text) => lib.parse_fonts_g(&text, locale),
            Err(e) => bevy::log::warn!("aa-ui: fonts/fonts.g: {e}"),
        }
        lib
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
        let (offset, key) = {
            let f = FontRef::from_index(&data, 0)?;
            (f.offset, f.key)
        };
        let i = self.faces.len();
        self.faces.push(Face { data, offset, key });
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

    /// Ascent (rounded) and line height for a font size.
    pub fn metrics(&mut self, font: &str, size: f32) -> (f32, f32) {
        let chain = self.chain_for(font);
        let px = size.round().max(1.0);
        match chain.first() {
            Some((face, _)) => {
                // The client's line height is the font's ascent + descent, rounded (yd_ygo540:
                // 13 px -> 16, 15 -> 18, 18 -> 22; yoon_snail_b 15 -> 19; checked against
                // GetTextHeight in reference/*/ui.json).
                let m = self.faces[*face].font().metrics(&[]).scale(px);
                (m.ascent.round(), (m.ascent + m.descent).round())
            }
            None => (px * 0.8, (px * 1.2).round()),
        }
    }

    /// Width of the longest line of `text` (colour codes stripped).
    pub fn measure(&mut self, font: &str, size: f32, text: &str) -> f32 {
        self.layout(font, size, text, None, 0.0, false).width
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
        let chain = self.chain_for(font);
        let px = size.round().max(1.0) as u16;
        let (ascent, line_h) = self.metrics(font, size);

        // 1. Characters with advance and colour; `None` face = line break.
        struct Item {
            ch: char,
            face: usize,
            gid: u16,
            adv: f32,
            color: Option<[f32; 4]>,
        }
        let mut items: Vec<Item> = Vec::new();
        // A trailing line break does not start another (empty) line: the client counts
        // `"We fear no death."\r\n\n` as 2 lines.
        let mut expanded = expand_codes(text);
        if expanded.ends_with('\n') {
            expanded.pop();
            if expanded.ends_with('\r') {
                expanded.pop();
            }
        }
        let chars: Vec<char> = expanded.chars().collect();
        let mut color: Option<[f32; 4]> = None;
        let mut i = 0;
        while i < chars.len() {
            let ch = chars[i];
            if ch == '|' && i + 1 < chars.len() {
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
            items.push(Item {
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
            if let Some(maxw) = max_width {
                if it.ch != ' ' && x + it.adv > maxw && k > start {
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
        for (li, &(a, b)) in lines.iter().enumerate() {
            let y = li as f32 * (line_h + line_space) + ascent;
            let mut x = 0.0f32;
            // Trailing spaces do not count towards the line width.
            let mut width = 0.0f32;
            for it in &items[a..b] {
                if it.face == ICON_FACE {
                    let top = li as f32 * (line_h + line_space)
                        + ((line_h - (it.adv - 2.0)) / 2.0).round();
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
            }
            out.line_widths.push(width);
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
/// icon, `|m` coins).
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
                        'b' => {
                            out.push_str(&body);
                            out.push(icon_char(3));
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
        assert_eq!(expand_codes("a|b"), "a|b");
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
