# aa-ui: the X2 UI widget toolkit in Bevy

The ArcheAge 1.2 UI is Lua (the `.alb` bytecode under `scriptsbin/x2ui`) driving a native widget
toolkit inside `x2game.dll`. `crates/aa-ui` is that toolkit in Rust: the original scripts run
unchanged in Lua 5.1 (through `aa-lua`) and build the screens the way the client does; aa-ui
lays the widgets out, draws them with Bevy and feeds mouse and keyboard input back to the
scripts.

Status: world select, character select, the 2D layer of character create (all three steps,
every race and gender) and the in-game HUD after spawning run from the original scripts with no
script errors. The layout tree matches the real client's capture (`reference/*/ui.json`) by
object path (class, offset, extent, visibility):

| screen | reference | match |
|---|---|---|
| world select, 0 / 1 characters, nothing / a row selected | `reference/world_select/ui*.json` | 51-53 of 51-53 |
| world select at 1600x900 | `reference/world_select/ui_1600x900.json` | 51 of 51 |
| character select, 0 / 1 characters | `reference/character_select/*/ui.json` | 53 of 53, 58 of 58 |
| character create step 1, 8 race/gender combinations | `reference/character_create/step1_*` | 48 of 49 each (*) |
| character create step 2, 8 race/gender combinations | `reference/character_create/step2_*` | 139 of 139 each |
| character create step 3, 8 race/gender combinations | `reference/character_create/step3_*` | 44 of 44 each |
| character create step 3, skillset picked and a name entered | `step3_nuian_male/ui_ability_name.json` | 52 of 52 |
| in-game HUD after spawning (fake game data like the capture) | `reference/ingame/spawn/ui.json` | 271 of 277 (**) |

(*) The one left is `step1.content.congestionTip_bg`, a drawable of a hidden tooltip that the
capture reports by its own visible flag; not drawn by either.

(**) Of the 6 left, 2 are the same objects under a path through a table keyed by the widget's
runtime id (`exp_bar_set.1105.exp_bar...`; ours has `6075`), and 4 are within 1 px: two text
widths, the roadmap cursor's sub-pixel offset and `notifyWindow`'s height (the client had run
`systemLayerParent`'s `OnScale` handler, which sets it to the line height + 1; aa-ui fires
`OnScale` only when the screen size changes).

In-game windows (no real captures; laid out by the original scripts, fake data in
`examples/fake/fake_<window>.lua`, opened with `--open ACTION T` or the keys, no script errors):

