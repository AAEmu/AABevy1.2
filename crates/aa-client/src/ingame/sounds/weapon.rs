//! Native weapon-sound selection and initial parameters (x2game 393913c0 / 393f2270).
//! Attack material belongs to the received impact; twohand belongs to the current mainhand.

use std::collections::HashMap;
use std::sync::Arc;

use aa_audio::Sound;
use aa_net::model::{Unit, slot};
use aa_net::skill::UnitDamage;
use rusqlite::{Connection, params};

pub(super) struct Target {
    pub model: u32,
    pub mainhand: u32,
    pub offhand: u32,
}

impl From<&Unit> for Target {
    fn from(unit: &Unit) -> Self {
        let item = |slot| unit.equipment.get(slot).map_or(0, |item| item.template_id);
        Self {
            model: unit.model_id,
            mainhand: item(slot::MAINHAND),
            offhand: item(slot::OFFHAND),
        }
    }
}

#[derive(Default)]
pub(super) struct Catalog {
    holdables: HashMap<u8, u32>,
    weapons: HashMap<u32, Option<u32>>,
    twohand: HashMap<u32, bool>,
    models: HashMap<u32, u32>,
}

impl Catalog {
    pub fn sound(
        &mut self,
        conn: &Connection,
        paths: &HashMap<(u32, u32), Arc<str>>,
        impact: &UnitDamage,
        mainhand: u32,
        target: Target,
    ) -> Option<Sound> {
        // This is narrower than the generic damage-number / hit-reaction classification.
        if impact.action.kind > 1
            || !matches!(impact.hit_type, 1 | 3 | 4 | 6 | 7 | 9 | 10 | 11 | 17 | 20)
        {
            return None;
        }
        let source = *self.holdables.entry(impact.holdable_id).or_insert_with(|| {
            conn.query_row(
                "SELECT sound_material_id FROM holdables WHERE id = ?1",
                params![impact.holdable_id],
                |r| r.get::<_, Option<u32>>(0),
            )
            .ok()
            .flatten()
            .unwrap_or(21)
        });
        let target = match impact.hit_type {
            6 | 17 => self.weapon_material(conn, target.offhand).unwrap_or(0),
            7 | 20 => self.weapon_material(conn, target.mainhand).unwrap_or(21),
            _ => *self.models.entry(target.model).or_insert_with(|| {
                conn.query_row(
                    "SELECT sound_material_id FROM models WHERE id = ?1",
                    params![target.model],
                    |r| r.get::<_, Option<u32>>(0),
                )
                .ok()
                .flatten()
                .unwrap_or(10)
            }),
        };
        let path = paths.get(&(source, target))?;
        let twohand = *self.twohand.entry(mainhand).or_insert_with(|| {
            conn.query_row("SELECT h.slot_type_id FROM item_weapons w JOIN holdables h ON h.id = w.holdable_id WHERE w.item_id = ?1", params![mainhand], |r| r.get::<_, Option<u32>>(0))
                .ok().flatten() == Some(16)
        });
        let pierce = match impact.pierce_selector() {
            1 => 1.0,
            2 => 0.3,
            _ => 0.0,
        };
        Some(
            Sound::event(path.to_string())
                .with_param("pierce", pierce)
                .with_param(
                    "miss",
                    u8::from(matches!(impact.hit_type, 4 | 10 | 14)) as f32,
                )
                .with_param(
                    "critical",
                    u8::from(matches!(impact.hit_type, 3 | 11)) as f32,
                )
                .with_param("twohand", u8::from(twohand) as f32),
        )
    }

