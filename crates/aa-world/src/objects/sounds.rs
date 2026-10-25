//! Placed sounds: the `AmbientVolume`, `RandomSoundVolume`, `SoundEventSpot` and `ReverbVolume` entities of a cell's
//! `entities.xml`, with the `AreaShape` outlines that own them. They become entities that play (through `aa-audio`) while
//! the listener is in their area, and are despawned with the cell, as the effects are ([`super::effects`]).
//!
//! What the original entity scripts do (`entities/sound/*.lua` in the pak) and what is done here:
//!
//! * **AmbientVolume**: the entity is bound to the `AreaShape`(s) that list its GUID. The sound starts when the player comes
//!   within `OuterRadius` of the area (or into it) and stops when he leaves that zone; its volume is 1 inside the area and
//!   falls linearly to 0 at `OuterRadius` outside. The events are 2D loops with a `daylight` parameter: a day sound and a
//!   night sound are placed in the same area and the parameter (from the time of day, [`crate::sky::Environment`]) cross
//!   fades them. A 3D event is placed at the point of the area closest to the listener. An entity without a linked area
//!   stays silent in this placement path (counted in [`PlacedSoundStats::unbound_ambient`]); native cold sound-proxy
//!   creation and `SetSoundEffectRadius` do not create a point trigger. Mission/FlowGraph activation remains separate.
//! * **RandomSoundVolume**: while the player is inside its area, `MinWaitTime` seconds after entering and then every
//!   `MinWaitTime` to `MaxWaitTime` seconds, the one-shot event plays near the subject (`bRandomPosition`) or at the entity.
//!   Integral waits in 0..=16,777,215 use inclusive integer draws. Random X/Y offsets are independent
//!   `sin(integer 0..3)*DiscRadius` / `cos(integer 0..3)*DiscRadius`, as in the original script under ordinary rounding.
//!   The existing per-entity RNG and listener/focus subject remain approximations. Leaving the area stops it.
//!   Without an area it never plays.
//! * **SoundEventSpot**: a 3D point sound that plays from the start (`bPlay`) when the listener is near.
//! * **ReverbVolume**: read only. The preset of the strongest volume the listener is in or near is in [`ListenerReverb`];
//!   nothing applies it (`aa-audio` has no reverb).
//!
//! The areas' `<TimeOfDay>` window (hours) switches an area off outside it. Sounds the original needs `OutdoorOnly` for
//! play everywhere here (the client has no indoor test).
//!
//! `AA_AUDIO_LOG=1` logs every start and stop with the event and the position; `AA_NO_SOUND_ENTITIES=1` leaves them out.
//! The systems do nothing without an [`aa_audio::Engine`] (no audio plugin in the app).

use std::collections::HashMap;
use std::sync::Arc;

use aa_audio::{Engine, Sound, SpatialListener};
use bevy::prelude::*;

use crate::WorldFocus;
use crate::coords::{cell_origin, cry_to_bevy};
use crate::sky::Environment;

/// How far from a sound spot the listener must be for it to play (metres). The events fade out by their own distance
/// curves (most are silent beyond 100 m); this only decides when a voice is worth starting.
const SPOT_RANGE: f32 = 100.0;

/// The areas and sounds are re-evaluated this often (seconds).
const TICK: f32 = 0.1;

fn disabled() -> bool {
    std::env::var_os("AA_NO_SOUND_ENTITIES").is_some()
}

/// Whether to log every sound start (`AA_AUDIO_LOG=1`).
pub fn audio_log() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("AA_AUDIO_LOG").is_ok_and(|v| v != "0" && !v.is_empty()))
}

// ---- parsing ---------------------------------------------------------------------------------------------------------

/// An `AreaShape` that owns sound entities: a polygon prism, in cell-local metres (cry space).
#[derive(Clone, Debug, PartialEq)]
pub struct SoundArea {
    /// The outline on the ground plane (entity placement applied).
    pub outline: Vec<Vec2>,
    /// The vertical extent: from the lowest point to the highest point plus `Height`.
    pub z_min: f32,
    pub z_max: f32,
    /// `<TimeOfDay>` window in hours (start, end; a window that wraps midnight has start > end).
    pub hours: Option<(f32, f32)>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SoundKind {
    /// `OuterRadius`.
    Ambient { outer_radius: f32 },
    Random { disc_radius: f32, min_wait: f32, max_wait: f32, random_position: bool },
    /// A point sound.
    Spot,
}

/// A sound entity of a cell.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedSound {
    pub name: String,
    pub kind: SoundKind,
    /// The event path as the file has it (`Sounds/x2ambience:world2_day:beach_near`).
    pub event: String,
    /// Cell-local position.
    pub pos: Vec3,
    /// Indices into [`SoundEntities::areas`] of the areas that list this entity.
    pub areas: Vec<usize>,
}

/// A `ReverbVolume`.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedReverb {
    pub name: String,
    pub preset: String,
    pub pos: Vec3,
    pub outer_radius: f32,
    pub full_inside: bool,
    pub areas: Vec<usize>,
}

/// Everything a cell's `entities.xml` places for sound.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SoundEntities {
    pub areas: Vec<SoundArea>,
    pub sounds: Vec<PlacedSound>,
    pub reverbs: Vec<PlacedReverb>,
}

fn floats(s: Option<&str>) -> Vec<f32> {
    s.map(|s| s.split(',').filter_map(|v| v.trim().parse().ok()).collect()).unwrap_or_default()
}

fn number(props: roxmltree::Node, name: &str, default: f32) -> f32 {
    props.attribute(name).and_then(|v| v.trim().parse().ok()).unwrap_or(default)
}

fn flag(props: roxmltree::Node, name: &str, default: bool) -> bool {
    props.attribute(name).map_or(default, |v| v.trim() != "0")
}

/// The placement of an entity: position, `Rotate="w,x,y,z"`, `Scale`.
fn placement(e: roxmltree::Node) -> Option<(Vec3, Quat, Vec3)> {
    let p = floats(e.attribute("Pos"));
    let [x, y, z] = p[..] else { return None };
    let rot = match floats(e.attribute("Rotate"))[..] {
        [w, x, y, z] => Quat::from_xyzw(x, y, z, w).normalize(),
        _ => Quat::IDENTITY,
    };
    let scale = match floats(e.attribute("Scale"))[..] {
        [s] => Vec3::splat(s),
        [x, y, z] => Vec3::new(x, y, z),
        _ => Vec3::ONE,
    };
    Some((Vec3::new(x, y, z), rot, scale))
}

