use super::tests::setup;
use super::*;

#[test]
fn roadmap_native_getter_setter_goldens_and_shared_tree_preservation()
-> Result<(), Box<dyn std::error::Error>> {
    let (rt, bridge) = setup()?;
    let oracle: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../tools/evidence/roadmap/native-cases.json"
    ))?;
    let x2: Table = rt.lua.globals().get("X2")?;
    //The original setter has no wire operation, and the app's NPC side effect
    //may safely query its newly written options without a retained HUD borrow.
    rt.lua
        .load("HUD.map_dots=function(on) DOTS=on;X2:GetRoadmapOptionUiData() end")
        .exec()?;
    for case in oracle["getter_cases"].as_array().unwrap() {
        if case["case"] == "npc_only" {
            continue;
        } //native uninitialized stack; separate safe-limit test below
        let payload = case["payload"].as_str().unwrap();
        {
            let mut data = bridge.borrow_mut();
            data.achievement.persistence.reset(47);
            data.achievement
                .persistence
                .receive(&aa_net::UiDataResponse {
                    character_id: 47,
                    data_type: 5,
                    payload: payload.into(),
                    raw_length: payload.len() as i32 + 1,
                });
        }
        let table = x2.call_method::<Option<Table>>("GetRoadmapOptionUiData", ())?;
        let actual = match table {
            None => serde_json::Value::Null,
            Some(table) => {
                let mut out = serde_json::Map::new();
                for key in ["size", "visible", "npcShow"] {
                    match table.get::<Value>(key)? {
                        Value::Number(v) => {
                            out.insert(key.into(), serde_json::json!(v));
                        }
                        Value::Integer(v) => {
                            out.insert(key.into(), serde_json::json!(v as f64));
                        }
                        Value::Boolean(v) => {
                            out.insert(key.into(), serde_json::json!(v));
                        }
                        Value::Nil => (),
                        value => panic!("unexpected native field {value:?}"),
                    }
                }
                serde_json::Value::Object(out)
            }
        };
        assert_eq!(actual, case["result"], "getter {}", case["case"]);
    }
    for case in oracle["setter_cases"].as_array().unwrap() {
        rt.lua.globals().set("DOTS", false)?;
        let seed = case["seed"].as_str().unwrap();
        {
            let mut data = bridge.borrow_mut();
            data.achievement.persistence.reset(47);
            data.achievement
                .persistence
                .receive(&aa_net::UiDataResponse {
                    character_id: 47,
                    data_type: 5,
                    payload: seed.into(),
                    raw_length: seed.len() as i32 + 1,
                });
        }
        let options = rt.lua.create_table()?;
        for (key, value) in case["input"].as_object().unwrap() {
            let value = match value {
                serde_json::Value::Null => Value::Nil,
                serde_json::Value::Bool(v) => Value::Boolean(*v),
                serde_json::Value::Number(v) => Value::Number(v.as_f64().unwrap()),
                serde_json::Value::String(v) => Value::String(rt.lua.create_string(v)?),
                serde_json::Value::Object(_) => Value::Table(rt.lua.create_table()?),
                value => panic!("unexpected input {value:?}"),
            };
            options.set(key.as_str(), value)?;
        }
        x2.call_method::<()>("SetRoadmapOptionUiData", options)?;
        assert_eq!(
            rt.lua.globals().get::<bool>("DOTS")?,
            case["input"]["npcShow"] == true
        );
        assert!(bridge.borrow().requests.is_empty());
        let aa_net::UiDataCommand::Save { payload, .. } = bridge
            .borrow_mut()
            .achievement
            .persistence
            .save(47)
            .unwrap()
        else {
            panic!()
        };
        let mut expected =
            aa_net::ui_data::NativeData::parse(case["serialized"].as_str().unwrap().as_bytes())
                .unwrap();
        expected.children.push(aa_net::ui_data::NativeDataNode {
            value: b"version".to_vec(),
            children: vec![aa_net::ui_data::NativeDataNode {
                value: b"1".to_vec(),
                children: vec![],
            }],
        });
        assert_eq!(
            aa_net::ui_data::NativeData::parse(payload.as_bytes()).unwrap(),
            expected,
            "setter {}",
            case["case"]
        );
    }
    Ok(())
}

