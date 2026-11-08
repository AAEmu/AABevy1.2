//! Item and skill slot interaction: picking up and dropping with the pointer, as in the client.
//!
//! * bag cells, equipment slots and the slots of the action bars and the skill window are drag sources and
//!   drop targets in the original scripts (`RegisterForDrag`, `OnDragStart` / `OnDragReceive`, `X2Bag:HandleClick`,
//!   `X2Equipment:PickupEquippedItem`); aa-ui routes the drags (`lua/drag.rs`) and `drag.lua` is what the client's
//!   C++ does behind those calls: the pointer holds the item (`X2Cursor`), a drop becomes a request;
//! * the requests (`HUD.drag_move`, `drag_equip`, `drag_unequip`, `drag_destroy`, `drag_action_set` /
//!   `drag_action_swap`) are queued here and become aa-net commands: `CSSwapItems` / `CSSplitBagItem` for the bag, the
//!   equipment and the bank, `CSDestroyItem`, `CSUpdateActionSlot` for the bars. The windows follow the server's
//!   answer (`SCItemTaskSuccess` folded into `Inventory`; the bag and the character window are told with
//!   `BAG_UPDATE` / `UNIT_EQUIPMENT_CHANGED`), the bars change at once;
//! * which equipment slots an item fits comes from the database (`item_armors`, `item_accessories`, the holdable of a
//!   weapon), with the server's table of slots per slot type.
//!
//! Scripted for unattended runs: `--hud-drag X1 Y1 X2 Y2 T` presses the left button at physical pixel X1, Y1, T seconds after
//! the HUD is up, moves to X2, Y2 in a few frames and lets go; `--hud-shift` holds Shift down for the whole run.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use aa_net::items::{container, ActionKind, ActionSlot, ItemMove};
use aa_net::CofferCommand;
use aa_net::model::slot as worn;
use aa_net::state::{ActionBar, Inventory};
use aa_net::{Command, Event, NetCommand, NetEvent};
use aa_ui::{UiRuntime, UiVirtualPointer};
use bevy::prelude::*;
use mlua::{Lua, Table};

use super::HudPointerOverUi;
use crate::Screen;

const DRAG_LUA: &str = include_str!("drag.lua");

/// What a script asked for, with slots 0 based (the scripts count from 1).
#[derive(Clone, Debug, PartialEq)]
enum Req {
    /// Move the item in `from` onto `to` (a swap, a merge, a move to an empty slot); `count` > 0: only that many of a stack.
    Move { from: (u8, usize), to: (u8, usize), count: i32 },
    /// Wear the bag item in `bag` in the worn slot `equip`.
    Equip { bag: usize, equip: usize },
    /// Take the worn item in `equip` off into the first free bag slot.
    Unequip { equip: usize },
    Destroy { container: u8, slot: usize, count: i32 },
    /// Put a skill / an item type into an action bar slot, `None` empties it.
    ActionSet { slot: usize, action: Option<ActionSlot> },
    ActionSwap { a: usize, b: usize },
}

/// The queue the Lua functions fill, kept in the Lua state's app data for the system to find.
#[derive(Clone, Default)]
struct Queue(Rc<RefCell<Vec<Req>>>);

/// Shift is down (`X2Input:IsShiftKeyDown()` is asked from Lua, outside the systems).
static SHIFT: AtomicBool = AtomicBool::new(false);

pub fn plugin(app: &mut App) {
    app.insert_resource(ScriptedDrags::from_args())
        // right after the HUD decided whether the pointer is over it, before the game looks at the mouse
        .add_systems(PreUpdate, hold_world.after(super::input::pointer_over_ui).in_set(super::cinema::CinemaInputRead).run_if(in_state(Screen::InGame)))
        .add_systems(
            Update,
            (track_shift, scripted_drags, bag_events, apply_requests, confirmations)
                .chain()
                .after(super::sync_hud)
                .run_if(super::hud_up)
                .run_if(in_state(Screen::InGame)),
        );
}

/// Install the functions the drag script calls (into the `HUD` table, before `ingame.lua`).
pub fn install(lua: &Lua, hud: &Table) -> mlua::Result<()> {
    start_equip_table();
    register_functions(lua, hud)
}

