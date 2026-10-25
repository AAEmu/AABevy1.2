//! The original six guard-buff metadata tables. Runtime inputs must be received.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Inputs {
    pub tower: u32,
    pub buffs: Vec<u32>,
    pub gates: i32,
    pub walls: i32,
}

struct Row {
    id: u32,
    name: String,
    path: String,
    description: String,
    kind: &'static str,
    current: bool,
    step: i8,
    counts: Option<(i32, i8, i32, i8)>,
}

fn metadata(data: &HudData, inputs: &Inputs, id: u32, step: i8) -> Option<Row> {
    // These are the six IDs supported by the original guard atlas. Their full
    // native394219f0 builder was executed with actual localized descriptors;
    // no dynamic formula path occurs for these authored descriptions.
    if !matches!(id, 4771..=4774 | 5392 | 5393) {
        return None;
    }
    let info = data.game.get()?.buffs.get(&id)?;
    if info.desc.contains("#{") {
        return None;
    }
    Some(Row {
        id,
        name: info.named.name.clone(),
        path: info.named.icon.clone(),
        description: info.desc.clone(),
        kind: if info.kind == 1 { "buff" } else { "debuff" },
        current: inputs.buffs.contains(&id),
        step,
        counts: None,
    })
}

fn project(data: &HudData) -> Option<(Vec<Row>, i8, i8)> {
    let context = data.territory.context?;
    let received = row(data, Value::Integer(i64::from(context.zone)))?;
    let inputs = data
        .territory
        .guard
        .as_ref()
        .filter(|input| input.tower == received.obj_id)?;
    let game = data.game.get()?;
    let mut rows = Vec::new();
    for step in game
        .territory
        .steps
        .iter()
        .filter(|step| step.setting == received.territory.id)
    {
        // Original skips a missing step buff descriptor. An unavailable full
        // builder for a present descriptor withholds this integration's view.
        if !game.buffs.contains_key(&step.buff) {
            continue;
        }
        if rows.is_empty() {
            rows.push(metadata(data, inputs, received.territory.id2, 0)?);
        }
        let mut current = metadata(data, inputs, step.buff, step.step)?;
        current.counts = Some((inputs.gates, step.gates, inputs.walls, step.walls));
        rows.push(current);
    }
    Some((
        rows,
        received.territory.max_gates as i8,
        received.territory.max_walls as i8,
    ))
}

pub(super) fn install(lua: &Lua, state: &Shared) -> mlua::Result<()> {
    let api: Table = lua.globals().get("X2House")?;
    let state = state.clone();
    api.set(
        "GetGuardTowerStepInfo",
        lua.create_function(move |lua, _: Value| {
            // Finish reading the bridge before calling the Lua localization API.
            let projected = { project(&state.borrow()) };
            let Some((rows, gates, walls)) = projected else {
                return Ok(MultiValue::new());
            };
            let locale: Table = lua.globals().get("X2Locale")?;
            let localize: mlua::Function = locale.get("LocalizeUiText")?;
            let out = lua.create_table()?;
            for row in rows {
                let category: Value = localize.call((locale.clone(), 33, row.kind))?;
                let value = lua.create_table()?;
                value.raw_set("description", row.description)?;
                value.raw_set("category", category)?;
                value.raw_set("tipType", row.kind)?;
                value.raw_set("name", row.name)?;
                value.raw_set("buff_id", row.id)?;
                value.raw_set("path", row.path)?;
                value.raw_set("step", row.step)?;
                value.raw_set("isCurStep", row.current)?;
                if let Some((gates, max_gates, walls, max_walls)) = row.counts {
                    value.raw_set("curGate", gates)?;
                    value.raw_set("maxGate", max_gates)?;
                    value.raw_set("curWall", walls)?;
                    value.raw_set("maxWall", max_walls)?;
                }
                out.raw_push(value)?;
            }
            Ok(MultiValue::from_iter([
                Value::Table(out),
                Value::Number(f64::from(gates)),
                Value::Number(f64::from(walls)),
            ]))
        })?,
    )
}
