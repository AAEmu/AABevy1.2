//! Original recruiting panel over explicit received player/target/queue state, without a connection.
use super::*;
use bevy::window::PrimaryWindow;

pub(super) fn runtime(width: f32, height: f32) -> Result<(UiRuntime, Shared, std::path::PathBuf), Box<dyn std::error::Error>> {
    let (rt, bridge, path) = crate::hud::breath::fixture::runtime(width, height)?;
    crate::hud::api::mark_pushed(&bridge);
    {
        let mut data = bridge.borrow_mut();
        data.character_id = 47;
        data.snapshot.level = 45;
        data.battlefield.owner = 47;
        data.battlefield.level = Some(45);
        data.battlefield.buffs = Some(vec![]);
        data.pos = Vec3::ZERO;
        data.target_pos = Some(Vec3::X);
        data.panels.target = Some(crate::hud::panels::TargetSnap {
            obj_id: 201,
            npc_type: Some(3),
            name: "Recruiting NPC fixture".into(),
            level: 45,
            hp: 100,
            max_hp: 100,
            unit_type: "npc",
            detail: "npc",
            relation: "friendly",
            ..default()
        });
    }
    crate::hud::api::push_changes(&rt, &bridge);
    rt.lua
        .load("ADDON:ShowContent(UIC_BAG,false);assert(X2Unit:GetNpcTypeIndex('target')==3)")
        .exec()?;
    rt.fire_event("NPC_INTERACTION_START", ("recruiter", "", "201"));
    aa_ui::lua::tick(&rt.lua, 500.);
    rt.lua
        .load(
            r#"
        assert(battlefield.entranceWnd:IsVisible())
        assert(battlefield.entranceWnd.targetNpcIndex==3)
        assert(battlefield.entranceWnd.contentFrame[1].fixedNumber:GetText()=='1 vs 1')
        assert(battlefield.entranceWnd.contentFrame[1].entranceRandomBtn:IsVisible())
        assert(battlefield.entranceAlarm==nil)
    "#,
        )
        .exec()?;
    Ok((rt, bridge, path))
}

pub(super) fn prepare(rt: &UiRuntime, bridge: &Shared, queued: bool, invited: bool) -> Result<(), Box<dyn std::error::Error>> {
    if queued || invited {
        crate::hud::expansion::fixture::click(rt, "battlefield.entranceWnd.contentFrame[1].entranceRandomBtn")?;
        let command = flush(&mut bridge.borrow_mut())[0];
        dispatch(&rt, &bridge, &Event::Battlefield(BattlefieldEvent::Requested(command)));
        dispatch(
            &rt,
            &bridge,
            &Event::Battlefield(BattlefieldEvent::Applied {
                battlefield_id: 1,
                corps: 2,
                error: 0,
            }),
        );
        dispatch(
            &rt,
            &bridge,
            &Event::Battlefield(BattlefieldEvent::QueueState(aa_net::battlefield::BattlefieldQueue {
                battlefield_id: 1,
                corps_size: 1,
                counts: [1, 1, 0],
            })),
        );
    }
    if invited {
        rt.lua
            .load(
                r#"
            local request=X2DialogManager.RequestTaskDialog
            X2DialogManager.RequestTaskDialog=function(self,task,info,owner)
                local wnd=request(self,task,info,owner)
                if task==DLG_TASK_JOIN_BATTLE_FIELD then BATTLE_DIALOG=wnd end
                return wnd
            end
        "#,
            )
            .exec()?;
        dispatch(
            &rt,
            &bridge,
            &Event::Battlefield(BattlefieldEvent::Invited(aa_net::battlefield::BattlefieldInvitation {
                zone_id: 501,
                instance_id: 9,
                ruleset_id: 27,
                corps: 1,
                qualified_id: 9_007_199_254_740_993,
            })),
        );
        rt.lua.load("assert(BATTLE_DIALOG and BATTLE_DIALOG:IsVisible())").exec()?;
    }
    Ok(())
}

pub(super) fn sample(rt: &UiRuntime, bridge: &Shared, local: u32) {
    let received_at = std::time::Instant::now();
    dispatch(
        rt,
        bridge,
        &Event::ClockSample(aa_net::ClockSample {
            pong: aa_net::proxy::ProxyPong {
                tm: 0,
                when: 0,
                elapsed: 0,
                remote: 0,
                local,
                world: 100000 + local,
            },
            local_receive_ms: local,
            received_at,
            local_origin: received_at - std::time::Duration::from_millis(u64::from(local)),
        }),
    );
}

