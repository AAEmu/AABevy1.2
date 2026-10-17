//! Shared literal edit layout for painting, mouse picking and keyboard navigation.

use super::{EditData, display_text};
use crate::{
    lua::Ui,
    text::{LaidOutText, TextCaret},
    tree::{Id, Rect, TextBlock, WidgetType},
};

pub(super) struct Geometry {
    pub layout: LaidOutText,
    pub inner: Rect,
    pub origin: [f32; 2],
    pub scale: f32,
    pub step: f32,
    pub line_height: f32,
}

impl Geometry {
    pub fn new(u: &mut Ui, text: &TextBlock, edit: &EditData, rect: Rect, multiline: bool) -> Self {
        let scale = u.ui_scale;
        let inner = Rect {
            x: rect.x + text.inset[0],
            y: rect.y + text.inset[1],
            w: (rect.w - text.inset[0] - text.inset[2]).max(0.0),
            h: (rect.h - text.inset[1] - text.inset[3]).max(0.0),
        };
        let width = (multiline && inner.w > 0.0).then_some(inner.w * scale);
        let layout = u.fonts.layout_edit(
            &text.style.font,
            text.style.size * scale,
            &text.text,
            width,
            text.line_space * scale,
        );
        let line_height = layout.line_height / scale;
        let step = line_height + text.line_space;
        let fx = if layout.width / scale > inner.w {
            0.0
        } else {
            text.style.align.fx()
        };
        let fy = if multiline {
            0.0
        } else {
            text.style.align.fy()
        };
        let origin = [
            inner.x + (inner.w - layout.width / scale) * fx - edit.scroll[0],
            inner.y + (inner.h - layout.height / scale) * fy - edit.scroll[1],
        ];
        Self {
            layout,
            inner,
            origin,
            scale,
            step,
            line_height,
        }
    }

    pub fn caret(&self, index: usize) -> TextCaret {
        self.layout
            .carets
            .get(index)
            .copied()
            .or_else(|| self.layout.carets.last().copied())
            .unwrap_or_default()
    }

    pub fn caret_on(&self, index: usize, line: Option<usize>) -> TextCaret {
        let p = self.caret(index);
        if let Some(line) = line
            && line != p.line
            && self
                .layout
                .line_ranges
                .get(line)
                .is_some_and(|&(_, end)| end == index)
        {
            TextCaret {
                x: self.layout.line_widths.get(line).copied().unwrap_or(0.0),
                line,
            }
        } else {
            p
        }
    }

    pub fn point_on(&self, index: usize, line: Option<usize>) -> [f32; 2] {
        let p = self.caret_on(index, line);
        [
            self.origin[0] + p.x / self.scale,
            self.origin[1] + p.line as f32 * self.step,
        ]
    }

    pub fn picked_line(&self, y: f32) -> usize {
        let max_line = self.layout.carets.last().map_or(0, |p| p.line);
        ((y - self.origin[1]) / self.step.max(1.0))
            .floor()
            .clamp(0.0, max_line as f32) as usize
    }

    pub fn pick(&self, x: f32, y: f32) -> usize {
        let line = self.picked_line(y);
        self.on_line(line, (x - self.origin[0]) * self.scale)
    }

    pub fn on_line(&self, line: usize, x: f32) -> usize {
        let (mut at, distance) = self
            .layout
            .carets
            .iter()
            .enumerate()
            .filter(|(_, p)| p.line == line)
            .min_by(|(_, a), (_, b)| (a.x - x).abs().total_cmp(&(b.x - x).abs()))
            .map_or((0, f32::MAX), |(i, p)| (i, (p.x - x).abs()));
        if let Some(&(_, end)) = self.layout.line_ranges.get(line) {
            let d = (self.layout.line_widths[line] - x).abs();
            if d < distance {
                at = end;
            }
        }
        at
    }

    pub fn line_end(&self, index: usize, end: bool, line: Option<usize>) -> usize {
        let line = self.caret_on(index, line).line;
        self.layout
            .line_ranges
            .get(line)
            .map_or(index, |&(a, b)| if end { b } else { a })
    }
}

pub(super) fn widget(u: &mut Ui, id: Id) -> Option<Geometry> {
    let w = u.tree.widget(id)?;
    let multiline = w.ty == WidgetType::EditBoxMultiline;
    let mut text = w.text.clone()?;
    let edit = w.edit.clone().unwrap_or_default();
    text.text = display_text(&text.text, Some(&edit));
    if !edit.prefix.is_empty() {
        let prefix = w.aux_style.clone().unwrap_or_default();
        text.inset[0] += edit.prefix_inset[0]
            + u.fonts
                .measure(&prefix.style.font, prefix.style.size, &edit.prefix)
            + edit.prefix_inset[2];
    }
    let rect = u.tree.rect(id);
    Some(Geometry::new(u, &text, &edit, rect, multiline))
}

/// Pointer coordinates are in the transformed widget's UI space; editing uses its authored
/// rectangle, so undo inherited animated/static scale before looking up character boundaries.
pub(super) fn pointer(u: &Ui, id: Id, p: (f32, f32)) -> [f32; 2] {
    let base = u.tree.rect(id);
    let view = u.animations.widget_rect(&u.tree, id);
    [
        base.x + (p.0 - view.x) * base.w / view.w.max(0.001),
        base.y + (p.1 - view.y) * base.h / view.h.max(0.001),
    ]
}

pub(super) fn ensure_caret(u: &mut Ui, id: Id) {
    let Some(g) = widget(u, id) else { return };
    let e = u
        .tree
        .widget(id)
        .and_then(|w| w.edit.as_ref())
        .cloned()
        .unwrap_or_default();
    let point = g.point_on(e.cursor, e.visual_line);
    let mut scroll = e.scroll;
    for i in 0..2 {
        let (lo, extent) = if i == 0 {
            (g.inner.x, g.inner.w)
        } else {
            (g.inner.y, g.inner.h)
        };
        let size = if i == 0 {
            u.ui_scale.recip()
        } else {
            g.line_height
        };
        if point[i] < lo {
            scroll[i] -= lo - point[i];
        } else if point[i] + size > lo + extent {
            scroll[i] += point[i] + size - lo - extent;
        }
        scroll[i] = scroll[i].max(0.0);
    }
    if scroll != e.scroll
        && let Some(edit) = u.tree.widget_mut(id).and_then(|w| w.edit.as_mut())
    {
        edit.scroll = scroll;
    }
}
