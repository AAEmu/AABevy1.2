//! The Teleport Book (`x2ui/portal`) on the book aa-net reports (`aa_net::Portals`): the native `X2Warp`
//! functions the original window calls, its events, and the requests it makes.
//!
//! * `X2Warp.GetReturnList()` / `GetPortalList()` (called without `self`) and their counts: rows of
//!   `{portal_id, portal_type, name, zone_id, zone_name, world_name, x, y, z}`, world coordinates (the wire's are
//!   relative to the zone's origin cell, `worlds/main_world/world.xml` `Zone originX/originY` * 1024), `world_name` the
//!   continent key of `locale.portal` from `zone_groups.target_id` (3 `west`, 4 `east`, 6 `sea`, else `origin`).
//! * `X2Warp:GetBoundPortalId()`: the return point of `SCCharacterReturnDistricts`.
//! * `X2Warp:SavePortal(name)` / `RenamePortal(id, name)`: skills 11215 / 16841 with a `SavePortalInfo` object.
//! * `X2Warp:OpenPortal(type, id, name)`: the Teleport Book's skill 11216 with a `PortalInfo` object, cast by the book
//!   (the one used to open the window, else the first in the bag), the gate [`GATE_AHEAD`] metres in front of the player.
//! * `X2Warp:DeletePortal(type, id)`: `CSDeletePortal`; the entry leaves the book at once (the server's `SCPortalDeleted` is
//!   idempotent, see `aa_net::portal`).
//! * Events: `TOGGLE_PORTAL_DIALOG(addPortal)` when the Teleport Book is used (with Shift held: the Record dialog on
//!   top, see `docs/aa-client-portals.md`), `SAVE_PORTAL` / `RENAME_PORTAL` / `DELETE_PORTAL` after the book changed.
//!
//! Also here: the visited sub zones (`CSNotifySubZone`, which fills the Default Site tab), going through a portal
//! gate (a right click or F on the gate NPC: `CSUsePortal` with the interface option "use only my portal") and the effect
//! of `SCUnitPortalUsed` ([`unit_used`]).

use super::api::{HudBridge, HudData, Shared};
use aa_net::portal::portal_type;
use aa_net::wire::Pos;
use aa_net::{Command, NetCommand, PortalChange, PortalCommand, Portals};
use aa_ui::UiRuntime;
use aa_net::{Event, NetEvent, PortalEvent};
use bevy::prelude::*;
use mlua::{Lua, MultiValue, Table, Value};
use std::collections::HashMap;
use std::sync::OnceLock;

/// Skill of the Teleport Book (item 4045): `OpenPortalEffect`.
pub(crate) const OPEN_PORTAL_SKILL: u32 = 11216;
/// "Entering location in Teleport Book...": `SavePortal` with id 0 (Memory Ink).
const RECORD_SKILL: u32 = 11215;
/// "Editing Location in Teleport Book...": `SavePortal` with the book id.
const RENAME_SKILL: u32 = 16841;
/// The Teleport Book item.
const TELEPORT_BOOK: u32 = 4045;
/// NPC templates of the summoned gates: the entrance and the exit.
pub(crate) const GATE_TEMPLATES: [u32; 2] = [3891, 6949];
/// Metres in front of the player the gate opens at (the server accepts up to `open_portal_effects.distance` 3 m).
const GATE_AHEAD: f32 = 2.5;

/// The fx item `OnUnitPortalUsed` plays (x2game `FUN_393558e0`; read from a live 1.2 client, it is a constant of the client's
/// `.data`): `fx_items` 57 ("Curse of Doll_hit", the particle `ability_skill_table.blackmagic.black0011_hit`).
const USE_FX_ITEM: u32 = 57;
/// Seconds it lives (the client sets the effect's end time to now + 5.0 s).
const USE_FX_SECS: f32 = 5.0;

