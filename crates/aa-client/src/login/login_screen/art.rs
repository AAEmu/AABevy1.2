//! The animated backdrop of the login screen, driven by `assets/login/manifest.json`
//! (its contract is `assets/login/README.md`).
//!
//! Everything lives on a 2560x1440 design canvas that is scaled uniformly to cover the window ([`Canvas`]).
//! Layers follow the mouse by their `parallax`, drift (`tile_x` layers wrap), and breathe in scale / alpha; the
//! motes are seeded fireflies that rise, sway and fade. Layers are plain `ImageNode`s whose `Node` is set every
//! frame, so a window resize needs no rebuild. Without the files (or the manifest) nothing of this exists: the
//! screen keeps its dark teal gradient.

use std::f32::consts::TAU;
use std::path::{Path, PathBuf};

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use serde_json::Value;

/// The asset source name of the art folder (`login://bg_far.png`).
pub const SOURCE: &str = "login";

/// The folder with the art: next to the exe (`assets/login`), else in the source tree.
pub fn assets_dir() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(dir) = std::env::var_os("AA_LOGIN_ASSETS") {
        candidates.push(dir.into());
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("assets/login"))) {
        candidates.push(dir);
    }
    candidates.push(PathBuf::from("crates/aa-client/assets/login"));
    candidates.push(PathBuf::from("assets/login"));
    candidates.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/login"));
    candidates
        .into_iter()
        .find(|d| d.join("manifest.json").is_file() || d.join("bg_far.png").is_file())
        // absolute: the asset source joins relative paths to its own base
        .map(|d| std::path::absolute(&d).unwrap_or(d))
}

/// The design canvas fitted to the window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Canvas {
    /// Window pixels per canvas pixel.
    pub s: f32,
    /// Window position of the canvas' top-left corner (negative: the crop).
    pub origin: Vec2,
    pub window: Vec2,
}

pub const CANVAS: Vec2 = Vec2::new(2560.0, 1440.0);

impl Canvas {
    pub fn fit(window: Vec2) -> Canvas {
        let s = (window.x / CANVAS.x).max(window.y / CANVAS.y);
        Canvas { s, origin: (window - CANVAS * s) / 2.0, window }
    }

    pub fn to_window(&self, p: Vec2) -> Vec2 {
        self.origin + p * self.s
    }
}

pub struct LayerDef {
    pub file: String,
    pub position: Vec2,
    pub parallax: f32,
    pub drift: Vec2,
    pub alpha: f32,
    pub scale: f32,
    pub tile_x: bool,
    pub scale_amp: f32,
    pub alpha_amp: f32,
    pub period: f32,
    pub phase: f32,
}

pub struct MoteDef {
    pub file: String,
    pub count: usize,
    pub seed: u64,
    /// left, top, right, bottom
    pub area: [f32; 4],
    pub rise: (f32, f32),
    pub sway_amp: (f32, f32),
    pub sway_period: (f32, f32),
    pub lifetime: (f32, f32),
    pub fade_in: f32,
    pub fade_out: f32,
    pub size: (f32, f32),
    pub tint: [f32; 4],
}

pub struct PanelDef {
    pub file: String,
    /// Canvas centre of the panel and its size.
    pub centre: Vec2,
    pub size: Vec2,
    /// left, top, right, bottom
    pub inset: [f32; 4],
}

impl PanelDef {
    /// The panel without art: same place and size, drawn as a plain colour.
    pub fn plain() -> PanelDef {
        PanelDef { file: String::new(), centre: Vec2::new(1720.0, 700.0), size: Vec2::new(720.0, 560.0), inset: [85.0, 75.0, 85.0, 70.0] }
    }
}

pub struct Manifest {
    pub mouse_clamp: Vec2,
    pub layers: Vec<LayerDef>,
    pub motes: Option<MoteDef>,
    pub panel: PanelDef,
}

