//! Golden tests for the 1.2 vehicle move codecs.
//!
//! The goldens were recorded from the running 1.2 client by the vehicle lane
//! (`rust/tools/capture_vehicle_goldens.py`); the fixed-position cases come from
//! `capture_vehicle_phase_c_synth.py`. They are byte/bits exact, not capture
//! traces: the type-2 velocity scale (1092 vs the server's 2048) is still a
//! pending live check (see `docs/aa-net.md`).

use super::*;
use serde_json::Value;

fn json(name: &str) -> Value {
    let text = match name {
        "position" => include_str!("goldens/position.json"),
        "vel60" => include_str!("goldens/vel60.json"),
        "vel30" => include_str!("goldens/vel30.json"),
        "vel50" => include_str!("goldens/vel50.json"),
        "delta" => include_str!("goldens/delta.json"),
        "quat_write" => include_str!("goldens/quat_write.json"),
        "quat8_write" => include_str!("goldens/quat8_write.json"),
        "quat_read" => include_str!("goldens/quat_read.json"),
        "quat8_read" => include_str!("goldens/quat8_read.json"),
        "move" => include_str!("goldens/move.json"),
        "move_copy" => include_str!("goldens/move_copy.json"),
        "fixed" => include_str!("goldens/fixed.json"),
        other => panic!("unknown golden file {other}"),
    };
    serde_json::from_str(text).expect("golden JSON parses")
}

fn f32_of(v: &Value) -> f32 {
    let bits = v["bits"].as_str().expect("bits");
    f32::from_bits(u32::from_str_radix(bits.trim_start_matches("0x"), 16).expect("hex"))
}

fn hex_of(v: &Value) -> Vec<u8> {
    let s = v.as_str().expect("hex string");
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
        .collect()
}

struct MockArchive {
    reading: bool,
    scripted: Vec<(String, Vec<u8>)>,
    cursor: usize,
    calls: Vec<(String, Vec<u8>)>,
}

impl MockArchive {
    fn write() -> Self {
        Self {
            reading: false,
            scripted: Vec::new(),
            cursor: 0,
            calls: Vec::new(),
        }
    }

    fn read(entries: &[Value]) -> Self {
        Self {
            reading: true,
            scripted: entries
                .iter()
                .map(|e| {
                    (
                        e["key"].as_str().expect("key").to_string(),
                        hex_of(&e["value"]),
                    )
                })
                .collect(),
            cursor: 0,
            calls: Vec::new(),
        }
    }

    fn put(&mut self, key: &'static str, bytes: &mut [u8]) {
        if self.reading {
            let (k, v) = &self.scripted[self.cursor];
            assert_eq!(k, key, "read key order");
            assert_eq!(v.len(), bytes.len(), "read value size for {key}");
            bytes.copy_from_slice(v);
            self.cursor += 1;
        } else {
            self.calls.push((key.to_string(), bytes.to_vec()));
        }
    }
}

impl MoveArchive for MockArchive {
    fn is_reading(&self) -> bool {
        self.reading
    }
    fn u8(&mut self, key: &'static str, v: &mut u8) {
        let mut b = [*v];
        self.put(key, &mut b);
        *v = b[0];
    }
    fn u16(&mut self, key: &'static str, v: &mut u16) {
        let mut b = v.to_le_bytes();
        self.put(key, &mut b);
        *v = u16::from_le_bytes(b);
    }
    fn u32(&mut self, key: &'static str, v: &mut u32) {
        let mut b = v.to_le_bytes();
        self.put(key, &mut b);
        *v = u32::from_le_bytes(b);
    }
    fn i8(&mut self, key: &'static str, v: &mut i8) {
        let mut b = [*v as u8];
        self.put(key, &mut b);
        *v = b[0] as i8;
    }
    fn i16(&mut self, key: &'static str, v: &mut i16) {
        let mut b = v.to_le_bytes();
        self.put(key, &mut b);
        *v = i16::from_le_bytes(b);
    }
    fn i32(&mut self, key: &'static str, v: &mut i32) {
        let mut b = v.to_le_bytes();
        self.put(key, &mut b);
        *v = i32::from_le_bytes(b);
    }
    fn f32(&mut self, key: &'static str, v: &mut f32) {
        let mut b = v.to_le_bytes();
        self.put(key, &mut b);
        *v = f32::from_le_bytes(b);
    }
    fn f32x3(&mut self, key: &'static str, v: &mut [f32; 3]) {
        let mut b = [0u8; 12];
        for (i, x) in v.iter().enumerate() {
            b[i * 4..i * 4 + 4].copy_from_slice(&x.to_le_bytes());
        }
        self.put(key, &mut b);
        for (i, x) in v.iter_mut().enumerate() {
            *x = f32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap());
        }
    }
}

