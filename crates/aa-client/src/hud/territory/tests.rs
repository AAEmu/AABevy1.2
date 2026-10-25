use super::*;
use aa_net::{DominionData, DominionEvent, Event, ExpeditionEvent, ExpeditionFaction};
use std::{cell::RefCell, rc::Rc};

#[test]
fn taxation_sql_text_prefix_admission_preserves_row_order_and_numeric_spelling()
-> rusqlite::Result<()> {
    let conn = rusqlite::Connection::open_in_memory()?;
    conn.execute_batch("CREATE TABLE taxations(id INT,tax INT,desc TEXT,SHOW NUM); CREATE TABLE localized_texts(idx INT,tbl_name TEXT,tbl_column_name TEXT,en_us TEXT); CREATE TABLE guard_tower_steps(id INT,buff_id INT,guard_tower_setting_id INT,num_gates INT,num_walls INT,step INT); INSERT INTO taxations VALUES(10,100,'ten','two'),(1,100,'one','f'),(11,100,'eleven','TRUE'),(2,100,'two',' true'),(3,100,'three','1junk'),(4,100,'four',2),(5,100,'five',10),(6,100,'six',-1),(7,100,'seven',NULL),(8,100,'tiny',1e-10);")?;
    let tables = Tables::load(&conn)?;
    assert_eq!(
        tables
            .taxes
            .iter()
            .map(|r| r.desc.as_str())
            .collect::<Vec<_>>(),
        ["ten", "eleven", "three", "five", "tiny"]
    );
    Ok(())
}

#[test]
fn received_load_and_actor_loss_retire_work_without_changing_dominion() -> mlua::Result<()> {
    let load = |zone_id| {
        Event::LoadInstance(aa_net::world_msg::InstanceLoad {
            raw_id: 7,
            zone_id,
            pos: aa_net::Pos::new(1., 2., 3.),
            angles: [0.; 3],
        })
    };
    for (event, retire) in [
        (load(204), true),
        (load(u32::MAX), true),
        (Event::UnitsRemoved(vec![201]), true),
        (Event::UnitsRemoved(vec![900]), true),
        (Event::UnitsRemoved(vec![100]), true),
        (Event::UnitsRemoved(vec![77]), false),
        (
            Event::GameDecodeFailed {
                message: "fixture gap".into(),
            },
            true,
        ),
    ] {
        let (_, shared) = runtime()?;
        let mut data = shared.borrow_mut();
        data.territory.context = Some(Context {
            npc: (201, 42),
            zone: 33,
        });
        data.territory.guard = Some(guard::Inputs {
            tower: 900,
            buffs: vec![4772],
            gates: 2,
            walls: 31,
        });
        queue(&mut data, Value::Integer(33), Value::Integer(21));
        assert_eq!(data.territory.requests.len(), 1);
        let generation = data.territory.generation;
        invalidate(&mut data, &event);
        assert_eq!(data.territory.context.is_none(), retire, "{event:?}");
        assert_eq!(data.territory.guard.is_none(), retire);
        assert_eq!(data.territory.requests.is_empty(), retire);
        assert_eq!(data.territory.hide_pending, retire);
        assert_eq!(data.territory.generation != generation, retire);
        assert_eq!(data.expedition.dominion().rows[&33].tax_rate, 17);
    }
    Ok(())
}

