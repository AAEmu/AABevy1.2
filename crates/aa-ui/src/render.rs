//! Turns the widget tree into a few batched 2D meshes.
//!
//! Every visible drawable / text glyph becomes a textured, vertex-coloured quad in paint order;
//! consecutive quads that use the same texture share one mesh (one draw call). The meshes are
//! rebuilt only when the tree changes, so a static screen costs nothing per frame.
//!
//! World units are physical pixels: the UI camera looks at `(W/2, -H/2)` so UI coordinate
//! `(x, y)` (y down) is world `(x, -y)`.

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::image::{CompressedImageFormatSupport, CompressedImageFormats, ImageSampler, ImageType};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

use crate::lua::Ui;
use crate::text::ATLAS_SIZE;
use crate::tree::{ButtonState, DrawKind, Id, Kind, Rect, TextBlock, Tree, WidgetType};

/// Texture of a quad.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Tex {
    /// The glyph atlas (also has a white texel at 0,0 for solid quads).
    Atlas,
    /// A pak texture (resolved path).
    Image(String),
}

#[derive(Clone, Debug)]
pub struct Quad {
    pub tex: Tex,
    /// Pixel rectangle x0, y0, x1, y1.
    pub pos: [f32; 4],
    /// Normalised texture coordinates u0, v0, u1, v1.
    pub uv: [f32; 4],
    /// sRGB colour.
    pub color: [f32; 4],
    /// Free vertex positions (top-left, top-right, bottom-right, bottom-left order) instead of
    /// the `pos` rectangle: triangles of the circle diagram.
    pub corners: Option<[[f32; 2]; 4]>,
    /// Per-vertex sRGB colours (same corner order) instead of `color`: gradients.
    pub vcolors: Option<[[f32; 4]; 4]>,
}

/// Collect the quads of the whole tree in paint order.
pub fn build_quads(u: &mut Ui, pressed: Option<Id>) -> Vec<Quad> {
    build_quads_split(u, pressed).0
}

/// The UI layer of the top-level window `id` belongs to.
fn top_layer(tree: &Tree, mut id: Id) -> crate::tree::Layer {
    while let Some(p) = tree.get(id).parent {
        id = p;
    }
    tree.widget(id).map(|w| w.layer).unwrap_or_default()
}

/// [`build_quads`] plus the number of leading quads that belong to `background`-layer windows
/// (paint order puts them first): those go to the background camera in split mode.
pub fn build_quads_split(u: &mut Ui, pressed: Option<Id>) -> (Vec<Quad>, usize) {
    let mut out = Vec::new();
    let order = u.tree.paint_order();
    let scale = u.ui_scale;
    let mut background_end = None;
    for id in order {
        if background_end.is_none()
            && id != crate::tree::ROOT
            && top_layer(&u.tree, id) > crate::tree::Layer::Background
        {
            background_end = Some(out.len());
        }
        let alpha = effective_alpha(&u.tree, id);
        if alpha <= 0.0 {
            continue;
        }
        let Some(w) = u.tree.widget(id).cloned() else {
            continue;
        };
        if w.ty == WidgetType::Root {
            continue;
        }
        let state = button_state(u, id, pressed);
        let quads_start = out.len();
        let mut ds: Vec<Id> = w
            .drawables
            .iter()
            .copied()
            .filter(|&d| {
                let o = u.tree.get(d);
                o.alive && o.shown
            })
            .collect();
        ds.sort_by_key(|&d| {
            u.tree
                .get(d)
                .drawable()
                .map(|x| x.layer)
                .unwrap_or_default()
        });
        let (below, above): (Vec<Id>, Vec<Id>) = ds.iter().partition(|&&d| {
            u.tree
                .get(d)
                .drawable()
                .is_some_and(|x| x.layer != crate::tree::DrawLayer::Overlay)
        });
        for d in below {
            drawable_quads(u, d, alpha, state, scale, &mut out);
        }
        if let Some(dg) = w.diagram.as_ref() {
            diagram_quads(u, id, dg, alpha, scale, &mut out);
        }
        if w.cooldown.is_some() {
            crate::lua::slot::cooldown_quads(u, id, &w, alpha, scale, &mut out);
        }
        if let Some(p) = w.picker.as_ref() {
            picker_quads(u, p, alpha, scale, &mut out);
        }
        // Message frames: their newest lines.
        if w.message.is_some() {
            let rect = u.tree.rect(id);
            if let Some(t) = crate::lua::chat::compose(u, id, rect)
                && !t.text.is_empty()
            {
                let c = t.style.color;
                text_quads(u, &t, rect, c, alpha, scale, &mut out);
            }
        } else if crate::lua::tooltip::has_lines(&w) {
            // Tooltips: their `AddLine` lines.
            let rect = u.tree.rect(id);
            crate::lua::tooltip::quads(u, id, rect, alpha, scale, &mut out);
        } else if let Some(t) = w.text.as_ref() {
            // Widget text above background/artwork, below overlay.
            let edit = crate::lua::edit::is_edit(&w.ty);
            let rect = u.tree.rect(id);
            let shown = if edit {
                let mut d = t.clone();
                d.text = crate::lua::edit::display_text(&t.text, w.edit.as_ref());
                // `SetPrefixText`: the prefix in `prefixStyle` first, the text after it.
                if let Some(e) = w.edit.as_ref().filter(|e| !e.prefix.is_empty()) {
                    let mut p = w.aux_style.clone().unwrap_or_default();
                    p.text = e.prefix.clone();
                    let pi = e.prefix_inset;
                    p.inset = [
                        t.inset[0] + pi[0],
                        t.inset[1] + pi[1],
                        t.inset[2],
                        t.inset[3] + pi[3],
                    ];
                    p.style.align = crate::tree::Align::Left;
                    let pw = u.fonts.measure(&p.style.font, p.style.size, &p.text);
                    let c = p.style.color;
                    text_quads(u, &p, rect, c, alpha, scale, &mut out);
                    d.inset[0] += pi[0] + pw + pi[2];
                }
                std::borrow::Cow::Owned(d)
            } else {
                std::borrow::Cow::Borrowed(t)
            };
            if !shown.text.is_empty() {
                let color = w
                    .button
                    .as_ref()
                    .and_then(|b| b.text_color[state_index(state)].or(b.text_color[0]))
                    .unwrap_or(t.style.color);
                text_quads(u, &shown, rect, color, alpha, scale, &mut out);
            }
            if edit && u.focus == Some(id) && u.caret_visible {
                caret_quad(u, &shown, w.edit.as_ref(), rect, alpha, scale, &mut out);
            }
        }
        if let Some(t) = w.title.as_ref()
            && !t.text.is_empty()
        {
            let r = u.tree.rect(id);
            // With `SetTitleInset` the title sits in the window minus the insets, aligned by
            // `titleStyle` (step 2's tab titles: TOPLEFT at 15, 15). Without, it is placed in
            // the top 45 units (the baselib title bar height), 1 unit lower: the step 3
            // confirm dialog in reference/character_create matches to the pixel.
            let rect = if t.inset != [0.0; 4] {
                r
            } else {
                Rect {
                    x: r.x,
                    y: r.y + 1.0,
                    w: r.w,
                    h: r.h.min(45.0),
                }
            };
            text_quads(u, t, rect, t.style.color, alpha, scale, &mut out);
        }
        for d in above {
            drawable_quads(u, d, alpha, state, scale, &mut out);
        }
        // Inside a scroll window (`EnableScroll`): only what is in its rectangle.
        if let Some(c) = u.tree.clip_rect(id) {
            clip_quads(&mut out[quads_start..], px(c, scale));
        }
    }
    let n = background_end.unwrap_or(out.len());
    (out, n)
}

