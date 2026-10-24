//! Only the recovered local-shop modelview methods. Customizer methods stay untouched.
use mlua::{Lua, Table};

use super::{Args, none, ret, ui};
use crate::ModelViewOp;
use crate::tree::{Anchor, DrawKind, DrawLayer, Id, Point, Tree};

fn method<F>(lua: &Lua, f: F) -> mlua::Result<mlua::Function>
where
    F: Fn(&Lua, Id, &Args) -> mlua::Result<mlua::MultiValue> + 'static,
{
    super::method(lua, move |lua, id, a| {
        if !ui(lua).tree.modelviews.live.contains_key(&id) {
            return none();
        }
        f(lua, id, a)
    })
}

// Native mulss constants, including the slightly different inverse conversion.
const TO_RADIANS: f32 = std::f32::consts::PI / 180.;
const TO_DEGREES: f32 = 57.295_776;

fn ints<const N: usize>(a: &Args) -> Option<[i32; N]> {
    if !(1..=N).all(|i| a.is_num(i)) {
        return None;
    }
    Some(std::array::from_fn(|i| match a.v(i + 1) {
        mlua::Value::Integer(n) => n as i32,
        mlua::Value::Number(n) if n.is_finite() && (-2147483648.0..2147483648.0).contains(&n) => {
            n as i32
        }
        _ => i32::MIN,
    }))
}

fn extent(tree: &mut Tree, id: Id, size: [i32; 2]) {
    let Some(v) = tree.modelviews.live.get_mut(&id) else {
        return;
    };
    v.extent = size;
    let d = v.drawable.filter(|&d| tree.objects[d as usize].alive);
    let d = d.unwrap_or_else(|| {
        let d = tree.create_drawable(
            id,
            DrawKind::Image,
            "drawable:modelview",
            DrawLayer::Background,
        );
        tree.get_mut(d).anchors = vec![
            Anchor {
                point: Point::TopLeft,
                target: id,
                rel: Point::TopLeft,
                x: 0.,
                y: 0.,
            },
            Anchor {
                point: Point::BottomRight,
                target: id,
                rel: Point::BottomRight,
                x: 0.,
                y: 0.,
            },
        ];
        tree.modelviews.live.get_mut(&id).unwrap().drawable = Some(d);
        d
    });
    let crop = tree.modelviews.live[&id].crop.map(|v| v as f32);
    let o = tree.get_mut(d);
    o.width = size[0] as f32;
    o.height = size[1] as f32;
    o.drawable_mut().unwrap().coords = Some(crop);
}

