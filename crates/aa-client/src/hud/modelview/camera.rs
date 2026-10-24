//! Original modelview camera inputs. No capture-fitted character-screen poses.
//!
//! Native import: 30149350 -> 3658e5a0/3658e7c0 -> 3014bf90/3014bff0.
//! Shop midpoint/offset/frustum: 392bdd90, 392bb5d0, 392bd640.
//! See the retained modelview-object-service and camera-prerequisite audits.

use aa_anim::{Gender, Race};
use anyhow::{Context, Result, ensure};
use bevy::prelude::{IVec2, PerspectiveProjection, Quat, Transform, UVec2, Vec3, default};

const FOV: f32 = f32::from_bits(0x3f060a92);

#[derive(Clone, Copy, Debug)]
pub(super) struct Pose {
    pub rotation: Quat,
    pub position: Vec3,
}

#[derive(Clone, Debug)]
pub(super) struct Scene {
    pub actor: Pose,
    cameras: [Pose; 4],
}

/// The actual applied state is a caller input, distinct from the latest reply.
/// An enabled correction needs both resolved values; zero is a valid value.
#[derive(Clone, Copy, Debug)]
pub(super) enum Height {
    Unknown,
    Disabled,
    Enabled { current: f32, reference: f32 },
}

impl Scene {
    pub fn load(race: Race, gender: Gender) -> Result<Self> {
        let path = format!("game/create_{}.lyr", race.folder());
        Self::parse(&aa_pak::Pak::global().read_string(&path)?, race, gender)
            .with_context(|| format!("modelview camera {path} {gender:?}"))
    }

    fn parse(text: &str, race: Race, gender: Gender) -> Result<Self> {
        let doc = roxmltree::Document::parse(text)?;
        let layer = doc
            .root_element()
            .children()
            .find(|n| n.has_tag_name("Layer"))
            .context("missing modelview Layer")?;
        let objects = layer
            .children()
            .find(|n| n.has_tag_name("LayerObjects"))
            .context("missing modelview LayerObjects")?;
        let lookup = |name: &str, kind: &str| -> Result<Pose> {
            let mut matches = objects.children().filter(|n| {
                n.has_tag_name("Object")
                    && n.attribute("Name") == Some(name)
                    && n.attribute("Type") == Some(kind)
            });
            let object = matches.next().with_context(|| format!("missing {name}"))?;
            ensure!(
                matches.next().is_none(),
                "ambiguous modelview object {name}"
            );
            let values = |key: &str, count: usize| -> Result<Vec<f32>> {
                let values = object
                    .attribute(key)
                    .with_context(|| format!("{name} missing {key}"))?
                    .split(',')
                    .map(|v| v.trim().parse::<f32>())
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                ensure!(
                    values.len() == count && values.iter().all(|v| v.is_finite()),
                    "invalid {name} {key}"
                );
                Ok(values)
            };
            let p = values("Pos", 3)?;
            let q = values("Rotate", 4)?;
            // Text-node Quat getter stores second/third/fourth/first, unchanged.
            let rotation = Quat::from_xyzw(q[1], q[2], q[3], q[0]);
            ensure!(
                rotation.length_squared().is_finite() && rotation.length_squared() > 0.,
                "invalid modelview rotation {name}"
            );
            Ok(Pose {
                rotation,
                position: Vec3::new(p[0], p[1], p[2]),
            })
        };
        let race = race.folder();
        let gender = match gender {
            Gender::Male => "male",
            Gender::Female => "female",
        };
        let actor = lookup(&format!("create_character_{race}"), "Entity")?;
        let cameras = (1..=4)
            .map(|i| lookup(&format!("04_customize_{race}_{gender}_{i}"), "Camera"))
            .collect::<Result<Vec<_>>>()?
            .try_into()
            .expect("four requested cameras");
        Ok(Self { actor, cameras })
    }

