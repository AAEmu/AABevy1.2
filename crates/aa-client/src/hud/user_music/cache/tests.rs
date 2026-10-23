use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

fn directory() -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "aa-score-cache-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}
fn active(path: PathBuf) -> Cache {
    Cache {
        directory: Some(path),
        world: Some(17),
        owner: 47,
        actor: 7,
        ..Default::default()
    }
}
fn note(id: u32, title: &str, notes: &str) -> UserNote {
    UserNote {
        song_id: id,
        tooltip: true,
        title: title.into(),
        notes: notes.into(),
    }
}
fn request(id: u64, tooltip: bool, buff: bool) -> (u32, MusicCommand) {
    (
        47,
        MusicCommand::RequestNotes {
            note_id: id,
            is_tooltip: tooltip,
            show_buff_only: buff,
        },
    )
}

#[test]
fn native_cache_miss_hit_flags_deduplicate_and_restart_without_automatic_play() {
    let path = directory();
    let mut cache = active(path.clone());
    assert_eq!(cache.lookup(42, true), None);
    assert_eq!(cache.lookup(42, false), None);
    assert_eq!(cache.drain(), [request(42, true, false)]);
    cache.apply(&Event::Notice(NoticeEvent::UserNoteLoaded(None)));
    assert!(
        cache.pending.contains(&42),
        "an empty reply carries no correlation ID"
    );
    cache.receive(&note(42, "Harbor", "O5 C D E"));
    assert!(
        cache.drain().is_empty(),
        "note receipt is not a play callback"
    );
    assert_eq!(cache.notes, "O5 C D E");
    assert!(!cache.pending.contains(&42));
    let score = cache.lookup(42, true).unwrap();
    assert_eq!(score.title, "Harbor");
    assert!(cache.drain().is_empty(), "tooltip cache hits are local");
    let mut restarted = active(path.clone());
    assert_eq!(restarted.lookup(42, true), Some(score.clone()));
    assert!(restarted.drain().is_empty());
    assert_eq!(restarted.lookup(42, false), Some(score));
    assert_eq!(restarted.drain(), [request(42, false, true)]);
    assert_eq!(restarted.notes, "O5 C D E");
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn world_scope_and_full_pending_ids_preserve_native_low32_cache_aliases() {
    let path = directory();
    let mut cache = active(path.clone());
    cache.receive(&note(7, "World17", "C"));
    cache.lookup(999, false);
    cache.identity(Some(18), 47, 8);
    assert!(cache.drain().is_empty());
    assert_eq!(cache.lookup(7, false), None);
    assert_eq!(cache.drain(), [request(7, false, false)]);
    cache.apply(&Event::MusicRequestRejected {
        character_id: 48,
        command: request(7, false, false).1,
        message: "old owner".into(),
    });
    assert!(cache.pending.contains(&7));
    cache.apply(&Event::MusicRequestRejected {
        character_id: 47,
        command: request(7, false, false).1,
        message: "write failed".into(),
    });
    assert!(!cache.pending.contains(&7));
    cache.identity(Some(17), 47, 9);
    assert_eq!(cache.lookup(7, true).unwrap().title, "World17");
    let unsigned = 0x8000_0000_u64;
    cache.lookup(unsigned, false);
    cache.drain();
    cache.receive(&note(unsigned as u32, "Signed key", "D"));
    assert!(
        cache.pending.contains(&unsigned),
        "pending IDs retain the native sign-extension mismatch"
    );
    assert!(cache.lookup(0xffff_ffff_8000_0000, true).is_some());
    assert_eq!(cache.lookup(unsigned, true).unwrap().title, "Signed key");
    assert_eq!(cache.path(unsigned), cache.path(0xffff_ffff_8000_0000));
    let alias = 0xfedc_ba98_0000_0007;
    assert_eq!(cache.lookup(alias, false).unwrap().title, "World17");
    assert_eq!(cache.drain(), [request(alias, false, true)]);
    cache.deactivate();
    assert!(cache.lookup(7, false).is_none());
    assert!(cache.drain().is_empty());
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn disk_failure_and_invalid_cache_never_become_cache_hits() {
    let path = directory();
    std::fs::create_dir_all(&path).unwrap();
    let blocker = path.join("blocked");
    std::fs::write(&blocker, b"not a directory").unwrap();
    let mut cache = active(blocker);
    cache.lookup(42, true);
    cache.drain();
    cache.receive(&note(42, "Title\0ignored", "C\0ignored"));
    assert_eq!(cache.notes, "C");
    assert!(cache.last.is_none());
    assert!(cache.lookup(42, true).is_none());
    assert_eq!(cache.drain(), [request(42, true, false)]);
    let mut cache = active(path.clone());
    std::fs::write(cache.path(1).unwrap(), b"{broken").unwrap();
    assert!(cache.lookup(1, true).is_none());
    std::fs::write(
        cache.path(2).unwrap(),
        serde_json::to_vec(&serde_json::json!({"title":"x","notes":"C".repeat(12001)})).unwrap(),
    )
    .unwrap();
    assert!(cache.lookup(2, true).is_none());
    assert_eq!(c_string(&"\u{b3c4}".repeat(33), 96).len(), 96);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn received_detail_eight_keeps_u64_identity_through_lua() {
    let id = 9_007_199_254_740_993_u64;
    let mut item = aa_net::model::Item {
        template_id: 28051,
        detail: aa_net::model::ItemDetail::Other(8),
        detail_payload: Some(id.to_le_bytes().to_vec()),
        ..Default::default()
    };
    let extra = crate::hud::tips::ItemExtra::of_item(&item);
    assert_eq!(extra.music_note, Some(id));
    let lua = Lua::new();
    let slot = lua.create_table().unwrap();
    extra.write(&slot, &lua).unwrap();
    assert_eq!(slot.get::<String>("musicNote").unwrap(), id.to_string());
    assert_eq!(crate::hud::tips::ItemExtra::of(Some(&slot)), extra);
    slot.set("musicNote", id as f64).unwrap();
    assert_eq!(
        crate::hud::tips::ItemExtra::of(Some(&slot)).music_note,
        None
    );
    item.detail_payload = Some(vec![0; 7]);
    assert_eq!(crate::hud::tips::ItemExtra::of_item(&item).music_note, None);
}

#[test]
fn actual_bag_and_native_chat_link_tooltips_share_received_persistent_scores()
-> Result<(), Box<dyn std::error::Error>> {
    let (rt, state, path) = crate::hud::user_music::fixture::runtime()?;
    let shared = state.borrow().user_music.cache.clone();
    shared.borrow_mut().identity(Some(17), 47, 100);
    let item = aa_net::model::Item {
        id: 100,
        template_id: 28051,
        count: 1,
        detail: aa_net::model::ItemDetail::Other(8),
        detail_payload: Some(42_u64.to_le_bytes().to_vec()),
        ..Default::default()
    };
    let slot = rt.lua.create_table()?;
    crate::hud::tips::ItemExtra::of_item(&item).write(&slot, &rt.lua)?;
    rt.lua.globals().set("SCORE_SLOT", slot)?;
    rt.lua
        .globals()
        .set("SCORE_LINK", crate::hud::itemlinks::link(&item).unwrap())?;
    rt.lua.load("local t=HUD.item_tip(28051,1,1,SCORE_SLOT);assert(t.userMusicTitle==nil);assert(X2Item:InfoFromLink(SCORE_LINK).userMusicTitle==nil)").exec()?;
    let directory = shared.borrow().directory.clone().unwrap();
    assert_eq!(
        directory,
        path.with_extension("music"),
        "fixtures must never write the account's real cache"
    );
    assert_eq!(shared.borrow_mut().drain(), [request(42, true, false)]);
    shared
        .borrow_mut()
        .apply(&Event::Notice(NoticeEvent::UserNoteLoaded(Some(note(
            42,
            "Harbor",
            "C 도😀",
        )))));
    rt.lua.load("for _,t in ipairs({HUD.item_tip(28051,1,1,SCORE_SLOT),X2Item:InfoFromLink(SCORE_LINK)}) do assert(t.userMusicTitle=='Harbor' and t.userMusicRequireGrade==0) end").exec()?;
    assert!(shared.borrow_mut().drain().is_empty());
    assert!(state.borrow().user_music.preview.is_none());
    assert!(state.borrow().user_music.cast.is_none());
    assert!(!directory.starts_with(aa_pak::DEFAULT_PAK));
    std::fs::remove_dir_all(directory)?;
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[test]
#[ignore = "original bag/tooltip physical rehover over an isolated received score fixture"]
fn original_bag_queries_received_score_only_on_new_hover() -> Result<(), Box<dyn std::error::Error>>
{
    let (rt, state, path) = fixture::runtime()?;
    let value = rt.lua.load("newbag.slots.btns[5]").eval()?;
    let id = aa_ui::lua::value_id(&rt.lua, &value).unwrap();
    let r = aa_ui::lua::ui(&rt.lua).tree.rect(id);
    let point = (r.x + r.w / 2., r.y + r.h / 2.);
    let mut mouse = Default::default();
    aa_ui::input::update(&rt.lua, &mut mouse, Some(point), false, false, false, 0.);
    aa_ui::lua::tick(&rt.lua, 1200.);
    let before = fixture::lines(&rt);
    assert!(!before.iter().any(|s| s.contains("Harbor")), "{before:?}");
    let cache = state.borrow().user_music.cache.clone();
    cache
        .borrow_mut()
        .apply(&Event::Notice(NoticeEvent::UserNoteLoaded(Some(note(
            42, "Harbor", "CDE",
        )))));
    for _ in 0..100 {
        aa_ui::input::update(&rt.lua, &mut mouse, Some(point), false, false, false, 0.);
        aa_ui::lua::tick(&rt.lua, 16.);
    }
    assert_eq!(fixture::lines(&rt), before);
    aa_ui::input::update(
        &rt.lua,
        &mut mouse,
        Some((600., 100.)),
        false,
        false,
        false,
        0.,
    );
    aa_ui::input::update(&rt.lua, &mut mouse, Some(point), false, false, false, 0.);
    aa_ui::lua::tick(&rt.lua, 1200.);
    let after = fixture::lines(&rt);
    assert!(after.iter().any(|s| s.contains("Harbor")), "{after:?}");
    assert!(after.iter().any(|s| s.contains("Artistry")), "{after:?}");
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    assert!(state.borrow().user_music.cast.is_none());
    std::fs::remove_dir_all(path.with_extension("music"))?;
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[test]
#[ignore = "original bag and bound HUD API argument masks with a received score"]
fn explicit_bag_flags_gate_user_music_extras() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state, path) = fixture::runtime()?;
    let cache = state.borrow().user_music.cache.clone();
    cache.borrow_mut().receive(&note(42, "Harbor", "C"));
    rt.lua
        .load(
            r#"
        for _,flag in ipairs({false,true,0,1,1024}) do
            assert(X2Bag:GetBagItemInfo(0,5,flag).userMusicTitle==nil)
        end
        assert(X2Bag:GetBagItemInfo(0,5,nil).userMusicTitle==nil)
        assert(X2Bag:GetBagItemInfo(0,5).userMusicTitle=='Harbor')
        assert(X2Bag:GetBagItemInfo(0,5,-1).userMusicTitle=='Harbor')
    "#,
        )
        .exec()?;
    std::fs::remove_dir_all(path.with_extension("music"))?;
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}
