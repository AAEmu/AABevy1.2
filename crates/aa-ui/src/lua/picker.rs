//! `paintcolorpicker` widgets (the colour pallets of character create step 2): a colour held as
//! RGB, read and written as RGB or HLS (all 0..1, hue 0..1 around the wheel, like Python's
//! `colorsys`), a spectrum child (hue across, saturation down) and a luminance child (light at
//! the top, dark at the bottom). `SetSpectrumBg(d)` / `SetLuminanceBg(d)` name the frames the
//! gradients are drawn in.

use mlua::{Lua, Table, Value};

use super::widget::create_child;
use super::{method, none, object_table, ret, ui, value_id};
use crate::tree::Id;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PickerData {
    pub rgb: [f32; 3],
    pub spectrum_bg: Option<Id>,
    pub luminance_bg: Option<Id>,
}

pub fn rgb_to_hls([r, g, b]: [f32; 3]) -> [f32; 3] {
    let (mx, mn) = (r.max(g).max(b), r.min(g).min(b));
    let l = (mx + mn) / 2.0;
    if mx == mn {
        return [0.0, l, 0.0];
    }
    let d = mx - mn;
    let s = if l <= 0.5 {
        d / (mx + mn)
    } else {
        d / (2.0 - mx - mn)
    };
    let h = if mx == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if mx == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    [h / 6.0, l, s]
}

pub fn hls_to_rgb([h, l, s]: [f32; 3]) -> [f32; 3] {
    if s == 0.0 {
        return [l, l, l];
    }
    let m2 = if l <= 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let m1 = 2.0 * l - m2;
    let v = |hue: f32| {
        let hue = hue.rem_euclid(1.0);
        if hue < 1.0 / 6.0 {
            m1 + (m2 - m1) * hue * 6.0
        } else if hue < 0.5 {
            m2
        } else if hue < 2.0 / 3.0 {
            m1 + (m2 - m1) * (2.0 / 3.0 - hue) * 6.0
        } else {
            m1
        }
    };
    [v(h + 1.0 / 3.0), v(h), v(h - 1.0 / 3.0)]
}

fn with<R>(lua: &Lua, id: Id, f: impl FnOnce(&mut PickerData) -> R) -> Option<R> {
    let mut u = ui(lua);
    let w = u.tree.widget_mut(id)?;
    Some(f(w.picker.get_or_insert_with(PickerData::default)))
}

pub(crate) fn methods(lua: &Lua, m: &Table) -> mlua::Result<()> {
    let def = |name: &str, f: mlua::Function| m.raw_set(name, f);
    // Spectrum / luminance children, created on first use.
    for (getter, field) in [
        ("GetSpectrumWidget", "__spectrum"),
        ("GetLuminanceWidget", "__luminance"),
    ] {
        def(
            getter,
            method(lua, move |lua, id, _| {
                let this = object_table(lua, id)?.unwrap();
                if let Some(t) = this.raw_get::<Option<Table>>(field)? {
                    return ret(lua, t);
                }
                let child = create_child(lua, Some(id), "emptywidget", field, 0, false)?;
                if let Some(cid) = value_id(lua, &Value::Table(child.clone())) {
                    ui(lua).tree.get_mut(cid).shown = true;
                }
                this.raw_set(field, &child)?;
                ret(lua, child)
            })?,
        )?;
    }
    def(
        "SetRGBColor",
        method(lua, |lua, id, a| {
            let c = [a.f(1), a.f(2), a.f(3)].map(|v| v.clamp(0.0, 1.0));
            with(lua, id, |p| p.rgb = c);
            none()
        })?,
    )?;
    def(
        "GetRGBColor",
        method(lua, |lua, id, _| {
            let [r, g, b] = with(lua, id, |p| p.rgb).unwrap_or_default();
            ret(lua, (r, g, b))
        })?,
    )?;
    def(
        "SetHLSColor",
        method(lua, |lua, id, a| {
            let hls = [a.f(1), a.f(2).clamp(0.0, 1.0), a.f(3).clamp(0.0, 1.0)];
            with(lua, id, |p| p.rgb = hls_to_rgb(hls));
            none()
        })?,
    )?;
    def(
        "GetHLSColor",
        method(lua, |lua, id, _| {
            let [h, l, s] = rgb_to_hls(with(lua, id, |p| p.rgb).unwrap_or_default());
            ret(lua, (h, l, s))
        })?,
    )?;
    for (name, spectrum) in [("SetSpectrumBg", true), ("SetLuminanceBg", false)] {
        def(
            name,
            method(lua, move |lua, id, a| {
                let d = value_id(lua, &a.v(1));
                with(lua, id, |p| {
                    if spectrum {
                        p.spectrum_bg = d;
                    } else {
                        p.luminance_bg = d;
                    }
                });
                none()
            })?,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hls_round_trip() {
        for c in [
            [1.0, 0.0, 0.0],
            [0.2, 0.6, 0.4],
            [0.5, 0.5, 0.5],
            [0.9, 0.8, 0.1],
        ] {
            let back = hls_to_rgb(rgb_to_hls(c));
            for i in 0..3 {
                assert!((back[i] - c[i]).abs() < 1e-5, "{c:?} -> {back:?}");
            }
        }
        assert_eq!(rgb_to_hls([1.0, 0.0, 0.0]), [0.0, 0.5, 1.0]);
    }
}