fn num(v: &Value, key: &str, default: f32) -> f32 {
    v.get(key).and_then(Value::as_f64).map_or(default, |n| n as f32)
}

fn nums<const N: usize>(v: &Value, key: &str) -> Option<[f32; N]> {
    let a = v.get(key)?.as_array()?;
    let mut out = [0.0; N];
    if a.len() < N {
        return None;
    }
    for (o, n) in out.iter_mut().zip(a) {
        *o = n.as_f64()? as f32;
    }
    Some(out)
}

fn range(v: &Value, key: &str, default: (f32, f32)) -> (f32, f32) {
    nums::<2>(v, key).map_or(default, |[a, b]| (a.min(b), a.max(b)))
}

impl Manifest {
    pub fn load(dir: &Path) -> Option<Manifest> {
        let text = std::fs::read_to_string(dir.join("manifest.json")).ok()?;
        let root: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(e) => {
                warn!("login screen: manifest.json: {e}");
                return None;
            }
        };
        let mut layers: Vec<(f32, LayerDef)> = root
            .get("layers")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|l| {
                let breathing = l.get("breathing").cloned().unwrap_or(Value::Null);
                let def = LayerDef {
                    file: l.get("file")?.as_str()?.to_string(),
                    position: nums::<2>(l, "position_px").map_or(CANVAS / 2.0, |[x, y]| Vec2::new(x, y)),
                    parallax: num(l, "parallax", 0.0),
                    drift: nums::<2>(l, "drift_px_s").map_or(Vec2::ZERO, |[x, y]| Vec2::new(x, y)),
                    alpha: num(l, "alpha", 1.0),
                    scale: num(l, "scale", 1.0),
                    tile_x: l.get("tile_x").and_then(Value::as_bool).unwrap_or(false),
                    scale_amp: num(&breathing, "scale_amplitude", 0.0),
                    alpha_amp: num(&breathing, "alpha_amplitude", 0.0),
                    period: num(&breathing, "period_s", 20.0).max(0.1),
                    phase: num(&breathing, "phase_rad", 0.0),
                };
                Some((num(l, "z", 0.0), def))
            })
            .collect();
        layers.sort_by(|a, b| a.0.total_cmp(&b.0));
        let motes = root.get("motes").and_then(|m| {
            Some(MoteDef {
                file: m.get("file")?.as_str()?.to_string(),
                count: m.get("count").and_then(Value::as_u64).unwrap_or(0) as usize,
                seed: m.get("seed").and_then(Value::as_u64).unwrap_or(1),
                area: nums::<4>(m, "spawn_area_px").unwrap_or([0.0, 0.0, CANVAS.x, CANVAS.y]),
                rise: range(m, "rise_speed_px_s", (10.0, 20.0)),
                sway_amp: range(m, "sway_amplitude_px", (10.0, 20.0)),
                sway_period: range(m, "sway_period_s", (5.0, 9.0)),
                lifetime: range(m, "lifetime_s", (18.0, 30.0)),
                fade_in: num(m, "fade_in_s", 2.0),
                fade_out: num(m, "fade_out_s", 3.0),
                size: range(m, "size_px", (8.0, 16.0)),
                tint: nums::<4>(m, "tint_rgba").unwrap_or([1.0; 4]),
            })
        });
        let p = root.get("panel")?;
        let offset = nums::<2>(p, "offset_px").map_or(Vec2::ZERO, |[x, y]| Vec2::new(x, y));
        let panel = PanelDef {
            file: p.get("file")?.as_str()?.to_string(),
            centre: CANVAS / 2.0 + offset,
            size: nums::<2>(p, "size_px").map_or(Vec2::new(720.0, 560.0), |[x, y]| Vec2::new(x, y)),
            inset: nums::<4>(p, "content_inset_px").unwrap_or([85.0, 75.0, 85.0, 70.0]),
        };
        Some(Manifest {
            mouse_clamp: nums::<2>(&root, "mouse_clamp_px").map_or(Vec2::new(1280.0, 720.0), |[x, y]| Vec2::new(x, y)),
            layers: layers.into_iter().map(|(_, l)| l).collect(),
            motes,
            panel,
        })
    }
}

