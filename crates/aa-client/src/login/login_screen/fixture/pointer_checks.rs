//! Actual headless Bevy layout and pointer delivery, with synthetic form text.
use super::*;
use parley::{editing::Cursor, layout::Affinity};

#[derive(Resource, Default)]
struct Check {
    phase: u8,
    offset: f32,
}

fn send_key(events: &mut Messages<KeyboardInput>, code: KeyCode, key: Key, state: ButtonState, text: Option<&str>) {
    events.write(KeyboardInput {
        key_code: code,
        logical_key: key,
        state,
        text: text.map(Into::into),
        repeat: false,
        window: Entity::PLACEHOLDER,
    });
}

fn select_all(events: &mut Messages<KeyboardInput>) {
    send_key(events, KeyCode::ControlLeft, Key::Control, ButtonState::Pressed, None);
    send_key(events, KeyCode::KeyA, Key::Character("a".into()), ButtonState::Pressed, None);
    send_key(events, KeyCode::KeyA, Key::Character("a".into()), ButtonState::Released, None);
    send_key(events, KeyCode::ControlLeft, Key::Control, ButtonState::Released, None);
}

pub(super) fn install(app: &mut App) {
    app.init_resource::<Check>()
        .add_systems(PreUpdate, drive.after(bevy::input::InputSystems))
        .add_systems(Last, deadline);
}