/// The fx item played on a unit that used a portal: dev override `AA_PORTAL_USE_FX=<fx_items id>` (e.g. 388, the casting
/// effect `portal_caster_target`, to see the pipeline draw something).
#[derive(Clone, Debug, PartialEq)]
struct UseFx {
    effect: String,
    /// `fx_items.offset_*` (cry axes).
    offset: Vec3,
}

fn use_fx() -> Option<&'static UseFx> {
    static FX: OnceLock<Option<UseFx>> = OnceLock::new();
    FX.get_or_init(|| {
        let id = std::env::var("AA_PORTAL_USE_FX").ok().and_then(|v| v.parse().ok()).unwrap_or(USE_FX_ITEM);
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        let read = || -> rusqlite::Result<Option<UseFx>> {
            let conn = rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
            let mut q = conn.prepare("SELECT asset_name, offset_x, offset_y, offset_z FROM fx_items WHERE id = ?1 AND fx_detail_type = 'FxParticle'")?;
            let mut rows = q.query_map([id], |r| Ok(UseFx { effect: r.get(0)?, offset: Vec3::new(r.get(1)?, r.get(2)?, r.get(3)?) }))?;
            rows.next().transpose()
        };
        read().unwrap_or_else(|e| {
            warn!("portal: fx item {id}: {e}");
            None
        })
    })
    .as_ref()
}

/// The effect of a unit that went through a portal, until its time is up.
#[derive(Component)]
struct UseEffect(Timer);

/// `SCUnitPortalUsed` (`OnUnitPortalUsed`): the client looks the unit up (the lane sends the gate NPC) and plays fx item
/// [`USE_FX_ITEM`] at its position for [`USE_FX_SECS`] seconds. In the 1.2 data that particle has no emitters (and no sound), so nothing shows by default.
fn unit_used(
    mut events: MessageReader<NetEvent>,
    mut commands: Commands,
    views: Res<crate::ingame::units::Views>,
    remote: Query<&GlobalTransform, With<crate::ingame::units::RemoteUnit>>,
    local: Query<(&crate::ingame::player::LocalPlayer, &GlobalTransform)>,
    units: Res<aa_net::state::Units>,
) {
    for NetEvent(e) in events.read() {
        let Event::Portal(PortalEvent::UnitPortalUsed { unit_id }) = e else { continue };
        let at = views
            .unit_entity(*unit_id)
            .and_then(|en| remote.get(en).ok())
            .map(|g| g.translation())
            .or_else(|| local.iter().find(|(l, _)| l.obj_id == *unit_id).map(|(_, g)| g.translation()))
            .or_else(|| units.get(*unit_id).map(|u| aa_world::cry_to_bevy(Vec3::new(u.pos.x, u.pos.y, u.pos.z))));
        let Some(at) = at else {
            debug!("portal: unit {unit_id} used a portal, not known");
            continue;
        };
        let Some(fx) = use_fx() else { continue };
        info!("portal: unit {unit_id} used a portal: {} at {:.1} {:.1} {:.1}", fx.effect, at.x, at.y, at.z);
        commands.spawn((
            Name::new(format!("portal use {unit_id}")),
            Transform::from_translation(at + aa_world::cry_to_bevy(fx.offset)),
            Visibility::default(),
            aa_fx::Effect { prime: false, item: true, ..aa_fx::Effect::new(fx.effect.clone()) },
            UseEffect(Timer::from_seconds(USE_FX_SECS, TimerMode::Once)),
        ));
    }
}

/// The server's answers to a portal delete, logged on arrival (the book itself is folded by aa-net).
fn log_deleted(mut events: MessageReader<NetEvent>, book: Res<Portals>) {
    for NetEvent(e) in events.read() {
        if let Event::Portal(PortalEvent::Deleted { portal_type, portal_id }) = e {
            info!("portal: SCPortalDeleted type {portal_type} id {portal_id} (book: {} private, {} districts)", book.private.len(), book.districts.len());
        }
    }
}

fn expire_use_effects(mut commands: Commands, time: Res<Time>, mut fx: Query<(Entity, &mut UseEffect)>) {
    for (e, mut t) in &mut fx {
        if t.0.tick(time.delta()).is_finished() {
            commands.entity(e).despawn();
        }
    }
}

