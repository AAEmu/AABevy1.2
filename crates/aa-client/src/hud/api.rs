//! The game API of the in-game scripts: the `X2*` objects they call, backed by a [`Snapshot`] of
//! aa-net's state that the app pushes into the Lua state (`HUD.state`), and the requests they queue
//! (`HUD.request`).
//!
//! Layers, in the order they are installed into the stage's Lua state:
//!
//! 1. [`crate::login::gameapi::install`]: every `X2*` object as a permissive stub, the native
//!    constants.
//! 2. **FALLBACK**: `aa-ui/examples/fake/fake_ingame.lua`, the fake in-game data of the reference
//!    capture. Everything aa-net does not provide yet is still read from it.
//! 3. `ingame.lua`: the real data. It overrides the fake, function by function, as the data lands.

use std::cell::RefCell;
use std::rc::Rc;
use web_time::Instant;

use aa_net::model::exp;
use aa_net::state::{LABOR_MAX, Player, PlayerStats};
use aa_net::{Command, NetCommand};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use mlua::{Lua, MultiValue, Table, Value};

use super::data::{SharedGameData, Snapshot};
use super::panels::{self, Panels};
use super::usable::Env;
use crate::login::gameapi;

/// What a script asked the app to do (queued by `HUD.request(kind, ...)`).
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// `HUD.request("chat", channel, message, target)`: a line typed into the chat input.
    Chat { chat_type: i16, message: String, target: String },
    /// `HUD.request("leave", how)`: `X2World:LeaveWorld(EXIT_*)`.
    Leave { how: i64 },
    /// `HUD.request("leave_cancel")`: `X2World:CancelLeaveWorld()`.
    CancelLeave,
    /// `HUD.request("use_action", slot)`: a click on action bar slot `slot` (1 based).
    UseAction {
        slot: usize,
    },
    UseModeAction { slot: usize },
    /// `HUD.request("use_pet_action", slot)`: a click or hotkey on pet action slot `slot` (1 based).
    UsePetAction { slot: usize },
    /// `X2Action:SetActionBarPage(page)`: switch the action bar to page `page` (0 based).
    SetActionBarPage { page: u8 },
    /// `HUD.request("use_bag", slot)`: a right click on the bag item in real slot `slot` (1 based).
    UseBag { slot: usize },
    /// `HUD.request("resurrect", inPlace)`: the death window's buttons (`X2Player:Resurrect(inPlace)`).
    Resurrect { in_place: bool },
    LearnSkill { id: u32 },
    LearnBuff { id: u32 },
    ResetSkills { ability: u8 },
    SwapAbility { old: u8, ability: u8, npc_token: Option<String> },
    Party(aa_net::party::PartyCommand),
    Friend(aa_net::friend::FriendCommand),
    Target { obj_id: u32 },
    /// `X2Store:SellBackPackGoods()`: sell the equipped trade pack to the interaction NPC (not the target).
    SellBackpack { obj_id: u32 },
    /// A chat command with a packet of its own: the dice, player channels (`chat/commands.rs`).
    ChatCommand(aa_net::ChatCommand),
    /// A chat command that uses a skill (`/report Name`), on the unit of this name when one is given.
    Skill { skill: u32, target_name: String },
    /// `/target Name`: select the unit of this name in view.
    TargetByName { name: String },
    /// A received slave binding request (`SlaveCommand::{Bind,Unbind}`).
    Slave(aa_net::slave_control::SlaveCommand),
    /// `CSStartSkill` for an interaction skill whose target is a unit (the slave's 13863 passenger
    /// seat): a plain unit-caster request, with no learned-skill or action-bar gate. The server
    /// owns the skill's requirements.
    UseSkill { skill_id: u32, target_obj_id: u32 },
    ItemEffect { character_id: u32, request: aa_net::item_effect::ItemEffectRequest },
    ItemLook { character_id: u32, request: aa_net::look_conversion::ItemLookRequest },
    /// `CSStartInteraction` 0x068, for an NPC's function or a slave's interaction skill list.
    Interact(aa_net::interaction::Interact),
}

/// Edge-to-edge distance on the ground plane, the server's `distance_to(..., include_z = false)`:
/// the x/y centre distance minus both units' scaled actor-model radii, clamped at 0. The target
/// frame shows this and the loot reach is measured with it.
pub(in crate::hud) fn distance_to(from: Vec3, from_radius: f32, to: Vec3, to_radius: f32) -> f32 {
    let (dx, dy) = (from.x - to.x, from.y - to.y);
    (dx.hypot(dy) - from_radius - to_radius).max(0.0)
}

/// The state shared between the Bevy systems and the Lua closures (single threaded).
pub struct HudData {
    pub snapshot: Snapshot,
    /// What the Lua state has seen last.
    pub pushed: Option<Snapshot>,
    /// The action bars, bag, buffs and target, and what the Lua state has seen of them.
    pub panels: Panels,
    pub panels_pushed: Option<Panels>,
    /// Game position of the target and its scaled actor-model radius.
    pub target_pos: Option<Vec3>,
    pub target_radius: f32,
    /// The player's scaled actor-model radius (the server's `Unit.ModelSize`).
    pub player_radius: f32,
    /// The HUD's clock starts here (`HUD.now_ms()`).
    pub start: Instant,
    /// The leave target the server counts down to / granted (`LeaveTarget` number).
    pub leaving: Option<u8>,
    /// The connection dropped and the disconnect dialog was shown.
    pub disconnected: bool,
    /// Current action bar page, 0 based (the scripts count from 1).
    pub action_page: u8,
    pub game: SharedGameData,
    pub requests: Vec<Request>,
    /// The local player's entity (found by its name once), its game position and facing.
    pub player_entity: Option<Entity>,
    pub pos: Vec3,
    pub yaw: f32,
    /// The slave whose point-1 seat is a turret (`ingame::LocalMount::turret_seat`): its bar
    /// shows the point-1 attached skill. `None` on foot and on a drivable vehicle.
    pub turret_seat: Option<u32>,
    /// The manned cannon's clamped aim and ground point (`ingame::turret::Sight`): a fire skill
    /// with a BallisticPos target goes out with this spot instead of the selected unit.
    pub turret: Option<crate::ingame::turret::Sight>,
    /// The ranks of the learned skills (`Skills::known`), for the skill tooltips.
    pub skill_levels: std::collections::HashMap<u32, u8>,
    pub passive_buffs: Vec<u32>,
    /// Timed received cooldowns and cast observations; read at widget tick time.
    pub cooldowns: aa_net::state::Skills,
    pub input_modes: crate::settings::InputModes,
    /// Received initial-config bytes1/8; unknown differs from a received zero.
    pub feature_limits: (Option<u8>, Option<u8>),
    /// Received feature byte4/mask0x40. Missing configuration does not admit the shop.
    pub in_game_shop_enabled: Option<bool>,
    /// The client's own combat state: the server only ever says "cleared" (`CombatChanged`); the client
    /// enters combat itself when it attacks or is attacked (`combat::route_combat`).
    pub local_combat: bool,
    pub(super) pvp: super::pvp::View,
    pub(super) radar: super::radar::View,
    pub(super) battlefield: super::battlefield::View,
    pub(super) breath: super::breath::View,
    pub(super) map_overlays: super::map_overlays::View,
    pub(super) expansion: super::expansion::View,
    pub(super) repair: super::repair::View,
    pub(super) crafting: super::crafting::View,
    pub(super) user_music: super::user_music::View,
    pub(super) mode_actions: super::mode_actions::View,
    pub(super) mounted: super::mounted::View,
    pub(super) pets: super::pets::View,
    /// The casts the cast bars show (`cast.rs`).
    pub casts: super::cast::Casts,
    /// The loot window's rows and what it asked for (`loot.rs`).
    pub loot: super::loot::LootView,
    /// The NPC talked to, the quest dialog and the quest rows the scripts read (`dialog.rs`).
    pub dialog: super::dialog::DialogView,
    /// The player is away from the keyboard ("-Away-" under the name plate, `plates::track_idle`).
    pub afk: bool,
    /// When the player may stand up again after dying (HUD clock, ms): `X2Player:GetResurrectionCountdown` counts down to it.
    pub death_ends: std::cell::Cell<Option<u64>>,
    pub death_started: std::cell::Cell<Option<u64>>,
    /// The server offered an in-place resurrection (the caster may be the player below level 10).
    pub revive_caster: std::cell::Cell<Option<u32>>,
    /// The offered caster's name, for `X2Unit:GetUnitNameById` in the death window
    /// (`PLAYER_RESURRECTION`), which may name a unit outside the player's target or party.
    pub revive_caster_name: std::cell::RefCell<Option<String>>,
    /// The player's attack numbers and worn items, what the tooltips are worked out from (`tips.rs`).
    pub tips: super::tips::TipState,
    /// The `tips.sheet_gen` the character window was told about.
    pub stats_pushed: u32,
    pub party: aa_net::Party,
    pub friends: aa_net::Friends,
    pub(super) family: super::family::View,
    pub(super) expedition: super::expedition::View,
    pub(super) territory: super::territory::View,
    pub(super) achievement: super::achievement::View,
    pub(super) chat_config: super::chat::config::View,
    pub(super) crime: super::crime::View,
    /// Where the speech bubbles go on the screen and who the speakers are (`speech.rs`).
    pub(super) speech: super::speech::View,
    pub(super) appellation: super::appellation::View,
    pub(super) portal: super::portal::View,
    /// The doodad in reach, the bar and the interaction list (`doodad.rs`).
    pub(super) doodad: super::doodad::View,
    pub(super) dynamic_actions: super::dynamic_actions::View,
    pub(super) duel: super::duel::View,
    pub(super) trial: super::trial::View,
    pub(super) ranking: super::ranking::View,
    pub(super) buff_cancel: super::buffs::cancel::View,
    pub(super) item_effects: super::item_effects::View,
    pub(super) enchant: super::enchant::View,
    pub(super) look_conversion: super::look_conversion::View,
    pub party_pushed: Option<aa_net::Party>,
    pub character_id: u32,
    pub store: super::store::View,
    pub(super) specialty: super::specialty::View,
}

pub type Shared = Rc<RefCell<HudData>>;

/// The non-send resource holding the shared state.
pub struct HudBridge(pub Shared);

