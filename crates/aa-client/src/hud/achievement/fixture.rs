//! Original achievement scripts, real metadata and explicitly encoded offline progress receipts.
use super::*;
use aa_net::{AchievementEvent, AchievementProgress};
use bevy::window::PrimaryWindow;

pub(super) fn runtime(width: f32, height: f32) -> Result<(UiRuntime, Shared, std::path::PathBuf), Box<dyn std::error::Error>> {
    let (rt, bridge, path) = super::super::breath::fixture::runtime(width, height)?;
    dispatch(
        None,
        &bridge,
        &Event::InitialConfig(aa_net::initial_config::InitialConfig {
            host: vec![],
            feature_bytes: vec![0, 0, 0, 0, 0, 0, 0, 128],
            raw_candidate_count: 0,
            candidate_codes: vec![],
            initial_lp: 0,
            service_flags: [0; 7],
            second_price_type: 0,
            second_password_max_fail_count: 0,
            idle_kick_time: 0,
        }),
    );
    let rows = [
        AchievementProgress {
            id: 1,
            amount: 999,
            completed_at: 1_792_276_001,
        },
        AchievementProgress {
            id: 2,
            amount: 3,
            completed_at: 0,
        },
        AchievementProgress {
            id: 3,
            amount: 4,
            completed_at: 0,
        },
        AchievementProgress {
            id: 4,
            amount: 1,
            completed_at: 0,
        },
        AchievementProgress {
            id: 1000,
            amount: 2,
            completed_at: 0,
        },
    ];
    let mut wire = aa_net::wire::Writer::new();
    wire.i32(rows.len() as i32);
    for row in rows {
        wire.u32(row.id).i32(row.amount).i64(row.completed_at);
    }
    let receipt = AchievementEvent::read_lane(0x1fc, &mut aa_net::wire::Reader::new(wire.as_bytes()))?;
    dispatch(None, &bridge, &Event::Achievement(receipt));
    assert!(aa_ui::content::hotkey(&rt.lua, "toggle_achievement")?);
    let window = aa_ui::lua::ui(&rt.lua)
        .tree
        .find("achievementWnd")
        .ok_or("original achievement window missing")?;
    rt.lua.globals().set(
        "ACHIEVEMENT_FIXTURE",
        aa_ui::lua::object_table(&rt.lua, window)?.ok_or("achievement table missing")?,
    )?;
    rt.lua
        .load(
            r#"
        assert(X2Achievement:AddTracingAchievement(2))
        assert(X2Achievement:AddTracingAchievement(1000))
        assert(ACHIEVEMENT_FIXTURE:IsVisible())
        ACHIEVEMENT_FIXTURE:ShowAchivementListByCategory(1,1)
        local i=X2Achievement:GetSubcategoryInfo(1,1);assert(i.complete==1 and i.total>7)
    "#,
        )
        .exec()?;
    aa_ui::lua::flush(&rt.lua);
    Ok((rt, bridge, path))
}

pub(crate) fn run() {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    let (rt, _bridge, path) = runtime(width as f32, height as f32).unwrap();
    if aa_dev::args().into_iter().any(|arg| arg == "--overall") {
        rt.lua.load("ACHIEVEMENT_FIXTURE:ShowOverallWnd()").exec().unwrap();
    }
    if aa_dev::args().into_iter().any(|arg| arg == "--tracing") {
        rt.lua.load("ACHIEVEMENT_FIXTURE:ShowTracingAchivementList()").exec().unwrap();
    }
    if aa_dev::args().into_iter().any(|arg| arg == "--detail") {
        rt.lua
            .load(
                "ACHIEVEMENT_FIXTURE.contentWnd:ShowAchievementDetail(1,{viewType=AVT_LIST,list=X2Achievement:GetAchievementList(1,1,1)})",
            )
            .exec()
            .unwrap();
    }
    App::new()
        .add_plugins((aa_dev::default_plugins("aa-client: original achievements"), aa_dev::DevPlugin))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(crate::login::host::ui_config()),
        })
        .insert_non_send(rt)
        .add_systems(Startup, physical_pixels)
        .run();
    if path.exists() {
        std::fs::remove_file(path).unwrap();
    }
}

fn physical_pixels(mut windows: Query<&mut Window, With<PrimaryWindow>>) {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    for mut window in &mut windows {
        window.resolution.set_scale_factor_override(Some(1.0));
        window.resolution.set_physical_resolution(width, height);
    }
}
