//! Loot: the F and G icons next to the player when a corpse has loot, their keys, and the original's loot window.
//!
//! When the server says a killed unit has loot for the player (`aa_net::state::Loot::lootable`) the client
//! shows two bag icons beside the character (`reference/ingame/combat_death_2`): F, three bags
//! (`ui/icon/interaction/icon_interaction20.dds`), takes everything (`Command::LootOpen` with `loot_all`); G, one bag
//! (`icon_interaction21.dds`), opens the loot window. The server answers with the items left (`Event::LootBag`) and the
//! window is the original's (`x2ui/loot/loot.lua`): it listens for `LOOT_BAG_CHANGED(setTime)` (`setTime`: the bag
//! was opened with "loot all", the window only shows what could not be taken, for two seconds) and `LOOT_BAG_CLOSE`,
//! and asks `X2Loot` for the rows (`ingame.lua`: `HUD.loot_count`, `HUD.loot_item`), takes an item with
//! `X2Loot:LootItem(k)` (`HUD.loot_take`) and closes with `X2Loot:CloseLoot()` (`HUD.loot_close`).
//!
//! The icons are drawn on the name plate camera like the plates; their place is a fixed pixel offset from the
//! player (25 px to each side, 0.52 m over the feet), as measured on the capture.

use aa_net::state::{Loot, Player, Units, item_index, item_owner};
use aa_net::{Command, NetCommand};
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::text::{FontSize, FontSource, TextColor, TextFont};
use mlua::Lua;

use super::api::{HudBridge, Shared};
use super::plates::{PlateFont, PlateState};
use super::{HudKeyboardFocus, chat::ScriptedKeys};

/// Three bags: "loot all" (F).
const ICON_ALL: &str = "ui/icon/interaction/icon_interaction20.dds";
/// One bag: "open the loot" (G).
const ICON_OPEN: &str = "ui/icon/interaction/icon_interaction21.dds";
/// The id of the coin item in a loot bag (its count is the money).
const COINS: u32 = 500;
/// Metres from the player to a corpse that can be looted from here (retail: 3 m, measured the
/// same centre-to-centre way as the target frame's distance).
const RANGE: f32 = 3.0;
/// Icon size (the 48 px texture, a little larger on the capture), pixels.
const ICON_PX: f32 = 52.0;
/// Pixels from the player to the middle of each icon.
const SPREAD: f32 = 25.0;
/// Metres over the player's feet.
const RAISE: f32 = 0.52;
/// Pixels from the icon's middle down to its letter.
const LETTER_DROP: f32 = 21.0;
/// The plate camera's render layer (`plates::LAYER`).
const LAYER: usize = 30;

/// One line of the loot window.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub template_id: u32,
    pub count: i32,
    pub grade: u8,
    pub index: u16,
    pub owner: (u16, u32),
}

/// What the window asked for.
#[derive(Clone, Copy, Debug)]
pub enum Req {
    /// The 1 based row.
    Take(usize),
    Close,
}

/// What the Lua natives and the icons read.
#[derive(Debug, Default)]
pub struct LootView {
    pub rows: Vec<Row>,
    /// The nearest corpse with loot in reach: what F and G act on.
    pub nearest: Option<(u16, u32)>,
    /// The corpse the window was last opened for (what it closes).
    pub last_owner: Option<(u16, u32)>,
    pub requests: Vec<Req>,
}

/// `HUD.loot_count`, `loot_item(k)`, `loot_take(k)`, `loot_close()`.
pub fn install(lua: &Lua, hud: &mlua::Table, bridge: &Shared) -> mlua::Result<()> {
    let b = bridge.clone();
    hud.raw_set("loot_count", lua.create_function(move |_, ()| Ok(b.borrow().loot.rows.len()))?)?;
    let b = bridge.clone();
    hud.raw_set(
        "loot_item",
        lua.create_function(move |_, k: usize| {
            let d = b.borrow();
            Ok(match k.checked_sub(1).and_then(|i| d.loot.rows.get(i)) {
                Some(r) => (Some(r.template_id), Some(r.count), Some(super::store::ui_grade(r.grade))),
                None => (None, None, None),
            })
        })?,
    )?;
    let b = bridge.clone();
    hud.raw_set(
        "loot_take",
        lua.create_function(move |_, k: usize| {
            b.borrow_mut().loot.requests.push(Req::Take(k));
            Ok(())
        })?,
    )?;
    let b = bridge.clone();
    hud.raw_set(
        "loot_close",
        lua.create_function(move |_, ()| {
            b.borrow_mut().loot.requests.push(Req::Close);
            Ok(())
        })?,
    )
}