    /// SetIngameShopMode(true): replace the stored pose with entries 3/4 midpoint.
    pub fn shop_pose(&self) -> Pose {
        let a = self.cameras[2];
        let b = self.cameras[3];
        let sign = if a.rotation.dot(b.rotation) < 0. {
            -1.
        } else {
            1.
        };
        // Portable normalization replaces native rsqrt/refinement; fixture bounds
        // compare resulting axes, rather than claiming identical arithmetic bits.
        let q = b.rotation.to_array();
        let a = a.rotation.to_array().map(|v| v * sign);
        let rotation =
            Quat::from_array(std::array::from_fn(|i| q[i] + (a[i] - q[i]) * 0.5)).normalize();
        Pose {
            rotation,
            position: b.position + (self.cameras[2].position - b.position) * 0.5,
        }
    }

    /// Rebase translation about the authored actor; retain its rotation and yaw.
    /// aa-anim's child model root already supplies the raw-mesh basis conversion.
    pub fn actor_transform(&self, yaw_radians: f32) -> Option<Transform> {
        if !yaw_radians.is_finite() {
            return None;
        }
        let basis = aa_cry::CRY_TO_BEVY;
        Some(Transform::from_rotation(
            basis * (self.actor.rotation * Quat::from_rotation_z(yaw_radians)) * basis.inverse(),
        ))
    }

    pub fn camera(
        &self,
        pose: Pose,
        extent: UVec2,
        height: Height,
    ) -> Option<(Transform, PerspectiveProjection)> {
        if extent.min_element() == 0 || !pose.position.is_finite() || !pose.rotation.is_finite() {
            return None;
        }
        let position = pose.with_height(height)?.position;
        let basis = aa_cry::CRY_TO_BEVY;
        let transform = Transform::from_translation(basis * (position - self.actor.position))
            .with_rotation(basis * pose.rotation * basis.inverse());
        Some((
            transform,
            PerspectiveProjection {
                fov: FOV,
                aspect_ratio: extent.x as f32 / extent.y as f32,
                near: 0.1,
                far: 100.,
                ..default()
            },
        ))
    }
}

impl Pose {
    fn with_height(mut self, height: Height) -> Option<Self> {
        match height {
            Height::Unknown => return None,
            Height::Disabled => {}
            Height::Enabled { current, reference } => {
                if !current.is_finite() || !reference.is_finite() {
                    return None;
                }
                self.position.z += current - reference;
            }
        }
        self.position.is_finite().then_some(self)
    }

    /// Native AdjustCameraPos adds in the imported scene frame, before rebasing.
    pub fn adjust(&mut self, offset: Vec3) -> bool {
        if !offset.is_finite() {
            return false;
        }
        self.position += offset;
        true
    }
}

/// Sequential native f32 scaling/truncation against actual renderer screen size.
/// None means no usable allocation; the host retains its existing target/state.
pub(super) fn texture_extent(request: IVec2, screen: UVec2) -> Option<UVec2> {
    if request.min_element() < 1
        || screen.min_element() == 0
        || screen.max_element() > i32::MAX as u32
    {
        return None;
    }
    let mut width = request.x;
    let mut height = request.y;
    // Native cvttss2si returns its indefinite signed value on overflow; Rust's
    // saturating float cast would instead invent a usable positive dimension.
    let truncate = |value: f32| {
        if !value.is_finite() || value >= 2147483648. || value < -2147483648. {
            i32::MIN
        } else {
            value as i32
        }
    };
    if width > screen.x as i32 {
        let ratio = height as f32 / width as f32;
        width = screen.x as i32;
        height = truncate(width as f32 * ratio);
    }
    if height > screen.y as i32 {
        let ratio = width as f32 / height as f32;
        height = screen.y as i32;
        width = truncate(height as f32 * ratio);
    }
    (width > 0 && height > 0).then_some(UVec2::new(width as u32, height as u32))
}

#[cfg(test)]
mod tests;
