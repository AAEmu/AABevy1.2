//! A received zero-ratio error over the original HUD, without a Session or sale.
use super::*;
use bevy::window::PrimaryWindow;

pub(in crate::hud) fn run() {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    let (rt, state, path) = crate::hud::breath::fixture::runtime(width as f32, height as f32).unwrap();
    // Explicit PRE cache without a UI callback; inject the zero receipt after2s
    // so a3s capture includes the1s-old message and a normal FPS log sample.
    receive(None, &state, &Event::SpecialtyRatio(137));
    let mut app = App::new();
    app.add_plugins((
        aa_dev::default_plugins("aa-client: original specialty receipt error"),
        aa_dev::DevPlugin,
    ))
    .add_plugins(aa_ui::UiPlugin {
        config: Some(crate::login::host::ui_config()),
    })
    .insert_non_send(rt)
    .insert_non_send(HudBridge(state))
    .add_message::<NetEvent>()
    .insert_resource(crate::hud::HudBuilt { up: true, fresh: false })
    .add_systems(PreUpdate, show_error)
    .add_systems(Startup, physical_pixels);
    crate::hud::enchant::plugin(&mut app);
    app.run();
    if path.exists() {
        std::fs::remove_file(path).unwrap();
    }
}

fn show_error(mut sent: Local<bool>, time: Res<Time>, mut events: MessageWriter<NetEvent>) {
    if !*sent && time.elapsed_secs() >= 2.0 {
        events.write(NetEvent(Event::SpecialtyRatio(0)));
        *sent = true;
    }
}

fn physical_pixels(mut windows: Query<&mut Window, With<PrimaryWindow>>) {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    for mut window in &mut windows {
        window.resolution.set_scale_factor_override(Some(1.0));
        window.resolution.set_physical_resolution(width, height);
    }
}
