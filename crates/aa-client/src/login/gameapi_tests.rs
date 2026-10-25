use super::*;
use aa_net::state::{Characters, Worlds};
use aa_net::{AccountPayment, Event, Phase, PremiumPoints};

#[test]
fn lobby_headers_preserve_unknown_zero_identity_and_connection_lifetime() {
    let worlds = Worlds {
        account: Some(AccountInfo {
            max_characters: 6,
            extra_characters_per_world: 2,
            ..default()
        }),
        ..default()
    };
    let mut characters = Characters::default();
    characters.list.push(Character {
        id: 47,
        point: 1234,
        ..default()
    });
    let mut bridge = Bridge::default();
    bridge.sync(&worlds, &characters);
    assert_eq!((bridge.extra_slots, bridge.payment, bridge.premium_points), (None, None, None));

    let payment = AccountPayment {
        pay_method: -7,
        pay_location: 11,
        pay_start: -42,
        pay_end: 9000,
        real_pay_time: i64::MIN,
    };
    let premium = PremiumPoints {
        point: -3,
        old_grade: 0,
        grade: 255,
    };
    for event in [
        Event::CharacterSlotCount(0),
        Event::AccountPayment(payment),
        Event::PremiumPoints(premium),
    ] {
        characters.apply(&event);
    }
    bridge.sync(&worlds, &characters);
    assert_eq!(
        (bridge.extra_slots, bridge.payment, bridge.premium_points),
        (Some(0), Some(payment), Some(premium))
    );
    assert_eq!(bridge.account, worlds.account, "game headers do not replace login account limits");
    assert_eq!(bridge.characters[0].point, 1234, "account premium is not a character's point value");

    characters.apply(&Event::CharacterList(vec![Character {
        id: 48,
        point: 5678,
        ..default()
    }]));
    characters.apply(&Event::LeftWorld { target: 1 });
    bridge.sync(&worlds, &characters);
    assert_eq!(bridge.extra_slots, Some(0));
    assert_eq!(bridge.premium_points, Some(premium));
    assert_eq!(bridge.characters[0].point, 5678);

    characters.apply(&Event::PhaseChanged(Phase::ReadyToConnect));
    bridge.sync(&worlds, &characters);
    assert_eq!((bridge.extra_slots, bridge.payment, bridge.premium_points), (None, None, None));
    characters.apply(&Event::CharacterSlotCount(3));
    bridge.sync(&worlds, &characters);
    assert_eq!(bridge.extra_slots, Some(3));
    characters.apply(&Event::Disconnected { message: "fixture".into() });
    bridge.sync(&worlds, &characters);
    assert_eq!((bridge.extra_slots, bridge.payment, bridge.premium_points), (None, None, None));
}

#[test]
fn original_slot_getters_use_received_extras_without_overflowing_or_changing_the_expandable_maximum() -> Result<(), Box<dyn std::error::Error>> {
    let rt = UiRuntime::new("en_us", 1280.0, 720.0)?;
    let shared = new_shared();
    let worlds = Worlds {
        account: Some(AccountInfo {
            max_characters: 255,
            extra_characters_per_world: 1000,
            ..default()
        }),
        ..default()
    };
    let mut characters = Characters::default();
    shared.borrow_mut().sync(&worlds, &characters);
    install(&rt, &shared)?;
    rt.lua.load("assert(NET.account().expanded_characters == nil); assert(X2World:GetExpandedCharacterCount() == 0); assert(X2World:GetCurrentTotalCharactersLimit() == 255)").exec()?;

    characters.apply(&Event::CharacterSlotCount(255));
    shared.borrow_mut().sync(&worlds, &characters);
    rt.lua.load("assert(NET.account().expanded_characters == 255); assert(X2World:GetExpandedCharacterCount() == 255); assert(X2World:GetCurrentTotalCharactersLimit() == 510); assert(X2World:GetTotalCharactersDefaultLimit() == 255); assert(X2World:GetTotalCharactersExpandableLimit() == 1000)").exec()?;

    characters.apply(&Event::CharacterSlotCount(0));
    shared.borrow_mut().sync(&worlds, &characters);
    rt.lua.load("assert(NET.account().expanded_characters == 0); assert(X2World:GetExpandedCharacterCount() == 0); assert(X2World:GetCurrentTotalCharactersLimit() == 255)").exec()?;
    characters.apply(&Event::PhaseChanged(Phase::ReadyToConnect));
    shared.borrow_mut().sync(&worlds, &characters);
    rt.lua
        .load("assert(NET.account().expanded_characters == nil); assert(X2World:GetCurrentTotalCharactersLimit() == 255)")
        .exec()?;
    Ok(())
}
