//! Turns the widget tree into a few batched 2D meshes.
//!
//! Every visible drawable / text glyph becomes a textured, vertex-coloured quad in paint order;
//! consecutive quads that use the same texture share one mesh (one draw call). The meshes are
//! rebuilt only when the tree changes, so a static screen costs nothing per frame.
//!
//! World units are physical pixels: the UI camera looks at `(W/2, -H/2)` so UI coordinate
//! `(x, y)` (y down) is world `(x, -y)`.

use std::collections::{HashMap, HashSet};

use bevy::asset::{RenderAssetUsages, embedded_asset};
use bevy::image::{CompressedImageFormatSupport, CompressedImageFormats, ImageSampler, ImageType};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dPlugin};

use crate::lua::Ui;
use crate::text::ATLAS_SIZE;
use crate::tree::{ButtonState, DrawKind, Id, Kind, Rect, TextBlock, Tree, WidgetType};

pub(crate) mod clip;

/// Texture of a quad.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Tex {
    /// The glyph atlas (also has a white texel at 0,0 for solid quads).
    Atlas,
    /// A pak texture (resolved path).
    Image(String),
    /// The native image-plus-mask pass. UV rectangles are raw f32 bits; the mesh UVs
    /// remain unit-square coordinates so both streams survive clipping and transforms.
    MaskedImage {
        image: String,
        mask: String,
        image_uv: [u32; 4],
        mask_uv: [u32; 4],
    },
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct MaskMaterial {
    #[uniform(0)]
    image_uv: Vec4,
    #[uniform(1)]
    mask_uv: Vec4,
    #[texture(2)]
    #[sampler(3)]
    image: Handle<Image>,
    #[texture(4)]
    #[sampler(5)]
    mask: Handle<Image>,
}

impl Material2d for MaskMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://aa_ui/cooldown_mask.wesl".into()
    }
    fn alpha_mode(&self) -> AlphaMode2d { AlphaMode2d::Blend }
}