fn assert_calls(actual: &[(String, Vec<u8>)], expected: &[Value], what: &str) {
    assert_eq!(actual.len(), expected.len(), "{what}: call count");
    for (i, (got_key, got_value)) in actual.iter().enumerate() {
        let want_key = expected[i]["key"].as_str().expect("key");
        let want_value = hex_of(&expected[i]["value"]);
        assert_eq!(got_key, want_key, "{what}: call {i} key");
        assert_eq!(
            got_value, &want_value,
            "{what}: call {i} value ({want_key})"
        );
    }
}

#[test]
fn position_matches_goldens() {
    let doc = json("position");
    for (i, case) in doc.as_array().expect("cases").iter().enumerate() {
        let x = case["in"]["x"].as_i64().expect("x");
        let y = case["in"]["y"].as_i64().expect("y");
        let z = f32_of(&case["in"]["z"]);
        let pos = FixedPos { x, y, z };

        let mut ar = MockArchive::write();
        let mut written = pos;
        position_codec(&mut written, &mut ar, DEFAULT_CELL_SIZE);
        assert_calls(
            &ar.calls,
            case["calls"].as_array().expect("calls"),
            "position write",
        );
        let wire: Vec<u8> = ar.calls.iter().flat_map(|(_, v)| v.clone()).collect();
        assert_eq!(wire, hex_of(&case["out"]), "position {i}: wire");
        assert_eq!(
            write_position(pos, DEFAULT_CELL_SIZE).to_vec(),
            wire,
            "position {i}: direct"
        );

        let mut ar = MockArchive::read(case["calls"].as_array().expect("calls"));
        let mut decoded = FixedPos::default();
        position_codec(&mut decoded, &mut ar, DEFAULT_CELL_SIZE);
        let want = hex_of(&case["read"]);
        assert_eq!(decoded.x.to_le_bytes(), want[0..8], "position {i}: read x");
        assert_eq!(decoded.y.to_le_bytes(), want[8..16], "position {i}: read y");
        assert_eq!(
            decoded.z.to_le_bytes(),
            want[16..20],
            "position {i}: read z"
        );
        let wire_out = hex_of(&case["out"]);
        let mut wb = [0u8; 9];
        wb.copy_from_slice(&wire_out);
        let direct = read_position(&wb, DEFAULT_CELL_SIZE);
        assert_eq!(direct.x, decoded.x, "position {i}: direct read x");
        assert_eq!(direct.y, decoded.y, "position {i}: direct read y");
        assert_eq!(
            direct.z.to_bits(),
            decoded.z.to_bits(),
            "position {i}: direct read z"
        );
    }
}

fn check_velocity(name: &str, scale: f32) {
    let doc = json(name);
    for (i, case) in doc.as_array().expect("cases").iter().enumerate() {
        let mut v = [0.0f32; 3];
        for (j, item) in case["in"].as_array().expect("in").iter().enumerate() {
            v[j] = f32_of(item);
        }
        let raw = write_i16_vec3(v, scale);
        let mut bytes = Vec::new();
        for x in raw {
            bytes.extend_from_slice(&x.to_le_bytes());
        }
        assert_eq!(bytes, hex_of(&case["out"]), "{name} case {i}");
    }
}

#[test]
fn velocity_matches_goldens() {
    check_velocity("vel60", VEL_SCALE_TYPE1);
    check_velocity("vel30", VEL_SCALE_VEHICLE);
    check_velocity("vel50", VEL_SCALE_TYPE0);
}

#[test]
fn delta_matches_goldens() {
    let doc = json("delta");
    for (i, case) in doc.as_array().expect("cases").iter().enumerate() {
        let mut v = [0.0f32; 3];
        for (j, item) in case["in"].as_array().expect("in").iter().enumerate() {
            v[j] = f32_of(item);
        }
        let raw = write_delta_i8_vec3(v);
        let bytes: Vec<u8> = raw.iter().map(|x| *x as u8).collect();
        assert_eq!(bytes, hex_of(&case["out"]), "delta case {i}");
    }
}

#[test]
fn quat_write_matches_goldens() {
    let doc = json("quat_write");
    for (i, case) in doc.as_array().expect("cases").iter().enumerate() {
        let mut q = [0.0f32; 4];
        for (j, item) in case["in"].as_array().expect("in").iter().enumerate() {
            q[j] = f32_of(item);
        }
        let raw = write_quat_i16(q);
        let want = hex_of(&case["out"]);
        for j in 0..3 {
            let want = i16::from_le_bytes([want[j * 2], want[j * 2 + 1]]);
            // rsqrtss is vendor-specific in its last bits: allow one step
            assert!((i32::from(raw[j]) - i32::from(want)).abs() <= 1, "quat_write case {i}: {raw:?}");
        }
    }
}

