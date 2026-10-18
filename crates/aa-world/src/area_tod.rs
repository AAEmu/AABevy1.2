//! Native Group-14 polygon time-of-day areas. The actor point is independent of
//! the render camera; only explicitly qualified subjects participate.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{
    Arc, Mutex,
    mpsc::{Receiver, channel},
};

use bevy::prelude::*;

use crate::{
    coords::{bevy_to_cry, cell_origin},
    env::TimeOfDay,
    stream::{Streamer, WorldConfig},
};

/// Marks the single entity used for native ToD area events. This is equivalent
/// to an IEntity carrying native flags bit 0x20, not a camera designation.
/// The manager samples its world position plus native Cry-Z offset 0.01 m.
#[derive(Component)]
pub struct AreaToDSubject;

/// Authored polygon in global Cry coordinates; the XML height is not scaled.
#[derive(Clone, Debug)]
pub struct ToDArea {
    pub guid: u64,
    pub name: String,
    pub points: Vec<Vec3>,
    pub height: f32,
    pub falloff: f32,
    pub priority: i32,
    pub hidden: bool,
    pub tod: Arc<TimeOfDay>,
    min: Vec3,
    max: Vec3,
}

impl ToDArea {
    /// CArea::CalcPointWithin/CalcPointWithinDist (3208F090/32094360).
    /// Height <= 0 is unbounded vertically. Positive heights use minimum
    /// transformed vertex Z and unobstructed floor/ceiling (native defaults).
    pub fn inward_distance(&self, point: Vec3) -> Option<f32> {
        if !point.is_finite()
            || point.x < self.min.x
            || point.x > self.max.x
            || point.y < self.min.y
            || point.y > self.max.y
            || (self.height > 0.0 && (point.z < self.min.z || point.z > self.min.z + self.height))
        {
            return None;
        }
        let p = point.truncate();
        let mut crossings = 0;
        let mut distance = f32::INFINITY;
        for (a, b) in self.points.iter().zip(self.points.iter().cycle().skip(1)) {
            let (a, b) = (a.truncate(), b.truncate());
            let lo = a.y.min(b.y);
            let hi = a.y.max(b.y);
            if lo < p.y && p.y <= hi && p.x <= a.x.max(b.x) {
                // Native vertical edges use >=, sloping edges use >.
                let crossing = if a.x == b.x {
                    a.x >= p.x
                } else {
                    let slope = (b.y - a.y) / (b.x - a.x);
                    (p.y - (a.y - a.x * slope)) / slope > p.x
                };
                crossings += u32::from(crossing);
            }
            let ab = b - a;
            let t = if ab.length_squared() > 0.0 {
                ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0)
            } else {
                0.0
            };
            distance = distance.min((p - (a + ab * t)).length());
        }
        if crossings & 1 == 0 {
            return None;
        }
        if self.height > 0.0 {
            distance = distance
                .min(point.z - self.min.z)
                .min(self.min.z + self.height - point.z);
        }
        Some(distance)
    }
}

