//! The wider view of a buff's effect group (`FxCamFov`), as 1.2 draws it: Dash (buff 2675, fx group 1034) widens the view by
//! 23 degrees over a second and narrows it again over a second when it ends.
//!
//! What 1.2 does (read in `x2game`):
//!
//! * **The row:** `fx_items` of kind `FxCamFov` name a `fx_cam_fovs` row (`camFov` degrees, `fadeIn` and `fadeOut` seconds; its
//!   `duration` is read and not kept) and, as `asset_name`, a particle effect of the libraries whose two curves are the fades
//!   (`abillity_skill_table_m.system_skill.camera_fov_curve`). `fx::create_cam_fov` `x2game+0x3f7b50`, the item's class
//!   `x2game+0x9e4a98`.
//! * **Only the own unit's:** the item starts when its unit is the own unit or the own vehicle (slot 1 `x2game+0x3f2150`).
//! * **Each frame** (slot 2 `x2game+0x3f21c0`): the field of view is the game's `cl_fov` plus `camFov` times a curve: during the
//!   fade in, the curve of the effect read with flag 1 at `elapsed / fadeIn`; held at full after it; during the fade out the curve
//!   read with flag 0 at `elapsed / fadeOut`; then `cl_fov` again. The curve is `C3DEngine` slot 87 (`cry3dengine+0x12db10`): the
//!   effect's `ParticleParams` at +0x110 (flag 1) or +0x120 (flag 0), sampled from its baked table (`cry3dengine+0x60890`). In
//!   `camera_fov_curve` only `SizeX` and `SizeY` carry curves, so +0x110 is taken as the age curve of `SizeX` and +0x120 as that of
//!   `SizeY` (their order in the struct; not read field by field): in `(0, 0; 0.1875, 0.704; 1, 1)`, out
//!   `(0, 0.75; 0.2946, 0.112; 1, 0)`. The fade out therefore opens at 0.75 of the widening, as 1.2's curve has it.
//! * **The end** (slot 4 `x2game+0x3f2300`): from the fade in, the fade out starts as far along as the fade in had got to go
//!   (`start = now - (1 - min(t, 1)) x fadeOut`); from the hold, at its start.
//!
//! The pulse ([`FovPulse`]) is plain Rust; [`drive_cam_fov`] feeds it the own buffs and puts the degrees on the world camera.

use std::collections::HashMap;
use std::sync::Arc;

use aa_net::state::Buffs;
use bevy::prelude::*;

use super::camera::OrbitCamera;
use super::dev::DevOptions;
use super::player::LocalPlayer;

/// A curve over 0..1 as the particle libraries write one: `(time, value)` keys, linear between them, flat outside (as aa-fx reads
/// every other curve of the libraries).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Curve(pub Vec<(f32, f32)>);

impl Curve {
    pub fn eval(&self, t: f32) -> Option<f32> {
        let first = self.0.first()?;
        if t <= first.0 {
            return Some(first.1);
        }
        for w in self.0.windows(2) {
            let ((a, va), (b, vb)) = (w[0], w[1]);
            if t <= b {
                let f = if b - a > 1e-6 { (t - a) / (b - a) } else { 1.0 };
                return Some(va + (vb - va) * f);
            }
        }
        self.0.last().map(|k| k.1)
    }
}

/// The two fades of a view item: in (flag 1) and out (flag 0).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fades {
    pub fade_in: Curve,
    pub fade_out: Curve,
}

impl Fades {
    /// The share of the widening `t` (0..1) into the fade in.
    fn fade_in(&self, t: f32) -> f32 {
        self.fade_in.eval(t).unwrap_or(t)
    }

    /// The share of the widening `t` (0..1) into the fade out.
    fn fade_out(&self, t: f32) -> f32 {
        self.fade_out.eval(t).unwrap_or(1.0 - t)
    }
}