/// Which manifest layer a node shows (and which repeat of a `tile_x` layer).
#[derive(Component)]
pub struct ArtLayer {
    index: usize,
    copy: usize,
    image: Handle<Image>,
}

#[derive(Component)]
pub struct Mote(usize);

struct MoteState {
    x: f32,
    y: f32,
    speed: f32,
    amp: f32,
    period: f32,
    phase: f32,
    life: f32,
    age: f32,
    size: f32,
}

/// What moves: the manifest, the smoothed mouse offset, the motes.
#[derive(Resource)]
pub struct Art {
    pub manifest: Manifest,
    time: f32,
    mouse: Vec2,
    motes: Vec<MoteState>,
    rng: u64,
}

fn rand(rng: &mut u64) -> f32 {
    let mut x = *rng | 1;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *rng = x;
    (x >> 40) as f32 / (1u64 << 24) as f32
}

fn between(rng: &mut u64, (a, b): (f32, f32)) -> f32 {
    a + (b - a) * rand(rng)
}

fn new_mote(def: &MoteDef, rng: &mut u64, prewarm: bool) -> MoteState {
    let life = between(rng, def.lifetime).max(0.5);
    MoteState {
        x: between(rng, (def.area[0], def.area[2])),
        y: between(rng, (def.area[1], def.area[3])),
        speed: between(rng, def.rise),
        amp: between(rng, def.sway_amp),
        period: between(rng, def.sway_period).max(0.5),
        phase: rand(rng) * TAU,
        life,
        age: if prewarm { rand(rng) * life } else { 0.0 },
        size: between(rng, def.size),
    }
}