#[test]
#[ignore = "original UI lifecycle component; no GPU or network"]
fn retirement_waits_for_hud_resume_and_then_closes_the_original_window()
-> Result<(), Box<dyn std::error::Error>> {
    let (rt, shared, path) = fixture::runtime(1280., 720., true, true)?;
    invalidate(
        &mut shared.borrow_mut(),
        &Event::GameDecodeFailed {
            message: "suspended fixture gap".into(),
        },
    );
    rt.lua.load("assert(territoryFrame:IsVisible())").exec()?;
    let mut app = App::new();
    app.init_resource::<super::super::HudBuilt>()
        .insert_non_send(HudBridge(shared.clone()))
        .add_systems(Update, hide_retired.run_if(super::super::hud_up));
    app.update();
    assert!(shared.borrow().territory.hide_pending);
    app.insert_non_send(rt);
    app.world_mut().resource_mut::<super::super::HudBuilt>().up = true;
    app.update();
    assert!(!shared.borrow().territory.hide_pending);
    app.world()
        .non_send::<aa_ui::UiRuntime>()
        .lua
        .load("assert(not territoryFrame:IsVisible())")
        .exec()?;
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

fn receive(shared: &Shared, event: Event) {
    super::super::expedition::dispatch(None, shared, &event);
}

fn runtime() -> mlua::Result<(Lua, Shared)> {
    let lua = Lua::new();
    lua.load("X2Dominion={};X2House={}").exec()?;
    let shared = Rc::new(RefCell::new(HudData::default()));
    let mut spawn = super::super::expansion::fixture::spawn();
    spawn.state.character.expedition_id = 900;
    spawn.unit.expedition_id = 900;
    receive(
        &shared,
        Event::CharacterSelected(Box::new(spawn.state.clone())),
    );
    receive(&shared, Event::PlayerSpawn(Box::new(spawn.clone())));
    shared.borrow_mut().character_id = spawn.character().id;
    receive(
        &shared,
        Event::Expedition(ExpeditionEvent::Factions(vec![ExpeditionFaction {
            id: 900,
            owner_id: spawn.character().id,
            name: b"Silver Vale\0unused".to_vec(),
            owner_name: b"Received leader\0unused".to_vec(),
            ..Default::default()
        }])),
    );
    receive(
        &shared,
        Event::Dominion(DominionEvent::Data {
            data: DominionData {
                zone_group_id: 33,
                expedition_id: 900,
                tax_rate: 17,
                peace_tax_money: i32::MIN,
                peace_tax_aa_point: i32::MAX,
                reign_start_time: 1_700_000_000,
                ..Default::default()
            },
            newly_declared: 0,
            final_data_by_request: 1,
        }),
    );
    install(&lua, &shared)?;
    Ok((lua, shared))
}

#[test]
fn original_getter_shapes_numeric_thunk_and_received_leader_gate() -> mlua::Result<()> {
    let (lua, shared) = runtime()?;
    lua.load(
        r#"
        assert(X2Dominion:GetOwnerExpeditionName(33)=='Silver Vale')
        assert(X2Dominion:GetOwnerPlayerName(33)=='Received leader')
        assert(X2Dominion:GetPeaceTaxMoney(33)=='-2147483648')
        assert(X2Dominion:GetPeaceTaxAAPoint(33)=='2147483647')
        assert(X2Dominion:GetTaxRate(33)==17)
        assert(X2Dominion:GetTaxRate(33.9)==17 and X2Dominion:GetTaxRate(65569)==17)
        assert(X2Dominion:IsTaxationWithoutMyExpedition(33)==true)
        assert(X2Dominion:IsDominionOwner(33)==true)
        for _,id in ipairs({'33',false,34,-1,0/0,1/0}) do
          assert(X2Dominion:GetOwnerPlayerName(id)=='')
          assert(X2Dominion:GetPeaceTaxMoney(id)=='')
          assert(select('#',X2Dominion:GetTaxRate(id))==0)
          assert(select('#',X2Dominion:IsTaxationWithoutMyExpedition(id))==0)
          assert(not X2Dominion:IsDominionOwner(id))
        end
    "#,
    )
    .exec()?;
    receive(
        &shared,
        Event::Expedition(ExpeditionEvent::Factions(vec![ExpeditionFaction {
            id: 900,
            owner_id: 999,
            name: b"Still same guild".to_vec(),
            ..Default::default()
        }])),
    );
    lua.load("assert(not X2Dominion:IsDominionOwner(33));assert(X2Dominion:GetOwnerExpeditionName(33)=='Still same guild')").exec()?;
    Ok(())
}

#[test]
fn local_calendar_projects_only_native_six_fields_and_never_uses_pong() -> mlua::Result<()> {
    let (lua, _shared) = runtime()?;
    lua.load(
        r#"
        local native=X2Dominion:GetReignStartDate(33)
        local host=os.date('*t',1700000000)
        assert(native.year==host.year and native.month==host.month and native.day==host.day)
        assert(native.hour==host.hour and native.minute==host.min and native.second==host.sec)
        local count=0;for _ in pairs(native) do count=count+1 end;assert(count==6)
        assert(select('#',X2Dominion:GetReignStartDate(34))==0)
    "#,
    )
    .exec()?;
    let date: mlua::Function = lua.globals().get::<Table>("os")?.get("date")?;
    for invalid in [-1, i64::MIN, i64::MAX, 32_535_216_000] {
        assert!(getters::date(&lua, &date, invalid)?.is_empty());
    }
    Ok(())
}

#[test]
fn decode_loss_withholds_stale_values_and_full_row_does_not_repair_history() -> mlua::Result<()> {
    let (lua, shared) = runtime()?;
    receive(
        &shared,
        Event::GameDecodeFailed {
            message: "fixture loss".into(),
        },
    );
    lua.load("assert(X2Dominion:GetPeaceTaxMoney(33)=='');assert(not X2Dominion:IsDominionOwner(33));assert(select('#',X2Dominion:GetReignStartDate(33))==0)").exec()?;
    receive(
        &shared,
        Event::Dominion(DominionEvent::Data {
            data: DominionData {
                zone_group_id: 33,
                tax_rate: 4,
                ..Default::default()
            },
            newly_declared: 0,
            final_data_by_request: 1,
        }),
    );
    lua.load("assert(select('#',X2Dominion:GetTaxRate(33))==0)")
        .exec()?;
    Ok(())
}

#[test]
fn captured_tax_requests_recheck_identity_owner_and_session_without_writing_received_tax()
-> mlua::Result<()> {
    let (lua, shared) = runtime()?;
    let mut spawn = super::super::expansion::fixture::spawn();
    spawn.state.character.expedition_id = 900;
    let player = aa_net::state::Player { spawn: Some(spawn) };
    let context = Context {
        npc: (201, 42),
        zone: 33,
    };
    shared.borrow_mut().territory.context = Some(context);
    lua.load("X2Dominion:ChangeDominionTaxRate(33,50.9);assert(X2Dominion:GetTaxRate(33)==17)")
        .exec()?;
    let commands = flush(&mut shared.borrow_mut(), &player);
    assert_eq!(commands.len(), 1);
    assert!(matches!(
        commands[0],
        aa_net::Command::Dominion {
            command: aa_net::DominionCommand::ChangeTaxRate {
                zone_group_id: 33,
                tax_rate: 50
            },
            ..
        }
    ));
    lua.load("for _,r in ipairs({'17',-1,51,1/0}) do X2Dominion:ChangeDominionTaxRate(33,r) end;X2Dominion:ChangeDominionTaxRate('33',17)").exec()?;
    assert!(flush(&mut shared.borrow_mut(), &player).is_empty());
    lua.load("X2Dominion:ChangeDominionTaxRate(33,-0.9)")
        .exec()?;
    let captured = shared.borrow_mut().territory.requests.pop().unwrap();
    reset(&mut shared.borrow_mut());
    shared.borrow_mut().territory.context = Some(context);
    shared.borrow_mut().territory.requests.push(captured);
    assert!(
        flush(&mut shared.borrow_mut(), &player).is_empty(),
        "same character/context in a new session cannot revive old work"
    );
    lua.load("X2Dominion:ChangeDominionTaxRate(33,0)").exec()?;
    receive(
        &shared,
        Event::Expedition(ExpeditionEvent::Factions(vec![ExpeditionFaction {
            id: 900,
            owner_id: 999,
            ..Default::default()
        }])),
    );
    assert!(
        flush(&mut shared.borrow_mut(), &player).is_empty(),
        "late leader loss rejects captured work"
    );
    assert_eq!(shared.borrow().expedition.dominion().rows[&33].tax_rate, 17);
    Ok(())
}

#[test]
#[ignore = "reads actual read-only catalog and retained original instruction goldens"]
fn actual_tax_catalog_matches_all_retained_native_preview_values()
-> Result<(), Box<dyn std::error::Error>> {
    let (lua, shared) = runtime()?;
    let mut game = super::super::data::GameData::default();
    let conn = rusqlite::Connection::open_with_flags(
        aa_anim::db::DEFAULT_DB,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    game.territory = Tables::load(&conn)?;
    assert!(shared.borrow_mut().game.0.set(game).is_ok());
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let evidence: serde_json::Value = serde_json::from_slice(&std::fs::read(
        root.join("data/extract/aa-ui/territory-management-20261024/more-native-cases.json"),
    )?)?;
    let loader:serde_json::Value=serde_json::from_slice(&std::fs::read(root.join("data/extract/aa-ui/territory-management-20261024/followup-prerequisites-20261024/catalog-loader-native-cases.json"))?)?;
    let order: Vec<_> = loader["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["kind"] == "taxations")
        .unwrap()["output_rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["SHOW"] == true)
        .collect();
    let mut cases = 0;
    for case in evidence["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["name"] == "original_taxation_catalog_loop")
    {
        let rate = case["rate"].as_i64().unwrap();
        let actual: Table = lua
            .load(format!("return X2House:GetTaxations({rate})"))
            .eval()?;
        assert_eq!(actual.raw_len(), 10);
        let expected = case["trace"].as_array().unwrap().last().unwrap()["result"]
            .as_object()
            .unwrap();
        assert_eq!(
            case["catalog_ids"],
            serde_json::json!([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11])
        );
        for index in 1..=actual.raw_len() {
            let value: Table = actual.raw_get(index)?;
            let id = order[index - 1]["id"].as_u64().unwrap();
            // Earlier arithmetic goldens supplied a sorted fixture vector; the
            // real loader proof supplies SQL order independently.
            assert_eq!(
                value.raw_get::<String>("taxString")?,
                expected[&(id - 1).to_string()]["taxString"]
                    .as_str()
                    .unwrap()
            );
            assert_eq!(
                value.raw_get::<String>("desc")?,
                order[index - 1]["desc"].as_str().unwrap()
            );
        }
        cases += 1;
    }
    assert_eq!(cases, 4);
    Ok(())
}

#[test]
fn original_request_parser_cases_match_captured_zone_and_rate()
-> Result<(), Box<dyn std::error::Error>> {
    let (lua, shared) = runtime()?;
    shared.borrow_mut().territory.context = Some(Context {
        npc: (201, 42),
        zone: 33,
    });
    let evidence: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/request-parser-native.json"))?;
    let method: mlua::Function = lua
        .globals()
        .get::<Table>("X2Dominion")?
        .get("ChangeDominionTaxRate")?;
    let mut count = 0;
    for case in evidence["cases"].as_array().unwrap() {
        let mut args = vec![Value::Nil];
        for value in case["params"].as_array().unwrap() {
            args.push(match value {
                serde_json::Value::Number(n) => Value::Number(n.as_f64().unwrap()),
                serde_json::Value::String(s) => Value::String(lua.create_string(s)?),
                other => panic!("unexpected retained input {other}"),
            });
        }
        assert!(
            method
                .call::<MultiValue>(MultiValue::from_iter(args))?
                .is_empty()
        );
        let requests = std::mem::take(&mut shared.borrow_mut().territory.requests);
        let sends: Vec<_> = case["trace"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|r| r.get("send_capture"))
            .collect();
        assert_eq!(requests.len(), sends.len(), "{case}");
        for (request, send) in requests.iter().zip(sends) {
            assert_eq!(
                i64::from(request.context.zone),
                send["zone_group"].as_i64().unwrap()
            );
            assert_eq!(i64::from(request.rate), send["rate"].as_i64().unwrap());
        }
        count += 1;
    }
    assert_eq!(count, 11);
    Ok(())
}

#[test]
#[ignore = "loads the original world ALBs and actual catalog; no GPU or network"]
fn original_world_construction_does_not_fabricate_territory()
-> Result<(), Box<dyn std::error::Error>> {
    let (rt, shared, path) = super::super::breath::fixture::runtime(1280., 720.)?;
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    assert!(shared.borrow().expedition.dominion().rows.is_empty());
    rt.lua.load("assert(not territoryFrame:IsVisible());assert(X2Dominion:GetPeaceTaxMoney(33)=='');assert(not X2Dominion:IsDominionOwner(33));assert(select('#',X2Dominion:GetTaxRate(33))==0)").exec()?;
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[test]
fn guard_projection_preserves_initial_row_shape_signed_limits_and_locale_reentry()
-> mlua::Result<()> {
    let (lua, shared) = runtime()?;
    lua.load("assert(select('#',X2House:GetGuardTowerStepInfo())==0);X2Locale={}")
        .exec()?;
    let state = shared.clone();
    let calls = Rc::new(std::cell::Cell::new(0));
    let seen = calls.clone();
    lua.globals().get::<Table>("X2Locale")?.set(
        "LocalizeUiText",
        lua.create_function(move |_, (_, table, key): (Value, i32, String)| {
            assert_eq!(table, 33);
            assert_eq!(key, "buff");
            let _reentry = state.borrow_mut();
            seen.set(seen.get() + 1);
            Ok("Localized buff")
        })?,
    )?;
    let mut game = super::super::data::GameData::default();
    for id in [4771, 4772, 4773] {
        game.buffs.insert(
            id,
            super::super::data::BuffInfo {
                named: super::super::data::Named {
                    name: format!("Fixture {id}"),
                    icon: format!("fixture/{id}.dds"),
                },
                desc: format!("Literal |cFFFF9C27{id}|r"),
                kind: 1,
                ..Default::default()
            },
        );
    }
    game.territory.steps = vec![
        catalog::Step {
            setting: 10,
            buff: 999999,
            step: 0,
            gates: 0,
            walls: 0,
        },
        catalog::Step {
            setting: 10,
            buff: 4772,
            step: 1,
            gates: 0,
            walls: 33,
        },
        catalog::Step {
            setting: 10,
            buff: 4773,
            step: 2,
            gates: 0,
            walls: 36,
        },
    ];
    assert!(shared.borrow_mut().game.0.set(game).is_ok());
    receive(
        &shared,
        Event::Dominion(DominionEvent::Data {
            data: DominionData {
                zone_group_id: 33,
                obj_id: 900,
                territory: aa_net::DominionTerritory {
                    id: 10,
                    id2: 4771,
                    max_gates: 128,
                    max_walls: 255,
                    ..Default::default()
                },
                ..Default::default()
            },
            newly_declared: 0,
            final_data_by_request: 1,
        }),
    );
    {
        let mut data = shared.borrow_mut();
        data.territory.context = Some(Context {
            npc: (201, 42),
            zone: 33,
        });
        data.territory.guard = Some(guard::Inputs {
            tower: 900,
            buffs: vec![4772],
            gates: 2,
            walls: 31,
        });
    }
    lua.load(r#"
        local rows,gates,walls=X2House:GetGuardTowerStepInfo()
        assert(#rows==3 and gates==-128 and walls==-1)
        assert(rows[1].buff_id==4771 and rows[1].step==0 and not rows[1].isCurStep)
        assert(rows[1].curGate==nil and rows[1].curWall==nil and rows[1].maxGate==nil and rows[1].maxWall==nil)
        assert(rows[2].buff_id==4772 and rows[2].step==1 and rows[2].isCurStep)
        assert(rows[2].curGate==2 and rows[2].maxGate==0 and rows[2].curWall==31 and rows[2].maxWall==33)
        assert(rows[3].buff_id==4773 and rows[3].step==2 and not rows[3].isCurStep)
        for i,row in ipairs(rows) do
          local n=0;for _ in pairs(row) do n=n+1 end;assert(n==(i==1 and 8 or 12))
          assert(row.tipType=='buff' and row.category=='Localized buff' and row.name=='Fixture '..row.buff_id)
          assert(row.description=='Literal |cFFFF9C27'..row.buff_id..'|r' and row.path=='fixture/'..row.buff_id..'.dds')
          assert(row.mine==nil and row.duration==nil and row.timeLeft==nil and row.stack==nil)
        end
    "#).exec()?;
    assert_eq!(calls.get(), 3);
    shared
        .borrow_mut()
        .territory
        .guard
        .as_mut()
        .unwrap()
        .buffs
        .clear();
    lua.load("local rows=X2House:GetGuardTowerStepInfo();assert(#rows==3);for _,row in ipairs(rows) do assert(not row.isCurStep) end").exec()?;
    shared.borrow_mut().territory.guard.as_mut().unwrap().tower = 901;
    lua.load("assert(select('#',X2House:GetGuardTowerStepInfo())==0)")
        .exec()?;
    Ok(())
}

#[test]
#[ignore = "reads actual guard descriptors and the complete original metadata builder goldens"]
fn actual_guard_metadata_matches_original_six_field_builder()
-> Result<(), Box<dyn std::error::Error>> {
    let (lua, shared) = runtime()?;
    let game = super::super::data::GameData::load(aa_anim::db::DEFAULT_DB)?;
    assert!(shared.borrow_mut().game.0.set(game).is_ok());
    lua.load("X2Locale={LocalizeUiText=function(self,id,key) assert(id==33);return key=='buff' and 'Buff' or 'Debuff' end}").exec()?;
    receive(
        &shared,
        Event::Dominion(DominionEvent::Data {
            data: DominionData {
                zone_group_id: 33,
                obj_id: 900,
                territory: aa_net::DominionTerritory {
                    id: 10,
                    id2: 4771,
                    max_gates: 4,
                    max_walls: 100,
                    ..Default::default()
                },
                ..Default::default()
            },
            newly_declared: 0,
            final_data_by_request: 1,
        }),
    );
    {
        let mut data = shared.borrow_mut();
        data.territory.context = Some(Context {
            npc: (201, 42),
            zone: 33,
        });
        data.territory.guard = Some(guard::Inputs {
            tower: 900,
            buffs: vec![4772],
            gates: 2,
            walls: 31,
        });
    }
    let (rows, gates, walls): (Table, i32, i32) =
        lua.load("return X2House:GetGuardTowerStepInfo()").eval()?;
    assert_eq!((gates, walls), (4, 100));
    assert_eq!(rows.raw_len(), 6);
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let evidence:serde_json::Value=serde_json::from_slice(&std::fs::read(root.join("data/extract/aa-ui/territory-management-20261024/followup-prerequisites-20261024/guard-buff-builder-native-cases.json"))?)?;
    let mut count = 0;
    for case in evidence["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["name"] == "guard_full_metadata")
    {
        let expected = case["table"].as_object().unwrap();
        let id = expected["buff_id"].as_u64().unwrap();
        let actual = rows
            .clone()
            .sequence_values::<Table>()
            .collect::<mlua::Result<Vec<_>>>()?
            .into_iter()
            .find(|r| r.raw_get::<u64>("buff_id").ok() == Some(id))
            .unwrap();
        for key in ["description", "category", "tipType", "name", "path"] {
            assert_eq!(
                actual.raw_get::<String>(key)?,
                expected[key].as_str().unwrap(),
                "buff{id}.{key}"
            );
        }
        count += 1;
    }
    assert_eq!(count, 6);
    Ok(())
}

#[test]
#[ignore = "original UI component with explicit synthetic NPC/population endpoints; no GPU or network"]
fn original_territory_controls_refresh_guard_and_queue_only_admitted_apply()
-> Result<(), Box<dyn std::error::Error>> {
    let (rt, shared, path) = fixture::runtime(1280., 720., true, true)?;
    rt.lua.load(r#"
        local page=territoryFrame.tab.window[1]
        assert(page.defaultInfo:GetText():find('Fixture expedition',1,true))
        assert(page.defaultInfo:GetText():find('Fixture lord',1,true))
        assert(page.taxRateSlider:IsEnabled() and not page.applyButton:IsEnabled() and page.cancelButton:IsVisible())
        assert(page.currentTaxRate:GetText()=='17%')
        page.taxRateSlider:SetValue(29.9,true)
        assert(page.taxRateSlider:GetValue('horz')==29 and page.currentTaxRate:GetText()=='29%')
        assert(page.applyButton:IsEnabled() and X2Dominion:GetTaxRate(33)==17)
    "#).exec()?;
    shared.borrow_mut().territory.guard.as_mut().unwrap().walls = 32;
    rt.fire_event("HOUSE_UNIT_CREATED", ());
    rt.lua.load(r#"
        local rows=X2House:GetGuardTowerStepInfo()
        assert(territoryFrame:IsVisible() and rows[2].curWall==32)
        assert(territoryFrame.tab.window[2].listCtrl.items[2].subItems[1].info:GetText():find('32/'..rows[2].maxWall,1,true))
    "#).exec()?;
    rt.lua.load("territoryFrame.tab:SelectTab(2)").exec()?;
    let mut widgets = Vec::new();
    for row in 1..=6 {
        for suffix in ["", ".buffName", ".info", ".buffIconBtn"] {
            let expression =
                format!("territoryFrame.tab.window[2].listCtrl.items[{row}].subItems[1]{suffix}");
            let object: Value = rt.lua.load(&expression).eval()?;
            widgets.push((
                expression,
                aa_ui::lua::value_id(&rt.lua, &object).ok_or("guard widget missing")?,
            ));
        }
    }
    {
        let ui = aa_ui::lua::ui(&rt.lua);
        let paint = ui.tree.paint_order();
        for (expression, id) in widgets {
            assert!(
                ui.tree.visible(id),
                "{expression} must be effectively visible"
            );
            assert!(
                paint.contains(&id),
                "{expression} must reach paint traversal without InsertData"
            );
        }
    }
    let mut mouse = aa_ui::input::MouseState::default();
    for (row, buff) in [4771, 4772, 4773, 4774, 5392, 5393].into_iter().enumerate() {
        let expression = format!(
            "territoryFrame.tab.window[2].listCtrl.items[{}].subItems[1].buffIconBtn",
            row + 1
        );
        let value: Value = rt.lua.load(&expression).eval()?;
        let id = aa_ui::lua::value_id(&rt.lua, &value).ok_or("guard icon missing")?;
        let rect = aa_ui::lua::ui(&rt.lua).tree.rect(id);
        let point = (rect.x + rect.w * 0.5, rect.y + rect.h * 0.5);
        assert_eq!(aa_ui::input::pick(&rt.lua, point.0, point.1), Some(id));
        aa_ui::input::update(&rt.lua, &mut mouse, Some(point), false, false, false, 0.);
        assert_eq!(aa_ui::lua::ui(&rt.lua).hover, Some(id));
        rt.lua.load(format!("local t=tooltip.tooltipWindows[7];assert(t and t:IsVisible() and t.tipType=='buff');assert(t.buffInfo.buff_id=={buff})")).exec()?;
        aa_ui::input::update(&rt.lua, &mut mouse, None, false, false, false, 0.);
        rt.lua
            .load("assert(not tooltip.tooltipWindows[7]:IsVisible())")
            .exec()?;
    }
    rt.lua.load("territoryFrame.tab:SelectTab(1)").exec()?;
    super::super::expansion::fixture::click(&rt, "territoryFrame.tab.window[1].applyButton")?;
    rt.lua
        .load("assert(not territoryFrame:IsVisible());assert(X2Dominion:GetTaxRate(33)==17)")
        .exec()?;
    let player = aa_net::state::Player {
        spawn: Some(super::super::expansion::fixture::spawn()),
    };
    let commands = flush(&mut shared.borrow_mut(), &player);
    assert_eq!(commands.len(), 1);
    assert!(matches!(
        commands[0],
        aa_net::Command::Dominion {
            command: aa_net::DominionCommand::ChangeTaxRate {
                zone_group_id: 33,
                tax_rate: 29
            },
            ..
        }
    ));
    rt.lua.load("ToggleTerritoryFrame(true,14,33)").exec()?;
    shared.borrow_mut().territory.guard = None;
    rt.fire_event("HOUSE_UNIT_REMOVED", ());
    rt.lua.load("assert(not territoryFrame:IsVisible());assert(select('#',X2House:GetGuardTowerStepInfo())==0)").exec()?;
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[test]
#[ignore = "original UI component with explicit synthetic NPC/population endpoints; no GPU or network"]
fn original_territory_nonleader_and_late_leader_loss_cannot_submit()
-> Result<(), Box<dyn std::error::Error>> {
    for leader in [false, true] {
        let (rt, shared, path) = fixture::runtime(1280., 720., true, leader)?;
        if leader {
            rt.lua
                .load("territoryFrame.tab.window[1].taxRateSlider:SetValue(23,true)")
                .exec()?;
            receive(
                &shared,
                Event::Expedition(ExpeditionEvent::Factions(vec![ExpeditionFaction {
                    id: 900,
                    owner_id: 999,
                    ..Default::default()
                }])),
            );
        } else {
            rt.lua.load("local page=territoryFrame.tab.window[1];assert(not page.taxRateSlider:IsEnabled() and page.applyButton:IsEnabled() and not page.cancelButton:IsVisible())").exec()?;
        }
        super::super::expansion::fixture::click(&rt, "territoryFrame.tab.window[1].applyButton")?;
        rt.lua
            .load("assert(not territoryFrame:IsVisible())")
            .exec()?;
        assert!(shared.borrow().territory.requests.is_empty());
        if path.exists() {
            std::fs::remove_file(path)?;
        }
    }
    Ok(())
}
