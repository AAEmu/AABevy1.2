//! Pointer placement uses the same shaped runs and transforms as the visible form.
use super::*;
use bevy::text::ComputedTextBlock;

pub(super) type Runs<'w, 's> = Query<
    'w,
    's,
    (
        &'static FieldText,
        &'static Text,
        &'static ComputedTextBlock,
        &'static ComputedNode,
        &'static UiGlobalTransform,
    ),
>;

#[derive(Clone, Copy)]
pub(super) struct Drag {
    field: usize,
    start: Vec2,
    extend: bool,
    pending: bool,
    end: Vec2,
    released: bool,
    revision: u64,
}

/// Do not pair new source text with a previous frame's layout. A pointer press
/// may wait one frame for shaping after an edit or a form rebuild.
pub(super) fn index_at(field: &Field, id: usize, point: Vec2, runs: &Runs) -> Option<usize> {
    let shown = field.shown(id == PASSWORD);
    let mut nearest: Option<(f32, usize)> = None;
    for (part, expected) in shown.iter().enumerate() {
        let (_, text, block, node, transform) = runs.iter().find(|(tag, ..)| tag.0 == id && tag.1 == part)?;
        if text.0 != *expected || (!expected.is_empty() && block.needs_rerender(false, false)) {
            return None;
        }
        if expected.is_empty() {
            continue;
        }
        let inverse = transform.try_inverse()?;
        let local = inverse.transform_point2(point) - node.content_box().min;
        let width = node.content_box().width();
        let distance = (local.x - local.x.clamp(0., width)).abs();
        let cursor = parley::editing::Cursor::from_point(block.buffer(), local.x, block.buffer().height() * 0.5);
        let index = field.source_index(part, expected, cursor.index())?;
        if nearest.is_none_or(|(best, _)| distance < best) {
            nearest = Some((distance, index));
        }
    }
    Some(nearest.map_or(0, |(_, index)| index))
}

pub(super) fn reveal(current: f32, prefix: f32, width: f32, available: f32, total: f32) -> f32 {
    let current = current.clamp((available - total).min(0.), 0.);
    if prefix + current < 0. {
        -prefix
    } else if prefix + width + current > available {
        (available - prefix - width).min(0.)
    } else {
        current.min(0.)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clicking_inside_a_scrolled_view_preserves_the_view_and_crossing_an_edge_reveals_it() {
        assert_eq!(reveal(-500., 600., 4., 300., 1000.), -500.);
        assert_eq!(reveal(-500., 499., 4., 300., 1000.), -499.);
        assert_eq!(reveal(-500., 810., 4., 300., 1000.), -514.);
        assert_eq!(reveal(-500., 0., 4., 300., 1000.), 0.);
        assert_eq!(reveal(0., 1., 2., 300., 100.), 0.);
        assert_eq!(reveal(-500., 40., 4., 300., 44.), 0.);
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn mouse(
    window: Query<&Window, With<PrimaryWindow>>,
    mut buttons: ResMut<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    fields: Query<(&FieldBox, &ComputedNode, &UiGlobalTransform)>,
    runs: Runs,
    connect: Query<(&ComputedNode, &UiGlobalTransform), With<ConnectButton>>,
    exit: Query<(&ComputedNode, &UiGlobalTransform), With<ExitButton>>,
    mut form: ResMut<LoginForm>,
    mut drag: Local<Option<Drag>>,
    mut acts: MessageWriter<Act>,
    sound: (Option<Res<Engine>>, Option<Res<Packs>>),
) {
    let Ok(window) = window.single() else { return };
    if !window.focused {
        *drag = None;
        // Winit focus loss clears keyboard state, not held mouse buttons.
        buttons.release(MouseButton::Left);
        form.hover = Hover::None;
        return;
    }
    let point = window.cursor_position().map(|point| point * window.resolution.scale_factor());
    let hit = |node: &ComputedNode, transform: &UiGlobalTransform| point.is_some_and(|p| node.contains_point(*transform, p));
    let over_connect = connect.iter().any(|(n, t)| hit(n, t));
    let over_exit = exit.iter().any(|(n, t)| hit(n, t));
    let hover = if over_connect {
        Hover::Connect
    } else if over_exit {
        Hover::Exit
    } else {
        Hover::None
    };
    if hover != form.hover {
        if hover != Hover::None {
            play(&sound.0, &sound.1, "default_over");
        }
        form.hover = hover;
    }
    if form.busy() || form.age < 0.2 {
        *drag = None;
    }
    if buttons.just_pressed(MouseButton::Left) && form.age >= 0.2 {
        *drag = None;
        if over_exit {
            acts.write(Act::Exit);
        } else if over_connect {
            acts.write(Act::Submit);
        } else if !form.busy()
            && let Some((FieldBox(field), ..)) = fields.iter().find(|(_, n, t)| hit(n, t))
            && let Some(point) = point
        {
            form.focus = *field;
            form.blink = 0.;
            *drag = Some(Drag {
                field: *field,
                start: point,
                extend: keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight),
                pending: true,
                end: point,
                released: !buttons.pressed(MouseButton::Left),
                revision: form.fields[*field].revision(),
            });
        }
    }
    let Some(active) = drag.as_mut() else { return };
    if active.field != form.focus || active.revision != form.fields[active.field].revision() {
        *drag = None;
        return;
    }
    if !active.released {
        if let Some(point) = point {
            active.end = point;
        }
        active.released = !buttons.pressed(MouseButton::Left);
    }
    if active.pending {
        let Some(index) = index_at(&form.fields[active.field], active.field, active.start, &runs) else {
            return;
        };
        form.fields[active.field].point(index, active.extend);
        active.revision = form.fields[active.field].revision();
        active.pending = false;
        if active.released && active.start == active.end {
            *drag = None;
        }
        // The selection split changed. Wait for its actual new layout before
        // interpreting a drag against those runs.
        return;
    }
    if let Some(index) = index_at(&form.fields[active.field], active.field, active.end, &runs) {
        form.fields[active.field].point(index, true);
        active.revision = form.fields[active.field].revision();
        form.blink = 0.;
        if active.released {
            *drag = None;
        }
    }
}