#[test]
fn quat8_write_matches_goldens() {
    let doc = json("quat8_write");
    for (i, case) in doc.as_array().expect("cases").iter().enumerate() {
        let mut q = [0.0f32; 4];
        for (j, item) in case["in"].as_array().expect("in").iter().enumerate() {
            q[j] = f32_of(item);
        }
        let raw = write_quat_i8(q);
        let want = hex_of(&case["out"]);
        for j in 0..3 {
            // rsqrtss is vendor-specific in its last bits: allow one step
            assert!((i32::from(raw[j]) - i32::from(want[j] as i8)).abs() <= 1, "quat8_write case {i}: {raw:?}");
        }
    }
}

#[test]
fn quat_read_matches_goldens() {
    let doc = json("quat_read");
    for (i, case) in doc.as_array().expect("cases").iter().enumerate() {
        let b = hex_of(&case["in"]);
        let mut raw = [0i16; 3];
        for j in 0..3 {
            raw[j] = i16::from_le_bytes([b[j * 2], b[j * 2 + 1]]);
        }
        let q = read_quat_i16(raw);
        for j in 0..4 {
            let want = f32_of(&case["out"][j]);
            assert_eq!(
                q[j].to_bits(),
                want.to_bits(),
                "quat_read case {i} component {j}"
            );
        }
    }
}

#[test]
fn quat8_read_matches_goldens() {
    let doc = json("quat8_read");
    for (i, case) in doc.as_array().expect("cases").iter().enumerate() {
        let b = hex_of(&case["in"]);
        let raw = [b[0] as i8, b[1] as i8, b[2] as i8];
        let q = read_quat_i8(raw);
        for j in 0..4 {
            let want = f32_of(&case["out"][j]);
            assert_eq!(
                q[j].to_bits(),
                want.to_bits(),
                "quat8_read case {i} component {j}"
            );
        }
    }
}

#[test]
fn move_matches_goldens() {
    let doc = json("move");
    for (i, case) in doc.as_array().expect("cases").iter().enumerate() {
        let input = hex_of(&case["input"]);
        let mut mv = MoveUnit::default();
        mv.bytes.copy_from_slice(&input);

        let mut ar = MockArchive::write();
        serialize_move(&mut mv, &mut ar);
        assert_calls(
            &ar.calls,
            case["calls"].as_array().expect("calls"),
            "move write",
        );

        let mut ar = MockArchive::read(case["calls"].as_array().expect("calls"));
        let mut read = MoveUnit::default();
        serialize_move(&mut read, &mut ar);
        assert_eq!(
            ar.cursor,
            ar.scripted.len(),
            "move {i}: all scripted reads consumed"
        );
        assert_eq!(
            read.bytes.as_slice(),
            hex_of(&case["read"]).as_slice(),
            "move {i}: read bytes"
        );
    }
}

#[test]
fn move_copy_matches_goldens() {
    let doc = json("move_copy");
    for (i, case) in doc.as_array().expect("cases").iter().enumerate() {
        let input = hex_of(&case["input"]);
        let mut src = MoveUnit::default();
        src.bytes.copy_from_slice(&input);
        let mut dst = MoveUnit {
            bytes: [0xAA; MOVE_UNIT_SIZE],
        };
        copy_move(&mut dst, &src);
        assert_eq!(
            dst.bytes.as_slice(),
            hex_of(&case["out"]).as_slice(),
            "move_copy case {i}"
        );
    }
}

#[test]
fn fixed_world_scale_matches_goldens() {
    let doc = json("fixed");
    let cases = doc["cases"].as_array().expect("cases");
    assert_eq!(cases.len(), 20);
    for c in cases {
        let x = c["x"].as_i64().unwrap();
        let y = c["y"].as_i64().unwrap();
        let z = f32_of(&c["z"]);
        let p = FixedPos { x, y, z };
        let got = fixed_to_world(&p);
        let want: Vec<u64> = c["out_bits"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| u64::from_str_radix(v.as_str().unwrap().trim_start_matches("0x"), 16).unwrap())
            .collect();
        assert_eq!(got.x.to_bits(), want[0], "fixed case {} x", c["i"]);
        assert_eq!(got.y.to_bits(), want[1], "fixed case {} y", c["i"]);
        assert_eq!(got.z.to_bits(), want[2], "fixed case {} z", c["i"]);
        assert_eq!(FIXED_SCALE, FIXED_SCALE_EXACT);
        assert_eq!(WIRE_STEP_M, 8.0 * 2f64.powi(32) * FIXED_SCALE);
    }
}