impl Default for HudData {
    fn default() -> Self {
        HudData {
            snapshot: Snapshot::default(),
            pushed: None,
            panels: Panels::default(),
            panels_pushed: None,
            target_pos: None,
            target_radius: 0.0,
            player_radius: 0.0,
            start: Instant::now(),
            leaving: None,
            disconnected: false,
            action_page: 0,
            game: SharedGameData::default(),
            requests: Vec::new(),
            player_entity: None,
            pos: Vec3::ZERO,
            yaw: 0.0,
            turret_seat: None,
            turret: None,
            skill_levels: Default::default(),
            passive_buffs: Vec::new(),
            cooldowns: Default::default(),
            input_modes: Default::default(),
            feature_limits: Default::default(),
            in_game_shop_enabled: None,
            local_combat: false,
            pvp: Default::default(),
            radar: Default::default(),
            battlefield: Default::default(),
            breath: Default::default(),
            map_overlays: Default::default(),
            expansion: Default::default(),
            repair: Default::default(),
            crafting: Default::default(),
            user_music: Default::default(),
            mode_actions: Default::default(),
            mounted: Default::default(),
            pets: Default::default(),
            casts: Default::default(),
            loot: Default::default(),
            dialog: Default::default(),
            afk: false,
            death_ends: Default::default(),
            death_started: Default::default(),
            revive_caster: Default::default(),
            revive_caster_name: Default::default(),
            tips: Default::default(),
            stats_pushed: 0,
            party: Default::default(),
            friends: Default::default(),
            family: Default::default(),
            expedition: Default::default(),
            territory: Default::default(),
            achievement: Default::default(),
            chat_config: Default::default(),
            crime: Default::default(),
            speech: Default::default(),
            appellation: Default::default(),
            portal: Default::default(),
            doodad: Default::default(),
            dynamic_actions: Default::default(),
            duel: Default::default(),
            trial: Default::default(),
            ranking: Default::default(),
            buff_cancel: Default::default(),
            item_effects: Default::default(),
            enchant: Default::default(),
            look_conversion: Default::default(),
            party_pushed: None,
            character_id: 0,
            store: Default::default(),
            specialty: Default::default(),
        }
    }
}

impl Default for HudBridge {
    fn default() -> Self {
        let data = HudData::default();
        data.game.load_in_background();
        HudBridge(Rc::new(RefCell::new(data)))
    }
}

/// Fake in-game game API of the aa-ui example (**FALLBACK** data): the HUD, then the data of the
/// windows players open, one file each (in the order the example installs them).
const FAKE_INGAME: [(&str, &str); 8] = [
    ("fake_ingame.lua", include_str!("../../../aa-ui/examples/fake/fake_ingame.lua")),
    ("fake_bag.lua", include_str!("../../../aa-ui/examples/fake/fake_bag.lua")),
    ("fake_character.lua", include_str!("../../../aa-ui/examples/fake/fake_character.lua")),
    ("fake_skill.lua", include_str!("../../../aa-ui/examples/fake/fake_skill.lua")),
    ("fake_quest.lua", include_str!("../../../aa-ui/examples/fake/fake_quest.lua")),
    ("fake_worldmap.lua", include_str!("../../../aa-ui/examples/fake/fake_worldmap.lua")),
    ("fake_options.lua", include_str!("../../../aa-ui/examples/fake/fake_options.lua")),
    ("fake_target.lua", include_str!("../../../aa-ui/examples/fake/fake_target.lua")),
];
const INGAME_LUA: &str = include_str!("ingame.lua");
/// Where the road map images sit in the roadmap window (generated from the client's `map/road_map_list.lua`).
const ROAD_FRAMES_LUA: &str = include_str!("road_frames.lua");

/// Install the game API into a fresh runtime, before the stage's scripts run.
pub fn install(rt: &mut UiRuntime, bridge: &Shared) -> mlua::Result<()> {
    // 1. the X2* objects and constants (the login screens' install, with an empty bridge)
    gameapi::install(rt, &gameapi::new_shared())?;
    let lua = &rt.lua;
    let before_fakes = super::stubs::snapshot(lua);
    // the table the fake file keeps its helpers in
    let fake = lua.create_table()?;
    fake.raw_set("locale", "en_us")?;
    fake.raw_set("worlds", lua.create_table()?)?;
    fake.raw_set("characters", lua.create_table()?)?;
    fake.raw_set(
        "fire_now",
        lua.create_function(|lua, (name, rest): (String, MultiValue)| {
            aa_ui::lua::fire_ui_event(lua, &name, rest.into_iter().collect());
            Ok(())
        })?,
    )?;
    fake.raw_set(
        "fire_later",
        lua.create_function(|lua, (name, rest): (String, MultiValue)| {
            aa_ui::lua::queue_ui_event(lua, &name, rest.into_iter().collect());
            Ok(())
        })?,
    )?;
    lua.globals().raw_set("FAKE", fake)?;
    // 2. FALLBACK: the fake in-game data
    for (name, code) in FAKE_INGAME {
        if let Err(e) = lua.load(code).set_name(format!("@{name}")).exec() {
            warn!("hud: {name}: {e}");
        }
    }
    let fakes = super::stubs::snapshot(lua);
    // The fallback has its own option store. Restore the account-backed methods after it.
    crate::settings::install_options(lua)?;
    aa_ui::lua::keyboard::install_input(lua)?;
    // 3. the real data
    let hud = lua.create_table()?;
    install_functions(lua, &hud, bridge)?;
    super::unit_id::install(lua, &hud)?;
    super::combat::install(lua)?;
    super::cast::install(lua, &hud, bridge)?;
    super::loot::install(lua, &hud, bridge)?;
    super::dialog::install(lua, &hud, bridge)?;
    super::map_quests::install(lua, &hud, bridge)?;
    super::drag::install(lua, &hud)?;
    super::skillbook::install(lua, &hud, bridge)?;
    super::buffs::query::install(lua, &hud, bridge)?;
    super::markers::install(lua, &hud)?;
    super::party::install(lua, &hud, bridge)?;
    super::chat::commands::install(lua, &hud, bridge)?;
    super::friends::install(lua, bridge)?;
    super::store::install(lua, &hud, bridge)?;
    lua.globals().raw_set("HUD", hud.clone())?;
    push_snapshot(lua, &hud, &bridge.borrow().snapshot)?;
    panels::push(lua, &hud, &bridge.borrow().panels, None)?;
    let frames: Table = lua.load(ROAD_FRAMES_LUA).set_name("@road_frames.lua").eval()?;
    lua.globals().raw_set("HUD_ROAD_FRAMES", frames)?;
    lua.load(include_str!("road_projection.lua")).set_name("@road_projection.lua").exec()?;
    lua.load(INGAME_LUA).set_name("@ingame.lua").exec()?;
    crate::login::name_policy::install(lua)?;
    super::cooldown::install(lua, bridge)?;
    install_feature_limits(lua, bridge)?;
    super::tips::install_native_lookup(lua, bridge)?;
    super::markers::load_script(lua)?;
    super::drag::load_script(lua)?;
    super::skillbook::load_script(lua)?;
    super::dialog::load_script(lua)?;
    super::map_quests::load_script(lua)?;
    super::party::load_script(lua)?;
    super::chat::commands::load_script(lua)?;
    super::store::load_script(lua)?;
    super::specialty::install(lua, bridge)?;
    super::mail::install(lua, bridge)?;
    super::auction::install(lua, bridge)?;
    super::housing::install(lua, bridge)?;
    super::itemlinks::install(lua, bridge)?;
    super::item_effects::install(lua, bridge)?;
    super::enchant::install(lua, bridge)?;
    super::look_conversion::install(lua, bridge)?;
    super::loot_roll::install(lua)?;
    super::trade::install(lua, bridge)?;
    super::pvp::install(lua, bridge)?;
    super::radar::install(lua, bridge)?;
    super::battlefield::install(lua, bridge)?;
    super::breath::install(lua, bridge)?;
    super::map_overlays::install(lua, bridge)?;
    super::expansion::install(lua, bridge)?;
    super::repair::install(lua, bridge)?;
    super::crafting::install(lua, bridge)?;
    super::user_music::install(lua, bridge)?;
    super::mode_actions::install(lua, bridge)?;
    super::mounted::install(lua, bridge)?;
    super::pets::install(lua, bridge)?;
    super::family::install(lua, bridge)?;
    super::expedition::install(lua, bridge)?;
    super::territory::install(lua, bridge)?;
    super::achievement::install(lua, bridge)?;
    crate::settings::install_hud(lua)?;
    super::chat::config::install(lua, bridge)?;
    super::chat::ui_data::install(lua, bridge)?;
    super::crime::install(lua, bridge)?;
    super::speech::install(lua, bridge)?;
    super::appellation::install(lua, bridge)?;
    super::portal::install(lua, bridge)?;
    super::doodad::install(lua, bridge)?;
    super::store::services::load_script(lua)?;
    super::duel::install(lua, bridge)?;
    super::trial::install(lua, bridge)?;
    super::ranking::install(lua, bridge)?;
    super::buffs::cancel::install(lua, bridge)?;
    super::salon::install(lua)?;
    super::sorting::install(lua, bridge)?;
    super::stubs::instrument(lua, &before_fakes, &fakes)
}

/// Native39496b20 returns numeric feature bytes, never the fallback's boolean placeholders.
fn install_feature_limits(lua: &Lua, bridge: &Shared) -> mlua::Result<()> {
    let player: Table = lua.globals().get("X2Player")?;
    let previous: mlua::Function = player.get("GetFeatureSet")?;
    let state = bridge.clone();
    player.set(
        "GetFeatureSet",
        lua.create_function(move |_, this: Value| {
            let ((level, mate), achievement, shop) = {
                let state = state.borrow();
                (
                    state.feature_limits,
                    state.achievement.cache.native_enabled,
                    state.in_game_shop_enabled,
                )
            };
            let out: Table = previous.call(this)?;
            // Until initial configuration arrives, zero leaves the original tooltip's upper
            // restriction unset. Do not invent a positive cap from experience-table length.
            out.set("levelLimit", f64::from(level.unwrap_or(0)))?;
            out.set("mateLevelLimit", f64::from(mate.unwrap_or(0)))?;
            out.set("achievement", achievement == Some(true))?;
            out.set("ingamecashshop", shop == Some(true))?;
            // the server serves the auction house only at an auctioneer: no remote button
            out.set("hudAuctionButton", false)?;
            Ok(out)
        })?,
    )?;
    if let Some(shop) = lua.globals().get::<Option<Table>>("X2InGameShop")? {
        let state = bridge.clone();
        shop.set(
            "IsInGameShopEnable",
            lua.create_function(move |_, _: Value| {
                Ok(state.borrow().in_game_shop_enabled == Some(true))
            })?,
        )?;
    }
    Ok(())
}

