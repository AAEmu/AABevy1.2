//! The cameras of the real client's create screens (`reference/character_create/step<N>_<race>_<gender>/camera.json`,
//! embedded) and where a character stands in front of them.

use aa_anim::{Gender, Race};
use bevy::prelude::*;

use crate::login::stage3d::{StagePose, StageSlot};

/// The text of the capture's `camera.json`.
fn camera_json(step: u8, race: Race, gender: Gender) -> &'static str {
    match (step, race, gender) {
        (1, Race::Nuian, Gender::Male) => include_str!("../../../../../reference/character_create/step1_nuian_male/camera.json"),
        (1, Race::Nuian, Gender::Female) => include_str!("../../../../../reference/character_create/step1_nuian_female/camera.json"),
        (1, Race::Elf, Gender::Male) => include_str!("../../../../../reference/character_create/step1_elf_male/camera.json"),
        (1, Race::Elf, Gender::Female) => include_str!("../../../../../reference/character_create/step1_elf_female/camera.json"),
        (1, Race::Hariharan, Gender::Male) => include_str!("../../../../../reference/character_create/step1_hariharan_male/camera.json"),
        (1, Race::Hariharan, Gender::Female) => include_str!("../../../../../reference/character_create/step1_hariharan_female/camera.json"),
        (1, Race::Ferre, Gender::Male) => include_str!("../../../../../reference/character_create/step1_ferre_male/camera.json"),
        (1, Race::Ferre, Gender::Female) => include_str!("../../../../../reference/character_create/step1_ferre_female/camera.json"),
        (2, Race::Nuian, Gender::Male) => include_str!("../../../../../reference/character_create/step2_nuian_male/camera.json"),
        (2, Race::Nuian, Gender::Female) => include_str!("../../../../../reference/character_create/step2_nuian_female/camera.json"),
        (2, Race::Elf, Gender::Male) => include_str!("../../../../../reference/character_create/step2_elf_male/camera.json"),
        (2, Race::Elf, Gender::Female) => include_str!("../../../../../reference/character_create/step2_elf_female/camera.json"),
        (2, Race::Hariharan, Gender::Male) => include_str!("../../../../../reference/character_create/step2_hariharan_male/camera.json"),
        (2, Race::Hariharan, Gender::Female) => include_str!("../../../../../reference/character_create/step2_hariharan_female/camera.json"),
        (2, Race::Ferre, Gender::Male) => include_str!("../../../../../reference/character_create/step2_ferre_male/camera.json"),
        (2, Race::Ferre, Gender::Female) => include_str!("../../../../../reference/character_create/step2_ferre_female/camera.json"),
        (3, Race::Nuian, Gender::Male) => include_str!("../../../../../reference/character_create/step3_nuian_male/camera.json"),
        (3, Race::Nuian, Gender::Female) => include_str!("../../../../../reference/character_create/step3_nuian_female/camera.json"),
        (3, Race::Elf, Gender::Male) => include_str!("../../../../../reference/character_create/step3_elf_male/camera.json"),
        (3, Race::Elf, Gender::Female) => include_str!("../../../../../reference/character_create/step3_elf_female/camera.json"),
        (3, Race::Hariharan, Gender::Male) => include_str!("../../../../../reference/character_create/step3_hariharan_male/camera.json"),
        (3, Race::Hariharan, Gender::Female) => include_str!("../../../../../reference/character_create/step3_hariharan_female/camera.json"),
        (3, Race::Ferre, Gender::Male) => include_str!("../../../../../reference/character_create/step3_ferre_male/camera.json"),
        (3, Race::Ferre, Gender::Female) => include_str!("../../../../../reference/character_create/step3_ferre_female/camera.json"),
        _ => include_str!("../../../../../reference/character_create/step1_nuian_male/camera.json"),
    }
}

/// The camera of a step (1 race, 2 appearance close-up, 3 skillset and name) for a model.
pub fn pose(step: u8, race: Race, gender: Gender) -> StagePose {
    StagePose::from_json(camera_json(step.clamp(1, 3), race, gender)).expect("embedded camera.json")
}

