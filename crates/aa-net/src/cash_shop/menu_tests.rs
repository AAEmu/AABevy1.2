use super::*;
use crate::{
    game::{ClientMsg, ServerMsg},
    initial_config::{GameConfig, InitialConfig},
    session::Event,
    wire::Writer,
};

const NATIVE: &str = include_str!("../../tests/fixtures/ingame-shop-menu/native.tsv");

fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

pub(crate) fn fixture(result: u8) -> (Vec<u8>, NativeInGameShopMenu) {
    let columns: Vec<_> = NATIVE
        .lines()
        .filter(|l| !l.starts_with('#'))
        .map(|l| l.split('\t').collect::<Vec<_>>())
        .find(|c| c[1].parse::<u8>().unwrap() == result)
        .unwrap();
    let subs = hex(columns[3]);
    let menu = NativeInGameShopMenu {
        result,
        main_tabs: hex(columns[2]).try_into().unwrap(),
        sub_tabs: std::array::from_fn(|i| subs[7 * i..7 * i + 7].try_into().unwrap()),
    };
    (hex(columns[4]), menu)
}

pub(crate) fn dd01(opcode: u16, body: &[u8]) -> Vec<u8> {
    let mut w = Writer::new();
    w.raw(&[0xdd, 1, 0, 0]).u16(opcode).raw(body);
    w.finish()
}

pub(crate) fn dd04(packets: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let mut inner = Writer::new();
    for (opcode, body) in packets {
        inner.u16(0).u16(*opcode).raw(body);
    }
    let raw = inner.finish();
    let len = u16::try_from(raw.len()).unwrap();
    let mut w = Writer::new();
    w.raw(&[0xdd, 4])
        .u16(packets.len() as u16)
        .u8(1)
        .u16(len)
        .u16(!len)
        .raw(&raw);
    w.finish()
}

#[test]
fn ingame_shop_menu_native_goldens_preserve_raw_ids_all_prefixes_and_cursor() {
    for result in [0, 1, 2, 255] {
        let (body, menu) = fixture(result);
        assert_eq!(body.len(), 49);
        for n in 0..49 {
            let mut r = Reader::new(&body[..n]);
            assert!(
                matches!(CashShopEvent::read(op::MENU_LIST, &mut r), Err(WireError::Eof { pos, wanted: 1, len }) if pos == n && len == n),
                "result {result}, prefix {n}"
            );
            let batch = ServerMsg::decode(&dd04(&[(op::MENU_LIST, body[..n].to_vec())])).unwrap();
            assert!(
                matches!(batch, ServerMsg::Batch(ref v) if matches!(v.as_slice(), [ServerMsg::DecodeFailed { .. }]))
            );
        }
        let mut suffixed = body.clone();
        suffixed.extend([0xde, 0xad, 0xbe, 0xef]);
        let mut r = Reader::new(&suffixed);
        assert_eq!(
            CashShopEvent::read(op::MENU_LIST, &mut r).unwrap(),
            CashShopEvent::NativeMenuList(menu)
        );
        assert_eq!(r.pos(), 49);
        assert_eq!(r.rest(), [0xde, 0xad, 0xbe, 0xef]);
        assert_eq!(menu.main_tabs, [0, 17, 255, 2, 6, 1]);
        assert_eq!(
            menu.sub_tabs[0][0], 255,
            "unavailable main does not discard its raw sub IDs"
        );
        assert_eq!(menu.sub_tabs[5][6], 144);
    }
}

#[test]
fn ingame_shop_menu_all_256_result_bytes_stay_raw_instead_of_bool() {
    let (mut body, mut expected) = fixture(0);
    for result in 0..=255u8 {
        body[0] = result;
        expected.result = result;
        assert_eq!(
            CashShopEvent::read(op::MENU_LIST, &mut Reader::new(&body)).unwrap(),
            CashShopEvent::NativeMenuList(expected)
        );
        assert_eq!(
            ServerMsg::decode(&dd01(op::MENU_LIST, &body)).unwrap(),
            ServerMsg::CashShop(CashShopEvent::NativeMenuList(expected))
        );
    }
    let legacy = CashShopEvent::MenuList {
        enabled: true,
        tabs: [[false; 7]; 6],
    };
    assert_eq!(legacy.name(), "SCICSMenuList");
    assert_eq!(
        CashShopEvent::NativeMenuList(expected).name(),
        legacy.name()
    );
}

#[test]
fn ingame_shop_menu_dd04_keeps_following_packet_despite_header_looking_raw_ids() {
    for result in [0, 1, 2, 255] {
        let (body, menu) = fixture(result);
        assert_eq!(
            ServerMsg::decode(&dd04(&[(op::MENU_LIST, body), (0xb4, vec![1])])).unwrap(),
            ServerMsg::Batch(vec![
                ServerMsg::CashShop(CashShopEvent::NativeMenuList(menu)),
                ServerMsg::Snow(true)
            ])
        );
    }
    let mut body = vec![0; 49];
    body[0] = 255;
    body[2..7].copy_from_slice(&[0, 0, 0xb4, 0, 1]);
    body[41] = 255;
    body[48] = 255;
    let batch = ServerMsg::decode(&dd04(&[(op::MENU_LIST, body), (0xb4, vec![0])])).unwrap();
    let ServerMsg::Batch(messages) = batch else {
        panic!()
    };
    assert!(
        matches!(messages.as_slice(), [ServerMsg::CashShop(CashShopEvent::NativeMenuList(menu)), ServerMsg::Snow(false)]
        if menu.result == 255 && menu.main_tabs[5] == 255 && menu.sub_tabs[5][6] == 255 && menu.sub_tabs[0][2] == 0xb4)
    );
}

#[test]
fn ingame_shop_menu_request_is_exact_empty_cs11b_at_every_counter() {
    let request = ClientMsg::RequestInGameShopMenu;
    assert_eq!(request.level(), 1);
    assert_eq!(request.opcode(), 0x11b);
    assert_eq!(request.name(), "CSICSMenuListPacket");
    assert!(request.encode_body().is_empty());
    for counter in 0..=255u8 {
        assert_eq!(request.encode(counter), [0xdd, 1, 0, counter, 0x1b, 1]);
    }
}

pub(crate) fn config(features: Vec<u8>) -> InitialConfig {
    InitialConfig {
        host: vec![],
        feature_bytes: features,
        raw_candidate_count: 0,
        candidate_codes: vec![],
        initial_lp: 0,
        service_flags: [0; 7],
        second_price_type: 0,
        second_password_max_fail_count: 0,
        idle_kick_time: 0,
    }
}

#[test]
fn ingame_shop_menu_and_transport_outcomes_never_hydrate_feature_admission() {
    let mut config_state = GameConfig::default();
    for features in [
        None,
        Some(vec![]),
        Some(vec![0; 4]),
        Some(vec![0; 5]),
        Some(vec![0, 0, 0, 0, 0x40]),
    ] {
        config_state.received = features.map(config);
        let before = config_state.clone();
        for result in [0, 1, 2, 255] {
            let (_, menu) = fixture(result);
            for event in [
                Event::CashShop(CashShopEvent::NativeMenuList(menu)),
                Event::CashShop(CashShopEvent::CheckTime),
                Event::InGameShopMenuRequested { character_id: 42 },
                Event::InGameShopMenuRequestRejected {
                    character_id: 42,
                    message: "fixture send failure".into(),
                },
            ] {
                assert!(!config_state.apply(&event));
                assert_eq!(config_state, before);
            }
        }
    }
}
