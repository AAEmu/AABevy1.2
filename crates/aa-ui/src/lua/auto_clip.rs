//! Native button UTF-32 prefix cache (39280850, 39281150, 39281210).

use mlua::Lua;

use super::{Ui, ui};
use crate::tree::{Id, TextBlock, Tree};

fn snapshot(u: &Ui, id: Id) -> ([f32; 4], f32) {
    let r = u.tree.rect(id);
    ([r.x, r.y, r.w, r.h], u.animations.widget_scale(&u.tree, id))
}

/// Full native button text width in local units (39275210(true)).
pub(super) fn full_width(u: &mut Ui, id: Id, t: &TextBlock) -> f64 {
    let scale = u.animations.widget_scale(&u.tree, id);
    let size = u.fonts.button_font_size(&t.style.font, t.style.size, scale);
    let physical = u.fonts.measure(&t.style.font, size, &t.text);
    // The native fdiv remains in the FPU while auto-resize adds its insets.
    physical as f64 / if scale > 0.0 { scale as f64 } else { 0.0 }
}

pub(super) fn set(lua: &Lua, id: Id, enabled: bool) {
    let mut u = ui(lua);
    geometry(&mut u, id);
    let (rect, scale) = snapshot(&u, id);
    if let Some(t) = u.tree.get_mut(id).text_mut() {
        t.auto_clip.enabled = enabled;
        // SetAutoClipChar only stores its flag. Remember today's geometry without
        // mistaking an earlier size assignment for a later native size callback.
        t.auto_clip.rect = rect;
        t.auto_clip.scale = scale;
    }
    if enabled {
        u.auto_clip_widgets.insert(id);
    } else {
        u.auto_clip_widgets.remove(&id);
    }
}

pub(super) fn refresh(u: &mut Ui, id: Id) {
    let Some(t) = u
        .tree
        .get(id)
        .text()
        .filter(|t| t.auto_clip.enabled)
        .cloned()
    else {
        return;
    };
    let (rect, scale) = snapshot(u, id);
    // Native skips its empty-string path, retaining the previous prefix count.
    let count = if t.text.is_empty() || t.text.starts_with('\0') {
        t.auto_clip.count
    } else {
        // Native effective extent +74, then RAW insets +564/+56c. The drawing
        // path scales those insets; fitting deliberately does not.
        let available = (rect[2] * scale - t.inset[0]) - t.inset[2];
        let size = u.fonts.button_font_size(&t.style.font, t.style.size, scale);
        u.fonts
            .fitting_chars(&t.style.font, size, &t.text, available)
    };
    if t.auto_clip.count != count || t.auto_clip.rect != rect || t.auto_clip.scale != scale {
        let t = u.tree.get_mut(id).text_mut().unwrap();
        t.auto_clip.count = count;
        t.auto_clip.rect = rect;
        t.auto_clip.scale = scale;
    }
}

pub(crate) fn geometry(u: &mut Ui, id: Id) {
    let Some(t) = u.tree.get(id).text().filter(|t| t.auto_clip.enabled) else {
        return;
    };
    let previous = (t.auto_clip.rect, t.auto_clip.scale);
    if previous != snapshot(u, id) {
        refresh(u, id);
    }
}

fn depends(tree: &Tree, id: Id, target: Id, seen: &mut Vec<Id>) -> bool {
    if id == target {
        return true;
    }
    if seen.contains(&id) {
        return false;
    }
    seen.push(id);
    let o = tree.get(id);
    if o.anchors.is_empty() && o.unanchored_origin.is_none() {
        return o.parent.is_some_and(|p| depends(tree, p, target, seen));
    }
    o.anchors
        .iter()
        .any(|a| depends(tree, a.target, target, seen))
}

/// Native size/anchor callbacks also reach anchor dependents (+44). Scale
/// changes additionally visit every ordinary child (+c8), including hidden ones.
/// Explicit setters call this even for identical values; inset-only writes do not.
pub(super) fn changed(u: &mut Ui, target: Id, scale: bool) {
    let enabled: Vec<Id> = u.auto_clip_widgets.iter().copied().collect();
    for id in enabled {
        if !u.tree.get(id).alive {
            u.auto_clip_widgets.remove(&id);
        } else if (scale && u.tree.is_descendant(id, target))
            || depends(&u.tree, id, target, &mut Vec::new())
        {
            refresh(u, id);
        }
    }
}

pub(crate) fn shown_text(text: &str, count: usize) -> &str {
    let text = text.split('\0').next().unwrap_or_default();
    let end = text
        .char_indices()
        .nth(count)
        .map_or(text.len(), |(i, _)| i);
    &text[..end]
}
