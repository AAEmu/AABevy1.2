use super::*;
use crate::hud::expansion::fixture::spawn;
use aa_net::model::UnitKind;

fn setup() -> Result<(UiRuntime, Shared), Box<dyn std::error::Error>> {
    let (rt, state, _) = super::super::breath::fixture::runtime(1280., 720.)?;
    rt.lua
        .load(
            r#"
        said = {}
        local listener = UIParent:CreateWidget('emptywidget', 'emoteListener', 'UIParent')
        listener:SetHandler('OnEvent', function(self, event, text) table.insert(said, text) end)
        listener:RegisterEvent('CHAT_EMOTION')
    "#,
        )
        .exec()?;
    {
        let mut d = state.borrow_mut();
        d.speech.pos.insert(200, [640.0, 300.0]);
        d.speech.info.insert(200, ("character", "friendly"));
    }
    Ok((rt, state))
}

fn world() -> (Units, Player) {
    let mut units = Units::default();
    let mut ann = spawn().unit;
    ann.obj_id = 200;
    ann.name = "Ann".into();
    ann.kind = UnitKind::Character { char_id: 48 };
    units.by_id.insert(200, ann);
    (units, Player { spawn: Some(spawn()) })
}

fn emotes() -> Emotes {
    let conn = Connection::open_with_flags(aa_anim::db::DEFAULT_DB, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    Emotes::load(&conn).unwrap()
}

fn send(rt: &UiRuntime, state: &Shared, event: EmoteEvent) {
    let (units, player) = world();
    dispatch(rt, state, &Event::Emote(event), &units, &player, Some(&emotes()), true);
}

#[test]
fn say_chat_of_a_unit_in_range_is_a_bubble_over_it() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, _state) = setup()?;
    rt.fire_event("CHAT_MESSAGE", (0i64, "c8".to_string(), false, "Ann".to_string(), "hello there".to_string(), true, None::<String>));
    rt.lua.load("assert(chatBubbleFrame:IsExistSpeakerId('c8'))").exec()?;
    // a speaker the client has not got (out of range) draws nothing
    rt.fire_event("CHAT_MESSAGE", (0i64, "c9".to_string(), false, "Bob".to_string(), "who am i".to_string(), false, None::<String>));
    rt.lua.load("assert(not chatBubbleFrame:IsExistSpeakerId('c9'))").exec()?;
    // whispers and the shout channel never get one
    rt.fire_event("CHAT_MESSAGE", (-3i64, "ca".to_string(), false, "Cy".to_string(), "psst".to_string(), true, None::<String>));
    rt.fire_event("CHAT_MESSAGE", (1i64, "cb".to_string(), false, "Di".to_string(), "hey".to_string(), true, None::<String>));
    rt.lua.load("assert(not chatBubbleFrame:IsExistSpeakerId('ca') and not chatBubbleFrame:IsExistSpeakerId('cb'))").exec()?;
    Ok(())
}

#[test]
fn the_script_finds_its_speaker_on_the_screen() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, _state) = setup()?;
    rt.lua
        .load(
            r#"
        local x, y, z = X2Unit:GetUnitScreenNameTagOffset('c8')
        assert(x == 640 and y == 300 and z == 1, tostring(x))
        -- a unit that is not on the screen: no position, the bubble hides
        assert(X2Unit:GetUnitScreenNameTagOffset('3e7') == nil)
        local info = X2Unit:GetUnitInfoById('c8')
        assert(info.type == 'character' and info.faction == 'friendly')
        assert(X2Unit:GetUnitInfoById('3e7') == nil)
    "#,
        )
        .exec()?;
    Ok(())
}

#[test]
fn a_server_bubble_with_text_is_said_over_the_unit() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state) = setup()?;
    send(&rt, &state, EmoteEvent::ChatBubble { obj_id: 200, kind: 1, how: 1, body: BubbleBody::Text("Over here!".into()) });
    rt.lua.load("assert(chatBubbleFrame:IsExistSpeakerId('c8'))").exec()?;
    // a bubble of an id nobody knows says nothing
    send(&rt, &state, EmoteEvent::ChatBubble { obj_id: 201, kind: 2, how: 2, body: BubbleBody::Id(999_999) });
    rt.lua.load("assert(not chatBubbleFrame:IsExistSpeakerId('c9'))").exec()?;
    Ok(())
}