/// The Rust functions of the `HUD` table.
fn install_functions(lua: &Lua, hud: &Table, bridge: &Shared) -> mlua::Result<()> {
    // the player's game position and facing (changes every frame while moving: not in the snapshot)
    let b = bridge.clone();
    hud.raw_set(
        "player_pos",
        lua.create_function(move |_, ()| {
            let d = b.borrow();
            Ok((d.pos.x as f64, d.pos.y as f64, d.pos.z as f64, d.yaw as f64))
        })?,
    )?;
    let b = bridge.clone();
    hud.raw_set(
        "world_rect",
        lua.create_function(move |_, id: u32| {
            Ok(b.borrow().game.get().and_then(|g| g.world_rects.get(&id).map(|r| r.iter().map(|v| *v as f64).collect::<Vec<f64>>())))
        })?,
    )?;
    let b = bridge.clone();
    hud.raw_set(
        "faction_name",
        lua.create_function(move |_, id: u32| Ok(b.borrow().game.get().and_then(|g| g.factions.get(&id).cloned())))?,
    )?;
    let b = bridge.clone();
    hud.raw_set(
        "request",
        lua.create_function(move |_, args: MultiValue| {
            let text = |i: usize| match args.iter().nth(i) {
                Some(Value::String(s)) => s.to_string_lossy(),
                _ => String::new(),
            };
            let num = |i: usize| match args.iter().nth(i) {
                Some(Value::Integer(n)) => i64::from(*n),
                Some(Value::Number(n)) => *n as i64,
                _ => 0,
            };
            let request = match text(0).as_str() {
                "chat" => Request::Chat { chat_type: num(1) as i16, message: text(2), target: text(3) },
                "leave" => Request::Leave { how: num(1) },
                "leave_cancel" => Request::CancelLeave,
                "use_action" => Request::UseAction { slot: num(1).max(1) as usize,
                },
                "use_mode_action" if num(1) > 0 => Request::UseModeAction {
                    slot: num(1) as usize },
                "use_pet_action" => Request::UsePetAction { slot: num(1).max(1) as usize },
                "use_bag" => Request::UseBag { slot: num(1).max(1) as usize },
                "resurrect" => Request::Resurrect { in_place: num(1) != 0 },
                "learn_skill" => Request::LearnSkill { id: num(1).max(0) as u32 },
                "learn_buff" => Request::LearnBuff { id: num(1).max(0) as u32 },
                "reset_skills" => Request::ResetSkills { ability: num(1).clamp(0, 255) as u8 },
                "swap_ability" => {
                    let old = num(1).clamp(0, 255) as u8;
                    let npc_token = if old == super::skillbook::NO_ABILITY {
                        None
                    } else {
                        let Some(scope) = super::store::services::allows(&b.borrow(), super::store::services::ABILITY) else { return Ok(()) };
                        Some(scope.token())
                    };
                    Request::SwapAbility { old, ability: num(2).clamp(0, 255) as u8, npc_token }
                },
                other => {
                    warn!("hud: unknown request {other:?}");
                    return Ok(());
                }
            };
            b.borrow_mut().requests.push(request);
            Ok(())
        })?,
    )?;
    // `X2Action:GetActionBarPage` / `SetActionBarPage`: the scripts' page API is 1 based, the
    // native one 0 based; the first 12 display cells follow the page, the sub bars do not.
    let b = bridge.clone();
    hud.raw_set(
        "get_action_bar_page",
        lua.create_function(move |_, ()| Ok(i64::from(b.borrow().action_page) + 1))?,
    )?;
    let b = bridge.clone();
    hud.raw_set(
        "action_slot",
        lua.create_function(move |_, slot: i64| {
            let display = usize::try_from(slot).ok().and_then(|slot| slot.checked_sub(1));
            Ok(display
                .and_then(|display| aa_net::state::ActionBar::display_slot_on(b.borrow().action_page, display))
                .map(|slot| slot as i64))
        })?,
    )?;
    let b = bridge.clone();
    hud.raw_set(
        "set_action_bar_page",
        lua.create_function(move |lua, page: i64| {
            if (1..=i64::from(aa_net::items::ACTION_PAGES)).contains(&page) {
                let page0 = (page - 1) as u8;
                {
                    let mut d = b.borrow_mut();
                    d.action_page = page0;
                    d.requests.push(Request::SetActionBarPage { page: page0 });
                }
                // Redraw the cells for the new page (`HUD.actions` itself does not change).
                aa_ui::lua::slot::refresh_slots(lua)?;
                // `FUN_393f6910` applies the page and fires the event even when it is the current
                // page; only the page range is checked.
                aa_ui::lua::fire_ui_event(lua, "ACTION_BAR_PAGE_CHANGED", vec![Value::Integer(page)]);
            }
            Ok(page)
        })?,
    )?;
    // the HUD's clock: buff and cooldown end times are on it
    let b = bridge.clone();
    hud.raw_set("now_ms", lua.create_function(move |_, ()| Ok(panels::now_ms(b.borrow().start) as f64))?)?;
    // metres from the player to the target
    let b = bridge.clone();
    hud.raw_set(
        "target_distance",
        lua.create_function(move |_, ()| {
            let d = b.borrow();
            Ok(d.target_pos.map(|t| distance_to(d.pos, d.player_radius, t, d.target_radius) as f64))
        })?,
    )?;
    // seconds until the player may stand up again (the death window's countdown)
    let b = bridge.clone();
    hud.raw_set(
        "death_wait",
        lua.create_function(move |_, ()| {
            let d = b.borrow();
            let left = d.death_ends.get().map_or(0, |end| end.saturating_sub(panels::now_ms(d.start)));
            Ok((left / 1000) as i64)
        })?,
    )?;
    let b = bridge.clone();
    hud.raw_set("death_timeout", lua.create_function(move |_, ()| {
        let d = b.borrow();
        Ok(death_timeout(d.death_started.get(), panels::now_ms(d.start)))
    })?)?;
    let b = bridge.clone();
    hud.raw_set("can_resurrect_here", lua.create_function(move |_, ()| Ok(b.borrow().revive_caster.get().is_some()))?)?;
    // `X2Unit:GetUnitNameById` for the resurrection caster, which need not be the target or a party member
    // (`FUN_391d39f0` pushes its live-object id to `PLAYER_RESURRECTION`).
    let b = bridge.clone();
    hud.raw_set(
        "unit_name",
        lua.create_function(move |_, id: String| {
            let d = b.borrow();
            if d.revive_caster.get().is_none() || d.revive_caster.get() != super::unit_id::string(&id) {
                return Ok(None);
            }
            Ok(d.revive_caster_name.borrow().clone())
        })?,
    )?;
    // database lookups for the content hooks: name, icon (and the rest)
    let b = bridge.clone();
    hud.raw_set(
        "skill_info",
        lua.create_function(move |_, id: u32| {
            let d = b.borrow();
            let info = d.game.get().and_then(|g| g.skills.get(&id));
            Ok((info.map(|s| s.named.name.clone()), info.map(|s| s.named.icon.clone())))
        })?,
    )?;
    let b = bridge.clone();
    hud.raw_set(
        "item_info",
        lua.create_function(move |_, id: u32| {
            let d = b.borrow();
            let info = d.game.get().and_then(|g| g.items.get(&id));
            Ok((
                info.map(|i| i.named.name.clone()),
                info.map(|i| i.named.icon.clone()),
                info.map(|i| i.max_stack),
                info.map(|i| i64::from(super::store::ui_grade(i.fixed_grade.max(0) as u8))),
                info.map(|i| i.level),
            ))
        })?,
    )?;
    // the player's Character Info numbers (`tipcalc/charstats.rs`): UnitInfo / UnitModifierInfo
    for (name, modifiers) in [("player_info", false), ("player_modifiers", true)] {
        let b = bridge.clone();
        hud.raw_set(
            name,
            lua.create_function(move |lua, ()| {
                let d = b.borrow();
                let sheet = if modifiers { &d.tips.sheet.modifiers } else { &d.tips.sheet.info };
                if sheet.is_empty() {
                    return Ok(None);
                }
                let t = lua.create_table()?;
                for (key, value) in sheet {
                    t.raw_set(*key, *value)?;
                }
                Ok(Some(t))
            })?,
        )?;
    }
    // the tooltip tables of items and skills (`tips.rs`), built when asked
    let b = bridge.clone();
    hud.raw_set(
        "item_tip",
        lua.create_function(move |lua, (id, grade, count, slot, flags): (u32, Option<i64>, Option<i64>, Option<Table>, mlua::Variadic<Value>)| {
            let d = b.borrow();
            match d.game.get() {
                Some(g) => {
                    if super::tips::native_info_flags(flags.first()) == -1 {
                        return super::tips::item_tip_of_slot(lua, g, &d.tips, id, grade, count, slot.as_ref());
                    }
                    let mut extra = super::tips::ItemExtra::of(slot.as_ref());
                    extra.music_note = None;
                    let result = super::tips::item_tip_with(lua, g, &d.tips, id, grade, count,
                        super::tips::Instance::of(slot.as_ref()), &extra)?;
                    if let Some(info) = &result {
                        info.raw_set("useAsSkin", g.tips.items.extra.use_as_skin(id, None, 0))?;
                    }
                    Ok(result)
                }
                None => Ok(None),
            }
        })?,
    )?;
    let b = bridge.clone();
    hud.raw_set(
        "skill_tip",
        lua.create_function(move |lua, id: u32| {
            let d = b.borrow();
            match d.game.get() {
                Some(g) => super::tips::skill_tip(lua, g, &d.tips, id, d.skill_levels.get(&id).copied().unwrap_or(1)),
                None => Ok(None),
            }
        })?,
    )?;
    // a buff's description with its numbers filled in (nil when it has none to fill)
    let b = bridge.clone();
    hud.raw_set(
        "buff_text",
        lua.create_function(move |_, (id, ab_level): (u32, Option<i32>)| {
            let d = b.borrow();
            Ok(d.game.get().and_then(|g| super::tips::buff_text(g, &d.tips, id, ab_level.unwrap_or(0))))
        })?,
    )?;
    // the chat window widget (the scripts create it; `GetEntityByName` is a stub)
    hud.raw_set(
        "chat_window",
        lua.create_function(|lua, ()| {
            let id = {
                let u = aa_ui::lua::ui(lua);
                u.tree.objects.iter().enumerate().find(|(_, o)| o.alive && o.widget().is_some_and(|w| w.class == "chatwindow")).map(|(i, _)| i as aa_ui::tree::Id)
            };
            match id {
                Some(id) => aa_ui::lua::object_table(lua, id),
                None => Ok(None),
            }
        })?,
    )?;
    Ok(())
}

