//! Native tree records and visible preorder (392454c0 / 392a1830 / 392a26e0).

use super::*;
use mlua::Value;

impl Row {
    fn count(&self) -> usize {
        1 + if self.opened {
            self.child.iter().map(Row::count).sum()
        } else {
            0
        }
    }
    pub(super) fn clear(&mut self, close: bool) -> bool {
        let mut changed = self.selected || (close && self.opened);
        self.selected = false;
        if close {
            self.opened = false;
        }
        for row in &mut self.child {
            changed |= row.clear(close);
        }
        changed
    }
    fn selected(&self) -> Option<&Row> {
        if self.selected {
            Some(self)
        } else {
            selected(&self.child)
        }
    }
}

pub(super) fn selected(rows: &[Row]) -> Option<&Row> {
    rows.iter().find_map(Row::selected)
}

impl ListBoxData {
    pub(super) fn opened_count(&self) -> usize {
        self.rows.iter().map(Row::count).sum()
    }
    pub(super) fn visible_rows(&self) -> Vec<(&Row, usize)> {
        fn add<'a>(rows: &'a [Row], depth: usize, out: &mut Vec<(&'a Row, usize)>) {
            for row in rows {
                out.push((row, depth));
                if row.opened {
                    add(&row.child, depth + 1, out);
                }
            }
        }
        let mut out = Vec::new();
        add(&self.rows, 0, &mut out);
        out
    }
    fn path(&self, mut index: usize) -> Option<Vec<usize>> {
        fn find(rows: &[Row], index: &mut usize, path: &mut Vec<usize>) -> bool {
            for (i, row) in rows.iter().enumerate() {
                path.push(i);
                if *index == 0 {
                    return true;
                }
                *index -= 1;
                if row.opened && find(&row.child, index, path) {
                    return true;
                }
                path.pop();
            }
            false
        }
        let mut path = Vec::new();
        find(&self.rows, &mut index, &mut path).then_some(path)
    }
    fn row_mut(&mut self, path: &[usize]) -> Option<&mut Row> {
        fn get<'a>(rows: &'a mut [Row], path: &[usize]) -> Option<&'a mut Row> {
            let (&i, tail) = path.split_first()?;
            let row = rows.get_mut(i)?;
            if tail.is_empty() {
                Some(row)
            } else {
                get(&mut row.child, tail)
            }
        }
        get(&mut self.rows, path)
    }
}

/// Invalid entries stop that sibling array, leaving its already appended prefix, as native.
fn parse(lua: &Lua, table: Table, ancestors: &mut Vec<usize>) -> mlua::Result<Vec<Row>> {
    let ptr = table.to_pointer() as usize;
    if ancestors.contains(&ptr) || ancestors.len() >= 64 {
        return Err(mlua::Error::RuntimeError(
            "cyclic or excessive listbox tree depth".into(),
        ));
    }
    ancestors.push(ptr);
    let mut rows = Vec::new();
    for index in 1..=table.raw_len() {
        let Value::Table(node) = table.raw_get(index)? else {
            break;
        };
        let Value::String(text) = node.raw_get("text")? else {
            break;
        };
        let number = |key| -> mlua::Result<Option<f32>> {
            Ok(match node.raw_get(key)? {
                Value::Number(n) => Some(n as f32),
                Value::Integer(n) => Some(n as f32),
                _ => None,
            })
        };
        let color = [
            number("colorR")?.unwrap_or(1.0),
            number("colorG")?.unwrap_or(1.0),
            number("colorB")?.unwrap_or(1.0),
            number("colorA")?.unwrap_or(1.0),
        ];
        let opened = matches!(node.raw_get("opened")?, Value::Boolean(true));
        let child = match node.raw_get("child")? {
            Value::Table(t) => parse(lua, t, ancestors)?,
            _ => Vec::new(),
        };
        rows.push(Row {
            text: text
                .to_string_lossy()
                .chars()
                .take_while(|c| *c != '\0')
                .take(511)
                .collect(),
            value: number("value")?.unwrap_or(0.0) as i32,
            color,
            selected: false,
            opened,
            child,
            icon: paint::icon(
                lua,
                match node.raw_get("iconPath")? {
                    Value::String(path) => Some(path.to_string_lossy()),
                    _ => None,
                },
            ),
        });
    }
    ancestors.pop();
    Ok(rows)
}

/// Whole-row toggling precedes selection and its separate native event.
pub(super) fn clicked(lua: &Lua, id: Id, index: usize) {
    let setup = change(lua, id, |d| {
        let Some(path) = d.path(index) else {
            return (None, false);
        };
        let old_index = d.selected;
        let allow_parent = d.select_parent;
        let row = d.row_mut(&path).unwrap();
        let parent = !row.child.is_empty();
        let closing_selected_parent = parent && row.opened && allow_parent;
        if parent {
            row.opened = !row.opened;
        }
        if closing_selected_parent {
            for row in &mut d.rows {
                row.clear(false);
            }
        }
        d.selected = if closing_selected_parent || (parent && !allow_parent) {
            -1
        } else {
            index as i32
        };
        (Some((path, parent, allow_parent, old_index, d.epoch)), true)
    })
    .flatten();
    let Some((path, parent, allow_parent, old_index, epoch)) = setup else {
        return;
    };
    if parent {
        call_handler(lua, id, "OnListboxToggled", vec![]);
    }
    let valid = change(lua, id, |d| {
        if d.epoch != epoch {
            return (false, false);
        }
        if parent && !allow_parent {
            d.selected = old_index;
        } else if d.selected >= 0 {
            let old_selected = d.row_mut(&path).is_some_and(|row| row.selected);
            for row in &mut d.rows {
                row.clear(false);
            }
            let on = !(d.toggle && old_selected);
            if let Some(row) = d.row_mut(&path) {
                row.selected = on;
            }
        }
        (true, false)
    })
    .unwrap_or(false);
    if valid && (!parent || allow_parent) {
        call_handler(lua, id, "OnSelChanged", vec![]);
    }
}

