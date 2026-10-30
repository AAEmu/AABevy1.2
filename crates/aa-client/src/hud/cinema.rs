//! Quest cinemas. A component's `cinema_id` (`quest_components`; 1065's Ready component 5336 is
//! `fly_east`) is played when the step is accepted or handed in: the 1.2 client sends
//! `CSRequestPermissionToPlayCinemaForDirectingMode` (0x0d0) and `CSStartedCinema` (0x0cf), plays
//! the sequence from `worlds/main_world/cinemas/<name>.xml`, then sends `CSCompletedCinema` (0x0ce)
//! and the waiting `CSStartQuestContext` / `CSCompleteQuestContext` (`cinema_flow`). Escape ends
//! 0x0ce without the quest request, as the directing window's close cancels the talk.
//!
//! The sequence itself is the camera work: the Scene node's camera track selects a Camera node and
//! its position/rotation/FOV tracks are sampled here. A missing sequence, or an animation the pak
//! does not ship (the 1.2 `invalid animation ID! name=machinima_fly_east_01`), logs and continues;
//! it never holds the quest.
//!
//! The old `ObjCinema` act (`quest_act_obj_cinemas`, 12 steps of the 1.2 story) is still only
//! tracked: those are Progress steps, not the accept/report cinemas handled here.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use aa_net::interaction::Interact;
use aa_net::quest::{Quest, status};
use aa_net::{Command, Event, NetCommand, NetEvent};
use aa_world::cinema::CameraPose;
use bevy::prelude::*;

use super::cinema_flow;
use super::quests::CinemaDef;

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<CinemaPlayer>()
        .add_systems(OnExit(crate::Screen::InGame), reset)
        .add_systems(
            Update,
            (
                track.run_if(in_state(crate::Screen::InGame)),
                tick_cinema.run_if(in_state(crate::Screen::InGame)),
                skip_cinema.run_if(in_state(crate::Screen::InGame)),
                drive_cinema_camera.after(crate::ingame::camera::follow_player).run_if(in_state(crate::Screen::InGame)),
            ),
        );
}

fn reset(mut player: ResMut<CinemaPlayer>) {
    *player = CinemaPlayer::default();
}

/// `quest_components.id` -> `cinemas.name` of the steps with an `ObjCinema` objective act.
fn load_act_steps() -> HashMap<u32, String> {
    let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
    let query = || -> rusqlite::Result<HashMap<u32, String>> {
        let conn = rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
        let mut q = conn.prepare(
            "SELECT a.quest_component_id, c.name FROM quest_acts a JOIN quest_act_obj_cinemas o ON o.id = a.act_detail_id \
             JOIN cinemas c ON c.id = o.cinema_id WHERE a.act_detail_type = 'QuestActObjCinema'",
        )?;
        q.query_map([], |r| Ok((r.get::<_, i64>(0)? as u32, r.get::<_, String>(1)?)))?.collect()
    };
    query().unwrap_or_else(|e| {
        warn!("hud: quest cinemas from {path}: {e}");
        HashMap::new()
    })
}

#[derive(Default)]
struct Cinemas {
    steps: Option<HashMap<u32, String>>,
    /// (quest instance, step) already reported.
    played: HashSet<(i64, u32)>,
}

impl Cinemas {
    /// The cinema to report for a quest, once per step.
    fn due(&mut self, q: &Quest) -> Option<String> {
        if q.status != status::PROGRESS {
            return None;
        }
        let name = self.steps.get_or_insert_with(load_act_steps).get(&q.component_id)?.clone();
        self.played.insert((q.id, q.component_id)).then_some(name)
    }
}

fn track(mut events: MessageReader<NetEvent>, mut cinemas: Local<Cinemas>) {
    for NetEvent(event) in events.read() {
        let quests: &[Quest] = match event {
            Event::QuestStarted(q) | Event::QuestUpdated(q) => std::slice::from_ref(q),
            Event::QuestsLoaded(list) => list,
            Event::CharacterSelected(_) | Event::Disconnected { .. } => {
                cinemas.played.clear();
                continue;
            }
            _ => continue,
        };
        for q in quests {
            if let Some(name) = cinemas.due(q) {
                warn!("hud: quest {} step {}: cinema {name} requires playback (not reported watched)", q.template_id, q.component_id);
            }
        }
    }
}

