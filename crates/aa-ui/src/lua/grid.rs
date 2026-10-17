//! `grid` widgets: a table of rows and columns that places other widgets in its cells.
//!
//! The quest list (a grid of folders, each holding a grid of quests, each quest a two column grid)
//! and the quest notifier (title and objective rows) are built from it. The natives the scripts use:
//!
//! - `SetColCount(n)`, `SetColWidth(width, col)`, `SetRowCount(n)`, `SetRowHeight(height, row)`,
//!   `SetDefaultRowHeight(h)`; rows and columns count from 1.
//! - `SetItem(widget, row, col, makeIfNotExist, value, withoutExtent)` puts a widget in a cell:
//!   it sits at the cell's top left corner plus `SetItemInset(row, col, left, top, right, bottom)`;
//!   unless `withoutExtent` the grid sets the widget's size to the cell rectangle inside the insets
//!   (and keeps following the cell while the widget keeps that size: a folder that opens grows past
//!   the cell, and then the row grows with it). A row is as high as its `SetRowHeight`, or its
//!   tallest widget that has a size of its own.
//! - `GetAdjustRowHeight()` is the summed row height (`folder.alb` sizes the grid and the scroll
//!   range of its scroll window by it), `GetRowCount`, `GetRowHeight(row)`.
//! - `LineSelect(row)` / `ReleaseSelect()` mark a row; with `SetSelectedLine(true)` it is drawn as an
//!   image (`SetSelectedTexture` / `SetSelectedTexCoord` / `SetSelectedColor`, inset by `SetTexInset`),
//!   a click on a widget in a row selects the row and raises the grid's `OnSelChanged`. The row under
//!   the cursor gets the `SetCurrent...` image when `SetCurrentLine(true)`.
//! - `RemoveAllItems()` empties the grid (rows, columns and widgets; the look stays) and hides the
//!   widgets it held; `SetItem` shows the widget it is given (scripts pool and reuse them).
//!
//! Not drawn: `SetDrawLineType` (the tree line at the left of a folder's rows) and the zebra
//! background of `SetLineBackGround`.

use std::collections::HashMap;

use mlua::{Lua, Table};

use super::{call_handler, method, none, ret, ui, value_id, Ui};
use crate::tree::{Anchor, DrawKind, DrawLayer, Id, Point, Tree};

/// Height of a row that has no `SetRowHeight` and no widget with a size of its own.
const DEFAULT_ROW_HEIGHT: f32 = 10.0;

/// A widget in a cell.
#[derive(Clone, Debug)]
struct Cell {
    id: Id,
    row: usize,
    col: usize,
    /// The grid sizes the widget to the cell (`withoutExtent` false).
    fit: bool,
    /// Which sides of the widget follow the cell: those it had no size on when the grid first laid
    /// it out (`None` until then). A side stops following once the widget has another size than
    /// the grid last gave it (the widget took the size itself, a folder opened ...).
    follow: Option<(bool, bool)>,
    /// The size the grid last gave the widget.
    last: (f32, f32),
}

/// How a selected / hovered row is drawn.
#[derive(Clone, Debug)]
struct Look {
    line: bool,
    texture: Option<String>,
    coords: Option<[f32; 4]>,
    color: [f32; 4],
    /// The image drawable (made when first needed).
    img: Option<Id>,
}

impl Default for Look {
    fn default() -> Look {
        Look {
            line: false,
            texture: None,
            coords: None,
            color: [1.0; 4],
            img: None,
        }
    }
}

#[derive(Clone, Debug, Default)]
struct Grid {
    cols: usize,
    rows: usize,
    col_w: HashMap<usize, f32>,
    row_h: HashMap<usize, f32>,
    default_row_h: Option<f32>,
    cells: Vec<Cell>,
    insets: HashMap<(usize, usize), [f32; 4]>,
    selected: Option<usize>,
    sel: Look,
    cur: Look,
    tex_inset: [f32; 4],
}

#[derive(Default)]
struct Grids {
    map: HashMap<Id, Grid>,
    /// Tree generation of the last layout pass that changed nothing.
    settled: u64,
    /// Content version at the last pass ([`crate::tree::Tree::content`]).
    content: u64,
}

/// Where the rows and columns of a grid are (relative to the grid's top left).
struct Metrics {
    col_x: Vec<f32>,
    col_w: Vec<f32>,
    row_y: Vec<f32>,
    row_h: Vec<f32>,
}

