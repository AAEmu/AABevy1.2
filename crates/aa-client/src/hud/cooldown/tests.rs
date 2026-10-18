use super::*;
use aa_net::state::ReceivedCooldown;
use std::cell::RefCell;
use std::rc::Rc;

fn game() -> GameData {
    let mut game = GameData::default();
    game.skills.insert(
        7,
        SkillInfo {
            tag: 999,
            tags: vec![20, 10, 20],
            cooldown_ms: 30_000,
            ..Default::default()
        },
    );
    game.skills.insert(
        8,
        SkillInfo {
            gcd_ms: 5000,
            ..Default::default()
        },
    );
    game
}

fn entry(id: u32, remaining_ms: u32, total_ms: u32, at: Instant) -> ReceivedCooldown {
    ReceivedCooldown {
        cooldown: aa_net::Cooldown { id, remaining_ms, total_ms },
        received_at: at,
        reset: false,
    }
}

#[test]
fn every_tag_and_raw_skill_use_native_order_and_unsigned_strict_greater() {
    let game = game();
    let now = Instant::now();
    let mut skills = Skills {
        last_cast: Some(now),
        last_cast_skill: 8,
        ..Default::default()
    };
    skills.received_tag_cooldowns = vec![entry(10, 5000, 11_000, now), entry(20, 5000, 22_000, now), entry(999, 60_000, 60_000, now)];
    skills.received_skill_cooldowns.push(entry(7, 5000, 33_000, now));
    // Equal remaining preserves global before tags and raw IDs, regardless of total.
    assert_eq!(combined_at(&game, &skills, 7, now), Some((5000, 5000)));
    skills.last_cast = None;
    assert_eq!(combined_at(&game, &skills, 7, now), Some((5000, 22_000)));
    skills.received_tag_cooldowns[0].cooldown.remaining_ms = 6000;
    assert_eq!(combined_at(&game, &skills, 7, now), Some((6000, 11_000)));
    skills.received_skill_cooldowns[0].cooldown.remaining_ms = u32::MAX;
    skills.received_skill_cooldowns[0].cooldown.total_ms = 0;
    assert_eq!(combined_at(&game, &skills, 7, now), Some((u32::MAX, 0)));
    // A reset keeps the original row but removes its timer from selection.
    skills.received_skill_cooldowns[0].reset = true;
    assert_eq!(combined_at(&game, &skills, 7, now), Some((6000, 11_000)));
    assert_eq!(
        observed_at(game.skills.get(&7).unwrap(), &skills, 7, now),
        (0, 0),
        "the estimate must not query a nonmember tag"
    );
    assert_eq!(combined_at(&game, &skills, 7, now + Duration::from_secs(7)), Some((0, 0)));
}

#[test]
fn global_exemption_and_clock_sampling_preserve_the_winning_duration() {
    let mut game = game();
    let now = Instant::now();
    let mut skills = Skills {
        last_cast: Some(now),
        last_cast_skill: 8,
        ..Default::default()
    };
    skills.received_tag_cooldowns.push(entry(20, 4000, 17_000, now));
    let later = now + Duration::from_millis(1234);
    assert_eq!(combined_at(&game, &skills, 7, later), Some((3766, 5000)));
    game.skills.get_mut(&7).unwrap().ignore_gcd = true;
    assert_eq!(combined_at(&game, &skills, 7, later), Some((2766, 17_000)));
    // Learned ownership does not enter native descriptor cooldown selection.
    assert!(skills.known.is_empty());
    assert_eq!(combined_at(&game, &skills, 123, later), None);
}

#[test]
fn own_cast_estimate_stays_separate_and_respects_its_existing_reset() {
    let game = game();
    let now = Instant::now();
    let mut skills = Skills::default();
    skills.last_used.insert(7, now);
    assert_eq!(combined_at(&game, &skills, 7, now), Some((0, 0)));
    assert_eq!(display_at(&game, &skills, 7, now), Some((30_000, 30_000)));
    skills.reset_tags.push((999, now));
    assert_eq!(display_at(&game, &skills, 7, now), Some((0, 0)));
}

#[test]
fn lua_hook_distinguishes_missing_descriptor_from_ready_or_missing_actor() -> Result<(), Box<dyn std::error::Error>> {
    let rt = aa_ui::UiRuntime::new("en_us", 1920., 1080.)?;
    let bridge = Rc::new(RefCell::new(super::super::api::HudData::default()));
    assert!(bridge.borrow().game.0.set(game()).is_ok());
    install(&rt.lua, &bridge)?;
    rt.lua
        .load(
            r#"
      assert(select('#',AAUI.SkillCooldown(123))==0)
      local a,b=AAUI.SkillCooldown(7);assert(a==0 and b==0)
    "#,
        )
        .exec()?;
    {
        let mut data = bridge.borrow_mut();
        data.cooldowns.obj_id = 100;
        data.cooldowns.received_tag_cooldowns.push(entry(20, 8000, 12_000, Instant::now()));
    }
    rt.lua.load("local a,b=AAUI.SkillCooldown(7);assert(a>7900 and a<=8000 and b==12000)").exec()?;
    Ok(())
}

#[test]
#[ignore = "loads the complete original HUD and its actual equipped-item update/cooldown handlers"]
fn original_equipment_handler_reads_received_cooldowns_without_changing_its_icon() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, bridge, path) = super::fixture::runtime(1920., 1080.)?;
    let id = aa_ui::lua::value_id(&rt.lua, &rt.lua.load("equippedItem.slot[7]").eval()?).unwrap();
    let before = aa_ui::lua::ui(&rt.lua).tree.widget(id).unwrap().clone();
    assert_eq!(before.button.as_ref().unwrap().cooldown_skill.as_ref().unwrap().skill_type, 13419);
    assert!(before.slot.is_none(), "a cooldown-only equipment identity is not a clickable skill accessor");
    aa_ui::lua::tick(&rt.lua, 16.);
    {
        let ui = aa_ui::lua::ui(&rt.lua);
        let after = ui.tree.widget(id).unwrap();
        let sweep = after.cooldown.as_ref().expect("original equipped-item handler starts the received sweep");
        assert_eq!(sweep.total_ms, 30_000.);
        assert!((17_000. ..=18_000.).contains(&(sweep.end_ms - ui.time_ms)));
        assert_eq!(after.button.as_ref().unwrap().bg, before.button.as_ref().unwrap().bg);
        assert_eq!(after.slot, before.slot);
    }
    bridge.borrow_mut().cooldowns.received_tag_cooldowns[0].reset = true;
    aa_ui::lua::tick(&rt.lua, 16.);
    assert!(aa_ui::lua::ui(&rt.lua).tree.widget(id).unwrap().cooldown.is_none());
    // The other actual item's separate tag continues counting down.
    let back = aa_ui::lua::value_id(&rt.lua, &rt.lua.load("equippedItem.slot[9]").eval()?).unwrap();
    assert!(aa_ui::lua::ui(&rt.lua).tree.widget(back).unwrap().cooldown.is_some());
    assert!(rt.errors.is_empty());
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}
