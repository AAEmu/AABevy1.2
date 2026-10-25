use super::*;
use serde_json::Value;

fn goldens() -> Value {
    serde_json::from_str(include_str!("native_goldens.json")).unwrap()
}
fn values(v: &Value) -> Vec<u32> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_u64().unwrap() as u32)
        .collect()
}
fn row(columns: &Value, custom: &Value) -> Row {
    let mut out = Row {
        custom: values(custom),
        ..Row::default()
    };
    for column in columns.as_array().unwrap() {
        let i = SERVICES
            .iter()
            .position(|s| Some(s.0) == column.as_str())
            .unwrap();
        out.enabled[i] = true;
    }
    out
}
fn known(row: &Row) -> Facts<'_> {
    Facts {
        excluded: Some(false),
        dead: false,
        loot: Some(false),
        admitted: Some(true),
        availability: Some(1),
        metadata: Some(true),
        row: Entry::Loaded(row),
        quest: Some(false),
        role: Some(MerchantRole::Ordinary),
        specialty_extra: Some(false),
        greeting: Some(false),
    }
}
fn native_byte(v: &Value) -> u8 {
    v.as_bool()
        .map_or_else(|| v.as_u64().unwrap() as u8, u8::from)
}

#[test]
fn candidate_native_insertion_and_service_goldens() {
    let data = goldens();
    for case in data["insertions"].as_array().unwrap() {
        let mut out = Vec::new();
        for skill in values(&case["input"]) {
            append(&mut out, skill);
        }
        assert_eq!(out, values(&case["rows"]));
    }
    for case in data["services"].as_array().unwrap() {
        let row = row(&case["columns"], &case["custom"]);
        let mut out = Vec::new();
        services(&mut out, &row, case["specialty_extra"].as_bool()).unwrap();
        assert_eq!(out, values(&case["rows"]));
    }
}

#[test]
fn candidate_native_qualified_composition_goldens() {
    let data = goldens();
    for case in data["compositions"].as_array().unwrap() {
        let input = &case["input"];
        let row = row(&input["columns"], &input["custom"]);
        let mut facts = known(&row);
        facts.dead = input["target_dead"].as_bool().unwrap();
        facts.loot = input["loot_provider"].as_bool();
        facts.admitted = Some(!input["player_admission_block"].as_bool().unwrap());
        facts.availability = Some(native_byte(&input["current_actor_interactable"]));
        facts.metadata = input["NPC_metadata_present"].as_bool();
        if !input["template_present"].as_bool().unwrap() {
            facts.row = Entry::Missing;
        }
        facts.quest = Some(
            ["talk", "ready", "start"]
                .iter()
                .any(|key| input[key].as_bool() == Some(true)),
        );
        let authorized = input["region_auth"].as_bool().unwrap();
        facts.role = Some(match input["region_role"].as_u64().unwrap() {
            0 => MerchantRole::Dominion { authorized },
            1 => MerchantRole::Defense { authorized },
            2 => MerchantRole::Offense { authorized },
            3 => MerchantRole::Ordinary,
            _ => unreachable!(),
        });
        facts.greeting = input["greeting"].as_bool();
        assert_eq!(
            compute(facts),
            Some(values(&case["rows"])),
            "{}",
            case["name"]
        );
    }
}

#[test]
fn candidate_native_refresh_clock_goldens() {
    let data = goldens();
    for case in data["clocks"].as_array().unwrap() {
        let mut clock = RefreshClock {
            deadline: case["initial"].as_u64().unwrap() as u32,
        };
        for step in case["steps"].as_array().unwrap() {
            assert_eq!(
                clock.due(step["now"].as_u64().unwrap() as u32),
                step["query_called"].as_bool().unwrap()
            );
            assert_eq!(clock.deadline, step["deadline"].as_u64().unwrap() as u32);
        }
    }
}

#[test]
fn unsupported_candidate_providers_do_not_become_empty_or_successful_facts() {
    let row = Row::default();
    for provider in 0..8 {
        let mut facts = known(&row);
        match provider {
            0 => facts.excluded = None,
            1 => facts.admitted = None,
            2 => facts.availability = None,
            3 => facts.metadata = None,
            4 => facts.row = Entry::Unknown,
            5 => facts.quest = None,
            6 => facts.role = None,
            _ => facts.greeting = None,
        }
        assert_eq!(compute(facts), None, "provider{provider}");
    }
    let mut specialty = Row::default();
    specialty.enabled[10] = true;
    let mut facts = known(&specialty);
    facts.specialty_extra = None;
    assert_eq!(
        compute(facts),
        None,
        "specialty does not establish the separate extra-vector provider"
    );
    let mut facts = known(&row);
    facts.quest = Some(true);
    facts.greeting = None;
    assert_eq!(
        compute(facts),
        Some(vec![13877]),
        "native quest branch does not query greeting"
    );
    let mut facts = known(&row);
    facts.row = Entry::Missing;
    facts.quest = None;
    assert_eq!(
        compute(facts),
        Some(vec![]),
        "known missing is distinct from unsupported lookup"
    );
    assert!(matches!(Catalog::default().get(1000), Entry::Unknown));
    assert!(Catalog::load(&Connection::open_in_memory().unwrap()).is_err());
}

#[test]
#[ignore = "read-only real catalog projection; does not supply runtime context/admission"]
fn candidate_native_real_catalog_rows_and_unsorted_custom_links()
-> Result<(), Box<dyn std::error::Error>> {
    let conn = Connection::open_with_flags(
        aa_anim::db::DEFAULT_DB,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let catalog = Catalog::load(&conn)?;
    let data = goldens();
    for case in data["catalog_rows"].as_array().unwrap() {
        let Entry::Loaded(row) = catalog.get(case["id"].as_u64().unwrap() as u32) else {
            panic!("missing fixture row")
        };
        assert_eq!(compute(known(row)), Some(values(&case["rows"])));
    }
    let sets: HashMap<u32, Vec<u32>> = data["custom_sets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| (v["id"].as_u64().unwrap() as u32, values(&v["skills"])))
        .collect();
    let mut query = conn.prepare("SELECT id,npc_interaction_set_id FROM npcs")?;
    let mut rows = query.query([])?;
    let mut linked = 0;
    while let Some(row) = rows.next()? {
        let id = crate::hud::data::any_int(row, 0)? as u32;
        let set = crate::hud::data::any_int(row, 1)? as u32;
        let Entry::Loaded(candidate) = catalog.get(id) else {
            panic!("lost loaded row")
        };
        let expected = sets.get(&set).map_or(&[][..], Vec::as_slice);
        assert_eq!(candidate.custom, expected, "NPC{id}, set{set}");
        linked += usize::from(!expected.is_empty());
    }
    assert!(linked > 0);
    Ok(())
}
