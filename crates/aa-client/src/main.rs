//! ArcheAge 1.2 client remake. Screen flow (mirrors the real client):
//! world select -> character select -> character create -> loading -> in game.

use bevy::prelude::*;

#[derive(States, Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Screen {
    #[default]
    WorldSelect,
    CharacterSelect,
    CharacterCreate,
    Loading,
    InGame,
}

fn main() {
    App::new()
        .add_plugins((aa_dev::default_plugins("ArcheAge"), aa_dev::DevPlugin))
        .init_state::<Screen>()
        .add_systems(Startup, setup)
        .run();
}

fn setup(mut commands: Commands) {
    info!("game_pak: {} files", aa_pak::Pak::global().len());
    commands.spawn(Camera2d);
}