pub(crate) fn install_material(app: &mut App) {
    embedded_asset!(app, "cooldown_mask.wesl");
    app.add_plugins(Material2dPlugin::<MaskMaterial>::default());
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
    /// UVs for clipped triangles; ordinary rectangles use `uv`.
    pub uvs: Option<[[f32; 2]; 4]>,
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
        let alpha = effective_alpha(u, id);
        if alpha <= 0.0 {
            continue;
        }
        crate::lua::auto_clip::geometry(u, id);
        let Some(w) = u.tree.widget(id).cloned() else {
            continue;
        };
        if w.ty == WidgetType::Root {
            continue;
        }
        let state = button_state(u, id, pressed);
        let quads_start = out.len();
        let mut physical_text = None;
        let mut divider_ranges = Vec::new();
        let mut ds: Vec<Id> = w
            .drawables
            .iter()
            .copied()
            .filter(|&d| {
                let o = u.tree.get(d);
                o.alive && o.shown && o.drawable().is_none_or(|dr| !dr.row_template)
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
            let first = out.len();
            drawable_quads(u, d, alpha, state, scale, None, &mut out);
            if w.chat.as_ref().is_some_and(|c| d == c.left_line || d == c.right_line) {
                divider_ranges.push((first..out.len(), d));
            }
        }
        if let Some(dg) = w.diagram.as_ref() {
            diagram_quads(u, id, dg, alpha, scale, &mut out);
        }
        if w.cooldown.is_some() {
            let background = w.button.as_ref().and_then(|b| b.bg[state_index(state)].or(b.bg[0])).map(|id| (id, state));
            crate::lua::slot::cooldown_quads(u, id, &w, alpha, scale, background, &mut out);
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
        } else if w.class == "damagedisplay" {
            crate::lua::damage::quads(u, id, &w, alpha, scale, &mut out);
        } else if let Some(d) = w.listbox.as_ref() {
            if let Some(t) = w.text.as_ref() {
                crate::lua::listbox::quads(u, id, d, t, alpha, scale, &mut out);
            }
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
            } else if t.auto_clip.enabled {
                let mut d = t.clone();
                d.text = crate::lua::auto_clip::shown_text(&t.text, t.auto_clip.count).to_string();
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
                if !edit {
                    if w.ty.is_button_like() && !u.fonts.is_image_font(&shown.style.font) {
                        let start = out.len();
                        button_text_quads(u, id, &shown, color, alpha, &mut out);
                        physical_text = Some(start..out.len());
                    } else {
                        text_quads(u, &shown, rect, color, alpha, scale, &mut out);
                    }
                }
            }
            if edit {
                let caret = u.focus == Some(id) && u.caret_visible;
                crate::lua::edit::render(u, id, &shown, w.edit.as_ref(), rect, alpha, scale, caret, &mut out);
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
            let first = out.len();
            drawable_quads(u, d, alpha, state, scale, None, &mut out);
            if w.chat.as_ref().is_some_and(|c| d == c.left_line || d == c.right_line) {
                divider_ranges.push((first..out.len(), d));
            }
        }
        // A widget's scale animation carries its text, art and descendants together.
        let mut owner = Some(id);
        while let Some(ancestor) = owner {
            let animation = u.animations.widget_sample(ancestor);
            if animation.scale != 1.0 || animation.offset != [0.0; 2] {
                let r = u.tree.rect(ancestor);
                let pivot = [(r.x + r.w * animation.pivot.fx()) * scale, (r.y + r.h * animation.pivot.fy()) * scale];
                let point = |p: [f32; 2]| std::array::from_fn(|i| pivot[i] + (p[i] - pivot[i]) * animation.scale + animation.offset[i] * scale);
                for (i, q) in out[quads_start..].iter_mut().enumerate() {
                    // Native ordinary button faces are already selected/rasterized at
                    // effective physical size. Their text must not be stretched again.
                    if physical_text.as_ref().is_some_and(|r| r.contains(&(quads_start + i))) {
                        continue;
                    }
                    let lo = point([q.pos[0], q.pos[1]]);
                    let hi = point([q.pos[2], q.pos[3]]);
                    q.pos = [lo[0], lo[1], hi[0], hi[1]];
                    if let Some(corners) = q.corners.as_mut() {
                        for corner in corners { *corner = point(*corner); }
                    }
                }
            }
            owner = u.tree.get(ancestor).parent;
        }
        // Clip after animation transforms, in the scaled ancestors' screen rectangles.
        let ancestor_clip = u.animations.widget_clip_rect(&u.tree, id).map(|c| px(c, scale));
        for (range, d) in divider_ranges.into_iter().rev() {
            if let Some(c) = crate::lua::chat::tab_clip(u, d) {
                let c = ancestor_clip.map_or(c, |old| crate::lua::chat::intersect_clip(c, old));
                clip_range(&mut out, range, c);
            }
        }
        let bounds = match (crate::lua::chat::tab_clip(u, id), ancestor_clip) {
            (Some(c), Some(old)) => Some(crate::lua::chat::intersect_clip(c, old)),
            (c, old) => c.or(old),
        };
        if let Some(c) = bounds {
            clip::quads(&mut out, quads_start, c);
        }
    }
    let n = background_end.unwrap_or(out.len());
    (out, n)
}

/// Internal dividers share their owner's paint range with unclipped content.
/// Clip only their own quads after transforms, including rotated/gradient art.
fn clip_range(out: &mut Vec<Quad>, range: std::ops::Range<usize>, bounds: [f32; 4]) {
    if out[range.clone()].iter().all(|q| q.corners.is_none() && q.uvs.is_none() && q.vcolors.is_none()) {
        clip_rectangles(&mut out[range], bounds);
    } else {
        let start = range.start;
        let mut part: Vec<Quad> = out.drain(range).collect();
        clip::quads(&mut part, 0, bounds);
        out.splice(start..start, part);
    }
}