/// Run the drag script: it replaces the bag's and the equipment's click handling of `ingame.lua`, so it comes after it.
pub fn load_script(lua: &Lua) -> mlua::Result<()> {
    lua.load(DRAG_LUA).set_name("@drag.lua").exec()
}

fn register_functions(lua: &Lua, hud: &Table) -> mlua::Result<()> {
    let queue = Queue::default();
    lua.set_app_data(queue.clone());
    let push = |q: &Queue, r: Req| q.0.borrow_mut().push(r);
    let slot0 = |n: usize| n.saturating_sub(1);
    let q = queue.clone();
    hud.raw_set(
        "drag_move",
        lua.create_function(move |_, (fc, fs, tc, ts, count): (u8, usize, u8, usize, Option<i32>)| {
            push(&q, Req::Move { from: (fc, slot0(fs)), to: (tc, slot0(ts)), count: count.unwrap_or(0) });
            Ok(())
        })?,
    )?;
    let q = queue.clone();
    hud.raw_set(
        "drag_equip",
        lua.create_function(move |_, (bag, equip): (usize, usize)| {
            push(&q, Req::Equip { bag: slot0(bag), equip: slot0(equip) });
            Ok(())
        })?,
    )?;
    let q = queue.clone();
    hud.raw_set(
        "drag_unequip",
        lua.create_function(move |_, equip: usize| {
            push(&q, Req::Unequip { equip: slot0(equip) });
            Ok(())
        })?,
    )?;
    let q = queue.clone();
    hud.raw_set(
        "drag_destroy",
        lua.create_function(move |_, (c, slot, count): (u8, usize, i32)| {
            push(&q, Req::Destroy { container: c, slot: slot0(slot), count });
            Ok(())
        })?,
    )?;
    let q = queue.clone();
    hud.raw_set(
        "drag_action_set",
        lua.create_function(move |_, (slot, kind, id): (usize, Option<String>, Option<u32>)| {
            let action = match (kind.as_deref(), id) {
                (Some("skill"), Some(id)) => Some(ActionSlot { kind: ActionKind::Spell, id: u64::from(id) }),
                (Some("item"), Some(id)) => Some(ActionSlot { kind: ActionKind::ItemType, id: u64::from(id) }),
                _ => None,
            };
            push(&q, Req::ActionSet { slot: slot0(slot), action });
            Ok(())
        })?,
    )?;
    let q = queue;
    hud.raw_set(
        "drag_action_swap",
        lua.create_function(move |_, (a, b): (usize, usize)| {
            push(&q, Req::ActionSwap { a: slot0(a), b: slot0(b) });
            Ok(())
        })?,
    )?;
    hud.raw_set("shift_down", lua.create_function(|lua, ()| {
        if SHIFT.load(Ordering::Relaxed) { return Ok(true); }
        // UI capture consumes Bevy buttons, while native modifiers keep the physical key state.
        let input: Option<Table> = lua.globals().get("X2Input")?;
        let Some(input) = input else { return Ok(false) };
        let f: mlua::Function = input.get("IsShiftKeyDown")?;
        f.call::<bool>(input)
    })?)?;
    // where an item can be worn: the equipment slots (`ES_*`, 1 based) in the order they are tried, nil for other items
    hud.raw_set(
        "equip_slots",
        lua.create_function(|lua, template: u32| {
            let Some(slots) = equip_table().and_then(|t| t.get(&template)).map(|st| allowed_slots(*st)) else { return Ok(None) };
            if slots.is_empty() {
                return Ok(None);
            }
            Ok(Some(lua.create_sequence_from(slots.iter().map(|s| *s as i64 + 1))?))
        })?,
    )?;
    Ok(())
}

// ---- where items can be worn -----------------------------------------------------------------------------------

/// `EquipmentItemSlotType` per item template, read from the game database on a thread.
static EQUIP: OnceLock<HashMap<u32, u32>> = OnceLock::new();
static EQUIP_STARTED: AtomicBool = AtomicBool::new(false);