/// Measured on the step 1 captures: the pixel row of the soles and of the head top of every figure,
/// and the figure's height above its soles (from the renders: the nominal actor heights are not
/// the visual ones). Both genders of a race stand at about the same spot but are scaled a little
/// differently in the client, so each model is solved on its own.
fn feet_pixels(race: Race, gender: Gender) -> (f32, f32, f32) {
    match (race, gender) {
        (Race::Nuian, Gender::Male) => (1020.0, 97.0, 2.03),
        (Race::Nuian, _) => (1009.0, 157.0, 1.81),
        (Race::Elf, Gender::Male) => (1007.0, 83.0, 2.02),
        (Race::Elf, _) => (1007.0, 147.0, 1.89),
        (Race::Hariharan, Gender::Male) => (947.0, 164.0, 1.80),
        (Race::Hariharan, _) => (960.0, 217.0, 1.68),
        (Race::Ferre, Gender::Male) => (1020.0, 87.0, 2.07),
        (Race::Ferre, _) => (1033.0, 217.0, 1.833),
    }
}

/// Where the figure stands on steps 1 and 2: solved from its feet and head in the step 1 capture,
/// facing the step 1 camera.
pub fn slot_steps_1_2(race: Race, gender: Gender) -> StageSlot {
    let cam = pose(1, race, gender);
    let (feet_v, head_v, height) = feet_pixels(race, gender);
    let feet = cam.solve_feet(Vec2::new(960.0, feet_v), head_v, height);
    StageSlot::facing(feet, cam.pos, race, gender)
}

/// How far (scale about the screen centre, then pixels right / down) the step 1 position has to be
/// corrected so the step 2 close-up matches the captures: the real client frames the head
/// differently for every model. Measured by registering our render onto the capture
/// (`data/extract/create/align3.py`).
fn step2_correction(race: Race, gender: Gender) -> (f32, f32, f32) {
    match (race, gender) {
        (Race::Nuian, Gender::Male) => (1.0, -20.0, -40.0),
        (Race::Nuian, _) => (1.0, -28.0, -20.0),
        (Race::Elf, Gender::Male) => (1.0, -24.0, -16.0),
        (Race::Elf, _) => (1.0, -16.0, 8.0),
        (Race::Hariharan, Gender::Male) => (1.188, 0.0, 28.0),
        (Race::Hariharan, _) => (1.125, 0.0, 4.0),
        (Race::Ferre, Gender::Male) => (1.0, 4.0, -32.0),
        (Race::Ferre, _) => (1.038, 0.0, 0.0),
    }
}

/// Where the figure stands on step 2 (the close-up): the step 1 spot moved so the head sits where
/// the capture has it.
pub fn slot_step_2(race: Race, gender: Gender) -> StageSlot {
    let mut slot = slot_steps_1_2(race, gender);
    let (scale, dx, dy) = step2_correction(race, gender);
    let cam = pose(2, race, gender);
    let (forward, right, up) = cam.axes();
    let (_, _, height) = feet_pixels(race, gender);
    let head = slot.feet + Vec3::Z * height * 0.92;
    let depth = (head - cam.pos).dot(forward);
    let focal = 540.0 / (cam.fov_deg.to_radians() / 2.0).tan();
    let new_depth = depth / scale;
    let t = -forward * (depth - new_depth) + right * (dx * new_depth / focal) - up * (dy * new_depth / focal);
    slot.feet += t;
    slot
}

/// Step 3: the pixel of the figure's origin, the row of the head top, the height of the stance in
/// metres and the turn towards the training dummies. Measured by registering the silhouette of our
/// render onto the captures (`data/extract/create/bbox.py`); the combat stance is lower than the
/// step 1 stand and differs per model.
fn feet_pixels_3(race: Race, gender: Gender) -> (Vec2, f32, f32, f32) {
    let yaw = 70.0;
    match (race, gender) {
        (Race::Nuian, Gender::Male) => (Vec2::new(407.0, 906.0), 376.0, 1.880, yaw),
        (Race::Nuian, _) => (Vec2::new(464.0, 891.0), 385.0, 1.793, yaw),
        (Race::Elf, Gender::Male) => (Vec2::new(452.0, 851.0), 385.0, 1.884, yaw),
        (Race::Elf, _) => (Vec2::new(450.0, 843.0), 417.0, 1.793, yaw),
        (Race::Hariharan, Gender::Male) => (Vec2::new(488.0, 915.0), 527.0, 1.305, yaw),
        (Race::Hariharan, _) => (Vec2::new(435.0, 935.0), 555.0, 1.230, yaw),
        (Race::Ferre, Gender::Male) => (Vec2::new(462.0, 871.0), 471.0, 1.576, yaw),
        (Race::Ferre, _) => (Vec2::new(466.0, 890.0), 514.0, 1.50, yaw),
    }
}