/// Native button39280510/392218a0 draws an ordinary physical face with scale1.
/// Keep its paint position between artwork and overlay, while only this range
/// bypasses the later widget/ancestor transforms. ImageText keeps its bitmap path.
fn button_text_quads(u: &mut Ui, id: Id, t: &TextBlock, color: [f32; 4], alpha: f32, out: &mut Vec<Quad>) {
    let local = u.tree.rect(id);
    let view = u.animations.widget_rect(&u.tree, id);
    let scale = u.animations.widget_scale(&u.tree, id);
    let rect = Rect {
        x: view.x * u.ui_scale,
        y: view.y * u.ui_scale,
        // Native +74/+78 are multiplied independently of position; subtracting
        // rounded transformed corners can lose a width ULP at a large origin.
        w: local.w * scale,
        h: local.h * scale,
    };
    let mut text = t.clone();
    text.style.size = u.fonts.button_font_size(&t.style.font, t.style.size, scale);
    text.inset = t.inset.map(|v| v * scale);
    text.line_space *= scale;
    text_quads(u, &text, rect, color, alpha, 1.0, out);
}

/// Cut quads to the pixel rectangle `c` (x0, y0, x1, y1), moving the texture coordinates with
/// the cut edges; quads fully outside collapse to a point (zero area, draws nothing).
fn clip_rectangles(quads: &mut [Quad], c: [f32; 4]) {
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

fn effective_alpha(u: &Ui, mut id: Id) -> f32 {
    let mut a = 1.0;
    loop {
        let o = u.tree.get(id);
        if !crate::lua::damage::visible(u, id) { return 0.0; }
        a *= u.animations.widget_sample(id).alpha.unwrap_or(o.alpha);
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

pub(crate) fn drawable_quads(
    u: &mut Ui,
    d: Id,
    alpha: f32,
    state: ButtonState,
    scale: f32,
    base: Option<Rect>,
    out: &mut Vec<Quad>,
) {
    if !state_visible(u, d, state) {
        return;
    }
    let o = u.tree.get(d).clone();
    if !o.alive || !o.shown { return; }
    let Kind::Drawable(dr) = &o.kind else { return };
    if dr.effect == Some(false) {
        return;
    }
    // Native listbox templates are placed at fractional row centres. Keep their explicit
    // rectangle intact; the ordinary drawable pass retains its existing pixel snapping.
    let at_row = base.is_some();
    let pixels = |r: Rect| if at_row {
        [r.x * scale, r.y * scale, r.right() * scale, r.bottom() * scale]
    } else { px(r, scale) };
    let animation = u.animations.sample(d);
    let base = base.unwrap_or_else(|| u.tree.rect(d));
    let rect = animation.rect(base);
    // Status bar layout already has the filled extent; crop only its texture here.
    let mut coords = animation.coords.or(dr.coords);
    if let Some((f, vertical)) = dr.fill {
        if vertical {
            if let Some(c) = coords.as_mut() {
                // Native 392c81bd..c7 writes height*f into the width slot.
                c[2] = c[3] * f;
            }
        } else {
            if let Some(c) = coords.as_mut() {
                c[2] *= f;
            }
        }
    }
    // Native3927af50 pushes this owner's layer factor and restores it before children.
    // It does not belong in effective_alpha's whole-subtree product or text opacity.
    let layer_alpha = o.parent.and_then(|p| u.tree.widget(p))
        .map_or(1., |w| w.dynamic_drawable.current[dr.layer as usize]);
    let a = alpha * o.alpha * animation.alpha * layer_alpha;
    let c = animation.color.unwrap_or(dr.color);
    let color = [c[0], c[1], c[2], c[3] * a];
    if (color[3] <= 0.0 || rect.w == 0.0 || rect.h == 0.0) && dr.kind != DrawKind::Text {
        return;
    }
    let first_quad = out.len();
    match dr.kind {
        DrawKind::Color => out.push(Quad {
            tex: Tex::Atlas,
            pos: pixels(rect),
            uv: white_uv(),
            color,
            corners: None,
            uvs: None,
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
                (None, Some((f, true))) => [0.0, 0.0, th * f, th],
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
                                pos: pixels(r),
                                uv: [sx[i] / tw, sy[j] / th, sx[i + 1] / tw, sy[j + 1] / th],
                                color,
                                corners: None,
                                uvs: None,
                                vcolors: None,
                            });
                        }
                    }
                }
                _ => {
                    out.push(Quad {
                        tex,
                        pos: pixels(rect),
                        uv: [c[0] / tw, c[1] / th, (c[0] + c[2]) / tw, (c[1] + c[3]) / th],
                        color,
                        corners: None,
                        uvs: None,
                        vcolors: None,
                    });
                    // Icon image overlays (`AddTexture`), whole texture over the same rect.
                    for extra in &dr.extra_textures {
                        if let Some(r) = u.textures.resolve(extra) {
                            out.push(Quad {
                                tex: Tex::Image(r),
                                pos: pixels(rect),
                                uv: [0.0, 0.0, 1.0, 1.0],
                                color,
                                corners: None,
                                uvs: None,
                                vcolors: None,
                            });
                        }
                    }
                }
            }
        }
    }
    let angle = dr.rotation + animation.rotation;
    if angle != 0.0 {
        let p = pixels(rect);
        let center = [
            p[0] + (p[2] - p[0]) * animation.pivot.fx(),
            p[1] + (p[3] - p[1]) * animation.pivot.fy(),
        ];
        let (sin, cos) = angle.sin_cos();
        for q in &mut out[first_quad..] {
            let [x0, y0, x1, y1] = q.pos;
            let corners = q.corners.unwrap_or([[x0, y0], [x1, y0], [x1, y1], [x0, y1]]);
            q.corners = Some(corners.map(|[x, y]| {
                let (x, y) = (x - center[0], y - center[1]);
                [center[0] + x * cos - y * sin, center[1] + x * sin + y * cos]
            }));
        }
    }
    if animation.clip {
        // Native 3928f877 saves this base rectangle before effect scale/rotation/movement;
        // 3928fab9 clips to it for this phase, independently of ancestor scroll clipping.
        clip::quads(out, first_quad, pixels(base));
    }
}