/// Whether a corpse `dist` metres away is close enough to loot from here.
fn in_reach(dist: f32) -> bool {
    dist <= RANGE
}

/// The rows of the bag the server sent, the corpse F and G act on, and the window's events.
#[allow(clippy::too_many_arguments)]
pub fn sync_loot(
    rt: Option<NonSend<aa_ui::UiRuntime>>,
    bridge: NonSend<HudBridge>,
    loot: Res<Loot>,
    units: Res<Units>,
    player: Res<Player>,
    mut seen: Local<(u32, u32)>,
) {
    let own = player.spawn.as_ref().map_or(0, |s| s.obj_id);
    let mut fire: Vec<Fire> = Vec::new();
    {
        let mut d = bridge.0.borrow_mut();
        let pos = d.pos;
        let game = d.game.clone();
        let game = game.get();
        // the server's edge-to-edge reach takes both units' scaled actor-model radii off
        let own_radius = player.spawn.as_ref().map_or(0.0, |s| game.map_or(0.0, |g| g.model_size(s.unit.model_id, s.unit.scale)));
        // the corpse in reach that is nearest (the player may be dead: nothing to loot)
        let dead = player.spawn.as_ref().is_some_and(|_| units.get(own).is_some_and(|u| u.dead));
        d.loot.nearest = loot
            .lootable
            .iter()
            .filter(|_| !dead)
            .filter_map(|&(kind, id)| {
                let u = units.get(id)?;
                let radius = game.map_or(0.0, |g| g.model_size(u.model_id, u.scale));
                let dist = super::api::distance_to(pos, own_radius, Vec3::new(u.pos.x, u.pos.y, u.pos.z), radius);
                in_reach(dist).then_some((dist, (kind, id)))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, o)| o);
        if loot.is_changed() {
            d.loot.rows = loot.bag.iter().map(|it| Row { template_id: it.template_id, count: it.count, grade: it.grade, index: item_index(it), owner: item_owner(it) }).collect();
            if let Some(r) = d.loot.rows.first() {
                d.loot.last_owner = Some(r.owner);
            }
        }
        if loot.bag_seq != seen.0 {
            seen.0 = loot.bag_seq;
            fire.push(Fire::Changed(loot.bag_loot_all));
        }
        if loot.taken_seq != seen.1 {
            seen.1 = loot.taken_seq;
            // the window lists what is left, or goes away with the last item
            fire.push(if loot.bag.is_empty() { Fire::Close } else { Fire::Changed(false) });
        }
    }
    let Some(rt) = rt else { return };
    for event in fire {
        match event {
            Fire::Changed(set_time) => rt.fire_event("LOOT_BAG_CHANGED", set_time),
            Fire::Close => rt.fire_event("LOOT_BAG_CLOSE", ()),
        }
    }
}

/// An event of the loot window.
enum Fire {
    /// `LOOT_BAG_CHANGED(setTime)`.
    Changed(bool),
    /// `LOOT_BAG_CLOSE`.
    Close,
}