/// Parses only native polygon Group 14 records with nested ToD curves. Group 0
/// sound areas, sphere/box records and flag-bit-1 zone effects are not inferred.
pub fn parse_areas(xml: &str, cell: (i32, i32)) -> Vec<ToDArea> {
    let Ok(doc) = roxmltree::Document::parse(xml.trim_start_matches('\u{feff}')) else {
        return Vec::new();
    };
    let source = xml.trim_start_matches('\u{feff}');
    let Some(objects) = doc
        .root_element()
        .children()
        .find(|n| n.has_tag_name("Objects"))
    else {
        return Vec::new();
    };
    let nodes: HashMap<_, _> = objects
        .children()
        .filter(|n| n.has_tag_name("Entity"))
        .filter_map(|n| Some((crate::entities::guid(n.attribute("EntityGuid")?)?, n)))
        .collect();
    let mut result = Vec::new();
    for node in objects.children().filter(|n| n.has_tag_name("Entity")) {
        let Some(area) = node
            .children()
            .find(|n| n.has_tag_name("Area") && n.attribute("Group") == Some("14"))
        else {
            continue;
        };
        let Some(points) = area.children().find(|n| n.has_tag_name("Points")) else {
            continue;
        };
        let Some(tod) = area.children().find(|n| n.has_tag_name("TimeOfDay")) else {
            continue;
        };
        let Some(tod) = TimeOfDay::parse(&source[tod.range()]) else {
            continue;
        };
        let Some((matrix, hidden)) = crate::entities::transform(node, &nodes, &mut HashSet::new())
        else {
            continue;
        };
        let Some(points): Option<Vec<_>> = points
            .children()
            .filter(|n| n.has_tag_name("Point"))
            .map(|p| {
                Some(
                    matrix.transform_point3(Vec3::from(crate::entities::vector::<3>(
                        p.attribute("Pos")?,
                    )?)) + cell_origin(cell.0, cell.1),
                )
            })
            .collect()
        else {
            continue;
        };
        if points.len() < 3 || points.iter().any(|p| !p.is_finite()) {
            continue;
        }
        let height = area
            .attribute("Height")
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(0.0);
        let falloff = area
            .attribute("value1")
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(0.0);
        if !height.is_finite() || !falloff.is_finite() {
            continue;
        }
        let min = points
            .iter()
            .copied()
            .fold(Vec3::splat(f32::INFINITY), Vec3::min);
        let max = points
            .iter()
            .copied()
            .fold(Vec3::splat(f32::NEG_INFINITY), Vec3::max);
        result.push(ToDArea {
            guid: node
                .attribute("EntityGuid")
                .and_then(crate::entities::guid)
                .unwrap_or(0),
            name: node.attribute("Name").unwrap_or("").to_string(),
            points,
            height,
            falloff,
            priority: area
                .attribute("Priority")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
            hidden,
            tod: Arc::new(tod),
            min,
            max,
        });
    }
    result
}

/// 365EEC20: only lower-than-highest priorities give up excess weight. Highest
/// priority starts at zero, including the native all-negative-priorities case.
fn reduce_priorities(weights: &mut [(i32, f32)]) {
    let total: f32 = weights.iter().map(|(_, w)| w).sum();
    let highest = weights
        .iter()
        .filter(|(_, w)| *w > 0.0)
        .fold(0, |p, (next, _)| p.max(*next));
    let lower: f32 = weights
        .iter()
        .filter(|(p, w)| *p != highest && *w > 0.0)
        .map(|(_, w)| w)
        .sum();
    if total > 1.0 && lower > 0.0 {
        for (priority, weight) in weights {
            if *priority != highest && *weight > 0.0 {
                *weight = (*weight - (*weight / lower) * (total - 1.0)).clamp(0.0, 1.0);
            }
        }
    }
}

type Cell = (i32, i32);
type Job = (String, Cell);

/// Resident area curves and their evaluated native weights. No asset work runs
/// in the evaluator, and no actor marker means no automatic area override.
#[derive(Resource, Default)]
pub struct AreaToDState {
    world: Option<String>,
    cells: BTreeMap<Cell, Vec<ToDArea>>,
    pending: BTreeMap<Job, Mutex<Receiver<Vec<ToDArea>>>>,
    registered: Vec<(Cell, usize)>,
    raw: Vec<(i32, f32)>,
    pub(crate) inputs: Vec<(Arc<TimeOfDay>, f32)>,
    /// Sum of all valid native area inputs before the manager's ten-slot cap.
    pub(crate) total: f32,
    last: Option<Vec3>,
    changed: bool,
}

impl AreaToDState {
    pub fn loaded_cells(&self) -> usize {
        self.cells.len()
    }
    pub fn loaded_areas(&self) -> usize {
        self.cells.values().map(Vec::len).sum()
    }
    pub fn pending_cells(&self) -> usize {
        self.pending.len()
    }
    pub fn active_inputs(&self) -> usize {
        self.inputs.len()
    }
    pub fn total_weight(&self) -> f32 {
        self.total
    }

