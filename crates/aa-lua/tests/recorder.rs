//! API recorder smoke test on world select (needs the real `game_pak`).

use std::collections::BTreeSet;

use aa_lua::Stage;
use aa_lua::recorder::{self, load_reference, reference_dir, static_scan, ui_files};

#[test]
fn world_select_records_api_and_matches_real_client() {
    let scan = static_scan(&ui_files()).unwrap();
    // static pass sees the native tables and the scripts' own globals
    assert!(scan.global_reads.contains_key("X2LoginCharacter"));
    assert!(scan.global_writes.contains("RegistUIEvent"));
    assert!(scan.members.contains_key(&("X2World".to_string(), "EnterWorld".to_string())));

    let x2game: serde_json::Value = serde_json::from_str(recorder::X2GAME_JSON).unwrap();
    assert_eq!(x2game["events"][477], "ENTERED_WORLD_SELECT");
    let natives: BTreeSet<String> = x2game["bind_groups"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|g| g["functions"].as_array().unwrap().iter().map(|f| f[0].as_str().unwrap().to_string()))
        .collect();
    assert!(natives.contains("SelectCharacter") && natives.contains("AddAnchor"));

    let reference = load_reference(&reference_dir());
    let rec = recorder::record_stage(Stage::WorldSelect, &scan, &natives, reference.as_ref()).unwrap();
    assert!(rec.load_errors.is_empty(), "{:#?}", rec.load_errors);
    let members = rec.rec["members"].as_object().unwrap();
    let has = |class: &str, member: &str| members.values().any(|m| m["class"] == class && m["member"] == member);
    assert!(has("X2World", "RequestWorldListRefresh"));
    assert!(has("button", "SetText"));
    assert!(rec.rec["widgets"]["button"]["count"].as_f64().unwrap() > 10.0);
    assert!(rec.rec["events"]["ENTERED_WORLD_SELECT"].is_object());
    assert!(rec.handler_calls > 100);

    // Same global functions as the real client at world select (when the capture is present).
    if let Some(r) = reference
        && !r.lua_functions.is_empty()
    {
        let missing: Vec<_> = r.lua_functions.difference(&rec.defined_functions).collect();
        let extra: Vec<_> = rec.defined_functions.difference(&r.lua_functions).collect();
        assert!(missing.is_empty() && extra.is_empty(), "missing {missing:?}, extra {extra:?}");
    }
}