/// The camera tracks of one sequence (`worlds/main_world/cinemas/<name>.xml`).
#[derive(Clone, Debug, Default)]
struct Sequence {
    name: String,
    duration: f32,
    cameras: HashMap<String, CameraPath>,
    /// The Scene node's camera-selection keys (`ParamId 6`).
    scene: Vec<(f32, String)>,
}

#[derive(Clone, Debug, Default)]
struct CameraPath {
    position: Vec<(f32, Vec3)>,
    rotation: Vec<(f32, Quat)>,
    fov: Vec<(f32, f32)>,
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

/// The animation ids the pak ships (`animations/**/<name>.caf`); built once, only when a movie plays.
fn shipped_animations() -> &'static HashSet<String> {
    static NAMES: OnceLock<HashSet<String>> = OnceLock::new();
    NAMES.get_or_init(|| {
        if !aa_world::pak_available() {
            return HashSet::new();
        }
        aa_pak::Pak::global()
            .list("animations/")
            .filter_map(|p| p.rsplit('/').next())
            .filter_map(|f| f.rsplit_once('.').map(|(stem, _)| stem.to_ascii_lowercase()))
            .collect()
    })
}

impl Sequence {
    fn load(name: &str) -> Option<Sequence> {
        if !aa_world::pak_available() {
            return None;
        }
        let xml = aa_pak::Pak::global().read_string(&format!("worlds/main_world/cinemas/{name}.xml")).ok()?;
        Self::parse(name, &xml)
    }