fn equip_table() -> Option<&'static HashMap<u32, u32>> {
    EQUIP.get()
}

fn start_equip_table() {
    if EQUIP_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    crate::task::spawn("hud-equip-slots", || {
            let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
            let _ = BACKPACKS.set(load_backpack_kinds(&path).unwrap_or_default());
            match load_equip_table(&path) {
                Ok(t) => {
                    debug!("hud: {} items can be worn", t.len());
                    let _ = EQUIP.set(t);
                }
                Err(e) => {
                    warn!("hud: equipment slots from {path}: {e}");
                    let _ = EQUIP.set(HashMap::new());
                }
            }
        });
}

/// `item_backpacks.backpack_type_id` per backpack template (2 is a glider), read with the equipment slots.
static BACKPACKS: OnceLock<HashMap<u32, u32>> = OnceLock::new();

/// The backpack kind of a template (`None`: not a backpack, or the table is not loaded yet).
fn backpack_kind(template: u32) -> Option<u32> {
    #[cfg(test)]
    if let Some(kind) = TEST_KINDS.with(|k| k.borrow().get(&template).copied()) {
        return Some(kind);
    }
    BACKPACKS.get()?.get(&template).copied()
}

#[cfg(test)]
thread_local! {
    static TEST_KINDS: std::cell::RefCell<HashMap<u32, u32>> = Default::default();
}

const BACKPACK_TYPE_GLIDER: u32 = 2;
/// `ErrorMessageType::CannotChangeBackpackInGliding`.
const CANNOT_CHANGE_BACKPACK_IN_GLIDING: i16 = 543;

/// `FUN_394e41f0`: wearing a glider in the back slot while the worn backpack is a glider too raises 543 during a flight.
/// (A worn backpack of another kind raises `BACKPACK_OCCUPIED` 315 first, flying or not; that one is not checked here.)
fn glider_swap_refused(inventory: &Inventory, mv: &ItemMove, kind: impl Fn(u32) -> Option<u32>) -> bool {
    let back = (container::EQUIPMENT, worn::BACKPACK);
    let from = (mv.from_container, usize::from(mv.from_slot));
    let to = (mv.to_container, usize::from(mv.to_slot));
    let other = if from == back {
        to
    } else if to == back {
        from
    } else {
        return false;
    };
    let glider = |place: (u8, usize)| inventory.item_at(place.0, place.1).and_then(|i| kind(i.template_id)) == Some(BACKPACK_TYPE_GLIDER);
    glider(back) && glider(other)
}

fn load_backpack_kinds(path: &str) -> rusqlite::Result<HashMap<u32, u32>> {
    let conn = rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    let mut q = conn.prepare("SELECT item_id, backpack_type_id FROM item_backpacks")?;
    let mut rows = q.query([])?;
    let mut out = HashMap::new();
    while let Some(r) = rows.next()? {
        let (item, kind): (i64, i64) = (r.get(0)?, r.get(1)?);
        out.insert(item as u32, kind as u32);
    }
    Ok(out)
}

fn load_equip_table(path: &str) -> rusqlite::Result<HashMap<u32, u32>> {
    let conn = rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    let mut out = HashMap::new();
    // armour and accessories name their slot type, a weapon its holdable's, a backpack is one slot type
    for sql in [
        "SELECT item_id, slot_type_id FROM item_armors",
        "SELECT item_id, slot_type_id FROM item_accessories",
        "SELECT w.item_id, h.slot_type_id FROM item_weapons w JOIN holdables h ON h.id = w.holdable_id",
        "SELECT item_id, 30 FROM item_backpacks",
    ] {
        let mut q = conn.prepare(sql)?;
        let mut rows = q.query([])?;
        while let Some(r) = rows.next()? {
            let (item, slot_type): (i64, i64) = (r.get(0)?, r.get(1)?);
            out.insert(item as u32, slot_type as u32);
        }
    }
    Ok(out)
}