/// The Teleport Book (or another item with its skill) was used: the HUD opens the book instead of casting
/// (`ingame::ground_target::route` sends this). `record`: with the Record dialog on top.
#[derive(Message, Clone, Copy, Debug)]
pub struct PortalBookUsed {
    pub item: Option<(u64, u32)>,
    pub record: bool,
}

/// What the scripts asked for, sent by [`requests`] (which knows the bag and the option).
#[derive(Clone, Debug, PartialEq)]
enum Req {
    Record(String),
    Rename(u32, String),
    Open { portal_type: u8, portal_id: u32, pos: Pos },
    Delete { portal_type: u8, portal_id: u32 },
    Use { obj_id: u32 },
}

#[derive(Default)]
pub(super) struct View {
    /// The book as last folded from aa-net (what the getters read).
    book: Portals,
    /// The revision whose event was raised.
    raised: u32,
    /// The Teleport Book that opened the window (the caster of the gate).
    item: Option<(u64, u32)>,
    requests: Vec<Req>,
    /// The sub zones last reported to the server (leaf first).
    sub_zones: Vec<u32>,
}

/// Zone origins (`world.xml`) and continents (`zone_groups.target_id`) by zone key.
#[derive(Default)]
struct Catalog {
    origins: HashMap<u32, (f32, f32)>,
    continents: HashMap<u32, u32>,
}

fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let mut c = Catalog::default();
        match aa_pak::Pak::global().read_string(&format!("worlds/{}/world.xml", crate::ingame::WORLD)) {
            Ok(xml) => c.origins = parse_zone_origins(&xml),
            Err(e) => warn!("portal: world.xml: {e}"),
        }
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        let read = || -> rusqlite::Result<HashMap<u32, u32>> {
            let conn = rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
            let mut q = conn.prepare("SELECT z.zone_key, g.target_id FROM zones z JOIN zone_groups g ON g.id = z.group_id")?;
            let rows = q.query_map([], |r| Ok((r.get::<_, i64>(0)? as u32, r.get::<_, Option<i64>>(1)?.unwrap_or(0) as u32)))?;
            rows.collect()
        };
        match read() {
            Ok(m) => c.continents = m,
            Err(e) => warn!("portal: zone continents: {e}"),
        }
        c
    })
}

/// `<Zone name=".." id="129" originX="9" originY="14">` -> zone key -> origin in metres.
fn parse_zone_origins(xml: &str) -> HashMap<u32, (f32, f32)> {
    let attr = |tag: &str, name: &str| -> Option<f32> {
        let key = format!(" {name}=\"");
        let start = tag.find(&key)? + key.len();
        tag[start..start + tag[start..].find('"')?].parse().ok()
    };
    let mut out = HashMap::new();
    for chunk in xml.split("<Zone ").skip(1) {
        let tag = &chunk[..chunk.find('>').unwrap_or(chunk.len())];
        let tag = format!(" {tag}");
        if let (Some(id), Some(x), Some(y)) = (attr(&tag, "id"), attr(&tag, "originX"), attr(&tag, "originY")) {
            out.insert(id as u32, (x * 1024.0, y * 1024.0));
        }
    }
    out
}

/// `locale.portal` continent keys.
fn world_name(target_id: u32) -> &'static str {
    match target_id {
        3 => "west",
        4 => "east",
        6 => "sea",
        _ => "origin",
    }
}