// Independent forward mapping: a known source boundary -> shaped cursor ->
// actual text entity transform. Production performs the reverse hit test.
fn position(field: &Field, id: usize, byte: usize, runs: &pointer::Runs) -> Option<Vec2> {
    let shown = field.shown(id == PASSWORD);
    for (part, range) in field.runs().iter().enumerate() {
        if range.is_empty() || !(range.start..=range.end).contains(&byte) {
            continue;
        }
        let (_, text, block, node, transform) = runs.iter().find(|(tag, ..)| tag.0 == id && tag.1 == part)?;
        if text.0 != shown[part] || block.needs_rerender(false, false) || block.buffer().height() <= 0. {
            return None;
        }
        let visible = if id == PASSWORD {
            field.text.get(range.start..byte)?.chars().count() * '•'.len_utf8()
        } else {
            byte - range.start
        };
        let rect = Cursor::from_byte_index(block.buffer(), visible, Affinity::Downstream).geometry(block.buffer(), 0.);
        let local = node.content_box().min + Vec2::new(rect.x0 as f32 + 0.05, ((rect.y0 + rect.y1) * 0.5) as f32);
        return Some(transform.affine().transform_point2(local));
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn drive(
    time: Res<Time>,
    mut check: ResMut<Check>,
    mut form: ResMut<LoginForm>,
    mut window: Query<&mut Window, With<PrimaryWindow>>,
    mut buttons: ResMut<ButtonInput<MouseButton>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    runs: pointer::Runs,
    rows: Query<(&FieldRow, &Node)>,
    mut acts: ResMut<Messages<Act>>,
    mut keyboard: ResMut<Messages<KeyboardInput>>,
    fields: Query<(&FieldBox, &ComputedNode, &UiGlobalTransform)>,
) {
    if time.elapsed_secs() < 0.3 || check.phase == 35 {
        return;
    }
    let Ok(mut window) = window.single_mut() else { return };
    // Synthetic pointer delivery is independent of OS foreground ownership.
    // Phase21 explicitly exercises focus loss without a mouse-up event.
    window.focused = check.phase != 21;
    let mut point = |p: Vec2| {
        let factor = window.resolution.scale_factor();
        window.set_cursor_position(Some(p / factor));
    };
    let offset = |id| {
        rows.iter()
            .find(|(tag, _)| tag.0 == id)
            .and_then(|(_, node)| if let Val::Px(x) = node.left { Some(x) } else { None })
    };
    assert!(acts.drain().next().is_none(), "pointer editing must not submit or exit");
    match check.phase {
        0 => {
            form.fields[USER].set("aéBCDEf");
            form.fields[PASSWORD].set("éBCδf");
            check.phase = 1;
        }
        1 => {
            let Some(p) = position(&form.fields[USER], USER, 3, &runs) else {
                return;
            };
            point(p);
            buttons.press(MouseButton::Left);
            check.phase = 2;
        }
        2 => {
            assert_eq!(form.fields[USER].cursor, 3, "Unicode click boundary");
            buttons.release(MouseButton::Left);
            check.phase = 3;
        }
        3 => {
            let Some(p) = position(&form.fields[USER], USER, 1, &runs) else {
                return;
            };
            point(p);
            buttons.press(MouseButton::Left);
            check.phase = 4;
        }
        4 => {
            assert_eq!(form.fields[USER].cursor, 1);
            let Some(p) = position(&form.fields[USER], USER, 6, &runs) else {
                return;
            };
            point(p);
            check.phase = 5;
        }
        5 => {
            assert_eq!(form.fields[USER].selected(), "éBCD", "Unicode drag range");
            buttons.release(MouseButton::Left);
            check.phase = 6;
        }
        6 => {
            let Some(p) = position(&form.fields[PASSWORD], PASSWORD, 2, &runs) else {
                return;
            };
            point(p);
            buttons.press(MouseButton::Left);
            check.phase = 7;
        }
        7 => {
            assert_eq!(form.focus, PASSWORD);
            assert_eq!(form.fields[PASSWORD].cursor, 2, "masked glyph -> source boundary");
            assert!(
                runs.iter()
                    .filter(|(tag, ..)| tag.0 == PASSWORD)
                    .all(|(_, text, ..)| text.chars().all(|c| c == '•'))
            );
            buttons.release(MouseButton::Left);
            check.phase = 8;
        }
        8 => {
            let Some(p) = position(&form.fields[USER], USER, 8, &runs) else {
                return;
            };
            keys.press(KeyCode::ShiftRight);
            point(p);
            buttons.press(MouseButton::Left);
            check.phase = 9;
        }
        9 => {
            assert_eq!(form.fields[USER].selected(), "éBCDEf", "Shift-click retains anchor");
            keys.release(KeyCode::ShiftRight);
            buttons.release(MouseButton::Left);
            form.fields[SERVER].set(&"server01.".repeat(9));
            form.focus = SERVER;
            check.phase = 10;
        }
        10 => {
            let Some(p) = position(&form.fields[SERVER], SERVER, 70, &runs) else {
                return;
            };
            let Some(x) = offset(SERVER) else { return };
            if x >= 0. {
                return;
            }
            check.offset = x;
            point(p);
            buttons.press(MouseButton::Left);
            check.phase = 11;
        }
        11 => {
            assert_eq!(form.fields[SERVER].cursor, 70);
            assert_eq!(offset(SERVER), Some(check.offset), "visible click must preserve scroll");
            buttons.release(MouseButton::Left);
            check.phase = 12;
        }
        12 => {
            let Some(p) = position(&form.fields[SERVER], SERVER, 70, &runs) else {
                return;
            };
            point(p);
            buttons.press(MouseButton::Left);
            check.phase = 13;
        }
        13 => {
            let Some(p) = position(&form.fields[SERVER], SERVER, 0, &runs) else {
                return;
            };
            point(p);
            check.phase = 14;
        }
        14 => {
            assert_eq!(form.fields[SERVER].cursor, 0, "drag retains its field outside its box");
            assert_eq!(form.fields[SERVER].selected().len(), 70);
            // The split text shapes in PostUpdate; its new widths reach the
            // following scroll pass. Verify the settled view, not old metrics.
            if offset(SERVER) != Some(0.) {
                return;
            }
            buttons.release(MouseButton::Left);
            check.phase = 15;
        }
        15 => {
            form.focus = USER;
            form.fields[USER].set("aéBCDEf");
            check.phase = 16;
        }
        16 => {
            let Some(p) = position(&form.fields[USER], USER, 0, &runs) else {
                return;
            };
            // Stale split geometry on the press frame, without changing text.
            form.fields[USER].select_all();
            point(p - Vec2::X * 2.);
            buttons.press(MouseButton::Left);
            check.phase = 17;
        }
        17 => {
            buttons.release(MouseButton::Left);
            check.phase = 18;
        }
        18 => {
            assert_eq!(form.fields[USER].cursor, 0, "released pending click must resolve");
            let Some(p) = position(&form.fields[USER], USER, 1, &runs) else {
                return;
            };
            point(p);
            buttons.press(MouseButton::Left);
            check.phase = 19;
        }
        19 => {
            assert_eq!(form.fields[USER].cursor, 1);
            let Some(p) = position(&form.fields[USER], USER, 6, &runs) else {
                return;
            };
            point(p);
            buttons.release(MouseButton::Left);
            check.phase = 20;
        }
        20 => {
            assert_eq!(form.fields[USER].selected(), "éBCD", "release must apply its final endpoint");
            let Some(p) = position(&form.fields[USER], USER, 1, &runs) else {
                return;
            };
            point(p);
            buttons.press(MouseButton::Left);
            check.phase = 21;
        }
        21 => {
            assert_eq!(form.fields[USER].cursor, 1);
            check.phase = 22;
        }
        22 => {
            let Some(p) = position(&form.fields[USER], USER, 8, &runs) else {
                return;
            };
            point(p);
            check.phase = 23;
        }
        23 => {
            assert_eq!(form.fields[USER].cursor, 1, "focus loss must cancel drag without mouse-up");
            assert!(form.fields[USER].selected().is_empty());
            assert!(!buttons.pressed(MouseButton::Left));
            check.phase = 24;
        }
        24 => {
            form.focus = SERVER;
            form.fields[SERVER].set(&"server01.".repeat(9));
            check.phase = 25;
        }
        25 => {
            if offset(SERVER).is_none_or(|x| x >= 0.)
                || position(&form.fields[SERVER], SERVER, form.fields[SERVER].text.len(), &runs).is_none()
            {
                return;
            }
            select_all(&mut keyboard);
            send_key(
                &mut keyboard,
                KeyCode::KeyT,
                Key::Character("tiny".into()),
                ButtonState::Pressed,
                Some("tiny"),
            );
            check.phase = 26;
        }
        26 => {
            assert_eq!(form.fields[SERVER].text, "tiny");
            if offset(SERVER) != Some(0.) {
                return;
            }
            let Some(p) = position(&form.fields[SERVER], SERVER, 0, &runs) else {
                return;
            };
            assert!(
                fields
                    .iter()
                    .find(|(tag, ..)| tag.0 == SERVER)
                    .is_some_and(|(_, node, transform)| node.contains_point(*transform, p)),
                "short replacement must be visible"
            );
            check.phase = 27;
        }
        27 => {
            form.focus = USER;
            form.fields[USER].set("aéBCDEf");
            check.phase = 28;
        }
        28 => {
            let Some(p) = position(&form.fields[USER], USER, 6, &runs) else {
                return;
            };
            point(p);
            buttons.press(MouseButton::Left);
            check.phase = 29;
        }
        29 => {
            assert_eq!(form.fields[USER].cursor, 6);
            send_key(&mut keyboard, KeyCode::Home, Key::Home, ButtonState::Pressed, None);
            buttons.release(MouseButton::Left);
            check.phase = 30;
        }
        30 => {
            assert_eq!(form.fields[USER].cursor, 0);
            assert!(form.fields[USER].selected().is_empty());
            check.phase = 31;
        }
        31 => {
            assert_eq!(form.fields[USER].cursor, 0, "old pointer work must not undo Home");
            let Some(p) = position(&form.fields[USER], USER, 6, &runs) else {
                return;
            };
            point(p);
            buttons.press(MouseButton::Left);
            check.phase = 32;
        }
        32 => {
            assert_eq!(form.fields[USER].cursor, 6);
            select_all(&mut keyboard);
            buttons.release(MouseButton::Left);
            check.phase = 33;
        }
        33 => {
            assert_eq!(form.fields[USER].selected(), "aéBCDEf");
            check.phase = 34;
        }
        34 => {
            assert_eq!(form.fields[USER].selected(), "aéBCDEf", "old pointer work must not shorten Ctrl+A");
            check.phase = 35;
            info!(
                "login pointer checks passed: Unicode/mask/Shift, scroll/replacement, pending/release endpoint, focus loss, keyboard retirement, no action"
            );
        }
        _ => unreachable!(),
    }
}

fn deadline(time: Res<Time>, check: Res<Check>) {
    if time.elapsed_secs() > 3. {
        assert_eq!(check.phase, 35, "login pointer checks stalled at phase {}", check.phase);
    }
}
