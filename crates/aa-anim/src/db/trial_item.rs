use super::*;

impl GameDb {
    /// Catalog metadata and constructor durability for original modelview `EquipItem`.
    /// This follows the widget's PE-initial grade byte 0, without fixed-grade/icon policy;
    /// it makes no claim that arbitrary indirect live writers cannot change that byte.
    /// Missing subtype data preserves the raw descriptor and lack of admission. Required
    /// constructor dependencies and database errors propagate. No actor, Item, clock or
    /// preview state is created or changed here.
    pub fn modelview_trial_item(&self, item_id: u32) -> Result<Option<ModelviewTrialItem>> {
        if item_id == 0 {
            return Ok(None);
        }
        let descriptor = self
            .conn
            .query_row(
                "SELECT impl_id, char_gender_id, bind_id FROM items WHERE id = ?1",
                [item_id],
                |r| {
                    Ok((
                        r.get::<_, Option<u32>>(0)?.unwrap_or(0),
                        r.get::<_, Option<u32>>(1)?.unwrap_or(0),
                        r.get::<_, Option<u32>>(2)?.unwrap_or(0),
                    ))
                },
            )
            .optional()
            .map_err(db_err)?;
        let Some((kind, gender, bind_id)) = descriptor else {
            return Ok(None);
        };
        let mut item = ModelviewTrialItem {
            kind,
            gender,
            bind_id,
            slot_type: None,
            backpack_type: None,
            default_dyeing_item_id: 0,
            durability: 0,
            charge_count: 0,
            recharge_buff_id: 0,
        };
        match kind {
            1 => {
                let weapon = self.conn.query_row(
                    "SELECT holdable_id, charge_count, recharge_buff_id FROM item_weapons WHERE item_id = ?1", [item_id],
                    |r| Ok((r.get::<_, Option<u32>>(0)?.unwrap_or(0), r.get::<_, Option<i32>>(1)?.unwrap_or(0) as u16, r.get::<_, Option<u32>>(2)?.unwrap_or(0))),
                ).optional().map_err(db_err)?;
                if let Some((holdable, count, recharge)) = weapon {
                    item.charge_count = count;
                    item.recharge_buff_id = recharge;
                    item.slot_type = self
                        .conn
                        .query_row(
                            "SELECT slot_type_id FROM holdables WHERE id = ?1",
                            [holdable],
                            |r| Ok(r.get::<_, Option<u32>>(0)?.unwrap_or(0)),
                        )
                        .optional()
                        .map_err(db_err)?;
                }
            }
            2 => {
                let armor = self.conn.query_row(
                    "SELECT slot_type_id, charge_count, recharge_buff_id FROM item_armors WHERE item_id = ?1", [item_id],
                    |r| Ok((r.get::<_, Option<u32>>(0)?.unwrap_or(0), r.get::<_, Option<i32>>(1)?.unwrap_or(0) as u16, r.get::<_, Option<u32>>(2)?.unwrap_or(0))),
                ).optional().map_err(db_err)?;
                if let Some((slot, count, recharge)) = armor {
                    item.slot_type = Some(slot);
                    item.charge_count = count;
                    item.recharge_buff_id = recharge;
                    item.default_dyeing_item_id = self
                        .conn
                        .query_row(
                            "SELECT default_dyeing_item_id FROM dyeable_items WHERE item_id = ?1",
                            [item_id],
                            |r| Ok(r.get::<_, Option<u32>>(0)?.unwrap_or(0)),
                        )
                        .optional()
                        .map_err(db_err)?
                        .unwrap_or(0);
                }
            }
            22 => {
                item.backpack_type = self
                    .conn
                    .query_row(
                        "SELECT backpack_type_id FROM item_backpacks WHERE item_id = ?1",
                        [item_id],
                        |r| Ok(r.get::<_, Option<u32>>(0)?.unwrap_or(0)),
                    )
                    .optional()
                    .map_err(db_err)?;
                item.slot_type = item.backpack_type.map(|_| 30);
            }
            _ => {}
        }
        // 3956f2e0 must map the raw slot before the widget calls the constructor.
        // This only controls whether the scalar is meaningful; it performs no trial update.
        if item
            .slot_type
            .is_some_and(|s| matches!(s, 1..=18 | 20..=21 | 23..=31))
        {
            item.durability = self.trial_durability(item_id)?;
        }
        Ok(Some(item))
    }