/// Native GetResurrectionCountdown (0x39495c00): a separate five-minute deadline, truncated to seconds.
fn death_timeout(started: Option<u64>, now: u64) -> i64 {
    started.map_or(0, |start| (300_000u64.saturating_sub(now.saturating_sub(start)) / 1000) as i64)
}

/// One row of `X2Ability:GetActiveAbility()`: the ability, its level and where its experience stands in the level
/// (`levelPercent` 0..100, `prev` / `nextLevelTotalExp` the totals the level starts and ends at).
fn ability_row(lua: &Lua, id: u8, total: i32) -> mlua::Result<Table> {
    let total = i64::from(total);
    let level = exp::level_for_total(total);
    let (earned, needed) = exp::progress(level, total);
    let start = exp::total_for_level(level);
    let t = lua.create_table()?;
    t.raw_set("type", i64::from(id))?;
    t.raw_set("level", i64::from(level))?;
    t.raw_set("exp", total)?;
    t.raw_set("levelPercent", if needed > 0 { 100.0 * earned as f64 / needed as f64 } else { 100.0 })?;
    t.raw_set("prevLevelTotalExp", start)?;
    t.raw_set("nextLevelTotalExp", start + needed)?;
    Ok(t)
}

/// The snapshot as the Lua table `HUD.state`.
fn push_snapshot(lua: &Lua, hud: &Table, s: &Snapshot) -> mlua::Result<()> {
    let t = lua.create_table()?;
    t.raw_set("unit_id", s.unit_id.as_str())?;
    t.raw_set("instance_id", s.instance_id)?;
    t.raw_set("name", s.name.as_str())?;
    t.raw_set("level", s.level as i64)?;
    t.raw_set("race", s.race as i64)?;
    t.raw_set("gender", s.gender as i64)?;
    t.raw_set("hp", s.hp as i64)?;
    t.raw_set("max_hp", s.max_hp as i64)?;
    t.raw_set("mp", s.mp as i64)?;
    t.raw_set("max_mp", s.max_mp as i64)?;
    t.raw_set("exp_percent", s.exp_percent)?;
    t.raw_set("dead", s.dead)?;
    t.raw_set("in_combat", s.in_combat)?;
    t.raw_set("recoverable_exp", s.recoverable_exp as i64)?;
    t.raw_set("honor", s.honor as i64)?;
    t.raw_set("pvp_honor", s.pvp_honor as i64)?;
    t.raw_set("pvp_kills", s.pvp_kills as i64)?;
    t.raw_set("vocation", s.vocation as i64)?;
    t.raw_set("exp_cur", s.exp_cur.to_string())?;
    t.raw_set("exp_need", s.exp_need.to_string())?;
    t.raw_set("labor", s.labor as i64)?;
    t.raw_set("max_labor", s.max_labor as i64)?;
    t.raw_set("money", s.money.to_string())?;
    t.raw_set("bank_money", s.bank_money.to_string())?;
    t.raw_set("bm_point", s.bm_point.to_string())?;
    t.raw_set("zone", s.zone.as_str())?;
    t.raw_set("zone_group", s.zone_group as i64)?;
    t.raw_set("zone_key_name", s.zone_key_name.as_str())?;
    t.raw_set("zone_rect", lua.create_sequence_from(s.zone_rect.iter().map(|v| *v as f64))?)?;
    t.raw_set("zone_faction", s.zone_faction as i64)?;
    t.raw_set("zone_faction_name", s.zone_faction_name.as_str())?;
    t.raw_set("sub_zone", s.sub_zone.as_str())?;
    t.raw_set("sub_map", s.sub_map as i64)?;
    t.raw_set("sub_map_rect", lua.create_sequence_from(s.sub_map_rect.iter().map(|v| *v as f64))?)?;
    t.raw_set("faction", s.faction.as_str())?;
    // X2Ability: the active abilities as the rows `GetActiveAbility` lists, and every ability's level (0 = none yet)
    let rows = lua.create_table()?;
    for &id in &s.abilities {
        rows.raw_push(ability_row(lua, id, s.ability_exp[usize::from(id)])?)?;
    }
    t.raw_set("abilities", rows)?;
    let levels = s.ability_exp.iter().enumerate().skip(1).map(|(id, &total)| {
        i64::from(if total > 0 || s.abilities.contains(&(id as u8)) { exp::level_for_total(i64::from(total)) } else { 0 })
    });
    t.raw_set("ability_levels", lua.create_sequence_from(levels)?)?;
    t.raw_set("hour", s.hour as i64)?;
    t.raw_set("minute", s.minute as f64)?;
    hud.raw_set("state", t)
}

/// The local player's entity: `ingame::player::spawn_player` names it "player <name>".
fn find_player(world: &mut World, name: &str) -> Option<Entity> {
    let wanted = format!("player {name}");
    let mut q = world.query::<(Entity, &Name, &Transform)>();
    q.iter(world).find(|(_, n, _)| n.as_str() == wanted).map(|(e, _, _)| e)
}

