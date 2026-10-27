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