    fn reset(&mut self, world: &str) {
        self.world = Some(world.to_string());
        self.cells.clear();
        // Keep in-flight jobs until drained so rapid off/world changes cannot
        // enqueue more than two jobs on the shared pool. Old results are gated.
        self.registered.clear();
        self.inputs.clear();
        self.raw.clear();
        self.total = 0.0;
        self.last = None;
        self.changed = true;
    }

    fn evaluate(&mut self, point: Vec3) {
        if self.last == Some(point) && !self.changed {
            return;
        }
        self.changed = false;
        self.last = Some(point);
        self.registered.retain(|(cell, index)| {
            self.cells
                .get(cell)
                .and_then(|areas| areas.get(*index))
                .is_some_and(|a| !a.hidden && a.inward_distance(point).is_some())
        });
        for (cell, areas) in &self.cells {
            for (index, area) in areas.iter().enumerate() {
                if !area.hidden
                    && area.inward_distance(point).is_some()
                    && !self.registered.contains(&(*cell, index))
                {
                    self.registered.push((*cell, index));
                }
            }
        }
        self.raw.clear();
        for (cell, index) in &self.registered {
            let area = &self.cells[cell][*index];
            let denominator = if area.falloff == 0.0 {
                30.0
            } else {
                area.falloff
            };
            self.raw.push((
                area.priority,
                (area.inward_distance(point).unwrap() / denominator).clamp(0.0, 1.0),
            ));
        }
        reduce_priorities(&mut self.raw);
        self.inputs.clear();
        self.total = 0.0;
        for ((cell, index), (_, weight)) in self.registered.iter().zip(&self.raw) {
            if *weight > 0.0 {
                self.total += weight;
                if let Some((_, existing)) = self
                    .inputs
                    .iter_mut()
                    .find(|(tod, _)| Arc::ptr_eq(tod, &self.cells[cell][*index].tod))
                {
                    *existing = *weight; // 300A4270 replaces duplicate-pointer weights.
                } else if self.inputs.len() < 10 {
                    self.inputs
                        .push((self.cells[cell][*index].tod.clone(), *weight));
                }
            }
        }
    }
}

