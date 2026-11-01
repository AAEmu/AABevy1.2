//! Original mode-bar Lua over explicit piano and memorized-score receipts, offline.
use bevy::prelude::*;

pub(in crate::hud) fn run() {
    let (rt, state, path) = crate::hud::user_music::fixture::runtime().unwrap();
    let before = std::env::args().any(|a| a == "--before");
    if before {
        // Explicit control of the former unimplemented getter; both runs fold
        // the same real catalog buff data. This is not an old-binary capture.
        rt.lua.load("X2Unit.GetModeActionsCount=function() return 0 end;HUD.mode_slot=function() return nil end").exec().unwrap();
    }
    for (id, index) in [(5068, 9), (6010, 10)] {
        crate::hud::enchant::dispatch(
            Some(&rt),
            &state,
            &aa_net::Event::BuffCreated {
                obj_id: 100,
                buff: aa_net::buff::Buff {
                    buff_id: id,
                    index,
                    ..default()
                },
            },
            None,
        );
    }
    assert_eq!(super::actions(&state.borrow()).len(), 11);
    assert_eq!(
        rt.lua
            .load("return X2Unit:GetModeActionsCount()")
            .eval::<usize>()
            .unwrap(),
        if before { 0 } else { 11 }
    );
    aa_ui::lua::tick(&rt.lua, 16.);
    aa_ui::lua::flush(&rt.lua);
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-client: original received mode actions"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(crate::login::host::ui_config()),
        })
        .insert_non_send(rt)
        .insert_non_send(crate::hud::api::HudBridge(state))
        .run();
    if path.exists() {
        std::fs::remove_file(path).unwrap();
    }
}

/// The Farm Wagon driver bar with its two function slots, and the "Grab Machine" F prompt for a
/// second slave in front (`vehicle_bar` example). The 1.2 rule (decompile `FUN_39204500`,
/// `FUN_393bbb80`): a slave seat's bar appends `unbind` on any seat and `showSlaveInfo` on the
/// driver seat; a slave in front of the player and not the one ridden is still a candidate, so
/// both are visible in this one frame.
pub(in crate::hud) fn run_vehicle_bar() {
    use aa_net::model::{Unit, UnitKind};
    use aa_net::{Event, Pos};
    use bevy::prelude::*;

    fn cart(obj_id: u32, tl_id: u16, name: &str, at: Vec3) -> Unit {
        let mut unit = crate::hud::expansion::fixture::spawn().unit;
        unit.obj_id = obj_id;
        unit.name = name.into();
        unit.kind = UnitKind::Slave {
            id: 700,
            tl_id,
            template_id: 60,
            summoner_id: 100,
        };
        unit.model_id = 1008;
        unit.pos = Pos::new(at.x, at.y, at.z);
        unit
    }

    let (width, height) = aa_dev::DevArgs::from_env().size;
    let (rt, state, path) =
        crate::hud::breath::fixture::runtime(width as f32, height as f32).unwrap();
    let own = crate::hud::expansion::fixture::spawn();
    crate::hud::enchant::dispatch(
        Some(&rt),
        &state,
        &Event::CharacterSelected(Box::new(own.state.clone())),
        None,
    );
    crate::hud::enchant::dispatch(Some(&rt), &state, &Event::PlayerSpawn(Box::new(own)), None);
    // The driven wagon, then a second one 2 m north (in front of its yaw 0). The HUD's own
    // position and yaw are the stale boarding point: the candidate must be measured from the
    // attached wagon (`player_centre`) and its current facing.
    let driven = cart(20, 8, "Farm Wagon", Vec3::new(10.0, 10.0, 0.0));
    let ahead = cart(21, 9, "Farm Wagon", Vec3::new(10.0, 12.0, 0.0));
    let mut units = aa_net::state::Units::default();
    for unit in [driven, ahead] {
        units.by_id.insert(unit.obj_id, unit.clone());
        crate::hud::enchant::dispatch(Some(&rt), &state, &Event::UnitSpawned(Box::new(unit)), None);
    }
    crate::hud::enchant::dispatch(
        Some(&rt),
        &state,
        &Event::UnitAttached {
            obj_id: 100,
            mount: 20,
            point: 1,
            reason: 1,
        },
        None,
    );
    {
        let mut data = state.borrow_mut();
        data.pos = Vec3::new(10.0, 20.0, 0.0);
        data.yaw = std::f32::consts::PI;
    }
    let candidate = crate::hud::slave::candidate(&state.borrow(), &units)
        .expect("the second wagon 2 m ahead of the driven one is a candidate");
    assert_eq!(candidate.obj_id, 21);
    assert_eq!(candidate.skills, vec![12076]);
    assert_eq!(
        candidate.dist, 2.0,
        "measured from the driven wagon, not the stale HUD position"
    );
    assert_eq!(
        candidate.key, 0.0,
        "the driven wagon's current yaw faces it (stale yaw would give 2.0)"
    );
    {
        let mut data = state.borrow_mut();
        data.dynamic_actions
            .replace(Some(crate::hud::dynamic_actions::Shown {
                source: crate::hud::dynamic_actions::Source::Npc {
                    obj_id: candidate.obj_id,
                    template: candidate.template,
                },
                skills: candidate.skills,
            }));
    }
    rt.fire_event("DYNAMIC_ACTION_BAR_SHOW", ("npc", candidate.obj_id));
    rt.lua
        .load(
            r#"
        assert(X2Unit:GetModeActionsCount()==7, 'five slave skills and two functions')
        local c6=AAUI.SlotContent(ISLOT_MODE_ACTION,6)
        assert(c6.type=='function' and c6.icon=='ui/icon/icon_skill_vehicle03.dds')
        assert(c6.tooltip.tootipText==X2Locale:LocalizeUiText(33,'unbind'))
        local c7=AAUI.SlotContent(ISLOT_MODE_ACTION,7)
        assert(c7.type=='function' and c7.icon=='ui/icon/icon_skill_info_view.dds')
        assert(c7.tooltip.tootipText==X2Locale:LocalizeUiText(33,'showSlaveInfo'))
        assert(dynamic_action_bar:IsVisible(), 'the Grab Machine prompt')
        assert(X2:GetDynamicActionCount()==1)
        assert(X2:GetDynamicActionSkill(1)==12076)
    "#,
        )
        .exec()
        .unwrap();
    aa_ui::lua::tick(&rt.lua, 16.);
    aa_ui::lua::flush(&rt.lua);
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-client: vehicle mode bar and Grab Machine prompt"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(crate::login::host::ui_config()),
        })
        .insert_non_send(rt)
        .insert_non_send(crate::hud::api::HudBridge(state))
        .run();
    if path.exists() {
        std::fs::remove_file(path).unwrap();
    }
}