#[test]
fn roadmap_native_numeric_boundaries_partial_record_limit_and_reentrant_fields()
-> Result<(), Box<dyn std::error::Error>> {
    let (rt, bridge) = setup()?;
    let oracle: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../tools/evidence/roadmap/native-boundaries.json"
    ))?;
    let x2: Table = rt.lua.globals().get("X2")?;
    let receive = |payload: &str| {
        let mut data = bridge.borrow_mut();
        data.achievement.persistence.reset(47);
        data.achievement
            .persistence
            .receive(&aa_net::UiDataResponse {
                character_id: 47,
                data_type: 5,
                payload: payload.into(),
                raw_length: payload.len() as i32 + 1,
            });
    };
    for case in oracle["Data_i32_to_script_f32"].as_array().unwrap() {
        receive(&format!(
            "roadmap_option ( size {}, npcShow false )\r\n",
            case["token"].as_str().unwrap()
        ));
        let options = x2.call_method::<Table>("GetRoadmapOptionUiData", ())?;
        for key in ["size", "npcShow"] {
            assert_eq!(
                options.get::<f64>(key)?,
                case["result"][key].as_f64().unwrap()
            );
        }
    }
    for case in oracle["setter_script_f32_cvttss2si"].as_array().unwrap() {
        receive("roadmap_option old\r\nother 17\r\n");
        let options = rt.lua.create_table()?;
        options.set("size", case["input"].as_f64().unwrap())?;
        x2.call_method::<()>("SetRoadmapOptionUiData", options)?;
        let aa_net::UiDataCommand::Save { payload, .. } = bridge
            .borrow_mut()
            .achievement
            .persistence
            .save(47)
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(
            payload,
            format!("{}version 1\r\n", case["serialized"].as_str().unwrap())
        );
    }
    receive("roadmap_option npcShow true\r\n");
    let options = x2.call_method::<Table>("GetRoadmapOptionUiData", ())?;
    assert!(options.get::<Option<f64>>("npcShow")?.is_none()); //chosen limit, not a native zero result
    rt.lua
        .load(
            r#"
        local order={}
        X2:SetRoadmapOptionUiData(setmetatable({}, {__index=function(_,key)
            assert(X2:GetRoadmapOptionUiData()==nil)
            order[#order+1]=key
            return ({size=2,visible=false,npcShow=true})[key]
        end}))
        assert(table.concat(order,',')=='size,visible,npcShow')
        assert(X2:GetRoadmapOptionUiData().npcShow==2)
    "#,
        )
        .exec()?;
    Ok(())
}