fn parse_area(e: roxmltree::Node) -> Option<(SoundArea, Vec<String>)> {
    let area = e.children().find(|c| c.has_tag_name("Area"))?;
    let guids: Vec<String> = area
        .children()
        .find(|c| c.has_tag_name("Entities"))?
        .children()
        .filter(|c| c.has_tag_name("Entity"))
        .filter_map(|c| c.attribute("EntityGuid"))
        .map(str::to_string)
        .collect();
    if guids.is_empty() {
        return None;
    }
    let (pos, rot, scale) = placement(e)?;
    let points: Vec<Vec3> = area
        .children()
        .find(|c| c.has_tag_name("Points"))?
        .children()
        .filter(|c| c.has_tag_name("Point"))
        .filter_map(|c| match floats(c.attribute("Pos"))[..] {
            [x, y, z] => Some(pos + rot * (Vec3::new(x, y, z) * scale)),
            _ => None,
        })
        .collect();
    if points.len() < 3 {
        return None;
    }
    let height = area.attribute("Height").and_then(|v| v.trim().parse::<f32>().ok()).unwrap_or(10.0) * scale.z.abs();
    let z_min = points.iter().map(|p| p.z).fold(f32::MAX, f32::min);
    let z_max = points.iter().map(|p| p.z).fold(f32::MIN, f32::max) + height;
    let hours = area.children().find(|c| c.has_tag_name("TimeOfDay")).and_then(|t| {
        let start = t.attribute("Start").or_else(|| t.attribute("TimeStart"))?.trim().parse::<f32>().ok()?;
        let end = t.attribute("End").or_else(|| t.attribute("TimeEnd"))?.trim().parse::<f32>().ok()?;
        // 0 to 24 is the whole day
        ((end - start).abs() < 23.99 && (start, end) != (0.0, 0.0)).then_some((start, end))
    });
    Some((SoundArea { outline: points.iter().map(|p| p.truncate()).collect(), z_min, z_max, hours }, guids))
}

/// The sound entities of an `entities.xml` and the areas that own them (positions are cell-local).
pub fn parse(xml: &str) -> SoundEntities {
    const CLASSES: [&str; 4] = ["AmbientVolume", "RandomSoundVolume", "SoundEventSpot", "ReverbVolume"];
    if !CLASSES.iter().any(|c| xml.contains(c)) {
        return SoundEntities::default();
    }
    let Ok(doc) = roxmltree::Document::parse(xml.trim_start_matches('\u{feff}')) else { return SoundEntities::default() };
    let entities = || doc.descendants().filter(|n| n.has_tag_name("Entity") && n.attribute("EntityClass").is_some());
    // the areas, by the GUIDs of the entities they list
    let mut areas = Vec::new();
    let mut owner: HashMap<String, Vec<usize>> = HashMap::new();
    for e in entities().filter(|e| e.attribute("EntityClass") == Some("AreaShape")) {
        if let Some((area, guids)) = parse_area(e) {
            for g in guids {
                owner.entry(g).or_default().push(areas.len());
            }
            areas.push(area);
        }
    }
    let mut out = SoundEntities { areas, ..Default::default() };
    for e in entities() {
        let class = e.attribute("EntityClass").unwrap_or_default();
        if !CLASSES.contains(&class) {
            continue;
        }
        let Some(props) = e.children().find(|c| c.has_tag_name("Properties")) else { continue };
        let Some((pos, _, _)) = placement(e) else { continue };
        let name = e.attribute("Name").unwrap_or_default().to_string();
        let owners = e.attribute("EntityGuid").and_then(|g| owner.get(g)).cloned().unwrap_or_default();
        if !flag(props, "bEnabled", true) {
            continue;
        }
        if class == "ReverbVolume" {
            let preset = props.attribute("reverbpresetReverbPreset").map(str::trim).unwrap_or_default();
            // three entities hold a sound path in this field (a slip of the data)
            if preset.is_empty() || preset.contains(':') || preset.contains('/') {
                continue;
            }
            out.reverbs.push(PlacedReverb {
                name,
                preset: preset.to_string(),
                pos,
                outer_radius: number(props, "OuterRadius", 2.0),
                full_inside: flag(props, "bFullEffectWhenInside", true),
                areas: owners,
            });
            continue;
        }
        let event = props.attribute("soundName").map(str::trim).unwrap_or_default();
        if event.is_empty() {
            continue;
        }
        let kind = match class {
            "AmbientVolume" => SoundKind::Ambient { outer_radius: number(props, "OuterRadius", 10.0) },
            "RandomSoundVolume" => SoundKind::Random {
                disc_radius: number(props, "DiscRadius", 10.0),
                min_wait: number(props, "MinWaitTime", 2.0),
                max_wait: number(props, "MaxWaitTime", 5.0),
                random_position: flag(props, "bRandomPosition", true),
            },
            // only a spot that plays by itself (`bPlay`) is heard; the others wait for a flow graph
            _ if props.attribute("bPlay").map(str::trim) == Some("1") => SoundKind::Spot,
            _ => continue,
        };
        out.sounds.push(PlacedSound { name, kind, event: event.to_string(), pos, areas: owners });
    }
    out
}

// ---- geometry --------------------------------------------------------------------------------------------------------

/// A [`SoundArea`] in world coordinates (cry space), with its bounding box.
#[derive(Debug)]
struct AreaWorld {
    outline: Vec<Vec2>,
    z: (f32, f32),
    /// min x, min y, max x, max y
    bounds: [f32; 4],
    hours: Option<(f32, f32)>,
}

impl AreaWorld {
    fn new(a: &SoundArea, origin: Vec3) -> AreaWorld {
        let outline: Vec<Vec2> = a.outline.iter().map(|p| *p + origin.truncate()).collect();
        let mut bounds = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for p in &outline {
            bounds = [bounds[0].min(p.x), bounds[1].min(p.y), bounds[2].max(p.x), bounds[3].max(p.y)];
        }
        AreaWorld { outline, z: (a.z_min + origin.z, a.z_max + origin.z), bounds, hours: a.hours }
    }

    /// Whether the area is switched on at `hour`.
    fn on_at(&self, hour: f32) -> bool {
        match self.hours {
            None => true,
            Some((s, e)) if s <= e => hour >= s && hour <= e,
            Some((s, e)) => hour >= s || hour <= e,
        }
    }

    /// The point of the prism closest to `p`, and the distance (0 inside).
    fn closest(&self, p: Vec3, reach: f32) -> Option<(Vec3, f32)> {
        let b = self.bounds;
        if p.x < b[0] - reach || p.x > b[2] + reach || p.y < b[1] - reach || p.y > b[3] + reach {
            return None;
        }
        let flat = p.truncate();
        let z = p.z.clamp(self.z.0, self.z.1);
        let c = if inside(&self.outline, flat) { flat } else { nearest_on_outline(&self.outline, flat).0 };
        let cp = Vec3::new(c.x, c.y, z);
        Some((cp, cp.distance(p)))
    }
}