| window | opened by | state |
|---|---|---|
| bag | `toggle_bag` (I, B) | tabs (All + the DB's default tabs, add-tab button), items with icons, counts, grades, the new-item mark, purse with coin icons, filtering by tab |
| character info | `toggle_character` (C) | stats panel, equipment slots with the starter gear; the model between the slot columns is the engine's (no frame there by design) |
| skills | `toggle_spellbook` (K) | the five tabs: Battlerage's skills and passives from the DB (learned / not yet), skill points, the locked skillset columns; basic skills, emotes, vocations with points, languages |
| quest journal | `toggle_quest` (L) | three DB quests in category folders (open / close), selecting one fills the detail panel (summary, objectives, rewards, the giver's lines); ticking one adds it to the HUD tracker (empty at start, as in the spawn capture) |
| world map | `toggle_worldmap` (M) | zone / continent / world maps from `ui/map`, hover highlights and click zoom by the colour masks, wheel zoom and panning (the scripts'), player cursor with heading, My Location, the filter panel |
| system menu | Escape, `open_config` | Options, Select Character, Exit Game |
| options | system menu -> Options, `toggle_option` | every page builds (screen, quality, sound, game, key bindings with the default keys) |
| target frame | `TARGET_CHANGED(unitId, "npc")` | name, level, health and mana, distance (`FAKE.set_target()`) |

HUD behaviour beyond the spawn capture: cooldown sweeps with the time left on slots
(`--cooldown`), the roadmap's quest / NPC markers and the player cursor turned by the heading
(the capture's quest marker and cursor match `real.png`), chat scrolling with the wheel and the
frame's scroll buttons.

Pixels: the world select screenshot matches `reference/world_select/real.png` to 0 px in
geometry and text placement; the remaining difference (mean absolute error about 10 of 255) is
tone, see [Known differences](#known-differences). A static screen costs nothing per frame
(meshes are rebuilt only on change): 300-490 FPS at 1920x1080 on the dev machine, bound by
Bevy's frame overhead, not by the UI.

## Running the examples

```text
cargo run -p aa-ui --example world_select -- --shot shots/aa-ui/world_select.png
cargo run -p aa-ui --example character_select -- --shot shots/aa-ui/character_select.png
cargo run -p aa-ui --example character_create -- --shot shots/aa-ui/character_create.png
cargo run -p aa-ui --example character_create -- --race elf --gender female --step 2
cargo run -p aa-ui --example ingame_hud -- --shot shots/aa-ui/ingame_hud.png
```

They run the screen's scripts with a fake game API (`examples/fake/`: 3 servers, one
character; `fake_ingame.lua`: the spawn capture's character, bars, buffs, zone and chat) and
are interactive: hover and click the server list, the character list and the buttons, type into
edit boxes. FPS, quads and draw calls are logged every few seconds.

| flag | meaning |
|---|---|
| `--shot PATH`, `--shot-after S`, `--exit-after S`, `--size WxH` | from `aa-dev`: screenshot, quit, window size in physical pixels |
| `--worlds ref0\|ref1` | world list of the reference capture (one world, 0 / 1 characters) instead of 3 servers |
| `--characters 0\|1` | character select with no / one character |
| `--race nuian\|elf\|ferre\|hariharan`, `--gender male\|female`, `--step 1\|2\|3` | character create like the capture (button clicks, then `characterCreate:OnNextStep()`) |
| `--click X Y T`, `--rclick X Y T`, `--hover X Y T`, `--wheel X Y DELTA T` | scripted mouse (physical pixels) at T seconds, through [`UiVirtualPointer`](#app-integration) (wheel notches, > 0 up) |
| `--cooldown` | in game: an action bar skill cooling down (the spawn capture has none) |
| `--open ACTION T` | in game: a key binding action (`toggle_bag`, `toggle_character`, `toggle_spellbook`, `toggle_quest`, `toggle_worldmap`, `open_config`) or a `UIC_*` content |
| `--lua CODE T` | run Lua at T seconds |
| `--type TEXT T`, `--key NAME T` | type into the focused edit box / press `enter`, `escape`, `backspace`, `delete`, `tab`, `left`, `right`, `home`, `end` |
| `--dump PATH`, `--dump-after S` | write the tree in the `reference/*/ui.json` format |
| `--coverage` | print the per-class native coverage table (below) |

Reference checks (tools in `data/extract/aa-ui/tools/`, not committed):
`check_all.sh` runs every reproducible capture and compares the dumps with
`cmp_tree.py <reference ui.json> <our dump>` (by object path: class, offset, extent,
visibility); `check_windows.sh` opens every in-game window and counts script warnings;
`imgdiff.py` and `crops.py` compare screenshots. Both scripts build the examples once and run
copies of the exes, so repeated runs do not hold the shared cargo build lock.

## Architecture

```text
 game_pak scriptsbin/x2ui/*.alb ──aa-lua──> Lua 5.1 state
                                              │  UIParent:CreateWidget, widget:AddAnchor, ...
                                              v
 lua/ (natives, handlers, events) ───────> tree.rs (widgets, drawables, anchors -> rects)
          ^                                   │
          │ OnClick, OnEnter, OnTextChanged   │ paint order, visible state
 input.rs, lua/edit.rs <── Bevy mouse/keys    v
                                           render.rs ──> batched meshes (one per texture run)
                                              ^
                       text.rs (fonts, glyph atlas), texture.rs (pak DDS, .g info)
```

| module | role |
|---|---|
| `lib.rs` | `UiPlugin`, `UiRuntime` (the Lua state), Bevy systems (`init_runtime`, `ui_keyboard`, `ui_frame`) |
| `tree.rs` | objects (widgets and drawables) in a `Vec` by id, anchors, rectangle cache, paint order |
| `lua/mod.rs` | `Ui` state (Lua app data), object tables, handler dispatch, UI events, the per-frame `tick` (fades, `OnUpdate`, queued events) |
| `lua/widget.rs` | the widget natives: bounds, anchors, visibility, text, drawables, buttons, lists, sliders |
| `lua/drawable.rs` | drawable natives (`SetTexture`, `SetCoords`, `SetColor`, ...) |
| `lua/folder.rs`, `lua/tab.rs`, `lua/edit.rs`, `lua/diagram.rs` | `folder`, `tab`, edit box and `circlediagram` behaviour |
| `lua/slot.rs`, `lua/statusbar.rs` | `slot` (action bar and skill icons bound to game slots, cooldown sweeps) and `statusbar` (HP / MP / exp / labor bars) |
| `lua/grid.rs` | `grid` cells, rows and selection (the quest list and the quest tracker) |
| `lua/bounds.rs` | `OnBoundChanged` after layout changes |
| `lua/chat.rs` | `message` frames and the tabbed `chatwindow`, `X2Chat:DispatchChatMessage` |
| `lua/roadmap.rs` | the `roadmap` zone map overlay (player cursor, markers); `listbox` tree images |
| `lua/worldmap.rs` | the `worldmap` window: map levels, colour-mask hit testing (`ui/map/image_map.tga`), zoom by click, player cursor |
| `content.rs` | key binding actions and Escape (`UiRuntime::hotkey`, `escape`, `toggle_content`) |
| `lua/globals.rs` | `UIParent` / `UI`, constants, `Script`, the toolkit side of `X2Locale`, `ADDON`, `X2DialogManager`, game API stubs |
| `lua/classes.rs` | the exact method list of every native class (from `reference/api/widget_methods.json`) |
| `lua/constants.rs` | the client's real constant values (`x2_globals`) |
| `lua/stubs.rs` | logging stubs for natives we do not implement, coverage accounting |
| `text.rs` | `fonts.g` fallback chains, client line metrics, layout (wrap, colour and value codes), glyph atlas |
| `texture.rs` | texture path resolution (locale copies), sizes, `.g` texture info (`GetTextureData`) |
| `render.rs` | tree to quads, batching, atlas upload, state backgrounds, caret, diagram polygons |
| `input.rs` | hit testing, mouse handlers, native button / check button / list / tab / focus behaviour |
| `dump.rs` | tree dump in the reference format (paths from `_G` and from upvalues, walked in Lua) |

### Objects

Every widget, drawable and text style is a plain Lua table, so scripts can add fields and
override methods per instance (`frame.InsertColumn = function ... end`), as they do. The id of
an object lives outside the table (a weak-keyed table maps table to id), so `pairs(widget)` sees
only script fields. The native methods are in one method table per class, reached through the
metatable's `__index`.

Each class's method table is exactly the client's (`lua/classes.rs`): natives we implement are
real, the others are logging stubs (they return nothing; `stub_calls()` counts them), and a
method the class does not have is absent. That matters: scripts test methods
(`if widget.SetChecked then`) and fields (`if self.bgs == nil`). `CreateChildWidget(type,
name, index, reflect)` creates the child and, when `reflect` is set, stores it on the parent
(`parent[name]` or `parent[name][index]`); `widget.style` / `titleStyle` are text style objects.

### Layout

Coordinates are UI units, origin top-left, y down; 1 unit is 1 pixel at UI scale 1.0, which the
client uses for both 1920x1080 and 1600x900 (0.85 below 1280x864). Widgets do not scale with
the resolution: they are anchored (`AddAnchor(point, target, relPoint, x, y)`, nine points),
and a widget with two anchors on opposite sides stretches. `tree.rs` resolves anchors to
rectangles lazily and caches them per tree generation.

Drawing order: top-level widgets by UI layer (`SetUILayer`: background, game, hud, normal,
dialog, tooltip, system), then creation / `Raise` order; children above their parent; inside a
widget the drawable layers background, artwork, then the widget's own text, then overlay.

### Rendering

`render::build_quads` walks the paint order and emits textured, vertex-coloured quads in
physical pixels; consecutive quads with the same texture share one mesh. Everything is rebuilt
only when the tree generation changes (any mutation bumps it) or the window is resized, so a
static screen costs no CPU per frame. Glyphs live in one 2048x2048 atlas (which also holds the
white texel for solid quads). Textures are the pak's DDS files, decoded by Bevy (DXT1, DXT5,
A8R8G8B8 so far).

Blending happens in gamma space (`UiConfig::gamma_blending`, `CompositingSpace::Srgb` on the
UI camera) like the client's Direct3D renderer: with linear blending the semi-transparent
panels come out visibly too light.

### Text

Fonts come from the pak through `fonts/fonts.g` (a primary font plus fallbacks for Unicode
ranges, per locale), rasterised with `swash` at integer sizes with hinting; advances are whole
pixels, so measured widths (`GetTextWidth`) and drawn text agree. Line height is the font's
ascent + descent rounded (yd_ygo540: 13 px -> 16, 15 -> 18, 18 -> 22), a text's height is
`n * lineHeight + (n - 1) * lineSpace`, and a trailing line break does not add a line, as in the
client. Inline codes: `|cAARRGGBB ... |r` colour; value codes `|,N;` (thousands separators),
`|mN;` (money with gold / silver / copper coin icons), `|bN;` (loyalty token icon).
Alignment without `SetAlign`: centred for buttons, labels and `textbox` (world select's
warning line, which never sets one, is centred in `real.png`), top left for game tooltips,
left for the rest.
`GetLineCount()` is the wrapped line count of the widget's text.

### Handlers and events

`SetHandler(name, fn)` stores handlers per object; `RegisterEvent(name)` subscribes to a UI
event, delivered as `OnEvent(self, name, ...)`. Natives that fire handlers (`Select`,
`SetChecked`, `SelectTab`, ...) queue the call and run it once the native returns, so a handler
never runs inside a borrowed native. `UiRuntime::fire_event` raises an event now;
`queue_event` raises it at the next frame (use it for game API answers that the client gets
from the server later). Every frame `tick` advances fades (`Show(true, ms)`), fires
`OnEndFadeIn` / `OnEndFadeOut` and calls `OnUpdate(self, dtMs)` and `OnEffect(self, dtMs)` on
visible widgets. `OnBoundChanged(self)` runs in the tick after a widget's laid-out rectangle
changed (the first layout counts, hidden widgets too; up to 4 passes when a handler moves other
widgets that have one): the skill window's scroll windows place their columns there.
`folder:ToggleState` raises `FOLDER_STATE_CHANGED(title)`.

### Input

Mouse: the topmost widget under the cursor that takes the mouse (buttons, check buttons, edit
boxes, sliders, list boxes, or anything with a mouse handler) by layer and paint order; a
visible modal window (`SetWindowModal`) blocks everything outside it. It gets `OnEnter` /
`OnLeave`, `OnMouseDown` / `OnMouseUp`, `OnClick(self, "LeftButton" | "RightButton")` (on the
nearest ancestor with a handler, if enabled), `OnWheelUp` / `OnWheelDown` (on the widget under
the cursor or its nearest ancestor with one; a wheel no script takes scrolls the message frame
under the cursor, 3 messages a notch). Native behaviour:
button state art (normal / highlight / pushed / disabled), check buttons toggle after their
`OnClick` ran (the scripts read the old state there: the quest list's `show = not
self:GetChecked()`, the radio buttons) and fire `OnCheckChanged`, grid rows select on a click
(`OnSelChanged`), list rows with data highlight and select (`OnSelChanged`,
double clicks), tab buttons select their tab (`OnTabChanged`), chat tab buttons select their
chat tab, chat frames show their scroll buttons (up, down, to the bottom) while hovered and keep
their view when lines arrive while scrolled back. Sliders: a press on the track or the thumb sets the value so the thumb centre is
under the cursor, dragging keeps updating it (captured until release, also outside the widget),
clamped and snapped to `SetValueStep`; each change fires `OnSliderChanged(self, value)`. The
thumb (`SetThumbButtonWidget`) is placed natively along the track by the value.

Keyboard: one widget has the focus (`SetFocus`, or a press on an edit box; `ClearFocus`, Escape
or a press elsewhere drops it; `OnAcceptFocus` on gain). Keys go to it: edit boxes insert typed
text at the caret (with `SetMaxTextLength`, `SetDigit`, `SetDigitMax`, `SetReadOnly`;
`SetPassword` shows `*`) and fire `OnTextChanged(self)`; Backspace, Delete, Left, Right, Home,
End edit and move the caret; Enter fires `OnEnterPressed` (multi-line boxes insert a line
break, `ClearTextOnEnter` empties the box), Escape `OnEscapePressed`, Tab `OnTabPressed`; every
key also fires `OnKeyDown` / `OnKeyUp(self, name)` with lower-case names (`"enter"`). `SetText`
does not fire `OnTextChanged` (the money edits call `SetText` from that handler). The caret
blinks every 530 ms.

## App integration

```rust
use bevy::prelude::*;

App::new()
    .add_plugins((DefaultPlugins, aa_ui::UiPlugin::default()))
    .insert_resource(aa_ui::UiSetup::new(|ui| {
        // 1. game API objects (called while the scripts load)
        let x2world = aa_ui::lua::install_game_stub(&ui.lua, "X2World").unwrap();
        x2world
            .raw_set("GetWorldList", ui.lua.create_function(|lua, _: mlua::Value| {
                // ... a table of world infos from aa-net
                lua.create_table()
            }).unwrap())
            .unwrap();
        // 2. the screen's scripts in the client's order (init script, base contents, contents)
        ui.run_stage(aa_ui::Stage::WorldSelect);
        // 3. the events the client raises when it enters the stage
        ui.fire_event("ENTERED_WORLD_SELECT", ());
        ui.fire_event("SHOW_SERVER_SELECT_WINDOW", true);
    }))
    .run();
```

* `UiPlugin { config: Some(UiConfig { locale, camera_order, clear, gamma_blending }) }`. The UI
  draws with its own 2D camera (order 10 by default) over the 3D scene. Cameras that share the
  window's main texture must use the same `CompositingSpace`, or Bevy falls back to linear for
  all of them: give the 3D camera `CompositingSpace::Srgb` too, or set `gamma_blending: false`.
* `UiSetup` runs once, when the window size is known, with the new `UiRuntime`. The runtime is
  a Bevy non-send resource (`NonSendMut<UiRuntime>`); call it from systems for later events.
* Stage changes: the client builds a fresh UI Lua state per stage. `commands.queue(
  aa_ui::ReloadUi::new(|ui| { ... }))` (or `aa_ui::reload_ui(world, setup)` in an exclusive
  system) drops the current runtime with all its widgets and handlers and builds a new one with
  the new setup at the next update; textures and the cameras stay. See
  `examples/character_select_3d.rs --from-world-select 3`.
* 3D between the background and the panels (character select / create): set
  `UiConfig { background_camera_order: Some(-10), camera_order: 10, .. }`. Windows on the
  `background` UI layer (the login backgrounds, `SetUILayer("background")`) are then drawn by
  a second camera at `background_camera_order` (render layer `background_render_layer`, 31 by
  default), which clears; the app's `Camera3d` goes in between with `order: 0` and
  `clear_color: ClearColorConfig::None`; the UI camera at `camera_order` loads and draws the
  rest. Bevy composites a camera stack that holds a `Camera3d` in linear space, so in split mode
  the UI cameras blend in linear space too (translucent panels come out slightly lighter than
  the client; mean error 7.9 instead of 6.4 on character select). Example:
  `cargo run -p aa-ui --example character_select_3d` (a cube where the character stands).
* UI texts: `UiConfig { ui_texts_db: Some("D:/aaemu-live/rs-game/Data/compact.sqlite3".into()), .. }`
  serves `X2Locale:LocalizeUiText(category, key, ...)` from the game database (`ui_texts`
  joined with the locale column of `localized_texts`, opened read-only, cached per path and
  locale, `$1`.. replaced by the extra arguments, the key when missing). Or
  `ui.load_ui_texts(path)` / `ui.set_ui_texts(Arc<UiTexts>)` from a setup.
* Game API: `lua::install_game_stub(lua, "X2World")` registers a table where every method
  exists and returns nothing (logged once); replace methods by setting fields, from Rust
  (`create_function`) or Lua (`ui.exec`). Scripts call them as methods (`X2World:GetWorldList()`),
  so the first argument is the table. `ADDON` is the same table as `X2` in the client.
  aa-ui already provides the UI-side parts: `X2Locale:GetLocale()`, the `ADDON` content
  registry (`RegisterContentWidget`, `GetContent`, `RegisterContentTriggerFunc`,
  `ShowContent`, `ToggleContent`) and the `X2DialogManager` message boxes (`SetHandler`,
  `RequestDefaultDialog`, `RequestNoticeDialog`, `OnOK`, `OnCancel`, `DeleteByOwnerWindow`;
  the step 3 confirm dialog matches `real_confirm_dialog.png` to a mean error of 2 of 255).
  `X2Locale:LocalizeUiText(category, key)` returns the key unless `UiConfig::ui_texts_db` is
  set (below).
  `examples/fake/fake_game.lua` is a complete fake of what the login stages call, a good map of
  what the app has to provide; `docs/ui-api.md` (aa-lua) lists every call site.
* In-game HUD (`Stage::World`): the stage runs every content script (about 900 files) and the
  root scripts; then the app raises what the client raises on entering the world
  (`ENTERED_WORLD`, `LEFT_LOADING`, chat channel joins, ...; `examples/fake/fake_ingame.lua`
  `FAKE.enter_world` is the list the example uses). Native widgets that show game data ask the
  app through Lua hooks on the global table `AAUI` (aa-ui creates it empty; define the
  functions before or after the scripts load):

  | hook | used by | returns |
  |---|---|---|
  | `AAUI.SlotContent(slotType, slotIdx)` | `slot:EstablishSlot` (action bars: `ISLOT_ACTION`, 1..) | nil, or `{ type = "skill"\|"item"\|..., id, icon = "ui/icon/x.dds", stack, grade, canUse, tooltip, extraInfo, cooldown = { remain = ms, total = ms } }` (`cooldown` optional: the sweep and the time left, counted down natively; `refresh_slots` restarts it from `remain`, a result without it clears it) |
  | `AAUI.SkillContent(skillType)`, `AAUI.ItemContent(itemType, grade)` | `slot:EstablishSkill` / `EstablishItem` (the skill window) | the same, plus for skills `learned` (default true), `canLearn`, `reqPoints`, `skillLevel` (sent as `OnContentUpdated` states) |
  | `AAUI.SlotHotKey(slotType, slotIdx)` | `slot:GetHotKey` (keybinding labels) | `"1"`, `"SHIFT-1"`, `"MINUS"`, ... or nil |
  | `AAUI.WorldMap()` | the `worldmap` window (every frame while shown) | nil, or `{ level = 2\|3, zone = <zone_groups id>, player = { u, v, angle } }` on the zone (or city) map image; without it the `RoadMap` result is used |
  | `AAUI.ChatTabs(windowId)` | `chatwindow:SetChatWindowId` | `{ { id, name = "All", filters = { CMF_PARTY, ... } }, ... }` (no `filters`: every line) |
  | `AAUI.ChatColor(filter)` | `X2Chat:DispatchChatMessage` | `{ r, g, b }` or nil (a default per filter) |
  | `AAUI.RoadMap()` | the `roadmap` (every frame) | nil, or `{ level = 2, zone = 5, player = { u, v, angle }, markers = { { u, v, kind, icon, coords, tooltip }, ... } }` (level as the scripts register maps: 2 zone, 3 city; `u, v` on the map image, 0..1; `angle` the heading in radians, 0 north, clockwise, `-yaw` of the engine; `kind` a lower-case `M_*` name of `map/common.lua` such as `"quest_zone_gives"` (yellow !) or `"quest_main_gives"`, icons from `ui/map/icon/npc_icon.dds` unless `icon` / `coords` are given; `npc_*` kinds show with the map's NPC option) |

  `ui.refresh_slots()` asks the slot hooks again (bars changed); the roadmap and chat colours
  are read when needed. `X2Chat:DispatchChatMessage(filter, text)` is the toolkit's (it routes
  the line to the chat tabs whose filters take it); chat messages from the server arrive as
  UI events the chat scripts turn into `DispatchChatMessage` calls. Example:
  `cargo run -p aa-ui --example ingame_hud`.
* In-game windows from key bindings: `ui.hotkey(action)` runs one of the client's key binding
  actions and toggles the window the main menu table (`MAIN_MENU_INFO`) binds to it:
  `toggle_bag`, `toggle_character`, `toggle_spellbook`, `toggle_quest`, `toggle_worldmap`,
  `toggle_achievement`, `toggle_option`, `toggle_ranking`, `toggle_craft_book`, `toggle_friend`,
  ...; `open_config` raises `OPEN_CONFIG` (the system menu). It returns false for actions that
  are not UI windows (movement, targeting). `ui.escape()` is the Escape key outside an edit box:
  it hides the topmost shown window that called `SetCloseOnEscape(true)` (its `OnHide` runs) or
  else raises `OPEN_CONFIG`; call it when `ui.key(&UiKey::Escape)` returned false. Lower level:
  `ui.toggle_content(id)`, `ui.show_content(id, bool)`, `ui.content_visible(id)` with
  `ui.content_id("UIC_BAG")` (the scripts' `ADDON:ToggleContent` / `ShowContent`). The
  client's default keys: I and B bag, C character, K skills, L quests, M world map, Escape;
  the examples bind them (`fake::hotkeys`) and take `--open ACTION T`.
* Target: when the player targets a unit the app raises `TARGET_CHANGED(unitId, "npc")` (any
  unit; `nil, nil` clears it) and answers the `X2Unit` calls for unit `"target"` (name, level,
  health, mana, unit type, relationship, distance, buffs); `examples/fake/fake_target.lua`
  (`FAKE.set_target()`, `--lua "FAKE.set_target()" 2`) is the list.
* `fire_event(name, args)` / `queue_event(name, args)` raise UI events (`args` is anything
  `IntoLuaMulti`: `()`, `true`, `("a", 1)`).
* `key(&UiKey)` / `type_text(&str)` / `focus()` / `set_focus(id)`: keyboard (the plugin already
  forwards Bevy's `KeyboardInput`). `UiStats::keyboard_focus` tells the app an edit box has the
  keys, so it should skip its key bindings.
* `UiVirtualPointer { pos, press, release, right_release, wheel }`: drive the mouse from code
  (tests, automation); the edges and wheel notches are cleared after use.
* `exec(name, code)`, `dump_json()`, `stub_calls()`, `errors`: debugging.

## Adding a widget type

1. The class and its exact method list: `lua/classes.rs` is generated from
   `reference/api/widget_methods.json` by `data/extract/aa-ui/tools/genclasses.py`. A class the
   capture could not name goes into `EXTRA_CLASSES` (`lua/mod.rs`); it gets our natives plus
   stubs for every known method name.
2. The type: `tree::WidgetType::parse` maps the `CreateWidget` type string; add a variant if
   the type needs native state, and decide `has_text` (gets a text block and `style`) and
   `mouse_native` (takes the mouse without handlers).
3. State: a `pub struct XData` in its own module (`lua/folder.rs`, `lua/tab.rs` and
   `lua/edit.rs` are small examples) and an `Option<XData>` field on `tree::Widget`.
4. Natives: `pub(crate) fn methods(lua, &Table)` in the module, defining each method with
   `method(lua, |lua, id, args| ...)` (`args.f(i)`, `args.s(i)`, `args.b(i, default)`; index 1
   is the first argument after `self`), returning `ret(lua, value)` or `none()`. Hook it into
   `widget::methods` under `if ty == "x"`. Methods defined later override earlier ones;
   `stubs::finalize` then adds stubs for the rest of the class list and drops anything the
   class does not have.
5. Behaviour: layout that depends on children goes into `tick` (see `tab::relayout_all`: only
   write anchors that changed, so a static screen does not rebuild), mouse behaviour into
   `input::click`, drawing into `render::build_quads`. Fire handlers through
   `ui(lua).pending.push((id, "OnX".into(), args))` inside natives, `call_handler` outside.
6. Check it: a test in `tests/natives.rs` (no window needed), then a reference capture with
   `--dump` and `cmp_tree.py`.

## Native behaviours from the aa-lua handover

| behaviour | where |
|---|---|
| `CreateChildWidget(type, name, index, true)` stores the child on the parent | `widget::create_child` |
| widgets carry `style` (and windows `titleStyle`) text style objects | `widget.rs` style methods |
| `AddTab` / `AddSimpleTab` create `window[i]`, `selectedButton[i]`, `unselectedButton[i]` | `lua/tab.rs` |
| `InsertColumn` / `InsertRows` create `column[i]` and `items[i].subItems[j]` | `widget::listctrl_methods` |
| `ADDON:GetContent(id)` is nil for unregistered ids (and the widget for registered ones) | `globals.rs` (`TOOLKIT_LUA`) |
| natives return real numbers (no proxies) | all natives |
| `X2Locale:GetLocale()` returns `"en_us"` (the configured locale) | `globals.rs` |
| `IsVisible()` is the object's own flag, not its ancestors' (the system menu counts its buttons before it shows) | `widget.rs` |
| `AddNewTab(text, selectedBtn, unselectedBtn, window)` uses the script's widgets (several tabs may share a window); `RemoveAllTabs` leaves script-made widgets alive | `lua/tab.rs` |
| `gametooltip:AddLine` returns the new line's 0-based index | `widget.rs` |
| `SetCloseOnEscape(true)` windows are what `UiRuntime::escape` hides | `content.rs` |
| `EnableScroll(true)` clips the children and the picking to the widget; `ChangeChildAnchorByScrollValue(dir, v)` offsets the anchors that target it | `tree.rs`, `widget.rs` |
| folders place their title button by `SetTitleButtonWidget` / `SetInset` | `lua/folder.rs` |
| `grid:SetItem(widget, row, col, makeIfNotExist, value, withoutExtent)` puts (and shows) a widget in a cell, sized to it unless `withoutExtent`; rows grow with their tallest widget; `SetRowHeight(height, row)`, `GetAdjustRowHeight()` the summed heights; `RemoveAllItems` hides the widgets (the scripts pool them) | `lua/grid.rs` |

mlua's `send` feature is not needed: the runtime is a non-send resource and errors are
converted with `.map_err(|e| anyhow::anyhow!("{e}"))`.

## Known differences

* Tone: the real screenshots have a global tone curve (highlights compressed, 244 -> 219,
  200 -> 193, 72 -> 55) and are darker in the middle of the screen than our render; probably a
  post effect of the login stage's 3D scene or the gamma setting, not a UI element (not found in
  the scripts). The login background sits 1 px higher than in the capture.
* 3D is not drawn: the character model view, the login stage scene behind the UI.
* Fades and animations are simplified: `Show(visible, ms)` alpha fades work; `SetStartAnimation`,
  `SetAlphaAnimation` and `SetScaleAnimation` are no-ops; effect drawables show while started
  (`SetStartEffect(true)`) without their animation phases.
* In-game HUD: what the 3D engine draws is not here (name plates over units, the world, the
  player model between the equipment slot columns). Cooldown sweeps: the direction and the
  time label format (`8`, `5m`, `2h`, `6d`) are guesses, the global cooldown is not told apart.
  Roadmap markers come only from the `RoadMap` hook (the natives that feed quest / NPC data,
  `AddGivenQuestInfo`, `UpdateNpcInfo`, ..., are stubs), at 24 px at every map size. Chat
  scrolls by messages, not wrapped lines (`GetMessageLines`, `SetScrollPos` are stubs). The idle
  chat window fades its tab backgrounds out and its unselected tabs to 60 % (measured by eye on
  the capture). Named fonts the scripts ask for (`actionBar`, `characterInfo`) are not in
  `fonts.g` and fall back to the default font (logged once).
* In-game windows: item and skill tooltips lay out roughly and have no background (the
  `gametooltip` section natives are stubs); the world map draws no conflict, climate, NPC,
  doodad, quest or ship markers and no zone-name tooltips, and on the continent and world maps
  the player cursor sits at the colour of the player's zone in the mask; the quest list's tree
  lines and zebra rows (`SetDrawLineType`, `SetLineBackGround`) are not drawn; long key
  binding titles wrap into the next row (the client may fit them in 170 px).
* Many natives are stubs (table below): mostly in-game widgets (model view, the chat window's
  options, map markers fed by game data) and rarely used setters.

## API coverage

Per native class: methods implemented natively (including deliberate no-ops such as sounds and
animations), methods present as logging stubs, and the client's total. Generated with
`cargo run -p aa-ui --example ingame_hud -- --coverage`.

| class | native | stub | listed |
|---|---|---|---|
| `button` | 87 | 41 | 128 |
| `chatwindow` | 95 | 30 | 125 |
| `checkbutton` | 91 | 41 | 132 |
| `circlediagram` | 74 | 28 | 102 |
| `cooldownactionbutton` | 90 | 42 | 132 |
| `cooldownbutton` | 90 | 41 | 131 |
| `cooldownconstantbutton` | 90 | 43 | 133 |
| `cooldowninventorybutton` | 90 | 44 | 134 |
| `damagedisplay` | 68 | 32 | 100 |
| `drawable:color` | 15 | 9 | 24 |
| `drawable:effect` | 19 | 29 | 48 |
| `drawable:iconimage` | 20 | 9 | 29 |
| `drawable:image` | 23 | 14 | 37 |
| `drawable:ninepart` | 18 | 10 | 28 |
| `drawable:text` | 16 | 13 | 29 |
| `drawable:threepart` | 18 | 9 | 27 |
| `editbox` | 86 | 33 | 119 |
| `editboxmultiline` | 77 | 30 | 107 |
| `emptywidget` | 66 | 27 | 93 |
| `folder` | 82 | 32 | 114 |
| `gametooltip` | 74 | 33 | 107 |
| `grid` | 91 | 57 | 148 |
| `label` | 71 | 27 | 98 |
| `listbox` | 72 | 66 | 138 |
| `listctrl` | 78 | 31 | 109 |
| `listctrlitem` | 99 | 1943 | 2042 |
| `message` | 89 | 30 | 119 |
| `modelview` | 66 | 93 | 159 |
| `paintcolorpicker` | 107 | 1935 | 2042 |
| `roadmap` | 74 | 66 | 140 |
| `slider` | 80 | 29 | 109 |
| `slot` | 78 | 28 | 106 |
| `statusbar` | 73 | 29 | 102 |
| `tab` | 79 | 27 | 106 |
| `textbox` | 74 | 30 | 104 |
| `uiparent` | 20 | 15 | 35 |
| `webbrowser` | 70 | 55 | 125 |
| `window` | 74 | 41 | 115 |
| `worldmap` | 155 | 1890 | 2045 |

`paintcolorpicker`, `worldmap` and `listctrlitem` are not in the capture's class list, so they
get our natives plus a stub for every method name any class has. Toolkit stubs the login
screens still call (with the fake game API): `listbox` item colours (`SetDefaultItemTextColor`,
`SetOveredItemColor`, ...), `emptywidget:EnableScroll` / `ChangeChildAnchorByScrollValue`,
`drawable:image:SetAnimFrameInfo` (frame animations), `button:IgnoreInputOnFading`, and the
`gametooltip` section natives when a tooltip shows.
