//! Quest cinemas. A component's `cinema_id` (`quest_components`; 1065's Ready component 5336 is
//! `fly_east`) is played when the step is accepted or handed in: the 1.2 client sends
//! `CSRequestPermissionToPlayCinemaForDirectingMode` (0x0d0) when the directing dialog opens, and
//! `CSStartedCinema` (0x0cf) when the player confirms; it plays the sequence from
//! `worlds/main_world/cinemas/<name>.xml`, then sends `CSCompletedCinema` (0x0ce) and the waiting
//! `CSStartQuestContext` / `CSCompleteQuestContext` (`cinema_flow`). Escape is ignored during the
//! movie (capture C1-run3); the close/X button was not drivable (C1b). Keyboard and mouse events
//! are dropped at the source while a movie plays, except while chat has the keyboard.
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
use bevy::input::keyboard::KeyboardInput;
use bevy::input::mouse::{MouseButtonInput, MouseWheel};
use bevy::prelude::*;

use super::cinema_flow;
use super::quests::{CinemaDef, kind};

/// Runs after Bevy turned the raw events into `ButtonInput`; every PreUpdate system that reads
/// input belongs to [`CinemaInputRead`], so the gate covers it by construction.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct CinemaInputGate;

/// PreUpdate systems that read keyboard/mouse input.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct CinemaInputRead;