    fn parse(name: &str, xml: &str) -> Option<Sequence> {
        let doc = roxmltree::Document::parse(xml).ok()?;
        let root = doc.root_element();
        let duration: f32 = root.attribute("EndTime").and_then(|v| v.parse().ok()).unwrap_or(0.0);
        if !duration.is_finite() || duration <= 0.0 {
            return None;
        }
        let mut sequence = Sequence { name: name.to_string(), duration, ..Sequence::default() };
        let mut animations: Vec<String> = Vec::new();
        for node in root.descendants().filter(|n| n.has_tag_name("Node")) {
            if node.attribute("Type") == Some("3") {
                let mut path = CameraPath::default();
                for track in node.children().filter(|n| n.has_tag_name("Track")) {
                    let id = track.attribute("ParamId").unwrap_or("");
                    for key in track.children().filter(|n| n.has_tag_name("Key")) {
                        let t: f32 = key.attribute("time").and_then(|v| v.parse().ok()).unwrap_or(0.0);
                        match id {
                            "1" => {
                                let v = floats(key.attribute("value").unwrap_or(""));
                                if v.len() == 3 {
                                    path.position.push((t, Vec3::new(v[0], v[1], v[2])));
                                }
                            }
                            "2" => {
                                let v = floats(key.attribute("value").unwrap_or(""));
                                if v.len() == 4 {
                                    path.rotation.push((t, Quat::from_xyzw(v[1], v[2], v[3], v[0]).normalize()));
                                }
                            }
                            "0" => {
                                if let Some(v) = key.attribute("value").and_then(|v| v.parse().ok()) {
                                    path.fov.push((t, v));
                                }
                            }
                            _ => {}
                        }
                    }
                }
                if !path.position.is_empty() {
                    sequence.cameras.insert(node.attribute("Name").unwrap_or("").to_string(), path);
                }
            }
            for track in node.children().filter(|n| n.has_tag_name("Track")) {
                match track.attribute("ParamId") {
                    Some("6") => {
                        for key in track.children().filter(|n| n.has_tag_name("Key")) {
                            let t: f32 = key.attribute("time").and_then(|v| v.parse().ok()).unwrap_or(0.0);
                            if let Some(camera) = key.attribute("node") {
                                sequence.scene.push((t, camera.to_string()));
                            }
                        }
                    }
                    Some("10") => {
                        for key in track.children().filter(|n| n.has_tag_name("Key")) {
                            if let Some(anim) = key.attribute("anim") {
                                animations.push(anim.to_string());
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        // 1.2 logs an animation the actor cannot play and carries on; the movie still runs its time.
        let shipped = shipped_animations();
        let mut warned = HashSet::new();
        for anim in animations {
            if anim.contains('_') && !shipped.contains(&anim.to_ascii_lowercase()) && warned.insert(anim.clone()) {
                warn!("hud: cinema {}: [cut scene error] invalid animation ID! name={anim}", sequence.name);
            }
        }
        Some(sequence)
    }

    fn pose(&self, t: f32) -> Option<CameraPose> {
        let name = self.scene.iter().filter(|(time, _)| *time <= t).max_by(|a, b| a.0.total_cmp(&b.0)).map(|(_, name)| name.as_str()).or_else(|| {
            self.cameras.keys().min().map(String::as_str)
        })?;
        let camera = self.cameras.get(name)?;
        let position = segment(&camera.position, t).map_or(Vec3::ZERO, |(a, b, u)| a.lerp(b, u));
        let rotation = segment(&camera.rotation, t).map_or(Quat::IDENTITY, |(a, b, u)| a.slerp(b, u));
        let fov_deg = segment(&camera.fov, t).map_or(60.0, |(a, b, u)| a + (b - a) * u);
        Some(CameraPose { position, rotation, fov_deg })
    }
}

/// A quest cinema in flight: the flow state and the sequence its camera follows.
#[derive(Resource, Default)]
pub struct CinemaPlayer {
    playing: Option<cinema_flow::Pending>,
    sequence: Option<Sequence>,
}

/// The quest request that waits for a component's cinema.
pub(super) struct StartCinema<'a> {
    pub quest_id: u32,
    pub component_id: u32,
    pub cinema: &'a CinemaDef,
    pub npc_obj_id: u32,
    pub doodad_obj_id: u32,
    pub then: Interact,
}

/// Starts the component's cinema: 0x0d0, 0x0cf and the sequence. `false` when the movie cannot
/// play (missing XML or animation); the caller then sends its quest request immediately.
pub(super) fn start_quest_cinema(player: &mut CinemaPlayer, net: &mut MessageWriter<NetCommand>, request: StartCinema) -> bool {
    let StartCinema { quest_id, component_id, cinema, npc_obj_id, doodad_obj_id, then } = request;
    if player.playing.is_some() {
        warn!("hud: quest {quest_id}: cinema {} already playing; request ignored", cinema.name);
        return true;
    }
    let Some(sequence) = Sequence::load(&cinema.name) else {
        warn!("hud: quest {quest_id} component {component_id}: cinema {} is missing or invalid; continuing", cinema.name);
        return false;
    };
    let Some(start) = cinema_flow::start(quest_id, component_id, &cinema.name, sequence.duration, npc_obj_id, doodad_obj_id, then) else {
        warn!("hud: quest {quest_id}: cinema {} has no duration; continuing", cinema.name);
        return false;
    };
    for packet in start.packets {
        net.write(NetCommand(Command::Interact(packet)));
    }
    info!(
        "hud: quest {quest_id} component {component_id}: cinema {} ({:.1} s{}) started",
        cinema.name,
        sequence.duration,
        if cinema.before_bubble { ", before the lines" } else { "" }
    );
    player.playing = Some(start.pending);
    player.sequence = Some(sequence);
    true
}

fn finish(player: &mut CinemaPlayer, net: &mut MessageWriter<NetCommand>, end: cinema_flow::End, why: &str) {
    let Some(pending) = player.playing.take() else { return };
    for packet in end.packets {
        net.write(NetCommand(Command::Interact(packet)));
    }
    info!("hud: quest {} component {}: cinema {} {why}", pending.quest_id, pending.component_id, pending.name);
    player.sequence = None;
}

fn tick_cinema(time: Res<Time>, mut player: ResMut<CinemaPlayer>, mut net: MessageWriter<NetCommand>) {
    let Some(pending) = player.playing.as_mut() else { return };
    if let Some(end) = cinema_flow::advance(pending, time.delta_secs()) {
        finish(&mut player, &mut net, end, "completed");
    }
}

fn skip_cinema(
    keys: Res<ButtonInput<KeyCode>>,
    mut player: ResMut<CinemaPlayer>,
    mut net: MessageWriter<NetCommand>,
    bridge: Option<NonSend<super::api::HudBridge>>,
) {
    if !keys.just_pressed(KeyCode::Escape) || player.playing.is_none() {
        return;
    }
    let pending = player.playing.as_ref().expect("checked").clone();
    finish(&mut player, &mut net, cinema_flow::skip(&pending), "skipped; the quest request is cancelled");
    if let Some(bridge) = bridge {
        bridge.0.borrow_mut().dialog.requests.push(super::dialog::Req::End);
    }
}

fn drive_cinema_camera(player: Res<CinemaPlayer>, mut cameras: Query<&mut Transform, (With<Camera3d>, Without<aa_ui::render::UiCamera>)>) {
    let (Some(pending), Some(sequence)) = (player.playing.as_ref(), player.sequence.as_ref()) else { return };
    let Some(pose) = sequence.pose(pending.elapsed) else { return };
    let Ok(mut transform) = cameras.single_mut() else { return };
    let forward = aa_world::cry_to_bevy(pose.forward());
    let up = aa_world::cry_to_bevy(pose.rotation * Vec3::Z);
    *transform = Transform::from_translation(aa_world::cry_to_bevy(pose.position)).looking_to(forward, up);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn xml() -> &'static str {
        r#"<Sequence Name="test" StartTime="0" EndTime="4">
            <Nodes>
              <Node Id="0" Type="2" Name="Scene">
                <Track ParamId="6"><Key time="0" node="Camera_1"/></Track>
              </Node>
              <Node Id="1" Type="3" Name="Camera_1">
                <Track ParamId="1"><Key time="0" value="0,0,0"/><Key time="4" value="40,0,0"/></Track>
                <Track ParamId="2"><Key time="0" value="1,0,0,0"/></Track>
                <Track ParamId="0"><Key time="0" value="60"/></Track>
              </Node>
            </Nodes>
          </Sequence>"#
    }

    #[test]
    fn the_sequence_samples_its_scene_camera() {
        let sequence = Sequence::parse("test", xml()).unwrap();
        assert_eq!(sequence.duration, 4.0);
        let start = sequence.pose(0.0).unwrap();
        let end = sequence.pose(4.0).unwrap();
        assert_eq!(start.position, Vec3::ZERO);
        assert_eq!(end.position, Vec3::new(40.0, 0.0, 0.0));
        assert_eq!(start.fov_deg, 60.0);
    }

    #[test]
    fn a_sequence_without_a_duration_or_camera_does_not_play() {
        assert!(Sequence::parse("empty", r#"<Sequence Name="empty" EndTime="0"/>"#).is_none());
        let no_camera = r#"<Sequence Name="x" EndTime="3"><Nodes><Node Id="0" Type="2" Name="Scene"/></Nodes></Sequence>"#;
        let sequence = Sequence::parse("x", no_camera).unwrap();
        assert!(sequence.pose(1.0).is_none());
    }

    #[test]
    fn a_cinema_step_is_reported_once() {
        let mut c = Cinemas { steps: Some(HashMap::from([(26187, "id_300_03".to_string())])), ..Cinemas::default() };
        let q = Quest { id: 9, template_id: 3000, status: status::PROGRESS, component_id: 26187, ..Quest::default() };
        assert_eq!(c.due(&q).as_deref(), Some("id_300_03"));
        assert_eq!(c.due(&q), None);
        // another step, a finished quest: nothing
        assert_eq!(c.due(&Quest { component_id: 1, ..q.clone() }), None);
        assert_eq!(c.due(&Quest { id: 10, status: status::READY, ..q }), None);
    }

    #[test]
    fn receiving_a_cinema_step_does_not_complete_an_unwatched_cinema() {
        let mut app = App::new();
        app.add_message::<NetEvent>().add_message::<aa_net::NetCommand>().add_systems(Update, track);
        app.world_mut().write_message(NetEvent(Event::QuestStarted(Quest {
            id: 9, template_id: 3000, status: status::PROGRESS, component_id: 26187, ..Quest::default()
        })));
        app.update();
        assert!(app.world().resource::<Messages<aa_net::NetCommand>>().is_empty());
    }
}
