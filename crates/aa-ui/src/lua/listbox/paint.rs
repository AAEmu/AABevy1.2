//! Authored state rectangles, native 64px cached icon sources and separator anchors.
use super::*;

/// Native cache cells are 64x64 (3921e161 / 3922439e). The DDS decoder preserves header
/// dimensions (renderer381d6740), so ordinary 48px item DDS are rejected by this cache.
pub(super) fn icon(lua: &Lua, path: Option<String>) -> Option<String> {
    let path = path?;
    let mut u = ui(lua);
    (u.textures.size(&path) == Some((64, 64)))
        .then(|| u.textures.resolve(&path))
        .flatten()
}

pub(super) fn methods(lua: &Lua, m: &Table) -> mlua::Result<()> {
    m.raw_set(
        "SetListItemStateTexture",
        method(lua, |lua, id, a| {
            let path = a.s(1).and_then(|p| ui(lua).textures.resolve(&p));
            change(lua, id, |d| {
                let dirty = d.state_texture != path;
                d.state_texture = path;
                ((), dirty)
            });
            none()
        })?,
    )?;
    m.raw_set(
        "SetListItemStateTextureInset",
        method(lua, |lua, id, a| {
            let inset = [a.f(1), a.f(2), a.f(3), a.f(4)];
            change(lua, id, |d| {
                let dirty = d.state_inset != inset;
                d.state_inset = inset;
                ((), dirty)
            });
            none()
        })?,
    )?;
    for (name, state) in [("Default", 0), ("Overed", 1), ("Selected", 2)] {
        m.raw_set(
            format!("Set{name}ItemCoord"),
            method(lua, move |lua, id, a| {
                let path = data(&ui(lua), id).and_then(|d| d.state_texture.clone());
                if let Some((w, h)) = path.and_then(|p| ui(lua).textures.size(&p)) {
                    let c = [a.f(1), a.f(2), a.f(3), a.f(4)];
                    // Native zero width/height extends to texture edge, not a degenerate quad.
                    let uv = [
                        c[0] / w as f32,
                        c[1] / h as f32,
                        if c[2] == 0.0 {
                            1.0
                        } else {
                            (c[0] + c[2]) / w as f32
                        },
                        if c[3] == 0.0 {
                            1.0
                        } else {
                            (c[1] + c[3]) / h as f32
                        },
                    ];
                    change(lua, id, |d| {
                        let dirty = d.state_coords[state] != uv;
                        d.state_coords[state] = uv;
                        ((), dirty)
                    });
                }
                none()
            })?,
        )?;
    }
    Ok(())
}

pub(super) fn quad(
    tex: Tex,
    rect: Rect,
    uv: [f32; 4],
    color: [f32; 4],
    scale: f32,
    out: &mut Vec<Quad>,
) {
    if rect.w <= 0.0 || rect.h <= 0.0 || color[3] <= 0.0 {
        return;
    }
    out.push(Quad {
        tex,
        pos: [
            rect.x * scale,
            rect.y * scale,
            (rect.x + rect.w) * scale,
            (rect.y + rect.h) * scale,
        ],
        uv,
        color,
        corners: None,
        uvs: None,
        vcolors: None,
    });
}

pub(super) fn state(
    u: &mut Ui,
    d: &ListBoxData,
    state: usize,
    base: Rect,
    alpha: f32,
    scale: f32,
    out: &mut Vec<Quad>,
) {
    let [l, t, r, b] = d.state_inset;
    let mut color = d.colors[state];
    color[3] *= alpha;
    if color[3] <= 0.0 {
        return;
    }
    let (tex, uv) = match d.state_texture.as_ref() {
        Some(path) => (Tex::Image(path.clone()), d.state_coords[state]),
        None => {
            // Native 392a0a2d/0a96/0aec: white for selected/default, grey for hover.
            let path = if state == 1 {
                "textures/defaults/grey.dds"
            } else {
                "textures/defaults/white.dds"
            };
            match u.textures.resolve(path) {
                Some(path) => (Tex::Image(path), [0., 0., 1., 1.]),
                None => (Tex::Atlas, crate::render::white_uv()),
            }
        }
    };
    quad(
        tex,
        Rect {
            x: base.x + l,
            y: base.y + t,
            w: base.w - l - r,
            h: base.h - t - b,
        },
        uv,
        color,
        scale,
        out,
    );
}