/// Recompute the snapshot from the Bevy world.
pub fn refresh_snapshot(world: &mut World, bridge: &Shared) {
    let config = world
        .get_resource::<aa_net::GameConfig>()
        .and_then(|config| config.received.as_ref());
    super::map_overlays::refresh(world, bridge);
    {
        let mut state = bridge.borrow_mut();
        state.feature_limits = config.map_or((None, None), |config| {
            (config.player_level_limit(), config.mate_level_limit())
        });
        state.in_game_shop_enabled = config
            .and_then(|config| config.feature_bytes.get(4))
            .map(|byte| byte & 0x40 != 0);
    }
    let Some(spawn) = world.resource::<Player>().spawn.clone() else {
        bridge.borrow_mut().pvp = Default::default();
        bridge.borrow_mut().radar = Default::default();
        bridge.borrow_mut().battlefield = Default::default();
        bridge.borrow_mut().expansion.clear();
        bridge.borrow_mut().repair = Default::default();
        bridge.borrow_mut().crafting = Default::default();
        bridge.borrow_mut().pets = Default::default();
        bridge.borrow_mut().cooldowns = Default::default();
        return;
    };
    // the world renderer's clock (`ingame::sync_clock` copies the server's into it, `--hour` pins it: the HUD
    // clock shows the time the world is at), else the server's (extrapolated from its last packet)
    let net_world = world.get_resource::<aa_net::state::WorldInfo>();
    let hour = world
        .get_resource::<aa_world::WorldClock>()
        .map(|c| c.hour)
        .or_else(|| net_world.and_then(|w| w.hour_now()))
        .unwrap_or(12.0);
    let net_group = net_world.and_then(|w| w.zone_group_id);
    // where the player is (game coordinates), from its entity's transform
    let known = bridge.borrow().player_entity;
    let entity = known.filter(|e| world.get::<Transform>(*e).is_some()).or_else(|| find_player(world, spawn.name()));
    let (pos, yaw) = match entity.and_then(|e| world.get::<Transform>(e)) {
        Some(t) => (aa_world::bevy_to_cry(t.translation), t.rotation.to_euler(EulerRot::YXZ).0),
        None => {
            let p = spawn.position();
            (Vec3::new(p.x, p.y, p.z), spawn.yaw())
        }
    };
    // the zone under the player (the world's sector map), else the zone channel's group, else the spawn's
    let zone_key = world.get_resource::<aa_world::stream::WorldInfo>().and_then(|w| w.0.zone_at(pos)).map(u32::from);
    let mut data = bridge.borrow_mut();
    super::pvp::refresh(world, &mut data, &spawn);
    super::radar::refresh(world, &mut data, &spawn);
    data.character_id = spawn.character().id;
    super::battlefield::refresh(world, &mut data, &spawn);
    super::expansion::refresh(world, &mut data);
    super::repair::refresh(world, &mut data);
    super::crafting::refresh(world, &mut data);
    super::pets::refresh(world, &mut data);
    data.mounted.local_cart = world
        .get_resource::<crate::ingame::cart::CartDrive>()
        .and_then(crate::ingame::cart::CartDrive::speed);
    if let Some(party) = world.get_resource::<aa_net::Party>() {
        if data.party != *party { data.party.clone_from(party); }
    }
    if let Some(friends) = world.get_resource::<aa_net::Friends>() {
        if data.friends != *friends { data.friends.clone_from(friends); }
    }
    if let Some(skills) = world.get_resource::<aa_net::state::Skills>() {
        data.cooldowns.clone_from(skills);
        if skills.known.len() != data.skill_levels.len() || skills.known.iter().any(|k| data.skill_levels.get(&k.id) != Some(&k.level)) {
            data.skill_levels = skills.known.iter().map(|k| (k.id, k.level)).collect();
        }
        if skills.passive_buffs != data.passive_buffs {
            data.passive_buffs.clone_from(&skills.passive_buffs);
        }
    }
    data.player_entity = entity;
    data.pos = pos;
    data.yaw = yaw;
    data.turret_seat = world
        .get_resource::<crate::ingame::LocalMount>()
        .and_then(crate::ingame::LocalMount::turret_seat);
    data.turret = world.get_resource::<crate::ingame::turret::Sight>().copied();
    let character = spawn.character();
    let game = data.game.clone();
    let game = game.get();
    data.player_radius = game.map_or(0.0, |g| g.model_size(spawn.unit.model_id, spawn.unit.scale));
    // the numbers the server keeps current (aa-net's PlayerStats), the spawn's values until it has them
    let stats = world.get_resource::<PlayerStats>().filter(|s| s.obj_id != 0);
    let level = stats.map_or(character.level, |s| s.level).max(1);
    let (exp_cur, exp_need) = stats.map_or((0, 1), |s| s.exp_progress());
    let exp_percent = if exp_need > 0 { 100.0 * exp_cur as f64 / exp_need as f64 } else { 100.0 };
    let faction_name = |g: &super::data::GameData, id: u32| g.factions.get(&id).cloned().filter(|f| !f.is_empty());
    let faction = game.and_then(|g| faction_name(g, character.faction_id)).unwrap_or_else(|| character.faction_name.clone());
    let zone = game.and_then(|g| {
        zone_key
            .and_then(|k| g.zone(k))
            .or_else(|| net_group.and_then(|id| g.zone_by_group(id).map(|(z, f)| (id, z, f))))
            .or_else(|| g.zone(spawn.zone_id()))
    });
    let sub_root = game.and_then(|g| g.sub_zone_root_at(pos.x, pos.y));
    let sub_zone = game.zip(sub_root).and_then(|(g, id)| g.sub_zone_names.get(&id)).cloned().unwrap_or_default();
    let (sub_map, sub_map_rect) = game
        .zip(sub_root)
        .and_then(|(g, id)| g.sub_zone_rects.get(&id).map(|r| (id, *r)))
        .unwrap_or((0, [0.0; 4]));
    // the maxima: the database's formulas over level, gear and buffs (the server sends none), the highest seen until it is read
    let own_buffs = world.get_resource::<aa_net::state::Buffs>().map_or(&[][..], |b| b.of(spawn.obj_id));
    let formula_max = game.zip(world.get_resource::<aa_net::state::Inventory>()).and_then(|(g, inv)| {
        g.stats.player_max(level, &super::stats::gear_of(&inv.equipment), &super::stats::buff_list(own_buffs))
    });
    let (known_hp, known_mp) = stats.map_or((character.hp, character.mp), |s| (s.max_hp, s.max_mp));
    let hp = stats.map_or(character.hp, |s| s.hp);
    let mp = stats.map_or(character.mp, |s| s.mp);
    let (max_hp, max_mp) = formula_max.map_or((known_hp, known_mp), |(h, m)| (h.max(hp), m.max(mp)));
    // the server's set for the character (the same ids the character list shows), kept current by swaps
    let ability_slots = stats.map_or(character.abilities, |s| s.active_abilities);
    let snapshot = Snapshot {
        unit_id: spawn.obj_id.to_string(),
        instance_id: spawn.state.instance_id,
        name: spawn.name().to_string(),
        level,
        race: character.race,
        gender: character.gender,
        hp,
        max_hp: max_hp.max(1),
        mp,
        max_mp: max_mp.max(1),
        dead: stats.is_some_and(|s| s.dead),
        in_combat: stats.is_some_and(|s| s.in_combat) || data.local_combat,
        exp_percent,
        exp_cur,
        exp_need,
        recoverable_exp: stats.map_or(0, |s| s.recoverable_exp),
        labor: stats.map_or(i32::from(character.labor_power), |s| s.labor),
        max_labor: LABOR_MAX,
        labor_log: stats.map_or_else(Vec::new, |s| s.labor_log.clone()),
        labor_seq: stats.map_or(0, |s| s.labor_seq),
        money: stats.map_or(character.money, |s| s.money),
        bank_money: stats.map_or(character.money2, |s| s.bank_money),
        bm_point: stats.map_or(character.bm_point, |s| s.loyalty),
        honor: stats.map_or(0, |s| s.honor),
        pvp_honor: spawn.unit.pvp_honor,
        pvp_kills: spawn.unit.pvp_kills,
        vocation: stats.map_or(0, |s| s.vocation),
        zone: zone.map_or_else(String::new, |(_, g, _)| g.text.clone()),
        zone_group: zone.map_or(0, |(id, _, _)| id),
        zone_key_name: zone.map_or_else(String::new, |(_, g, _)| g.key.clone()),
        zone_rect: zone.map_or([0.0; 4], |(_, g, _)| g.rect),
        zone_faction: zone.map_or(0, |(_, _, f)| f),
        zone_faction_name: zone.and_then(|(_, _, f)| game.and_then(|g| faction_name(g, f))).unwrap_or_default(),
        sub_zone,
        sub_map,
        sub_map_rect,
        faction,
        abilities: ability_slots.into_iter().filter(|a| (1..aa_net::model::NO_ABILITY).contains(a)).collect(),
        ability_exp: stats.map_or([0; 11], |s| s.ability_exp),
        hour: hour.floor() as u8 % 24,
        minute: (hour.fract() * 60.0).floor(),
    };
    if formula_max.is_some() {
        // everything else that reads `PlayerStats` gets the real maxima too
        let (max_hp, max_mp) = (snapshot.max_hp, snapshot.max_mp);
        drop(data);
        if let Some(mut s) = world.get_resource_mut::<PlayerStats>().filter(|s| s.obj_id != 0 && (s.max_hp, s.max_mp) != (max_hp, max_mp)) {
            debug!("hud: max hp/mp {max_hp}/{max_mp} from the formulas (was {}/{}, current {}/{})", s.max_hp, s.max_mp, s.hp, s.mp);
            s.set_max(max_hp, max_mp);
        }
        data = bridge.borrow_mut();
    }
    if data.snapshot.abilities != snapshot.abilities {
        // slots as the server sent them: 11 (`NO_ABILITY`, the scripts' `ABILITY_MAX`) is an empty slot, not an ability
        info!("hud: abilities {:?} from the slots {ability_slots:?} (11 = empty), exp {:?}", snapshot.abilities, snapshot.ability_exp);
    }
    let mut snapshot = snapshot;
    dev_sim(&mut snapshot);
    data.snapshot = snapshot;
    // what the tooltips are worked out from: level, gear, buffs and abilities (only when one changed)
    {
        let (level, ability_exp) = (data.snapshot.level, data.snapshot.ability_exp);
        let shared = data.game.clone();
        if let (Some(g), Some(inv)) = (shared.get(), world.get_resource::<aa_net::state::Inventory>()) {
            let buffs = world.get_resource::<aa_net::state::Buffs>().map_or_else(Vec::new, |b| super::stats::buff_list(b.of(spawn.obj_id)));
            data.tips.refresh(g, level, &inv.equipment, buffs, &ability_exp);
            let return_point = world.get_resource::<aa_net::Portals>().and_then(|b| b.districts.iter().find(|p| p.id == b.return_point_id)).map(|p| (g.zone(p.zone_id).map_or_else(String::new, |(_, z, _)| z.text.clone()), p.name.clone()));
            data.tips.set_context(crate::settings::option_number("n:69").is_some_and(|v| v != 0.0), return_point);
        }
    }
    let own_obj_id = spawn.obj_id;
    let own_faction = character.faction_id;
    let start = data.start;
    let game = data.game.clone();
    let (new_panels, target_pos, target_radius) = panels::refresh(world, game.get(), own_faction, own_obj_id, start, &data.snapshot, data.pos);
    data.panels = new_panels;
    data.target_pos = target_pos;
    data.target_radius = target_radius;
    super::battlefield::refresh_slots(world, &mut data);
super::mode_actions::refresh_slots(world, &mut data);
}

/// `--hud-sim mp=N` / `--hud-sim dead` (dev): the HUD behaves as if the player had at most N mana / were dead, for screenshots of
/// the dark action bar states without a fight. Only the HUD's view: the server knows nothing of it.
fn dev_sim(s: &mut Snapshot) {
    static SIM: std::sync::OnceLock<(Option<i32>, bool)> = std::sync::OnceLock::new();
    let (mp, dead) = *SIM.get_or_init(|| {
        let args: Vec<String> = aa_dev::args();
        let mut sim = (None, false);
        for (i, a) in args.iter().enumerate().filter(|(_, a)| *a == "--hud-sim") {
            match args.get(i + 1).map(String::as_str) {
                Some("dead") => sim.1 = true,
                Some(v) if v.starts_with("mp=") => sim.0 = v[3..].parse().ok(),
                other => warn!("hud: --hud-sim {}: use mp=N or dead (arg {a})", other.unwrap_or("")),
            }
        }
        sim
    });
    if let Some(mp) = mp {
        s.mp = s.mp.min(mp);
    }
    if dead {
        s.dead = true;
        s.hp = 0;
    }
}

/// The events the client raises when the player has entered the world and the loading screen closed,
/// then the chat channels joined and the lines received so far.
pub fn enter_world(rt: &UiRuntime, bridge: &Shared, chat: &aa_net::state::Chat) {
    super::battlefield::entered_world(rt, bridge);
    super::expansion::extend_slots(rt, bridge.borrow().panels.bag_slots);
    if let Err(e) = rt.exec("@enter_world", "HUD.enter_world()") {
        warn!("hud: enter world events: {e}");
    }
    let (game,friends) = { let d=bridge.borrow(); (d.game.clone(),d.friends.clone()) };
    super::chat::replay(rt, chat, game.get(), &friends);
    // the labor gained so far prints as chat lines too ("Gained 10 Labor Point(s). (Total LP: 105)"): push the
    // current state and let the difference against the stage's own say that all of the session's changes are new
    {
        let mut d = bridge.borrow_mut();
        let mut base = d.pushed.take().unwrap_or_else(|| d.snapshot.clone());
        base.labor_seq = d.snapshot.labor_seq.saturating_sub(d.snapshot.labor_log.len() as u32);
        d.pushed = Some(base);
    }
    push_state(rt, bridge);
}

/// The emotion a typed line expresses: `/sad`, `/Clap  now` (the first word is the command).
pub(super) fn emote_command(emotes: Option<&crate::ingame::emote::Emotes>, message: &str) -> Option<u32> {
    let word = message.trim().strip_prefix('/')?.split_whitespace().next()?;
    emotes?.command(word)
}