#[test]
fn the_emote_line_is_printed_for_whoever_reads_it() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state) = setup()?;
    let e = emotes();
    // /clap
    let emotion = e.command("clap").expect("the database has /clap");
    send(&rt, &state, EmoteEvent::Expressed { obj_id: 200, target_obj_id: 0, emotion_id: emotion });
    send(&rt, &state, EmoteEvent::Expressed { obj_id: 100, target_obj_id: 0, emotion_id: emotion });
    let mine = fill(e.line(emotion, true, false, false), "", "");
    let hers = fill(e.line(emotion, false, false, false), "Ann", "");
    assert!(!mine.is_empty() && !hers.is_empty() && hers.starts_with("Ann "), "{mine:?} {hers:?}");
    rt.lua.load(&format!("assert(#said == 2 and said[1] == {hers:?} and said[2] == {mine:?}, table.concat(said, '|'))")).exec()?;
    // an emotion the database does not know prints nothing
    send(&rt, &state, EmoteEvent::Expressed { obj_id: 200, target_obj_id: 0, emotion_id: 99_999 });
    rt.lua.load("assert(#said == 2)").exec()?;
    Ok(())
}

#[test]
fn effect_speech_uses_its_own_locale_entries_and_native_raw_fallback() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE bubble_effects (id INT, kind_id INT, speech TEXT);
         CREATE TABLE localized_texts (idx INT, tbl_name TEXT, tbl_column_name TEXT, en_us TEXT);
         INSERT INTO bubble_effects VALUES (1, 2, 'Raw one'), (3, 1, 'Raw three'), (5, 1, 'Hidden'), (6, 1, NULL);
         INSERT INTO localized_texts VALUES (1, 'bubble_effects', 'speech', 'Localized one'), (5, 'bubble_effects', 'speech', ''),
             (3, 'bubble_chats', 'speech', 'Unrelated conversation');",
    ).unwrap();
    let t = Tables::load(&conn).unwrap();
    assert_eq!(t.text(1), Some("Localized one"));
    assert_eq!(t.text(3), Some("Raw three"));
    assert_eq!(t.text(5), Some("Hidden"));
    assert_eq!(t.text(6), Some(""));
    assert!(t.text(4).is_none());
}

#[test]
fn direct_native_timing_retains_sse_fractional_fade() {
    assert_eq!(timing("A").0, 500.0);
    assert_eq!(timing("A").1.to_bits(), 0x43963211);
    assert_eq!(timing("So King Peredin was weaker than King Solzrean?"), (4094.0, f32::from_bits(0x439eff00)));
}

#[test]
fn a_typed_slash_command_names_an_emote() {
    use crate::hud::api::emote_command;
    let e = emotes();
    let clap = e.command("clap");
    assert!(clap.is_some());
    assert_eq!(emote_command(Some(&e), "/clap"), clap);
    assert_eq!(emote_command(Some(&e), "  /Clap  at you "), clap);
    // not a command, not an emote, no table
    assert_eq!(emote_command(Some(&e), "clap"), None);
    assert_eq!(emote_command(Some(&e), "/nosuchemote"), None);
    assert_eq!(emote_command(Some(&e), "/"), None);
    assert_eq!(emote_command(None, "/clap"), None);
}


#[test]
fn legacy_unit_info_keeps_owner_fields_and_received_name_colour_metadata() -> Result<(), Box<dyn std::error::Error>> {
    let rt = UiRuntime::new("en_us", 1280.0, 720.0)?;
    let state: Shared = std::rc::Rc::new(std::cell::RefCell::new(super::super::api::HudData::default()));
    // The existing provider's private decimal256 collides with native100.
    rt.lua.load("X2Unit={GetUnitInfoById=function(_,id) if id=='256' then return {owner_name='Keeper'} end end}").exec()?;
    state.borrow_mut().speech.info.insert(256, ("npc", "friendly"));
    install(&rt.lua, &state)?;
    rt.lua.load("local i=X2Unit:GetUnitInfoById('100');assert(i.owner_name=='Keeper' and i.type=='npc' and i.faction=='friendly');assert(X2Unit:GetUnitInfoById(100)==nil)").exec()?;
    Ok(())
}