/// Cut quads to the pixel rectangle `c` (x0, y0, x1, y1), moving the texture coordinates with
/// the cut edges; quads fully outside collapse to a point (zero area, draws nothing).
fn clip_quads(quads: &mut [Quad], c: [f32; 4]) {
    for q in quads {
        if q.corners.is_some() {
            continue;
        }
        let [x0, y0, x1, y1] = q.pos;
        let (nx0, ny0, nx1, ny1) = (x0.max(c[0]), y0.max(c[1]), x1.min(c[2]), y1.min(c[3]));
        if nx0 >= nx1 || ny0 >= ny1 {
            q.pos = [x0, y0, x0, y0];
            continue;
        }
        let (w, h) = (x1 - x0, y1 - y0);
        if w > 0.0 && h > 0.0 {
            let [u0, v0, u1, v1] = q.uv;
            let (du, dv) = (u1 - u0, v1 - v0);
            q.uv = [
                u0 + du * (nx0 - x0) / w,
                v0 + dv * (ny0 - y0) / h,
                u0 + du * (nx1 - x0) / w,
                v0 + dv * (ny1 - y0) / h,
            ];
        }
        q.pos = [nx0, ny0, nx1, ny1];
    }
}

fn state_index(s: ButtonState) -> usize {
    match s {
        ButtonState::Normal => 0,
        ButtonState::Highlight => 1,
        ButtonState::Pushed => 2,
        ButtonState::Disabled => 3,
    }
}

/// Current visual state of a button.
pub fn button_state(u: &Ui, id: Id, pressed: Option<Id>) -> ButtonState {
    let Some(w) = u.tree.widget(id) else {
        return ButtonState::Normal;
    };
    if w.button.is_none() {
        return ButtonState::Normal;
    }
    if !w.enabled {
        ButtonState::Disabled
    } else if pressed == Some(id) && u.hover.is_some_and(|h| u.tree.is_descendant(h, id)) {
        ButtonState::Pushed
    } else if u.hover.is_some_and(|h| u.tree.is_descendant(h, id)) {
        ButtonState::Highlight
    } else {
        ButtonState::Normal
    }
}

fn effective_alpha(tree: &Tree, mut id: Id) -> f32 {
    let mut a = 1.0;
    loop {
        let o = tree.get(id);
        a *= o.alpha;
        match o.parent {
            Some(p) => id = p,
            None => return a,
        }
    }
}

