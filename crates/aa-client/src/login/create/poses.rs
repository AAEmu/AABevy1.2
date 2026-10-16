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
        (Race::Ferre, Gender::Male) => (1020.0, 87.0, 2.06),
        (Race::Ferre, _) => (1033.0, 217.0, 1.97),
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

/// Step 3's feet pixel and head row, measured on the step 3 captures (provisional values for the
/// models not measured yet), and the turn towards the training dummies.
fn feet_pixels_3(race: Race, gender: Gender) -> (Vec2, f32, f32, f32) {
    // the combat stance of step 3 is lower than the step 1 stand: the rendered height is smaller
    let (_, _, height) = feet_pixels(race, gender);
    let height = height * 0.906;
    match (race, gender) {
        (_, _) => (Vec2::new(440.0, 910.0), 362.0, height, 52.0),
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