/// The worn slots (`aa_net::model::slot`) an `EquipmentItemSlotType` fits, as the server's `allowed_gear_slots`.
fn allowed_slots(slot_type: u32) -> &'static [usize] {
    match slot_type {
        1 => &[worn::HEAD],
        2 => &[worn::NECK],
        3 => &[worn::CHEST],
        4 => &[worn::WAIST],
        5 => &[worn::LEGS],
        6 => &[worn::HANDS],
        7 => &[worn::FEET],
        8 => &[worn::ARMS],
        9 => &[worn::BACK],
        10 => &[worn::EAR1, worn::EAR2],
        11 => &[worn::FINGER1, worn::FINGER2],
        12 => &[worn::UNDERSHIRT],
        13 => &[worn::UNDERPANTS],
        14 | 16 => &[worn::MAINHAND],
        15 | 20 => &[worn::OFFHAND],
        17 => &[worn::MAINHAND, worn::OFFHAND],
        18 => &[worn::RANGED],
        21 => &[worn::MUSICAL],
        30 => &[worn::BACKPACK],
        31 => &[worn::COSPLAY],
        _ => &[],
    }
}

// ---- systems ---------------------------------------------------------------------------------------------------

fn track_shift(keys: Res<ButtonInput<KeyCode>>, script: Res<ScriptedDrags>) {
    SHIFT.store(script.shift || keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight), Ordering::Relaxed);
}

/// The bag and the character window redraw when the server changed what is in them: `BAG_UPDATE` with -1, -1 (everything)
/// makes the bag update, and the character window's equipment too. Runs after the HUD has pushed the new lists.
fn bag_events(rt: Option<NonSend<UiRuntime>>, inventory: Res<Inventory>) {
    let Some(rt) = rt else { return };
    if inventory.is_changed() {
        rt.fire_event("BAG_UPDATE", (-1i64, -1i64));
    }
}

/// While the pointer holds something the game world does not get the mouse (no walking, no targeting); a left click on
/// the world is the drop there (the item asks to be destroyed, an action bar icon leaves the bar).
pub(super) fn hold_world(mouse: Res<ButtonInput<MouseButton>>, mut over: ResMut<HudPointerOverUi>, rt: Option<NonSend<UiRuntime>>) {
    let Some(rt) = rt else { return };
    if !rt.holding() {
        return;
    }
    if mouse.just_pressed(MouseButton::Left) && !over.0 {
        rt.drop_on_nothing(false);
    }
    over.0 = true;
}