/// Where the figure stands on step 3 (skillset and name): in front of the training dummies, turned
/// towards them.
pub fn slot_step_3(race: Race, gender: Gender) -> StageSlot {
    let cam = pose(3, race, gender);
    let (feet_px, head_v, height, yaw) = feet_pixels_3(race, gender);
    let feet = cam.solve_feet(feet_px, head_v, height);
    let mut slot = StageSlot::facing(feet, cam.pos, race, gender);
    slot.yaw_deg = std::env::var("AA_YAW3").ok().and_then(|v| v.parse().ok()).unwrap_or(yaw);
    slot
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "lists animations, needs the game data"]
    fn list_animations() {
        let cal = aa_anim::cal::Cal::load("objects/characters/nuian/male/nu_m_base.cal").or_else(|_| aa_anim::cal::Cal::load("objects/characters/nuian/male/nude/nu_m_base.cal")).unwrap();
        let mut names: Vec<&String> = cal.animations.keys().filter(|n| n.contains("stage") || n.contains("idle") || n.contains("select") || n.contains("create")).collect();
        names.sort();
        for n in names { println!("anim {n} -> {}", cal.animations[n].path); }
    }

    #[test]
    fn step3_slot_probe() {
        let cam = pose(3, Race::Nuian, Gender::Male);
        let slot = slot_step_3(Race::Nuian, Gender::Male);
        println!("cam {:?}
feet {:?} dist {:.2} face {:?} px {:?}", cam, slot.feet, (slot.feet - cam.pos).length(), slot.face, cam.project(slot.feet));
        println!("head px {:?}", cam.project(slot.feet + Vec3::Z * 2.03));
    }

    #[test]
    fn figures_stand_on_their_feet_pixel() {
        for race in [Race::Nuian, Race::Elf, Race::Hariharan, Race::Ferre] {
            for gender in [Gender::Male, Gender::Female] {
                let cam = pose(1, race, gender);
                let slot = slot_steps_1_2(race, gender);
                let (feet_v, _, _) = feet_pixels(race, gender);
                let px = cam.project(slot.feet);
                assert!((px - Vec2::new(960.0, feet_v)).length() < 1.0, "{race:?} {gender:?}: {px:?}");
                let t = slot.transform();
                assert!(t.translation.is_finite(), "{race:?} {gender:?}: {t:?}");
                println!("{race:?} {gender:?}: feet {:?} stage {:?}", slot.feet, t.translation);
            }
        }
    }
}

#[cfg(test)]
mod scene_probe {
    use super::*;