pub(crate) fn update_areas(
    cfg: Res<WorldConfig>,
    streamer: Res<Streamer>,
    subjects: Query<&GlobalTransform, With<AreaToDSubject>>,
    mut state: ResMut<AreaToDState>,
) {
    if state.world.as_deref() != Some(&cfg.world) {
        state.reset(&cfg.world);
    }
    let finished: Vec<_> = state
        .pending
        .iter()
        .filter_map(|(job, rx)| match rx.lock().unwrap().try_recv() {
            Ok(areas) => Some((job.clone(), areas)),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Some((job.clone(), Vec::new())),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
        })
        .collect();
    for ((world, cell), areas) in finished {
        state.pending.remove(&(world.clone(), cell));
        if world == cfg.world
            && cfg.environment
            && subjects.single().is_ok()
            && streamer.loaded_cells().any(|c| c == cell)
        {
            state.cells.insert(cell, areas);
            state.changed = true;
        }
    }
    let Ok(subject) = subjects.single() else {
        if !state.cells.is_empty() || state.last.is_some() {
            state.reset(&cfg.world);
        }
        return;
    };
    if !cfg.environment {
        state.reset(&cfg.world);
        return;
    }
    let mut point = bevy_to_cry(subject.translation());
    point.z += 0.01; // CAreaManager::Update 3209CD40 / ForceUpdate 32031670.
    // Areas follow cell lifetime, independently of object/vegetation visibility.
    state
        .cells
        .retain(|cell, _| streamer.loaded_cells().any(|c| c == *cell));
    if state
        .registered
        .iter()
        .any(|(cell, _)| !state.cells.contains_key(cell))
    {
        state.changed = true;
    }
    for cell in streamer.loaded_cells() {
        if state.pending.len() >= 2 {
            break;
        }
        if state.cells.contains_key(&cell) || state.pending.contains_key(&(cfg.world.clone(), cell))
        {
            continue;
        }
        let (tx, rx) = channel();
        state
            .pending
            .insert((cfg.world.clone(), cell), Mutex::new(rx));
        let world = cfg.world.clone();
        aa_cry::workers::Workers::global().spawn(-50000.0, move || {
            let path = format!(
                "{}/client/entities.xml",
                crate::world::cell_dir(&world, cell.0, cell.1)
            );
            let areas = aa_pak::Pak::global()
                .read_string(&path)
                .ok()
                .map(|xml| parse_areas(&xml, cell))
                .unwrap_or_default();
            let _ = tx.send(areas);
        });
    }
    state.evaluate(point);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(height: f32) -> ToDArea {
        let xml = format!(
            r#"<Mission><Objects><Entity EntityGuid="A"><Area Group="14" Height="{height}" value1="3"><Points>
            <Point Pos="0,0,3"/><Point Pos="10,0,1"/><Point Pos="10,10,2"/><Point Pos="0,10,3"/>
            </Points><TimeOfDay sRGB="1"/></Area></Entity></Objects></Mission>"#
        );
        parse_areas(&xml, (0, 0)).remove(0)
    }

    #[test]
    fn native_polygon_caps_and_boundary_goldens() {
        // Original x86 3208F090/32094360, native-area-geometry.json.
        let area = square(4.0);
        for (point, expected) in [
            ([5.0, 5.0, 1.0], Some(0.0)),
            ([5.0, 5.0, 2.0], Some(1.0)),
            ([5.0, 5.0, 4.0], Some(1.0)),
            ([5.0, 5.0, 5.0], Some(0.0)),
            ([5.0, 5.0, 5.1], None),
            ([0.0, 5.0, 3.0], None),
            ([10.0, 5.0, 3.0], Some(0.0)),
        ] {
            assert_eq!(area.inward_distance(Vec3::from(point)), expected);
        }
        assert_eq!(
            square(0.0).inward_distance(Vec3::new(5.0, 5.0, -10000.0)),
            Some(5.0)
        );
        assert_eq!(
            square(-1.0).inward_distance(Vec3::new(5.0, 5.0, 10000.0)),
            Some(5.0)
        );
    }

    #[test]
    fn native_priority_goldens_preserve_highest_and_negative_priorities() {
        // Whole original Seamless365EEC20 executed with bounded fake interfaces;
        // no replacement math in the x86 oracle. Masters here include setter cases.
        for (mut values, expected) in [
            (vec![(0, 1.0 / 3.0)], vec![0.3333333432674408]),
            (vec![(0, 1.0), (1, 1.0)], vec![0.0, 1.0]),
            (
                vec![(2, 0.8), (2, 0.8), (1, 0.8)],
                vec![0.800000011920929, 0.800000011920929, 0.0],
            ),
            (vec![(-2, 0.8), (-1, 0.8)], vec![0.5, 0.5]),
            (vec![(1, 0.5), (0, 0.75)], vec![0.5, 0.5]),
        ] {
            reduce_priorities(&mut values);
            for ((_, actual), expected) in values.iter().zip(expected) {
                assert!((*actual - expected as f32).abs() < 2e-7);
            }
        }
    }

    #[test]
    fn area_parser_parent_transform_hidden_and_group_scope() {
        let xml = r#"<Mission><Objects>
          <Entity EntityGuid="A" Pos="10,20,30" Scale="2,3,4" HiddenInGame="1"/>
          <Entity EntityGuid="B" ParentGUID="A" Pos="1,2,3"><Area Group="14" Height="4" value1="7" Priority="9"><Points>
            <Point Pos="0,0,0"/><Point Pos="10,0,0"/><Point Pos="10,10,0"/>
          </Points><TimeOfDay sRGB="1"/></Area></Entity>
          <Entity><Area Group="0"><Points><Point Pos="0,0,0"/><Point Pos="10,0,0"/><Point Pos="10,10,0"/></Points><TimeOfDay/></Area></Entity>
          <Entity><Area Group="14" SphereRadius="5"><TimeOfDay/></Area></Entity>
        </Objects></Mission>"#;
        let areas = parse_areas(xml, (1, 2));
        assert_eq!(areas.len(), 1);
        let area = &areas[0];
        assert_eq!(area.points[0], Vec3::new(1036.0, 2074.0, 42.0));
        assert_eq!(area.points[1], Vec3::new(1056.0, 2074.0, 42.0));
        assert_eq!(
            (area.height, area.falloff, area.priority, area.hidden),
            (4.0, 7.0, 9, true)
        );
    }

    #[test]
    fn registration_order_budget_falloff_and_unload() {
        let mut state = AreaToDState::default();
        state
            .cells
            .insert((0, 0), (0..12).map(|_| square(0.0)).collect());
        state.evaluate(Vec3::new(5.0, 5.0, 0.0));
        assert_eq!(state.inputs.len(), 10, "300A4270 manager capacity");
        assert_eq!(
            state.total, 12.0,
            "365EEC20 sums even rejected manager inputs"
        );
        state.evaluate(Vec3::new(1.5, 5.0, 0.0));
        assert_eq!(state.total, 6.0);
        state.evaluate(Vec3::new(-1.0, 5.0, 0.0));
        assert_eq!(state.total, 0.0);
        assert!(state.registered.is_empty());
        state.cells.get_mut(&(0, 0)).unwrap().truncate(1);
        state.cells.get_mut(&(0, 0)).unwrap()[0].falloff = 0.0;
        state.changed = true;
        state.evaluate(Vec3::new(5.0, 5.0, 0.0));
        assert_eq!(
            state.total,
            5.0 / 30.0,
            "only exactly zero selects native 30 m default"
        );
        state.cells.get_mut(&(0, 0)).unwrap()[0].falloff = -3.0;
        state.changed = true;
        state.evaluate(Vec3::new(5.0, 5.0, 0.0));
        assert!(state.inputs.is_empty());
        // In-flight jobs stay bounded while world/off resets discard resident state.
        let (tx, rx) = channel();
        state.pending.insert(("old".into(), (0, 0)), Mutex::new(rx));
        state.reset("next");
        assert_eq!(state.pending.len(), 1);
        assert!(state.cells.is_empty() && state.inputs.is_empty() && state.registered.is_empty());
        tx.send(Vec::new()).unwrap();
    }

    #[test]
    fn original_login_polygon_matches_native_geometry_and_legacy_curves() {
        if !crate::pak_available() {
            return;
        }
        let xml = aa_pak::Pak::global()
            .read_string("worlds/login2/cells/001_000/client/entities.xml")
            .unwrap();
        let areas = parse_areas(&xml, (1, 0));
        let area = areas
            .iter()
            .find(|a| a.name == "login_224_oktop_2")
            .unwrap();
        assert_eq!(
            (
                area.guid,
                area.height,
                area.falloff,
                area.priority,
                area.hidden
            ),
            (0x49DC7BE26212E2FE, 0.0, 3.0, 0, false)
        );
        for (point, expected) in [
            ([1652.0, 465.0, 130.0], Some(34.52779769897461)),
            ([1652.0, 465.0, -10000.0], Some(34.52779769897461)),
            ([1618.0, 431.0, 130.0], Some(0.529569685459137)),
            ([1618.0, 430.0, 130.0], Some(0.14320269227027893)),
            ([1618.0, 429.0, 130.0], None),
            ([1590.0, 465.0, 130.0], None),
            ([1700.0, 465.0, 130.0], Some(0.9327362179756165)),
            ([1652.0, 525.0, 130.0], None),
        ] {
            let distance = area.inward_distance(Vec3::from(point));
            assert_eq!(distance.is_some(), expected.is_some(), "{point:?}");
            if let (Some(a), Some(b)) = (distance, expected) {
                assert!((a - b).abs() < 0.0002, "{point:?}: {a} != native {b}");
            }
        }
        let doc = roxmltree::Document::parse(&xml).unwrap();
        let tod = doc
            .descendants()
            .find(|n| n.has_tag_name("Entity") && n.attribute("Name") == Some("login_224_oktop_2"))
            .unwrap()
            .descendants()
            .find(|n| n.has_tag_name("TimeOfDay"))
            .unwrap();
        let native = TimeOfDay::parse(&xml[tod.range()]).unwrap();
        assert_eq!(
            area.tod.color("Sun color", 0.3),
            native.color("Sun color", 0.3)
        );
    }
}