/// F loots everything, G opens the loot; what the window asks for becomes commands.
pub fn loot_input(
    actions: Res<crate::settings::Actions>,
    focus: Res<HudKeyboardFocus>,
    rt: Option<NonSend<aa_ui::UiRuntime>>,
    bridge: NonSend<HudBridge>,
    mut script: ResMut<ScriptedKeys>,
    mut net: MessageWriter<NetCommand>,
) {
    let mut d = bridge.0.borrow_mut();
    // the window's requests
    for req in std::mem::take(&mut d.loot.requests) {
        match req {
            Req::Take(k) => {
                if let Some(r) = k.checked_sub(1).and_then(|i| d.loot.rows.get(i)) {
                    net.write(NetCommand(Command::LootItem { owner_type: r.owner.0, owner_obj_id: r.owner.1, item_index: r.index }));
                }
            }
            Req::Close => {
                if let Some((kind, id)) = d.loot.last_owner.take() {
                    net.write(NetCommand(Command::LootClose { owner_type: kind, owner_obj_id: id }));
                }
            }
        }
    }
    // the keys (a scripted one, `--hud-key f 20`, counts too)
    let scripted = script.take_loot();
    let typing = focus.0 || rt.is_some_and(|rt| rt.keyboard_captured());
    let all = scripted == Some(true) || (!typing && actions.just_pressed("do_interaction_1", 0));
    let open = scripted == Some(false) || (!typing && actions.just_pressed("do_interaction_2", 0));
    if (all || open)
        && let Some((_, id)) = d.loot.nearest
    {
        d.loot.last_owner = d.loot.nearest;
        net.write(NetCommand(Command::LootOpen { obj_id: id, loot_all: all }));
    }
}

/// `--hud-fake-loot T` (a dev flag: the lane server's Angry Rat drops nothing): T seconds after the HUD is up the unit
/// nearest to the player is a lootable corpse, and two seconds later the loot window opens on a bag of three items.
pub fn fake_loot(time: Res<Time>, bridge: NonSend<HudBridge>, units: Res<Units>, player: Res<Player>, inventory: Res<aa_net::state::Inventory>, mut loot: ResMut<Loot>, mut state: Local<(Option<f32>, Option<f32>, u8)>) {
    let at = *state.0.get_or_insert_with(|| {
        let args: Vec<String> = std::env::args().collect();
        let t = args.iter().position(|a| a == "--hud-fake-loot").and_then(|i| args.get(i + 1)).and_then(|v| v.parse::<f32>().ok());
        t.unwrap_or(f32::MAX)
    });
    if at == f32::MAX || player.spawn.is_none() {
        return;
    }
    let now = time.elapsed_secs();
    let started = *state.1.get_or_insert(now);
    let t = now - started;
    let own = player.spawn.as_ref().map_or(0, |s| s.obj_id);
    if state.2 == 0 && t >= at {
        let me = bridge.0.borrow().pos;
        let nearest = units.iter().filter(|u| u.obj_id != own).min_by(|a, b| {
            me.distance(Vec3::new(a.pos.x, a.pos.y, a.pos.z)).total_cmp(&me.distance(Vec3::new(b.pos.x, b.pos.y, b.pos.z)))
        });
        let Some(unit) = nearest else { return };
        info!("hud: fake loot on unit {} ({})", unit.obj_id, unit.name);
        loot.lootable = vec![(aa_net::state::owner_type::NPC, unit.obj_id)];
        state.2 = 1;
    }
    if state.2 == 1 && t >= at + 2.0 {
        let Some(&(kind, id)) = loot.lootable.first() else { return };
        let templates: Vec<(u32, u8)> = inventory.bag_items().map(|(_, it)| (it.template_id, it.grade)).take(2).collect();
        let mut items = vec![aa_net::model::Item { template_id: COINS, count: 1234, ..Default::default() }];
        items.extend(templates.iter().map(|&(template_id, grade)| aa_net::model::Item { template_id, grade, count: 3, ..Default::default() }));
        for (i, it) in items.iter_mut().enumerate() {
            it.id = (u64::from(id) << 32) + (u64::from(kind) << 16) + (i as u64 + 1);
        }
        loot.bag = items;
        loot.bag_loot_all = false;
        loot.bag_seq = loot.bag_seq.wrapping_add(1);
        state.2 = 2;
    }
}

#[derive(Component)]
pub struct LootIcon {
    all: bool,
}

#[derive(Resource, Default)]
pub struct LootAssets {
    all: Option<Handle<Image>>,
    open: Option<Handle<Image>>,
}