    fn weapon_material(&mut self, conn: &Connection, item: u32) -> Option<u32> {
        *self.weapons.entry(item).or_insert_with(|| {
            conn.query_row("SELECT h.sound_material_id FROM item_weapons w JOIN holdables h ON h.id = w.holdable_id WHERE w.item_id = ?1", params![item], |r| r.get::<_, Option<u32>>(0))
                .ok().flatten()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aa_net::skill::CastAction;

    fn impact(hit_type: u8, holdable_id: u8, selector: u16) -> UnitDamage {
        UnitDamage {
            action: CastAction {
                kind: 0,
                skill_id: 3,
                tl: 5,
                buff_id: 0,
            },
            caster: aa_net::buff::BuffCaster { kind: 0, obj_id: 1 },
            caster_obj_id: 1,
            target_obj_id: 2,
            damage: 0,
            absorbed: 0,
            mana_burn: 0,
            hit_type,
            holdable_id,
            raw_hit: (selector << 12) | 0x0120 | u16::from(hit_type),
        }
    }

    fn target() -> Target {
        Target {
            model: 1035,
            mainhand: 5569,
            offhand: 6152,
        }
    }

    fn fixture() -> (Connection, HashMap<(u32, u32), Arc<str>>) {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE holdables(id INTEGER,slot_type_id INTEGER,sound_material_id INTEGER);
            CREATE TABLE item_weapons(item_id INTEGER,holdable_id INTEGER);
            CREATE TABLE models(id INTEGER,sound_material_id INTEGER);
            INSERT INTO holdables VALUES(3,16,16),(6,17,16),(17,16,14),(24,17,15);
            INSERT INTO item_weapons VALUES(5660,3),(5569,6),(6102,17),(6152,24);
            INSERT INTO models VALUES(1035,9);",
        )
        .unwrap();
        let paths = [
            ((16, 9), "sword"),
            ((16, 15), "shield"),
            ((16, 16), "parry"),
            ((14, 9), "staff"),
        ]
        .into_iter()
        .map(|(pair, path)| (pair, Arc::from(path)))
        .collect();
        (conn, paths)
    }

    #[test]
    fn received_material_survives_equipment_changes_until_impact() {
        let (conn, paths) = fixture();
        let mut catalog = Catalog::default();
        let received = impact(3, 3, 2);
        let before = catalog
            .sound(&conn, &paths, &received, 5660, target())
            .unwrap();
        let swapped = catalog
            .sound(&conn, &paths, &received, 5569, target())
            .unwrap();
        let staff_now = catalog
            .sound(&conn, &paths, &received, 6102, target())
            .unwrap();
        assert_eq!(before.event, "sword");
        assert_eq!(
            swapped.event, before.event,
            "attack holdable belongs to receipt, not current equipment"
        );
        assert_eq!(staff_now.event, before.event);
        assert_eq!(
            before.params,
            [
                ("pierce".into(), 0.3),
                ("miss".into(), 0.0),
                ("critical".into(), 1.0),
                ("twohand".into(), 1.0)
            ]
        );
        assert_eq!(swapped.params[3].1, 0.0);
        assert_eq!(
            staff_now.params[3].1, 1.0,
            "native sound includes two-handed staves"
        );
        let next = catalog
            .sound(&conn, &paths, &impact(1, 17, 0), 6102, target())
            .unwrap();
        assert_eq!(next.event, "staff");
        assert_eq!(
            next.params[2].1, 0.0,
            "a following normal hit cannot inherit the earlier critical"
        );
    }

    #[test]
    fn audio_admission_is_independent_of_hurt_fx_and_damage_amount() {
        let (conn, paths) = fixture();
        let mut catalog = Catalog::default();
        let missed = catalog
            .sound(&conn, &paths, &impact(4, 3, 0), 5660, target())
            .unwrap();
        assert_eq!(
            missed.params[1].1, 1.0,
            "zero-damage miss is still a weapon sound"
        );
        assert_eq!(
            catalog
                .sound(&conn, &paths, &impact(6, 3, 0), 5660, target())
                .unwrap()
                .event,
            "shield"
        );
        assert_eq!(
            catalog
                .sound(&conn, &paths, &impact(7, 3, 0), 5660, target())
                .unwrap()
                .event,
            "parry"
        );
        for hit in [0, 2, 5, 8, 12, 13, 14, 15, 16, 18, 19, 21, 255] {
            assert!(
                catalog
                    .sound(&conn, &paths, &impact(hit, 3, 0), 5660, target())
                    .is_none(),
                "hit {hit}"
            );
        }
        let mut buff_tick = impact(3, 3, 0);
        buff_tick.action.kind = 2;
        assert!(
            catalog
                .sound(&conn, &paths, &buff_tick, 5660, target())
                .is_none()
        );
    }

    #[test]
    #[ignore = "requires the original read-only game catalog"]
    fn original_native_selection_and_parameter_cases() {
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.into());
        let conn =
            Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let paths = super::super::read_blows(&conn);
        let goldens: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tools/evidence/combat_sound/native-producer-cases.json"
        )))
        .unwrap();
        let mut catalog = Catalog::default();
        let cases = goldens["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 336);
        for c in cases {
            let damage = impact(
                c["hit_type"].as_u64().unwrap() as u8,
                c["context_holdable"].as_u64().unwrap() as u8,
                c["high_nibble"].as_u64().unwrap() as u16,
            );
            let sound = catalog.sound(
                &conn,
                &paths,
                &damage,
                c["mainhand_item"].as_u64().unwrap() as u32,
                target(),
            );
            if c["selection"].is_null() {
                assert!(sound.is_none(), "{c}");
                continue;
            }
            let sound = sound.unwrap();
            let path: String = conn
                .query_row(
                    "SELECT path FROM sounds WHERE id=?1",
                    params![c["selection"]["sound_id"].as_u64().unwrap() as u32],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(sound.event, path.trim(), "{c}");
            let expected: Vec<(String, f32)> = c["parameters"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| (p[0].as_str().unwrap().into(), p[1].as_f64().unwrap() as f32))
                .collect();
            assert_eq!(sound.params, expected, "{c}");
        }
    }

    #[test]
    #[ignore = "requires the original catalog and game pak; CPU only"]
    fn original_critical_impact_reaches_the_celt_layer() {
        use aa_audio::library::{Library, PickState, Rng, initial_values, layer_gain, plan};
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.into());
        let conn =
            Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let paths = super::super::read_blows(&conn);
        let mut catalog = Catalog::default();
        for (hit, mainhand, selected, audible) in [
            (3, 5660, true, true),
            (1, 5660, false, false),
            (4, 5660, false, false),
            (3, 5569, true, false),
        ] {
            let sound = catalog
                .sound(&conn, &paths, &impact(hit, 3, 0), mainhand, target())
                .unwrap();
            assert_eq!(sound.event, "sounds/x2weapons_sword:sword:sword_wind");
            let event = Library::global().event(&sound.event).unwrap();
            let values = initial_values(event.event(), Some(1.0), &sound.params);
            let planned = plan(&event, &values, &mut PickState::default(), &mut Rng::new(3));
            let celt = planned.iter().find(|p| p.layer == 6);
            assert_eq!(celt.is_some(), selected, "hit{hit}/mainhand{mainhand}");
            assert_eq!(
                celt.is_some() && layer_gain(event.event(), &values, 6) > 0.99,
                audible
            );
            if audible {
                let part = celt.unwrap();
                assert_eq!((part.bank, part.index), (54, 0));
                assert_eq!(part.name, "weapons2/sword_2h/sword_2h_wind_crit.wav");
                let bytes = aa_pak::Pak::global()
                    .read("sounds/x2weapons_sword/x2weapons_sword_bank055.fsb")
                    .unwrap();
                let bank = aa_audio::Fsb5::parse(&bytes).unwrap();
                assert_eq!(bank.codec, aa_audio::fsb5::Codec::Celt);
                let pcm = aa_audio::decode(&bank, &bytes, part.index as usize).unwrap();
                assert_eq!((pcm.rate, pcm.channels, pcm.frames()), (44100, 1, 116224));
                assert!((0.17..0.18).contains(&pcm.rms()));
            }
        }
    }
}