impl Metrics {
    fn height(&self) -> f32 {
        self.row_h.iter().sum()
    }
}

fn with<R>(lua: &Lua, id: Id, f: impl FnOnce(&mut Grid) -> R) -> R {
    if lua.app_data_ref::<Grids>().is_none() {
        lua.set_app_data(Grids::default());
    }
    let mut gs = lua.app_data_mut::<Grids>().expect("aa-ui: grid state");
    f(gs.map.entry(id).or_default())
}

fn alive(tree: &Tree, id: Id) -> bool {
    tree.objects.get(id as usize).is_some_and(|o| o.alive)
}

fn inset_of(g: &Grid, c: &Cell) -> [f32; 4] {
    g.insets.get(&(c.row, c.col)).copied().unwrap_or_default()
}

/// Which sides of a fitted widget follow the cell right now.
fn follows(c: &Cell, w: f32, h: f32) -> (bool, bool) {
    match c.follow {
        Some((fw, fh)) => (fw && w == c.last.0, fh && h == c.last.1),
        None => (w == 0.0, h == 0.0),
    }
}

fn metrics(tree: &Tree, gid: Id, g: &Grid) -> Metrics {
    let rows = g.rows.max(g.cells.iter().map(|c| c.row).max().unwrap_or(0));
    let cols = g.cols.max(g.cells.iter().map(|c| c.col).max().unwrap_or(0));
    let grid_w = tree.rect(gid).w;
    let fixed: f32 = (1..=cols).filter_map(|c| g.col_w.get(&c)).sum();
    let unset = (1..=cols).filter(|c| !g.col_w.contains_key(c)).count();
    let share = if unset > 0 {
        (grid_w - fixed).max(0.0) / unset as f32
    } else {
        0.0
    };
    let mut col_x = Vec::with_capacity(cols);
    let mut col_w = Vec::with_capacity(cols);
    let mut x = 0.0;
    for c in 1..=cols {
        let w = g.col_w.get(&c).copied().unwrap_or(share);
        col_x.push(x);
        col_w.push(w);
        x += w;
    }
    let mut row_y = Vec::with_capacity(rows);
    let mut row_h = Vec::with_capacity(rows);
    let mut y = 0.0;
    for r in 1..=rows {
        // The tallest widget that kept a size of its own.
        let mut content: f32 = 0.0;
        let mut any = false;
        for c in g.cells.iter().filter(|c| c.row == r && alive(tree, c.id)) {
            any = true;
            let o = tree.get(c.id);
            let (_, fh) = follows(c, o.width, o.height);
            if !(c.fit && fh) {
                let ins = inset_of(g, c);
                content = content.max(o.height + ins[1] + ins[3]);
            }
        }
        let h = match g.row_h.get(&r) {
            Some(h) => h.max(content),
            None if any && content > 0.0 => content,
            None => g.default_row_h.unwrap_or(DEFAULT_ROW_HEIGHT),
        };
        row_y.push(y);
        row_h.push(h);
        y += h;
    }
    Metrics {
        col_x,
        col_w,
        row_y,
        row_h,
    }
}

/// The row of a grid that the widget `t` (or one of its ancestors) sits in.
fn row_of(u: &Ui, gid: Id, g: &Grid, mut t: Id) -> Option<usize> {
    while let Some(p) = u.tree.get(t).parent {
        if p == gid {
            return g.cells.iter().find(|c| c.id == t).map(|c| c.row);
        }
        t = p;
    }
    None
}