/// Carry out what the scripts asked for.
#[allow(clippy::too_many_arguments)]
pub fn handle_requests(
    bridge: NonSend<HudBridge>,
    mut net: MessageWriter<NetCommand>,
    mut next: ResMut<NextState<crate::Screen>>,
    credentials: Option<Res<crate::ingame::enter::Credentials>>,
    mut bar: ResMut<aa_net::state::ActionBar>,
    inventory: Res<aa_net::state::Inventory>,
    target: Res<aa_net::state::Target>,
    skills: Res<aa_net::state::Skills>,
    buffs: Res<aa_net::state::Buffs>,
    mut flight: ResMut<super::usable::InFlight>,
    mut failed: MessageWriter<super::alerts::UseFailed>,
    mut pressed: MessageWriter<crate::ingame::ground_target::SkillPressed>,
    emotes: Option<Res<crate::ingame::emote::Emotes>>,
    units: Res<aa_net::state::Units>,
    seat: Option<Res<crate::ingame::LocalMount>>,
    mut ship_drive: Option<ResMut<crate::ingame::ShipDrive>>,
) {
    let requests = std::mem::take(&mut bridge.0.borrow_mut().requests);
    let selected = target.obj_id;
    for request in requests {
        match request {
            Request::Interact(interact) => {
                net.write(NetCommand(Command::Interact(interact)));
            }
            Request::Party(command) => { net.write(NetCommand(Command::Party(command))); }
            Request::Friend(command) => { net.write(NetCommand(Command::Friend(command))); }
            Request::Target { obj_id } => { net.write(NetCommand(Command::Target { obj_id })); }
            Request::Slave(command) => {
                // The driver's leave: the capture's last type-4 0/0 goes out just before CSDiscardSlave.
                if let aa_net::slave_control::SlaveCommand::Unbind { tl_id } = &command
                    && let (Some(seat), Some(ship_drive)) = (seat.as_deref(), ship_drive.as_deref_mut())
                    && let Some(request) = crate::ingame::leave_before_unbind(ship_drive, seat, &units, *tl_id)
                {
                    net.write(NetCommand(Command::ShipControl(request)));
                }
                net.write(NetCommand(Command::Slave(command)));
            }
            Request::UseSkill { skill_id, target_obj_id } => {
                net.write(NetCommand(Command::UseSkill { skill_id, target_obj_id }));
            }
            Request::SellBackpack { obj_id } => { net.write(NetCommand(Command::SellBackpackGoods { obj_id })); }
            Request::ChatCommand(command) => { net.write(NetCommand(Command::ChatCommand(command))); }
            Request::TargetByName { name } => {
                if let Some(obj_id) = super::chat::commands::unit_named(&units, &name) {
                    net.write(NetCommand(Command::Target { obj_id }));
                }
            }
            Request::Skill { skill, target_name } => {
                let named = super::chat::commands::unit_named(&units, &target_name);
                if let Some(obj_id) = named {
                    net.write(NetCommand(Command::Target { obj_id }));
                }
                pressed.write(crate::ingame::ground_target::SkillPressed { skill, item: None, target_obj_id: named.or(selected).unwrap_or(0), gcd: false });
            }
            Request::ItemEffect { character_id, request } => {
                // The typed request was captured by the original enchant API.
                // A replacement operation may retire a queued command before
                // this frame drains; Session rechecks genuine received records.
                if super::enchant::request_current(&bridge.0, character_id, &request) {
                    net.write(NetCommand(Command::ItemEffect { character_id, request }));
                }
            }
            Request::ItemLook { character_id, request } => {
                // Full received captures are checked again by Session at write time.
                {
                    let data = bridge.0.borrow();
                    if data.disconnected || data.character_id != character_id
                        || super::enchant::records(&data).owner() != Some(character_id)
                    {
                        continue;
                    }
                }
                if let Some(skill)=request.skill_id() {
                    let data=bridge.0.borrow();
                    let env=Env::of(&data,Some(&skills),buffs.of(skills.obj_id),Some(&inventory));
                    match super::look_conversion::admit_request(&data,&env,&flight,&request) {
                        super::look_conversion::Admission::Ready => {},
                        super::look_conversion::Admission::Failed(fail) => { failed.write(super::alerts::UseFailed { skill,result:fail.result,value:fail.value,server:false });continue; },
                        _ => continue,
                    }
                    if env.uses_gcd(skill) { flight.sent(Instant::now()); }
                }
                net.write(NetCommand(Command::ItemLook { character_id, request }));
            }
            Request::Chat { chat_type, message, target } => {
                // a slash command that names an emote (`/sad`) expresses it, at the target, instead of saying it
                if let Some(emotion_id) = emote_command(emotes.as_deref(), &message) {
                    let target_obj_id = selected.filter(|&t| t != skills.obj_id).unwrap_or(0);
                    net.write(NetCommand(Command::ExpressEmotion { emotion_id, target_obj_id }));
                    continue;
                }
                let message = aa_ui::lua::chat::encode_for_transport(&message);
                net.write(NetCommand(Command::Chat { chat_type, target, message }));
            }
            Request::Leave { how } => {
                let disconnected = bridge.0.borrow().disconnected;
                if disconnected {
                    // the connection is gone: the dialog's OK (EXIT_CLIENT) goes back to the server list
                    super::session::return_to_world_select(credentials.as_deref(), &mut net, &mut next);
                } else {
                    if let Some(save) = super::achievement::save_for_leave(&mut bridge.0.borrow_mut(), how) {
                        net.write(NetCommand(Command::UiData(save)));
                    }
                    super::session::leave(how, &mut net);
                }
            }
            Request::CancelLeave => {
                net.write(NetCommand(Command::CancelLeave));
            }
            Request::SetActionBarPage { page } => {
                if bar.set_page(page) {
                    bridge.0.borrow_mut().action_page = page;
                }
            }
            Request::UseAction { slot } => {
                let Some(slot) = bar.display_slot(slot.saturating_sub(1)) else { continue };
                let data = bridge.0.borrow();
                let env = Env::of(&data, Some(&skills), buffs.of(skills.obj_id), Some(&inventory));
                panels::use_slot(slot, &env, &flight, &bar, &inventory, &target, &mut pressed, &mut failed);
            }
            Request::UseModeAction { slot } => {
                let data = bridge.0.borrow();
                let env = Env::of(
                    &data,
                    Some(&skills),
                    buffs.of(skills.obj_id),
                    Some(&inventory),
                );
                super::mode_actions::use_slot(
                    &data,
                    slot - 1,
                    &env,
                    &mut flight,
                    &target,
                    &mut pressed,
                    &mut net,
                    &mut failed,
                );
            }
            Request::UsePetAction { slot } => {
                let target = selected.unwrap_or(0);
                let result = super::pets::use_skill(&mut bridge.0.borrow_mut(), slot, target);
                if let Err(result) = result {
                    failed.write(super::alerts::UseFailed {
                        skill: 0,
                        result,
                        value: 0,
                        server: false,
                    });
                }
            }
            Request::UseBag { slot } => {
                let data = bridge.0.borrow();
                let env = Env::of(&data, Some(&skills), buffs.of(skills.obj_id), Some(&inventory));
                panels::use_bag_slot(slot - 1, &env, &flight, &inventory, &target, &mut pressed, &mut failed);
            }
            Request::Resurrect { in_place } => {
                net.write(NetCommand(Command::Resurrect { in_place }));
            }
            Request::LearnSkill { id } => super::skillbook::learn_skill(&bridge.0.borrow(), id, &mut net),
            Request::LearnBuff { id } => super::skillbook::learn_buff(&bridge.0.borrow(), id, &mut net),
            Request::ResetSkills { ability } => {
                if bridge.0.borrow().snapshot.abilities.contains(&ability) {
                    net.write(NetCommand(Command::ResetSkills { ability }));
                }
            }
            Request::SwapAbility { old, ability, npc_token } => {
                let data = bridge.0.borrow();
                let active = &data.snapshot.abilities;
                let replace = active.contains(&old) || (old == super::skillbook::NO_ABILITY && active.len() < 3);
                let authorized = if old == super::skillbook::NO_ABILITY {
                    npc_token.is_none()
                } else {
                    npc_token.as_ref().is_some_and(|token| super::store::services::allows(&data,super::store::services::ABILITY).is_some_and(|scope| scope.token()==*token))
                };
                if replace && (1..=10).contains(&ability) && !active.contains(&ability) && authorized {
                    net.write(NetCommand(Command::SwapAbility { old, ability }));
                }
            }
        }
    }
}

/// The stage was built from the current snapshot.
pub fn mark_pushed(bridge: &Shared) {
    let mut d = bridge.borrow_mut();
    d.pushed = Some(d.snapshot.clone());
    d.panels_pushed = Some(d.panels.clone());
}

/// Push what changed into the Lua state and raise the events the scripts listen for.
pub fn push_changes(rt: &UiRuntime, bridge: &Shared) {
    super::mode_actions::push_slots(rt, bridge);
    super::pets::push(rt, bridge);
    push_state(rt, bridge);
    // after the state: the character window's handler reads HUD.state (level, ...)
    push_character_stats(rt, bridge);
    push_panels(rt, bridge);
}

/// The character window shows `UnitInfo` / `UnitModifierInfo`: when the numbers changed (level, gear, buffs) its own update
/// handler (the one every event it listens to calls) reads them again; it does nothing while the window is closed.
fn push_character_stats(rt: &UiRuntime, bridge: &Shared) {
    let generation = {
        let mut d = bridge.borrow_mut();
        if d.stats_pushed == d.tips.sheet_gen {
            return;
        }
        d.stats_pushed = d.tips.sheet_gen;
        d.tips.sheet_gen
    };
    if generation == 1 {
        return; // the first numbers: nothing was shown before
    }
    if let Err(e) = rt.lua.load("if HUD.refresh_character_info then HUD.refresh_character_info() end").exec() {
        warn!("hud: character info refresh: {e}");
    }
}