    fn trial_durability(&self, item_id: u32) -> Result<u8> {
        // Original397ce6b0 resolves armor, accessory, then weapon metadata in that order,
        // independently of the descriptor's kind. Keep even that fallback precedence.
        let armor = self.conn.query_row(
            "SELECT type_id, slot_type_id, durability_multiplier FROM item_armors WHERE item_id = ?1", [item_id],
            |r| Ok((r.get::<_, Option<u32>>(0)?.unwrap_or(0), r.get::<_, Option<u32>>(1)?.unwrap_or(0), r.get::<_, Option<i32>>(2)?.unwrap_or(0))),
        ).optional().map_err(db_err)?;
        let wearable = match armor {
            Some(row) => Some(row),
            None => self.conn.query_row(
                "SELECT type_id, slot_type_id, durability_multiplier FROM item_accessories WHERE item_id = ?1", [item_id],
                |r| Ok((r.get::<_, Option<u32>>(0)?.unwrap_or(0), r.get::<_, Option<u32>>(1)?.unwrap_or(0), r.get::<_, Option<i32>>(2)?.unwrap_or(0))),
            ).optional().map_err(db_err)?,
        };
        let (ratio, coverage, multiplier) = if let Some((kind, slot, multiplier)) = wearable {
            let ratio = self
                .conn
                .query_row(
                    "SELECT durability_ratio FROM wearable_kinds WHERE armor_type_id = ?1",
                    [kind],
                    native_float,
                )
                .map_err(db_err)?;
            let coverage = self
                .conn
                .query_row(
                    "SELECT coverage FROM wearable_slots WHERE slot_type_id = ?1",
                    [slot],
                    |r| Ok(r.get::<_, Option<i32>>(0)?.unwrap_or(0)),
                )
                .map_err(db_err)?;
            (ratio, Some(coverage), multiplier)
        } else {
            let weapon = self.conn.query_row(
                "SELECT holdable_id, durability_multiplier FROM item_weapons WHERE item_id = ?1", [item_id],
                |r| Ok((r.get::<_, Option<u32>>(0)?.unwrap_or(0), r.get::<_, Option<i32>>(1)?.unwrap_or(0))),
            ).optional().map_err(db_err)?;
            // Proven397ce8ee..397cea58 fallback (including the ordinary backpack fixtures).
            let Some((holdable, multiplier)) = weapon else {
                return Ok(0);
            };
            let ratio = self
                .conn
                .query_row(
                    "SELECT durability_ratio FROM holdables WHERE id = ?1",
                    [holdable],
                    native_float,
                )
                .map_err(db_err)?;
            (ratio, None, multiplier)
        };
        let (durability_const, holdable_const, wearable_const) = self.conn.query_row(
            "SELECT durability_const, holdable_durability_const, wearable_durability_const FROM item_configs WHERE id = 1", [],
            |r| Ok((native_float(r)?, native_float_at(r, 1)?, native_float_at(r, 2)?)),
        ).map_err(db_err)?;
        let grade = self
            .conn
            .query_row(
                "SELECT durability_value FROM item_grades WHERE id = 0",
                [],
                native_float,
            )
            .map_err(db_err)?;
        let base = durability_base(ratio, coverage, holdable_const, wearable_const, grade)?;
        Ok(durability_byte(base, durability_const, multiplier))
    }
}

fn native_float(r: &rusqlite::Row<'_>) -> rusqlite::Result<f32> {
    native_float_at(r, 0)
}

fn native_float_at(r: &rusqlite::Row<'_>, column: usize) -> rusqlite::Result<f32> {
    // Original loader stores SQLite double to f32. A known NULL scalar converts to zero;
    // an absent dependency row, invalid SQLite type or nonfinite f32 does not become zero.
    let value = r.get::<_, Option<f64>>(column)?.unwrap_or(0.0) as f32;
    if !value.is_finite() {
        return Err(rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Real,
            "nonfinite modelview durability input".into(),
        ));
    }
    Ok(value)
}