/// Whether a button state drawable shows while its owner is in `state`: the background of the
/// current state (missing states fall back to normal), and `Create<State>Drawable` extras of
/// that state. Other drawables always pass.
pub fn state_visible(u: &Ui, d: Id, state: ButtonState) -> bool {
    let Some(dr) = u.tree.get(d).drawable() else {
        return true;
    };
    // Check marks: `SetCheckedBackground` while checked, `SetDisabledCheckedBackground`
    // instead while checked and disabled.
    if let Some(owner) = u.tree.get(d).parent
        && let Some(w) = u.tree.widget(owner)
        && let Some(b) = w.button.as_ref()
    {
        let disabled = state == ButtonState::Disabled;
        if b.checked_bg == Some(d) {
            return b.checked && !(disabled && b.disabled_checked_bg.is_some());
        }
        if b.disabled_checked_bg == Some(d) {
            return b.checked && disabled;
        }
    }
    if let Some((owner, _)) = dr.state_of {
        let Some(b) = u.tree.widget(owner).and_then(|w| w.button.as_ref()) else {
            return true;
        };
        if b.bg[state_index(state)].or(b.bg[0]) != Some(d) {
            return false;
        }
    }
    dr.state_only.is_none_or(|st| st == state)
}

fn drawable_quads(
    u: &mut Ui,
    d: Id,
    alpha: f32,
    state: ButtonState,
    scale: f32,
    out: &mut Vec<Quad>,
) {
    if !state_visible(u, d, state) {
        return;
    }
    let o = u.tree.get(d).clone();
    let Kind::Drawable(dr) = &o.kind else { return };
    if dr.effect == Some(false) {
        return;
    }
    let mut rect = u.tree.rect(d);
    // Status bar fill: crop the rect and the texture rectangle alike.
    let mut coords = dr.coords;
    if let Some((f, vertical)) = dr.fill {
        if vertical {
            let h = rect.h * f;
            rect.y += rect.h - h;
            rect.h = h;
            if let Some(c) = coords.as_mut() {
                let ch = c[3] * f;
                c[1] += c[3] - ch;
                c[3] = ch;
            }
        } else {
            rect.w *= f;
            if let Some(c) = coords.as_mut() {
                c[2] *= f;
            }
        }
    }
    let a = alpha * o.alpha;
    let color = [dr.color[0], dr.color[1], dr.color[2], dr.color[3] * a];
    if (color[3] <= 0.0 || rect.w == 0.0 || rect.h == 0.0) && dr.kind != DrawKind::Text {
        return;
    }
    match dr.kind {
        DrawKind::Color => out.push(Quad {
            tex: Tex::Atlas,
            pos: px(rect, scale),
            uv: white_uv(),
            color,
            corners: None,
            vcolors: None,
        }),
        DrawKind::Text => {
            if let Some(t) = dr.text.as_ref() {
                text_quads(u, t, rect, t.style.color, a, scale, out);
            }
        }
        DrawKind::Image | DrawKind::Icon | DrawKind::NinePart | DrawKind::ThreePart => {
            let Some(path) = dr.texture.as_ref() else {
                return;
            };
            let Some(resolved) = u.textures.resolve(path) else {
                return;
            };
            let Some((tw, th)) = u.textures.size(path) else {
                return;
            };
            let (tw, th) = (tw as f32, th as f32);
            let c = match (coords, dr.fill) {
                (Some(c), _) => c,
                // Whole texture, cropped like the rect.
                (None, Some((f, false))) => [0.0, 0.0, tw * f, th],
                (None, Some((f, true))) => [0.0, th * (1.0 - f), tw, th * f],
                (None, None) => [0.0, 0.0, tw, th],
            };
            let tex = Tex::Image(resolved);
            match dr.kind {
                DrawKind::NinePart | DrawKind::ThreePart => {
                    let [l, t, r, b] = dr.inset;
                    let (t, b) = if dr.kind == DrawKind::ThreePart {
                        (0.0, 0.0)
                    } else {
                        (t, b)
                    };
                    // Source edges (texels) and destination edges (UI units).
                    let sx = [c[0], c[0] + l, c[0] + c[2] - r, c[0] + c[2]];
                    let sy = [c[1], c[1] + t, c[1] + c[3] - b, c[1] + c[3]];
                    let dl = l.min(rect.w / 2.0);
                    let dr_ = r.min(rect.w / 2.0);
                    let dt = t.min(rect.h / 2.0);
                    let db = b.min(rect.h / 2.0);
                    let dx = [rect.x, rect.x + dl, rect.right() - dr_, rect.right()];
                    let dy = [rect.y, rect.y + dt, rect.bottom() - db, rect.bottom()];
                    for j in 0..3 {
                        for i in 0..3 {
                            if dx[i + 1] <= dx[i] || dy[j + 1] <= dy[j] {
                                continue;
                            }
                            let r = Rect {
                                x: dx[i],
                                y: dy[j],
                                w: dx[i + 1] - dx[i],
                                h: dy[j + 1] - dy[j],
                            };
                            out.push(Quad {
                                tex: tex.clone(),
                                pos: px(r, scale),
                                uv: [sx[i] / tw, sy[j] / th, sx[i + 1] / tw, sy[j + 1] / th],
                                color,
                                corners: None,
                                vcolors: None,
                            });
                        }
                    }
                }
                _ => {
                    out.push(Quad {
                        tex,
                        pos: px(rect, scale),
                        uv: [c[0] / tw, c[1] / th, (c[0] + c[2]) / tw, (c[1] + c[3]) / th],
                        color,
                        corners: rotated_corners(px(rect, scale), dr.rotation),
                        vcolors: None,
                    });
                    // Icon image overlays (`AddTexture`), whole texture over the same rect.
                    for extra in &dr.extra_textures {
                        if let Some(r) = u.textures.resolve(extra) {
                            out.push(Quad {
                                tex: Tex::Image(r),
                                pos: px(rect, scale),
                                uv: [0.0, 0.0, 1.0, 1.0],
                                color,
                                corners: None,
                                vcolors: None,
                            });
                        }
                    }
                }
            }
        }
    }
}

