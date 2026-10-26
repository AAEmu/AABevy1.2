//! Native positive-page presses on a Slider receiver; other input paths stay with the caller.
use crate::{lua, tree::Id};
use mlua::{Lua, Value};

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Press {
    Track,
    Thumb,
}

/// `392c2a80 -> 392c2830`: page only on the track, with no continuing slider drag.
/// A direct Slider hit on an inclusive thumb edge is a thumb press, not a page.
pub(super) fn press(lua: &Lua, id: Id, at: (f32, f32)) -> Option<Press> {
    {
        let mut u = lua::ui(lua);
        let s = u.tree.widget(id)?.slider.clone()?;
        if s.page_step <= 0.0 || s.page_step.is_nan() {
            return None;
        }
        // Script mouse-down can have changed these after the original hit test.
        if !u.tree.visible(id) || !u.tree.widget(id).is_some_and(|w| w.enabled) {
            return Some(Press::Track);
        }
        let Some(thumb) = s
            .thumb
            .filter(|&t| u.tree.get(t).alive && u.tree.widget(t).is_some_and(|w| w.enabled))
        else {
            return Some(Press::Track);
        };
        // Input is in root UI units. Native +6c/+74 and the pointer are physical:
        // convert each once, preserving the established effective/frozen transforms.
        let rect = u.animations.widget_rect(&u.tree, thumb);
        let scale = u.ui_scale;
        let (x, y, w, h) = (
            rect.x * scale,
            rect.y * scale,
            rect.w * scale,
            rect.h * scale,
        );
        let (cx, cy) = (at.0 * scale, at.1 * scale);
        // Native392757c0 admits all four edges, unlike generic Rect::contains.
        if x <= cx && y <= cy && cx <= x + w && cy <= y + h {
            return Some(Press::Thumb);
        }
        let (pos, start, end) = if s.vertical {
            (cy, y, y + h)
        } else {
            (cx, x, x + w)
        };
        let value = if pos < start {
            s.value - s.page_step
        } else if pos > end {
            s.value + s.page_step
        } else {
            return Some(Press::Track);
        };
        let value = value.clamp(s.min, s.max);
        u.tree
            .widget_mut(id)
            .unwrap()
            .slider
            .as_mut()
            .unwrap()
            .value = value;
        // Native392c24b0 notifies even when clamping leaves the value unchanged.
        // Do not broaden the generic setter's notification/argument contract.
        u.pending.push((
            id,
            "OnSliderChanged".into(),
            vec![Value::Number(value as f64)],
        ));
    }
    lua::chat::slider_changed(lua, id);
    lua::flush(lua);
    Some(Press::Track)
}
