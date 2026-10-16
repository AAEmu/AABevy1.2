//! Compares the bind pose of every part of a character with the base skeleton's, bone by bone:
//! `cargo run -p aa-anim --example bind_check -- --race ferre --gender m [--all]`.
//!
//! Parts are skinned with the base skeleton's inverse bind matrices; a part exported against a
//! skeleton whose bones sit elsewhere ends up displaced (head sunk into the body or floating).

use aa_anim::{CharacterDesc, GameDb, Gender, Race};
use aa_anim::caf::Clip;
use aa_anim::pose::{Rig, SampleOptions};
use aa_anim::Cal;
use aa_cry::Skeleton;

fn arg(name: &str) -> Option<String> {
    let a: Vec<String> = std::env::args().collect();
    a.iter().position(|x| x == name).and_then(|i| a.get(i + 1).cloned())
}

fn main() {
    let race = match arg("--race").as_deref().unwrap_or("ferre") {
        "nuian" => Race::Nuian,
        "elf" => Race::Elf,
        "hariharan" => Race::Hariharan,
        _ => Race::Ferre,
    };
    let gender = if arg("--gender").as_deref() == Some("f") { Gender::Female } else { Gender::Male };
    let all = std::env::args().any(|a| a == "--all");
    let db = GameDb::open().expect("database");
    let desc = CharacterDesc::from_preset(&db, race, gender, 0).expect("preset");
    let assets = desc.resolve(&db).expect("resolve");
    let base = Skeleton::load(&assets.skeleton).expect("base skeleton");
    println!("base skeleton {} ({} bones)", assets.skeleton, base.len());
    for (name, i) in ["Head", "Neck", "Neck1", "Spine3", "Bip01 Head", "Bip01 Neck"].iter().filter_map(|n| base.find_ignore_case(n).map(|i| (n, i))) {
        println!("  base {name:12} at {:?}", base.bones[i].bind_model.translation);
    }
    if std::env::args().any(|a| a == "--pose") {
        let cal = Cal::load(&assets.cal).expect("cal");
        let entry = cal.animation("fist_ac_stage_idle").expect("idle entry");
        println!("cal base height {} / idle {} anim height {} -> position scale {}", cal.base_height, entry.path, entry.anim_height, entry.position_scale(cal.base_height));
        let rig = Rig::new(base.clone());
        let clip = Clip::load(&entry.path).expect("clip");
        let binding = rig.bind_clip(&clip);
        let mut pos_tracks = 0;
        for t in &clip.tracks {
            if t.sample_pos(0.0).is_some() {
                pos_tracks += 1;
            }
        }
        println!("{} tracks, {} with positions, {} bound", clip.tracks.len(), pos_tracks, binding.pairs.len());
        for scale in [entry.position_scale(cal.base_height), 1.0] {
            let mut pose = rig.bind.clone();
            pose.sample(&rig, &clip, &binding, 0.0, SampleOptions { position_scale: scale, lock_root_motion: true });
            let world = pose.model_space(&rig);
            println!("scale {scale}:");
            for name in ["Bip01 Pelvis", "Bip01 Spine", "Bip01 Spine1", "Bip01 Spine2", "Bip01 Neck", "Bip01 Head", "Bip01 L Clavicle", "Bip01 L Thigh"] {
                if let Some(i) = base.find_ignore_case(name) {
                    println!("  {name:18} bind {:?} pose {:?}", base.bones[i].bind_model.translation, world[i].1);
                }
            }
        }
    }
    for part in &assets.parts {
        let Ok(skel) = Skeleton::load(&part.path) else {
            println!("{:?} {} : no skeleton", part.kind, part.path);
            continue;
        };
        if let Ok(model) = aa_cry::Model::load(&part.path) {
            let (mut lo, mut hi) = (glam::Vec3::splat(f32::MAX), glam::Vec3::splat(f32::MIN));
            for mesh in &model.meshes {
                for p in &mesh.positions {
                    lo = lo.min(*p);
                    hi = hi.max(*p);
                }
            }
            println!("  {:?} mesh bbox z {:.3} .. {:.3}  y {:.3} .. {:.3}", part.kind, lo.z, hi.z, lo.y, hi.y);
        }
        if let Ok(Some(set)) = aa_anim::MorphSet::load(&part.path) {
            for t in set.targets.iter().filter(|t| t.name.to_ascii_lowercase().contains("morph") || std::env::args().any(|a| a == "--targets")) {
                let n = t.deltas.len();
                let mean = t.deltas.iter().map(|d| d.1).fold(glam::Vec3::ZERO, |a, b| a + b) / n.max(1) as f32;
                println!("    morph {:30} {} verts, mean delta {:?}", t.name, n, mean);
            }
        }
        let map = skel.map_to(&base);
        let mut worst = (0.0f32, String::new());
        let mut unmapped = 0;
        let mut shown = 0;
        for (i, b) in skel.bones.iter().enumerate() {
            let Some(j) = map[i] else {
                unmapped += 1;
                continue;
            };
            let d = (b.bind_model.translation - base.bones[j].bind_model.translation).length();
            if d > worst.0 {
                worst = (d, b.name.clone());
            }
            if all || (d > 0.01 && shown < 6) {
                println!("    {:20} part {:?} base {:?} d={d:.3}", b.name, b.bind_model.translation, base.bones[j].bind_model.translation);
                shown += 1;
            }
        }
        println!("{:?} {} : {} bones, {unmapped} unmapped, worst displacement {:.3} m ({})", part.kind, part.path, skel.len(), worst.0, worst.1);
    }
}