pub(crate) fn white_uv() -> [f32; 4] {
    let t = 0.5 / ATLAS_SIZE as f32;
    [t, t, t, t]
}

/// The corners (top left, top right, bottom right, bottom left) of a pixel rect turned
/// `angle` radians clockwise about its centre; `None` without rotation.
fn rotated_corners(p: [f32; 4], angle: f32) -> Option<[[f32; 2]; 4]> {
    if angle == 0.0 {
        return None;
    }
    let (cx, cy) = ((p[0] + p[2]) / 2.0, (p[1] + p[3]) / 2.0);
    let (hw, hh) = ((p[2] - p[0]) / 2.0, (p[3] - p[1]) / 2.0);
    let (s, c) = angle.sin_cos();
    Some(
        [(-hw, -hh), (hw, -hh), (hw, hh), (-hw, hh)]
            .map(|(x, y)| [cx + x * c - y * s, cy + x * s + y * c]),
    )
}

/// UI rect -> pixel rect, snapped to whole pixels.
fn px(r: Rect, s: f32) -> [f32; 4] {
    [
        (r.x * s).round(),
        (r.y * s).round(),
        (r.right() * s).round(),
        (r.bottom() * s).round(),
    ]
}

pub(crate) fn text_quads(
    u: &mut Ui,
    t: &TextBlock,
    rect: Rect,
    color: [f32; 4],
    alpha: f32,
    scale: f32,
    out: &mut Vec<Quad>,
) {
    let inner = Rect {
        x: rect.x + t.inset[0],
        y: rect.y + t.inset[1],
        w: (rect.w - t.inset[0] - t.inset[2]).max(0.0),
        h: (rect.h - t.inset[1] - t.inset[3]).max(0.0),
    };
    let maxw = (t.wrap && inner.w > 0.0).then_some(inner.w);
    let size = t.style.size * scale;
    let mut text = t.text.clone();
    let mut lo = u.fonts.layout(
        &t.style.font,
        size,
        &text,
        maxw.map(|w| w * scale),
        t.line_space * scale,
        false,
    );
    if t.style.ellipsis && !t.wrap && inner.w > 0.0 && lo.width > inner.w * scale {
        // Trim characters until "text..." fits.
        let mut chars: Vec<char> = text.chars().collect();
        while !chars.is_empty() {
            chars.pop();
            let s: String = chars.iter().collect::<String>() + "...";
            let l = u.fonts.layout(&t.style.font, size, &s, None, 0.0, false);
            if l.width <= inner.w * scale {
                text = s;
                lo = l;
                break;
            }
        }
    }
    let (fx, fy) = (t.style.align.fx(), t.style.align.fy());
    let ox = (inner.x * scale + (inner.w * scale - lo.width) * fx).round();
    let oy = (inner.y * scale + (inner.h * scale - lo.height) * fy).round();
    let a = color[3] * alpha;
    let atlas = ATLAS_SIZE as f32;
    let mut emit = |g: &crate::text::PlacedGlyph, dx: f32, dy: f32, c: [f32; 4], line_x: f32| {
        let gx = ox + line_x + g.x + g.glyph.left as f32 + dx;
        let gy = oy + g.y + g.glyph.top as f32 + dy;
        out.push(Quad {
            tex: Tex::Atlas,
            pos: [gx, gy, gx + g.glyph.w as f32, gy + g.glyph.h as f32],
            uv: [
                g.glyph.ax as f32 / atlas,
                g.glyph.ay as f32 / atlas,
                (g.glyph.ax + g.glyph.w) as f32 / atlas,
                (g.glyph.ay + g.glyph.h) as f32 / atlas,
            ],
            color: c,
            corners: None,
            vcolors: None,
        });
    };
    // Per-line horizontal alignment.
    let line_offsets: Vec<f32> = lo
        .line_widths
        .iter()
        .map(|w| ((lo.width - w) * fx).round())
        .collect();
    if t.style.outline {
        let olo = u.fonts.layout(
            &t.style.font,
            size,
            &text,
            maxw.map(|w| w * scale),
            t.line_space * scale,
            true,
        );
        for g in &olo.glyphs {
            let lx = line_offsets.get(g.line as usize).copied().unwrap_or(0.0);
            emit(g, 0.0, 0.0, [0.0, 0.0, 0.0, a * 0.85], lx);
        }
    }
    if t.style.shadow {
        for g in &lo.glyphs {
            let lx = line_offsets.get(g.line as usize).copied().unwrap_or(0.0);
            emit(g, 1.0, 1.0, [0.0, 0.0, 0.0, a * 0.8], lx);
        }
    }
    for g in &lo.glyphs {
        let lx = line_offsets.get(g.line as usize).copied().unwrap_or(0.0);
        let c = g
            .color
            .map(|gc| [gc[0], gc[1], gc[2], gc[3] * a])
            .unwrap_or([color[0], color[1], color[2], a]);
        emit(g, 0.0, 0.0, c, lx);
    }
    // Image font glyphs, in the texture's own colours.
    if let (false, Some(path)) = (lo.images.is_empty(), lo.image_texture.as_deref())
        && let (Some(resolved), Some((tw, th))) = (u.textures.resolve(path), u.textures.size(path))
    {
        let (tw, th) = (tw as f32, th as f32);
        for im in &lo.images {
            let lx = line_offsets.get(im.line as usize).copied().unwrap_or(0.0);
            let [sx, sy, sw, sh] = im.src;
            let (x, y) = (ox + lx + im.x, oy + im.y);
            out.push(Quad {
                tex: Tex::Image(resolved.clone()),
                pos: [x, y, x + im.w, y + im.h],
                uv: [sx / tw, sy / th, (sx + sw) / tw, (sy + sh) / th],
                color: [1.0, 1.0, 1.0, a],
                corners: None,
                vcolors: None,
            });
        }
    }
    // Inline icons (coins, loyalty token).
    if !lo.icons.is_empty() {
        let path = crate::text::INLINE_ICON_TEXTURE;
        if let (Some(resolved), Some((tw, th))) = (u.textures.resolve(path), u.textures.size(path))
        {
            for ic in &lo.icons {
                let lx = line_offsets.get(ic.line as usize).copied().unwrap_or(0.0);
                let [cx, cy, cw, ch] = crate::text::INLINE_ICONS[ic.icon];
                let sz = (cw * size / 13.0).round().max(cw);
                let x = ox + lx + ic.x;
                let y = oy + ic.y;
                out.push(Quad {
                    tex: Tex::Image(resolved.clone()),
                    pos: [x, y, x + sz, y + sz * ch / cw],
                    uv: [
                        cx / tw as f32,
                        cy / th as f32,
                        (cx + cw) / tw as f32,
                        (cy + ch) / th as f32,
                    ],
                    color: [1.0, 1.0, 1.0, a],
                    corners: None,
                    vcolors: None,
                });
            }
        }
    }
}

