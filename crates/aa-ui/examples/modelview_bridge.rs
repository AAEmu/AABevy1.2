//! Supplied diagnostic images through the real host seam; no character or shop simulation.
//! --shot shots/aa-ui/modelview-bridge-20261024.png --shot-after 8
use std::collections::HashMap;

use aa_ui::{ModelViewId, ModelViewImage, ModelViewOp, UiModelViews, UiRuntime};
use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

#[derive(Resource, Default)]
struct Host {
    patterns: Vec<Handle<Image>>,
    live: HashMap<ModelViewId, ModelViewImage>,
    retired: Option<ModelViewId>,
    phase: u8,
    stable: Option<(u64, usize)>,
}

fn pattern(alternate: bool) -> Image {
    let mut pixels = vec![0; 512 * 512 * 4];
    for y in 0..512 {
        for x in 0..512 {
            let i = (y * 512 + x) * 4;
            let grid = x % 32 < 2 || y % 32 < 2;
            let border = !(8..504).contains(&x) || !(8..504).contains(&y);
            let center = (250..262).contains(&x) || (250..262).contains(&y);
            pixels[i..i + 4].copy_from_slice(&if border {
                [255, 255, 255, 255]
            } else if center {
                [255, 215, 40, 255]
            } else if grid {
                [35, 45, 65, 255]
            } else if alternate {
                [210, (y / 2) as u8, (x / 3) as u8, 255]
            } else {
                [(x / 2) as u8, 150, (y / 2) as u8, 255]
            });
        }
    }
    Image::new(
        Extent3d {
            width: 512,
            height: 512,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        pixels,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    )
}

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui modelview: supplied host pattern"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(aa_ui::UiConfig {
                clear: bevy::camera::ClearColorConfig::Custom(Color::srgb(0.035, 0.045, 0.065)),
                ..default()
            }),
        })
        .init_resource::<Host>()
        .insert_resource(aa_ui::UiSetup::new(|ui| {
            ui.exec(
                "@modelview_image_fixture",
                include_str!("fixtures/modelview_bridge.lua"),
            )
            .unwrap();
        }))
        .add_systems(
            Startup,
            |mut host: ResMut<Host>, mut images: ResMut<Assets<Image>>| {
                host.patterns = vec![images.add(pattern(false)), images.add(pattern(true))];
            },
        )
        .add_systems(Update, supply.in_set(UiModelViews))
        .run();
}

fn supply(
    ui: Option<NonSend<UiRuntime>>,
    time: Res<Time>,
    mut host: ResMut<Host>,
    mut images: ResMut<Assets<Image>>,
    stats: Res<aa_ui::UiStats>,
) {
    let Some(ui) = ui else { return };
    let t = time.elapsed_secs();
    let desired = if t >= 5. { 5 } else { t.floor() as u8 };
    while host.phase < desired {
        host.phase += 1;
        match host.phase {
            1 => ui.exec("@resize", "resized:SetExtent(180,290);resized:SetModelViewExtent(180,290)").unwrap(),
            2 => ui.exec("@hidden", "hidden:Show(false);clipped:SetAlpha(.65)").unwrap(),
            3 => {
                host.live.clear(); // next associations replace the actual image handles
                ui.exec("@replaced", "status:SetText('Actual image B replaced A; geometry and crop retained')").unwrap();
            }
            4 => ui.exec("@retirement", "retired:EnableHidingIsRemove(true);retired:Show(false)").unwrap(),
            5 => ui.exec("@new_incarnation", "makeReplacement();status:SetText('Hidden surface retained; old incarnation retired; new incarnation admitted')").unwrap(),
            _ => unreachable!(),
        }
        info!("modelview bridge: phase {}", host.phase);
    }
    let work = ui.take_modelview_work();
    for command in work.commands {
        match command.op {
            ModelViewOp::Destroy => {
                host.live.remove(&command.view);
                host.retired = Some(command.view);
                assert!(!ui.set_modelview_image(command.view, None));
                info!(
                    "modelview bridge: Destroy retired {:?}; stale image rejected",
                    command.view
                );
            }
            ModelViewOp::ClearModel => {
                host.live.remove(&command.view);
            }
            op => info!("modelview bridge: {:?} {:?}", command.view, op),
        }
    }
    host.live
        .retain(|view, _| work.surfaces.iter().any(|s| s.view == *view));
    for surface in &work.surfaces {
        // These actual 512x512 diagnostic images are supplied by this example's host.
        // No target-size clamp, actor readiness, renderer pass or camera is implied.
        let which = usize::from(host.phase >= 3);
        let image = ModelViewImage {
            image: host.patterns[which].clone(),
            extent_px: [512, 512],
        };
        host.live.insert(surface.view, image.clone());
        assert!(ui.set_modelview_image(surface.view, Some(image)));
    }
    // Repeated changes to a real host Image's pixels do not invalidate UI geometry.
    let handle = host.patterns[1].clone();
    if let Some(mut image) = images.get_mut(&handle)
        && let Some(data) = image.data.as_mut()
    {
        data[0] = ((t * 60.) as u32 % 255) as u8;
    }
    if t >= 6. && host.stable.is_none() {
        host.stable = Some((stats.rebuilds, stats.mesh_uploads));
        info!(
            "modelview bridge: stable={} surfaces, admitted={}; rebuilds={} uploads={}",
            work.surfaces.len(),
            work.surfaces.iter().filter(|s| s.paint_admitted).count(),
            stats.rebuilds,
            stats.mesh_uploads
        );
    }
    if let Some((rebuilds, uploads)) = host.stable {
        assert_eq!(
            stats.rebuilds, rebuilds,
            "pixel-only updates rebuilt UI geometry"
        );
        assert_eq!(stats.mesh_uploads, uploads);
    }
}