fn push_panels(rt: &UiRuntime, bridge: &Shared) {
    let (old, new) = {
        let mut d = bridge.borrow_mut();
        if d.panels_pushed.as_ref() == Some(&d.panels) {
            return;
        }
        let new = d.panels.clone();
        (d.panels_pushed.replace(new.clone()), new)
    };
    let Some(old) = old else { return };
    if old.bag_slots != new.bag_slots {
        super::expansion::extend_slots(rt, new.bag_slots);
    }
    // which action bar slots are dark and why (`usable.rs`)
    let dark = |p: &Panels| -> Vec<(usize, &'static str)> {
        p.actions.iter().enumerate().filter_map(|(i, s)| Some((i, super::usable::result_name(s.as_ref()?.blocked?)))).collect()
    };
    if dark(&old) != dark(&new) {
        let d = bridge.borrow();
        info!("hud: dark slots {:?}; target {:?} at {:?} m", dark(&new), new.target.as_ref().map(|t| t.name.as_str()), d.target_pos.map(|t| (distance_to(d.pos, d.player_radius, t, d.target_radius) * 10.0).round() / 10.0));
    }
    let lua = &rt.lua;
    let Ok(hud) = lua.globals().raw_get::<Table>("HUD") else { return };
    let changed = match panels::push(lua, &hud, &new, Some(&old)) {
        Ok(c) => c,
        Err(e) => {
            warn!("hud: panels: {e}");
            return;
        }
    };
    if changed.slots
        && let Err(e) = rt.refresh_slots()
    {
        warn!("hud: refresh slots: {e}");
    }
    if changed.bank {
        rt.fire_event("BANK_UPDATE", (-1i64, -1i64));
    }
    let id = |t: &Option<panels::TargetSnap>| t.as_ref().map(|t| t.obj_id);
    if id(&old.target) != id(&new.target) {
        match &new.target {
            Some(t) => rt.fire_event("TARGET_CHANGED", (super::unit_id::format(t.obj_id), "npc")),
            None => rt.fire_event("TARGET_CHANGED", (mlua::Value::Nil, mlua::Value::Nil)),
        }
    } else if let (Some(o), Some(n)) = (&old.target, &new.target) {
        if (o.hp, o.max_hp) != (n.hp, n.max_hp) {
            rt.fire_event("TARGET_HEALTH_CHANGED", (n.hp as i64, n.max_hp as i64));
        }
        if (o.mp, o.max_mp) != (n.mp, n.max_mp) {
            rt.fire_event("TARGET_MANA_CHANGED", (n.mp as i64, n.max_mp as i64));
        }
    }
}

