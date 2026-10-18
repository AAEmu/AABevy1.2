//! Native button/label UTF-32 prefix cache (39280850, 39281150, 39281210).

use mlua::Lua;

use super::{Ui, ui};
use crate::tree::Id;

pub(super) fn set(lua: &Lua, id: Id, enabled: bool) {
    let mut u = ui(lua);
    geometry(&mut u, id);
    let (w, h) = u.tree.size(id);
    if let Some(t) = u.tree.get_mut(id).text_mut() {
        t.auto_clip.enabled = enabled;
        // SetAutoClipChar only stores its flag. Remember today's geometry without
        // mistaking an earlier size assignment for a later native size callback.
        t.auto_clip.extent = [w, h];
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
    let (w, h) = u.tree.size(id);
    // Native skips its empty-string path, retaining the previous prefix count.
    let count = if t.text.is_empty() || t.text.starts_with('\0') {
        t.auto_clip.count
    } else {
        let available = (w - t.inset[0]) - t.inset[2];
        u.fonts
            .fitting_chars(&t.style.font, t.style.size, &t.text, available)
    };
    if t.auto_clip.count != count || t.auto_clip.extent != [w, h] {
        let t = u.tree.get_mut(id).text_mut().unwrap();
        t.auto_clip.count = count;
        t.auto_clip.extent = [w, h];
    }
}

pub(crate) fn geometry(u: &mut Ui, id: Id) {
    let Some(t) = u.tree.get(id).text().filter(|t| t.auto_clip.enabled) else {
        return;
    };
    let previous = t.auto_clip.extent;
    let (w, h) = u.tree.size(id);
    if previous != [w, h] {
        refresh(u, id);
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
