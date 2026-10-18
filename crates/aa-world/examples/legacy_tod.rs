//! Controlled native curve swatches from login2's real nested Area/TimeOfDay.
//! No world override or new lighting calibration; --shot captures the parser's raw outputs.
use aa_world::env::TimeOfDay;
use bevy::prelude::*;

fn main() {
    let xml = aa_pak::Pak::global()
        .read_string("worlds/login2/cells/001_000/client/entities.xml")
        .unwrap();
    let doc = roxmltree::Document::parse(&xml).unwrap();
    let entity = doc
        .descendants()
        .find(|n| n.has_tag_name("Entity") && n.attribute("Name") == Some("login_224_oktop_2"))
        .unwrap();
    let block = entity
        .descendants()
        .find(|n| n.has_tag_name("TimeOfDay"))
        .unwrap();
    let control = std::env::args().any(|arg| arg == "--old-parser-control");
    let block_xml = &xml[block.range()];
    // The previous parser treated pre-sRGB XML exactly like modern XML. This
    // control reproduces that behavior in the same executable for a fixed comparison.
    let block_xml = if control {
        block_xml.replacen("<TimeOfDay", "<TimeOfDay sRGB=\"1\"", 1)
    } else {
        block_xml.to_string()
    };
    let tod = TimeOfDay::parse(&block_xml).unwrap();
    App::new()
        .add_plugins(aa_dev::default_plugins(
            "aa-world original legacy area curves",
        ))
        .add_plugins(aa_dev::DevPlugin)
        .insert_resource(ClearColor(Color::srgb(0.035, 0.045, 0.065)))
        .add_systems(Startup, move |mut commands: Commands| {
            commands.spawn(Camera2d);
            label(
                &mut commands,
                if control { "login2 / login_224_oktop_2 : previous parser control" } else { "login2 / login_224_oktop_2 : native legacy migration" },
                -885.0,
                465.0,
                28.0,
            );
            label(
                &mut commands,
                "Raw RGB; scalar strips use native catalog ranges. Clock hours 0-24.",
                -885.0,
                423.0,
                19.0,
            );
            for hour in [0, 6, 12, 18, 24] {
                label(
                    &mut commands,
                    &format!("{hour:02}:00"),
                    -470.0 + hour as f32 / 24.0 * 1200.0,
                    375.0,
                    18.0,
                );
            }
            for (row, (name, color, min, max)) in [
                ("Sun color", true, 0.0, 1.0),
                ("Sun color multiplier", false, 0.0, 16.0),
                ("Sky color", true, 0.0, 1.0),
                ("Sky color multiplier", false, 0.0, 16.0),
                ("Fog color", true, 0.0, 1.0),
                ("HDR dynamic power factor", false, -4.0, 4.0),
                ("Film curve whitepoint", false, 0.0, 10.0),
            ]
            .into_iter()
            .enumerate()
            {
                let y = 310.0 - row as f32 * 105.0;
                label(&mut commands, name, -885.0, y, 20.0);
                for column in 0..120 {
                    let t = column as f32 / 119.0;
                    let value = if color {
                        tod.color(name, t)
                    } else {
                        Vec3::splat((tod.scalar(name, t) - min) / (max - min))
                    };
                    commands.spawn((
                        Sprite::from_color(
                            Color::linear_rgb(value.x, value.y, value.z),
                            Vec2::new(10.1, 72.0),
                        ),
                        Transform::from_xyz(-470.0 + column as f32 * 10.0, y, 0.0),
                    ));
                }
            }
            info!(
                "legacy area: noon Sun {:?} x {}, Sky {:?} x {}, HDR {}, Film whitepoint {}",
                tod.color("Sun color", 0.5),
                tod.scalar("Sun color multiplier", 0.5),
                tod.color("Sky color", 0.5),
                tod.scalar("Sky color multiplier", 0.5),
                tod.scalar("HDR dynamic power factor", 0.5),
                tod.scalar("Film curve whitepoint", 0.5)
            );
        })
        .run();
}

fn label(commands: &mut Commands, value: &str, x: f32, y: f32, size: f32) {
    commands.spawn((
        Text2d::new(value),
        TextFont {
            font_size: FontSize::Px(size),
            ..default()
        },
        bevy::sprite::Anchor::CENTER_LEFT,
        Transform::from_xyz(x, y, 1.0),
    ));
}