#[test]
#[ignore = "loads original roadmap and exercises production receipt scheduling"]
fn roadmap_suspended_receipt_applies_on_resume_without_another_event()
-> Result<(), Box<dyn std::error::Error>> {
    let (rt, bridge, path) = super::super::breath::fixture::runtime(1280., 720.)?;
    rt.lua
        .load("local previous=HUD.map_dots;HUD.map_dots=function(on) DOTS=on;previous(on) end")
        .exec()?;
    let spawn = super::super::expansion::fixture::spawn();
    dispatch(
        Some(&rt),
        &bridge,
        &Event::CharacterSelected(Box::new(spawn.state)),
    );
    bridge.borrow_mut().character_id = 47;
    let mut app = App::new();
    app.add_message::<NetEvent>()
        .init_resource::<super::super::HudBuilt>()
        .insert_non_send(rt)
        .insert_non_send(HudBridge(bridge.clone()))
        .add_systems(Update, track);
    let payload = "sibling ( keep me )\r\ntracing_achievement ( 2, 0, 0, 0, 0, 0 )\r\nroadmap_option ( size 2, visible true, npcShow false )\r\n";
    let response = aa_net::UiDataResponse {
        character_id: 47,
        data_type: 5,
        payload: payload.into(),
        raw_length: payload.len() as i32 + 1,
    };
    app.world_mut()
        .write_message(NetEvent(Event::UiData(aa_net::UiDataEvent::Response(
            response.clone(),
        ))));
    app.update();
    assert!(bridge.borrow().achievement.roadmap_dirty);
    app.world()
        .get_non_send::<UiRuntime>()
        .unwrap()
        .lua
        .load("assert(roadmapSizeButton:GetText()=='L')")
        .exec()?;
    app.world_mut().resource_mut::<super::super::HudBuilt>().up = true;
    app.update(); //no new packet; the restored runtime must consume the pending refresh
    let rt = app.world().get_non_send::<UiRuntime>().unwrap();
    rt.lua.load("assert(roadmapSizeButton:GetText()=='M');assert(roadmapOptionButton.wnd.chk2:GetChecked());assert(X2:GetRoadmapOptionUiData().npcShow==2)").exec()?;
    assert!(!bridge.borrow().achievement.roadmap_dirty);
    dispatch(
        Some(rt),
        &bridge,
        &Event::UiData(aa_net::UiDataEvent::Response(response.clone())),
    );
    super::super::expansion::fixture::click(rt, "roadmapSizeButton")?;
    //The repeated getter leaves a numeric NPC value in original optionInfo.
    //A size-only save omits that value but must not disable still-enabled dots.
    rt.lua.load("assert(roadmapSizeButton:GetText()=='L');assert(roadmapOptionButton.wnd.chk2:GetChecked());assert(DOTS==true);assert(X2:GetRoadmapOptionUiData().npcShow==nil)").exec()?;
    let mut invalid = response.clone();
    invalid.character_id = 48;
    dispatch(
        Some(rt),
        &bridge,
        &Event::UiData(aa_net::UiDataEvent::Response(invalid)),
    );
    let mut invalid = response;
    invalid.raw_length = 3;
    dispatch(
        Some(rt),
        &bridge,
        &Event::UiData(aa_net::UiDataEvent::Response(invalid)),
    );
    rt.lua
        .load("assert(roadmapSizeButton:GetText()=='L')")
        .exec()?;
    assert!(save_for_leave(&mut bridge.borrow_mut(), 1).is_none());
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    drop(app);
    drop(bridge);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[test]
#[ignore = "loads original roadmap twice to check preconstruction hydration and stage rebuild"]
fn roadmap_preconstruction_and_rebuilt_stage_restore_saved_tree_and_empty_defaults()
-> Result<(), Box<dyn std::error::Error>> {
    let (mut rt, bridge) = setup()?;
    let spawn = super::super::expansion::fixture::spawn();
    dispatch(
        None,
        &bridge,
        &Event::CharacterSelected(Box::new(spawn.state)),
    );
    bridge.borrow_mut().character_id = 47;
    let payload = "sibling saved\r\ntracing_achievement ( 2, 0, 0, 0, 0, 0 )\r\nroadmap_option ( size 1, visible false, npcShow false )\r\n";
    dispatch(
        None,
        &bridge,
        &Event::UiData(aa_net::UiDataEvent::Response(aa_net::UiDataResponse {
            character_id: 47,
            data_type: 5,
            payload: payload.into(),
            raw_length: payload.len() as i32 + 1,
        })),
    );
    rt.load_ui_texts(std::path::Path::new(aa_anim::db::DEFAULT_DB))?;
    let path = crate::settings::isolated_bindings(&rt)?;
    rt.run_stage(aa_ui::Stage::World);
    roadmap::flush(Some(&rt), &bridge);
    rt.lua.load("assert(roadmapSizeButton:GetText()=='S' and not roadmapWindow:IsVisible());assert(roadmapOptionButton.wnd.chk2:GetChecked())").exec()?;
    let aa_net::UiDataCommand::Save { payload: saved, .. } =
        save_for_leave(&mut bridge.borrow_mut(), 1).unwrap()
    else {
        panic!()
    };
    assert!(saved.starts_with("sibling saved\r\ntracing_achievement ( 2, 0, 0, 0, 0, 0 )\r\n"));
    assert!(saved.contains("roadmap_option ( size 1, visible false, npcShow true )"));
    assert!(bridge.borrow().requests.is_empty());
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    drop(rt);
    let mut rebuilt = UiRuntime::new("en_us", 1280., 720.)?;
    rebuilt.load_ui_texts(std::path::Path::new(aa_anim::db::DEFAULT_DB))?;
    super::super::api::install(&mut rebuilt, &bridge)?;
    let rebuilt_path = crate::settings::isolated_bindings(&rebuilt)?;
    rebuilt.run_stage(aa_ui::Stage::World);
    rebuilt
        .lua
        .load("assert(roadmapSizeButton:GetText()=='S' and not roadmapWindow:IsVisible())")
        .exec()?;
    dispatch(
        Some(&rebuilt),
        &bridge,
        &Event::UiData(aa_net::UiDataEvent::Response(aa_net::UiDataResponse {
            character_id: 47,
            data_type: 5,
            payload: String::new(),
            raw_length: 1,
        })),
    );
    rebuilt.lua.load("assert(roadmapSizeButton:GetText()=='L' and roadmapWindow:IsVisible());assert(not roadmapOptionButton.wnd.chk2:GetChecked())").exec()?;
    assert!(rebuilt.errors.is_empty(), "{:?}", rebuilt.errors);
    drop(rebuilt);
    drop(bridge);
    for path in [path, rebuilt_path] {
        if path.exists() {
            std::fs::remove_file(path)?;
        }
    }
    Ok(())
}

#[test]
#[ignore = "loads original world-stage roadmap bytecode"]
fn roadmap_original_size_visibility_and_npc_controls() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, bridge, path) = super::super::breath::fixture::runtime(1280., 720.)?;
    rt.lua
        .load(
            r#"
        local get = X2.GetRoadmapOptionUiData
        for _, case in ipairs({
            {nil, 926, 556, 'L', true, false},
            {{}, 926, 556, 'L', true, false},
            {{size=1,visible=false,npcShow=false}, 463, 278, 'S', false, false},
            {{size=2,visible=true,npcShow=true}, 648.2, 389.2, 'M', true, true},
            {{size=3,visible=true,npcShow=false}, 926, 556, 'L', true, false},
        }) do
            X2.GetRoadmapOptionUiData = function() return case[1] end
            roadmapSizeButton:Init()
            assert(math.abs(roadmapWindow:GetWidth()-case[2])<.01)
            assert(math.abs(roadmapWindow:GetHeight()-case[3])<.01)
            assert(roadmapSizeButton:GetText()==case[4])
            assert(roadmapWindow:IsVisible()==case[5])
            assert(roadmapOptionButton.wnd.chk2:GetChecked()==case[6])
        end
        X2.GetRoadmapOptionUiData = get
    "#,
        )
        .exec()?;
    for size in ["S", "M", "L"] {
        super::super::expansion::fixture::click(&rt, "roadmapSizeButton")?;
        rt.lua
            .load(format!("assert(roadmapSizeButton:GetText()=='{size}')"))
            .exec()?;
    }
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    drop(rt);
    drop(bridge);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[test]