impl Art {
    pub fn new(manifest: Manifest) -> Art {
        let mut rng = manifest.motes.as_ref().map_or(1, |m| m.seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let motes = manifest.motes.as_ref().map(|d| (0..d.count).map(|_| new_mote(d, &mut rng, true)).collect()).unwrap_or_default();
        Art { manifest, time: 0.0, mouse: Vec2::ZERO, motes, rng }
    }
}

/// The manifest's `size_px` is the glowing core; the sprite around it is this much wider.
const MOTE_SCALE: f32 = 3.0;

/// Copies of a `tile_x` layer (enough for the widest window at the smallest scale).
const TILE_COPIES: usize = 6;

fn bg_node() -> Node {
    Node { position_type: PositionType::Absolute, ..default() }
}

/// Spawn the layers and the motes as children of `parent`, bottom to top.
pub fn spawn(commands: &mut Commands, parent: Entity, assets: &AssetServer, art: &Art) {
    let load = |file: &str| -> Handle<Image> { assets.load(format!("{SOURCE}://{file}")) };
    commands.entity(parent).with_children(|c| {
        for (index, layer) in art.manifest.layers.iter().enumerate() {
            let image = load(&layer.file);
            for copy in 0..if layer.tile_x { TILE_COPIES } else { 1 } {
                c.spawn((
                    Name::new(format!("login layer {}", layer.file)),
                    ArtLayer { index, copy, image: image.clone() },
                    bg_node(),
                    ImageNode { image: image.clone(), color: Color::NONE, ..default() },
                ));
            }
        }
        if let Some(def) = &art.manifest.motes {
            let image = load(&def.file);
            for i in 0..art.motes.len() {
                c.spawn((Name::new("login mote"), Mote(i), bg_node(), ImageNode { image: image.clone(), color: Color::NONE, ..default() }));
            }
        }
    });
}

/// The panel image of the manifest.
pub fn panel_image(assets: &AssetServer, art: &Art) -> Handle<Image> {
    assets.load(format!("{SOURCE}://{}", art.manifest.panel.file))
}

/// Move everything: mouse parallax, drift with wrap, breathing, the motes.
#[allow(clippy::type_complexity)]
pub fn animate(
    time: Res<Time>,
    window: Query<&Window, With<PrimaryWindow>>,
    images: Res<Assets<Image>>,
    mut art: ResMut<Art>,
    mut layers: Query<(&ArtLayer, &mut Node, &mut ImageNode), Without<Mote>>,
    mut motes: Query<(&Mote, &mut Node, &mut ImageNode), Without<ArtLayer>>,
) {
    let Ok(window) = window.single() else { return };
    let size = Vec2::new(window.resolution.physical_width() as f32, window.resolution.physical_height() as f32);
    if size.x < 1.0 || size.y < 1.0 {
        return;
    }
    let canvas = Canvas::fit(size);
    let dt = time.delta_secs().min(0.1);
    let art = &mut *art;
    art.time += dt;
    // the mouse, as canvas pixels from the window centre, eased
    let scale_factor = window.resolution.scale_factor();
    let target = window.cursor_position().map_or(Vec2::ZERO, |p| ((p * scale_factor - size / 2.0) / canvas.s).clamp(-art.manifest.mouse_clamp, art.manifest.mouse_clamp));
    art.mouse += (target - art.mouse) * (1.0 - (-3.0 * dt).exp());
    let t = art.time;

    for (layer, mut node, mut image) in &mut layers {
        let def = &art.manifest.layers[layer.index];
        let Some(native) = images.get(&layer.image).map(|i| i.size().as_vec2()) else { continue };
        let w = (TAU * t / def.period + def.phase).sin();
        let scale = def.scale * (1.0 + def.scale_amp * w);
        let alpha = (def.alpha + def.alpha_amp * w).clamp(0.0, 1.0);
        let size_c = native * scale;
        let centre_c = def.position + art.mouse * def.parallax;
        let size_w = size_c * canvas.s;
        let left;
        let centre = canvas.to_window(centre_c);
        let top = centre.y - size_w.y / 2.0;
        if def.tile_x {
            let tile = size_w.x.max(1.0);
            // the first copy starts left of the window; the others follow, the ones past the right edge are hidden
            let drift = def.drift.x * canvas.s * t;
            let first = (centre.x - tile / 2.0 + drift).rem_euclid(tile) - tile;
            left = first + layer.copy as f32 * tile;
            node.display = if left > size.x { Display::None } else { Display::Flex };
        } else {
            left = centre.x - size_w.x / 2.0 + def.drift.x * canvas.s * t;
        }
        let top = top + if def.tile_x { 0.0 } else { def.drift.y * canvas.s * t };
        node.left = px(left);
        node.top = px(top);
        node.width = px(size_w.x);
        node.height = px(size_w.y);
        image.color = Color::srgba(1.0, 1.0, 1.0, alpha);
    }

    let Some(def) = &art.manifest.motes else { return };
    let def_tint = def.tint;
    let (fade_in, fade_out) = (def.fade_in.max(0.01), def.fade_out.max(0.01));
    for (mote, mut node, mut image) in &mut motes {
        let Some(m) = art.motes.get_mut(mote.0) else { continue };
        m.age += dt;
        if m.age >= m.life {
            *m = new_mote(art.manifest.motes.as_ref().unwrap(), &mut art.rng, false);
        }
        let x = m.x + (TAU * m.age / m.period + m.phase).sin() * m.amp;
        let y = m.y - m.speed * m.age;
        let fade = (m.age / fade_in).min(1.0) * ((m.life - m.age) / fade_out).min(1.0);
        let d = m.size * MOTE_SCALE * canvas.s;
        let p = canvas.to_window(Vec2::new(x, y));
        node.left = px(p.x - d / 2.0);
        node.top = px(p.y - d / 2.0);
        node.width = px(d);
        node.height = px(d);
        image.color = Color::srgba(def_tint[0], def_tint[1], def_tint[2], def_tint[3] * fade.clamp(0.0, 1.0));
    }
}
