//! The 1.2 facial bone path: accumulate state weights, compose unnormalised weighted QuatTs
//! in effector order, normalise once, convert model-space translation, then default * offset.
use std::sync::Arc;

use bevy::prelude::*;

use super::{EffectorKind, ExpressionLibrary};
use crate::{Animator, Rig};

#[cfg_attr(test, derive(Debug))]
struct Binding {
    joint: usize,
    rotation: Quat,
    position: Vec3,
}

#[cfg_attr(test, derive(Debug))]
struct Joint {
    bone: usize,
    inverse_parent: Quat,
    rotation: Quat,
    position: Vec3,
    active: bool,
}

impl Joint {
    fn local(&self, rig: &Rig) -> (Quat, Vec3) {
        (rig.bind.rot[self.bone] * self.rotation, rig.bind.pos[self.bone] + rig.bind.rot[self.bone] * (self.inverse_parent * self.position))
    }
}

#[cfg_attr(test, derive(Debug))]
pub(super) struct BoneChannels {
    pub library: Arc<ExpressionLibrary>,
    pub rig_id: u64,
    pub names: Vec<String>,
    bindings: Vec<Binding>,
    joints: Vec<Joint>,
    last_body: Vec<(Entity, Quat, Vec3)>,
}

impl BoneChannels {
    pub fn bind(library: Arc<ExpressionLibrary>, rig: &Rig) -> Self {
        let mut result = Self { library, rig_id: rig.id, names: Vec::new(), bindings: Vec::new(), joints: Vec::new(), last_body: Vec::new() };
        for name in &result.library.bone_order {
            let Some(effector) = result.library.effectors.get(name) else { continue };
            let EffectorKind::Bone { attachment, position, rotation } = &effector.kind else { continue };
            let Some(bone) = rig.skeleton.bones.iter().position(|b| b.name.eq_ignore_ascii_case(attachment)) else {
                // Attachment names address real joints; missing eyes/jaws must not turn into
                // rotations of a nearby bone or morph displacements.
                continue;
            };
            let joint = result.joints.iter().position(|j| j.bone == bone).unwrap_or_else(|| {
                let inverse_parent = rig.skeleton.bones[bone]
                    .parent
                    .map(|p| rig.skeleton.bones[p].model_rotation_translation().0.conjugate())
                    .unwrap_or(Quat::IDENTITY);
                result.joints.push(Joint { bone, inverse_parent, rotation: Quat::IDENTITY, position: Vec3::ZERO, active: false });
                result.joints.len() - 1
            });
            // CryEngine SetRotationXYZ is Rz * Ry * Rx, with angles in radians.
            let rotation = Quat::from_rotation_z(rotation.z) * Quat::from_rotation_y(rotation.y) * Quat::from_rotation_x(rotation.x);
            result.names.push(name.clone());
            result.bindings.push(Binding { joint, rotation, position: *position });
        }
        result
    }

    pub fn restore(&self, body: &Animator, transforms: &mut Query<&mut Transform>) {
        // Animator intervals can skip writes, and a stopped body has no current clip. Always
        // start from its stored pose rather than the previous frame's facial Transform.
        for joint in &self.joints {
            if let Some(&entity) = body.bones.get(joint.bone) {
                if let Ok(mut transform) = transforms.get_mut(entity) {
                    let (rotation, position) = (body.pose().rot[joint.bone], body.pose().pos[joint.bone]);
                    if transform.rotation != rotation || transform.translation != position {
                        transform.rotation = rotation;
                        transform.translation = position;
                    }
                }
            }
        }
    }

    pub fn apply(&mut self, body: &Animator, weights: &[f32], transforms: &mut Query<&mut Transform>) {
        self.evaluate(weights);
        self.last_body.clear();
        for joint in &self.joints {
            if !joint.active {
                continue;
            }
            if let Some(&entity) = body.bones.get(joint.bone) {
                if let Ok(mut transform) = transforms.get_mut(entity) {
                    self.last_body.push((entity, body.pose().rot[joint.bone], body.pose().pos[joint.bone]));
                    // 31599DC0 restores active joints from the default local pose, then
                    // 31599E80 right-multiplies it by the facial displacement.
                    (transform.rotation, transform.translation) = joint.local(&body.rig);
                }
            }
        }
    }