/// Place one grid's widgets and row images. True when something changed.
fn place(u: &mut Ui, gid: Id, g: &mut Grid) -> bool {
    let m = metrics(&u.tree, gid, g);
    let mut changed = false;
    for i in 0..g.cells.len() {
        let c = g.cells[i].clone();
        if !alive(&u.tree, c.id) || c.row == 0 || c.col == 0 {
            continue;
        }
        let (r, k) = (c.row - 1, c.col - 1);
        let ins = inset_of(g, &c);
        let want = vec![Anchor {
            point: Point::TopLeft,
            target: gid,
            rel: Point::TopLeft,
            x: m.col_x[k] + ins[0],
            y: m.row_y[r] + ins[1],
        }];
        if u.tree.get(c.id).anchors != want {
            u.tree.get_mut(c.id).anchors = want;
            changed = true;
        }
        if !c.fit {
            continue;
        }
        let (w, h) = {
            let o = u.tree.get(c.id);
            (o.width, o.height)
        };
        let (fw, fh) = follows(&c, w, h);
        let cell_w = (m.col_w[k] - ins[0] - ins[2]).max(0.0);
        let cell_h = (m.row_h[r] - ins[1] - ins[3]).max(0.0);
        let (nw, nh) = (if fw { cell_w } else { w }, if fh { cell_h } else { h });
        if (nw, nh) != (w, h) {
            let o = u.tree.get_mut(c.id);
            o.width = nw;
            o.height = nh;
            changed = true;
        }
        g.cells[i].follow = Some((fw, fh));
        g.cells[i].last = (nw, nh);
    }
    let grid_w = u.tree.rect(gid).w.max(m.col_w.iter().sum());
    let hovered = u.hover.and_then(|h| row_of(u, gid, g, h));
    for (is_sel, row) in [(true, g.selected), (false, hovered)] {
        let look = if is_sel { &mut g.sel } else { &mut g.cur };
        let row = row.filter(|r| look.line && *r >= 1 && *r <= m.row_y.len());
        changed |= row_image(u, gid, look, &m, grid_w, g.tex_inset, row);
    }
    changed
}

/// Show / hide / move the image of a selected or hovered row.
fn row_image(
    u: &mut Ui,
    gid: Id,
    look: &mut Look,
    m: &Metrics,
    grid_w: f32,
    inset: [f32; 4],
    row: Option<usize>,
) -> bool {
    let Some(row) = row else {
        if let Some(img) = look.img {
            if u.tree.get(img).shown {
                u.tree.get_mut(img).shown = false;
                return true;
            }
        }
        return false;
    };
    let img = match look.img {
        Some(i) => i,
        None => {
            let i = u
                .tree
                .create_drawable(gid, DrawKind::Image, "drawable:image", DrawLayer::Background);
            look.img = Some(i);
            i
        }
    };
    let (x, y) = (inset[0], m.row_y[row - 1] + inset[1]);
    let w = (grid_w - inset[0] - inset[2]).max(0.0);
    let h = (m.row_h[row - 1] - inset[1] - inset[3]).max(0.0);
    let want = vec![Anchor {
        point: Point::TopLeft,
        target: gid,
        rel: Point::TopLeft,
        x,
        y,
    }];
    let o = u.tree.get(img);
    let same = o.shown
        && o.anchors == want
        && (o.width, o.height) == (w, h)
        && o.drawable().is_some_and(|d| {
            d.texture == look.texture && d.coords == look.coords && d.color == look.color
        });
    if same {
        return false;
    }
    let o = u.tree.get_mut(img);
    o.shown = true;
    o.anchors = want;
    o.width = w;
    o.height = h;
    if let Some(d) = o.drawable_mut() {
        d.texture = look.texture.clone();
        d.coords = look.coords;
        d.color = look.color;
    }
    true
}

/// Lay every grid out (called each tick; cheap while the tree is unchanged).
pub(crate) fn update(lua: &Lua) {
    if lua.app_data_ref::<Grids>().is_none() {
        return;
    }
    {
        // Only writes of the same values since the last pass: the grids are where they were.
        let gs = lua.app_data_ref::<Grids>().expect("aa-ui: grid state");
        let u = ui(lua);
        if gs.settled != 0 && gs.content == u.tree.content() && !u.tree.changed() {
            return;
        }
    }
    for _ in 0..4 {
        let mut gs = lua.app_data_mut::<Grids>().expect("aa-ui: grid state");
        let mut u = ui(lua);
        if gs.settled == u.tree.generation {
            return;
        }
        u.tree.checkpoint();
        gs.content = u.tree.content();
        let mut ids: Vec<Id> = gs.map.keys().copied().collect();
        ids.sort_unstable();
        let mut changed = false;
        for id in ids {
            if !alive(&u.tree, id) {
                gs.map.remove(&id);
                continue;
            }
            if let Some(g) = gs.map.get_mut(&id) {
                changed |= place(&mut u, id, g);
            }
        }
        if !changed {
            gs.settled = u.tree.generation;
            return;
        }
    }
}

/// Mark a row (`None`: none); a changed selection raises `OnSelChanged` when `fire`.
fn select(lua: &Lua, gid: Id, row: Option<usize>, fire: bool) {
    let changed = with(lua, gid, |g| {
        let changed = g.selected != row;
        g.selected = row;
        changed
    });
    ui(lua).tree.touch();
    if changed && fire {
        call_handler(lua, gid, "OnSelChanged", vec![]);
    }
}

