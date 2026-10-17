//! Diagnostic-only selection of the original dock assembly; no aa-world dependency.
use bevy::prelude::*;

pub struct Part {
    pub model: String,
    pub material: String,
    pub local: Mat4,
}

pub fn parts() -> Vec<Part> {
    const CELL: &str = "worlds/main_world/cells/018_007/client/";
    let read = |name| aa_cry::read(&format!("{CELL}{name}")).unwrap();
    let table = |b: &[u8], stride, skip| -> Vec<String> {
        let count = u32::from_le_bytes(b[..4].try_into().unwrap()) as usize;
        (0..count)
            .map(|i| {
                let s = &b[4 + i * stride + skip..4 + (i + 1) * stride];
                String::from_utf8_lossy(s.split(|&c| c == 0).next().unwrap())
                    .trim()
                    .to_ascii_lowercase()
            })
            .collect()
    };
    let models = table(&read("statobjs.dat"), 256, 0);
    let materials = table(&read("material_list.dat"), 260, 4);
    let data = read("brush.dat");
    let u32_at = |b: &[u8], at| u32::from_le_bytes(b[at..at + 4].try_into().unwrap());
    assert_eq!(u32_at(&data, 0), 1);
    let mut parts = Vec::new();
    for sector in 0..256 {
        let off = u32_at(&data, 4 + sector * 4) as usize;
        let size = u32_at(&data, 1028 + sector * 4) as usize;
        for b in data[off..off + size].chunks_exact(128) {
            let model = &models[u32_at(b, 123) as usize];
            if !model.contains("/galleon/") {
                continue;
            }
            let material = &materials[u32_at(b, 115) as usize];
            let mut m: [f32; 12] = std::array::from_fn(|i| {
                f32::from_le_bytes(b[67 + i * 4..71 + i * 4].try_into().unwrap())
            });
            m[3] += (sector / 16) as f32 * 64.0;
            m[7] += (sector % 16) as f32 * 64.0;
            parts.push(Part {
                model: model.clone(),
                material: format!("{material}.mtl"),
                local: Mat4::from_cols_array(&[
                    m[0], m[4], m[8], 0.0, m[1], m[5], m[9], 0.0, m[2], m[6], m[10], 0.0, m[3],
                    m[7], m[11], 1.0,
                ]),
            });
        }
    }
    let anchor = parts
        .iter()
        .find(|p| p.model.ends_with("galleon_deco_sail01.cgf"))
        .unwrap()
        .local;
    let origin = anchor.transform_point3(Vec3::ZERO);
    parts.retain(|p| p.local.transform_point3(Vec3::ZERO).distance(origin) < 60.0);
    assert_eq!(
        parts.len(),
        14,
        "the authored dock galleon consists of fourteen brush parts"
    );
    for part in &mut parts {
        part.local = anchor.inverse() * part.local;
    }
    parts
}
