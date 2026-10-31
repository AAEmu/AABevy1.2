//! Received 0x154 effects use the actual effect table and immediate Lua route.
use super::*;
use crate::hud::expansion::fixture::spawn;

fn fixture() -> Result<(UiRuntime, Shared, Units, Player), Box<dyn std::error::Error>> {
    let (rt, shared, path) = super::super::breath::fixture::runtime(1280., 720.)?;
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    rt.exec(
        "@native_chat_bubble_observer",
        r#"
        nativeLines={}
        local w=UIParent:CreateWidget('emptywidget','nativeChatBubbleObserver','UIParent')
        w:RegisterEvent('CHAT_MSG_QUEST')
        w:SetHandler('OnEvent',function(_,event,...) nativeLines[#nativeLines+1]={n=select('#',...),...} end)
        "#,
    )?;
    {
        let mut data = shared.borrow_mut();
        data.speech.pos.insert(200, [640., 300.]);
        data.speech.info.insert(200, ("character", "friendly"));
    }
    let mut units = Units::default();
    let mut ann = spawn().unit;
    ann.obj_id = 200;
    ann.name = "Ann".into();
    ann.kind = UnitKind::Character { char_id: 48 };
    units.by_id.insert(200, ann);
    Ok((rt, shared, units, Player { spawn: Some(spawn()) }))
}

fn receive(rt: &UiRuntime, shared: &Shared, units: &Units, player: &Player, body: BubbleBody) {
    let how = if matches!(body, BubbleBody::Text(_)) { 1 } else { 2 };
    let _ = dispatch(
        rt,
        shared,
        &Event::Emote(EmoteEvent::ChatBubble {
            obj_id: 200,
            kind: 1,
            how,
            body,
        }),
        units,
        player,
        None,
        true,
    );
}

#[test]
fn received_ids_select_actual_bubble_effect_speech() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, shared, units, player) = fixture()?;
    receive(&rt, &shared, &units, &player, BubbleBody::Id(1));
    receive(&rt, &shared, &units, &player, BubbleBody::Id(3));
    // Literal localized bubble_effects rows, independently selected from the
    // read-only server database. These are not bubbles/bubble_chats groups.
    rt.lua
        .load(
            r#"
      assert(#nativeLines==2)
      assert(nativeLines[1][1]=='You sure your name isn\'t "Moneybags"?')
      assert(nativeLines[2][1]=='Do you smell something... fishy?')
      assert(nativeLines[1][2]=='Ann' and nativeLines[1][3]=='c8')
      assert(chatBubbleFrame:IsExistSpeakerId('c8'))
    "#,
        )
        .exec()?;
    Ok(())
}

#[test]
fn a_conversation_group_is_not_an_effect_id() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, shared, units, player) = fixture()?;
    // Group4 has the old two-row conversation10 ->11, but bubble_effects has
    // no row4. Native effect lookup must not start that unrelated dialogue.
    receive(&rt, &shared, &units, &player, BubbleBody::Id(4));
    rt.lua
        .load("assert(#nativeLines==0);assert(not chatBubbleFrame:IsExistSpeakerId('c8'))")
        .exec()?;
    Ok(())
}

#[test]
fn direct_text_uses_the_native_six_argument_event_and_short_text_minimum() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, shared, units, player) = fixture()?;
    receive(&rt, &shared, &units, &player, BubbleBody::Text("Hello".into()));
    rt.lua.load("assert(#nativeLines==1 and nativeLines[1].n==6);assert(nativeLines[1][1]=='Hello' and nativeLines[1][4]==1 and nativeLines[1][5]==500);assert(nativeLines[1][7]==nil)").exec()?;
    Ok(())
}