    /// Restore the last underlying body pose even if the client has parked its Animator.
    pub fn detach(&self, world: &mut World) {
        for &(entity, rotation, translation) in &self.last_body {
            if let Some(mut transform) = world.get_mut::<Transform>(entity) {
                transform.rotation = rotation;
                transform.translation = translation;
            }
        }
    }

    fn evaluate(&mut self, weights: &[f32]) {
        for joint in &mut self.joints {
            joint.rotation = Quat::IDENTITY;
            joint.position = Vec3::ZERO;
            joint.active = false;
        }
        for (binding, &weight) in self.bindings.iter().zip(weights) {
            if !weight.is_finite() || weight.abs() < 0.0001 {
                continue;
            }
            let joint = &mut self.joints[binding.joint];
            let q = binding.rotation;
            // Native 31599C60 does not normalise each weighted quaternion. Negative weights
            // conjugate the source, and values outside [-1, 1] extrapolate rather than clamp.
            let weighted = Quat::from_xyzw(q.x * weight, q.y * weight, q.z * weight, 1.0 + (q.w - 1.0) * weight.abs());
            joint.position += rotate_unnormalised(joint.rotation, binding.position * weight);
            joint.rotation *= weighted;
            joint.active = true;
        }
        for joint in &mut self.joints {
            if joint.active {
                if joint.rotation.is_finite() && joint.rotation.length_squared() > 1e-12 && joint.position.is_finite() {
                    joint.rotation = joint.rotation.normalize();
                } else {
                    joint.active = false;
                }
            }
        }
    }
}