/// Colour picker gradients inside the spectrum / luminance frames (3 units in from the frame
/// edge): hue across and saturation down at mid lightness; the luminance bar runs from white
/// through the current hue and saturation to black.
fn picker_quads(
    u: &Ui,
    p: &crate::lua::picker::PickerData,
    alpha: f32,
    scale: f32,
    out: &mut Vec<Quad>,
) {
    use crate::lua::picker::{hls_to_rgb, rgb_to_hls};
    let inner = |d: Id| {
        let r = u.tree.rect(d);
        Rect {
            x: r.x + 3.0,
            y: r.y + 3.0,
            w: (r.w - 6.0).max(0.0),
            h: (r.h - 6.0).max(0.0),
        }
    };
    let c4 = |[r, g, b]: [f32; 3]| [r, g, b, alpha];
    let grad = |r: Rect, vc: [[f32; 4]; 4], out: &mut Vec<Quad>| {
        if r.w > 0.0 && r.h > 0.0 {
            out.push(Quad {
                tex: Tex::Atlas,
                pos: px(r, scale),
                uv: white_uv(),
                color: [1.0; 4],
                corners: None,
                vcolors: Some(vc),
            });
        }
    };
    if let Some(d) = p.spectrum_bg.filter(|&d| u.tree.visible(d)) {
        let r = inner(d);
        let grey = c4([0.5; 3]);
        for i in 0..6 {
            let (h0, h1) = (i as f32 / 6.0, (i + 1) as f32 / 6.0);
            let x0 = r.x + r.w * h0;
            let seg = Rect {
                x: x0,
                y: r.y,
                w: r.w / 6.0,
                h: r.h,
            };
            let (a, b) = (
                c4(hls_to_rgb([h0, 0.5, 1.0])),
                c4(hls_to_rgb([h1, 0.5, 1.0])),
            );
            grad(seg, [a, b, grey, grey], out);
        }
    }
    if let Some(d) = p.luminance_bg.filter(|&d| u.tree.visible(d)) {
        let r = inner(d);
        let [h, _, s] = rgb_to_hls(p.rgb);
        let mid = c4(hls_to_rgb([h, 0.5, s]));
        let (white, black) = (c4([1.0; 3]), c4([0.0; 3]));
        let top = Rect { h: r.h / 2.0, ..r };
        let bottom = Rect {
            y: r.y + r.h / 2.0,
            h: r.h / 2.0,
            ..r
        };
        grad(top, [white, white, mid, mid], out);
        grad(bottom, [mid, mid, black, black], out);
    }
}