pub(super) fn methods(lua: &Lua, m: &Table) -> mlua::Result<()> {
    m.raw_set(
        "SetTextureSize",
        method(lua, |lua, id, a| {
            if let Some(size) = ints::<2>(a).filter(|s| s.iter().all(|&n| n > 0)) {
                let mut u = ui(lua);
                if let Some(v) = u.tree.modelviews.live.get_mut(&id) {
                    v.requested = size;
                }
                u.tree
                    .modelviews
                    .push(id, ModelViewOp::SetTextureSize(size));
            }
            none()
        })?,
    )?;
    m.raw_set(
        "SetModelViewExtent",
        method(lua, |lua, id, a| {
            if let Some(size) = ints::<2>(a) {
                extent(&mut ui(lua).tree, id, size);
            }
            none()
        })?,
    )?;
    m.raw_set(
        "SetModelViewCoords",
        method(lua, |lua, id, a| {
            if let Some(crop) = ints::<4>(a) {
                let mut u = ui(lua);
                let d = u
                    .tree
                    .modelviews
                    .live
                    .get(&id)
                    .and_then(|v| v.drawable)
                    .filter(|&d| u.tree.get(d).alive);
                // Native does not even store the crop until a displayed drawable exists.
                if let Some(d) = d {
                    u.tree.modelviews.live.get_mut(&id).unwrap().crop = crop;
                    u.tree.drawable_mut(d).unwrap().coords = Some(crop.map(|v| v as f32));
                }
            }
            none()
        })?,
    )?;
    m.raw_set(
        "Init",
        method(lua, |lua, id, a| {
            if let Some(unit) = a.s(1) {
                let mut u = ui(lua);
                let size = u.tree.modelviews.live.get(&id).map(|v| v.extent);
                // A successful host Init may rebind this native drawable after Clear. Its
                // existence alone admits no texture: the host must associate a real image.
                if let Some(size) = size {
                    extent(&mut u.tree, id, size);
                }
                u.tree.modelviews.push(
                    id,
                    ModelViewOp::Init {
                        unit,
                        create_model: a.b(2, false),
                    },
                );
            }
            // Unresolved native input returns false; the valid branch submits true then false.
            // Its real VM stack is unproved. Expose one conservative false, never readiness.
            ret(lua, false)
        })?,
    )?;
    m.raw_set(
        "SetIngameShopMode",
        method(lua, |lua, id, a| {
            ui(lua)
                .tree
                .modelviews
                .push(id, ModelViewOp::SetIngameShopMode(a.b(1, false)));
            none()
        })?,
    )?;
    m.raw_set(
        "AdjustCameraPos",
        method(lua, |lua, id, a| {
            if (1..=3).all(|i| a.is_num(i)) {
                ui(lua)
                    .tree
                    .modelviews
                    .push(id, ModelViewOp::AdjustCameraPos([a.f(1), a.f(2), a.f(3)]));
            }
            none()
        })?,
    )?;
    m.raw_set(
        "EquipItem",
        method(lua, |lua, id, a| {
            if let Some([item]) = ints::<1>(a) {
                ui(lua)
                    .tree
                    .modelviews
                    .push(id, ModelViewOp::EquipItem(item as u32));
            }
            none()
        })?,
    )?;
    m.raw_set(
        "PlayAnimation",
        method(lua, |lua, id, a| {
            if let Some(name) = a.s(1) {
                ui(lua).tree.modelviews.push(
                    id,
                    ModelViewOp::PlayAnimation {
                        name,
                        looping: a.b(2, false),
                    },
                );
            }
            none()
        })?,
    )?;
    for (name, op) in [
        ("ResetEquips", ModelViewOp::ResetEquips),
        ("StopAnimation", ModelViewOp::StopAnimation),
    ] {
        m.raw_set(
            name,
            method(lua, move |lua, id, _| {
                ui(lua).tree.modelviews.push(id, op.clone());
                none()
            })?,
        )?;
    }
    for (name, additive) in [("SetRotation", false), ("AddRotation", true)] {
        m.raw_set(
            name,
            method(lua, move |lua, id, a| {
                if a.is_num(1) {
                    let mut u = ui(lua);
                    if let Some(v) = u.tree.modelviews.live.get_mut(&id) {
                        let radians = a.f(1) * TO_RADIANS;
                        v.rotation = if additive {
                            radians + v.rotation
                        } else {
                            radians
                        };
                        let rotation = v.rotation;
                        u.tree
                            .modelviews
                            .push(id, ModelViewOp::SetRotation(rotation));
                    }
                }
                none()
            })?,
        )?;
    }
    m.raw_set(
        "GetRotation",
        method(lua, |lua, id, _| {
            let rotation = ui(lua)
                .tree
                .modelviews
                .live
                .get(&id)
                .map(|v| v.rotation * TO_DEGREES);
            match rotation {
                Some(v) => ret(lua, v),
                None => none(),
            }
        })?,
    )?;
    m.raw_set(
        "ClearModel",
        method(lua, |lua, id, _| {
            let mut u = ui(lua);
            if let Some(v) = u.tree.modelviews.live.get_mut(&id) {
                v.image = None;
                if let Some(d) = v.drawable.take() {
                    u.tree.get_mut(d).alive = false;
                    if let Some(w) = u.tree.widget_mut(id) {
                        w.drawables.retain(|&old| old != d);
                    }
                }
                u.tree.repaint();
                u.tree.modelviews.push(id, ModelViewOp::ClearModel);
            }
            none()
        })?,
    )?;
    Ok(())
}