pub(super) fn plugin(app: &mut App) {
    app.configure_sets(
        PreUpdate,
        (CinemaInputGate.after(bevy::input::InputSystems), CinemaInputRead.after(CinemaInputGate)),
    )
    .init_resource::<CinemaPlayer>()
        .add_systems(OnExit(crate::Screen::InGame), reset)
        .add_systems(
            Update,
            (
                track.run_if(in_state(crate::Screen::InGame)),
                tick_cinema.run_if(in_state(crate::Screen::InGame)),
                cinema_ui.after(tick_cinema).run_if(in_state(crate::Screen::InGame)),
                drive_cinema_camera.after(crate::ingame::camera::follow_player).in_set(crate::ingame::CameraFinal).run_if(in_state(crate::Screen::InGame)),
            ),
        )
        .add_systems(PreUpdate, block_cinema_input.in_set(CinemaInputGate));
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
pub(crate) struct Cinemas {
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

fn progress_cinema(bridge: &super::api::HudBridge, q: &Quest) -> Option<CinemaDef> {
    let data = bridge.0.borrow();
    let game = data.game.get()?;
    let part = game.quests.parts.get(&q.component_id)?;
    (part.kind == kind::PROGRESS).then(|| part.cinema.clone()).flatten()
}

pub(crate) fn track(
    mut events: MessageReader<NetEvent>,
    mut cinemas: Local<Cinemas>,
    bridge: Option<NonSend<super::api::HudBridge>>,
    mut player: Option<ResMut<CinemaPlayer>>,
    mut net: MessageWriter<NetCommand>,
) {
    for NetEvent(event) in events.read() {
        if let Event::QuestCompleted { quest_id, .. } = event {
            if let Some(player) = player.as_deref_mut() {
                player.forget_quest(*quest_id);
            }
            continue;
        }
        let (quests, live_update): (&[Quest], bool) = match event {
            Event::QuestUpdated(q) if matches!(q.status, status::DROPPED | status::FAILED) => {
                if let Some(player) = player.as_deref_mut() {
                    player.forget_quest(q.template_id);
                }
                (std::slice::from_ref(q), false)
            }
            Event::QuestUpdated(q) => (std::slice::from_ref(q), true),
            Event::QuestStarted(q) => (std::slice::from_ref(q), false),
            Event::QuestsLoaded(list) => (list, false),
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
            if !live_update {
                continue;
            }
            let Some(bridge) = bridge.as_deref() else { continue };
            let Some(cinema) = progress_cinema(bridge, q) else { continue };
            let Some(player) = player.as_deref_mut() else { continue };
            if player.has(q.template_id, q.component_id) {
                continue;
            }
            start_quest_cinema(
                player,
                &mut net,
                StartCinema { quest_id: q.template_id, component_id: q.component_id, cinema: &cinema, then: None, hide_dialog: false },
            );
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
#[derive(Clone, Debug)]
struct Queued {
    quest_id: u32,
    component_id: u32,
    cinema: String,
    then: Option<Interact>,
    hide_dialog: bool,
}

#[derive(Resource, Default)]
pub struct CinemaPlayer {
    playing: Option<cinema_flow::Pending>,
    sequence: Option<Sequence>,
    restore_fov: Option<f32>,
    hide_dialog: bool,
    played: HashSet<(u32, u32)>,
    queued: std::collections::VecDeque<Queued>,
    held_keys: Vec<KeyCode>,
}

impl CinemaPlayer {
    fn has(&self, quest_id: u32, component_id: u32) -> bool {
        self.played.contains(&(quest_id, component_id))
            || self.queued.iter().any(|q| q.quest_id == quest_id && q.component_id == component_id)
    }

    fn forget_quest(&mut self, quest_id: u32) {
        self.played.retain(|&(quest, _)| quest != quest_id);
        self.queued.retain(|q| q.quest_id != quest_id);
    }

    fn attach(&mut self, quest_id: u32, component_id: u32, then: Option<Interact>) -> bool {
        if let Some(pending) = self.playing.as_mut()
            && pending.quest_id == quest_id
            && pending.component_id == component_id
        {
            if pending.then.is_none() {
                pending.then = then;
            }
            return true;
        }
        false
    }
}

impl CinemaPlayer {
    #[allow(dead_code)]
    pub(crate) fn elapsed(&self) -> Option<f32> {
        self.playing.as_ref().map(|p| p.elapsed)
    }

    pub(crate) fn active(&self) -> bool {
        self.playing.is_some()
    }
}

/// Extra seconds an accept may wait for its movie: the running/queued start-component movie, else
/// the sequence the directing dialog would start.
pub(crate) fn accept_wait_seconds(bridge: &super::api::HudBridge, player: Option<&CinemaPlayer>, quest: u32) -> f32 {
    if let Some(player) = player {
        if let Some(pending) = player.playing.as_ref().filter(|p| p.quest_id == quest) {
            return (pending.duration - pending.elapsed).max(0.0) + 1.0;
        }
        if let Some(queued) = player.queued.iter().find(|q| q.quest_id == quest) {
            return Sequence::load(&queued.cinema).map_or(0.0, |sequence| sequence.duration + 1.0);
        }
    }
    let data = bridge.0.borrow();
    if !data.dialog.directing {
        return 0.0;
    }
    let Some(game) = data.game.get() else { return 0.0 };
    let Some(def) = game.quests.def(quest) else { return 0.0 };
    let Some(part) = def.components.iter().filter_map(|id| game.quests.parts.get(id)).find(|part| part.kind == kind::START && part.cinema.is_some()) else {
        return 0.0;
    };
    if player.is_some_and(|player| player.has(quest, part.id)) {
        return 0.0;
    }
    part.cinema.as_ref().and_then(|cinema| Sequence::load(&cinema.name)).map_or(0.0, |sequence| sequence.duration + 1.0)
}

/// The quest request that waits for a component's cinema.
pub(super) struct StartCinema<'a> {
    pub quest_id: u32,
    pub component_id: u32,
    pub cinema: &'a CinemaDef,
    pub then: Option<Interact>,
    pub hide_dialog: bool,
}

/// Starts the component's cinema: 0x0cf and the sequence. `false` when the movie cannot play
/// (missing XML or animation); the caller then sends its quest request immediately.
pub(super) fn start_quest_cinema(player: &mut CinemaPlayer, net: &mut MessageWriter<NetCommand>, request: StartCinema) -> bool {
    let StartCinema { quest_id, component_id, cinema, then, hide_dialog } = request;
    if player.attach(quest_id, component_id, then.clone()) {
        return true;
    }
    if player.playing.is_some() {
        if let Some(queued) = player.queued.iter_mut().find(|q| q.quest_id == quest_id && q.component_id == component_id) {
            if queued.then.is_none() {
                queued.then = then;
            }
            return true;
        }
        player.queued.push_back(Queued { quest_id, component_id, cinema: cinema.name.clone(), then, hide_dialog });
        return true;
    }
    if player.played.contains(&(quest_id, component_id)) {
        return false;
    }
    let Some(sequence) = Sequence::load(&cinema.name) else {
        warn!("hud: quest {quest_id} component {component_id}: cinema {} is missing or invalid; continuing", cinema.name);
        return false;
    };
    let Some(start) = cinema_flow::start(quest_id, component_id, &cinema.name, sequence.duration, then) else {
        warn!("hud: quest {quest_id}: cinema {} has no duration; continuing", cinema.name);
        return false;
    };
    for packet in start.packets {
        net.write(NetCommand(Command::Interact(packet)));
    }
    info!(
        "hud: quest {quest_id} component {component_id}: cinema {} ({:.1} s) started",
        cinema.name,
        sequence.duration
    );
    player.played.insert((quest_id, component_id));
    player.playing = Some(start.pending);
    player.sequence = Some(sequence);
    player.hide_dialog = hide_dialog;
    true
}

fn start_queued(player: &mut CinemaPlayer, net: &mut MessageWriter<NetCommand>) {
    while let Some(queued) = player.queued.pop_front() {
        let cinema = CinemaDef { name: queued.cinema, before_bubble: false };
        let started = start_quest_cinema(
            player,
            net,
            StartCinema { quest_id: queued.quest_id, component_id: queued.component_id, cinema: &cinema, then: queued.then.clone(), hide_dialog: queued.hide_dialog },
        );
        if started {
            return;
        }
        if let Some(then) = queued.then {
            net.write(NetCommand(Command::Interact(then)));
        }
    }
}

fn finish(player: &mut CinemaPlayer, net: &mut MessageWriter<NetCommand>, end: cinema_flow::End, why: &str) {
    let Some(pending) = player.playing.take() else { return };
    for packet in end.packets {
        net.write(NetCommand(Command::Interact(packet)));
    }
    info!("hud: quest {} component {}: cinema {} {why}", pending.quest_id, pending.component_id, pending.name);
    player.sequence = None;
    start_queued(player, net);
}

pub(crate) fn tick_cinema(time: Res<Time>, mut player: ResMut<CinemaPlayer>, mut net: MessageWriter<NetCommand>) {
    let Some(pending) = player.playing.as_mut() else { return };
    if let Some(end) = cinema_flow::advance(pending, time.delta_secs()) {
        finish(&mut player, &mut net, end, "completed");
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn block_cinema_input(
    mut player: ResMut<CinemaPlayer>,
    focus: Res<super::HudKeyboardFocus>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    mut keyboard: ResMut<Messages<KeyboardInput>>,
    mut buttons: ResMut<Messages<MouseButtonInput>>,
    mut wheel: ResMut<Messages<MouseWheel>>,
    mut actions: ResMut<crate::settings::Actions>,
    mut send: Option<ResMut<crate::ingame::player::SendState>>,
) {
    if !player.active() {
        if !player.held_keys.is_empty() {
            // a key released in this same frame was already processed as unpressed: re-pressing it would stick it
            // down; Escape is not replayed as a fresh press
            let released: Vec<KeyCode> = keyboard
                .iter_current_update_messages()
                .filter(|event| event.state == bevy::input::ButtonState::Released)
                .map(|event| event.key_code)
                .collect();
            for key in player.held_keys.drain(..) {
                if key != KeyCode::Escape && !released.contains(&key) {
                    keys.press(key);
                }
            }
            actions.reread_physical();
        }
        return;
    }
    player.held_keys = keys.get_pressed().copied().collect();
    keys.reset_all();
    mouse.reset_all();
    buttons.clear();
    wheel.clear();
    actions.clear_for_movie();
    if let Some(send) = send.as_deref_mut() {
        send.stop_autorun();
    }
    if !focus.0 {
        keyboard.clear();
    }
}

pub(crate) fn cinema_ui(rt: Option<NonSend<aa_ui::UiRuntime>>, bridge: Option<NonSend<super::api::HudBridge>>, mut player: ResMut<CinemaPlayer>) {
    let Some(bridge) = bridge else { return };
    let mut data = bridge.0.borrow_mut();
    if player.active() && player.hide_dialog {
        super::dialog::cinema_hide(rt.as_deref(), &mut data.dialog, true);
    } else {
        super::dialog::cinema_hide(rt.as_deref(), &mut data.dialog, false);
        player.hide_dialog = false;
    }
}

#[allow(clippy::type_complexity)]
pub(crate) fn drive_cinema_camera(
    mut player: ResMut<CinemaPlayer>,
    mut cameras: Query<(&mut Transform, &mut Projection), (With<Camera3d>, With<crate::ingame::OrbitCamera>, Without<aa_ui::render::UiCamera>)>,
) {
    let pose = player.playing.as_ref().and_then(|pending| player.sequence.as_ref().and_then(|sequence| sequence.pose(pending.elapsed)));
    let Some(pose) = pose else {
        let Some(fov) = player.restore_fov.take() else { return };
        let Ok((_, mut projection)) = cameras.single_mut() else { return };
        if let Projection::Perspective(perspective) = &mut *projection {
            perspective.fov = fov;
        }
        return;
    };
    let Ok((mut transform, mut projection)) = cameras.single_mut() else { return };
    if let Projection::Perspective(perspective) = &mut *projection {
        player.restore_fov.get_or_insert(perspective.fov);
        perspective.fov = pose.fov_deg.to_radians();
    }
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
    fn the_camera_fov_track_is_applied_and_restored() {
        let xml = r#"<Sequence Name="fov" EndTime="4"><Nodes>
            <Node Id="0" Type="2" Name="Scene"><Track ParamId="6"><Key time="0" node="Camera_1"/></Track></Node>
            <Node Id="1" Type="3" Name="Camera_1">
              <Track ParamId="1"><Key time="0" value="0,0,0"/></Track>
              <Track ParamId="0"><Key time="0" value="60"/><Key time="4" value="90"/></Track>
            </Node></Nodes></Sequence>"#;
        let sequence = Sequence::parse("fov", xml).unwrap();
        let start = cinema_flow::start(1, 2, "fov", 4.0, Some(Interact::Cinema { completed: true })).unwrap();
        let mut player = CinemaPlayer { playing: Some(start.pending), sequence: Some(sequence), restore_fov: None, hide_dialog: false, played: Default::default(), queued: Default::default(), held_keys: Default::default() };
        player.playing.as_mut().unwrap().elapsed = 2.0;
        let mut app = App::new();
        app.insert_resource(player)
            .add_systems(Update, drive_cinema_camera);
        let camera = app.world_mut().spawn((
            Camera3d::default(),
            crate::ingame::OrbitCamera::default(),
            Projection::Perspective(PerspectiveProjection { fov: 1.0, ..default() }),
            Transform::default(),
        )).id();
        app.update();
        let fov = match app.world().get::<Projection>(camera).unwrap() {
            Projection::Perspective(p) => p.fov,
            _ => panic!("perspective"),
        };
        assert!((fov - 75f32.to_radians()).abs() < 1e-4, "{fov}");
        app.world_mut().resource_mut::<CinemaPlayer>().playing = None;
        app.update();
        let fov = match app.world().get::<Projection>(camera).unwrap() {
            Projection::Perspective(p) => p.fov,
            _ => panic!("perspective"),
        };
        assert_eq!(fov, 1.0);
    }

    #[test]
    fn escape_and_movement_are_consumed_while_a_movie_plays() {
        let start = cinema_flow::start(1, 2, "fov", 4.0, Some(Interact::Cinema { completed: true })).unwrap();
        let mut app = App::new();
        app.insert_resource(ButtonInput::<KeyCode>::default())
            .insert_resource(ButtonInput::<MouseButton>::default())
            .insert_resource(super::super::HudKeyboardFocus(false))
            .insert_resource(crate::settings::Actions::default())
            .init_resource::<crate::ingame::player::SendState>()
            .insert_resource(CinemaPlayer { playing: Some(start.pending.clone()), sequence: None, restore_fov: None, hide_dialog: false, played: Default::default(), queued: Default::default(), held_keys: Default::default() })
            .add_message::<KeyboardInput>()
            .add_message::<MouseButtonInput>()
            .add_message::<MouseWheel>()
            .add_systems(PreUpdate, block_cinema_input);
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::Escape);
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::KeyW);
        app.world_mut().resource_mut::<ButtonInput<MouseButton>>().press(MouseButton::Left);
        app.update();
        assert!(!app.world().resource::<ButtonInput<KeyCode>>().pressed(KeyCode::Escape));
        assert!(!app.world().resource::<ButtonInput<KeyCode>>().pressed(KeyCode::KeyW));
        assert!(!app.world().resource::<ButtonInput<MouseButton>>().pressed(MouseButton::Left));
        let _ = start.pending;
    }

    #[derive(Default, Resource)]
    struct StartResult {
        missing: bool,
        first: bool,
        second: bool,
    }

    fn try_start(mut player: ResMut<CinemaPlayer>, mut net: MessageWriter<NetCommand>, mut result: ResMut<StartResult>) {
        let missing = CinemaDef { name: "missing_cinema_xyz".into(), before_bubble: false };
        result.missing = start_quest_cinema(&mut player, &mut net, StartCinema { quest_id: 1, component_id: 2, cinema: &missing, then: None, hide_dialog: false });
        *player = CinemaPlayer::default();
        let first = CinemaDef { name: "fly_east".into(), before_bubble: false };
        let second = CinemaDef { name: "zq_09_helmire".into(), before_bubble: false };
        result.first = start_quest_cinema(&mut player, &mut net, StartCinema { quest_id: 1065, component_id: 5336, cinema: &first, then: None, hide_dialog: false });
        result.second = start_quest_cinema(&mut player, &mut net, StartCinema { quest_id: 1352, component_id: 6758, cinema: &second, then: None, hide_dialog: false });
    }

    #[test]
    fn a_missing_sequence_and_a_queued_request_are_not_marked_played() {
        if !aa_world::pak_available() {
            return;
        }
        let mut app = App::new();
        app.add_message::<NetCommand>()
            .init_resource::<CinemaPlayer>()
            .init_resource::<StartResult>()
            .add_systems(Update, try_start);
        app.update();
        let result = app.world().resource::<StartResult>();
        assert!(!result.missing);
        assert!(!app.world().resource::<CinemaPlayer>().has(1, 2));
        assert!(result.first && result.second);
        let player = app.world().resource::<CinemaPlayer>();
        assert!(player.has(1352, 6758), "the queued request must not be dropped");
        assert!(!player.played.contains(&(1352, 6758)), "a queued movie has not played yet");
    }

    #[test]
    fn a_movie_clears_saved_actions_and_does_not_restore_a_released_key() {
        let start = cinema_flow::start(1, 2, "fov", 4.0, None).unwrap();
        let mut app = App::new();
        app.insert_resource(ButtonInput::<KeyCode>::default())
            .insert_resource(ButtonInput::<MouseButton>::default())
            .insert_resource(super::super::HudKeyboardFocus(false))
            .insert_resource(crate::settings::Actions::default())
            .init_resource::<crate::ingame::player::SendState>()
            .insert_resource(CinemaPlayer { playing: Some(start.pending), sequence: None, restore_fov: None, hide_dialog: false, played: Default::default(), queued: Default::default(), held_keys: Default::default() })
            .add_message::<KeyboardInput>()
            .add_message::<MouseButtonInput>()
            .add_message::<MouseWheel>()
            .add_systems(PreUpdate, block_cinema_input);
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::KeyW);
        app.world_mut().resource_mut::<crate::settings::Actions>().hold_for_test("moveforward");
        app.update();
        assert!(!app.world().resource::<crate::settings::Actions>().pressed("moveforward", 0));
        assert!(!app.world().resource::<ButtonInput<KeyCode>>().pressed(KeyCode::KeyW));
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().release(KeyCode::KeyW);
        app.update();
        app.world_mut().resource_mut::<CinemaPlayer>().playing = None;
        app.update();
        assert!(!app.world().resource::<crate::settings::Actions>().pressed("moveforward", 0));
        assert!(!app.world().resource::<ButtonInput<KeyCode>>().pressed(KeyCode::KeyW));
    }

    #[test]
    fn a_held_key_comes_back_after_the_movie() {
        let start = cinema_flow::start(1, 2, "fov", 4.0, None).unwrap();
        let mut app = App::new();
        app.insert_resource(ButtonInput::<KeyCode>::default())
            .insert_resource(ButtonInput::<MouseButton>::default())
            .insert_resource(super::super::HudKeyboardFocus(false))
            .insert_resource(crate::settings::Actions::default())
            .init_resource::<crate::ingame::player::SendState>()
            .insert_resource(CinemaPlayer { playing: Some(start.pending), sequence: None, restore_fov: None, hide_dialog: false, played: Default::default(), queued: Default::default(), held_keys: Default::default() })
            .add_message::<KeyboardInput>()
            .add_message::<MouseButtonInput>()
            .add_message::<MouseWheel>()
            .add_systems(PreUpdate, block_cinema_input);
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::KeyW);
        app.update();
        app.world_mut().resource_mut::<CinemaPlayer>().playing = None;
        app.update();
        assert!(app.world().resource::<ButtonInput<KeyCode>>().pressed(KeyCode::KeyW));
    }

    #[test]
    fn the_movie_camera_ignores_a_preview_camera() {
        let xml = r#"<Sequence Name="cam" EndTime="4"><Nodes><Node Id="0" Type="2" Name="Scene"><Track ParamId="6"><Key time="0" node="Camera_1"/></Track></Node><Node Id="1" Type="3" Name="Camera_1"><Track ParamId="1"><Key time="0" value="0,0,0"/><Key time="4" value="40,0,0"/></Track></Node></Nodes></Sequence>"#;
        let sequence = Sequence::parse("cam", xml).unwrap();
        let start = cinema_flow::start(1, 2, "cam", 4.0, None).unwrap();
        let mut player = CinemaPlayer { playing: Some(start.pending), sequence: Some(sequence), restore_fov: None, hide_dialog: false, played: Default::default(), queued: Default::default(), held_keys: Default::default() };
        player.playing.as_mut().unwrap().elapsed = 2.0;
        let mut app = App::new();
        app.insert_resource(player).add_systems(Update, drive_cinema_camera);
        let world = app.world_mut().spawn((Camera3d::default(), crate::ingame::OrbitCamera::default(), Projection::Perspective(PerspectiveProjection::default()), Transform::default())).id();
        let preview = app.world_mut().spawn((Camera3d::default(), Projection::Perspective(PerspectiveProjection::default()), Transform::from_xyz(7.0, 8.0, 9.0))).id();
        app.update();
        assert_ne!(app.world().get::<Transform>(world).unwrap().translation, Vec3::ZERO);
        assert_eq!(app.world().get::<Transform>(preview).unwrap().translation, Vec3::new(7.0, 8.0, 9.0));
    }

    #[test]
    fn a_queued_before_bubble_accept_keeps_its_hand_in() {
        if !aa_world::pak_available() {
            return;
        }
        let mut app = App::new();
        app.add_message::<NetCommand>().init_resource::<CinemaPlayer>().init_resource::<QueuedThen>();
        app.add_systems(Update, queue_then);
        app.update();
        assert!(app.world().resource::<QueuedThen>().kept);
    }

    #[derive(Default, Resource)]
    struct QueuedThen {
        kept: bool,
    }

    fn queue_then(mut player: ResMut<CinemaPlayer>, mut net: MessageWriter<NetCommand>, mut out: ResMut<QueuedThen>) {
        let first = CinemaDef { name: "fly_east".into(), before_bubble: false };
        let second = CinemaDef { name: "zq_09_helmire".into(), before_bubble: true };
        start_quest_cinema(&mut player, &mut net, StartCinema { quest_id: 1065, component_id: 5336, cinema: &first, then: None, hide_dialog: false });
        start_quest_cinema(&mut player, &mut net, StartCinema { quest_id: 1352, component_id: 6758, cinema: &second, then: None, hide_dialog: false });
        let then = Interact::QuestStart { quest_id: 1352, npc_obj_id: 0, doodad_obj_id: 0, sphere_id: 0 };
        start_quest_cinema(&mut player, &mut net, StartCinema { quest_id: 1352, component_id: 6758, cinema: &second, then: Some(then.clone()), hide_dialog: false });
        out.kept = player.queued.front().and_then(|q| q.then.as_ref()) == Some(&then);
    }

    #[test]
    fn a_dropped_quest_forgets_its_played_movie() {
        let mut player = CinemaPlayer::default();
        player.played.insert((1352, 6758));
        player.queued.push_back(Queued { quest_id: 1352, component_id: 6758, cinema: "zq_09_helmire".into(), then: None, hide_dialog: false });
        player.forget_quest(1352);
        assert!(!player.has(1352, 6758));
    }

    #[test]
    fn an_accept_waits_for_a_before_bubble_movie() {
        if !aa_world::pak_available() {
            return;
        }
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        let Ok(game) = crate::hud::GameData::load(&path) else { return };
        let shared = std::rc::Rc::new(std::cell::RefCell::new(crate::hud::api::HudData::default()));
        {
            let mut data = shared.borrow_mut();
            data.game = crate::hud::SharedGameData::default();
            let _ = data.game.0.set(game);
            data.dialog.directing = true;
        }
        let bridge = crate::hud::api::HudBridge(shared);
        assert!(accept_wait_seconds(&bridge, None, 1352) >= 13.0);
    }

    #[test]
    fn the_movie_camera_wins_after_the_lens() {
        let xml = r#"<Sequence Name="cam" EndTime="4"><Nodes><Node Id="0" Type="2" Name="Scene"><Track ParamId="6"><Key time="0" node="Camera_1"/></Track></Node><Node Id="1" Type="3" Name="Camera_1"><Track ParamId="1"><Key time="0" value="0,0,0"/></Track><Track ParamId="0"><Key time="0" value="90"/></Track></Node></Nodes></Sequence>"#;
        let sequence = Sequence::parse("cam", xml).unwrap();
        let start = cinema_flow::start(1, 2, "cam", 4.0, None).unwrap();
        let player = CinemaPlayer { playing: Some(start.pending), sequence: Some(sequence), restore_fov: None, hide_dialog: false, played: Default::default(), queued: Default::default(), held_keys: Default::default() };
        let mut app = App::new();
        app.insert_resource(player)
            .configure_sets(Update, crate::ingame::CameraFinal.after(lens))
            .add_systems(Update, lens)
            .add_systems(Update, drive_cinema_camera.in_set(crate::ingame::CameraFinal));
        let camera = app.world_mut().spawn((Camera3d::default(), crate::ingame::OrbitCamera::default(), Projection::Perspective(PerspectiveProjection { fov: 0.5, ..default() }), Transform::default())).id();
        app.update();
        let fov = match app.world().get::<Projection>(camera).unwrap() {
            Projection::Perspective(p) => p.fov,
            _ => panic!("perspective"),
        };
        assert!((fov - 90f32.to_radians()).abs() < 1e-4, "{fov}");
    }

    fn lens(mut cameras: Query<&mut Projection, With<Camera3d>>) {
        for mut projection in &mut cameras {
            if let Projection::Perspective(perspective) = &mut *projection {
                perspective.fov = 0.5;
            }
        }
    }

    #[test]
    fn a_progress_component_update_starts_its_cinema_without_permission() {
        if !aa_world::pak_available() {
            return;
        }
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        let Ok(game) = super::super::GameData::load(&path) else { return };
        let shared = std::rc::Rc::new(std::cell::RefCell::new(super::super::api::HudData::default()));
        {
            let mut data = shared.borrow_mut();
            data.game = super::super::SharedGameData::default();
            let _ = data.game.0.set(game);
        }
        let mut app = App::new();
        app.add_message::<NetEvent>()
            .add_message::<NetCommand>()
            .init_resource::<CinemaPlayer>()
            .insert_non_send(super::super::api::HudBridge(shared))
            .add_systems(Update, track);
        let quest = Quest { id: 1, template_id: 1061, status: status::PROGRESS, component_id: 5320, ..Quest::default() };
        app.world_mut().write_message(NetEvent(Event::QuestsLoaded(vec![quest.clone()])));
        app.update();
        let packets: Vec<Interact> = app
            .world_mut()
            .resource_mut::<Messages<NetCommand>>()
            .drain()
            .filter_map(|NetCommand(command)| if let Command::Interact(packet) = command { Some(packet) } else { None })
            .collect();
        assert!(packets.is_empty(), "a relog QuestsLoaded must not replay the progress cinema");
        app.world_mut().write_message(NetEvent(Event::QuestUpdated(quest)));
        app.update();
        let packets: Vec<Interact> = app
            .world_mut()
            .resource_mut::<Messages<NetCommand>>()
            .drain()
            .filter_map(|NetCommand(command)| if let Command::Interact(packet) = command { Some(packet) } else { None })
            .collect();
        assert_eq!(packets, [Interact::Cinema { completed: false }]);
    }

    #[test]
    fn chat_focus_keeps_only_keyboard_text_events() {
        let start = cinema_flow::start(1, 2, "fov", 4.0, Some(Interact::Cinema { completed: true })).unwrap();
        let mut app = App::new();
        app.insert_resource(ButtonInput::<KeyCode>::default())
            .insert_resource(ButtonInput::<MouseButton>::default())
            .insert_resource(super::super::HudKeyboardFocus(true))
            .insert_resource(crate::settings::Actions::default())
            .init_resource::<crate::ingame::player::SendState>()
            .insert_resource(CinemaPlayer { playing: Some(start.pending), sequence: None, restore_fov: None, hide_dialog: false, played: Default::default(), queued: Default::default(), held_keys: Default::default() })
            .add_message::<KeyboardInput>()
            .add_message::<MouseButtonInput>()
            .add_message::<MouseWheel>()
            .add_systems(PreUpdate, block_cinema_input);
        app.world_mut().resource_mut::<Messages<KeyboardInput>>().write(KeyboardInput { key_code: KeyCode::KeyA, logical_key: bevy::input::keyboard::Key::Character("a".into()), state: bevy::input::ButtonState::Pressed, text: Some("a".into()), repeat: false, window: Entity::PLACEHOLDER });
        app.world_mut().resource_mut::<Messages<MouseButtonInput>>().write(MouseButtonInput { button: MouseButton::Left, state: bevy::input::ButtonState::Pressed, window: Entity::PLACEHOLDER });
        app.update();
        assert_eq!(app.world().resource::<Messages<KeyboardInput>>().len(), 1, "chat typing must still reach the edit box");
        assert_eq!(app.world().resource::<Messages<MouseButtonInput>>().len(), 0, "mouse input must stay gated");
    }

    #[derive(Default, Resource)]
    struct QueueResult {
        started: bool,
    }

    fn run_queue(mut player: ResMut<CinemaPlayer>, mut net: MessageWriter<NetCommand>, mut result: ResMut<QueueResult>) {
        player.queued.push_back(Queued { quest_id: 1, component_id: 2, cinema: "missing_cinema_xyz".into(), then: Some(Interact::QuestComplete { quest_id: 1, npc_obj_id: 0, doodad_obj_id: 0, selected: 0 }), hide_dialog: false });
        player.queued.push_back(Queued { quest_id: 1065, component_id: 5336, cinema: "fly_east".into(), then: None, hide_dialog: false });
        start_queued(&mut player, &mut net);
        result.started = player.active();
    }

    #[test]
    fn a_queued_movie_that_cannot_start_sends_its_hand_in_and_tries_the_next() {
        if !aa_world::pak_available() {
            return;
        }
        let mut app = App::new();
        app.add_message::<NetCommand>()
            .init_resource::<CinemaPlayer>()
            .init_resource::<QueueResult>()
            .add_systems(Update, run_queue);
        app.update();
        let result = app.world().resource::<QueueResult>();
        assert!(result.started, "the second queued movie must start");
    }

    #[test]
    fn a_falling_player_lands_and_reports_zero_velocity_while_the_movie_plays() {
        use crate::ingame::player::{self, LocalPlayer, Mover};
        if !aa_world::pak_available() {
            return;
        }
        let terrain = aa_world::TerrainQuery::new("main_world");
        let ground = terrain.height_at(22298.5, 8658.5).unwrap_or(615.95);
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<CinemaPlayer>()
            .insert_resource(ButtonInput::<KeyCode>::default())
            .insert_resource(ButtonInput::<MouseButton>::default())
            .insert_resource(crate::settings::Actions::default())
            .insert_resource(crate::ingame::DevOptions::default())
            .init_resource::<player::SendState>()
            .init_resource::<player::PlayerInput>()
            .init_resource::<crate::ingame::LocalJump>()
            .insert_resource(aa_world::WorldCollision::new("main_world", terrain.clone()))
            .insert_resource(aa_world::WaterQuery::with_terrain("main_world", terrain.clone()))
            .insert_resource(terrain)
            .add_message::<NetCommand>()
            .add_systems(Update, player::control_player);
        let start = cinema_flow::start(1, 2, "fov", 60.0, None).unwrap();
        app.world_mut().resource_mut::<CinemaPlayer>().playing = Some(start.pending);
        app.world_mut().spawn(crate::ingame::camera::OrbitCamera { yaw: 0.0, ..default() });
        app.world_mut().resource_mut::<player::SendState>().settled = true;
        let actor = app
            .world_mut()
            .spawn((
                LocalPlayer { obj_id: 47 },
                Mover { pos: Vec3::new(22298.5, 8658.5, ground + 5.0), facing: 0.0, vz: -10.0, grounded: false, velocity: Vec3::ZERO, swim: None, wading: 0.0 },
                Transform::default(),
                crate::ingame::Motion::default(),
                crate::ingame::ActorParams::default(),
            ))
            .id();
        let mut moves = Vec::new();
        for _ in 0..90 {
            std::thread::sleep(std::time::Duration::from_millis(20));
            app.update();
            moves.extend(app.world_mut().resource_mut::<Messages<NetCommand>>().drain().filter_map(|NetCommand(command)| if let Command::Move(movement) = command { Some(movement) } else { None }));
        }
        let mover = app.world().get::<Mover>(actor).unwrap();
        assert!(mover.grounded && mover.pos.z <= ground + 5.0, "the fall froze at {:?}", mover.pos);
        let stopped = moves.iter().find(|m| !m.moving && m.state == aa_net::MoveState::Ground).expect("a landing/stop report");
        assert_eq!(stopped.velocity, [0, 0, 0], "the landing report must be velocity zero");
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