    #[test]
    #[ignore = "scene probe, needs the game data"]
    fn login2_heights() {
        for (cx, cy) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
            let dir = aa_world::world::cell_dir("login2", cx, cy);
            let Ok(bytes) = aa_pak::Pak::global().read(&format!("{dir}/client/terrain/heightmap.dat")) else { println!("cell {cx},{cy}: no heightmap"); continue };
            let h = aa_world::heightmap::CellHeights::parse(&bytes).unwrap();
            println!("cell {cx},{cy}: z {:.1}..{:.1}", h.min_z, h.max_z);
            for race in [Race::Nuian, Race::Elf, Race::Hariharan, Race::Ferre] {
                let slot = slot_step_3(race, Gender::Male);
                let (lx, ly) = (slot.feet.x - 1024.0 * cx as f32, slot.feet.y - 1024.0 * cy as f32);
                if (0.0..1024.0).contains(&lx) && (0.0..1024.0).contains(&ly) {
                    println!("  {race:?} feet {:?} terrain {:.2}", slot.feet, h.sample(lx, ly));
                }
            }
            let c = (727.19, 620.7);
            if cx == 0 && cy == 0 { println!("  step1 nuian spot terrain {:.2}", h.sample(c.0, c.1)); }
        }
    }

    #[test]
    #[ignore = "scene probe, needs the game data"]
    fn login2_vegetation_groups() {
        let o = aa_world::placement::load_cell_objects("login2", 0, 1);
        let groups = aa_world::vegetation::VegGroups::load("login2").unwrap();
        for race in [Race::Nuian, Race::Elf, Race::Hariharan, Race::Ferre] {
            let cam = pose(3, race, Gender::Male);
            let mut count = std::collections::BTreeMap::<u16, (usize, f32)>::new();
            for v in &o.vegetation {
                let d = (v.pos - cam.pos).length();
                if d < 150.0 {
                    let e = count.entry(v.group).or_insert((0, 0.0));
                    e.0 += 1;
                    e.1 += v.scale;
                }
            }
            println!("== {race:?}");
            for (g, (n, sc)) in count {
                let gr = groups.groups.get(&g);
                println!("  group {g}: {n} x scale {:.2} {} sprites {} view {} -> {}", sc / n as f32, gr.map(|g| g.name.as_str()).unwrap_or("?"), gr.is_some_and(|g| g.use_sprites), gr.map_or(0.0, |g| g.max_view_dist_ratio), gr.map(|g| g.model.as_str()).unwrap_or("?"));
            }
        }
    }

    #[test]
    #[ignore = "scene probe, needs the game data"]
    fn dummy_nodes() {
        for f in aa_pak::Pak::global().list("game/objects/env/01_nuia/001_ndeco/old/learning") {
            println!("file {f}");
        }
        println!("{}", aa_pak::Pak::global().read_string("game/objects/env/01_nuia/001_ndeco/old/learning_ob01_c.cal").unwrap_or_default());
        let chr = aa_cry::Model::load("game/objects/env/01_nuia/001_ndeco/old/learning_ob01_c.chr");
        println!("chr: {:?}", chr.as_ref().map(|m| (m.nodes.len(), m.meshes.len())).map_err(|e| e.to_string()));
        {
            let m = aa_cry::Model::load("game/Objects/Env/01_nuia/001_ndeco/old/learning_ob01_c.cgf").unwrap();
            let mesh = &m.meshes[0];
            let mut hist = std::collections::BTreeMap::<(i32, i32), usize>::new();
            for p in &mesh.positions {
                *hist.entry(((p.x.abs() * 10.0) as i32, (p.z * 5.0).floor() as i32)).or_default() += 1;
            }
            for ((x, z), n) in hist { if x >= 2 { println!("hist |x| {:.1} z {:.1}: {n}", x as f32 / 10.0, z as f32 / 5.0); } }
        }
        for name in ["c"] {
            for ext in ["cgf", "chr"] {
                if let Ok(m) = aa_cry::Model::load(&format!("game/objects/env/01_nuia/001_ndeco/old/learning_ob01_{name}.{ext}")) {
                    for mesh in &m.meshes {
                        let (mut lo, mut hi) = (bevy::math::Vec3::splat(1e9), bevy::math::Vec3::splat(-1e9));
                        for p in &mesh.positions { lo = lo.min(*p); hi = hi.max(*p); }
                        println!("variant {name}.{ext}: verts {} tris {} bounds {lo:?} {hi:?} skin {}", mesh.positions.len(), mesh.indices.len() / 3, mesh.skin.is_some());
                    }
                }
            }
        }
        let m = aa_cry::Model::load("game/Objects/Env/01_nuia/001_ndeco/old/learning_ob01_c.cgf").unwrap();
        for (i, n) in m.nodes.iter().enumerate() {
            println!("node {i} {:?} kind {:?} mesh {:?} mtl {:?} parent {:?} props {:?} pos {:?}", n.name, n.kind, n.mesh, n.material_name, n.parent, n.properties, n.local.translation);
        }
        for (i, mesh) in m.meshes.iter().enumerate() {
            println!("mesh {i}: {} subsets", mesh.subsets.len());
            for s in &mesh.subsets {
                println!("  subset mat {} first {} count {} center {:?} radius {}", s.material_id, s.first_index, s.index_count, s.center, s.radius);
            }
            println!("  verts {} colors {} indices {}", mesh.positions.len(), mesh.colors.len(), mesh.indices.len());
            let (mut lo, mut hi) = (bevy::math::Vec3::splat(1e9), bevy::math::Vec3::splat(-1e9));
            for p in &mesh.positions { lo = lo.min(*p); hi = hi.max(*p); }
            println!("  bounds {lo:?} {hi:?}");
        }
        let mtl = aa_cry::material::Material::load("game/Objects/Env/01_nuia/001_ndeco/old/learning_ob01.mtl").or_else(|_| aa_cry::material::Material::load("Objects/Env/01_nuia/001_ndeco/old/learning_ob01.mtl")).unwrap();
        println!("mtl {} shader {} subs {}", mtl.name, mtl.shader, mtl.sub_materials.len());
        for (i, sub) in mtl.sub_materials.iter().enumerate().take(2) {
            println!("  sub {i} gen {:?} vert_modif {} public {:?} surface {:?} glow {} spec {:?} diffuse {:?}", sub.string_gen_mask, sub.vert_modif_type, sub.public_params, sub.surface_type, sub.glow_amount, sub.specular, sub.diffuse);
        }
        for (i, sub) in mtl.sub_materials.iter().enumerate() {
            println!("  sub {i} {} shader {} opacity {} alpha_test {} flags {:#x} nodraw {} textures {:?}", sub.name, sub.shader, sub.opacity, sub.alpha_test, sub.flags, sub.no_draw(), sub.textures.iter().map(|t| t.file.clone()).collect::<Vec<_>>());
        }
    }

    #[test]
    #[ignore = "scene probe, needs the game data"]
    fn login2_find_field() {
        let mut cells = Vec::new();
        for (cx, cy) in [(0, 0), (0, 1), (1, 0)] {
            let dir = aa_world::world::cell_dir("login2", cx, cy);
            let bytes = aa_pak::Pak::global().read(&format!("{dir}/client/terrain/heightmap.dat")).unwrap();
            cells.push(((cx, cy), aa_world::heightmap::CellHeights::parse(&bytes).unwrap()));
        }
        for race in [Race::Nuian, Race::Elf, Race::Hariharan, Race::Ferre] {
            let slot = slot_step_3(race, Gender::Male);
            let (fx, fy, fz) = (slot.feet.x, slot.feet.y, slot.feet.z);
            for ((cx, cy), h) in &cells {
                // all local (x, y) within 8 m of the capture's local position whose height is within 0.6 m of the feet
                for ox in [-1024.0f32, 0.0, 1024.0] {
                    for oy in [-1024.0f32, 0.0, 1024.0] {
                        let (lx, ly) = (fx + ox, fy + oy);
                        if !(0.0..1024.0).contains(&lx) || !(0.0..1024.0).contains(&ly) { continue; }
                        let z = h.sample(lx, ly);
                        println!("{race:?} cell {cx},{cy} local ({lx:.0},{ly:.0}) terrain {z:.1} feet z {fz:.1} delta {:.1}", z - fz);
                    }
                }
            }
        }
    }

    #[test]
    #[ignore = "scene probe, needs the game data"]
    fn login2_objects_near_step3() {
        for (cx, cy) in [(0, 1), (1, 0)] {
            let o = aa_world::placement::load_cell_objects("login2", cx, cy);
            println!("cell {cx},{cy}: brushes {} vegetation {} decals {} water {} models {}", o.brushes.len(), o.vegetation.len(), o.decals.len(), o.water.len(), o.models.len());
            for race in [Race::Nuian, Race::Elf, Race::Hariharan, Race::Ferre] {
                let cam = pose(3, race, Gender::Male);
                let mut models = std::collections::BTreeMap::<String, usize>::new();
                let mut near = 0;
                for b in &o.brushes {
                    let t = b.translation();
                    let d = (t - cam.pos).length();
                    if d < 120.0 {
                        near += 1;
                        *models.entry(o.models.get(b.model as usize).cloned().unwrap_or_default()).or_default() += 1;
                        if d < 70.0 {
                            println!("  {race:?} brush {:6.1} m at {:?} {}", d, t, o.models.get(b.model as usize).map(|s| s.as_str()).unwrap_or("?"));
                        }
                    }
                }
                let veg = o.vegetation.iter().filter(|v| (v.pos - cam.pos).length() < 120.0).count();
                println!("  {race:?}: {near} brushes, {veg} vegetation within 120 m; models {models:?}");
            }
        }
    }
}
