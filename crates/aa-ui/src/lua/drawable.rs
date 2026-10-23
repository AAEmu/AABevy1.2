//! Native drawable methods (image, nine/three part, colour and text drawables).

use mlua::{Lua, Table, Value};

use super::widget::parse_anchor;
use super::{method, none, ret, ui};
use crate::tree::{Align, Id};

fn with<R>(lua: &Lua, id: Id, f: impl FnOnce(&mut crate::tree::Drawable) -> R) -> Option<R> {
    ui(lua).tree.drawable_mut(id).map(f)
}

/// Read a drawable's state (no change: the layout cache stays).
fn read<R>(lua: &Lua, id: Id, f: impl FnOnce(&crate::tree::Drawable) -> R) -> Option<R> {
    ui(lua).tree.get(id).drawable().map(f)
}

pub(crate) fn methods(lua: &Lua, class: &str) -> mlua::Result<Table> {
    let m = lua.create_table()?;
    let text_drawable = class == "drawable:text";
    let def = |name: &str, f: mlua::Function| m.raw_set(name, f);

    def(
        "AddAnchor",
        method(lua, |lua, id, a| {
            if let Some(an) = parse_anchor(lua, id, a) {
                let accepted = ui(lua).tree.install_anchor(id, an);
                if accepted { super::anchors::changed(lua, id); }
            }
            none()
        })?,
    )?;
    def(
        "RemoveAllAnchors",
        method(lua, |lua, id, _| {
            ui(lua).tree.remove_anchors(id);
            none()
        })?,
    )?;
    def(
        "SetExtent",
        method(lua, |lua, id, a| {
            if !a.is_num(1) || !a.is_num(2) { return none(); }
            let mut u = ui(lua);
            u.tree.prepare_anchor_change(id);
            let o = u.tree.get_mut(id);
            o.width = a.f(1);
            o.height = a.f(2);
            drop(u);
            super::anchors::size_changed(lua, id, a);
            none()
        })?,
    )?;
    def(
        "SetWidth",
        method(lua, |lua, id, a| {
            if !a.is_num(1) { return none(); }
            ui(lua).tree.prepare_anchor_change(id);
            ui(lua).tree.get_mut(id).width = a.f(1);
            super::anchors::size_changed(lua, id, a);
            none()
        })?,
    )?;
    def(
        "SetHeight",
        method(lua, |lua, id, a| {
            if !a.is_num(1) { return none(); }
            ui(lua).tree.prepare_anchor_change(id);
            ui(lua).tree.get_mut(id).height = a.f(1);
            super::anchors::size_changed(lua, id, a);
            none()
        })?,
    )?;
    def(
        "GetWidth",
        method(lua, |lua, id, _| ret(lua, ui(lua).tree.size(id).0))?,
    )?;
    def(
        "GetHeight",
        method(lua, |lua, id, _| ret(lua, ui(lua).tree.size(id).1))?,
    )?;
    def(
        "GetExtent",
        method(lua, |lua, id, _| ret(lua, ui(lua).tree.size(id)))?,
    )?;
    def(
        "GetOffset",
        method(lua, |lua, id, _| {
            let r = ui(lua).tree.rect(id);
            ret(lua, (r.x, r.y))
        })?,
    )?;
    def(
        "SetVisible",
        method(lua, |lua, id, a| {
            ui(lua).tree.get_mut(id).shown = a.b(1, true);
            none()
        })?,
    )?;
    def(
        "Show",
        method(lua, |lua, id, a| {
            ui(lua).tree.get_mut(id).shown = a.b(1, true);
            none()
        })?,
    )?;
    def(
        "IsVisible",
        method(lua, |lua, id, _| ret(lua, ui(lua).tree.get(id).shown))?,
    )?;
    def(
        "SetCoords",
        method(lua, |lua, id, a| {
            let c = [a.f(1), a.f(2), a.f(3), a.f(4)];
            with(lua, id, |d| d.coords = Some(c));
            none()
        })?,
    )?;
    def(
        "GetCoords",
        method(lua, |lua, id, _| {
            let c = read(lua, id, |d| d.coords).flatten().unwrap_or_default();
            ret(lua, (c[0], c[1], c[2], c[3]))
        })?,
    )?;
    def(
        "SetInset",
        method(lua, |lua, id, a| {
            let i = [a.f(1), a.f(2), a.f(3), a.f(4)];
            let mut u = ui(lua);
            let o = u.tree.get_mut(id);
            if let Some(t) = o.text_mut() {
                t.inset = i;
            }
            if let Some(d) = o.drawable_mut() {
                d.inset = i;
            }
            none()
        })?,
    )?;
    def(
        "SetColor",
        method(lua, move |lua, id, a| {
            let c = [a.f(1), a.f(2), a.f(3), a.opt_f(4).unwrap_or(1.0)];
            if text_drawable {
                if read(lua, id, |d| d.text.as_ref().map(|t| t.style.color)).flatten() != Some(c) {
                    if let Some(t) = ui(lua).tree.get_mut(id).text_mut() {
                        t.style.color = c;
                    }
                }
            } else if read(lua, id, |d| d.color) != Some(c) {
                with(lua, id, |d| d.color = c);
            }
            none()
        })?,
    )?;
    def(
        "GetColor",
        method(lua, |lua, id, _| {
            // Native39235760 returns a fresh named RGBA table, with no numeric aliases.
            let Some(c) = read(lua, id, |d| d.color) else { return none() };
            let table = lua.create_table_with_capacity(0, 4)?;
            for (name, value) in ["r", "g", "b", "a"].into_iter().zip(c) {
                table.raw_set(name, value)?;
            }
            ret(lua, table)
        })?,
    )?;
    def(
        "SetAlpha",
        method(lua, |lua, id, a| {
            let (mut u, v) = (ui(lua), a.f(1));
            if u.tree.get(id).alpha != v {
                u.tree.get_mut(id).alpha = v;
            }
            none()
        })?,
    )?;
    def(
        "SetTexture",
        method(lua, |lua, id, a| {
            let p = a.s(1);
            with(lua, id, |d| d.texture = p);
            none()
        })?,
    )?;
    if class == "drawable:iconimage" {
        // Icon images stack textures: the first `AddTexture` is the image, later ones overlay it.
        def(
            "AddTexture",
            method(lua, |lua, id, a| {
                let Some(p) = a.s(1) else { return none() };
                with(lua, id, |d| match d.texture {
                    None => d.texture = Some(p),
                    Some(_) => d.extra_textures.push(p),
                });
                none()
            })?,
        )?;
        def(
            "ClearAllTextures",
            method(lua, |lua, id, _| {
                with(lua, id, |d| {
                    d.texture = None;
                    d.extra_textures.clear();
                });
                none()
            })?,
        )?;
    }
    if matches!(class, "drawable:image" | "drawable:iconimage") {
        def("SetMask", method(lua, |lua, id, a| {
            let Value::String(raw) = a.v(1) else { return none() };
            let raw = raw.as_bytes();
            let path = String::from_utf8_lossy(raw.split(|&b| b == 0).next().unwrap_or_default());
            let mut u = ui(lua);
            if !mask_class(&u, id) { return none(); }
            if let Some(texture) = u.textures.resolve(&path)
                && u.textures.size(&texture).is_some()
            {
                u.tree.drawable_mut(id).unwrap().mask = Some(crate::tree::DrawableMask {
                    texture, uv: [0., 0., 1., 1.],
                });
            }
            none()
        })?)?;
        def("SetMaskCoords", method(lua, |lua, id, a| {
            if !(1..=4).all(|i| a.is_num(i)) { return none(); }
            let mut u = ui(lua);
            if !mask_class(&u, id) { return none(); }
            let Some(mask) = u.tree.get(id).drawable().and_then(|d| d.mask.clone()) else {
                // Native icon39289680 dereferences the absent texture. Refuse safely.
                return none();
            };
            if let Some((w, h)) = u.textures.size(&mask.texture) {
                let (rw, rh) = (1.0_f32 / w as f32, 1.0_f32 / h as f32);
                let (x, y, w, h) = (a.f(1), a.f(2), a.f(3), a.f(4));
                u.tree.drawable_mut(id).unwrap().mask.as_mut().unwrap().uv = [
                    x * rw, y * rh,
                    if w == 0. { 1. } else { (x + w) * rw },
                    if h == 0. { 1. } else { (y + h) * rh },
                ];
            }
            none()
        })?)?;
        def("SetMaskVisible", method(lua, |lua, id, a| {
            let visible = match a.v(1) {
                Value::Boolean(v) => v,
                Value::Integer(v) => v != 0,
                Value::Number(v) => v != 0.,
                _ => return none(),
            };
            let mut u = ui(lua);
            if mask_class(&u, id) && u.tree.get(id).drawable().unwrap().mask_visible != visible {
                u.tree.drawable_mut(id).unwrap().mask_visible = visible;
            }
            none()
        })?)?;
    }
    def(
        "SetTextureInfo",
        method(lua, |lua, id, a| {
            // (key[, ...]): coords and extent from the texture's .g file.
            let key = a.s(1).unwrap_or_default();
            let mut g = ui(lua);
            let u = &mut *g;
            let tex = u.tree.get(id).drawable().and_then(|d| d.texture.clone());
            if let Some(info) = tex.and_then(|t| u.textures.info(&t, &key)) {
                let coords = info
                    .child("coords")
                    .map(|c| c.nums())
                    .filter(|v| v.len() >= 4);
                let extent = info
                    .child("extent")
                    .map(|c| c.nums())
                    .filter(|v| v.len() >= 2);
                let inset = info
                    .child("inset")
                    .map(|c| c.nums())
                    .filter(|v| v.len() >= 4);
                let o = u.tree.get_mut(id);
                if let Some(c) = &coords {
                    if let Some(d) = o.drawable_mut() {
                        d.coords = Some([c[0], c[1], c[2], c[3]]);
                    }
                    o.width = c[2].abs();
                    o.height = c[3].abs();
                }
                if let Some(e) = extent {
                    o.width = e[0];
                    o.height = e[1];
                }
                if let (Some(i), Some(d)) = (inset, o.drawable_mut()) {
                    d.inset = [i[0], i[1], i[2], i[3]];
                }
            }
            none()
        })?,
    )?;
    // Text drawables.
    def(
        "SetText",
        method(lua, |lua, id, a| {
            let s = a.s(1).unwrap_or_default();
            if let Some(t) = ui(lua).tree.get_mut(id).text_mut() {
                t.text = s;
            }
            none()
        })?,
    )?;
    def(
        "GetText",
        method(lua, |lua, id, _| {
            let s = ui(lua)
                .tree
                .get(id)
                .text()
                .map(|t| t.text.clone())
                .unwrap_or_default();
            ret(lua, s)
        })?,
    )?;
    if class == "drawable:image" {
        super::animation::frame_methods(lua, &m)?;
    }
    // `SetRotation(r)`: radians, clockwise on screen, about the centre of the drawable's rect.
    def(
        "SetRotation",
        method(lua, |lua, id, a| {
            let r = a.f(1);
            with(lua, id, |d| d.rotation = r);
            none()
        })?,
    )?;
    for name in [
        "SetSnap",
        "SetBlendMode",
        "SetTextureColor",
    ] {
        def(name, method(lua, |_, _, _| none())?)?;
    }
    if class == "drawable:text" {
        def(
            "SetAlign",
            method(lua, |lua, id, a| {
                if let Some(t) = ui(lua).tree.get_mut(id).text_mut() {
                    t.style.align = Align::from_i(a.f(1) as i64);
                }
                none()
            })?,
        )?;
        for name in ["SetShadow", "SetOutline"] {
            def(
                name,
                method(lua, move |lua, id, a| {
                    if let Some(t) = ui(lua).tree.get_mut(id).text_mut() {
                        if name == "SetShadow" {
                            t.style.shadow = a.b(1, false);
                        } else {
                            t.style.outline = a.b(1, false);
                        }
                    }
                    none()
                })?,
            )?;
        }
    }
    if class == "drawable:effect" {
        super::animation::effect_methods(lua, &m)?;
        // "ninepart" / "threepart" / "image": how the effect's texture is drawn.
        def(
            "SetInternalDrawType",
            method(lua, |lua, id, a| {
                let kind = match a.s(1).unwrap_or_default().to_ascii_lowercase().as_str() {
                    "ninepart" => crate::tree::DrawKind::NinePart,
                    "threepart" => crate::tree::DrawKind::ThreePart,
                    _ => crate::tree::DrawKind::Image,
                };
                with(lua, id, |d| d.kind = kind);
                none()
            })?,
        )?;
    }
    let _ = Value::Nil;
    super::stubs::finalize(lua, &m, class)?;
    Ok(m)
}

fn mask_class(u: &super::Ui, id: Id) -> bool {
    u.tree.get(id).drawable().is_some_and(|d| {
        matches!(d.class, "drawable:image" | "drawable:iconimage")
    })
}