/// The scripts' requests as aa-net commands.
pub(super) fn apply_requests(
    rt: Option<NonSend<UiRuntime>>,
    bridge: NonSend<super::api::HudBridge>,
    inventory: Res<Inventory>,
    bar: Res<ActionBar>,
    gliding: Option<Res<crate::ingame::glide::Gliding>>,
    mut errors: Option<MessageWriter<super::ShowError>>,
    mut net: MessageWriter<NetCommand>,
) {
    let Some(rt) = rt else { return };
    let Some(queue) = rt.lua.app_data_ref::<Queue>().map(|q| q.clone()) else { return };
    let requests = std::mem::take(&mut *queue.0.borrow_mut());
    // The page the scripts set is visible at once; the ActionBar resource follows through
    // handle_requests.  Use the scripts' page so a page change and a drop in the same frame
    // cannot send the drop to the previous page's slot.
    let page = bridge.0.borrow().action_page;
    let flying = gliding.as_deref().is_some_and(|g| g.flying());
    let mut glider_swap = |mv: &ItemMove| {
        let refused = flying && glider_swap_refused(&inventory, mv, backpack_kind);
        if refused {
            debug!("hud: drag: a glider cannot be swapped while gliding");
            if let Some(errors) = errors.as_mut() {
                errors.write(super::ShowError(CANNOT_CHANGE_BACKPACK_IN_GLIDING));
            }
        }
        refused
    };
    for req in requests {
        match req {
            Req::Move { from, to, count } => {
                let Some(mv) = inventory.plan_move(from, (to.0, to.1)) else {
                    debug!("hud: drag: nothing to move from {from:?} to {to:?}");
                    continue;
                };
                let stack = inventory.item_at(from.0, from.1).map_or(0, |i| i.count);
                if from.0 == container::TRADE || to.0 == container::TRADE {
                    info!("hud: drag: move item {} between the bag and the coffer ({from:?} to {to:?})", mv.from_item_id);
                    let count = if count < stack { count } else { 0 };
                    net.write(NetCommand(Command::Coffer(CofferCommand::moving(mv, count, inventory.coffer.container_id))));
                } else if count > 0 && count < stack {
                    info!("hud: drag: split {count} of item {} from {from:?} to {to:?}", mv.from_item_id);
                    net.write(NetCommand(Command::SplitItem { mv, count }));
                } else if glider_swap(&mv) {
                    continue;
                } else {
                    info!("hud: drag: move item {} from {from:?} to {to:?} (target item {})", mv.from_item_id, mv.to_item_id);
                    net.write(NetCommand(Command::SwapItems(mv)));
                }
            }
            Req::Equip { bag, equip } => match inventory.plan_equip(bag, equip) {
                Some(mv) if glider_swap(&mv) => {}
                Some(mv) => {
                    info!("hud: drag: wear item {} (bag slot {bag}) in worn slot {equip} (worn item {})", mv.to_item_id, mv.from_item_id);
                    net.write(NetCommand(Command::SwapItems(mv)));
                }
                None => debug!("hud: drag: no item in bag slot {bag}"),
            },
            Req::Unequip { equip } => {
                let Some(free) = inventory.first_free_bag_slot() else {
                    debug!("hud: drag: the bag is full, cannot take off worn slot {equip}");
                    continue;
                };
                if let Some(mv) = inventory.plan_unequip(equip, free) {
                    info!("hud: drag: take off item {} (worn slot {equip}) into bag slot {free}", mv.from_item_id);
                    net.write(NetCommand(Command::SwapItems(mv)));
                }
            }
            Req::Destroy { container: c, slot, count } => {
                let Some(item) = inventory.item_at(c, slot) else { continue };
                info!("hud: drag: destroy {count} of item {} ({c}:{slot})", item.id);
                net.write(NetCommand(Command::DestroyItem { item_id: item.id, container: c, slot: slot as u8, count }));
            }
            Req::ActionSet { slot, action } => {
                let Some(slot) = ActionBar::display_slot_on(page, slot) else {
                    debug!("hud: drag: action bar slot {slot} is off the bar");
                    continue;
                };
                info!("hud: drag: action bar slot {slot} = {action:?}");
                net.write(NetCommand(Command::SetActionSlot { slot: slot as u8, action: action.unwrap_or_default() }));
            }
            Req::ActionSwap { a, b } => {
                let (Some(a), Some(b)) = (ActionBar::display_slot_on(page, a), ActionBar::display_slot_on(page, b)) else {
                    debug!("hud: drag: action bar slots {a} and {b} are off the bar");
                    continue;
                };
                let at = |i: usize| bar.slots.get(i).copied().unwrap_or_default();
                info!("hud: drag: action bar slots {a} and {b} change places ({:?} / {:?})", at(a), at(b));
                net.write(NetCommand(Command::SetActionSlot { slot: a as u8, action: at(b) }));
                net.write(NetCommand(Command::SetActionSlot { slot: b as u8, action: at(a) }));
            }
        }
    }
}

/// The server's answers to the moves, in the log (reasons: 1 destroy, 5 swap, 6 split).
fn confirmations(mut events: MessageReader<NetEvent>) {
    for NetEvent(event) in events.read() {
        match event {
            Event::ItemsChanged(changes) if matches!(changes.reason, 1 | 5 | 6) => {
                let what = match changes.reason {
                    1 => "destroy",
                    5 => "move / swap",
                    _ => "split",
                };
                info!("hud: server confirmed the item {what}: {:?}", changes.tasks);
            }
            Event::ServerError { code } => warn!("hud: server error {code}"),
            _ => {}
        }
    }
}

// ---- scripted drags --------------------------------------------------------------------------------------------

/// Frames a scripted drag takes to move from one end to the other.
const DRAG_STEPS: u32 = 6;