/// A left click landed on `target`: the rows of the grids above it get selected.
pub(crate) fn clicked(lua: &Lua, target: Id) {
    let mut rows: Vec<(Id, usize)> = Vec::new();
    {
        let Some(gs) = lua.app_data_ref::<Grids>() else {
            return;
        };
        let u = ui(lua);
        let mut t = target;
        while let Some(p) = u.tree.get(t).parent {
            if let Some(g) = gs.map.get(&p).filter(|g| g.sel.line) {
                if let Some(c) = g.cells.iter().find(|c| c.id == t) {
                    rows.push((p, c.row));
                }
            }
            t = p;
        }
    }
    for (g, row) in rows {
        select(lua, g, Some(row), true);
    }
}

pub(crate) fn methods(lua: &Lua, m: &Table) -> mlua::Result<()> {
    let def = |name: &str, f: mlua::Function| m.raw_set(name, f);
    // Counts and sizes.
    def(
        "SetColCount",
        method(lua, |lua, id, a| {
            let n = a.f(1).max(0.0) as usize;
            with(lua, id, |g| g.cols = n);
            ui(lua).tree.touch();
            none()
        })?,
    )?;
    def(
        "SetRowCount",
        method(lua, |lua, id, a| {
            let n = a.f(1).max(0.0) as usize;
            with(lua, id, |g| {
                g.rows = n;
                g.row_h.retain(|r, _| *r <= n);
                g.cells.retain(|c| c.row <= n);
                g.insets.retain(|(r, _), _| *r <= n);
                if g.selected.is_some_and(|r| r > n) {
                    g.selected = None;
                }
            });
            ui(lua).tree.touch();
            none()
        })?,
    )?;
    def(
        "SetColWidth",
        method(lua, |lua, id, a| {
            let (w, col) = (a.f(1), a.f(2) as usize);
            with(lua, id, |g| {
                g.col_w.insert(col, w);
                g.cols = g.cols.max(col);
            });
            ui(lua).tree.touch();
            none()
        })?,
    )?;
    def(
        "SetRowHeight",
        method(lua, |lua, id, a| {
            let (h, row) = (a.f(1), a.f(2) as usize);
            with(lua, id, |g| {
                g.row_h.insert(row, h);
            });
            ui(lua).tree.touch();
            none()
        })?,
    )?;
    def(
        "SetDefaultRowHeight",
        method(lua, |lua, id, a| {
            let h = a.f(1);
            with(lua, id, |g| g.default_row_h = Some(h));
            ui(lua).tree.touch();
            none()
        })?,
    )?;
    def(
        "GetRowCount",
        method(lua, |lua, id, _| {
            let u = ui(lua);
            let n = with(lua, id, |g| g.clone());
            ret(lua, metrics(&u.tree, id, &n).row_h.len() as i64)
        })?,
    )?;
    def(
        "GetColCount",
        method(lua, |lua, id, _| {
            let u = ui(lua);
            let n = with(lua, id, |g| g.clone());
            ret(lua, metrics(&u.tree, id, &n).col_w.len() as i64)
        })?,
    )?;
    def(
        "GetRowHeight",
        method(lua, |lua, id, a| {
            let u = ui(lua);
            let g = with(lua, id, |g| g.clone());
            let row = a.f(1) as usize;
            let h = metrics(&u.tree, id, &g)
                .row_h
                .get(row.wrapping_sub(1))
                .copied()
                .unwrap_or(0.0);
            ret(lua, h)
        })?,
    )?;
    def(
        "GetAdjustRowHeight",
        method(lua, |lua, id, _| {
            let u = ui(lua);
            let g = with(lua, id, |g| g.clone());
            ret(lua, metrics(&u.tree, id, &g).height())
        })?,
    )?;
    // Cells.
    def(
        "SetItem",
        method(lua, |lua, id, a| {
            let Some(item) = value_id(lua, &a.v(1)) else {
                return none();
            };
            let (row, col) = (a.f(2) as usize, a.f(3) as usize);
            if row == 0 || col == 0 {
                return none();
            }
            let fit = !a.b(6, false);
            with(lua, id, |g| {
                g.cells
                    .retain(|c| c.id != item && (c.row, c.col) != (row, col));
                g.cells.push(Cell {
                    id: item,
                    row,
                    col,
                    fit,
                    follow: None,
                    last: (0.0, 0.0),
                });
                g.rows = g.rows.max(row);
                g.cols = g.cols.max(col);
            });
            // A widget put in a cell is shown (`RemoveAllItems` hid the pooled ones).
            super::widget::show(lua, item, true, None);
            ui(lua).tree.touch();
            none()
        })?,
    )?;
    def(
        "SetItemInset",
        method(lua, |lua, id, a| {
            let (row, col) = (a.f(1) as usize, a.f(2) as usize);
            let ins = [a.f(3), a.f(4), a.f(5), a.f(6)];
            with(lua, id, |g| g.insets.insert((row, col), ins));
            ui(lua).tree.touch();
            none()
        })?,
    )?;
    def(
        "ClearItem",
        method(lua, |lua, id, a| {
            let (row, col) = (a.f(1) as usize, a.f(2) as usize);
            with(lua, id, |g| {
                g.cells
                    .retain(|c| !(c.row == row && (col == 0 || c.col == col)))
            });
            ui(lua).tree.touch();
            none()
        })?,
    )?;
    for name in ["RemoveAllItems", "RemoveItem"] {
        def(
            name,
            method(lua, |lua, id, a| {
                // `RemoveItem(widget)` takes one widget out; no argument empties the grid. The
                // widgets that left the grid are hidden (the scripts keep them in pools).
                let gone: Vec<Id> = match value_id(lua, &a.v(1)) {
                    Some(w) => with(lua, id, |g| {
                        g.cells.retain(|c| c.id != w);
                        vec![w]
                    }),
                    None => with(lua, id, |g| {
                        let ids = g.cells.iter().map(|c| c.id).collect();
                        *g = Grid {
                            sel: g.sel.clone(),
                            cur: g.cur.clone(),
                            tex_inset: g.tex_inset,
                            ..Grid::default()
                        };
                        ids
                    }),
                };
                for w in gone {
                    super::widget::show(lua, w, false, None);
                }
                ui(lua).tree.touch();
                none()
            })?,
        )?;
    }
    // Selection.
    def(
        "LineSelect",
        method(lua, |lua, id, a| {
            let row = a.f(1) as usize;
            select(lua, id, (row > 0).then_some(row), true);
            none()
        })?,
    )?;
    def(
        "ReleaseSelect",
        method(lua, |lua, id, _| {
            select(lua, id, None, false);
            none()
        })?,
    )?;
    def(
        "GetSelectedRowIndex",
        method(lua, |lua, id, _| {
            ret(lua, with(lua, id, |g| g.selected.unwrap_or(0)) as i64)
        })?,
    )?;
    // Look of the selected / the hovered row.
    for (which, selected) in [("Selected", true), ("Current", false)] {
        def(
            &format!("Set{which}Line"),
            method(lua, move |lua, id, a| {
                let on = a.b(1, false);
                with(lua, id, |g| {
                    let l = if selected { &mut g.sel } else { &mut g.cur };
                    l.line = on;
                });
                ui(lua).tree.touch();
                none()
            })?,
        )?;
        def(
            &format!("Set{which}Texture"),
            method(lua, move |lua, id, a| {
                let t = a.s(1);
                with(lua, id, |g| {
                    let l = if selected { &mut g.sel } else { &mut g.cur };
                    l.texture = t;
                });
                ui(lua).tree.touch();
                none()
            })?,
        )?;
        def(
            &format!("Set{which}TexCoord"),
            method(lua, move |lua, id, a| {
                let c = [a.f(1), a.f(2), a.f(3), a.f(4)];
                with(lua, id, |g| {
                    let l = if selected { &mut g.sel } else { &mut g.cur };
                    l.coords = Some(c);
                });
                ui(lua).tree.touch();
                none()
            })?,
        )?;
        def(
            &format!("Set{which}Color"),
            method(lua, move |lua, id, a| {
                let c = [a.f(1), a.f(2), a.f(3), a.opt_f(4).unwrap_or(1.0)];
                with(lua, id, |g| {
                    let l = if selected { &mut g.sel } else { &mut g.cur };
                    l.color = c;
                });
                ui(lua).tree.touch();
                none()
            })?,
        )?;
    }
    def(
        "SetTexInset",
        method(lua, |lua, id, a| {
            let i = [a.f(1), a.f(2), a.f(3), a.f(4)];
            with(lua, id, |g| g.tex_inset = i);
            ui(lua).tree.touch();
            none()
        })?,
    )?;
    Ok(())
}