#[ignore = "loads original world-stage scripts to check received roadmap options"]
fn roadmap_type5_restores_original_small_hidden_controls_after_construction()
-> Result<(), Box<dyn std::error::Error>> {
    let (rt, bridge, path) = super::super::breath::fixture::runtime(1280., 720.)?;
    let spawn = super::super::expansion::fixture::spawn();
    dispatch(
        Some(&rt),
        &bridge,
        &Event::CharacterSelected(Box::new(spawn.state.clone())),
    );
    bridge.borrow_mut().character_id = spawn.character().id;
    let payload =
        "other saved\r\nroadmap_option\r\n size 1\r\n visible false\r\n npcShow false\r\n";
    dispatch(
        Some(&rt),
        &bridge,
        &Event::UiData(aa_net::UiDataEvent::Response(aa_net::UiDataResponse {
            character_id: spawn.character().id,
            data_type: 5,
            payload: payload.into(),
            raw_length: payload.len() as i32 + 1,
        })),
    );
    rt.lua.load("assert(roadmapSizeButton:GetText()=='S', 'saved small roadmap was not restored');assert(not roadmapWindow:IsVisible());assert(math.abs(roadmapWindow:GetWidth()-463)<.01)").exec()?;
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    drop(rt);
    drop(bridge);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}
