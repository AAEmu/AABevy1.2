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
}

/// Collect the quads of the whole tree in paint order.
pub fn build_quads(u: &mut Ui, pressed: Option<Id>) -> Vec<Quad> {
    let mut out = Vec::new();
    let order = u.tree.paint_order();
    let scale = u.ui_scale;
    for id in order {
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
        // Widget text above background/artwork, below overlay.
        if let Some(t) = w.text.as_ref() {
            let edit = crate::lua::edit::is_edit(&w.ty);
            let shown = if edit {
                let mut d = t.clone();
                d.text = crate::lua::edit::display_text(&t.text, w.edit.as_ref());
                std::borrow::Cow::Owned(d)
            } else {
                std::borrow::Cow::Borrowed(t)
            };
            let rect = u.tree.rect(id);
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
        if let Some(t) = w.title.as_ref() {
            if !t.text.is_empty() {
                let r = u.tree.rect(id);
                let rect = Rect {
                    x: r.x,
                    y: r.y + 10.0,
                    w: r.w,
                    h: 30.0,
                };
                let mut t = t.clone();
                t.style.align = crate::tree::Align::Center;
                text_quads(u, &t, rect, t.style.color, alpha, scale, &mut out);
            }
        }
        for d in above {
            drawable_quads(u, d, alpha, state, scale, &mut out);
        }
    }
    out
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
    if let Some(owner) = u.tree.get(d).parent {
        if let Some(w) = u.tree.widget(owner) {
            if let Some(b) = w.button.as_ref() {
                let disabled = state == ButtonState::Disabled;
                if b.checked_bg == Some(d) {
                    return b.checked && !(disabled && b.disabled_checked_bg.is_some());
                }
                if b.disabled_checked_bg == Some(d) {
                    return b.checked && disabled;
                }
            }
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
    let rect = u.tree.rect(d);
    let a = alpha * o.alpha;
    let color = [dr.color[0], dr.color[1], dr.color[2], dr.color[3] * a];
    if color[3] <= 0.0 || rect.w == 0.0 || rect.h == 0.0 {
        if dr.kind != DrawKind::Text {
            return;
        }
    }
    match dr.kind {
        DrawKind::Color => out.push(Quad {
            tex: Tex::Atlas,
            pos: px(rect, scale),
            uv: white_uv(),
            color,
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
            let c = dr.coords.unwrap_or([0.0, 0.0, tw, th]);
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
                            });
                        }
                    }
                }
                _ => out.push(Quad {
                    tex,
                    pos: px(rect, scale),
                    uv: [c[0] / tw, c[1] / th, (c[0] + c[2]) / tw, (c[1] + c[3]) / th],
                    color,
                }),
            }
        }
    }
}

fn white_uv() -> [f32; 4] {
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

fn text_quads(
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
                });
            }
        }
    }
}

/// The caret of the focused edit box: a 1 px bar after the character at the cursor, as tall as
/// a text line (or `SetCursorHeight`), in the text colour (or `SetCursorColor`).
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
    let full = u.fonts.layout(&t.style.font, size, &t.text, maxw, ls, false);
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
    let x = (ox + (full.width - full_line_w) * fx + line_w).round();
    let h = edit.and_then(|e| e.cursor_height).map_or(lh, |h| h * scale);
    let y = (oy + line as f32 * (lh + ls) + (lh - h) / 2.0).round();
    let c = edit.and_then(|e| e.cursor_color).unwrap_or(t.style.color);
    out.push(Quad {
        tex: Tex::Atlas,
        pos: [x, y, x + scale.max(1.0), y + h],
        uv: white_uv(),
        color: [c[0], c[1], c[2], c[3] * alpha],
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
}

/// Marker for the UI camera.
#[derive(Component)]
pub struct UiCamera;

/// Marker for batch meshes.
#[derive(Component)]
pub struct UiBatch;

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
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
    images: &mut Assets<Image>,
    formats: Option<&CompressedImageFormatSupport>,
    existing: &Query<Entity, With<UiBatch>>,
) {
    let formats = formats.map(|f| f.0).unwrap_or(CompressedImageFormats::BC);
    // Group consecutive quads by texture.
    let mut batches: Vec<(Tex, Vec<&Quad>)> = Vec::new();
    for q in quads {
        match batches.last_mut() {
            Some((t, v)) if *t == q.tex => v.push(q),
            _ => batches.push((q.tex.clone(), vec![q])),
        }
    }
    for e in existing.iter() {
        commands.entity(e).despawn();
    }
    state.batches.clear();
    for (i, (tex, qs)) in batches.iter().enumerate() {
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
            pos.extend([
                [x0, -y0, 0.0],
                [x1, -y0, 0.0],
                [x1, -y1, 0.0],
                [x0, -y1, 0.0],
            ]);
            uv.extend([[u0, v0], [u1, v0], [u1, v1], [u0, v1]]);
            let c = Color::srgba(
                q.color[0],
                q.color[1],
                q.color[2],
                q.color[3].clamp(0.0, 1.0),
            )
            .to_linear()
            .to_f32_array();
            col.extend([c; 4]);
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
        let e = commands
            .spawn((
                UiBatch,
                Mesh2d(meshes.add(mesh)),
                MeshMaterial2d(mat),
                Transform::from_xyz(0.0, 0.0, i as f32 * 0.01),
                bevy::camera::visibility::NoFrustumCulling,
            ))
            .id();
        state.batches.push(e);
    }
    state.quads = quads.len();
    state.draw_calls = batches.len();
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