/// `--hud-drag X1 Y1 X2 Y2 T`: a left button drag from physical pixel X1, Y1 to X2, Y2, T seconds after the HUD is up.
#[derive(Resource, Default)]
pub struct ScriptedDrags {
    /// (time, from, to), the next one last.
    drags: Vec<(f32, Vec2, Vec2)>,
    run: Option<(Vec2, Vec2, u32)>,
    started: Option<f32>,
    /// `--hud-shift`: Shift is down.
    shift: bool,
}

impl ScriptedDrags {
    fn from_args() -> ScriptedDrags {
        let args: Vec<String> = aa_dev::args();
        let num = |i: usize| args.get(i).and_then(|v| v.parse::<f32>().ok());
        let mut s = ScriptedDrags { shift: args.iter().any(|a| a == "--hud-shift"), ..Default::default() };
        for (i, a) in args.iter().enumerate() {
            if a == "--hud-drag"
                && let (Some(x1), Some(y1), Some(x2), Some(y2), Some(t)) = (num(i + 1), num(i + 2), num(i + 3), num(i + 4), num(i + 5))
            {
                s.drags.push((t, Vec2::new(x1, y1), Vec2::new(x2, y2)));
            }
        }
        s.drags.sort_by(|a, b| b.0.total_cmp(&a.0));
        s
    }
}