fn scheduled() -> Result<(App, Entity, Entity), Box<dyn std::error::Error>> {
    use crate::ingame::{player::LocalPlayer, units::Views};
    let (rt, shared, units, player) = fixture()?;
    rt.exec(
        "@native_alarm_observer",
        "nativeAlarms={}; UIParent:SetEventHandler('CHAT_MSG_ALARM',function(text) nativeAlarms[#nativeAlarms+1]=text end)",
    )?;
    let mut app = App::new();
    app.add_plugins(bevy::transform::TransformPlugin)
        .add_message::<NetEvent>()
        .init_resource::<EffectVoice>()
        .insert_resource(units)
        .insert_resource(player)
        .insert_resource(super::super::HudBuilt { up: true, ..default() })
        .insert_non_send(rt)
        .insert_non_send(HudBridge(shared));
    let own = app.world_mut().spawn((LocalPlayer { obj_id: 100 }, Transform::default())).id();
    let remote = app.world_mut().spawn(Transform::from_xyz(29.0, 0.0, 0.0)).id();
    let mut views = Views::default();
    let unit = app.world().resource::<Units>().get(200).unwrap();
    views.fixture_actor(unit, Some(remote));
    app.insert_resource(views).add_systems(Update, track);
    Ok((app, own, remote))
}

fn scheduled_bubble(app: &mut App, id: u32, kind: u8, body: BubbleBody) {
    app.world_mut().write_message(NetEvent(Event::Emote(EmoteEvent::ChatBubble {
        obj_id: id,
        kind,
        how: 1,
        body,
    })));
    app.update();
}

fn counts(app: &App) -> (usize, usize) {
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("return #nativeLines,#nativeAlarms")
        .eval()
        .unwrap()
}

#[test]
fn scheduled_bubbles_use_current_roots_and_exact_three_dimensional_range() -> Result<(), Box<dyn std::error::Error>> {
    let (mut app, own, remote) = scheduled()?;
    app.world_mut().resource_mut::<Units>().by_id.get_mut(&200).unwrap().pos = aa_net::Pos::new(500.0, 500.0, 500.0);
    app.world().non_send::<HudBridge>().0.borrow_mut().pos = Vec3::splat(-500.0);
    for (point, accepted) in [
        (Vec3::X * 29.0, true),
        (Vec3::X * 31.0, false),
        (Vec3::Y * 30.0, true),
        (Vec3::Y * f32::from_bits(0x41f00001), false),
        (Vec3::new(18.0, 0.0, 24.0), true),
    ] {
        app.world_mut().get_mut::<Transform>(remote).unwrap().translation = point;
        app.update();
        let before = counts(&app);
        scheduled_bubble(&mut app, 200, 1, BubbleBody::Text("Hi".into()));
        scheduled_bubble(&mut app, 200, 4, BubbleBody::Id(1));
        assert_eq!(
            counts(&app),
            (before.0 + usize::from(accepted), before.1 + usize::from(accepted)),
            "{point:?}"
        );
    }
    app.world_mut().get_mut::<Transform>(own).unwrap().translation = Vec3::splat(100.0);
    app.update();
    let before = counts(&app);
    scheduled_bubble(&mut app, 200, 1, BubbleBody::Id(3));
    assert_eq!(counts(&app), before, "own root changes are used without a HUD refresh");
    app.world_mut().despawn(own);
    scheduled_bubble(&mut app, 200, 1, BubbleBody::Text("No own actor".into()));
    assert_eq!(counts(&app), before);
    Ok(())
}

#[test]
fn pending_actor_and_own_speaker_do_not_require_projection_or_rendering() -> Result<(), Box<dyn std::error::Error>> {
    use crate::ingame::{player::LocalPlayer, units::Views};
    let (mut app, own, remote) = scheduled()?;
    app.world_mut().despawn(remote);
    let mut unit = app.world().resource::<Units>().get(200).unwrap().clone();
    unit.pos = aa_net::Pos::new(29.0, 0.0, 0.0);
    app.world_mut().resource_mut::<Units>().by_id.insert(200, unit.clone());
    app.world_mut().resource_mut::<Views>().fixture_actor(&unit, None);
    app.world().non_send::<HudBridge>().0.borrow_mut().speech.pos.clear();
    scheduled_bubble(&mut app, 200, 1, BubbleBody::Id(1));
    assert_eq!(
        counts(&app),
        (1, 0),
        "logical actor is admitted even without a projected bubble anchor"
    );
    // A wrong cached HUD entity is not the current own-actor supplier.
    let wrong = app
        .world_mut()
        .spawn((LocalPlayer { obj_id: 999 }, Transform::from_xyz(500.0, 0.0, 0.0)))
        .id();
    app.world().non_send::<HudBridge>().0.borrow_mut().player_entity = Some(wrong);
    scheduled_bubble(&mut app, 100, 1, BubbleBody::Text("Me".into()));
    assert_eq!(counts(&app), (2, 0));
    app.world_mut().resource_mut::<Units>().by_id.remove(&200);
    scheduled_bubble(&mut app, 200, 1, BubbleBody::Text("Retired".into()));
    assert_eq!(counts(&app), (2, 0));
    app.world_mut().despawn(own);
    scheduled_bubble(&mut app, 100, 1, BubbleBody::Text("Wrong id root only".into()));
    assert_eq!(counts(&app), (2, 0));
    Ok(())
}