/// A circle diagram: a triangle fan from the widget centre through the current vertices.
fn diagram_quads(
    u: &Ui,
    id: Id,
    dg: &crate::lua::diagram::DiagramData,
    alpha: f32,
    scale: f32,
    out: &mut Vec<Quad>,
) {
    let n = dg.points.len();
    if n < 3 {
        return;
    }
    let r = u.tree.rect(id);
    let at = |(x, y): (f32, f32)| [(r.x + x) * scale, (r.y + y) * scale];
    let c = at((r.w / 2.0, r.h / 2.0));
    let color = [dg.color[0], dg.color[1], dg.color[2], dg.color[3] * alpha];
    for i in 0..n {
        let mut a = at(dg.vertex(i, r.w, r.h));
        let mut b = at(dg.vertex((i + 1) % n, r.w, r.h));
        // Same winding as the rectangles (clockwise on screen).
        let cross = (a[0] - c[0]) * (b[1] - c[1]) - (a[1] - c[1]) * (b[0] - c[0]);
        if cross == 0.0 {
            continue;
        }
        if cross < 0.0 {
            std::mem::swap(&mut a, &mut b);
        }
        out.push(Quad {
            tex: Tex::Atlas,
            pos: [0.0; 4],
            uv: white_uv(),
            color,
            corners: Some([c, a, b, b]),
            vcolors: None,
        });
    }
}

/// The caret of the focused edit box: a 1 px bar after the character at the cursor, as tall as
/// a text line plus `SetCursorHeight`, moved by `SetCursorOffset`, in the text colour (or
/// `SetCursorColor`).
fn caret_quad(
    u: &mut Ui,
    t: &TextBlock,
    edit: Option<&crate::lua::edit::EditData>,
    rect: Rect,
    alpha: f32,
    scale: f32,
    out: &mut Vec<Quad>,
) {
    let inner = Rect {
        x: rect.x + t.inset[0],
        y: rect.y + t.inset[1],
        w: (rect.w - t.inset[0] - t.inset[2]).max(0.0),
        h: (rect.h - t.inset[1] - t.inset[3]).max(0.0),
    };
    let size = t.style.size * scale;
    let maxw = (t.wrap && inner.w > 0.0).then_some(inner.w * scale);
    let ls = t.line_space * scale;
    let full = u
        .fonts
        .layout(&t.style.font, size, &t.text, maxw, ls, false);
    let cursor = edit.map_or(usize::MAX, |e| e.cursor);
    let head: String = t.text.chars().take(cursor).collect();
    let lo = u.fonts.layout(&t.style.font, size, &head, maxw, ls, false);
    let (_, lh) = u.fonts.metrics(&t.style.font, size);
    let mut line = lo.line_widths.len().max(1) - 1;
    let mut line_w = lo.line_widths.last().copied().unwrap_or(0.0);
    if head.ends_with('\n') {
        // Layout drops a trailing line break; the caret sits at the start of the next line.
        line += 1;
        line_w = 0.0;
    }
    let (fx, fy) = (t.style.align.fx(), t.style.align.fy());
    let total_h = full.height.max(lh);
    let ox = inner.x * scale + (inner.w * scale - full.width) * fx;
    let oy = inner.y * scale + (inner.h * scale - total_h) * fy;
    let full_line_w = full.line_widths.get(line).copied().unwrap_or(full.width);
    let offset = edit.map_or(0.0, |e| e.cursor_offset) * scale;
    let x = (ox + (full.width - full_line_w) * fx + line_w + offset).round();
    let h = lh + edit.and_then(|e| e.cursor_height).unwrap_or(0.0) * scale;
    let y = (oy + line as f32 * (lh + ls) + (lh - h) / 2.0).round();
    let c = edit.and_then(|e| e.cursor_color).unwrap_or(t.style.color);
    out.push(Quad {
        tex: Tex::Atlas,
        pos: [x, y, x + scale.max(1.0), y + h],
        uv: white_uv(),
        color: [c[0], c[1], c[2], c[3] * alpha],
        corners: None,
        vcolors: None,
    });
}

/// Render-side state: textures, materials and the batch entities.
#[derive(Resource, Default)]
pub struct UiRenderState {
    pub generation: u64,
    pub screen: (u32, u32),
    pub images: HashMap<String, Handle<Image>>,
    pub materials: HashMap<Tex, Handle<ColorMaterial>>,
    pub atlas: Option<Handle<Image>>,
    pub batches: Vec<Entity>,
    pub quads: usize,
    pub draw_calls: usize,
    pub force: bool,
    /// Split mode: render layer of the background camera (`UiConfig::background_camera_order`).
    pub background_layer: Option<usize>,
    /// What the pointer's meshes were built for: held generation, screen size and UI scale bits.
    pub cursor_key: Option<(u64, (u32, u32), u32)>,
}