pub(super) fn join(rt: &UiRuntime, bridge: &Shared) -> Result<(), Box<dyn std::error::Error>> {
    prepare(rt, bridge, true, true)?;
    crate::hud::expansion::fixture::click(rt, "BATTLE_DIALOG.btnOk")?;
    let command = flush(&mut bridge.borrow_mut())[0];
    dispatch(rt, bridge, &Event::Battlefield(BattlefieldEvent::Requested(command)));
    dispatch(rt, bridge, &Event::PlayerSpawn(Box::new(crate::hud::expansion::fixture::spawn())));
    sample(rt, bridge, 0);
    let mut skills = [0; 24];
    skills[..4].copy_from_slice(&[12399, 12374, 12380, 12416]);
    dispatch(
        rt,
        bridge,
        &Event::Battlefield(BattlefieldEvent::Joined(aa_net::battlefield::BattlefieldJoined {
            zone_id: 501,
            instance_id: 9,
            corps: 1,
            victory_score: 0,
            victory_kill_count: 50,
            victory_kill_corps_head: 0,
            phase_times_ms: [6000, 120000, 30000],
            factions: [159, 160],
            killstreak_skills: skills,
        })),
    );
    Ok(())
}

pub(super) fn play(rt: &UiRuntime, bridge: &Shared) {
    dispatch(
        rt,
        bridge,
        &Event::Battlefield(BattlefieldEvent::Started(aa_net::battlefield::BattlefieldStart {
            zone_id: 501,
            instance_id: 9,
            now_ms: 0,
            start_ms: 160000,
        })),
    );
    sample(rt, bridge, 60000);
    notifications::advance(rt, bridge);
    dispatch(
        rt,
        bridge,
        &Event::Battlefield(BattlefieldEvent::SkillEnable(aa_net::battlefield::BattlefieldSkillEnable {
            zone_id: 501,
            instance_id: 9,
            killstreak: 0,
            skill_id: 12399,
            enabled: true,
        })),
    );
    dispatch(
        rt,
        bridge,
        &Event::Battlefield(BattlefieldEvent::Kill(aa_net::battlefield::BattlefieldKill {
            zone_id: 501,
            instance_id: 9,
            killer_id: 47,
            victim_id: 88,
            killer_corps: 1,
            victim_corps: 0,
            killer_killstreak: 3,
            killer_name: "Fixture player".into(),
            victim_name: "Opponent".into(),
            killer_corps_kills: 12,
            victim_corps_deaths: 12,
        })),
    );
    dispatch(
        rt,
        bridge,
        &Event::Battlefield(BattlefieldEvent::Point(aa_net::battlefield::BattlefieldPoint {
            zone_id: 501,
            instance_id: 9,
            corps: 1,
            points: 10,
            scores: [34, 12],
            character_name: "Fixture player".into(),
        })),
    );
    slots::refresh(&World::new(), &mut bridge.borrow_mut());
    slots::push(rt, bridge, false);
}

pub(super) fn end(rt: &UiRuntime, bridge: &Shared) {
    let member = |character_id, name: &str, kills, deaths| aa_net::battlefield::BattlefieldMemberResult {
        character_id,
        name: name.into(),
        score: 100,
        bonus: 25,
        bonus_set: if character_id == 47 { 3 } else { 16 },
        killstreak: 3,
        kills,
        assists: 2,
        deaths,
    };
    dispatch(
        rt,
        bridge,
        &Event::Battlefield(BattlefieldEvent::Ended(aa_net::battlefield::BattlefieldEnd {
            zone_id: 501,
            instance_id: 9,
            reason: 1,
            teams: [
                aa_net::battlefield::BattlefieldTeamResult {
                    state: 2,
                    faction_id: 159,
                    score: 34,
                    total_kills: 34,
                    total_deaths: 12,
                    members: vec![member(88, "Opponent", 34, 12)],
                },
                aa_net::battlefield::BattlefieldTeamResult {
                    state: 1,
                    faction_id: 160,
                    score: 12,
                    total_kills: 12,
                    total_deaths: 34,
                    members: vec![member(47, "Fixture player", 12, 34)],
                },
            ],
        })),
    );
}

pub(crate) fn run() {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    let (rt, bridge, path) = runtime(width as f32, height as f32).unwrap();
    let playing = std::env::args().any(|arg| arg == "--playing");
    let results = std::env::args().any(|arg| arg == "--results");
    if playing || results {
        join(&rt, &bridge).unwrap();
        play(&rt, &bridge);
        if results {
            end(&rt, &bridge);
            rt.lua
                .load("ShowScoreBoard(true,false);assert(battlefield.scoreboard:IsVisible())")
                .exec()
                .unwrap();
        }
    } else {
        let invited = std::env::args().any(|arg| arg == "--invited");
        let queued = std::env::args().any(|arg| arg == "--queued");
        prepare(&rt, &bridge, queued, invited).unwrap();
    }
    App::new()
        .add_plugins((aa_dev::default_plugins("aa-client: original battlefield entrance"), aa_dev::DevPlugin))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(crate::login::host::ui_config()),
        })
        .insert_non_send(rt)
        .insert_non_send(HudBridge(bridge))
        .add_systems(Startup, physical_pixels)
        .run();
    if path.exists() {
        std::fs::remove_file(path).unwrap();
    }
}

fn physical_pixels(mut windows: Query<&mut Window, With<PrimaryWindow>>) {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    for mut window in &mut windows {
        window.resolution.set_scale_factor_override(Some(1.));
        window.resolution.set_physical_resolution(width, height);
    }
}