// CryEngine's Quat * Vec3 formula is used before final normalisation. Keep that arithmetic
// instead of calling a helper which assumes a unit quaternion or normalises it implicitly.
fn rotate_unnormalised(q: Quat, v: Vec3) -> Vec3 {
    let xyz = Vec3::new(q.x, q.y, q.z);
    v + 2.0 * xyz.cross(xyz.cross(v) + q.w * v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Affine3A;

    fn rig() -> Rig {
        let bone = |name: &str, parent, local, model| aa_cry::Bone {
            name: name.into(),
            controller_id: 0,
            parent,
            limb_id: 0,
            bind_local: local,
            bind_model: model,
            physics: Default::default(),
        };
        let root = Affine3A::from_rotation_translation(Quat::from_rotation_z(0.6), Vec3::ZERO);
        let head = Affine3A::from_rotation_translation(Quat::from_rotation_z(0.3), Vec3::new(0.2, -0.1, 1.3));
        Rig::new(aa_cry::Skeleton { bones: vec![bone("root", None, root, root), bone("head", Some(0), head, root * head)] })
    }

    const XML: &str = r#"<ExpressionLibrary><AllEffectors>
      <Effector Name="z_first" Type="bone" Attachment="HEAD" PosOffset="0.02,-0.01,0.03" RotOffset="0.4,-0.2,0.3"/>
      <Effector Name="a_second" Type="bone" Attachment="head" PosOffset="-0.03,0.04,0.01" RotOffset="-0.1,0.5,0.2"/>
      <Effector Name="missing" Type="bone" Attachment="absent" RotOffset="1,0,0"/>
      <Effector Name="expression" Type="expression"/>
    </AllEffectors><Controllers><Effector Name="expression"><SubCtrl Effector="z_first" Weight="0.5"/></Effector></Controllers>
    </ExpressionLibrary>"#;

    // Goldens execute original DLL bytes with Unicorn, not a second Rust formula. SHA256
    // c630e58a62e36b33fd4dedad8917aad000474bf35d7054d9a2bbe00fb057ca6f.
    // Helpers 31599C60 (signed weights), 31599DC0/31599E80 (default * displacement),
    // 3159A3C0 (inverse default-parent translation). See docs/aa-anim.md.
    #[test]
    fn native_ordered_weights_compose_before_normalising() {
        let library = Arc::new(ExpressionLibrary::parse(XML).unwrap());
        let mut bones = BoneChannels::bind(library.clone(), &rig());
        assert_eq!(bones.names, ["z_first", "a_second"]);
        // Direct and expression-driven contributions sum at the same state index.
        let sequence = super::super::FacialSequence::parse(
            r#"<FacialSequence EndTime="1"><Channel Name="z_first"><Spline Keys="0:0.2:0,"/></Channel>
            <Channel Name="expression"><Spline Keys="0:0.4:0,"/></Channel>
            <Channel Name="a_second"><Spline Keys="0:-0.5:0,"/></Channel></FacialSequence>"#,
        )
        .unwrap();
        let mut weights = [0.0; 2];
        library.sample_weights(&sequence, 0.0, &bones.names, &mut weights);
        assert_eq!(weights, [0.4, -0.5]);
        bones.evaluate(&weights);
        let joint = &bones.joints[0];
        let native = Quat::from_xyzw(0.12771487, -0.13802622, 0.0021069124, 0.96291715).normalize();
        assert!(joint.rotation.abs_diff_eq(native, 2e-7), "{:?}", joint.rotation);
        assert!(joint.position.abs_diff_eq(Vec3::new(0.025750961, -0.020811973, 0.004800463), 2e-7), "{:?}", joint.position);
    }

    #[test]
    fn native_signed_extrapolation_and_default_parent_space() {
        let rig = rig();
        let mut bones = BoneChannels::bind(Arc::new(ExpressionLibrary::parse(XML).unwrap()), &rig);
        for (weight, expected) in [
            (-1.4, [-0.2941101, 0.09408597, -0.23147425, 0.9457588]),
            (-0.5, [-0.10503932, 0.033602133, -0.08266938, 0.98062813]),
            (1.3, [0.27310222, -0.087365545, 0.21494037, 0.9496331]),
        ] {
            bones.evaluate(&[weight, 0.0]);
            assert!(bones.joints[0].rotation.abs_diff_eq(Quat::from_array(expected).normalize(), 2e-7));
        }
        bones.evaluate(&[1.0, 0.0]);
        let (rotation, position) = bones.joints[0].local(&rig);
        assert!(rotation.abs_diff_eq(Quat::from_xyzw(0.21776256, -0.035055876, 0.30713052, 0.9257545), 2e-7));
        assert!(position.abs_diff_eq(Vec3::new(0.21615154, -0.11546377, 1.3299999), 2e-7), "{position:?}");
        bones.evaluate(&[0.0, f32::NAN]);
        assert!(!bones.joints[0].active);
    }

    #[test]
    fn original_ferre_eye_channels_bind_only_exact_joints() {
        if !aa_cry::pak_available() {
            return;
        }
        let rig = crate::pose::shared_rig("objects/characters/ferre/male/nude/fe_m_base.chr").unwrap();
        let library = ExpressionLibrary::load("animations/ferre/male/facial/ferre_m.fxl").unwrap();
        let mut bones = BoneChannels::bind(library.clone(), &rig);
        for name in ["horizontal_l", "horizontal_r", "vertical_l", "vertical_r"] {
            assert!(bones.names.iter().any(|s| s == name), "{name}");
        }
        let sequence = super::super::FacialSequence::load("animations/ferre/male/facial/fist_em_cry.fsq").unwrap();
        let mut weights = vec![0.0; bones.names.len()];
        library.sample_weights(&sequence, 0.239096, &bones.names, &mut weights);
        let left = bones.names.iter().position(|s| s == "vertical_l").unwrap();
        assert!((weights[left] + 0.590164).abs() < 1e-6);
        bones.evaluate(&weights);
        assert_eq!(bones.joints.iter().filter(|j| j.active).count(), 2);
        for file in ["fe_m_face00.chr", "fe_m_face01.chr"] {
            let model = aa_cry::Model::load(&format!("objects/characters/ferre/male/face/{file}")).unwrap();
            let skeleton = model.skeleton.as_ref().unwrap();
            for name in ["eye_left_bone", "eye_right_bone"] {
                let bone = skeleton.bones.iter().position(|b| b.name == name).unwrap();
                let skin = model.meshes[0].skin.as_ref().unwrap();
                let count = skin.joints.iter().zip(&skin.weights).filter(|(j,w)| j.iter().zip(w.iter()).any(|(&j,&w)| j as usize == bone && w>0.0)).count();
                // These old FXL eye channels exist, but neither current root head assigns
                // vertices to those joints. Preserve the original weights; do not fake a gaze.
                assert_eq!(count, 0, "{file} {name}");
            }
        }
    }
}