/// Keeps the two icons beside the player while a corpse in reach has loot.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn update_icons(
    mut commands: Commands,
    bridge: NonSend<HudBridge>,
    player: Res<Player>,
    plates: Res<PlateState>,
    font: Res<PlateFont>,
    mut assets: ResMut<LootAssets>,
    mut images: ResMut<Assets<Image>>,
    world_camera: Query<(&Camera, &Transform), (With<Camera3d>, Without<LootIcon>)>,
    transforms: Query<&Transform, (Without<LootIcon>, Without<Camera3d>)>,
    mut icons: Query<(Entity, &LootIcon, &mut Transform), Without<Camera3d>>,
) {
    let own = player.spawn.as_ref().map_or(0, |s| s.obj_id);
    let wanted = bridge.0.borrow().loot.nearest.is_some();
    let feet = plates.entities.get(&own).and_then(|e| transforms.get(*e).ok()).map(|t| t.translation);
    let (Some(feet), true) = (feet, wanted) else {
        for (e, _, _) in &icons {
            commands.entity(e).despawn();
        }
        return;
    };
    let Some((camera, camera_transform)) = world_camera.iter().find(|(c, _)| c.is_active) else { return };
    let Some(viewport) = camera.logical_viewport_size() else { return };
    let Ok(screen) = camera.world_to_viewport(&GlobalTransform::from(*camera_transform), feet + Vec3::Y * RAISE) else { return };
    let (x, y) = ((screen.x - viewport.x / 2.0).round(), (viewport.y / 2.0 - screen.y).round());

    if icons.is_empty() {
        for (all, path) in [(true, ICON_ALL), (false, ICON_OPEN)] {
            let image = {
                let cached = if all { &mut assets.all } else { &mut assets.open };
                if cached.is_none() {
                    *cached = super::ring::dds_image(path).map(|i| images.add(i));
                }
                cached.clone()
            };
            let Some(image) = image else {
                warn!("hud: cannot read {path}: no loot icon");
                continue;
            };
            let mut sprite = Sprite::from_image(image);
            sprite.custom_size = Some(Vec2::splat(ICON_PX));
            commands
                .spawn((Name::new(if all { "loot all icon" } else { "loot open icon" }), LootIcon { all }, sprite, Transform::from_xyz(x, y, 1.0), RenderLayers::layer(LAYER)))
                .with_children(|c| {
                    c.spawn((
                        Text2d::new(if all { "F" } else { "G" }),
                        TextFont { font: FontSource::Handle(font.0.clone()), font_size: FontSize::Px(15.0), ..default() },
                        TextColor(Color::srgb_u8(244, 150, 34)),
                        bevy::sprite::Text2dShadow { offset: Vec2::new(1.0, -1.0), color: Color::srgba(0.0, 0.0, 0.0, 0.9) },
                        Transform::from_xyz(0.0, -LETTER_DROP, 0.1),
                        RenderLayers::layer(LAYER),
                    ));
                });
        }
        return;
    }
    for (_, icon, mut t) in &mut icons {
        t.translation = Vec3::new(x + if icon.all { -SPREAD } else { SPREAD }, y, 1.0);
    }
}

pub fn despawn(mut commands: Commands, icons: Query<Entity, With<LootIcon>>) {
    for e in &icons {
        commands.entity(e).despawn();
    }
}

pub fn plugin(app: &mut App) {
    app.init_resource::<LootAssets>().add_systems(OnExit(crate::Screen::InGame), despawn);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loot_reach_is_the_retail_three_metres() {
        assert_eq!(RANGE, 3.0);
        assert!(in_reach(2.9));
        assert!(in_reach(3.0));
        assert!(!in_reach(3.1));
    }

    #[test]
    fn a_big_corpse_is_lootable_from_its_edge() {
        let edge = super::super::api::distance_to(Vec3::ZERO, 0.0, Vec3::new(22.0, 0.0, 0.0), 22.0);
        assert_eq!(edge, 0.0);
        assert!(in_reach(edge));
        let just_outside = super::super::api::distance_to(Vec3::ZERO, 0.0, Vec3::new(25.1, 0.0, 0.0), 22.0);
        assert!(!in_reach(just_outside));
    }
}
