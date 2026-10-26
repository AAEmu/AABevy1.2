//! The blur behind the quest talk's close-up, as 1.2's renderer draws it ([`aa_world::depth_of_field`] holds the rules and what was
//! read): while a close-up frames a row that has `dof` and the camera's depth of field is on ([`aa_world::post::Post::dof`]), the focus
//! is the row's (distance `camera_offset_y`, range 2.5 times it, blur 1). The renderer's eased copy of the focus has long settled when a
//! talk starts, so the blur is there from the talk's first frame and only glides from the last talk's distance to this one's; it is off
//! at once when the close-up ends. [`drive`] feeds the rules the frame time and writes the result into the world camera's
//! [`PostUniform::dof`], which `aa-world`'s post passes draw over the time of day's rest blur (`dof_rest`) while it is above 0.
//!
//! It reads [`QuestTalkCamera`] and [`QuestShot`] (`quest_camera.rs`) and changes neither.

use aa_world::depth_of_field::{self as rule, Eased};
use aa_world::post::{Post, PostUniform};
use bevy::prelude::*;

use super::camera::OrbitCamera;
use super::quest_camera::{QuestShot, QuestTalkCamera};

/// The 1080 p height the blur is stated for, when the camera has no size yet.
const FALLBACK_HEIGHT: f32 = 1080.0;

/// Eases the focus and puts it on the world camera.
pub fn drive(
    talk: Res<QuestTalkCamera>,
    shot: Res<QuestShot>,
    time: Res<Time>,
    mut eased: Local<Eased>,
    mut cameras: Query<(&mut PostUniform, Option<&Post>, &Camera), With<OrbitCamera>>,
) {
    let framed = talk.frame.filter(|_| talk.on && shot.pose.is_some());
    let on = cameras.iter().next().is_none_or(|(_, post, _)| post.is_none_or(|p| p.dof));
    let ask = framed.and_then(|f| rule::asked(true, f.row.dof, on, f.row.camera[1]));
    // the renderer eases its copy every frame, talk or none (`focus_params`)
    let focus = eased.frame(ask, time.delta_secs());
    for (mut u, _, camera) in &mut cameras {
        let height = camera.physical_target_size().map_or(FALLBACK_HEIGHT, |s| s.y as f32);
        let wanted = Vec4::from_array(rule::shader_params(focus, rule::talk_sigma_texels(height)));
        if u.dof != wanted {
            u.dof = wanted;
        }
    }
}