pub(super) fn scripted_drags(time: Res<Time>, mut script: ResMut<ScriptedDrags>, mut pointer: ResMut<UiVirtualPointer>) {
    let now = time.elapsed_secs();
    let started = *script.started.get_or_insert(now);
    if script.run.is_none()
        && script.drags.last().is_some_and(|d| now - started >= d.0)
        && let Some((_, from, to)) = script.drags.pop()
    {
        info!("hud: --hud-drag {from} -> {to} at {:.1} s", now - started);
        script.run = Some((from, to, 0));
    }
    let Some((from, to, frame)) = script.run.as_mut() else { return };
    match *frame {
        // hover first, then press, then move, then let go
        0 => pointer.pos = Some(*from),
        1 => {
            pointer.pos = Some(*from);
            pointer.press = true;
        }
        f if f <= 1 + DRAG_STEPS => pointer.pos = Some(from.lerp(*to, (f - 1) as f32 / DRAG_STEPS as f32)),
        _ => {
            pointer.pos = Some(*to);
            pointer.release = true;
            script.run = None;
            return;
        }
    }
    *frame += 1;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_types_fit_the_servers_worn_slots() {
        assert_eq!(allowed_slots(1), &[worn::HEAD]);
        assert_eq!(allowed_slots(10), &[worn::EAR1, worn::EAR2]);
        assert_eq!(allowed_slots(17), &[worn::MAINHAND, worn::OFFHAND]);
        assert_eq!(allowed_slots(16), &[worn::MAINHAND]);
        assert!(allowed_slots(99).is_empty());
    }

    fn backpack(template_id: u32, id: u64) -> Option<aa_net::model::Item> {
        Some(aa_net::model::Item { template_id, id, count: 1, ..Default::default() })
    }

    /// Worn glider 1001 and, in the bag, glider 1002 (slot 3) and a plain backpack 1003 (slot 4).
    fn glider_inventory() -> Inventory {
        let mut inventory = Inventory::default();
        inventory.equipment.slots[worn::BACKPACK] = backpack(1001, 100);
        inventory.bag = vec![None, None, None, backpack(1002, 101), backpack(1003, 102), None];
        inventory
    }

    /// Backpack kinds of the fixture: 1001 and 1002 are gliders (type 2), 1003 a plain backpack (type 3).
    fn kind(template: u32) -> Option<u32> {
        match template {
            1001 | 1002 => Some(2),
            1003 => Some(3),
            _ => None,
        }
    }

    #[test]
    fn a_glider_swap_is_refused_only_between_two_gliders() {
        let inventory = glider_inventory();
        let swap = |bag| inventory.plan_equip(bag, worn::BACKPACK).unwrap();
        assert!(glider_swap_refused(&inventory, &swap(3), kind), "glider for glider");
        assert!(!glider_swap_refused(&inventory, &swap(4), kind), "a plain backpack is not this check");
        let unrelated = inventory.plan_move((container::INVENTORY, 3), (container::INVENTORY, 5)).unwrap();
        assert!(!glider_swap_refused(&inventory, &unrelated, kind));
    }

    #[allow(clippy::field_reassign_with_default)] // the other fields are private
    fn flying() -> crate::ingame::glide::Gliding {
        use crate::ingame::glide::{Active, Flight, GlideParams, Gliding, Pose};
        let mut gliding = Gliding::default();
        gliding.active = Some(Active {
            flight: Flight::start(GlideParams::EXPERIMENTAL),
            buff_id: 1029,
            buff_index: 0,
            pose: Pose::Start,
            touched: false,
            touched_for: 0.0,
            missing: 0.0,
            water: false,
            deck: None,
        });
        gliding
    }

    fn equip_app(in_flight: bool) -> App {
        TEST_KINDS.with(|k| *k.borrow_mut() = HashMap::from([(1001, 2), (1002, 2), (1003, 3)]));
        let rt = UiRuntime::new("en_us", 1280.0, 720.0).unwrap();
        let hud = rt.lua.create_table().unwrap();
        register_functions(&rt.lua, &hud).unwrap();
        rt.lua.globals().set("HUD", hud).unwrap();
        rt.lua.load(format!("HUD.drag_equip(4, {})", worn::BACKPACK + 1)).exec().unwrap();
        let bridge = crate::hud::api::HudBridge(Rc::new(std::cell::RefCell::new(crate::hud::api::HudData::default())));
        let mut app = App::new();
        app.insert_non_send(rt)
            .insert_non_send(bridge)
            .insert_resource(ActionBar::default())
            .insert_resource(glider_inventory())
            .add_message::<NetCommand>()
            .add_message::<super::super::ShowError>()
            .add_systems(Update, apply_requests);
        if in_flight {
            app.insert_resource(flying());
        }
        app
    }

    #[test]
    fn the_glider_swap_while_flying_raises_543_and_sends_nothing() {
        let mut app = equip_app(true);
        app.update();
        assert!(app.world_mut().resource_mut::<Messages<NetCommand>>().drain().next().is_none());
        let shown: Vec<_> = app.world_mut().resource_mut::<Messages<super::super::ShowError>>().drain().map(|e| e.0).collect();
        assert_eq!(shown, [543]);
    }

    #[test]
    fn the_same_swap_on_the_ground_is_sent() {
        let mut app = equip_app(false);
        app.update();
        assert_eq!(app.world_mut().resource_mut::<Messages<NetCommand>>().drain().count(), 1);
        assert!(app.world_mut().resource_mut::<Messages<super::super::ShowError>>().drain().next().is_none());
    }

    #[test]
    fn the_quest_tracker_uses_the_first_bag_or_worn_item_of_a_type() {
        let rt = UiRuntime::new("en_us", 1280.0, 720.0).unwrap();
        rt.lua
            .load(
                r#"
                X2Bag, X2Equipment = {}, {}
                X2Cursor = { GetHeld = function() return nil end }
                ISLOT_BAG = 2
                USED = nil
                USE_OK = true
                AAUI = { UseSlot = function(kind, slot, button) USED = { kind, slot, button }; return USE_OK end }
                ITEM_IMPL = 22
                HUD = { bag_slots = 4, bag = { [2] = { id = 777, count = 1 } }, equipment = { [4] = { id = 777 } },
                        item_impl = function(id) return ITEM_IMPL end,
                        requests = {}, request = function(kind, slot) HUD.requests[#HUD.requests + 1] = { kind, slot }; return true end }
                "#,
            )
            .exec()
            .unwrap();
        load_script(&rt.lua).unwrap();
        rt.lua
            .load(
                r#"
                assert(X2Bag:FindFirstBagSlotItemByItemType(777) == 2)
                assert(X2Bag:FindFirstBagSlotItemByItemType(9) == nil)
                assert(X2Bag:UseItemByType(777) == true)
                assert(USED[1] == ISLOT_BAG and USED[2] == 2 and USED[3] == "RightButton")
                assert(X2Bag:UseItemByType(9) == false)
                USE_OK = false
                assert(X2Bag:UseItemByType(777) == false, 'a use that cannot be issued is false')
                assert(X2Equipment:FindEquippedItemByType(777) == 4)
                assert(X2Equipment:UseEquippedItemByType(777) == true)
                assert(HUD.requests[1][1] == "use_equip" and HUD.requests[1][2] == 4)
                assert(X2Equipment:UseEquippedItemByType(9) == false)
                ITEM_IMPL = 5
                assert(X2Equipment:UseEquippedItemByType(777) == false, 'FUN_3946e770 refuses an impl it cannot use worn')
                assert(#HUD.requests == 1, 'and sends nothing')
                "#,
            )
            .exec()
            .unwrap();
    }

    #[test]
    fn requests_arrive_with_slots_counted_from_zero() {
        let lua = Lua::new();
        let hud = lua.create_table().unwrap();
        register_functions(&lua, &hud).unwrap();
        lua.globals().set("HUD", hud).unwrap();
        lua.load("HUD.drag_move(2, 3, 2, 8, 4); HUD.drag_equip(5, 16); HUD.drag_unequip(16); HUD.drag_action_set(1, 'skill', 18132); HUD.drag_action_set(2, nil)")
            .exec()
            .unwrap();
        let got = lua.app_data_ref::<Queue>().unwrap().0.borrow().clone();
        assert_eq!(
            got,
            vec![
                Req::Move { from: (2, 2), to: (2, 7), count: 4 },
                Req::Equip { bag: 4, equip: 15 },
                Req::Unequip { equip: 15 },
                Req::ActionSet { slot: 0, action: Some(ActionSlot { kind: ActionKind::Spell, id: 18132 }) },
                Req::ActionSet { slot: 1, action: None },
            ]
        );
    }

    #[test]
    fn a_page_slot_sends_the_original_global_slot_and_bytes() {
        use std::cell::RefCell;
        use std::rc::Rc;
        // The original's real page-cell drops: first cell of pages 1..4 -> slots 1/49/61/73.
        for (page, want_slot, want_body) in [
            (0u8, 1u8, [0x01, 0x02, 0xd4, 0x46, 0x00, 0x00]),
            (1, 49, [0x31, 0x02, 0xd4, 0x46, 0x00, 0x00]),
            (2, 61, [0x3d, 0x02, 0xd4, 0x46, 0x00, 0x00]),
            (3, 73, [0x49, 0x02, 0xd4, 0x46, 0x00, 0x00]),
        ] {
            let rt = UiRuntime::new("en_us", 1280.0, 720.0).unwrap();
            let hud = rt.lua.create_table().unwrap();
            register_functions(&rt.lua, &hud).unwrap();
            rt.lua.globals().set("HUD", hud).unwrap();
            rt.lua.load("HUD.drag_action_set(1, 'skill', 18132)").exec().unwrap();
            let mut bar = ActionBar::default();
            assert!(bar.set_page(page));
            let bridge = crate::hud::api::HudBridge(Rc::new(RefCell::new(crate::hud::api::HudData {
                action_page: page,
                ..Default::default()
            })));
            let mut app = App::new();
            app.insert_non_send(rt)
                .insert_non_send(bridge)
                .insert_resource(bar)
                .init_resource::<Inventory>()
                .add_message::<NetCommand>()
                .add_systems(Update, apply_requests);
            app.update();
            let commands: Vec<_> = app.world_mut().resource_mut::<Messages<NetCommand>>().drain().map(|c| c.0).collect();
            let [Command::SetActionSlot { slot, action }] = &commands[..] else { panic!("{commands:?}") };
            assert_eq!(*slot, want_slot, "page {} cell 1", page + 1);
            assert_eq!((action.kind, action.id), (ActionKind::Spell, 18132));
            let body = aa_net::game::ClientMsg::UpdateActionSlot { slot: *slot, action: *action }.encode_body();
            assert_eq!(body, want_body, "page {} cell 1 packet body", page + 1);
        }
    }
}