/// An `FxCamFov` item: `fx_cam_fovs` and the effect that holds its curves.
#[derive(Clone, Debug, PartialEq)]
pub struct FovItem {
    pub degrees: f32,
    pub fade_in: f32,
    pub fade_out: f32,
    /// `library.group.name` of the curves' effect.
    pub curves: Arc<str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Off,
    In,
    Hold,
    Out,
}

/// 1.2's view item in plain Rust: [`FovPulse::start`] (slot 1), [`FovPulse::degrees`] each frame (slot 2), [`FovPulse::end`] (slot 4).
#[derive(Clone, Debug)]
pub struct FovPulse {
    phase: Phase,
    since: f64,
    degrees: f32,
    fade_in: f32,
    fade_out: f32,
}

impl Default for FovPulse {
    fn default() -> Self {
        FovPulse { phase: Phase::Off, since: 0.0, degrees: 0.0, fade_in: 0.0, fade_out: 0.0 }
    }
}

impl FovPulse {
    /// The item starts at `now` (s).
    pub fn start(&mut self, item: &FovItem, now: f64) {
        *self = FovPulse { phase: Phase::In, since: now, degrees: item.degrees, fade_in: item.fade_in, fade_out: item.fade_out };
    }

    /// The item ends at `now`: the view fades back from where it got to.
    pub fn end(&mut self, now: f64) {
        match self.phase {
            Phase::In => {
                let t = if self.fade_in > 0.0 { ((now - self.since) as f32 / self.fade_in).clamp(0.0, 1.0) } else { 1.0 };
                self.since = now - f64::from((1.0 - t) * self.fade_out);
                self.phase = Phase::Out;
            }
            Phase::Hold => {
                self.since = now;
                self.phase = Phase::Out;
            }
            Phase::Off | Phase::Out => {}
        }
    }

    /// The pulse is over (nothing to add).
    pub fn is_off(&self) -> bool {
        self.phase == Phase::Off
    }

    /// The degrees the view gets on top of its own at `now`.
    pub fn degrees(&mut self, now: f64, fades: &Fades) -> f32 {
        let elapsed = (now - self.since) as f32;
        match self.phase {
            Phase::Off => 0.0,
            Phase::In if elapsed >= self.fade_in => {
                self.phase = Phase::Hold;
                self.degrees
            }
            Phase::In => self.degrees * fades.fade_in(elapsed / self.fade_in),
            Phase::Hold => self.degrees,
            Phase::Out if elapsed >= self.fade_out => {
                self.phase = Phase::Off;
                0.0
            }
            Phase::Out => self.degrees * fades.fade_out(elapsed / self.fade_out),
        }
    }
}

/// The view items of the own buffs and the pulse they drive.
#[derive(Resource, Default)]
pub struct CamFov {
    pulse: FovPulse,
    /// The buff (index, id) whose item runs.
    active: Option<(u32, u32)>,
    /// `FxCamFov` items by buff id, read once.
    items: Option<HashMap<u32, FovItem>>,
    /// The fades by curve effect (`None` once the effect is known not to have them).
    fades: HashMap<Arc<str>, Option<Fades>>,
    curves: Option<Arc<str>>,
    /// The degrees last put on the camera.
    written: Option<f32>,
}

impl CamFov {
    /// The camera's base field of view changed (the Options window's camera mode): write the lens again.
    pub fn rewrite(&mut self) {
        self.written = None;
    }
}