/// The rows of one tab, as `FillPortalList` and `GetPortalInfo` read them.
fn rows(lua: &Lua, data: &HudData, list: &[aa_net::PortalEntry], kind: u8) -> mlua::Result<Table> {
    let cat = catalog();
    let game = data.game.get();
    let out = lua.create_table()?;
    for (i, p) in list.iter().enumerate() {
        let (ox, oy) = cat.origins.get(&p.zone_id).copied().unwrap_or((0.0, 0.0));
        let row = lua.create_table()?;
        row.raw_set("portal_id", p.id)?;
        row.raw_set("portal_type", kind)?;
        row.raw_set("name", p.name.as_str())?;
        row.raw_set("zone_id", p.zone_id)?;
        row.raw_set("zone_name", game.and_then(|g| g.zone(p.zone_id)).map_or(String::new(), |(_, z, _)| z.text.clone()))?;
        row.raw_set("world_name", world_name(cat.continents.get(&p.zone_id).copied().unwrap_or(0)))?;
        row.raw_set("x", p.off_x + ox)?;
        row.raw_set("y", p.off_y + oy)?;
        row.raw_set("z", p.z)?;
        out.raw_set(i + 1, row)?;
    }
    Ok(out)
}

fn arg_num(args: &[Value], i: usize) -> Option<f64> {
    match args.get(i)? {
        Value::Integer(n) => Some(*n as f64),
        Value::Number(n) => Some(*n),
        Value::String(s) => s.to_str().ok()?.trim().parse().ok(),
        _ => None,
    }
}