fn push_state(rt: &UiRuntime, bridge: &Shared) {
    let (old, new) = {
        let mut d = bridge.borrow_mut();
        if d.pushed.as_ref() == Some(&d.snapshot) {
            return;
        }
        let new = d.snapshot.clone();
        (d.pushed.replace(new.clone()), new)
    };
    let Some(old) = old else { return };
    let lua = &rt.lua;
    let Ok(hud) = lua.globals().raw_get::<Table>("HUD") else { return };
    if let Err(e) = push_snapshot(lua, &hud, &new) {
        warn!("hud: state: {e}");
        return;
    }
    let script_id = super::unit_id::from_private(&new.unit_id).unwrap_or_default();
    let id = script_id.as_str();
    if (old.hp, old.max_hp) != (new.hp, new.max_hp) {
        rt.fire_event("PLAYER_HEALTH_CHANGED", (new.hp as i64, new.max_hp as i64));
    }
    if (old.mp, old.max_mp) != (new.mp, new.max_mp) {
        rt.fire_event("PLAYER_MANA_CHANGED", (new.mp as i64, new.max_mp as i64));
    }
    if old.level != new.level {
        rt.fire_event("LEVEL_CHANGED", (new.level as i64, id));
    }
    if old.ability_exp != new.ability_exp {
        // Refresh the book's level and progress together. ABILITY_EXP_CHANGED also prints a gain in chat;
        // these snapshots carry totals, including initial sync and ability swaps, rather than an earned amount.
        rt.fire_event("PLAYER_ABILITY_LEVEL_CHANGED", ());
    }
    // EXP_CHANGED carries the amount gained: it comes with the server's event (`combat::route_combat`)
    // each labor change the server announced: the chat scripts print "Gained 10 Labor Point(s). (Total LP: 105)"
    // and the labor bar refreshes, both from the state pushed just now
    let fresh = new.labor_seq.saturating_sub(old.labor_seq) as usize;
    for (amount, total) in &new.labor_log[new.labor_log.len().saturating_sub(fresh)..] {
        rt.fire_event("LABORPOWER_CHANGED", (i64::from(*amount), i64::from(*total)));
    }
    if old.in_combat != new.in_combat {
        rt.fire_event("UNIT_COMBAT_STATE_CHANGED", (new.in_combat, id));
    }
    if old.recoverable_exp != new.recoverable_exp {
        rt.fire_event("RECOVERABLE_EXP", (id, new.recoverable_exp as i64));
    }
    if old.sub_zone != new.sub_zone && !new.sub_zone.is_empty() {
        rt.fire_event("ENTERED_SUBZONE", (0i64, new.sub_zone.as_str()));
    }
    if old.bank_money != new.bank_money {
        rt.fire_event("PLAYER_BANK_MONEY", (new.bank_money - old.bank_money, (new.bank_money - old.bank_money).to_string()));
    }
    if old.money != new.money {
        rt.fire_event("PLAYER_MONEY", (new.money - old.money, (new.money - old.money).to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_to_subtracts_both_radii_in_the_ground_plane_and_clamps() {
        let from = Vec3::new(0.0, 0.0, 5.0);
        let to = Vec3::new(4.0, 0.0, 0.0);
        // x/y only: the 5 m height difference does not count
        assert_eq!(distance_to(from, 1.0, to, 2.0), 1.0);
        assert_eq!(distance_to(from, 3.0, to, 4.0), 0.0);
    }

    #[test]
    fn received_feature_level_caps_are_numeric_and_preserve_zero_without_xp_defaults()
    -> mlua::Result<()> {
        let lua = Lua::new();
        lua.load("X2Player={GetFeatureSet=function()return{levelLimit=false,mateLevelLimit=false,premium=false}end}").exec()?;
        let bridge = Rc::new(RefCell::new(HudData::default()));
        install_feature_limits(&lua, &bridge)?;
        lua.load("local f=X2Player:GetFeatureSet();assert(f.levelLimit==0 and f.mateLevelLimit==0 and f.premium==false)").exec()?;
        bridge.borrow_mut().feature_limits = (Some(55), Some(50));
        lua.load(
            "local f=X2Player:GetFeatureSet();assert(f.levelLimit==55 and f.mateLevelLimit==50)",
        )
        .exec()?;
        bridge.borrow_mut().feature_limits = (Some(0), Some(255));
        lua.load(
            "local f=X2Player:GetFeatureSet();assert(f.levelLimit==0 and f.mateLevelLimit==255)",
        )
        .exec()
    }

    #[test]
    fn shop_feature_uses_received_configuration_without_menu_or_missing_byte_defaults()
    -> mlua::Result<()> {
        let lua = Lua::new();
        lua.load("X2Player={GetFeatureSet=function()return{ingamecashshop=true,premium=true}end};X2InGameShop={}").exec()?;
        let bridge = Rc::new(RefCell::new(HudData::default()));
        install_feature_limits(&lua, &bridge)?;
        let mut world = World::new();
        world.init_resource::<Player>();
        world.init_resource::<aa_net::GameConfig>();
        let received = |features| aa_net::InitialConfig {
            host: Vec::new(),
            feature_bytes: features,
            raw_candidate_count: 0,
            candidate_codes: Vec::new(),
            initial_lp: 0,
            service_flags: [0; 7],
            second_price_type: 0,
            second_password_max_fail_count: 0,
            idle_kick_time: 0,
        };
        refresh_snapshot(&mut world, &bridge);
        assert_eq!(bridge.borrow().in_game_shop_enabled, None);
        for features in [
            vec![],
            vec![0; 4],
            vec![0; 5],
            vec![0, 0, 0, 0, 0x40],
            vec![0, 0, 0, 0, 0xff],
            vec![0; 4],
        ] {
            let expected = features.get(4).map(|byte| byte & 0x40 != 0);
            world
                .resource_mut::<aa_net::GameConfig>()
                .apply(&aa_net::Event::InitialConfig(received(features)));
            refresh_snapshot(&mut world, &bridge);
            assert_eq!(bridge.borrow().in_game_shop_enabled, expected);
            let enabled = expected == Some(true);
            assert_eq!(
                lua.load("return X2Player:GetFeatureSet().ingamecashshop")
                    .eval::<bool>()?,
                enabled
            );
            assert_eq!(
                lua.load("return X2InGameShop:IsInGameShopEnable()")
                    .eval::<bool>()?,
                enabled
            );
            assert!(
                lua.load("return X2Player:GetFeatureSet().premium")
                    .eval::<bool>()?
            );
        }
        world
            .resource_mut::<aa_net::GameConfig>()
            .apply(&aa_net::Event::InitialConfig(received(vec![
                0, 0, 0, 0, 0x40,
            ])));
        for result in [0, 1, 2, 255] {
            let event = aa_net::Event::CashShop(aa_net::CashShopEvent::NativeMenuList(
                aa_net::cash_shop::NativeInGameShopMenu {
                    result,
                    main_tabs: [0; 6],
                    sub_tabs: [[0; 7]; 6],
                },
            ));
            assert!(!world.resource_mut::<aa_net::GameConfig>().apply(&event));
            refresh_snapshot(&mut world, &bridge);
            assert_eq!(bridge.borrow().in_game_shop_enabled, Some(true));
        }
        world.resource_mut::<aa_net::GameConfig>().received = None;
        refresh_snapshot(&mut world, &bridge);
        assert_eq!(bridge.borrow().in_game_shop_enabled, None);
        assert!(
            !lua.load("return X2InGameShop:IsInGameShopEnable()")
                .eval::<bool>()?
        );
        Ok(())
    }

    #[test]
    fn feature_wrapper_releases_hud_borrow_before_calling_the_previous_getter() -> mlua::Result<()>
    {
        let lua = Lua::new();
        let bridge = Rc::new(RefCell::new(HudData::default()));
        let state = bridge.clone();
        let player = lua.create_table()?;
        player.set(
            "GetFeatureSet",
            lua.create_function(move |lua, _: Value| {
                state
                    .try_borrow_mut()
                    .expect("feature wrapper kept the HUD borrowed across Lua reentry")
                    .local_combat = true;
                lua.create_table()
            })?,
        )?;
        lua.globals().set("X2Player", player)?;
        install_feature_limits(&lua, &bridge)?;
        lua.load("X2Player:GetFeatureSet()").exec()?;
        assert!(bridge.borrow().local_combat);
        Ok(())
    }

    #[test]
    fn chat_requests_apply_native_transport_encoding_once() {
        let bridge = Rc::new(RefCell::new(HudData::default()));
        bridge.borrow_mut().requests.push(Request::Chat {
            chat_type: -3,
            target: "Peer".into(),
            message: "@name |literal\r\n|q42; |i5,2,payload;".into(),
        });
        let mut app = App::new();
        app.insert_non_send(HudBridge(bridge)).init_resource::<NextState<crate::Screen>>()
            .init_resource::<aa_net::state::Units>()
            .init_resource::<aa_net::state::ActionBar>().init_resource::<aa_net::state::Inventory>()
            .init_resource::<aa_net::state::Target>().init_resource::<aa_net::state::Skills>().init_resource::<aa_net::state::Buffs>()
            .init_resource::<super::super::usable::InFlight>()
            .add_message::<NetCommand>().add_message::<super::super::alerts::UseFailed>()
            .add_message::<crate::ingame::ground_target::SkillPressed>().add_systems(Update, handle_requests);
        app.update();
        let commands: Vec<_> = app.world_mut().resource_mut::<Messages<NetCommand>>().drain().collect();
        assert!(matches!(&commands[..], [NetCommand(Command::Chat { chat_type:-3,target,message })]
            if target=="Peer" && message=="@@name ||literal |q42; |i5,2,payload;"));
    }

    #[test]
    fn action_bar_page_api_is_one_based_and_maps_the_main_bar() {
        let mut rt = UiRuntime::new("en_us", 1280.0, 720.0).unwrap();
        let bridge = Rc::new(RefCell::new(HudData::default()));
        install(&mut rt, &bridge).unwrap();
        mark_pushed(&bridge);
        rt.lua.load(r#"
            pageProbe=UIParent:CreateWidget('emptywidget','pageProbe','UIParent')
            pageProbe:Show(true)
            pageProbe:RegisterEvent('ACTION_BAR_PAGE_CHANGED')
            pageEvents={}
            pageProbe:SetHandler('OnEvent',function(self,event,page) pageEvents[#pageEvents+1]=page end)
        "#).exec().unwrap();
        assert_eq!(rt.lua.load("return X2Action:GetActionBarPage()").eval::<i64>().unwrap(), 1);
        rt.lua.load(r#"
            assert(X2Action:SetActionBarPage(3)==3)
            assert(X2Action:GetActionBarPage()==3)
            assert(HUD.action_slot(1)==61 and HUD.action_slot(12)==72)
            assert(HUD.action_slot(13)==13 and HUD.action_slot(24)==24)
            assert(X2Action:SetActionBarPage(3)==3)
            assert(#pageEvents==2)
            assert(X2Action:SetActionBarPage(5)==5)
            assert(X2Action:GetActionBarPage()==3)
        "#).exec().unwrap();
        assert_eq!(bridge.borrow().action_page, 2);
        assert_eq!(rt.lua.load("return pageEvents").eval::<Vec<i64>>().unwrap(), vec![3, 3]);
        assert_eq!(bridge.borrow().requests, vec![
            Request::SetActionBarPage { page: 2 },
            Request::SetActionBarPage { page: 2 },
        ]);
    }

    #[test]
    #[ignore = "loads the original HUD fonts from game_pak; run with AA_PAK set"]
    fn pet_action_slot_clicks_queue_the_native_pet_action() {
        let mut rt = UiRuntime::new("en_us", 1280.0, 720.0).unwrap();
        let bridge = Rc::new(RefCell::new(HudData::default()));
        install(&mut rt, &bridge).unwrap();
        rt.lua
            .load("AAUI.UseSlot(ISLOT_PET_ACTION,2,'LeftButton')")
            .exec()
            .unwrap();
        assert_eq!(
            bridge.borrow().requests,
            vec![Request::UsePetAction { slot: 2 }]
        );
    }

    #[test]
    fn page_requests_apply_and_action_slots_follow_the_page() {
        let bridge = Rc::new(RefCell::new(HudData::default()));
        bridge.borrow_mut().requests.push(Request::SetActionBarPage { page: 1 });
        let mut bar = aa_net::state::ActionBar::default();
        bar.slots = vec![aa_net::items::ActionSlot::default(); aa_net::items::ACTION_SLOTS];
        bar.slots[49] = aa_net::items::ActionSlot { kind: aa_net::items::ActionKind::Spell, id: 18132 };
        let handle = bridge.clone();
        let mut app = App::new();
        app.insert_non_send(HudBridge(bridge))
            .insert_resource(bar)
            .init_resource::<NextState<crate::Screen>>()
            .init_resource::<aa_net::state::Inventory>()
            .init_resource::<aa_net::state::Target>()
            .init_resource::<aa_net::state::Skills>()
            .init_resource::<aa_net::state::Buffs>()
            .init_resource::<super::super::usable::InFlight>()
            .init_resource::<aa_net::state::Units>()
            .add_message::<NetCommand>()
            .add_message::<super::super::alerts::UseFailed>()
            .add_message::<crate::ingame::ground_target::SkillPressed>()
            .add_systems(Update, handle_requests);
        app.update();
        assert_eq!(app.world().resource::<aa_net::state::ActionBar>().page(), 1);
        assert_eq!(handle.borrow().action_page, 1);
        handle.borrow_mut().requests.push(Request::UseAction { slot: 1 });
        app.update();
        let pressed: Vec<_> = app.world_mut().resource_mut::<Messages<crate::ingame::ground_target::SkillPressed>>().drain().collect();
        assert!(matches!(&pressed[..], [skill] if skill.skill == 18132), "{pressed:?}");
    }

    #[test]
    fn money_events_use_numeric_delta_and_decimal_delta_string() {
        let mut rt=UiRuntime::new("en_us",1280.0,720.0).unwrap();
        let bridge=Rc::new(RefCell::new(HudData::default()));
        bridge.borrow_mut().snapshot.money=33;
        install(&mut rt,&bridge).unwrap();
        mark_pushed(&bridge);
        rt.lua.load(r#"
            moneyProbe=UIParent:CreateWidget('emptywidget','moneyProbe','UIParent')
            moneyProbe:Show(true)
            moneyProbe:RegisterEvent('PLAYER_MONEY'); moneyProbe:RegisterEvent('PLAYER_BANK_MONEY')
            moneyEvents={}
            moneyProbe:SetHandler('OnEvent',function(self,event,delta,text)
                assert(type(delta)=='number' and type(text)=='string')
                moneyEvents[event]={delta,text}
            end)
        "#).exec().unwrap();
        { let mut d=bridge.borrow_mut(); d.snapshot.money=28; d.snapshot.bank_money=5; }
        push_changes(&rt,&bridge);
        rt.lua.load("assert(moneyEvents.PLAYER_MONEY[1]==-5 and moneyEvents.PLAYER_MONEY[2]=='-5'); assert(moneyEvents.PLAYER_BANK_MONEY[1]==5); assert(HUD.state.money=='28' and HUD.state.bank_money=='5')").exec().unwrap();
    }

    #[test]
    fn empty_ability_learning_is_unscoped_and_rechecked_when_flushed() -> Result<(), Box<dyn std::error::Error>> {
        let mut rt = UiRuntime::new("en_us", 1280., 720.)?;
        let bridge = Rc::new(RefCell::new(HudData::default()));
        install(&mut rt, &bridge)?;
        bridge.borrow_mut().snapshot.abilities = vec![1];
        rt.lua.load("X2Ability:LearnAbility(2);HUD.request('swap_ability',1,3)").exec()?;
        assert!(matches!(&bridge.borrow().requests[..], [Request::SwapAbility { old:11, ability:2, npc_token:None }]));
        let mut app = App::new();
        app.insert_non_send(HudBridge(bridge.clone())).init_resource::<NextState<crate::Screen>>()
            .init_resource::<aa_net::state::Units>()
            .init_resource::<aa_net::state::ActionBar>().init_resource::<aa_net::state::Inventory>()
            .init_resource::<aa_net::state::Target>().init_resource::<aa_net::state::Skills>().init_resource::<aa_net::state::Buffs>()
            .init_resource::<super::super::usable::InFlight>()
            .add_message::<NetCommand>().add_message::<super::super::alerts::UseFailed>()
            .add_message::<crate::ingame::ground_target::SkillPressed>().add_systems(Update, handle_requests);
        app.update();
        let commands: Vec<_> = app.world_mut().resource_mut::<Messages<NetCommand>>().drain().collect();
        assert!(matches!(&commands[..], [NetCommand(Command::SwapAbility { old:11, ability:2 })]));
        rt.lua.load("X2Ability:LearnAbility(2)").exec()?;
        bridge.borrow_mut().snapshot.abilities = vec![1,3,4];
        app.update();
        assert!(app.world().resource::<Messages<NetCommand>>().is_empty(), "a slot filled after enqueue is not learnable");
        Ok(())
    }

    #[test]
    fn automatic_resurrection_uses_the_native_five_minute_deadline() {
        assert_eq!(death_timeout(None, 1000), 0);
        assert_eq!(death_timeout(Some(1000), 1000), 300);
        assert_eq!(death_timeout(Some(1000), 1001), 299);
        assert_eq!(death_timeout(Some(1000), 11_000), 290);
        assert_eq!(death_timeout(Some(1000), 301_000), 0);
        assert_eq!(death_timeout(Some(1000), 401_000), 0);
    }

    /// An Archery character (ability 6) with 2000 ability exp: what the scripts read in `HUD.state`.
    #[test]
    fn abilities_reach_the_scripts() {
        let lua = Lua::new();
        let hud = lua.create_table().unwrap();
        let mut ability_exp = [0; 11];
        ability_exp[6] = 2_000;
        ability_exp[1] = 50; // Battlerage was played a little before: its level shows, it is not active
        let s = Snapshot { abilities: vec![6], ability_exp, ..Snapshot::default() };
        push_snapshot(&lua, &hud, &s).unwrap();
        let state: Table = hud.get("state").unwrap();
        let rows: Table = state.get("abilities").unwrap();
        assert_eq!(rows.raw_len(), 1);
        let row: Table = rows.get(1).unwrap();
        assert_eq!(row.get::<i64>("type").unwrap(), 6);
        // 2000 exp: level 3 (1400..3200), 600 of 1800 into it
        assert_eq!(row.get::<i64>("level").unwrap(), 3);
        assert_eq!((row.get::<i64>("prevLevelTotalExp").unwrap(), row.get::<i64>("nextLevelTotalExp").unwrap()), (1400, 3200));
        assert!((row.get::<f64>("levelPercent").unwrap() - 100.0 / 3.0).abs() < 1e-9);
        let levels: Vec<i64> = state.get::<Table>("ability_levels").unwrap().sequence_values().collect::<mlua::Result<_>>().unwrap();
        assert_eq!(levels, [1, 0, 0, 0, 0, 3, 0, 0, 0, 0]);
    }
}
