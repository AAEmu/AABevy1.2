//! How a row of the skill trace is written: the common fields, the normalisation of names and paths, and the conversion of
//! Bevy poses to game coordinates. Plain functions, no Bevy state, so the format is testable (`tools/skillparity/README.md`,
//! schema `skillparity/v1`, is the contract).

use bevy::math::{Quat, Vec3};

/// A name as both clients compare it: lower case, `/` for `\`.
pub fn name(text: &str) -> String {
    text.to_ascii_lowercase().replace('\\', "/")
}

/// A pak path as both clients compare it: [`name`] without the `game/` the pak adds in front of every file.
pub fn path(text: &str) -> String {
    let n = name(text);
    n.strip_prefix("game/").map(str::to_string).unwrap_or(n)
}

/// Rounds to `places` decimals so a row does not carry `f32` noise (`0.07999999821186066`).
fn round(v: f64, places: i32) -> f64 {
    let k = 10f64.powi(places);
    (v * k).round() / k
}

/// A Bevy world position as the game's: Z up, X east, Y north (`aa_world::bevy_to_cry`), to the millimetre.
pub fn game_pos(bevy: Vec3) -> [f64; 3] {
    let g = aa_world::bevy_to_cry(bevy);
    [round(g.x.into(), 3), round(g.y.into(), 3), round(g.z.into(), 3)]
}

/// The heading of a Bevy rotation in degrees: 0 = north, counter-clockwise (west is +90), in (-180, 180].
pub fn yaw_degrees(rotation: Quat) -> f32 {
    let forward = aa_world::bevy_to_cry(rotation * Vec3::NEG_Z);
    (-forward.x).atan2(forward.y).to_degrees()
}

/// One row, written field by field. The common fields come first, the rest in the order added.
pub struct Row(String);

impl Row {
    pub fn new(kind: &str, wall_us: u64, frame: u32) -> Row {
        Row(format!("{{\"src\":\"bevy\",\"wall_us\":{wall_us},\"frame\":{frame},\"kind\":{}", json(kind)))
    }

    fn key(mut self, key: &str) -> Row {
        self.0.push(',');
        self.0.push_str(&json(key));
        self.0.push(':');
        self
    }

    pub fn str(mut self, key: &str, value: &str) -> Row {
        self = self.key(key);
        self.0.push_str(&json(value));
        self
    }

    /// The field only when there is a value ("a field a client cannot know is left out, never guessed").
    pub fn opt_str(self, key: &str, value: Option<&str>) -> Row {
        match value {
            Some(v) => self.str(key, v),
            None => self,
        }
    }

    pub fn int(mut self, key: &str, value: i64) -> Row {
        self = self.key(key);
        self.0.push_str(&value.to_string());
        self
    }

    pub fn opt_int(self, key: &str, value: Option<i64>) -> Row {
        match value {
            Some(v) => self.int(key, v),
            None => self,
        }
    }

    /// A number with `places` decimals; a value that is not finite is left out.
    pub fn num(mut self, key: &str, value: f64, places: i32) -> Row {
        if !value.is_finite() {
            return self;
        }
        self = self.key(key);
        self.0.push_str(&json_number(round(value, places)));
        self
    }

    pub fn flag(mut self, key: &str, value: bool) -> Row {
        self = self.key(key);
        self.0.push_str(if value { "true" } else { "false" });
        self
    }

    /// `[x, y, z]` in game coordinates.
    pub fn pos(mut self, key: &str, at: [f64; 3]) -> Row {
        self = self.key(key);
        self.0.push_str(&format!("[{},{},{}]", json_number(at[0]), json_number(at[1]), json_number(at[2])));
        self
    }

    /// `[x, y, z]` when there is a position.
    pub fn opt_pos(self, key: &str, at: Option<[f64; 3]>) -> Row {
        match at {
            Some(at) => self.pos(key, at),
            None => self,
        }
    }

    /// A small object of integers (left out when empty).
    pub fn fields(mut self, key: &str, fields: &[(&str, i64)]) -> Row {
        if fields.is_empty() {
            return self;
        }
        self = self.key(key);
        self.0.push('{');
        for (i, (k, v)) in fields.iter().enumerate() {
            if i > 0 {
                self.0.push(',');
            }
            self.0.push_str(&format!("{}:{v}", json(k)));
        }
        self.0.push('}');
        self
    }

    /// The finished line (without the newline).
    pub fn line(mut self) -> String {
        self.0.push('}');
        self.0
    }
}

fn json(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| "\"\"".into())
}

fn json_number(v: f64) -> String {
    serde_json::Number::from_f64(v).map_or_else(|| "0".into(), |n| n.to_string())
}

