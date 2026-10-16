//! Camera paths of the login scene: `worlds/login2/cinemas/<name>.xml` (CryEngine track view sequences).
//!
//! A sequence has nodes; the camera node (`Type="3"`) carries tracks: `ParamId 1` = position (world
//! metres, cry space), `2` = rotation quaternion written **`w,x,y,z`** (a camera looks along local +Y,
//! checked against the character-select scene: the camera looks at the character standing 3.6 m away),
//! `0` = field of view in degrees. Keys are `<Key time=".." value=".."/>`.
//!
//! The cinema coordinates are world coordinates: login2's camera positions (x about 1652) lie in cell
//! `001_000`, the entity positions in that cell's `entities.xml` are the same minus 1024.

use bevy::math::{Quat, Vec3};

#[derive(Clone, Debug, Default)]
pub struct CinemaCamera {
    pub name: String,
    pub duration: f32,
    pub position: Vec<(f32, Vec3)>,
    /// cry space rotation (`w,x,y,z` in the file).
    pub rotation: Vec<(f32, Quat)>,
    pub fov: Vec<(f32, f32)>,
}

/// One evaluated camera state.
#[derive(Clone, Copy, Debug)]
pub struct CameraPose {
    /// Cry space position.
    pub position: Vec3,
    /// Cry space orientation (forward = local +Y).
    pub rotation: Quat,
    pub fov_deg: f32,
}

impl CameraPose {
    /// Cry space forward vector.
    pub fn forward(&self) -> Vec3 {
        self.rotation * Vec3::Y
    }
}

fn floats(s: &str) -> Vec<f32> {
    s.split(',').filter_map(|p| p.trim().parse().ok()).collect()
}

fn ease(u: f32) -> f32 {
    u * u * (3.0 - 2.0 * u)
}

fn segment<T: Copy>(keys: &[(f32, T)], t: f32) -> Option<(T, T, f32)> {
    let (first, last) = (keys.first()?, keys.last()?);
    if t <= first.0 {
        return Some((first.1, first.1, 0.0));
    }
    if t >= last.0 {
        return Some((last.1, last.1, 0.0));
    }
    let i = keys.iter().position(|k| k.0 > t)?;
    let (a, b) = (keys[i - 1], keys[i]);
    Some((a.1, b.1, ease(((t - a.0) / (b.0 - a.0)).clamp(0.0, 1.0))))
}

impl CinemaCamera {
    /// The first camera node of a cinema file's XML.
    pub fn parse(xml: &str) -> Option<CinemaCamera> {
        let doc = roxmltree::Document::parse(xml).ok()?;
        let seq = doc.root_element();
        let duration = seq.attribute("EndTime").and_then(|v| v.parse().ok()).unwrap_or(0.0);
        let node = doc.descendants().find(|n| n.has_tag_name("Node") && n.attribute("Type") == Some("3"))?;
        let mut cam = CinemaCamera { name: node.attribute("Name").unwrap_or("").to_string(), duration, ..Default::default() };
        for track in node.children().filter(|n| n.has_tag_name("Track")) {
            let id = track.attribute("ParamId").unwrap_or("");
            for key in track.children().filter(|n| n.has_tag_name("Key")) {
                let t: f32 = key.attribute("time").and_then(|v| v.parse().ok()).unwrap_or(0.0);
                let v = floats(key.attribute("value").unwrap_or(""));
                match (id, v.len()) {
                    ("1", 3) => cam.position.push((t, Vec3::new(v[0], v[1], v[2]))),
                    ("2", 4) => cam.rotation.push((t, Quat::from_xyzw(v[1], v[2], v[3], v[0]).normalize())),
                    ("0", 1) => cam.fov.push((t, v[0])),
                    _ => {}
                }
            }
        }
        if cam.position.is_empty() {
            return None;
        }
        Some(cam)
    }

    /// Reads and parses `worlds/<world>/cinemas/<name>.xml` from the pak.
    pub fn load(world: &str, name: &str) -> Option<CinemaCamera> {
        let xml = aa_pak::Pak::global().read_string(&format!("worlds/{world}/cinemas/{name}.xml")).ok()?;
        CinemaCamera::parse(&xml)
    }

    pub fn pose(&self, t: f32) -> CameraPose {
        let position = segment(&self.position, t).map_or(Vec3::ZERO, |(a, b, u)| a.lerp(b, u));
        let rotation = segment(&self.rotation, t).map_or(Quat::IDENTITY, |(a, b, u)| a.slerp(b, u));
        let fov_deg = segment(&self.fov, t).map_or(60.0, |(a, b, u)| a + (b - a) * u);
        CameraPose { position, rotation, fov_deg }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_world_select_camera() {
        if !crate::pak_available() {
            return;
        }
        let c = CinemaCamera::load("login2", "01_world_select_e1").unwrap();
        assert_eq!(c.name, "cam_server");
        let p = c.pose(0.0);
        assert!((p.position - Vec3::new(1652.551, 465.72119, 164.7596)).length() < 1e-3);
        assert_eq!(p.fov_deg, 80.0);
        // looks west, a little up
        let f = p.forward();
        assert!(f.x < -0.9 && f.z > 0.1, "{f}");
        let end = c.pose(10.0);
        assert!((end.position - Vec3::new(1655.411, 461.6911, 156.34689)).length() < 1e-3);
        assert_eq!(end.fov_deg, 30.0);
    }

    #[test]
    fn character_select_camera_looks_at_the_character() {
        if !crate::pak_available() {
            return;
        }
        let c = CinemaCamera::load("login2", "03_character_select_s").unwrap();
        let p = c.pose(0.0);
        // `select_character` stands at (1651.83, 461.45, 159.82)
        let to_char = (Vec3::new(1651.83, 461.45, 159.82) - p.position).normalize();
        assert!(p.forward().dot(to_char) > 0.9, "{}", p.forward().dot(to_char));
    }
}
