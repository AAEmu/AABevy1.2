use super::tests::setup;
use super::*;

fn original_forms(lua: &Lua) -> Result<(), Box<dyn std::error::Error>> {
    lua.load("TRION=3;mapLocale={};X2Util.GetGameProvider=function()return TRION end;X2Locale.GetLocale=function()return 'en_us' end;roadmap={GetExtent=function()return MAP_W,MAP_H end};MAP_W=926;MAP_H=556").exec()?;
    aa_lua::run_alb(lua, "scriptsbin/x2ui/map/form.alb")?;
    aa_lua::run_alb(lua, "scriptsbin/x2ui/map/locale/en_us.alb")?;
    Ok(())
}

fn vector(lua: &Lua, value: &serde_json::Value) -> mlua::Result<Table> {
    lua.create_sequence_from(
        value
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap()),
    )
}

#[test]
fn roadmap_projection_all_original_frames_and_native_player_npc_quest_centres()
-> Result<(), Box<dyn std::error::Error>> {
    let (rt, bridge) = setup()?;
    original_forms(&rt.lua)?;
    let oracle: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../tools/evidence/roadmap/projection/native-cases.json"
    ))?;
    let literal: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../tools/evidence/roadmap/projection/literal-keys.json"
    ))?;
    let literal = literal["keys"].as_array().unwrap();
    assert_eq!(literal.len(), 271);
    let constants: Table = rt.lua.globals().get("HUD_ROAD_FRAMES")?;
    let hud: Table = rt.lua.globals().get("HUD")?;
    for (key, form) in oracle["forms"].as_object().unwrap() {
        let source_key = key;
        let (level, rest) = key.split_once(':').unwrap();
        let (id, size) = rest.split_once('_').unwrap();
        let level = level.parse::<u32>()?;
        let id = id.parse::<u32>()?;
        let size = size.parse::<u32>()?;
        let key = bridge
            .borrow()
            .game
            .get()
            .unwrap()
            .zone_groups
            .get(&id)
            .map(|z| z.key.to_ascii_uppercase())
            .unwrap_or_default();
        let frame: Table = hud.call_function("road_frame", (level, id, key, size))?;
        let group = constants.get::<Table>(level)?.get::<Option<Table>>(id)?;
        let present = group
            .and_then(|group| group.get::<Option<Table>>(size).ok().flatten())
            .is_some();
        assert_eq!(
            present,
            literal.iter().any(|v| v.as_str() == Some(source_key)),
            "locale/constant source {source_key}"
        );
        for (index, wanted) in form["offset"]
            .as_array()
            .unwrap()
            .iter()
            .chain(form["extent"].as_array().unwrap())
            .enumerate()
        {
            assert_eq!(
                frame.get::<f64>(index + 1)?,
                wanted.as_f64().unwrap(),
                "form {level}:{id}_{size}"
            );
        }
    }
    let zone_rect = &oracle["cases"][0]["world_rect"];
    let mut last_version = None;
    for (index, case) in oracle["cases"].as_array().unwrap().iter().enumerate() {
        let level = case["level"].as_u64().unwrap();
        let id = case["map"].as_u64().unwrap();
        let state: Table = hud.get("state")?;
        state.set("zone_group", 5)?;
        state.set("zone_key_name", "w_solzreed")?;
        state.set("zone_rect", vector(&rt.lua, zone_rect)?)?;
        state.set("sub_map", if level == 3 { id } else { 0 })?;
        state.set("sub_map_rect", vector(&rt.lua, &case["world_rect"])?)?;
        let xy = &case["world_point"];
        bridge.borrow_mut().pos = Vec3::new(
            xy[0].as_f64().unwrap() as f32,
            xy[1].as_f64().unwrap() as f32,
            0.,
        );
        rt.lua
            .globals()
            .set("MAP_W", case["widget_extent"][0].as_f64().unwrap())?;
        rt.lua
            .globals()
            .set("MAP_H", case["widget_extent"][1].as_f64().unwrap())?;
        //Contacts stay unchanged across each S/M/L triple; only the size changes.
        hud.set("map_icons_version", index / 3)?;
        let icons = rt.lua.create_table()?;
        for (n, kind) in ["npc_store_bank", "quest_zone_gives"]
            .into_iter()
            .enumerate()
        {
            let icon = rt.lua.create_table()?;
            icon.set("x", xy[0].as_f64().unwrap())?;
            icon.set("y", xy[1].as_f64().unwrap())?;
            icon.set("kind", kind)?;
            icon.set("tip", "projection fixture")?;
            icon.set("city", if level == 3 { id } else { 0 })?;
            icons.set(n + 1, icon)?;
        }
        hud.set("map_icons", icons)?;
        let api: Table = rt.lua.globals().get("AAUI")?;
        let result: Table = api.call_function("RoadMap", ())?;
        assert_eq!(result.get::<u64>("level")?, level);
        assert_eq!(result.get::<u64>("zone")?, id);
        let original = &case["original_form"];
        let centre = |point: &Table| -> mlua::Result<[f64; 2]> {
            Ok([
                original["offset"][0].as_f64().unwrap()
                    + point.get::<f64>("u")? * original["extent"][0].as_f64().unwrap(),
                original["offset"][1].as_f64().unwrap()
                    + point.get::<f64>("v")? * original["extent"][1].as_f64().unwrap(),
            ])
        };
        let expected = [
            case["native_centres"]["player"][0].as_f64().unwrap(),
            case["native_centres"]["player"][1].as_f64().unwrap(),
        ];
        for (actual, wanted) in centre(&result.get::<Table>("player")?)?
            .into_iter()
            .zip(expected)
        {
            assert!((actual - wanted).abs() < 0.001, "player {case}");
        }
        let markers: Table = result.get("markers")?;
        let count = if case["current_marker_uv"].is_null() {
            0
        } else {
            2
        };
        assert_eq!(
            markers.raw_len(),
            count,
            "unchanged legacy admission {case}"
        );
        for marker in markers.sequence_values::<Table>() {
            for (actual, wanted) in centre(&marker?)?.into_iter().zip(expected) {
                assert!((actual - wanted).abs() < 0.001, "marker {case}");
            }
        }
        let version = result.get::<u64>("markers_version")?;
        if let Some(old) = last_version {
            assert!(version > old, "size must invalidate unchanged contact list");
        }
        last_version = Some(version);
    }
    rt.lua.load("FORM_ROAD_MAP_LIST.W_SOLZREED_50={7,8,0,0,300,200};assert(HUD.road_frame(2,5,'W_SOLZREED',50)[1]==7);FORM_ROAD_MAP_LIST.W_GWEONID_FOREST_100={999,999,0,0,1,1};assert(HUD.road_frame(2,1,'W_GWEONID_FOREST',100)[1]==182)").exec()?;
    Ok(())
}
