//! Native drawable methods (image, nine/three part, colour and text drawables).

use mlua::{Lua, Table, Value};

use super::widget::parse_anchor;
use super::{method, none, ret, ui};
use crate::tree::Id;

fn with<R>(lua: &Lua, id: Id, f: impl FnOnce(&mut crate::tree::Drawable) -> R) -> Option<R> {
    ui(lua).tree.drawable_mut(id).map(f)
}

pub(crate) fn methods(lua: &Lua, class: &str) -> mlua::Result<Table> {
    let m = lua.create_table()?;
    let def = |name: &str, f: mlua::Function| m.raw_set(name, f);

    def(
        "AddAnchor",
        method(lua, |lua, id, a| {
            if let Some(an) = parse_anchor(lua, id, a) {
                ui(lua).tree.add_anchor(id, an);
            }
            none()
        })?,
    )?;
    def(
        "RemoveAllAnchors",
        method(lua, |lua, id, _| {
            ui(lua).tree.get_mut(id).anchors.clear();
            none()
        })?,
    )?;
    def(
        "SetExtent",
        method(lua, |lua, id, a| {
            let mut u = ui(lua);
            let o = u.tree.get_mut(id);
            o.width = a.f(1);
            o.height = a.f(2);
            none()
        })?,
    )?;
    def(
        "SetWidth",
        method(lua, |lua, id, a| {
            ui(lua).tree.get_mut(id).width = a.f(1);
            none()
        })?,
    )?;
    def(
        "SetHeight",
        method(lua, |lua, id, a| {
            ui(lua).tree.get_mut(id).height = a.f(1);
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
            let c = with(lua, id, |d| d.coords).flatten().unwrap_or_default();
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
        method(lua, |lua, id, a| {
            let c = [a.f(1), a.f(2), a.f(3), a.opt_f(4).unwrap_or(1.0)];
            with(lua, id, |d| d.color = c);
            none()
        })?,
    )?;
    def(
        "GetColor",
        method(lua, |lua, id, _| {
            // A table `{ r, g, b, a }` (`colorTable[4] == 0` in item_icon_button).
            let c = with(lua, id, |d| d.color).unwrap_or([1.0; 4]);
            ret(lua, lua.create_sequence_from(c)?)
        })?,
    )?;
    def(
        "SetAlpha",
        method(lua, |lua, id, a| {
            ui(lua).tree.get_mut(id).alpha = a.f(1);
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
    def(
        "Animation",
        method(lua, |lua, id, a| {
            // (play, visible): frame animation; we show the first frame while it plays.
            let on = if a.len() >= 3 {
                a.b(2, true)
            } else {
                a.b(1, true)
            };
            ui(lua).tree.get_mut(id).shown = on;
            none()
        })?,
    )?;
    for name in [
        "SetSnap",
        "SetRotation",
        "SetBlendMode",
        "SetMask",
        "SetMaskCoords",
        "SetMaskVisible",
        "SetTextureColor",
    ] {
        def(name, method(lua, |_, _, _| none())?)?;
    }
    if class == "drawable:effect" {
        def(
            "SetStartEffect",
            method(lua, |lua, id, a| {
                let on = a.b(1, true);
                with(lua, id, |d| d.effect = Some(on));
                none()
            })?,
        )?;
        def(
            "IsStartEffect",
            method(lua, |lua, id, _| {
                ret(lua, with(lua, id, |d| d.effect == Some(true)).unwrap_or(false))
            })?,
        )?;
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