pub(crate) fn white_uv() -> [f32; 4] {
    let t = 0.5 / ATLAS_SIZE as f32;
    [t, t, t, t]
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
    text_quads_mode(u, t, rect, color, alpha, scale, out, false);
}

pub(crate) fn edit_text_quads(
    u: &mut Ui, t: &TextBlock, rect: Rect, color: [f32; 4],
    alpha: f32, scale: f32, out: &mut Vec<Quad>,
) {
    text_quads_mode(u, t, rect, color, alpha, scale, out, true);
}

fn text_quads_mode(
    u: &mut Ui, t: &TextBlock, rect: Rect, color: [f32; 4],
    alpha: f32, scale: f32, out: &mut Vec<Quad>, edit: bool,
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
    let mut lo = if edit {
        u.fonts.layout_edit(&t.style.font, size, &text, maxw.map(|w| w * scale), t.line_space * scale)
    } else {
        u.fonts.layout(&t.style.font, size, &text, maxw.map(|w| w * scale), t.line_space * scale, false)
    };
    if !edit && t.style.ellipsis && !t.wrap && lo.width > inner.w * scale {
        // Trim characters until "text..." fits.
        let mut chars: Vec<char> = text.chars().collect();
        let mut shortened = None;
        while !chars.is_empty() {
            chars.pop();
            let s: String = chars.iter().collect::<String>() + "...";
            let l = u.fonts.layout(&t.style.font, size, &s, None, 0.0, false);
            if l.width <= inner.w * scale {
                shortened = Some((s, l));
                break;
            }
        }
        (text, lo) = shortened.unwrap_or_else(|| (
            String::new(), u.fonts.layout(&t.style.font, size, "", None, 0.0, false),
        ));
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
            uvs: None,
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
                uvs: None,
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
                    uvs: None,
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
                uvs: None,
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
            uvs: None,
            vcolors: None,
        });
    }
}

