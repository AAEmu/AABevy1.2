//! Offline visual control for the real login form. No Session or saved settings.
use super::*;
mod pointer_checks;

pub(super) fn run() {
    let args: Vec<_> = std::env::args().collect();
    let mut form = LoginForm::new(
        config::Saved {
            user: "bevy_selection_demo".into(),
            server: "127.0.0.1:1237".into(),
        },
        None,
    );
    form.fields[PASSWORD].set("selection_test_only");
    form.focus = if args.iter().any(|a| a == "--password-selection") {
        PASSWORD
    } else {
        USER
    };
    form.fields[form.focus].select_all();
    if args.iter().any(|a| a == "--reverse-selection") {
        form.fields[form.focus].home(false);
        form.fields[form.focus].right(false);
        form.fields[form.focus].right(false);
        form.fields[form.focus].home(true);
    }
    assert!(!form.fields[form.focus].selected().is_empty());
    let mut app = App::new();
    register_art_source(&mut app);
    app.add_plugins((aa_dev::default_plugins("aa-client: offline login selection"), aa_dev::DevPlugin))
        .insert_resource(form)
        .init_resource::<FormBuilt>()
        .add_message::<NetCommand>()
        .add_message::<Act>()
        .add_systems(Startup, (fonts, spawn_screen))
        .add_systems(
            Update,
            (
                rebuild,
                art::animate.run_if(resource_exists::<Art>),
                keyboard,
                mouse,
                tick,
                sync,
                scroll_fields,
                art::mipmaps,
            )
                .chain(),
        );
    if let Some(page) = flag(&args, "--settings-page") {
        let page = page.parse().expect("settings page index");
        let world = app.world_mut();
        let mut windows = world.query_filtered::<&mut Window, With<PrimaryWindow>>();
        if let Ok(mut window) = windows.single_mut(world) {
            window.focused = false;
        }
        if page >= 0 {
            app.add_plugins(aa_ui::UiPlugin { config: Some(host::ui_config()) })
                .insert_non_send(crate::login::LoginBridge(crate::login::gameapi::new_shared()))
                .insert_resource(SettingsPage(page))
                .add_systems(PreUpdate, settings_click.after(bevy::input::InputSystems))
                .add_systems(Update, settings_action.after(mouse).before(aa_ui::UiInput));
        }
    }
    if let Some(manifest) = art::assets_dir().and_then(|d| art::Manifest::load(&d)) {
        app.insert_resource(Art::new(manifest));
    }
    if args.iter().any(|arg| arg == "--pointer-check") {
        pointer_checks::install(&mut app);
    }
    if let Some(value) = args
        .windows(2)
        .find(|pair| pair[0] == "--ui-scale")
        .and_then(|pair| pair[1].parse::<f32>().ok())
    {
        assert!(value.is_finite() && value > 0.);
        app.insert_resource(UiScale(value));
    }
    app.run();
}

fn fonts(mut commands: Commands, mut fonts: ResMut<Assets<Font>>) {
    let heading = load_font(&mut fonts, "ui/font/librebaskerville-bold.ttf");
    let text = load_font(&mut fonts, "ui/font/flareserif_821_roman.ttf");
    commands.insert_resource(Fonts { heading, text });
    info!("offline login selection fixture: synthetic text; no Session, Connect handling or persistence");
}

fn tick(time: Res<Time>, mut form: ResMut<LoginForm>) {
    form.age += time.delta_secs();
    form.blink += time.delta_secs();
}

#[derive(Resource)]
struct SettingsPage(i64);

fn settings_click(
    time: Res<Time>,
    mut clicked: Local<bool>,
    cog: Query<&UiGlobalTransform, With<options::Button>>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
) {
    if *clicked {
        if mouse.pressed(MouseButton::Left) {
            mouse.release(MouseButton::Left);
        }
        return;
    }
    if time.elapsed_secs() < 0.5 {
        return;
    }
    let (Ok(cog), Ok(mut window)) = (cog.single(), windows.single_mut()) else { return };
    let point = cog.affine().translation / window.resolution.scale_factor();
    window.focused = true;
    window.set_cursor_position(Some(point));
    mouse.press(MouseButton::Left);
    *clicked = true;
}

fn settings_action(world: &mut World) {
    let open = world.resource_mut::<Messages<Act>>().drain().any(|act| matches!(act, Act::Settings));
    if !open {
        return;
    }
    let mut windows = world.query_filtered::<&mut Window, With<PrimaryWindow>>();
    if let Ok(mut window) = windows.single_mut(world) {
        window.focused = false;
    }
    options::open(world);
    let rt = world.non_send::<aa_ui::UiRuntime>();
    assert!(options::visible(Some(rt)), "settings cog should open Options");
    let page = world.resource::<SettingsPage>().0;
    if page == 0 {
        return;
    }
    rt.lua.load(r#"
        local index = ...
        local content = optionFrame.contentWindow
        for _, category in ipairs(content.menuListFrame.categoryFrames) do
            for _, button in ipairs(category.buttons) do
                if button.subFrameIndex == index then button:OnClick("LeftButton") end
            end
        end
        local page = content.pageWindow.subFrames[index]
        assert(page:IsVisible())
        page.content:ChangeChildAnchorByScrollValue("vert", 80)
        for _, row in ipairs(page.content.optionFrames) do
            if row.OnEnter and row.optionControl and row.optionControl.tooltipText then
                row:OnEnter()
                assert(tooltip.window:GetWidth() == 200)
                break
            end
        end
    "#).call::<()>(page).unwrap();
}
