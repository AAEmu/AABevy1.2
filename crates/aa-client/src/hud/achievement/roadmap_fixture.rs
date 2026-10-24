//! Original roadmap controls over a received type-5 fixture, without a server.
use super::*;

pub(in crate::hud) fn run() {
    let args = std::env::args().collect::<Vec<_>>();
    let (width, height) = aa_dev::DevArgs::from_env().size;
    let (rt, bridge, path) =
        super::super::breath::fixture::runtime(width as f32, height as f32).unwrap();
    let before = args.iter().any(|arg| arg == "--before");
    let size = args
        .windows(2)
        .find(|a| a[0] == "--map-size")
        .map(|a| a[1].parse::<u8>().unwrap())
        .unwrap_or(1);
    assert!((1..=3).contains(&size));
    let visible = !args.iter().any(|arg| arg == "--hidden");
    let npc = args.iter().any(|arg| arg == "--npc");
    let projection = args.iter().any(|arg| arg == "--projection-before");
    let centre = args.iter().any(|arg| arg == "--centre");
    {
        let mut data = bridge.borrow_mut();
        data.character_id = 47;
        data.pos = if centre {
            Vec3::new(14330.122, 14707., 128.)
        } else {
            Vec3::new(15579.389, 15385.888, 128.)
        };
        data.snapshot.zone_group = 5;
        data.snapshot.zone_key_name = "w_solzreed".into();
        data.snapshot.zone_rect = [11925., 13266., 4810.2446, 2882.];
    }
    super::super::api::push_changes(&rt, &bridge);
    let spawn = super::super::expansion::fixture::spawn();
    dispatch(
        Some(&rt),
        &bridge,
        &Event::CharacterSelected(Box::new(spawn.state)),
    );
    let payload = format!(
        "other saved\r\nroadmap_option\r\n size {size}\r\n visible {visible}\r\n npcShow {npc}\r\n"
    );
    let response = aa_net::UiDataResponse {
        character_id: 47,
        data_type: 5,
        raw_length: payload.len() as i32 + 1,
        payload,
    };
    if projection {
        //The former producer always projected against L, independently of
        //the actual selected art. Keep that as an explicit same-binary control.
        rt.lua.load("HUD.road_layout=function(level,id,key) return HUD.road_frame(level,id,key,100),926,556,100 end").exec().unwrap();
    }
    if before {
        bridge
            .borrow_mut()
            .achievement
            .persistence
            .receive(&response);
    } else {
        dispatch(
            Some(&rt),
            &bridge,
            &Event::UiData(aa_net::UiDataEvent::Response(response)),
        );
    }
    // The before control omits only projection of the same saved record into
    // the controls. It uses the final binary, not a historical or native capture.
    let expected = if before {
        "L"
    } else {
        ["S", "M", "L"][size as usize - 1]
    };
    rt.lua.load(format!("assert(roadmapSizeButton:GetText()=='{expected}');assert(roadmapWindow:IsVisible()=={})", before || visible)).exec().unwrap();
    aa_ui::lua::tick(&rt.lua, 16.);
    aa_ui::lua::flush(&rt.lua);
    if centre {
        let (w,h,x,y)=rt.lua.load("local w,h=roadmap:GetExtent();local size=math.floor(w*100/926+.5);local f=HUD.road_frame(2,5,'W_SOLZREED',size);local p=AAUI.RoadMap().player;return w,h,f[1]+p.u*f[3],f[2]+p.v*f[4]").eval::<(f64,f64,f64,f64)>().unwrap();
        println!(
            "roadmap-projection: map={w:.6},{h:.6} centre={x:.6},{y:.6} drift={:.6},{:.6}",
            x - w * 0.5,
            y - h * 0.5
        );
        if !projection {
            assert!((x - w * 0.5).abs() < 0.002 && (y - h * 0.5).abs() < 0.002);
        }
    }
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    info!("roadmap fixture: before={before} saved_size={size} visible={visible} npc={npc}");
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-client: original roadmap settings"),
            aa_dev::DevPlugin,
            aa_world::pipelines::build,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(crate::login::host::ui_config()),
        })
        .insert_non_send(rt)
        .insert_non_send(HudBridge(bridge))
        .add_systems(Update, report)
        .run();
    if path.exists() {
        std::fs::remove_file(path).unwrap();
    }
}

fn report(
    time: Res<Time<Real>>,
    pipelines: Res<aa_world::pipelines::PipelineStatus>,
    mut done: Local<bool>,
) {
    if !*done && time.elapsed_secs() > 6. {
        *done = true;
        info!(
            "roadmap-control: pipelines {} waiting {}",
            pipelines.total(),
            pipelines.waiting()
        );
    }
}