pub(super) fn methods(lua: &Lua, m: &Table) -> mlua::Result<()> {
    m.raw_set(
        "SetItemTrees",
        method(lua, |lua, id, a| {
            let Value::Table(table) = a.v(1) else {
                return none();
            };
            let rows = parse(lua, table, &mut Vec::new())?;
            change(lua, id, |d| {
                let dirty = d.rows != rows || d.selected != -1 || d.top != 0;
                d.tree_mode = !rows.is_empty();
                d.rows = rows;
                d.selected = -1;
                d.top = 0;
                d.hover = None;
                d.epoch = d.epoch.wrapping_add(1);
                ((), dirty)
            });
            none()
        })?,
    )?;
    m.raw_set(
        "IsItemOpened",
        method(lua, |lua, id, a| {
            let index = a.f(1) as i32;
            ret(
                lua,
                data(&ui(lua), id)
                    .and_then(|d| usize::try_from(index).ok().and_then(|i| d.rows.get(i)))
                    .is_some_and(|r| r.opened),
            )
        })?,
    )?;
    m.raw_set(
        "GetViewItemsInfo",
        method(lua, |lua, id, _| {
            let count = metrics(&mut ui(lua), id).map_or(0, |m| m.2.max(0) as usize);
            let values = data(&ui(lua), id)
                .map(|d| {
                    d.visible_rows()
                        .into_iter()
                        .skip(d.top.max(0) as usize)
                        .take(count)
                        .map(|(r, _)| (r.value, r.opened))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let out = lua.create_table()?;
            for (i, (value, opened)) in values.into_iter().enumerate() {
                let item = lua.create_table()?;
                item.raw_set("value", value)?;
                item.raw_set("opened", opened)?;
                out.raw_set(i + 1, item)?;
            }
            ret(lua, out)
        })?,
    )?;
    m.raw_set(
        "EnableSelectParent",
        method(lua, |lua, id, a| {
            change(lua, id, |d| {
                d.select_parent = a.b(1, true);
                ((), false)
            });
            none()
        })?,
    )?;
    m.raw_set(
        "SetTreeTypeIndent",
        method(lua, |lua, id, a| {
            change(lua, id, |d| {
                let next = a.b(1, false).then_some(a.f(2) as i32);
                let dirty = d.indent != next;
                d.indent = next;
                ((), dirty)
            });
            none()
        })?,
    )?;
    m.raw_set(
        "UseChildStyle",
        method(lua, |lua, id, a| {
            let on = a.b(1, false);
            let mut u = ui(lua);
            if let Some(w) = u.tree.widget_mut(id) {
                let copied = if on { w.text.clone() } else { None };
                let mut dirty = w.aux_style != copied;
                w.aux_style = copied;
                if let Some(d) = w.listbox.as_mut() {
                    dirty |= d.child_style != on;
                    d.child_style = on;
                }
                if dirty {
                    u.tree.repaint();
                }
            }
            none()
        })?,
    )?;
    for (name, kind) in [
        ("CreateOpenedImageDrawable", 0),
        ("CreateClosedImageDrawable", 1),
        ("CreateSeparatorImageDrawable", 2),
    ] {
        m.raw_set(
            name,
            method(lua, move |lua, id, a| {
                let image = {
                    let mut u = ui(lua);
                    let image = u.tree.create_drawable(
                        id,
                        crate::tree::DrawKind::Image,
                        "drawable:image",
                        crate::tree::DrawLayer::Artwork,
                    );
                    u.tree.drawable_mut(image).unwrap().texture = a.s(1);
                    // List-owned templates are painted once per applicable visible row.
                    u.tree.get_mut(image).shown = false;
                    image
                };
                change(lua, id, |d| {
                    match kind {
                        0 => d.opened_image = Some(image),
                        1 => d.closed_image = Some(image),
                        _ => d.separator_image = Some(image),
                    }
                    ((), true)
                });
                ret(lua, super::super::new_object_table(lua, image)?)
            })?,
        )?;
    }
    Ok(())
}

pub(super) fn image(
    u: &mut Ui,
    id: Id,
    center: (f32, f32),
    alpha: f32,
    scale: f32,
    out: &mut Vec<Quad>,
) {
    let Some(o) = u.tree.objects.get(id as usize).filter(|o| o.alive) else {
        return;
    };
    let Some(dr) = o.drawable() else { return };
    let Some(path) = dr.texture.clone() else {
        return;
    };
    let (width, height, coords, color, a) = (o.width, o.height, dr.coords, dr.color, o.alpha);
    let Some(resolved) = u.textures.resolve(&path) else {
        return;
    };
    let Some((tw, th)) = u.textures.size(&path) else {
        return;
    };
    let (tw, th) = (tw as f32, th as f32);
    let c = coords.unwrap_or([0.0, 0.0, tw, th]);
    out.push(Quad {
        tex: Tex::Image(resolved),
        pos: [
            (center.0 - width * 0.5) * scale,
            (center.1 - height * 0.5) * scale,
            (center.0 + width * 0.5) * scale,
            (center.1 + height * 0.5) * scale,
        ],
        uv: [c[0] / tw, c[1] / th, (c[0] + c[2]) / tw, (c[1] + c[3]) / th],
        color: [color[0], color[1], color[2], color[3] * a * alpha],
        corners: None,
        uvs: None,
        vcolors: None,
    });
}