/// Marker for the UI camera.
#[derive(Component)]
pub struct UiCamera;

/// Marker for batch meshes.
#[derive(Component)]
pub struct UiBatch;

/// Marker for the meshes of what the pointer holds ([`held_quads`]): moved to the pointer every frame.
#[derive(Component)]
pub struct UiCursorBatch;

/// Size (UI units) of the icon that follows the pointer: a slot's 40 x 40 icon.
const HELD_ICON: f32 = 40.0;

/// The quads of what the pointer holds, around the pointer (0, 0), in pixels: an item or skill icon with its grade frame
/// and the count in the corner, or the image of an image cursor. The caller moves them to the pointer.
pub fn held_quads(u: &mut Ui) -> Vec<Quad> {
    let Some(h) = u.drag.held.clone() else { return Vec::new() };
    let scale = u.ui_scale;
    let mut out = Vec::new();
    let white = [1.0, 1.0, 1.0, 1.0];
    if h.kind == "image" {
        let Some(path) = h.icon.as_deref() else { return out };
        let (Some(resolved), Some((tw, th))) = (u.textures.resolve(path), u.textures.size(path)) else { return out };
        let r = Rect { x: h.offset[0], y: h.offset[1], w: tw as f32, h: th as f32 };
        out.push(Quad { tex: Tex::Image(resolved), pos: px(r, scale), uv: [0.0, 0.0, 1.0, 1.0], color: white, corners: None, vcolors: None });
        return out;
    }
    let r = Rect { x: -HELD_ICON / 2.0, y: -HELD_ICON / 2.0, w: HELD_ICON, h: HELD_ICON };
    for path in [h.icon.as_deref(), h.frame.as_deref()].into_iter().flatten() {
        if let Some(resolved) = u.textures.resolve(path) {
            out.push(Quad { tex: Tex::Image(resolved), pos: px(r, scale), uv: [0.0, 0.0, 1.0, 1.0], color: white, corners: None, vcolors: None });
        }
    }
    if h.count > 1 {
        let tb = TextBlock {
            text: h.count.to_string(),
            style: crate::tree::TextStyle { size: 11.0, align: crate::tree::Align::Right, shadow: true, ..Default::default() },
            inset: [0.0, 0.0, 4.0, 3.0],
            ..Default::default()
        };
        text_quads(u, &tb, Rect { x: r.x, y: r.bottom() - 16.0, w: r.w, h: 16.0 }, white, 1.0, scale, &mut out);
    }
    out
}

/// Keep the meshes of the held thing in step: rebuilt when it, the screen or the scale changed, moved to `pointer` (pixels)
/// every frame, hidden without one.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn sync_cursor(
    commands: &mut Commands,
    state: &mut UiRenderState,
    u: &mut Ui,
    pointer: Option<Vec2>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
    images: &mut Assets<Image>,
    formats: Option<&CompressedImageFormatSupport>,
    existing: &mut Query<(Entity, &mut Transform, &mut Visibility), (With<UiCursorBatch>, Without<UiCamera>)>,
) {
    let key = (u.drag.held_gen, state.screen, u.ui_scale.to_bits());
    if state.cursor_key != Some(key) {
        for (e, _, _) in existing.iter() {
            commands.entity(e).despawn();
        }
        state.cursor_key = Some(key);
        let quads = held_quads(u);
        if !quads.is_empty() {
            // Stack-count glyphs can be added here after the normal UI uploaded its atlas.
            sync_atlas(u, state, images);
            spawn_batches(commands, state, &quads, 0, meshes, materials, images, formats, || UiCursorBatch, pointer.unwrap_or(Vec2::splat(-10_000.0)), 100.0);
        }
        return;
    }
    for (_, mut tf, mut vis) in existing.iter_mut() {
        match pointer {
            Some(p) => {
                tf.translation.x = p.x;
                tf.translation.y = -p.y;
                *vis = Visibility::Inherited;
            }
            None => *vis = Visibility::Hidden,
        }
    }
}

pub(crate) fn load_image(path: &str, formats: CompressedImageFormats) -> Option<Image> {
    let bytes = aa_pak::Pak::global().read(path).ok()?;
    let ext = path.rsplit('.').next().unwrap_or("dds");
    match Image::from_buffer(
        &bytes,
        ImageType::Extension(ext),
        formats,
        true,
        ImageSampler::linear(),
        RenderAssetUsages::RENDER_WORLD,
    ) {
        Ok(img) => Some(img),
        Err(e) => {
            warn!("aa-ui: cannot decode {path}: {e}");
            None
        }
    }
}