fn durability_base(
    ratio: f32,
    coverage: Option<i32>,
    holdable_const: f32,
    wearable_const: f32,
    grade: f32,
) -> Result<f64> {
    let ratio = native_floor_u32(pc64_scale_bias(ratio, 1000.0)?);
    let grade = native_floor_u32(pc64_scale_bias(grade, 1000.0)?);
    let (base, scale) = if let Some(coverage) = coverage {
        // 397ce6be..723: separate MULSS/ADDSS, then the original f32 floor.
        let config = ((wearable_const * 100.0) + 0.5).floor() as f64;
        (
            pc64_product(coverage as f64, config, ratio as f64)?,
            f64::from_bits(0x3ddb_7cdf_d9d7_bdbb),
        )
    } else {
        // 397ce735 stores MULSS first; the +.5/floor here is x87 -> double.
        let config = pc64_scale_bias(holdable_const * 100.0, 1.0)?.floor();
        (
            pc64_product(config, ratio as f64, 1.0)?,
            f64::from_bits(0x3e45_798e_e230_8c3a),
        )
    };
    // Both original FSTP-double boundaries matter; keep PC64 between multiplications.
    pc64_product(grade as f64, base, scale)
}

fn native_floor_u32(value: f64) -> u32 {
    let value = value.floor();
    // Original FISTP i64 uses truncation, then reads the low32 as unsigned.
    // Masked invalid conversion produces i64::MIN (low32 zero), not saturation.
    if (-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&value) {
        value as i64 as u32
    } else {
        0
    }
}

fn cvtt_i32(value: f32) -> i32 {
    if (-2_147_483_648.0..2_147_483_648.0).contains(&value) {
        value as i32
    } else {
        i32::MIN
    }
}

fn durability_byte(base: f64, durability_const: f32, multiplier: i32) -> u8 {
    if !(0.0..=255.0).contains(&base) {
        return 0;
    }
    let first = cvtt_i32((base as i32 as f32) * durability_const);
    if multiplier == 100 {
        return first as u8;
    }
    // Original MOVZX AL occurs before this multiplication; a Rust float-to-u8
    // cast would saturate, and applying the multiplier before narrowing is wrong.
    let factor = (multiplier as f32) * f32::from_bits(0x3c23_d70a);
    cvtt_i32(factor * (first as u8 as f32)) as u8
}

// The source/retained oracle uses x87 PC64 round-to-nearest, with explicit double
// stores. Use that arithmetic directly, restoring the caller's control word within
// each balanced block; ordinary Rust f64 multiplies would silently substitute PC53.
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn pc64_scale_bias(value: f32, scale: f32) -> Result<f64> {
    let (mut saved, control, half, mut out) = (0_u16, 0x037f_u16, 0.5_f32, 0.0_f64);
    // SAFETY: all pointers refer to live locals; the x87 stack and control word are restored.
    unsafe {
        std::arch::asm!(
            "fnstcw word ptr [{saved}]", "fldcw word ptr [{control}]",
            "fld dword ptr [{value}]", "fmul dword ptr [{scale}]", "fadd dword ptr [{half}]",
            "fstp qword ptr [{out}]", "fldcw word ptr [{saved}]",
            saved = in(reg) &mut saved, control = in(reg) &control, value = in(reg) &value,
            scale = in(reg) &scale, half = in(reg) &half, out = in(reg) &mut out,
            options(nostack, preserves_flags),
        );
    }
    Ok(out)
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn pc64_product(a: f64, b: f64, c: f64) -> Result<f64> {
    let (mut saved, control, mut out) = (0_u16, 0x037f_u16, 0.0_f64);
    // SAFETY: all pointers refer to live locals; the x87 stack and control word are restored.
    unsafe {
        std::arch::asm!(
            "fnstcw word ptr [{saved}]", "fldcw word ptr [{control}]",
            "fld qword ptr [{a}]", "fmul qword ptr [{b}]", "fmul qword ptr [{c}]",
            "fstp qword ptr [{out}]", "fldcw word ptr [{saved}]",
            saved = in(reg) &mut saved, control = in(reg) &control, a = in(reg) &a,
            b = in(reg) &b, c = in(reg) &c, out = in(reg) &mut out,
            options(nostack, preserves_flags),
        );
    }
    Ok(out)
}

#[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
fn pc64_scale_bias(_: f32, _: f32) -> Result<f64> {
    Err(Error::Format(
        "modelview constructor requires x87 PC64 arithmetic".into(),
    ))
}

#[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
fn pc64_product(_: f64, _: f64, _: f64) -> Result<f64> {
    Err(Error::Format(
        "modelview constructor requires x87 PC64 arithmetic".into(),
    ))
}

#[cfg(test)]
mod tests;