/// Every buff whose effect group has an `FxCamFov` item starting with the buff (2675 Dash and the other `질주` groups).
fn load_items() -> HashMap<u32, FovItem> {
    let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
    let mut items = HashMap::new();
    let Ok(conn) = rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI) else {
        warn!("cam fov: no game database, buffs do not widen the view");
        return items;
    };
    let read = conn
        .prepare(
            "SELECT b.id, f.camFov, f.fadeIn, f.fadeOut, i.asset_name FROM buffs b \
             JOIN fx_group_fx_items g ON g.fx_group_id = b.fx_group_id JOIN fx_items i ON i.id = g.fx_item_id \
             JOIN fx_cam_fovs f ON f.id = i.fx_detail_id WHERE i.fx_detail_type = 'FxCamFov' AND i.fx_event_start_id = 0 ORDER BY g.rowid",
        )
        .and_then(|mut st| {
            st.query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)? as u32,
                    FovItem {
                        degrees: r.get::<_, Option<f64>>(1)?.unwrap_or(0.0) as f32,
                        fade_in: r.get::<_, Option<f64>>(2)?.unwrap_or(0.0) as f32,
                        fade_out: r.get::<_, Option<f64>>(3)?.unwrap_or(0.0) as f32,
                        curves: Arc::from(r.get::<_, Option<String>>(4)?.unwrap_or_default().trim()),
                    },
                ))
            })?
            .collect::<Result<Vec<_>, _>>()
        });
    match read {
        Ok(list) => {
            for (id, item) in list {
                items.entry(id).or_insert(item);
            }
        }
        Err(e) => warn!("cam fov: fx_cam_fovs: {e}"),
    }
    items
}

/// The fades of a curve effect: its root's `SizeX` and `SizeY` curves over the age.
pub fn fades_of(def: &aa_fx::EffectDef) -> Option<Fades> {
    let keys = |v: &aa_fx::params::Var| v.age.as_ref().map(|c| Curve(c.keys.clone()));
    let d = &def.root.def;
    Some(Fades { fade_in: keys(&d.size_x)?, fade_out: keys(&d.size_y)? })
}