#[test]
fn hard_retirement_persists_until_a_new_own_spawn_without_retiring_on_hud_suspension() -> Result<(), Box<dyn std::error::Error>> {
    let (mut app, _, _) = scheduled()?;
    app.world_mut().resource_mut::<super::super::HudBuilt>().up = false;
    app.update();
    app.world_mut().resource_mut::<super::super::HudBuilt>().up = true;
    scheduled_bubble(&mut app, 200, 1, BubbleBody::Text("After temporary loading".into()));
    assert_eq!(counts(&app), (1, 0));
    app.world_mut()
        .write_message(NetEvent(Event::Disconnected { message: "fixture".into() }));
    app.update();
    scheduled_bubble(&mut app, 200, 1, BubbleBody::Text("Late".into()));
    assert_eq!(counts(&app), (1, 0));
    app.world_mut().write_message(NetEvent(Event::PlayerSpawn(Box::new(spawn()))));
    scheduled_bubble(&mut app, 200, 1, BubbleBody::Text("New spawn".into()));
    assert_eq!(counts(&app), (2, 0));
    Ok(())
}

#[test]
fn scheduled_id_speech_uses_voice_pack_hits_after_lua_and_reaps_exact_request_handles() -> Result<(), Box<dyn std::error::Error>> {
    let (mut app, _, remote) = scheduled()?;
    app.insert_resource(aa_audio::Engine::start(aa_audio::Settings { audible: false, ..default() }))
        .insert_resource(crate::sound::Packs::fixture(aa_audio::SoundPacks::open(aa_anim::db::DEFAULT_DB)?));
    // Actual original rows1/3 deliver speech but miss the native voice cache.
    for id in [1, 3, 4] {
        scheduled_bubble(&mut app, 200, 1, BubbleBody::Id(id));
        assert_eq!(app.world().resource::<EffectVoice>().0, None);
    }
    let before = counts(&app);
    scheduled_bubble(&mut app, 200, 1, BubbleBody::Id(969));
    assert_eq!(counts(&app).0, before.0 + 1, "the original Lua delivery still occurs");
    let first = app.world().resource::<EffectVoice>().0.expect("cache hit issues a request");
    assert!(!app.world().resource::<aa_audio::Engine>().is_playing(first), "muted fixture has no playback backend");
    scheduled_bubble(&mut app, 200, 4, BubbleBody::Id(969));
    let second = app.world().resource::<EffectVoice>().0.unwrap();
    assert_ne!(first, second, "accepted alarm-route effects invoke the hook too");
    app.update();
    assert_eq!(app.world().resource::<EffectVoice>().0, None, "poll exact handle, without consuming completion events");
    app.world_mut().get_mut::<Transform>(remote).unwrap().translation.x = 31.0;
    app.update();
    scheduled_bubble(&mut app, 200, 1, BubbleBody::Id(969));
    assert_eq!(app.world().resource::<EffectVoice>().0, None);
    app.world_mut().get_mut::<Transform>(remote).unwrap().translation.x = 29.0;
    app.update();
    scheduled_bubble(&mut app, 200, 1, BubbleBody::Text("direct text has no effect key".into()));
    assert_eq!(app.world().resource::<EffectVoice>().0, None);
    Ok(())
}
