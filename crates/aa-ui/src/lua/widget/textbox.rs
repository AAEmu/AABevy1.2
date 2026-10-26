//! Ordinary TextBox rows use their retained physical font, shared by queries and paint.

use super::super::Ui;
use crate::{
    text::LaidOutText,
    tree::{Id, TextBlock, Tree, WidgetType},
};

pub(crate) fn ordinary(u: &Ui, id: Id, text: &TextBlock) -> bool {
    u.tree
        .widget(id)
        .is_some_and(|w| w.ty == WidgetType::TextBox)
        && !u.fonts.is_image_font(&text.style.font)
}

pub(crate) fn layout(u: &mut Ui, id: Id, text: &TextBlock) -> (LaidOutText, f32) {
    let scale = u.animations.widget_scale(&u.tree, id);
    // TextBox392cddec -> FontStyle39275150 selects the physical face first.
    let size = u
        .fonts
        .button_font_size(&text.style.font, text.style.size, scale);
    let width = u.tree.rect(id).w * scale - text.inset[0] * scale - text.inset[2] * scale;
    //392cdf79..b6 ->39273b20: retain the full alignment extent, but reserve
    // one physical pixel in the rich-text wrapper's breaking limit.
    let max_width = text.wrap.then_some(width - 1.0);
    let layout = u.fonts.layout_textbox(
        &text.style.font,
        size,
        &text.text,
        max_width,
        text.line_space * scale,
        false,
    );
    (layout, scale)
}

/// Native392cddd0 ->392cdca0 refits an auto-sized TextBox after applying scale.
/// Enroll at the existing auto-size setter/content paths, never in a size getter.
pub(crate) fn auto_size(u: &mut Ui, id: Id, text: &TextBlock) {
    let (layout, scale) = layout(u, id, text);
    if !text.text.is_empty() && !layout.line_widths.is_empty() {
        u.auto_textbox_scales.insert(id, scale);
    } else {
        u.auto_textbox_scales.remove(&id);
    }
    let height = layout.height / scale + text.inset[1] + text.inset[3];
    if u.tree.get(id).height != height {
        u.tree.get_mut(id).height = height;
        refresh_geometry(&mut u.tree, id);
    }
}

// A nested height change also affects stretched dependents already visited by
// the outer dispatch. Reuse their existing geometry path, without events or
// promoting unresolved targets. The dependency vector is stable: no Lua runs.
fn refresh_geometry(tree: &mut Tree, id: Id) {
    if !tree.get(id).alive || !tree.begin_anchor_update(id) {
        return;
    }
    let mut index = 0;
    while let Some(dependent) = tree.anchor_dependent(id, index) {
        refresh_geometry(tree, dependent);
        index += 1;
    }
    tree.end_anchor_update();
}

/// Native completion and callback entry are synchronous boundaries for scale writes.
/// Check only enrolled TextBoxes; frozen effective scales and manual heights are untouched.
/// This pass calls no Lua, and all liveness/auto-size checks use the current tree.
pub(crate) fn refit_scales(lua: &mlua::Lua) {
    let mut u = super::super::ui(lua);
    let previous: Vec<_> = u
        .auto_textbox_scales
        .iter()
        .map(|(&id, &scale)| (id, scale))
        .collect();
    for (id, before) in previous {
        let text = if u.tree.get(id).alive {
            u.tree
                .get(id)
                .text()
                .filter(|t| t.auto_resize && !t.text.is_empty())
        } else {
            None
        };
        let Some(text) = text.filter(|t| ordinary(&u, id, t)) else {
            u.auto_textbox_scales.remove(&id);
            continue;
        };
        if u.animations.widget_scale(&u.tree, id) != before {
            let text = text.clone();
            auto_size(&mut u, id, &text);
        }
    }
}