/// Even-odd point in polygon test.
fn inside(poly: &[Vec2], p: Vec2) -> bool {
    let mut inside = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[j]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// The closest point of the polygon's outline to `p` and its distance.
fn nearest_on_outline(poly: &[Vec2], p: Vec2) -> (Vec2, f32) {
    let mut best = (poly[0], f32::MAX);
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let (a, b) = (poly[j], poly[i]);
        let ab = b - a;
        let t = if ab.length_squared() > 0.0 { ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
        let c = a + ab * t;
        let d = c.distance(p);
        if d < best.1 {
            best = (c, d);
        }
        j = i;
    }
    best
}

// ---- entities --------------------------------------------------------------------------------------------------------

/// The areas of a sound, in world space; none for a sound no area lists.
#[derive(Component)]
struct Areas(Vec<Arc<AreaWorld>>);

impl Areas {
    /// The closest point over the areas that are on at `hour`: `(point, distance)`.
    fn closest(&self, p: Vec3, reach: f32, hour: f32) -> Option<(Vec3, f32)> {
        self.0.iter().filter(|a| a.on_at(hour)).filter_map(|a| a.closest(p, reach)).min_by(|a, b| a.1.total_cmp(&b.1))
    }
}

#[derive(Component)]
struct Ambient {
    outer_radius: f32,
    /// What was last given to the `Sound` (to change it only when it moved).
    volume: f32,
    daylight: f32,
}

#[derive(Component)]
struct Random {
    event: String,
    disc_radius: f32,
    min_wait: f32,
    max_wait: f32,
    random_position: bool,
    world_pos: Vec3,
    inside: bool,
    /// Seconds (of `Time::elapsed_secs_f64`) of the next play.
    next: f64,
    current: Option<Entity>,
    seed: u64,
}

#[derive(Component)]
struct Spot {
    world_pos: Vec3,
}

#[derive(Component)]
struct Reverb {
    preset: String,
    outer_radius: f32,
    full_inside: bool,
    world_pos: Vec3,
}

/// A one-shot voice of a [`Random`]: gone after this many seconds even if the engine never reports it ended (a muted run).
#[derive(Component)]
struct Expires(f32);

/// The reverb preset of the strongest `ReverbVolume` around the listener. Read from the data; nothing applies it.
#[derive(Resource, Default, Clone, Debug, PartialEq)]
pub struct ListenerReverb {
    pub preset: Option<String>,
    /// 1 inside a volume that is fully effective inside, falling to 0 at `OuterRadius` outside.
    pub weight: f32,
}

/// The cost of the evaluation, logged with `AA_AUDIO_LOG`: ticks, total and worst microseconds.
#[derive(Default)]
struct Cost {
    ticks: u32,
    total: f64,
    worst: f64,
}

/// What the placed sounds do now, for logs and checks.
#[derive(Resource, Default, Clone, Debug, PartialEq)]
pub struct PlacedSoundStats {
    pub ambient: usize,
    pub ambient_playing: usize,
    /// Ambient slots with no linked area; retained as silent data.
    pub unbound_ambient: usize,
    pub random: usize,
    pub random_inside: usize,
    pub spots: usize,
    pub spots_playing: usize,
    pub reverbs: usize,
}

pub(crate) fn build(app: &mut App) {
    if disabled() {
        return;
    }
    app.init_resource::<ListenerReverb>().init_resource::<PlacedSoundStats>().add_systems(Update, (update_sounds, expire));
}

/// Spawns the sound entities of a cell; returns them.
pub fn spawn_cell(commands: &mut Commands, cell: (i32, i32), placed: &SoundEntities) -> Vec<Entity> {
    spawn_at(commands, cell_origin(cell.0, cell.1), placed)
}

/// Spawns the sound entities of a cell whose cell-local cry space starts at `origin` (cry space of the scene the
/// listener is in: the world, or a login stage that places a cell elsewhere); returns them.
pub fn spawn_at(commands: &mut Commands, origin: Vec3, placed: &SoundEntities) -> Vec<Entity> {
    if disabled() || (placed.sounds.is_empty() && placed.reverbs.is_empty()) {
        return Vec::new();
    }
    let areas: Vec<Arc<AreaWorld>> = placed.areas.iter().map(|a| Arc::new(AreaWorld::new(a, origin))).collect();
    let pick = |idx: &[usize]| Areas(idx.iter().filter_map(|&i| areas.get(i).cloned()).collect());
    let mut out = Vec::new();
    for (n, s) in placed.sounds.iter().enumerate() {
        let world_pos = origin + s.pos;
        let transform = Transform::from_translation(cry_to_bevy(world_pos));
        let name = Name::new(format!("sound {}", s.name));
        let id = match &s.kind {
            SoundKind::Ambient { outer_radius } => {
                commands.spawn((name, transform, pick(&s.areas), Ambient { outer_radius: *outer_radius, volume: 0.0, daylight: -1.0 }, EventName(s.event.clone()))).id()
            }
            SoundKind::Random { disc_radius, min_wait, max_wait, random_position } => commands
                .spawn((
                    name,
                    transform,
                    pick(&s.areas),
                    Random {
                        event: s.event.clone(),
                        disc_radius: *disc_radius,
                        min_wait: *min_wait,
                        max_wait: max_wait.max(*min_wait),
                        random_position: *random_position,
                        world_pos,
                        inside: false,
                        next: 0.0,
                        current: None,
                        seed: (origin.x.to_bits() as u64) << 32 ^ origin.y.to_bits() as u64 ^ (n as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15),
                    },
                ))
                .id(),
            SoundKind::Spot => commands.spawn((name, transform, Spot { world_pos }, EventName(s.event.clone()))).id(),
        };
        out.push(id);
    }
    for r in &placed.reverbs {
        let world_pos = origin + r.pos;
        let reverb = Reverb { preset: r.preset.clone(), outer_radius: r.outer_radius, full_inside: r.full_inside, world_pos };
        out.push(commands.spawn((Name::new(format!("reverb {}", r.name)), Transform::from_translation(cry_to_bevy(world_pos)), pick(&r.areas), reverb)).id());
    }
    out
}

/// The event an ambient volume or a spot plays.
#[derive(Component)]
struct EventName(String);

/// A small xorshift for the random plays (no `rand` in the tree).
fn next_random(seed: &mut u64) -> f32 {
    let mut x = *seed | 1;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *seed = x;
    (x >> 40) as f32 / (1u64 << 24) as f32
}

/// The original ALB calls inclusive native `math.random(min, max)`. Its fractional argument conversion uses
/// x87 FISTP and the live rounding mode is unclosed. Apply integer sampling only to exact nonnegative integers
/// through 2^24-1 (every result fits f32 exactly); keep the prior policy for all other authored values.
/// `unit` comes from next_random's unchanged [0,1) stream. Widen before computing the width/product/sum so a
/// near-one draw or a high lower bound cannot round the result up past max. Native global RNG parity is separate.
fn random_wait(min: f32, max: f32, unit: f32) -> f32 {
    if min >= 0.0 && min <= max && max <= 16_777_215.0 && min.fract() == 0.0 && max.fract() == 0.0 {
        (f64::from(min) + (f64::from(unit) * (f64::from(max) - f64::from(min) + 1.0)).floor()) as f32
    } else {
        min + (max - min) * unit
    }
}

/// 1 by day, 0 by night (the time of day's `day`); 1 when the world has no environment.
fn daylight(env: Option<&Environment>) -> f32 {
    env.and_then(|e| e.sample.as_ref()).map_or(1.0, |s| s.day.clamp(0.0, 1.0))
}

fn log_start(what: &str, event: &str, at: Vec3, extra: &str) {
    if audio_log() {
        info!("audio: {what} {event} at ({:.1}, {:.1}, {:.1}){extra}", at.x, at.y, at.z);
    }
}

type Listener<'w, 's> = Query<'w, 's, &'static GlobalTransform, With<SpatialListener>>;
type Focus<'w, 's> = Query<'w, 's, &'static GlobalTransform, With<WorldFocus>>;

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn update_sounds(
    time: Res<Time>,
    mut since: Local<f32>,
    mut commands: Commands,
    engine: Option<Res<Engine>>,
    env: Option<Res<Environment>>,
    listener: Listener,
    focus: Focus,
    mut ambient: Query<(Entity, &Areas, &mut Ambient, &EventName, &mut Transform, Option<&mut Sound>)>,
    mut random: Query<(Entity, &Areas, &mut Random, &Transform), Without<Ambient>>,
    spots: Query<(Entity, &Spot, &EventName, Has<Sound>), (Without<Ambient>, Without<Random>)>,
    reverbs: Query<(&Areas, &Reverb), (Without<Ambient>, Without<Random>)>,
    mut reverb: ResMut<ListenerReverb>,
    mut stats: ResMut<PlacedSoundStats>,
    mut last_stats: Local<PlacedSoundStats>,
    mut cost: Local<Cost>,
) {
    *since += time.delta_secs();
    if *since < TICK || engine.is_none() {
        return;
    }
    *since = 0.0;
    let started = audio_log().then(std::time::Instant::now);
    let Some(gt) = listener.iter().next().or_else(|| focus.iter().next()) else { return };
    let ear = crate::coords::bevy_to_cry(gt.translation());
    let env = env.as_deref();
    let hour = env.map_or(12.0, |e| e.hour);
    let day = daylight(env);
    let now = time.elapsed_secs_f64();
    let mut s = PlacedSoundStats::default();

    // ambient volumes: inside the area or within OuterRadius of it
    for (entity, areas, mut a, event, mut transform, sound) in &mut ambient {
        s.ambient += 1;
        let (target, volume) = if areas.0.is_empty() {
            s.unbound_ambient += 1;
            (ear, 0.0)
        } else {
            let (target, distance) = areas.closest(ear, a.outer_radius, hour).unwrap_or((ear, f32::MAX));
            let volume = if distance <= 0.0 { 1.0 } else { (1.0 - distance / a.outer_radius.max(0.01)).clamp(0.0, 1.0) };
            (target, volume)
        };
        match sound {
            None if volume > 0.0 => {
                a.volume = volume;
                a.daylight = day;
                let at = cry_to_bevy(target);
                transform.translation = at;
                log_start("ambient start", &event.0, target, &format!(" volume={volume:.2} daylight={day:.2}"));
                commands.entity(entity).insert(Sound::event(event.0.clone()).with_volume(volume).with_param("daylight", day));
                s.ambient_playing += 1;
            }
            Some(_) if volume <= 0.0 => {
                if audio_log() {
                    info!("audio: ambient stop {}", event.0);
                }
                commands.entity(entity).remove::<Sound>();
            }
            Some(mut sound) => {
                s.ambient_playing += 1;
                if (volume - a.volume).abs() > 0.02 || (day - a.daylight).abs() > 0.02 {
                    a.volume = volume;
                    a.daylight = day;
                    sound.volume = volume;
                    sound.params = vec![("daylight".to_string(), day)];
                }
                let at = cry_to_bevy(target);
                if transform.translation.distance_squared(at) > 1.0 {
                    transform.translation = at;
                }
            }
            None => {}
        }
    }

    // random sounds: only inside an area
    for (entity, areas, mut r, transform) in &mut random {
        s.random += 1;
        let inside = areas.closest(ear, 0.0, hour).is_some_and(|(_, d)| d <= 0.0);
        if inside {
            s.random_inside += 1;
        }
        if inside && !r.inside {
            r.next = now + f64::from(r.min_wait);
        }
        if !inside && r.inside
            && let Some(old) = r.current.take()
        {
            commands.entity(old).try_despawn();
        }
        r.inside = inside;
        if !inside || now < r.next {
            continue;
        }
        if let Some(old) = r.current.take() {
            commands.entity(old).try_despawn();
        }
        let unit = next_random(&mut r.seed);
        let wait = random_wait(r.min_wait, r.max_wait, unit);
        r.next = now + f64::from(wait);
        let target = if r.random_position {
            // Original Play takes TWO math.random(0, math.pi) draws. Pi converts to 3 under
            // nearest/down/toward-zero x87 modes; upward rounding and the native RNG's rare unit=1 stay open.
            let x = (next_random(&mut r.seed) * 4.0).floor().sin();
            let y = (next_random(&mut r.seed) * 4.0).floor().cos();
            ear + Vec3::new(x, y, 0.0) * r.disc_radius
        } else {
            r.world_pos
        };
        log_start("random", &r.event, target, &format!(" next in {wait:.0} s"));
        let local = cry_to_bevy(target) - transform.translation;
        let child = commands
            .spawn((ChildOf(entity), Transform::from_translation(local), Sound::event(r.event.clone()).despawn_when_done(), Expires(60.0)))
            .id();
        r.current = Some(child);
    }

    // sound spots: near the listener
    for (entity, spot, event, playing) in &spots {
        s.spots += 1;
        let near = spot.world_pos.distance(ear) <= SPOT_RANGE;
        if near && !playing {
            log_start("spot start", &event.0, spot.world_pos, "");
            commands.entity(entity).insert(Sound::event(event.0.clone()));
        } else if !near && playing {
            if audio_log() {
                info!("audio: spot stop {}", event.0);
            }
            commands.entity(entity).remove::<Sound>();
        }
        s.spots_playing += usize::from(near);
    }

    // reverb volumes: the strongest one around
    let mut best = ListenerReverb::default();
    for (areas, r) in &reverbs {
        s.reverbs += 1;
        let (distance, inside) = if areas.0.is_empty() {
            (r.world_pos.distance(ear), false)
        } else {
            areas.closest(ear, r.outer_radius, hour).map_or((f32::MAX, false), |(_, d)| (d, d <= 0.0))
        };
        let weight = if inside && r.full_inside { 1.0 } else { (1.0 - distance / r.outer_radius.max(0.01)).clamp(0.0, 1.0) };
        if weight > best.weight {
            best = ListenerReverb { preset: Some(r.preset.clone()), weight };
        }
    }
    if best.preset != reverb.preset || (best.weight - reverb.weight).abs() > 0.1 {
        if audio_log() {
            info!("audio: reverb {:?} weight {:.2} (read only, not applied)", best.preset, best.weight);
        }
        *reverb = best;
    }
    if s != *last_stats {
        if audio_log() {
            info!(
                "audio: placed sounds loaded: ambient {} ({} playing, {} without an area), random {} ({} inside), spots {} ({} playing), reverb volumes {}",
                s.ambient, s.ambient_playing, s.unbound_ambient, s.random, s.random_inside, s.spots, s.spots_playing, s.reverbs
            );
        }
        *last_stats = s.clone();
    }
    *stats = s;
    if let Some(t) = started {
        let us = t.elapsed().as_secs_f64() * 1e6;
        cost.ticks += 1;
        cost.total += us;
        cost.worst = cost.worst.max(us);
        if cost.ticks >= 100 {
            info!("audio: placed sound evaluation, {} ticks of 0.1 s: mean {:.0} us, worst {:.0} us", cost.ticks, cost.total / f64::from(cost.ticks), cost.worst);
            *cost = Cost::default();
        }
    }
}

fn expire(time: Res<Time>, mut commands: Commands, mut sounds: Query<(Entity, &mut Expires)>) {
    for (e, mut left) in &mut sounds {
        left.0 -= time.delta_secs();
        if left.0 <= 0.0 {
            commands.entity(e).try_despawn();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Resource)]
    struct Spawned(Vec<Entity>);

    fn sound_app(placed: SoundEntities, origin: Vec3, ear: Vec3) -> (App, Entity) {
        let mut app = App::new();
        app.init_resource::<Time>()
            .insert_resource(Engine::start(aa_audio::Settings { audible: false, ..default() }))
            .init_resource::<ListenerReverb>()
            .init_resource::<PlacedSoundStats>()
            .add_systems(Startup, move |mut commands: Commands| {
                let entities = spawn_at(&mut commands, origin, &placed);
                commands.insert_resource(Spawned(entities));
            })
            .add_systems(Update, update_sounds);
        let focus = app.world_mut().spawn((WorldFocus, GlobalTransform::from_translation(cry_to_bevy(ear)))).id();
        (app, focus)
    }

    fn tick(app: &mut App) {
        app.world_mut().resource_mut::<Time>().advance_by(std::time::Duration::from_millis(200));
        app.update();
    }

    #[test]
    fn original_unbound_ambient_stays_silent_and_retires_existing_sound_in_actual_system() {
        if !crate::pak_available() { return; }
        let xml = aa_pak::Pak::global().read_string("worlds/main_world/cells/014_015/client/entities.xml").unwrap();
        let mut placed = parse(&xml);
        placed.sounds.retain(|s| matches!(s.name.as_str(), "solz_AmbientVolume_sea04" | "solz_AmbientVolume_sea_night1"));
        placed.reverbs.clear();
        assert_eq!(placed.sounds.len(), 2);
        assert!(placed.sounds.iter().all(|s| s.areas.is_empty()));
        let origin = cell_origin(14, 15);
        let points: Vec<_> = placed.sounds.iter().map(|s| origin + s.pos).collect();
        let (mut app, focus) = sound_app(placed, origin, points[0]);
        tick(&mut app);
        let ids = app.world().resource::<Spawned>().0.clone();
        for point in points {
            app.world_mut().entity_mut(focus).insert(GlobalTransform::from_translation(cry_to_bevy(point)));
            tick(&mut app);
            let stats = app.world().resource::<PlacedSoundStats>();
            assert_eq!((stats.ambient, stats.unbound_ambient, stats.ambient_playing), (2, 2, 0));
            assert!(ids.iter().all(|&id| app.world().get::<Sound>(id).is_none()), "an unbound volume must not become a point sound at zero distance");
        }
        // Handles may already carry a Sound (an old fallback or caller insertion).
        for &id in &ids { app.world_mut().entity_mut(id).insert(Sound::event("sounds/test:unbound")); }
        tick(&mut app);
        assert!(ids.iter().all(|&id| app.world().get::<Sound>(id).is_none()));
        assert!(ids.iter().all(|&id| app.world().get::<Ambient>(id).is_some()), "retain slots and diagnostics");
        for &id in &ids { app.world_mut().despawn(id); }
        tick(&mut app);
        assert_eq!(app.world().resource::<PlacedSoundStats>().ambient, 0);
    }

    #[test]
    fn bound_ambient_keeps_actual_area_distance_hour_daylight_and_unload_behavior() {
        let mut placed = parse(XML);
        placed.sounds.retain(|s| s.name == "amb");
        placed.reverbs.clear();
        placed.areas[0].hours = Some((6.0, 18.0));
        let inside = Vec3::new(150.0, 150.0, 60.0);
        let (mut app, focus) = sound_app(placed, Vec3::ZERO, inside);
        tick(&mut app);
        let id = app.world().resource::<Spawned>().0[0];
        assert_eq!(app.world().get::<Sound>(id).unwrap().volume, 1.0);
        assert_eq!(app.world().get::<Sound>(id).unwrap().params, [("daylight".into(), 1.0)]);
        app.world_mut().entity_mut(focus).insert(GlobalTransform::from_translation(cry_to_bevy(Vec3::new(90.0, 150.0, 60.0))));
        tick(&mut app);
        assert_eq!(app.world().get::<Sound>(id).unwrap().volume, 0.75, "10m outside original 40m OuterRadius");
        app.world_mut().entity_mut(focus).insert(GlobalTransform::from_translation(cry_to_bevy(Vec3::new(0.0, 150.0, 60.0))));
        tick(&mut app);
        assert!(app.world().get::<Sound>(id).is_none());
        app.world_mut().entity_mut(focus).insert(GlobalTransform::from_translation(cry_to_bevy(inside)));
        tick(&mut app);
        assert_eq!(app.world().get::<Sound>(id).unwrap().volume, 1.0);
        let mut sample = crate::env::TimeOfDay::default().sample(12.0, crate::env::Lighting::default());
        sample.day = 0.25;
        app.insert_resource(Environment { sample: Some(sample), zone: None, hour: 12.0, ocean_fog: Vec3::ZERO, ocean_density: 0.0 });
        tick(&mut app);
        assert_eq!(app.world().get::<Sound>(id).unwrap().params, [("daylight".into(), 0.25)]);
        app.world_mut().resource_mut::<Environment>().hour = 22.0;
        tick(&mut app);
        assert!(app.world().get::<Sound>(id).is_none(), "keep the authored area time window");
        app.world_mut().resource_mut::<Environment>().hour = 12.0;
        tick(&mut app);
        assert!(app.world().get::<Sound>(id).is_some());
        assert_eq!(app.world().resource::<PlacedSoundStats>().unbound_ambient, 0);
        app.world_mut().despawn(id);
        tick(&mut app);
        assert_eq!(app.world().resource::<PlacedSoundStats>().ambient_playing, 0);
        assert!(app.world().get_entity(id).is_err());
    }

    // Original ALB Play with native math.random at controlled units from the existing xorshift seeds.
    // Host sin/cos goldens, all 16 ordinary independent angle pairs; see random_sound_source.py.
    // (seed, wait for 10..20, wait for 15..30, X offset at R30, Y offset at R30).
    const RANDOM_GOLDENS: &[(u64, f64, f64, f32, f32)] = &[
        (0x8ff34785799e5cbd, 12.0, 18.0, 0.0, 30.0),
        (0xf1bbcdcbfa53e0a8, 19.0, 29.0, 0.0, 16.20907),
        (0xbe1e08c47287358f, 14.0, 21.0, 0.0, -12.4844055),
        (0x1715609f7c746c69, 18.0, 26.0, 0.0, -29.699776),
        (0x0c2f590e5eacc7db, 20.0, 30.0, 25.24413, 30.0),
        (0x9e3779b97f4a7c15, 19.0, 28.0, 25.24413, 16.20907),
        (0xcfc0659b6017cfb1, 11.0, 16.0, 25.24413, -12.4844055),
        (0xe3779b97f4a7c150, 18.0, 27.0, 25.24413, -29.699776),
        (0xa195a45c672ef6df, 12.0, 18.0, 27.278923, 30.0),
        (0x11a25cd6ed909a22, 18.0, 26.0, 27.278923, 16.20907),
        (0x56e27eb562eddf5d, 14.0, 21.0, 27.278923, -12.4844055),
        (0xdaa66d2c7ddf743f, 19.0, 28.0, 27.278923, -29.699776),
        (0xfa8cfc37711c2db9, 14.0, 21.0, 4.2336, 30.0),
        (0x3c6ef372fe94f82a, 17.0, 26.0, 4.2336, 16.20907),
        (0x6a99b4b1f77dd0fc, 15.0, 22.0, 4.2336, -12.4844055),
        (0xa708a824f612c926, 20.0, 29.0, 4.2336, -29.699776),
    ];

    fn nuian_random_data() -> Option<SoundEntities> {
        if !crate::pak_available() { return None; }
        let xml = aa_pak::Pak::global().read_string("worlds/main_world/cells/015_015/client/entities.xml").unwrap();
        let mut placed = parse(&xml);
        placed.sounds.retain(|s| matches!(s.kind, SoundKind::Random { .. }));
        placed.reverbs.clear();
        assert_eq!(placed.sounds.len(), 2);
        for s in &placed.sounds {
            assert_eq!(s.areas.len(), 1);
            let (min, max) = match s.name.as_str() {
                "solz_RandomSoundVolume_seagull_random_1" => (10.0, 20.0),
                "solz_RandomSoundVolume_seagull_random_2" => (15.0, 30.0),
                other => panic!("unexpected original placement {other}"),
            };
            assert_eq!(s.kind, SoundKind::Random { disc_radius: 30.0, min_wait: min, max_wait: max, random_position: true });
        }
        Some(placed)
    }

    fn ready_random(app: &mut App, ids: &[Entity], seed: u64) -> f64 {
        let now = app.world().resource::<Time>().elapsed_secs_f64();
        for &id in ids {
            let mut r = app.world_mut().get_mut::<Random>(id).unwrap();
            r.seed = seed;
            r.next = now;
        }
        tick(app);
        app.world().resource::<Time>().elapsed_secs_f64()
    }

    #[test]
    fn original_random_integer_deadlines_match_alb_in_actual_system() {
        let Some(placed) = nuian_random_data() else { return };
        let origin = cell_origin(15, 15);
        let (mut app, _) = sound_app(placed, origin, origin + Vec3::new(500.0, 500.0, 200.0));
        tick(&mut app);
        let ids = app.world().resource::<Spawned>().0.clone();
        assert_eq!(app.world().resource::<PlacedSoundStats>().random_inside, 2);
        for &(seed, wait10, wait15, _, _) in RANDOM_GOLDENS {
            let now = ready_random(&mut app, &ids, seed);
            for &id in &ids {
                let r = app.world().get::<Random>(id).unwrap();
                let expected = if r.min_wait == 10.0 { wait10 } else { wait15 };
                assert_eq!(r.next, now + expected, "original integer timer, seed={seed:016x}");
            }
        }
    }

    #[test]
    fn original_random_independent_xy_matches_alb_in_actual_system() {
        let Some(placed) = nuian_random_data() else { return };
        let origin = cell_origin(15, 15);
        let ear = origin + Vec3::new(500.0, 500.0, 200.0);
        let (mut app, _) = sound_app(placed, origin, ear);
        tick(&mut app);
        let ids = app.world().resource::<Spawned>().0.clone();
        for &(seed, _, _, x, y) in RANDOM_GOLDENS {
            let old: Vec<_> = ids.iter().filter_map(|&id| app.world().get::<Random>(id).unwrap().current).collect();
            ready_random(&mut app, &ids, seed);
            for child in old { assert!(app.world().get_entity(child).is_err(), "replace the previous voice"); }
            for &id in &ids {
                let child = app.world().get::<Random>(id).unwrap().current.unwrap();
                let at = app.world().get::<Transform>(id).unwrap().translation + app.world().get::<Transform>(child).unwrap().translation;
                let expected = cry_to_bevy(ear + Vec3::new(x, y, 0.0));
                assert!(at.distance(expected) < 0.002, "ALB independent XY pair seed={seed:016x}: {at:?} vs {expected:?}");
                assert_eq!(at.y, expected.y, "retain the subject's Cry Z");
                assert_eq!(app.world().get::<Sound>(child).unwrap().event, "Sounds/x2ambience:world2_random:seagull");
            }
        }
    }

    #[test]
    fn random_wait_bounds_and_existing_rng_endpoints() {
        let low = next_random(&mut 1);
        let high = next_random(&mut 0xab4555ca962450e1);
        assert_eq!(low, 0.0);
        assert_eq!(high.to_bits(), 0x3f7fffff, "largest current unit stays below 1; native unit=1 remains separate");
        for (min, max) in [(10.0, 20.0), (15.0, 30.0), (0.0, 16_777_215.0), (16_777_213.0, 16_777_215.0), (16_777_215.0, 16_777_215.0)] {
            assert_eq!(random_wait(min, max, low), min);
            assert_eq!(random_wait(min, max, high), max, "no max+1 from host rounding");
        }
    }

    #[test]
    fn random_wait_keeps_fractional_out_of_domain_and_nonfinite_policy() {
        // Bit-exact prior f32 interpolation: fractional endpoints, negative integral endpoints,
        // a finite upper bound outside the documented integer domain, and inverted bounds.
        for (min, max, unit, expected) in [
            (2.25, 5.75, 0.25, 3.125_f32),
            (-2.0, 3.0, 0.5, 0.5),
            (0.0, 33_554_432.0, 0.25, 8_388_608.0),
            (5.5, 2.5, 0.25, 4.75),
        ] {
            assert_eq!(random_wait(min, max, unit).to_bits(), expected.to_bits());
        }
        assert!(random_wait(0.0, f32::INFINITY, 0.0).is_nan());
        assert_eq!(random_wait(0.0, f32::INFINITY, 0.5), f32::INFINITY);
        assert!(random_wait(f32::NAN, 10.0, 0.5).is_nan());
    }

    #[test]
    fn original_random_keeps_first_delay_near_leave_reentry_hour_and_unload_controls() {
        let Some(mut placed) = nuian_random_data() else { return };
        // The original Nuian owner has no hour restriction. Exercise the existing authored-window path explicitly.
        let owner = placed.sounds[0].areas[0];
        placed.areas[owner].hours = Some((6.0, 18.0));
        let origin = cell_origin(15, 15);
        let inside = origin + Vec3::new(500.0, 500.0, 200.0);
        let near = origin + Vec3::new(-10.0, 500.0, 200.0);
        let area = AreaWorld::new(&placed.areas[owner], origin);
        assert_eq!(area.closest(inside, 0.0).unwrap().1, 0.0);
        assert!(area.closest(near, 40.0).is_some_and(|(_, d)| d > 0.0 && d < 40.0));
        let (mut app, focus) = sound_app(placed, origin, near);
        tick(&mut app);
        let ids = app.world().resource::<Spawned>().0.clone();
        assert_eq!(app.world().resource::<PlacedSoundStats>().random_inside, 0, "near does not admit a random volume");
        app.world_mut().entity_mut(focus).insert(GlobalTransform::from_translation(cry_to_bevy(inside)));
        tick(&mut app);
        let id = *ids.iter().find(|&&id| app.world().get::<Random>(id).unwrap().min_wait == 10.0).unwrap();
        let entered = app.world().resource::<Time>().elapsed_secs_f64();
        let r = app.world().get::<Random>(id).unwrap();
        assert_eq!(r.next, entered + 10.0);
        assert!(r.current.is_none());
        let seed = r.seed;
        app.world_mut().resource_mut::<Time>().advance_by(std::time::Duration::from_millis(9800));
        app.update();
        let r = app.world().get::<Random>(id).unwrap();
        assert!(r.current.is_none());
        assert_eq!(r.seed, seed, "first delay must not consume entropy");
        tick(&mut app);
        let child = app.world().get::<Random>(id).unwrap().current.unwrap();
        assert!(app.world().get::<Sound>(child).is_some(), "play when the initial minimum expires");
        app.world_mut().entity_mut(focus).insert(GlobalTransform::from_translation(cry_to_bevy(near)));
        tick(&mut app);
        assert!(app.world().get_entity(child).is_err());
        assert!(app.world().get::<Random>(id).unwrap().current.is_none());
        app.world_mut().entity_mut(focus).insert(GlobalTransform::from_translation(cry_to_bevy(inside)));
        tick(&mut app);
        let now = app.world().resource::<Time>().elapsed_secs_f64();
        assert_eq!(app.world().get::<Random>(id).unwrap().next, now + 10.0);
        assert!(app.world().get::<Random>(id).unwrap().current.is_none());
        ready_random(&mut app, &ids, 1);
        let now = app.world().resource::<Time>().elapsed_secs_f64();
        for &id in &ids {
            let r = app.world().get::<Random>(id).unwrap();
            assert_eq!(r.next, now + f64::from(r.min_wait), "zero unit reaches the inclusive minimum");
        }
        let children: Vec<_> = ids.iter().map(|&id| app.world().get::<Random>(id).unwrap().current.unwrap()).collect();
        app.insert_resource(Environment { sample: None, zone: None, hour: 22.0, ocean_fog: Vec3::ZERO, ocean_density: 0.0 });
        tick(&mut app);
        assert_eq!(app.world().resource::<PlacedSoundStats>().random_inside, 0);
        assert!(children.iter().all(|&child| app.world().get_entity(child).is_err()));
        app.world_mut().resource_mut::<Environment>().hour = 12.0;
        tick(&mut app);
        assert_eq!(app.world().resource::<PlacedSoundStats>().random_inside, 2);
        assert!(ids.iter().all(|&id| app.world().get::<Random>(id).unwrap().current.is_none()));
        ready_random(&mut app, &ids, 1);
        let children: Vec<_> = ids.iter().map(|&id| app.world().get::<Random>(id).unwrap().current.unwrap()).collect();
        for &id in &ids { app.world_mut().despawn(id); }
        tick(&mut app);
        assert_eq!(app.world().resource::<PlacedSoundStats>().random, 0);
        assert!(children.iter().all(|&child| app.world().get_entity(child).is_err()), "unload retires child voices");
    }

    #[test]
    fn original_random_keeps_unbound_entity_position_subject_and_disabled_controls() {
        let Some(mut placed) = nuian_random_data() else { return };
        // Controlled overrides exercise existing branches while retaining the original positions/events/owner geometry.
        if let SoundKind::Random { random_position, .. } = &mut placed.sounds[0].kind { *random_position = false; }
        placed.sounds[1].areas.clear();
        let origin = cell_origin(15, 15);
        let inside = origin + Vec3::new(500.0, 500.0, 200.0);
        let (mut app, _) = sound_app(placed, origin, inside);
        tick(&mut app);
        let ids = app.world().resource::<Spawned>().0.clone();
        assert_eq!(app.world().resource::<PlacedSoundStats>().random_inside, 1);
        ready_random(&mut app, &ids, 1);
        let r = app.world().get::<Random>(ids[0]).unwrap();
        let child = r.current.unwrap();
        assert_eq!(r.seed, 0x40822041, "entity-position branch consumes only the wait draw");
        let at = app.world().get::<Transform>(ids[0]).unwrap().translation + app.world().get::<Transform>(child).unwrap().translation;
        assert_eq!(at, cry_to_bevy(r.world_pos));
        let r = app.world().get::<Random>(ids[1]).unwrap();
        assert!(!r.inside && r.current.is_none());
        assert_eq!(r.seed, 1, "an unbound random slot must not consume draws or play");
        let outside = origin + Vec3::new(-100.0, -100.0, 200.0);
        let listener = app.world_mut().spawn((SpatialListener, GlobalTransform::from_translation(cry_to_bevy(outside)))).id();
        tick(&mut app);
        assert!(app.world().get_entity(child).is_err(), "existing listener priority over focus is unchanged; not native actor authority");
        app.world_mut().despawn(listener);
        tick(&mut app);
        assert_eq!(app.world().resource::<PlacedSoundStats>().random_inside, 1, "existing WorldFocus fallback");
        assert!(app.world().get::<Random>(ids[0]).unwrap().current.is_none(), "reentry restarts the first delay");

        let xml = XML.replace("DiscRadius=\"30\"", "bEnabled=\"0\" DiscRadius=\"30\"");
        let mut disabled = parse(&xml);
        disabled.sounds.retain(|s| matches!(s.kind, SoundKind::Random { .. }));
        disabled.reverbs.clear();
        assert!(disabled.sounds.is_empty());
        let (mut app, _) = sound_app(disabled, Vec3::ZERO, Vec3::new(150.0, 150.0, 60.0));
        tick(&mut app);
        assert!(app.world().resource::<Spawned>().0.is_empty());
        assert_eq!(app.world().resource::<PlacedSoundStats>().random, 0);
    }

    const XML: &str = r#"<Mission><Objects>
      <Entity Name="amb" Pos="10,20,300" EntityClass="AmbientVolume" EntityGuid="aa11">
        <Properties bEnabled="1" soundName="Sounds/x2ambience:world2_day:beach_near" OuterRadius="40"/></Entity>
      <Entity Name="rnd" Pos="10,20,300" EntityClass="RandomSoundVolume" EntityGuid="bb22">
        <Properties DiscRadius="30" MinWaitTime="10" MaxWaitTime="20" soundName="Sounds/x2ambience:world2_random:seagull" bRandomPosition="1"/></Entity>
      <Entity Name="spot" Pos="1,2,3" EntityClass="SoundEventSpot" EntityGuid="cc33"><Properties bPlay="1" soundName="Sounds/x2ambience:world2_object:waterfall"/></Entity>
      <Entity Name="quiet" Pos="1,2,3" EntityClass="SoundEventSpot" EntityGuid="cc34"><Properties bPlay="0" soundName="Sounds/x.y:z"/></Entity>
      <Entity Name="off" Pos="1,2,3" EntityClass="AmbientVolume" EntityGuid="dd44"><Properties bEnabled="0" soundName="Sounds/x.y:z"/></Entity>
      <Entity Name="empty" Pos="1,2,3" EntityClass="AmbientVolume" EntityGuid="dd45"><Properties soundName=""/></Entity>
      <Entity Name="rev" Pos="5,5,5" EntityClass="ReverbVolume" EntityGuid="ee55"><Properties reverbpresetReverbPreset="x2_outdoor_1" OuterRadius="40" bFullEffectWhenInside="1"/></Entity>
      <Entity Name="slip" Pos="5,5,5" EntityClass="ReverbVolume" EntityGuid="ee56"><Properties reverbpresetReverbPreset="Sounds/x2ambience:login_stage:wind_soft_03"/></Entity>
      <Entity Name="shape" Pos="100,100,50" EntityClass="AreaShape" EntityGuid="ff66">
        <Area Id="0" Height="20"><Points><Point Pos="0,0,0"/><Point Pos="100,0,0"/><Point Pos="100,100,0"/><Point Pos="0,100,0"/></Points>
          <Entities><Entity EntityGuid="aa11"/><Entity EntityGuid="bb22"/><Entity EntityGuid="ee55"/></Entities><TimeOfDay /></Area></Entity>
    </Objects></Mission>"#;

    #[test]
    fn entities_and_areas_are_read() {
        let p = parse(XML);
        assert_eq!(p.areas.len(), 1);
        assert_eq!(p.areas[0].z_min, 50.0);
        assert_eq!(p.areas[0].z_max, 70.0);
        assert_eq!(p.areas[0].outline[2], Vec2::new(200.0, 200.0));
        assert_eq!(p.areas[0].hours, None);
        let names: Vec<_> = p.sounds.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["amb", "rnd", "spot"]);
        assert_eq!(p.sounds[0].areas, vec![0]);
        assert_eq!(p.sounds[0].kind, SoundKind::Ambient { outer_radius: 40.0 });
        assert!(matches!(p.sounds[1].kind, SoundKind::Random { disc_radius, min_wait, max_wait, random_position: true } if disc_radius == 30.0 && min_wait == 10.0 && max_wait == 20.0));
        assert!(p.sounds[2].areas.is_empty());
        assert_eq!(p.reverbs.len(), 1);
        assert_eq!(p.reverbs[0].preset, "x2_outdoor_1");
        assert_eq!(p.reverbs[0].areas, vec![0]);
    }

    #[test]
    fn area_geometry() {
        let p = parse(XML);
        let a = AreaWorld::new(&p.areas[0], Vec3::new(1024.0, 0.0, 0.0));
        // inside the square (1124..1224, 100..200) at a height within 50..70
        let (c, d) = a.closest(Vec3::new(1150.0, 150.0, 60.0), 40.0).unwrap();
        assert_eq!((d, c), (0.0, Vec3::new(1150.0, 150.0, 60.0)));
        // 10 m west of the edge
        let (c, d) = a.closest(Vec3::new(1114.0, 150.0, 60.0), 40.0).unwrap();
        assert!((d - 10.0).abs() < 1e-4 && (c.x - 1124.0).abs() < 1e-4);
        // above the prism: the vertical gap counts
        let (_, d) = a.closest(Vec3::new(1150.0, 150.0, 90.0), 40.0).unwrap();
        assert!((d - 20.0).abs() < 1e-4);
        // beyond the reach: not even tested
        assert!(a.closest(Vec3::new(1000.0, 150.0, 60.0), 40.0).is_none());
        assert!(a.on_at(3.0));
    }

    #[test]
    fn hour_windows() {
        let mut area = parse(XML).areas.remove(0);
        area.hours = Some((6.0, 18.0));
        let day = AreaWorld::new(&area, Vec3::ZERO);
        assert!(day.on_at(12.0) && !day.on_at(22.0));
        area.hours = Some((20.0, 5.0));
        let night = AreaWorld::new(&area, Vec3::ZERO);
        assert!(night.on_at(23.0) && night.on_at(2.0) && !night.on_at(12.0));
    }

    #[test]
    fn spawn_cell_data() {
        if !crate::pak_available() {
            return;
        }
        // the Nuian spawn cell: 3 ambient, 2 random, 1 reverb, all on `_sound_solzreed`
        let pak = aa_pak::Pak::global();
        let xml = pak.read_string("worlds/main_world/cells/015_015/client/entities.xml").unwrap();
        let p = parse(&xml);
        let count = |f: fn(&SoundKind) -> bool| p.sounds.iter().filter(|s| f(&s.kind)).count();
        assert_eq!(count(|k| matches!(k, SoundKind::Ambient { .. })), 3);
        assert_eq!(count(|k| matches!(k, SoundKind::Random { .. })), 2);
        assert_eq!(p.reverbs.len(), 1);
        assert_eq!(p.areas.len(), 2);
        // every one of them is listed by an area
        assert!(p.sounds.iter().all(|s| !s.areas.is_empty()), "{:?}", p.sounds);
        // the spawn (218, 22) is inside the pond's area, not the sea's
        let origin = Vec3::new(15.0 * 1024.0, 15.0 * 1024.0, 0.0);
        let spawn = Vec3::new(15578.042, 15382.122, 126.25);
        let pond = p.sounds.iter().find(|s| s.name.ends_with("pond_1")).unwrap();
        let d = AreaWorld::new(&p.areas[pond.areas[0]], origin).closest(spawn, 40.0).unwrap().1;
        assert_eq!(d, 0.0);
        let sea = p.sounds.iter().find(|s| s.name.ends_with("sea06")).unwrap();
        let d = AreaWorld::new(&p.areas[sea.areas[0]], origin).closest(spawn, 100.0).unwrap().1;
        assert!(d > 40.0 && d < 60.0, "{d}");
    }
}