fn arg_text(args: &[Value], i: usize) -> Option<String> {
    match args.get(i)? {
        Value::String(s) => Some(s.to_str().ok()?.to_string()),
        Value::Integer(n) => Some(n.to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// The arguments of a method call without its `self` (the `X2Warp` table).
fn method_args(args: MultiValue) -> Vec<Value> {
    let mut v: Vec<Value> = args.into_iter().collect();
    if matches!(v.first(), Some(Value::Table(_))) {
        v.remove(0);
    }
    v
}

pub(super) fn install(lua: &Lua, state: &Shared) -> mlua::Result<()> {
    let warp: Table = match lua.globals().get::<Value>("X2Warp")? {
        Value::Table(t) => t,
        _ => {
            let t = lua.create_table()?;
            lua.globals().raw_set("X2Warp", t.clone())?;
            t
        }
    };
    for (name, private) in [("GetReturnList", false), ("GetPortalList", true)] {
        let s = state.clone();
        warp.raw_set(
            name,
            lua.create_function(move |lua, _: MultiValue| {
                let d = s.borrow();
                let (list, kind) = if private { (&d.portal.book.private, portal_type::PRIVATE) } else { (&d.portal.book.districts, portal_type::DISTRICT) };
                rows(lua, &d, list, kind)
            })?,
        )?;
        let s = state.clone();
        warp.raw_set(
            format!("{name}Count"),
            lua.create_function(move |_, _: MultiValue| {
                let d = s.borrow();
                Ok(if private { d.portal.book.private.len() } else { d.portal.book.districts.len() })
            })?,
        )?;
    }
    let s = state.clone();
    warp.raw_set("GetBoundPortalId", lua.create_function(move |_, _: MultiValue| Ok(s.borrow().portal.book.return_point_id))?)?;
    let s = state.clone();
    warp.raw_set(
        "SavePortal",
        lua.create_function(move |_, args: MultiValue| {
            let args = method_args(args);
            if let Some(name) = arg_text(&args, 0) {
                s.borrow_mut().portal.requests.push(Req::Record(name));
            }
            Ok(())
        })?,
    )?;
    let s = state.clone();
    warp.raw_set(
        "RenamePortal",
        lua.create_function(move |_, args: MultiValue| {
            let args = method_args(args);
            if let (Some(id), Some(name)) = (arg_num(&args, 0), arg_text(&args, 1)) {
                s.borrow_mut().portal.requests.push(Req::Rename(id as u32, name));
            }
            Ok(())
        })?,
    )?;
    let s = state.clone();
    warp.raw_set(
        "OpenPortal",
        lua.create_function(move |_, args: MultiValue| {
            let args = method_args(args);
            let (Some(kind), Some(id)) = (arg_num(&args, 0), arg_num(&args, 1)) else { return Ok(()) };
            let mut d = s.borrow_mut();
            let ahead = crate::ingame::player::forward(d.yaw) * GATE_AHEAD;
            let pos = Pos::new(d.pos.x + ahead.x, d.pos.y + ahead.y, d.pos.z);
            d.portal.requests.push(Req::Open { portal_type: kind as u8, portal_id: id as u32, pos });
            Ok(())
        })?,
    )?;
    let s = state.clone();
    warp.raw_set(
        "DeletePortal",
        lua.create_function(move |_, args: MultiValue| {
            let args = method_args(args);
            let (Some(kind), Some(id)) = (arg_num(&args, 0), arg_num(&args, 1)) else { return Ok(()) };
            let (kind, id) = (kind as u8, id as u32);
            let mut d = s.borrow_mut();
            // the window refills its page from the getters right after this call
            let list = if kind == portal_type::DISTRICT { &mut d.portal.book.districts } else { &mut d.portal.book.private };
            list.retain(|p| p.id != id);
            d.portal.requests.push(Req::Delete { portal_type: kind, portal_id: id });
            Ok(())
        })?,
    )?;
    let s = state.clone();
    warp.raw_set(
        "EndPortalInteraction",
        lua.create_function(move |_, _: MultiValue| {
            s.borrow_mut().portal.item = None;
            Ok(())
        })?,
    )?;
    // NPC teleporters: no NPC of the 1.2 data is one, and the lane's `CSUseTeleport` only logs
    warp.raw_set("IsTeleporterNpc", lua.create_function(|_, _: MultiValue| Ok(false))?)?;
    warp.raw_set("UseTeleport", lua.create_function(|_, _: MultiValue| Ok(()))?)?;
    Ok(())
}

/// A right click / F on an NPC: a summoned gate is gone through, not talked to (`hud::dialog`). True when it was a gate.
pub(super) fn use_gate(data: &mut HudData, obj_id: u32, template_id: u32) -> bool {
    if !GATE_TEMPLATES.contains(&template_id) {
        return false;
    }
    info!("portal: going through gate {obj_id} (npc {template_id})");
    data.portal.requests.push(Req::Use { obj_id });
    true
}

/// Folds aa-net's book into the view and raises the window's events.
fn track(book: Res<Portals>, bridge: NonSend<HudBridge>, rt: Option<NonSend<UiRuntime>>, built: Res<super::HudBuilt>) {
    let event = {
        let mut d = bridge.0.borrow_mut();
        if book.is_changed() {
            d.portal.book = book.clone();
        }
        if d.portal.raised == book.revision || !built.up {
            None
        } else {
            d.portal.raised = book.revision;
            book.last.map(|c| match c {
                PortalChange::Saved => "SAVE_PORTAL",
                PortalChange::Renamed => "RENAME_PORTAL",
                // the Default Site tab shows the new list on its next refresh: the delete handler refreshes the page
                PortalChange::Deleted | PortalChange::Districts => "DELETE_PORTAL",
            })
        }
    };
    if let (Some(name), Some(rt)) = (event, rt) {
        debug!("portal: {name}");
        rt.fire_event(name, ());
    }
}

/// The Teleport Book was used: open the book.
fn book_used(mut used: MessageReader<PortalBookUsed>, bridge: NonSend<HudBridge>, rt: Option<NonSend<UiRuntime>>, built: Res<super::HudBuilt>) {
    for u in used.read() {
        bridge.0.borrow_mut().portal.item = u.item;
        let Some(rt) = rt.as_deref().filter(|_| built.up) else { continue };
        info!("portal: the Teleport Book opens (record {})", u.record);
        rt.fire_event("TOGGLE_PORTAL_DIALOG", (u.record, ()));
    }
}

/// "Use only my portal" (`OPTION_ITEM_USE_ONLY_MY_PORTAL`, Safe Portal Mode).
fn only_my_portal(rt: Option<&UiRuntime>) -> bool {
    rt.and_then(|rt| rt.lua.load("return X2Option:GetOptionItemValue(OPTION_ITEM_USE_ONLY_MY_PORTAL)").eval::<Option<f64>>().ok().flatten())
        .is_some_and(|v| v != 0.0)
}

fn requests(
    bridge: NonSend<HudBridge>,
    rt: Option<NonSend<UiRuntime>>,
    inventory: Res<aa_net::state::Inventory>,
    mut out: MessageWriter<NetCommand>,
) {
    let (reqs, item) = {
        let mut d = bridge.0.borrow_mut();
        (std::mem::take(&mut d.portal.requests), d.portal.item)
    };
    for r in reqs {
        let command = match r {
            Req::Record(name) => PortalCommand::Record { skill_id: RECORD_SKILL, name },
            Req::Rename(portal_id, name) => PortalCommand::Rename { skill_id: RENAME_SKILL, portal_id, name },
            Req::Open { portal_type, portal_id, pos } => {
                let book = item.or_else(|| inventory.bag_items().find(|(_, i)| i.template_id == TELEPORT_BOOK).map(|(_, i)| (i.id, i.template_id)));
                let Some((item_id, template_id)) = book else {
                    warn!("portal: no Teleport Book in the bag: the gate cannot be opened");
                    continue;
                };
                let caster = aa_net::skill::StartCaster::Item { item_id, template_id };
                PortalCommand::Open { skill_id: OPEN_PORTAL_SKILL, caster, portal_type, portal_id, pos }
            }
            Req::Delete { portal_type, portal_id } => PortalCommand::Delete { portal_type, portal_id },
            Req::Use { obj_id } => PortalCommand::Use { obj_id, only_my_portal: only_my_portal(rt.as_deref()) },
        };
        info!("portal: {command:?}");
        out.write(NetCommand(Command::Portal(command)));
    }
}

/// `CSNotifySubZone` whenever the sub zone under the player changes: the root first, the smallest one last (the server
/// keeps the last one as the character's sub zone and adds the districts with a recall point to the book).
fn notify_sub_zone(bridge: NonSend<HudBridge>, player: Res<aa_net::state::Player>, time: Res<Time>, mut since: Local<f32>, mut out: MessageWriter<NetCommand>) {
    *since += time.delta_secs();
    if *since < 0.5 || player.spawn.is_none() {
        return;
    }
    *since = 0.0;
    let mut d = bridge.0.borrow_mut();
    let Some(game) = d.game.get() else { return };
    let chain = game.sub_zone_chain_at(d.pos.x, d.pos.y);
    if chain.is_empty() || chain == d.portal.sub_zones {
        return;
    }
    for &sub_zone_id in chain.iter().rev() {
        out.write(NetCommand(Command::Portal(PortalCommand::NotifySubZone { sub_zone_id })));
    }
    debug!("portal: entered sub zones {chain:?}");
    d.portal.sub_zones = chain;
}

/// A new character, or the world left: start again (the server sends the book on entering).
fn reset(mut events: MessageReader<aa_net::NetEvent>, bridge: NonSend<HudBridge>) {
    for aa_net::NetEvent(e) in events.read() {
        if matches!(e, aa_net::Event::PlayerSpawn(_) | aa_net::Event::LeftWorld { .. } | aa_net::Event::Disconnected { .. }) {
            let mut d = bridge.0.borrow_mut();
            d.portal.sub_zones.clear();
            d.portal.item = None;
            d.portal.requests.clear();
        }
    }
}

/// `--portal-lua T CODE` (repeatable): run CODE in the HUD's Lua state T seconds after the HUD came up (unattended checks
/// of the book: `X2Warp:SavePortal('Lake')`, `FAKE.fire_now('TOGGLE_PORTAL_DIALOG', true)` ...).
#[derive(Resource, Default)]
struct ScriptedLua {
    steps: Vec<(f32, String)>,
    since: f32,
}

impl ScriptedLua {
    fn from_args() -> Self {
        let args: Vec<String> = aa_dev::args();
        let mut steps: Vec<(f32, String)> = args
            .windows(3)
            .filter(|w| w[0] == "--portal-lua")
            .filter_map(|w| Some((w[1].parse().ok()?, w[2].clone())))
            .collect();
        steps.sort_by(|a, b| b.0.total_cmp(&a.0));
        ScriptedLua { steps, since: 0.0 }
    }
}

fn scripted_lua(mut script: ResMut<ScriptedLua>, time: Res<Time>, rt: Option<NonSend<UiRuntime>>, built: Res<super::HudBuilt>) {
    if script.steps.is_empty() || !built.up {
        return;
    }
    let Some(rt) = rt else { return };
    script.since += time.delta_secs();
    while script.steps.last().is_some_and(|s| s.0 <= script.since) {
        let (_, code) = script.steps.pop().unwrap();
        info!("portal: --portal-lua {code}");
        if let Err(e) = rt.exec("portal-lua", &code) {
            warn!("portal: --portal-lua: {e}");
        }
    }
}

/// `--portal-used T` (repeatable): T seconds after the player is in the world, fake a `SCUnitPortalUsed` for the player's unit
/// (what a bystander at a gate gets), to see the use effect (`AA_PORTAL_USE_FX=388` draws a visible one).
fn scripted_used(
    mut times: Local<Option<Vec<f32>>>,
    mut since: Local<f32>,
    time: Res<Time>,
    player: Query<&crate::ingame::player::LocalPlayer>,
    mut out: MessageWriter<NetEvent>,
) {
    let times = times.get_or_insert_with(|| {
        let args: Vec<String> = aa_dev::args();
        let mut t: Vec<f32> = args.windows(2).filter(|w| w[0] == "--portal-used").filter_map(|w| w[1].parse().ok()).collect();
        t.sort_by(|a, b| b.total_cmp(a));
        t
    });
    let Ok(me) = player.single() else { return };
    if times.is_empty() {
        return;
    }
    *since += time.delta_secs();
    while times.last().is_some_and(|t| *t <= *since) {
        times.pop();
        info!("portal: --portal-used: unit {}", me.obj_id);
        out.write(NetEvent(Event::Portal(PortalEvent::UnitPortalUsed { unit_id: me.obj_id })));
    }
}

pub(super) fn plugin(app: &mut App) {
    app.insert_resource(ScriptedLua::from_args())
        .add_systems(Update, scripted_lua.after(super::sync_hud).run_if(in_state(crate::Screen::InGame)))
        .add_message::<PortalBookUsed>()
        .add_systems(Update, (scripted_used, unit_used, log_deleted, expire_use_effects).chain().run_if(in_state(crate::Screen::InGame)))
        .add_systems(Update, (reset, track).chain().before(aa_ui::UiInput).before(super::sync_hud).before(super::build_hud))
        .add_systems(
            Update,
            (book_used, requests, notify_sub_zone.run_if(in_state(crate::Screen::InGame)))
                .chain()
                .after(super::sync_hud)
                .after(aa_ui::UiInput),
        );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zone_origins_come_from_the_zone_tags() {
        let xml = r#"<World><ZoneList><Zone name="w_gweonid_forest_1" id="129" originX="9" originY="14"><cellList/></Zone>
            <Zone name="e_rainbow_field_2" id="187" originX="18" originY="7"></Zone></ZoneList></World>"#;
        let o = parse_zone_origins(xml);
        assert_eq!(o.get(&129), Some(&(9216.0, 14336.0)));
        assert_eq!(o.get(&187), Some(&(18432.0, 7168.0)));
        assert_eq!(world_name(3), "west");
        assert_eq!(world_name(4), "east");
        assert_eq!(world_name(1), "origin");
    }

    #[test]
    fn the_getters_build_rows_and_the_requests_queue() {
        let lua = Lua::new();
        let state: Shared = std::rc::Rc::new(std::cell::RefCell::new(HudData::default()));
        {
            let mut d = state.borrow_mut();
            d.portal.book.private.push(aa_net::PortalEntry { id: 7, name: "Home".into(), zone_id: 0, off_x: 10.0, off_y: 20.0, z: 30.0, z_rot: 0.0 });
            d.portal.book.return_point_id = 240;
        }
        install(&lua, &state).unwrap();
        let (n, id, kind, name, x): (usize, u32, u8, String, f32) = lua
            .load("local l = X2Warp.GetPortalList(); return X2Warp.GetPortalListCount(), l[1].portal_id, l[1].portal_type, l[1].name, l[1].x")
            .eval()
            .unwrap();
        assert_eq!((n, id, kind, name.as_str(), x), (1, 7, 2, "Home", 10.0));
        assert_eq!(lua.load("return X2Warp:GetBoundPortalId()").eval::<u32>().unwrap(), 240);
        assert_eq!(lua.load("return X2Warp.GetReturnListCount()").eval::<usize>().unwrap(), 0);
        lua.load("X2Warp:SavePortal('Farm'); X2Warp:RenamePortal(7, 'Barn'); X2Warp:DeletePortal(2, 7); X2Warp:OpenPortal(1, 240, 'Harani')").exec().unwrap();
        let d = state.borrow();
        assert!(d.portal.book.private.is_empty(), "a deleted row leaves the getters at once");
        assert_eq!(d.portal.requests[0], Req::Record("Farm".into()));
        assert_eq!(d.portal.requests[1], Req::Rename(7, "Barn".into()));
        assert_eq!(d.portal.requests[2], Req::Delete { portal_type: 2, portal_id: 7 });
        // facing north (yaw 0): the gate is 2.5 m north of the player
        assert!(matches!(d.portal.requests[3], Req::Open { portal_type: 1, portal_id: 240, pos } if (pos.y - GATE_AHEAD).abs() < 1e-4 && pos.x.abs() < 1e-4));
    }

    /// The unit that used a portal gets the client's fx item for 5 s (needs the game database).
    #[test]
    fn a_unit_that_used_a_portal_gets_the_use_effect_for_five_seconds() {
        let Some(fx) = use_fx() else { return };
        assert_eq!(fx.effect, "ability_skill_table.blackmagic.black0011_hit");
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_message::<NetEvent>()
            .init_resource::<crate::ingame::units::Views>()
            .init_resource::<aa_net::state::Units>()
            .add_systems(Update, (unit_used, expire_use_effects).chain());
        let player = app
            .world_mut()
            .spawn((crate::ingame::player::LocalPlayer { obj_id: 77 }, GlobalTransform::from(Transform::from_xyz(10.0, 20.0, 30.0))))
            .id();
        app.world_mut().write_message(NetEvent(Event::Portal(PortalEvent::UnitPortalUsed { unit_id: 5 })));
        app.update();
        assert_eq!(app.world_mut().query::<&UseEffect>().iter(app.world()).count(), 0, "an unknown unit gets nothing");
        app.world_mut().write_message(NetEvent(Event::Portal(PortalEvent::UnitPortalUsed { unit_id: 77 })));
        app.update();
        let mut q = app.world_mut().query::<(&Transform, &aa_fx::Effect, &UseEffect)>();
        let (t, e, _) = q.single(app.world()).unwrap();
        assert!((t.translation - Vec3::new(10.0, 20.0, 30.0)).length() < 0.5, "{:?}", t.translation);
        assert_eq!(e.name, fx.effect);
        for mut t in app.world_mut().query::<&mut UseEffect>().iter_mut(app.world_mut()) {
            t.0.tick(std::time::Duration::from_secs_f32(5.1));
        }
        app.world_mut().despawn(player);
        app.update();
        assert_eq!(app.world_mut().query::<&UseEffect>().iter(app.world()).count(), 0, "gone after its five seconds");
    }

    #[test]
    fn only_gate_npcs_are_gone_through() {
        let mut d = HudData::default();
        assert!(!use_gate(&mut d, 5, 1000));
        assert!(use_gate(&mut d, 5, 3891));
        assert_eq!(d.portal.requests, vec![Req::Use { obj_id: 5 }]);
    }
}
