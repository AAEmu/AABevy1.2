//! `.lmg` locomotion groups (XML): a set of clips placed on a 2D "movement direction" plane.
//!
//! ```xml
//! <LocomotionGroup>
//!  <BLENDTYPE type="XL15"/>  <CAPS code="RUN_"/>
//!  <ExampleList>
//!   <Example Position=" 0, 1, 0" AName="fist_mo_normal_run_f"/>      forward
//!   <Example Position="-1, 1, 0" AName="fist_mo_normal_run_fl"/>     forward-left ...
//!  </ExampleList>
//!  <TimeList><TimeRange Range="0.0, 0.3" IndexValue="0"/> ... </TimeList>   (foot-phase ranges)
//! </LocomotionGroup>
//! ```
//!
//! `Position` is (strafe x, forward y, speed-tier z); several examples share a direction, the
//! tiers (walk / run / sprint, uphill / downhill) are separate `Example`s at the same position.
//! [`Lmg::blend`] returns the weights of the examples nearest the requested direction.

use glam::Vec2;

use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq)]
pub struct Example {
    /// (x = right, y = forward); z (always 0 in the 1.2 files) is dropped.
    pub direction: Vec2,
    /// Animation name in the `.cal`.
    pub animation: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TimeRange {
    pub start: f32,
    pub end: f32,
    pub index: i32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lmg {
    /// `BLENDTYPE@type` (`XL15`, `MI02`, ...).
    pub blend_type: String,
    /// `CAPS@code` (`RUN_`, `M2I_`, ...).
    pub caps: String,
    pub examples: Vec<Example>,
    pub time_ranges: Vec<TimeRange>,
}

impl Lmg {
    pub fn parse(xml: &str) -> Result<Lmg> {
        let xml = xml.trim_start_matches('\u{feff}');
        let doc = roxmltree::Document::parse(xml).map_err(|e| Error::Xml(e.to_string()))?;
        let root = doc.root_element();
        if !root.has_tag_name("LocomotionGroup") {
            return Err(Error::Format(format!("lmg root is <{}>", root.tag_name().name())));
        }
        let mut lmg = Lmg::default();
        for n in root.descendants().filter(|n| n.is_element()) {
            match n.tag_name().name() {
                "BLENDTYPE" => lmg.blend_type = n.attribute("type").unwrap_or("").to_string(),
                "CAPS" => lmg.caps = n.attribute("code").unwrap_or("").to_string(),
                "Example" => {
                    let pos = floats(n.attribute("Position").unwrap_or(""));
                    lmg.examples.push(Example {
                        direction: Vec2::new(pos.first().copied().unwrap_or(0.0), pos.get(1).copied().unwrap_or(0.0)),
                        animation: n.attribute("AName").unwrap_or("").to_string(),
                    });
                }
                "TimeRange" => {
                    let r = floats(n.attribute("Range").unwrap_or(""));
                    lmg.time_ranges.push(TimeRange {
                        start: r.first().copied().unwrap_or(0.0),
                        end: r.get(1).copied().unwrap_or(1.0),
                        index: n.attribute("IndexValue").and_then(|v| v.trim().parse().ok()).unwrap_or(0),
                    });
                }
                _ => {}
            }
        }
        Ok(lmg)
    }

    pub fn load(path: &str) -> Result<Lmg> {
        Lmg::parse(&String::from_utf8_lossy(&aa_cry::read(path)?))
    }

    /// Weights `(example index, weight)` for a movement direction (x = right, y = forward,
    /// each in -1..1; the vector is clamped to the unit disc). The nearest two examples of the
    /// first speed tier (the examples before the first repeated direction) are blended by
    /// inverse distance; weights sum to 1.
    pub fn blend(&self, dir: Vec2) -> Vec<(usize, f32)> {
        let dir = dir.clamp_length_max(1.0);
        let tier_end = {
            let mut seen: Vec<Vec2> = Vec::new();
            let mut end = self.examples.len();
            for (i, e) in self.examples.iter().enumerate() {
                if seen.contains(&e.direction) {
                    end = i;
                    break;
                }
                seen.push(e.direction);
            }
            end
        };
        // examples sitting at the origin are special cases (uphill / downhill), never blended
        let mut near: Vec<(usize, f32)> = self.examples[..tier_end]
            .iter()
            .enumerate()
            .filter(|(_, e)| e.direction != Vec2::ZERO)
            .map(|(i, e)| (i, e.direction.normalize().distance(dir.normalize_or_zero())))
            .collect();
        near.sort_by(|a, b| a.1.total_cmp(&b.1));
        near.truncate(2);
        if near.is_empty() {
            return Vec::new();
        }
        if near[0].1 < 1e-4 || near.len() == 1 {
            return vec![(near[0].0, 1.0)];
        }
        let w0 = 1.0 / near[0].1.max(1e-4);
        let w1 = 1.0 / near[1].1.max(1e-4);
        let s = w0 + w1;
        vec![(near[0].0, w0 / s), (near[1].0, w1 / s)]
    }
}

fn floats(s: &str) -> Vec<f32> {
    s.split(',').filter_map(|v| v.trim().parse().ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUN: &str = r#"<LocomotionGroup>
<BLENDTYPE type="XL15" />
<CAPS code="RUN_" />
<ExampleList>
    <Example Position=" 0, 1, 0" AName="run_f" />
    <Example Position="-1, 1, 0" AName="run_fl" />
    <Example Position=" 1, 1, 0" AName="run_fr" />
    <Example Position=" 0,-1, 0" AName="run_b" />
    <Example Position=" 0, 0, 0" AName="uphill" />
    <Example Position=" 0, 1, 0" AName="sprint_f" />
</ExampleList>
<TimeList><TimeRange Range="0.0, 0.3" IndexValue="0" /><TimeRange Range="0.3, 0.8" IndexValue="1" /></TimeList>
</LocomotionGroup>"#;

    #[test]
    fn parses() {
        let l = Lmg::parse(RUN).unwrap();
        assert_eq!(l.blend_type, "XL15");
        assert_eq!(l.caps, "RUN_");
        assert_eq!(l.examples.len(), 6);
        assert_eq!(l.examples[1].direction, Vec2::new(-1.0, 1.0));
        assert_eq!(l.time_ranges[1], TimeRange { start: 0.3, end: 0.8, index: 1 });
    }

    #[test]
    fn blends_nearest_directions() {
        let l = Lmg::parse(RUN).unwrap();
        assert_eq!(l.blend(Vec2::new(0.0, 1.0)), vec![(0, 1.0)]);
        let w = l.blend(Vec2::new(0.2, 1.0));
        assert_eq!(w.len(), 2);
        assert!((w.iter().map(|x| x.1).sum::<f32>() - 1.0).abs() < 1e-5);
        assert_eq!(w[0].0, 0);
        assert_eq!(w[1].0, 2);
        assert!(w[0].1 > w[1].1);
    }

    #[test]
    fn real_lmg() {
        if !aa_cry::pak_available() {
            return;
        }
        let l = Lmg::load("animations/nuian/male/lmg_files/fist_mo_normal_run.lmg").unwrap();
        assert_eq!(l.caps, "RUN_");
        assert!(l.examples.len() >= 8);
        assert!(!l.blend(Vec2::new(0.0, 1.0)).is_empty());
    }
}