/// Upload a batch list as meshes (one entity per batch).
#[allow(clippy::too_many_arguments)]
pub fn sync_batches(
    commands: &mut Commands,
    state: &mut UiRenderState,
    quads: &[Quad],
    background_quads: usize,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
    images: &mut Assets<Image>,
    formats: Option<&CompressedImageFormatSupport>,
    existing: &Query<Entity, With<UiBatch>>,
) {
    for e in existing.iter() {
        commands.entity(e).despawn();
    }
    state.batches.clear();
    let ids = spawn_batches(commands, state, quads, background_quads, meshes, materials, images, formats, || UiBatch, Vec2::ZERO, 0.0);
    state.draw_calls = ids.len();
    state.batches = ids;
    state.quads = quads.len();
}

/// Spawn one entity per run of quads that share a texture (`marker` tags them, `z0` is the depth of the first).
#[allow(clippy::too_many_arguments)]
fn spawn_batches<M: Component>(
    commands: &mut Commands,
    state: &mut UiRenderState,
    quads: &[Quad],
    background_quads: usize,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
    images: &mut Assets<Image>,
    formats: Option<&CompressedImageFormatSupport>,
    marker: impl Fn() -> M,
    origin: Vec2,
    z0: f32,
) -> Vec<Entity> {
    let formats = formats.map(|f| f.0).unwrap_or(CompressedImageFormats::BC);
    // Group consecutive quads by texture (and, in split mode, by camera).
    let split = state.background_layer.is_some();
    let mut batches: Vec<(Tex, bool, Vec<&Quad>)> = Vec::new();
    for (i, q) in quads.iter().enumerate() {
        let bg = split && i < background_quads;
        match batches.last_mut() {
            Some((t, b, v)) if *t == q.tex && *b == bg => v.push(q),
            _ => batches.push((q.tex.clone(), bg, vec![q])),
        }
    }
    let mut spawned = Vec::with_capacity(batches.len());
    for (i, (tex, bg, qs)) in batches.iter().enumerate() {
        let image = match tex {
            Tex::Atlas => state.atlas.clone(),
            Tex::Image(p) => {
                if !state.images.contains_key(p) {
                    let h = load_image(p, formats)
                        .map(|img| images.add(img))
                        .unwrap_or_default();
                    state.images.insert(p.clone(), h);
                }
                state.images.get(p).cloned()
            }
        };
        let mat = state
            .materials
            .entry(tex.clone())
            .or_insert_with(|| {
                materials.add(ColorMaterial {
                    texture: image.clone(),
                    ..default()
                })
            })
            .clone();
        let mut pos = Vec::with_capacity(qs.len() * 4);
        let mut uv = Vec::with_capacity(qs.len() * 4);
        let mut col = Vec::with_capacity(qs.len() * 4);
        let mut idx = Vec::with_capacity(qs.len() * 6);
        for q in qs {
            let base = pos.len() as u32;
            let [x0, y0, x1, y1] = q.pos;
            let [u0, v0, u1, v1] = q.uv;
            match q.corners {
                Some(c) => pos.extend(c.map(|[x, y]| [x, -y, 0.0])),
                None => pos.extend([
                    [x0, -y0, 0.0],
                    [x1, -y0, 0.0],
                    [x1, -y1, 0.0],
                    [x0, -y1, 0.0],
                ]),
            }
            uv.extend([[u0, v0], [u1, v0], [u1, v1], [u0, v1]]);
            let lin = |c: [f32; 4]| {
                Color::srgba(c[0], c[1], c[2], c[3].clamp(0.0, 1.0))
                    .to_linear()
                    .to_f32_array()
            };
            match q.vcolors {
                Some(vc) => col.extend(vc.map(lin)),
                None => col.extend([lin(q.color); 4]),
            }
            idx.extend([base, base + 2, base + 1, base, base + 3, base + 2]);
        }
        let mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, col)
        .with_inserted_indices(Indices::U32(idx));
        let mut e = commands.spawn((
            marker(),
            Mesh2d(meshes.add(mesh)),
            MeshMaterial2d(mat),
            Transform::from_xyz(origin.x, -origin.y, z0 + i as f32 * 0.01),
            bevy::camera::visibility::NoFrustumCulling,
        ));
        if let (true, Some(layer)) = (*bg, state.background_layer) {
            e.insert(bevy::camera::visibility::RenderLayers::layer(layer));
        }
        spawned.push(e.id());
    }
    spawned
}

/// Upload the glyph atlas if it changed.
pub fn sync_atlas(u: &mut Ui, state: &mut UiRenderState, images: &mut Assets<Image>) {
    if !u.fonts.atlas_dirty && state.atlas.is_some() {
        return;
    }
    let img = Image::new(
        bevy::render::render_resource::Extent3d {
            width: ATLAS_SIZE,
            height: ATLAS_SIZE,
            depth_or_array_layers: 1,
        },
        bevy::render::render_resource::TextureDimension::D2,
        u.fonts.atlas.clone(),
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    match &state.atlas {
        Some(h) => {
            let _ = images.insert(h.id(), img);
        }
        None => state.atlas = Some(images.add(img)),
    }
    u.fonts.atlas_dirty = false;
}