/// `skill:14376:start` -> 14376 (the runner's labels, `tools/skillparity/README.md`, `mark`).
pub fn skill_of_label(label: &str) -> Option<u32> {
    let mut parts = label.split(':');
    (parts.next()? == "skill").then_some(())?;
    parts.next()?.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn parsed(line: &str) -> Value {
        serde_json::from_str(line).expect("a row is one JSON object")
    }

    #[test]
    fn names_are_lower_case_with_forward_slashes() {
        assert_eq!(name("Bow_CO_SK_Launch"), "bow_co_sk_launch");
        assert_eq!(name("Sounds\\X2Skills:Wild"), "sounds/x2skills:wild");
        assert_eq!(name("Abillity_Skill_Table_M.System_Skill.Return_Skill_Form"), "abillity_skill_table_m.system_skill.return_skill_form");
    }

    #[test]
    fn paths_lose_the_game_prefix() {
        assert_eq!(path("game/animations/nuian/male/bow/bow_co_sk_launch.caf"), "animations/nuian/male/bow/bow_co_sk_launch.caf");
        assert_eq!(path("Game\\Objects\\Effects\\Crow\\Crow.chr"), "objects/effects/crow/crow.chr");
        // only a leading `game/` is the pak's, not a folder further down
        assert_eq!(path("objects/game/x.cgf"), "objects/game/x.cgf");
    }

    #[test]
    fn positions_are_in_game_space() {
        // Bevy (x, y up, z south) -> game (x east, y north, z up)
        assert_eq!(game_pos(Vec3::new(10.0, 5.0, -20.0)), [10.0, 20.0, 5.0]);
        assert_eq!(game_pos(Vec3::new(15601.2, 126.3, -15380.0)), [15601.2, 15380.0, 126.3]);
        // rounded to the millimetre, no f32 noise
        assert_eq!(game_pos(Vec3::new(0.1, 0.2, 0.3))[0], 0.1);
    }

    #[test]
    fn yaw_is_degrees_counter_clockwise_from_north() {
        let yaw = |deg: f32| yaw_degrees(Quat::from_rotation_y(deg.to_radians()));
        assert!(yaw(0.0).abs() < 1e-4);
        assert!((yaw(90.0) - 90.0).abs() < 1e-3, "a quarter turn counter-clockwise faces west");
        assert!((yaw(-90.0) + 90.0).abs() < 1e-3);
        assert!((yaw(180.0).abs() - 180.0).abs() < 1e-3);
        // the character's own heading (`Mover::facing`) is the same number
        assert!((yaw(35.5) - 35.5).abs() < 1e-3);
        // facing north in game space is Bevy -Z
        assert_eq!(aa_world::bevy_to_cry(Vec3::NEG_Z), Vec3::Y);
    }

    #[test]
    fn a_row_has_the_common_fields_first_and_parses() {
        let line = Row::new("anim", 1_700_000_000_123_456, 42)
            .str("inst", "4294967298")
            .str("role", "player")
            .opt_str("clip", Some(&name("Bow_CO_SK_Launch")))
            .opt_str("file", None)
            .int("layer", 1)
            .num("speed", f64::from(1.0f32), 4)
            .num("blend", f64::from(0.08f32), 4)
            .flag("loop", false)
            .line();
        assert!(line.starts_with("{\"src\":\"bevy\",\"wall_us\":1700000000123456,\"frame\":42,\"kind\":\"anim\""), "{line}");
        let v = parsed(&line);
        assert_eq!(v["src"], "bevy");
        assert_eq!(v["wall_us"], 1_700_000_000_123_456u64);
        assert_eq!(v["frame"], 42);
        assert_eq!(v["kind"], "anim");
        assert_eq!(v["clip"], "bow_co_sk_launch");
        assert_eq!(v["layer"], 1);
        assert_eq!(v["speed"], 1.0);
        assert_eq!(v["blend"], 0.08, "{line}");
        assert_eq!(v["loop"], false);
        assert!(v.get("file").is_none(), "an unknown field is left out");
        assert!(!line.contains('\n'));
    }

    #[test]
    fn positions_and_fields_are_arrays_and_objects() {
        let line = Row::new("fx", 1, 2)
            .str("name", &name("Abillity_Skill_Table_M.System_Skill.Return_Skill_Form"))
            .str("phase", "spawn")
            .pos("pos", game_pos(Vec3::new(1.0, 2.0, -3.0)))
            .num("scale", f64::NAN, 3)
            .fields("fields", &[("cast_ms", 1500), ("result", 0)])
            .line();
        let v = parsed(&line);
        assert_eq!(v["pos"], serde_json::json!([1.0, 3.0, 2.0]));
        assert_eq!(v["fields"]["cast_ms"], 1500);
        assert!(v.get("scale").is_none(), "a number that is not finite is left out");
    }

    #[test]
    fn strings_are_escaped() {
        let line = Row::new("mark", 1, 1).str("label", "say \"hi\"\\ now\nok").line();
        assert_eq!(parsed(&line)["label"], "say \"hi\"\\ now\nok");
        assert!(!line.contains('\n'));
    }

    #[test]
    fn a_label_names_its_skill() {
        assert_eq!(skill_of_label("skill:14376:start"), Some(14376));
        assert_eq!(skill_of_label("skill:14376"), Some(14376));
        assert_eq!(skill_of_label("fight:14376:start"), None);
        assert_eq!(skill_of_label("skill:abc:start"), None);
        assert_eq!(skill_of_label("hello"), None);
    }
}