/// Starts and ends the view item of the own buffs and widens the world camera by it (system).
pub fn drive_cam_fov(
    time: Res<Time>,
    dev: Res<DevOptions>,
    player: Query<&LocalPlayer>,
    buffs: Option<Res<Buffs>>,
    libraries: Option<ResMut<aa_fx::FxLibraries>>,
    mut state: ResMut<CamFov>,
    mut cameras: Query<&mut Projection, With<OrbitCamera>>,
) {
    let now = time.elapsed_secs_f64();
    let state = &mut *state;
    if state.items.is_none() {
        state.items = Some(load_items());
    }
    let items = state.items.as_ref().expect("loaded above");
    // the own unit's first buff with a view item
    let own = player.single().ok().and_then(|l| buffs.as_deref().map(|b| b.of(l.obj_id))).unwrap_or(&[]);
    let wanted = own.iter().find_map(|e| items.get(&e.buff.buff_id).map(|item| ((e.buff.index, e.buff.buff_id), item.clone())));
    let key = wanted.as_ref().map(|w| w.0);
    if key != state.active {
        if state.active.is_some() {
            state.pulse.end(now);
        }
        if let Some((_, item)) = &wanted {
            state.pulse.start(item, now);
            state.curves = Some(item.curves.clone());
        }
        state.active = key;
    }
    if state.pulse.is_off() && state.written == Some(0.0) {
        return;
    }
    // the curves of the item's effect, once its library is in
    let fades = match state.curves.clone() {
        Some(name) => match state.fades.get(&name) {
            Some(f) => f.clone(),
            None => {
                let mut found = None;
                if let (Some(mut libs), Some((lib, effect))) = (libraries, aa_fx::split_name(&name)) {
                    libs.request(lib);
                    match libs.status(lib) {
                        aa_fx::LibStatus::Ready(l) => {
                            let fades = l.get(effect).and_then(|d| fades_of(d));
                            if fades.is_none() {
                                warn!("cam fov: {name} has no fade curves");
                            }
                            state.fades.insert(name.clone(), fades.clone());
                            found = fades;
                        }
                        aa_fx::LibStatus::Failed => {
                            state.fades.insert(name.clone(), None);
                        }
                        aa_fx::LibStatus::Loading => {}
                    }
                }
                found
            }
        },
        None => None,
    }
    .unwrap_or_default();
    let extra = state.pulse.degrees(now, &fades);
    if state.written.is_none_or(|w| (w - extra).abs() > 1e-4) {
        let base = super::camera::base_fov(&dev);
        for mut projection in &mut cameras {
            if let Projection::Perspective(p) = &mut *projection {
                p.fov = (base + extra).to_radians();
            }
        }
        state.written = Some(extra);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1.2's `fx_cam_fovs` 1 and the curves of `camera_fov_curve`.
    fn dash() -> (FovItem, Fades) {
        let item = FovItem { degrees: 23.0, fade_in: 1.0, fade_out: 1.0, curves: Arc::from("abillity_skill_table_m.system_skill.camera_fov_curve") };
        let fades = Fades { fade_in: Curve(vec![(0.0, 0.0), (0.1875, 0.704), (1.0, 1.0)]), fade_out: Curve(vec![(0.0, 0.75), (0.29464287, 0.112), (1.0, 0.0)]) };
        (item, fades)
    }

    #[test]
    fn dash_widens_the_view_by_23_degrees_over_a_second() {
        let (item, fades) = dash();
        let mut p = FovPulse::default();
        assert_eq!(p.degrees(0.0, &fades), 0.0);
        p.start(&item, 10.0);
        assert_eq!(p.degrees(10.0, &fades), 0.0);
        assert!((p.degrees(10.1875, &fades) - 23.0 * 0.704).abs() < 1e-3, "the curve's knee");
        assert!((p.degrees(11.0, &fades) - 23.0).abs() < 1e-4);
        assert_eq!(p.degrees(15.0, &fades), 23.0, "held");
        // the end: the fade out opens at 0.75 of the widening, as 1.2's curve has it, and is over a second later
        p.end(15.0);
        assert!((p.degrees(15.0, &fades) - 23.0 * 0.75).abs() < 1e-4);
        assert!(p.degrees(15.5, &fades) < 2.0);
        assert_eq!(p.degrees(16.0, &fades), 0.0);
        assert!(p.is_off());
    }

    #[test]
    fn an_end_during_the_fade_in_fades_out_from_as_far_along() {
        let (item, fades) = dash();
        let mut p = FovPulse::default();
        p.start(&item, 0.0);
        p.degrees(0.25, &fades);
        // a quarter of the way in: the fade out starts three quarters of the way along (1.2: now - (1 - t) x fadeOut)
        p.end(0.25);
        let at_end = p.degrees(0.25, &fades);
        assert!((at_end - 23.0 * fades.fade_out(0.75)).abs() < 1e-4, "{at_end}");
        assert_eq!(p.degrees(0.5, &fades), 0.0, "over after the rest of the second");
        // an end with nothing running changes nothing
        p.end(1.0);
        assert!(p.is_off());
    }

    #[test]
    fn without_its_curves_the_view_fades_in_a_straight_line() {
        let (item, _) = dash();
        let mut p = FovPulse::default();
        p.start(&item, 0.0);
        assert!((p.degrees(0.5, &Fades::default()) - 11.5).abs() < 1e-4);
    }

    /// The database's Dash row and the library's curve effect are the ones the rule was read from (needs `AA_DB` and `AA_PAK`).
    #[test]
    fn the_data_holds_dash_s_view() {
        let items = load_items();
        let Some(item) = items.get(&super::super::sprint::DASH_BUFF) else {
            eprintln!("no game database: skipped");
            return;
        };
        assert_eq!((item.degrees, item.fade_in, item.fade_out), (23.0, 1.0, 1.0));
        assert_eq!(&*item.curves, "abillity_skill_table_m.system_skill.camera_fov_curve");
        if !aa_cry::pak_available() {
            return;
        }
        let library = aa_fx::Library::load("abillity_skill_table_m").expect("the library");
        let fades = library.get("system_skill.camera_fov_curve").and_then(|d| fades_of(d)).expect("the curves");
        assert_eq!(fades, dash().1);
    }
}
