use super::*;

fn harness() -> App {
    let mut app = App::new();
    let mut form = LoginForm::new(config::Saved::default(), None);
    form.age = 1.;
    form.fields[USER].set("aé界");
    form.fields[PASSWORD].set("secret");
    app.insert_resource(form)
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<Time>()
        .add_message::<KeyboardInput>()
        .add_message::<Act>()
        .add_systems(Update, (keyboard, sync).chain());
    for i in 0..3 {
        for part in 0..4 {
            app.world_mut()
                .spawn((FieldText(i, part), Text::default(), TextColor(INK), BackgroundColor(Color::NONE)));
        }
    }
    app
}

fn key(app: &mut App, key: Key, ctrl: bool, shift: bool, text: Option<&str>) {
    let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
    keys.reset_all();
    if ctrl {
        keys.press(KeyCode::ControlLeft);
    }
    if shift {
        keys.press(KeyCode::ShiftLeft);
    }
    app.world_mut().write_message(KeyboardInput {
        key_code: KeyCode::Unidentified(bevy::input::keyboard::NativeKeyCode::Unidentified),
        logical_key: key,
        state: ButtonState::Pressed,
        text: text.map(Into::into),
        repeat: false,
        window: Entity::PLACEHOLDER,
    });
    app.update();
}

fn event(app: &mut App, logical_key: Key, key_code: KeyCode, state: ButtonState) {
    app.world_mut().write_message(KeyboardInput {
        key_code,
        logical_key,
        state,
        text: None,
        repeat: false,
        window: Entity::PLACEHOLDER,
    });
}

#[test]
fn modifier_releases_later_in_the_same_batch_do_not_change_earlier_shortcuts() {
    use ButtonState::{Pressed, Released};
    let mut app = harness();
    app.world_mut().resource_mut::<LoginForm>().fields[USER].home(false);
    app.world_mut().resource_mut::<LoginForm>().fields[USER].right(false);
    // Shift was held before this batch and has been released by InputPlugin.
    event(&mut app, Key::ArrowRight, KeyCode::ArrowRight, Pressed);
    event(&mut app, Key::ArrowRight, KeyCode::ArrowRight, Released);
    event(&mut app, Key::Shift, KeyCode::ShiftLeft, Released);
    app.update();
    assert_eq!(app.world().resource::<LoginForm>().fields[USER].selected(), "é");

    // A complete Ctrl+A stroke also arrives in one event batch.
    event(&mut app, Key::Control, KeyCode::ControlLeft, Pressed);
    event(&mut app, Key::Character("a".into()), KeyCode::KeyA, Pressed);
    event(&mut app, Key::Character("a".into()), KeyCode::KeyA, Released);
    event(&mut app, Key::Control, KeyCode::ControlLeft, Released);
    app.update();
    assert_eq!(app.world().resource::<LoginForm>().fields[USER].selected(), "aé界");
    key(&mut app, Key::Character("replacement".into()), false, false, Some("replacement"));
    assert_eq!(app.world().resource::<LoginForm>().fields[USER].text, "replacement");
}

#[test]
fn releasing_one_modifier_side_keeps_the_other_side_active() {
    use ButtonState::{Pressed, Released};
    let mut app = harness();
    // Both Ctrl keys were held before the batch, then released on opposite
    // sides of A. The logical-key aggregate ends false, but A still had Ctrl.
    event(&mut app, Key::Control, KeyCode::ControlLeft, Released);
    event(&mut app, Key::Character("a".into()), KeyCode::KeyA, Pressed);
    event(&mut app, Key::Control, KeyCode::ControlRight, Released);
    app.update();
    assert_eq!(app.world().resource::<LoginForm>().fields[USER].selected(), "aé界");
}

#[test]
fn keyboard_selection_replaces_text_and_projects_the_active_highlight() {
    let mut app = harness();
    key(&mut app, Key::Character("a".into()), true, false, Some("a"));
    assert_eq!(app.world().resource::<LoginForm>().fields[USER].selected(), "aé界");
    let world = app.world_mut();
    let mut texts = world.query::<(&FieldText, &Text, &BackgroundColor)>();
    let (_, text, background) = texts.iter(world).find(|(part, _, _)| part.0 == USER && part.1 == 1).unwrap();
    assert_eq!(text.0, "aé界");
    assert_ne!(background.0, Color::NONE);
    key(&mut app, Key::Character("雪".into()), false, false, Some("雪"));
    assert_eq!(app.world().resource::<LoginForm>().fields[USER].text, "雪");
    key(&mut app, Key::Home, false, true, None);
    assert_eq!(app.world().resource::<LoginForm>().fields[USER].selected(), "雪");
    key(&mut app, Key::Backspace, false, false, None);
    assert!(app.world().resource::<LoginForm>().fields[USER].text.is_empty());
    assert!(app.world_mut().resource_mut::<Messages<Act>>().drain().next().is_none());
}

#[test]
fn selected_password_stays_masked_and_busy_login_rejects_editing() {
    let mut app = harness();
    app.world_mut().resource_mut::<LoginForm>().focus = PASSWORD;
    key(&mut app, Key::Character("a".into()), true, false, None);
    let world = app.world_mut();
    let mut texts = world.query::<(&FieldText, &Text)>();
    let shown = texts
        .iter(world)
        .filter(|(part, _)| part.0 == PASSWORD)
        .map(|(_, text)| text.0.as_str())
        .collect::<String>();
    assert_eq!(shown, "••••••");
    // Both copy and cut are inert for the masked field, even with a selection.
    for name in ["c", "x"] {
        key(&mut app, Key::Character(name.into()), true, false, None);
        assert_eq!(app.world().resource::<LoginForm>().fields[PASSWORD].text, "secret");
    }
    app.world_mut().resource_mut::<LoginForm>().status = Status::Connecting { since: 0. };
    key(&mut app, Key::Character("replacement".into()), false, false, Some("replacement"));
    key(&mut app, Key::Delete, false, false, None);
    assert_eq!(app.world().resource::<LoginForm>().fields[PASSWORD].text, "secret");
    let world = app.world_mut();
    let mut backgrounds = world.query::<(&FieldText, &BackgroundColor)>();
    assert!(backgrounds.iter(world).all(|(_, background)| background.0 == Color::NONE));
}