/// The caret of the focused edit box: a 1 px bar after the character at the cursor, as tall as
/// a text line plus `SetCursorHeight`, moved by `SetCursorOffset`, in the text colour (or
/// `SetCursorColor`).
/// Render-side state: textures, materials and the batch entities.
#[derive(Resource, Default)]
pub struct UiRenderState {
    pub generation: u64,
    pub screen: (u32, u32),
    pub images: HashMap<String, Handle<Image>>,
    pub materials: HashMap<Tex, Handle<ColorMaterial>>,
    pub mask_materials: HashMap<Tex, Handle<MaskMaterial>>,
    pub mask_images: HashMap<String, Handle<Image>>,
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
        out.push(Quad { tex: Tex::Image(resolved), pos: px(r, scale), uv: [0.0, 0.0, 1.0, 1.0], color: white, corners: None, uvs: None, vcolors: None });
        return out;
    }
    let r = Rect { x: -HELD_ICON / 2.0, y: -HELD_ICON / 2.0, w: HELD_ICON, h: HELD_ICON };
    for path in [h.icon.as_deref(), h.frame.as_deref()].into_iter().flatten() {
        if let Some(resolved) = u.textures.resolve(path) {
            out.push(Quad { tex: Tex::Image(resolved), pos: px(r, scale), uv: [0.0, 0.0, 1.0, 1.0], color: white, corners: None, uvs: None, vcolors: None });
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
    mask_materials: &mut Assets<MaskMaterial>,
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
            spawn_batches(commands, state, &quads, 0, meshes, materials, mask_materials, images, formats, || UiCursorBatch, pointer.unwrap_or(Vec2::splat(-10_000.0)), 100.0);
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
    mask_materials: &mut Assets<MaskMaterial>,
    images: &mut Assets<Image>,
    formats: Option<&CompressedImageFormatSupport>,
    existing: &Query<Entity, With<UiBatch>>,
) {
    for e in existing.iter() {
        commands.entity(e).despawn();
    }
    state.batches.clear();
    // UV setters can change a material view; retain only views still used by this mesh list.
    let mask_views: HashSet<_> = quads.iter().filter_map(|q| {
        matches!(q.tex, Tex::MaskedImage { .. }).then_some(&q.tex)
    }).collect();
    state.mask_materials.retain(|tex, _| mask_views.contains(tex));
    let ids = spawn_batches(commands, state, quads, background_quads, meshes, materials, mask_materials, images, formats, || UiBatch, Vec2::ZERO, 0.0);
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
    mask_materials: &mut Assets<MaskMaterial>,
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
        let image_path = match tex {
            Tex::Atlas => None,
            Tex::Image(p) | Tex::MaskedImage { image: p, .. } => Some(p),
        };
        let image = if let Some(p) = image_path {
            Some(state.images.entry(p.clone()).or_insert_with(|| {
                load_image(p, formats).map(|img| images.add(img)).unwrap_or_default()
            }).clone())
        } else { state.atlas.clone() };
        let masked = if let Tex::MaskedImage { mask, image_uv, mask_uv, .. } = tex {
            let mask = state.mask_images.entry(mask.clone()).or_insert_with(|| {
                load_image(mask, formats).map(|mut img| {
                    img.sampler = ImageSampler::Descriptor(bevy::image::ImageSamplerDescriptor {
                        address_mode_u: bevy::image::ImageAddressMode::Repeat,
                        address_mode_v: bevy::image::ImageAddressMode::Repeat,
                        ..bevy::image::ImageSamplerDescriptor::linear()
                    });
                    images.add(img)
                }).unwrap_or_default()
            }).clone();
            Some(state.mask_materials.entry(tex.clone()).or_insert_with(|| {
                mask_materials.add(MaskMaterial {
                    image_uv: Vec4::from_array(image_uv.map(f32::from_bits)),
                    mask_uv: Vec4::from_array(mask_uv.map(f32::from_bits)),
                    image: image.clone().unwrap_or_default(), mask,
                })
            }).clone())
        } else { None };
        let mat = if masked.is_none() {
            Some(state.materials.entry(tex.clone()).or_insert_with(|| {
                materials.add(ColorMaterial { texture: image, ..default() })
            }).clone())
        } else { None };
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
            uv.extend(q.uvs.unwrap_or([[u0, v0], [u1, v0], [u1, v1], [u0, v1]]));
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
            Transform::from_xyz(origin.x, -origin.y, z0 + i as f32 * 0.01),
            bevy::camera::visibility::NoFrustumCulling,
        ));
        if let Some(mat) = masked { e.insert(MeshMaterial2d(mat)); }
        else if let Some(mat) = mat { e.insert(MeshMaterial2d(mat)); }
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
