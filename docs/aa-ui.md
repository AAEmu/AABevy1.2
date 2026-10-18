# aa-ui: the X2 UI widget toolkit in Bevy

The ArcheAge 1.2 UI is Lua (the `.alb` bytecode under `scriptsbin/x2ui`) driving a native widget
toolkit inside `x2game.dll`. `crates/aa-ui` is that toolkit in Rust: the original scripts run
unchanged in Lua 5.1 (through `aa-lua`) and build the screens the way the client does; aa-ui
lays the widgets out, draws them with Bevy and feeds mouse and keyboard input back to the
scripts.

Status: world select, character select, the 2D layer of character create (all three steps,
every race and gender), the in-game HUD after spawning and the in-game windows run from the
original scripts with no script errors. The layout tree matches the real client's capture
(`reference/*/ui.json`) by object path (class, offset, extent, visibility):

| screen | reference | match |
|---|---|---|
| world select, 0 / 1 characters, nothing / a row selected | `reference/world_select/ui*.json` | 51-53 of 51-53 |
| world select at 1600x900 | `reference/world_select/ui_1600x900.json` | 51 of 51 |
| character select, 0 / 1 characters | `reference/character_select/*/ui.json` | 53 of 53, 58 of 58 |
| character create step 1, 8 race/gender combinations | `reference/character_create/step1_*` | 48 of 49 each (*) |
| character create step 2, 8 race/gender combinations | `reference/character_create/step2_*` | 139 of 139 each |
| character create step 3, 8 race/gender combinations | `reference/character_create/step3_*` | 44 of 44 each |
| character create step 3, skillset picked and a name entered | `step3_nuian_male/ui_ability_name.json` | 52 of 52 |
| in-game HUD after spawning (fake game data like the capture) | `reference/ingame/spawn/ui.json` | 274 of 277 (**) |
| character info (C), objects not visible at spawn | `reference/ingame/character_info` | 182 of 182 |
| bag (I), 4 of 50 slots used | `reference/ingame/bag` | 331 of 331 |
| world map (M): zone / continent (one right click) / world (two) | `reference/ingame/worldmap_*` | 88 of 88, 86 of 86 (+5), 86 of 86 |
| options, Game Keybinds page | `reference/ingame/options_keybinds` | 180 of 182 (***) |
| tooltips: bag item (Teleport Book), action bar skill (Shoot Arrow), buff (Patron), HUD button ("Bag (I)") | `reference/ingame/tooltip_*` | 349 of 349, 19 of 19, 14 of 15, 8 of 9 (****) |

(*) The one left is `step1.content.congestionTip_bg`, a drawable of a hidden tooltip that the
capture reports by its own visible flag; not drawn by either.

(**) The 3 left are within 1 px: a text width, the roadmap cursor's sub-pixel offset and
`notifyWindow`'s height (the client had run `systemLayerParent`'s `OnScale` handler, which sets
it to the line height + 1; aa-ui fires `OnScale` only when the screen size changes). The window
captures count only the objects not also visible at spawn: the rest of their screens is the HUD
at another moment (the clock's sun, a buff that ran out).

(***) The 2 left are state backgrounds of disabled buttons that the client reports hidden: the
menu list's scroll thumb and `releaseHotkeyBtn` (disabled before its skin is applied); ours
draws them and the pixels are the same. (+5): the continent's conflict circles
(`MAP_LIST[3][i].conflictImg`, shown by the scripts) are drawn in `real.png` (same place and
colour as ours) but the capture's walk does not list them.

(****) The one left in the buff and HUD captures is the action bar's background effect: the
client's pointer had crossed the bar before, and its leave effect ends at alpha 0 but leaves it
shown. Pixels inside the tooltips (`check_ingame.sh`): mean absolute error 2.4 (item), 3.2
(skill), 7.4 (buff: the bright sky behind the translucent background), 4.3 (HUD button); the
text sits on the capture's pixels, the rest is the 3D world behind.

In-game windows (laid out by the original scripts, fake data in
`examples/fake/fake_<window>.lua` (the captured ones as in the capture), opened with
`--open ACTION T` or the keys, no script errors):

| window | opened by | state |
|---|---|---|
| bag | `toggle_bag` (I, B) | tabs (All + the DB's default tabs, add-tab button), items with icons, counts, grades, the new-item mark, purse with coin icons, filtering by tab |
| character info | `toggle_character` (C) | stats panel (the level in the `characterInfo` bitmap font, honor and vocation icons), equipment slots with the starter gear; the model between the slot columns is the engine's (no frame there by design) |
| skills | `toggle_spellbook` (K) | the five tabs: Battlerage's skills and passives from the DB (learned / not yet), skill points, the locked skillset columns; basic skills, emotes, vocations with points, languages |
| quest journal | `toggle_quest` (L) | three DB quests in category folders (open / close), selecting one fills the detail panel (summary, objectives, rewards, the giver's lines); ticking one adds it to the HUD tracker (empty at start, as in the spawn capture) |
| world map | `toggle_worldmap` (M) | zone / continent / world maps from `ui/map`, hover highlights and click zoom by the colour masks, wheel zoom and panning (the scripts'), player cursor with heading on every level, the zone's climate, the continent's conflict zones, My Location, the filter panel |
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
visibility); `check_ingame.sh` opens the captured in-game windows like the captures
(`reference/ingame/*`) and compares each with the whole screen and inside the window only
(`--new-vs` the spawn dumps: objects also visible at spawn do not count), plus the window's
pixels with `imgdiff.py`; `check_windows.sh` opens every in-game window and counts script
warnings; `imgdiff.py` and `crops.py` compare screenshots. The scripts build the examples once
and run copies of the exes (`--nobuild` reuses them), so repeated runs do not hold the shared
cargo build lock. `cmp_tree.py` drops the runtime ids that tables are keyed by from paths
(the client's `exp_bar_set.10df.exp_bar` and ours `exp_bar_set.6075.exp_bar` both compare as
`exp_bar_set.exp_bar`).

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
| `lua/tooltip.rs` | `gametooltip` lines (`AddLine`, `AddAnotherSideLine`, line spaces, `GetHeightToLastLine`), wrapping, sizing to the lines, drawing |
| `lua/slot.rs`, `lua/statusbar.rs` | `slot` (action bar and skill icons bound to game slots, cooldown sweeps) and `statusbar` (HP / MP / exp / labor bars) |
| `lua/grid.rs` | `grid` cells, rows and selection (the quest list and the quest tracker) |
| `lua/bounds.rs` | `OnBoundChanged` after layout changes |
| `lua/chat.rs` | `message` frames and the tabbed `chatwindow`, `X2Chat:DispatchChatMessage` |
| `lua/chat_input.rs` | the chat input's slash commands (`/s`, `/p`, `/w Name`, ...), channel and whisper target, sending through `AAUI.ChatSend` |
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
| `prof.rs` | opt-in frame profiler (`AA_UI_PROFILE=1`: ms per part of the UI frame, what changed the content, the costly per-frame handlers; `=2`: per native method, and which ones wrote) |
| `dump.rs` | tree dump in the reference format (paths from `_G` and from upvalues, walked in Lua 10 steps deep like `reference/tools/lib.lua`) |

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
rectangles lazily and caches them per tree generation. Re-adding a point replaces its anchor;
when two different points set the same edge (TOPLEFT and TOPRIGHT both set the top), the point
first in the order TOPLEFT, TOP, TOPRIGHT, LEFT, CENTER, RIGHT, BOTTOMLEFT, BOTTOM, BOTTOMRIGHT
wins, whatever the order they were added in (`reference/ingame/bag`: the tab line's TOPLEFT
beats its TOPRIGHT, the slot grid's later TOPLEFT beats the LEFT it was created with). An
anchor that would make an object depend on itself (directly or through its target's anchors)
is refused and the old one stays, as in the client: the tooltip scripts' `ResetSetctions`
anchors every section below the second last one, itself included
(`reference/ingame/tooltip_item` keeps the chain `PushSection` built).

Drawing order: top-level widgets by UI layer (`SetUILayer`: background, game, normal, hud,
questdirecting, dialog, tooltip, system), then creation / `Raise` order; children above their parent; inside a
widget the drawable layers background, artwork, then the widget's own text, then overlay.
The native names table at `39ac6a80` lists the reverse order (system index 0 through
background index 7); hit testing at `392a8b65`–`392a8ba2` visits those buckets from 0 to 7,
returning the first hit. In particular, the original shared combo dropdown's HUD layer must
receive input above its normal-layer owner.

### Rendering

`render::build_quads` walks the paint order and emits textured, vertex-coloured quads in
physical pixels; consecutive quads with the same texture share one mesh. Everything is rebuilt
only when the content changed or the window is resized, so a static screen costs no CPU per
frame. Content: the first mutable access to an object in a frame keeps a copy of it, and
`Tree::settle` (once per frame, before the rebuild) compares; writing the same values again is no
change (the HUD scripts re-anchor, re-texture, re-text and re-show their unit frames and buffs
every frame). `Tree::touch` marks changes outside the objects (hover, pressed button, focus,
caret, grid rows, screen size). The layout cache follows `generation`, which every mutable
access bumps; the `OnBoundChanged` and grid passes skip frames without a net change. Text
layouts are cached by font, size, text and width (`Fonts::layout_size` measures without a copy).
Cost of the in-game HUD (release, standing at the Nuian spawn, `AA_PROFILE=1`, main thread
`update` slice of aa-client, 3 alternating runs each): 4.4-6.6 ms before this (the scripts'
every-frame rewrites rebuilt the meshes every frame), 1.6-2.2 ms after, against 0.7-1.4 ms with
`--no-hud`, so the HUD went from about 4.3 ms to about 1 ms; the aa-ui frame itself 0.6-0.8 ms,
about 0.45 ms of it the unit frame scripts' own `OnUpdate`. `AA_UI_PROFILE=1` prints
the split every 3 s. Glyphs live in one 2048x2048 atlas (which also holds the
white texel for solid quads). Textures are the pak's DDS files, decoded by Bevy (DXT1, DXT5,
A8R8G8B8 so far).

Blending happens in gamma space (`UiConfig::gamma_blending`, `CompositingSpace::Srgb` on the
UI camera) like the client's Direct3D renderer: with linear blending the semi-transparent
panels come out visibly too light.

### Text

Fonts come from the pak through `fonts/fonts.g` (a primary font plus fallbacks for Unicode
ranges, per locale), rasterised with `swash` at integer sizes with hinting; advances are whole
pixels, so measured widths (`GetTextWidth`) and drawn text agree. The font's `kern` pairs
apply as FreeType's default kerning does (scaled, times `ppem / 25` below 25 px, rounded to
whole pixels: "Vo" is 1 px tighter at 13 px). Line height is the font's ascent + descent
rounded (yd_ygo540: 13 px -> 16, 15 -> 18, 18 -> 22); the baseline is the ascent rounded up
(LibreBaskerville, the window titles' font, at 18 px: 17.46 -> 18). A text's height is
`n * lineHeight + (n - 1) * lineSpace`, and a trailing line break does not add a line, as in the
client. Inline codes: `|cAARRGGBB ... |r` colour; value codes `|,N;` (thousands separators),
`|mN;` (money with gold / silver / copper coin icons), `|bN;` (loyalty token icon), `|hN;`
(honor medal), `|lN;` (vocation badge); the icons come from `ui/common/money_window.dds`,
1 px after the value, centred on the line.
Alignment without `SetAlign`: centred for buttons, labels and `textbox` (world select's
warning line, which never sets one, is centred in `real.png`), top left for game tooltips,
left for the rest.
`GetLineCount()` is the wrapped line count of the widget's text.
Game tooltips (`gametooltip`, `lua/tooltip.rs`) hold lines rather than one text:
`AddLine(text, fontPath, fontSize, column, align, indent)` (`""` / 0: the widget's style;
`indent` 45 puts item and skill names next to their icon) returns the line's index,
`AddAnotherSideLine(index, ...)` adds a second text on its rows,
`AttachUpperSpaceLine` / `AttachLowerSpaceLine(index, px)` set the space around a line, and
`GetHeightToLastLine()` returns the height of the lines so far without and with the last
line's lower space; the tooltip scripts size their sections (title, item time, portal,
description, sell price, skill info, ...) from those differences and chain them 10 units apart.
Measured on `reference/ingame/tooltip_*`: a 13 px row is 16 high, the rows of a wrapped line
have no extra space, an empty line has no rows, and a line wraps 5 units before the right
inset (a 204 wide row of the 230 wide item tooltip stays, a 206 one wraps). The widget sizes
itself: the height is the insets plus the lines; without word wrap (`SetTooltip`'s text
tooltips) the width follows the longest row ("Bag (I)" is 58 x 36). `GetLineCount()` counts the
rows (24 for the Teleport Book, as the capture reports).
Image fonts: font names that are not files (`style:SetFont("characterInfo", 37)` for the
character window's level, `actionBar` for the menu bar badges, `combat`, `arrest`) are the
bitmap fonts of `ui/image_text.g` (`infomation` blocks: `size`, cell `width` / `height`, one
`glyph ( key K, x, y, insetLeft, insetRight)` per character in `ui/image_text.dds`); a glyph
advances `width - insetLeft - insetRight` (the level "1" is 15 px, as in
`reference/ingame/character_info`), is drawn in the texture's colours and scales with
`size / font size`.

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
visible modal window (`SetWindowModal`) blocks everything outside it. The mouse is over that
widget and its ancestors whose rectangle holds the cursor (the captures' `mouse_over`: the
action bar window with its hovered slot, `newbag`, its tab and slot window with the hovered
bag button); each gets `OnEnter` when the cursor comes onto it and `OnLeave` when it leaves it
(the action bar shows its lock and page buttons while a slot is hovered). The hovered widget
gets `OnMouseDown` / `OnMouseUp`, `OnClick(self, "LeftButton" | "RightButton")` (on the
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
thumb (`SetThumbButtonWidget`) is placed natively along the track by the value; with
`SetMinThumbLength(n)` and no `SetFixedThumb(true)` (the scripts' scroll bars) its length is
`track * page / (range + page)` (`SetPageStep`: the scroll windows pass their height), at
least `n`, the whole track when nothing scrolls (`reference/ingame/options_keybinds`: 217.2 of
412, and 437 of 437). A click on a bound `slot` uses it through the app (`AAUI.UseSlot`, below)
after the script's `OnClick`.

Dragging and the pointer's hand (`lua/drag.rs`, tests in `tests/drag.rs`). A widget that called
`RegisterForDrag("LeftButton")` is a drag source (a child of it counts: the press goes to the
source). With the left button down on it, a move of 4 UI units starts the drag instead of a click:
the source gets `OnDragStart(self, "LeftButton")`; on release the widget under the pointer, or its
nearest ancestor with a handler, gets `OnDragReceive(self, "LeftButton")` (only while the pointer
holds something), then the source `OnDragStop`; what is still held then is dropped on nothing
(`AAUI.DropOnNothing(overUi)`, `overUi` false over the game world). The click that would follow
the release is not sent. A right click while holding clears the hand. The thing in the hand is a
Lua table the app sets with `X2Cursor:SetHeld(t)` (`kind`, `icon`, `frame`, `count` are what the
toolkit draws, the rest is the app's; `GetHeld`, `GetCursorInfo`, `ClearCursor`; the window move
cursor is `SetCursorImage`, never droppable). It is drawn as a 40 px icon with its grade frame and
the count (above 1) on the pointer, over everything: separate meshes (`UiCursorBatch`, z 100), moved
every frame and rebuilt only when the held thing or the screen changes. `slot` widgets bound to game
data pick up and take drops natively through `AAUI.PickupSlot(info)` / `AAUI.DropSlot(info)` (a left
click on a bound slot while holding drops too). Rust side: `UiRuntime::holding()`,
`clear_cursor()` and `drop_on_nothing(over_ui)` (the app calls the last for a click on the world).

`X2Cursor:GetCursorInfo()` exposes native names rather than the internal `GetHeld().kind`:
`Item`, `Item_Id`, `Spell`, `Macro`, or nil. The native name table is `39ac9974`, getter
`394649d0`, with empty mode 0 handled by `39424540`. `SetHeld` accepts optional `cursorInfo`
using those exact four names; unknown values return nil. Physical bag/equipment pickups
(`3944d210` / `3946e830` -> `39429460` -> `394293b0`) choose `Item` when the definition's
max-stack-size is above 1, otherwise `Item_Id`. Setters `394244b0` / `394292f0` store the
template/grade versus complete UID respectively. The app must provide this field from its
definition: a stackable item with count 1 still uses `Item`. Legacy bag/template hand data
defaults to `Item`, equipped hand data to `Item_Id`, skills to `Spell`; action hand data uses
its item/skill/macro subtype. Visual window cursors return nil. Internal kinds, UID strings,
source slots and count stay unchanged in `GetHeld` and the app hooks.

`tests/cursor_trade.rs` runs original `trade.alb` and `logic/trade.alb` handlers without
replacing them: stackable bag drag reaches `PutupTradeItem(slot,count)` with exact held UID,
unique `Item_Id` is accepted by the original item-slot click path, and locked trades reject
drops. The original trade background `OnDragReceive` deliberately accepts only `Item`;
per-item `OnClickProc` accepts `Item` and `Item_Id`.

Native slots and widgets with only `OnDragReceive` take mouse input without an `OnClick` handler.
Disabled controls cannot start or receive a drag. If an `OnClick` handler changes the held item,
that consumes the native slot click: placing an item cannot also use it, and picking one up
cannot immediately place it again. The held icon rebuilds when the UI scale changes; new stack
count glyphs upload before the first draw.

The original title bars' `StartMoving` / `StopMovingOrSizing` now capture the window and move it
with the pointer. Its resolved size survives replacing two stretching anchors with a single
top-left anchor. Releasing the left button outside the OS window ends capture and clears its
move cursor. `tests/drag.rs` covers capture and the bag/action-slot routing; `tests/slot_state.rs`
runs the original `components/action_slot.alb` to check dark unusable skills, red/cyan learning
states, passive required points and clearing the tooltip of an emptied slot.

Checked 2026-10-17: 12 drag tests, the original-bytecode slot-state test, 6 HUD-widget tests and
3 tooltip tests passed. The existing `ingame_hud` example produced
`shots/aa-ui/drag-moved-bag.png`: the original bag moved 300 px left and 50 px up, retaining
its layout, and a held item with grade frame and count 37 followed the pointer. Compared the
bag to `reference/ingame/bag/real.png`; the harness logged 213–234 FPS during the final run.

Keyboard: one widget has the focus (`SetFocus`, or a press on an edit box; `ClearFocus`, Escape
or a press elsewhere drops it; `OnAcceptFocus` on gain and `OnReleaseFocus` on loss). Keys go to it: edit boxes insert typed
text at the caret (with `SetMaxTextLength`, `SetDigit`, `SetDigitMax`, `SetReadOnly`;
`SetPassword` shows `*`) and fire `OnTextChanged(self)`; Backspace, Delete, Left, Right, Home,
End edit and move the caret; Enter fires `OnEnterPressed` (multi-line boxes insert a line
break, `ClearTextOnEnter` empties the box), Escape `OnEscapePressed`, Tab `OnTabPressed`; every
key also fires `OnKeyDown` / `OnKeyUp(self, name)` with lower-case names (`"enter"`). Changed
`SetText` fires `OnTextChanged` synchronously; identical text is a no-op. The caret
blinks every 530 ms. `SetPrefixText` draws a prefix in `prefixStyle` before the text
(`SetPrefixInset` around it).

Edit selection uses character indices, replaces the selected range on typing/deletion, and
counts the replaced text when applying maximum length. `UseSelectAllWhenFocused(true)` is
honored before `OnAcceptFocus`, as original `InitEdit` expects. Pointer clicks place the caret;
dragging and Shift-click extend selection through inherited widget scales. Shift navigation
and Ctrl+A select, while other Ctrl-letter shortcuts do not insert their letters. Multiline
Up/Down retain a preferred horizontal column; Home/End use the wrapped visual line and
Ctrl+Home/End use the document. Soft-wrap boundary affinity preserves the correct visual line.
The same cached literal layout drives text, selection, picking, and navigation, retaining
markup characters and final empty lines. Text, backing and caret clip to the authored inset;
typing/navigation scroll the caret into view, and wheel input scrolls multiline text.
The native multiline Lua class has no `SetFocus`; pointer input and the application focus API
can focus it. Native clipboard shortcuts and IME composition are implemented below.
Native Left/Right remain per-scalar with Ctrl held: model `392291f0` uses
`3922962d` / `3922964a`, then `39224c70` applies Shift selection without a Ctrl-word branch.

Native evidence: select-all flag `39239c40` / focus path `39292230`; single/multiline rendering
`392935b0` / `39295060`; selected backing in `39220fd0` uses the game's
`Textures/Defaults/grey.dds` (loaded at `3921ddba`). `SetLimitWidth` (`39242dc0` -> `3929ee80`)
and style `SetEllipsis` (`3925b190`) both reach `39274900`, so the label method is now a real
alias. Labels narrower than the ellipsis marker no longer retain the overflowing full text.

`tests/edit_selection.rs` covers selection, Unicode replacement/limits, focus release, real
modifier edges, read-only input, scaled pointer drag, literal layout, soft-wrap navigation,
scroll clipping, and the original relationship/mail `.alb` views. `examples/native_edits.rs`
shows those original forms using `aa_dev`; `shots/aa-ui/native-edits.png` shows grey selection,
literal markup and a vertically scrolled letter. There is no dedicated social/mail real-client
capture in `reference`; panel textures and fonts were checked against the shared real HUD/bag
assets and original authored dimensions. The final capture logged 478-524 FPS, zero script errors,
and exited. The full suite passed 110 runtime tests plus both doc tests (one ignored);
the final pointer-focus regression brings runtime coverage to 111, with all nine edit tests
passing on the final implementation. No application hook is required for editing.

`AttachWidget(widget)` is a separate lifetime link: it preserves parentage and anchors.
Replacing the link hides the old shown widget and dispatches its `OnHide` before assigning
the new one; attaching the same widget again does nothing. `DetachWidget()` clears the
link without hiding it. `GetAttachedWidget()` returns the linked live widget or nil.
After the owner's `OnHide`, attachments owned by it and its ordinary descendants close
and clear, including links on hidden descendants. This runs only on hide/replacement;
metadata-only attach/detach does not dirty the visual tree or add per-frame work.

Native evidence: binding `39266b00` -> replacement `3927fd00`, getter `39266c70`
(field +0xc4), detach `39265380` -> `392774a0`, hide `3927ff20` -> recursive cleanup
`3927f9b0`. Five `tests/attachment.rs` cases pass: unchanged metadata, reentrant replacement,
ancestor/nested close, cyclic/dead links, and original `popup_menu.alb` owner-specific close
and reopen. The fixture retains `CreateEmptyWindow`'s return because the original popup
handle is a local upvalue. Generic widget removal/ClearChildren remains a separate gap.

Chat character links (`|kName,0;` friendly / `|kName,1;` hostile) now show only the name.
Cached font layouts retain a link rectangle per wrapped segment and its original name/flag.
`message:GetLinkInfoOnCursor()` returns the native fields
`{linkType='character', characterName=..., isHostile=...}` on those rectangles, and
`{linkType='none'}` elsewhere. Picking uses the visible composed message and the renderer's
inset, alignment, scale and wrapping, so scrolled/expired/cleared lines do not leave old targets.
Literal edit text does not interpret links.

Original `chat_msg_event.alb` authors the two `|k` forms; `chat/common.alb` reads these fields
for its right-click `ActivatePopupMenu` call. Native binding `3924b360` confirms character
link type 2 and those exact keys (`characterName` at payload +4, `isHostile` at +0x85).
Four `tests/chat_links.rs` cases pass for name-only metrics, colored/wrapped links, transformed
pointer bounds, stale-target cleanup, and the original CHAT_MESSAGE/right-click path with both
flags. The original formatter fixture loads the game's UI texts, since untranslated fallback
keys omit the name placeholder. The `native_edits` example also injects an original whisper
event to show the corrected friendly name.
`shots/aa-ui/native-links.png` was inspected with the original social/mail forms and the
pink whisper line: `Socialpeerone` has no literal metadata suffix. The `aa_dev` run logged
312-353 FPS, zero script errors, and exited. The final full aa-ui suite passes 120 runtime
tests and both doc tests, with one existing font metrics test ignored.

Item links use `|i<itemType>,<grade>,<encodedInstance>;`; quest links use
`|q<questType>;`. The toolkit now resolves their display names and colors once when a
message arrives, through the optional application hook:

```lua
AAUI.ChatLink = function(kind, id, grade)
  -- kind = "item" or "quest"; grade is nil for quests.
  return {name = resolvedName, color = {r, g, b, a}} -- color optional, defaults white
end
```

The callback runs outside the UI appdata borrow and may read UI state. Rendering and
picking use plain cached data and never invoke Lua. Labels display as `[name]`, with
the enclosing chat color restored after each link. Missing/invalid lookups suppress the
link, as a missing native item/quest definition does. Raw message strings stay unchanged.
The label cache is bounded at 4096 item/grade or quest keys; a changed result invalidates
only layouts containing item/quest codes. Literal `||` escapes the bar, including apparent
item, quest, character and color codes. Native link picking retains at most 150 visible
segments per composed layout; item instance payloads are limited to 382 ASCII bytes.

`GetLinkInfoOnCursor()` adds the native tables:

```lua
{linkType="item", itemType=24488, itemGrade=4, itemLinkText=encodedInstance}
{linkType="quest", questType=2531}
```

`itemLinkText` contains only the encoded instance, not the surrounding `|i...;` string.
The original `AddItemLinkHandlerToMessageWidget` passes that value to
`X2Item:InfoFromLink`, then `ShowItemLinkTooltip`; quest left-click calls the original
`ShowQuestLinkTooltip`. Character right-click continues to use its original popup payload.
Application-owned insertion, instance decoding into tooltip fields and quest data remain
game API responsibilities. Native evidence: item parser `3926f690`, quest parser
`3926fdd0`, item/quest display `39271cc0` / `39271e10`, doubled-bar dispatch `392740ab`,
and link table binding `3924b360`. Native item grade colors come from `39695c30`; quest
color depends on quest type/level versus the player, which the application lookup supplies.

Native quest formatter `3949d820` returns `|q%d;`. Quest color `39271e10` uses RGBA
`(.72,.97,.99,1)` for `quest_contexts.detail_id == 6`. Otherwise quest level minus player
level >=6 is `(.73,.22,.22,1)`, >=3 is `(.82,.55,.04,1)`, and quest level below
`player - max(5,floor((player+1)/5))` is `(.56,.56,.56,1)`; other levels are white.
The detail mapping is explicit: SQL at `39a19db0` selects `detail_id` as column4; loader
`398940ee` stores it at record+0x10, exactly the field tested by the color function.

`aa_ui::item_link` exposes transport for an exact caller-supplied native item record:
`encode_payload(&[u8;192])`, `decode_payload(&str) -> Option<[u8;192]>`, and
`format(&[u8;192]) -> String`. Native codec `394df700` / `394df7f0` uses MSB-first
six-bit groups and alphabet `0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz*+`,
without padding: 192 bytes become 256 characters. The formatter (`394e4ef0`) clears
instance ID bytes `0..8`, sets count bytes `16..20` to little-endian one, and reads the
header item type from little-endian bytes `8..12` and grade from byte `12`.
Every other supplied byte survives unchanged. This byte transport remains independent of
aa-net's Rust `Item`; the source-backed semantic packer below handles proven native fields.
The original loot debug constants contain older 235-character/176-byte payloads; the
current 192-byte decoder rejects them rather than filling unknown tail bytes.

`tests/chat_links.rs` covers colored item/quest labels, enclosing color restoration,
opaque payload preservation, unresolved definitions, payload/segment limits and literal
escapes, in addition to character links. Its original `.alb` fixture captures the loot
debug function's local event upvalue, then clicks through the original item and quest
tooltip windows. Item tooltip data in that view test comes from a known bag fixture;
the legacy encoded payload is preserved but is not represented as decoded instance data.
Three codec unit tests cover alphabet/bit ordering, complete byte round-trips, formatter
normalization and rejection of truncated/invalid payloads.

`examples/native_item_links.rs` displays the original loot-debug constants using the
database names and grade colors alongside the original quest tooltip. The inspected
`shots/aa-ui/native-item-links.png` shows purple Crushing Grip, blue Radiant Fire Lunadrop,
green Exiled Wizard's Sash, and The Prophet Terrien's title/summary. Shared HUD fonts,
chat bounds and textures were checked against `reference/ingame/bag/real.png`; there is
no item/quest chat-tooltip capture in the supplied reference set. The `aa_dev` run logged
478-518 FPS, zero script errors, and exited. The full suite passes 128 runtime tests and
both doc tests, with one existing font metrics test ignored.

Native item records can now be built/read with `item_link::{ItemRecord, ItemDetail,
EquipDetail, pack, unpack}`. All common fields are established by native packet serializer
`3961f500`, reached by the item-bearing packet vtables (for example `399b5c58` ->
`397c2070` -> `3961f500`):

| Native byte offset | Field | Native representation |
|---|---|---|
| 0 | `id` | u64 little endian |
| 8 | `item_type` (`aa_net::Item.template_id`) | u32 little endian |
| 12, 13 | `grade`, `flags` | u8 each, unchanged wire values |
| 16 | `count` | i32 little endian |
| 20 | `made_unit_id` | u32 little endian |
| 24 | detail tag + payload | 128-byte union |
| 152 | `create_time` | i64 little endian |
| 160 | `lifespan_mins` | i32 little endian |
| 168 | `world_id` | u8 |
| 176, 184 | `unsecure_time`, `unpack_time` | i64 little endian each |

`pack` preserves the supplied ID/count; `format(&pack(&record))` applies native chat
normalization afterward. The detail reader `3961edf0` copies payload bytes directly after
the tag. `ItemDetail::from_wire(kind, payload)` accepts exactly the native payload size:
tag 0 = 0 bytes, 1 = 55, 2 = 29, 3 = 6, 4 = 9, 5 = 24, 6/7 = 16, 8 = 8, 9 = 4.
Unknown tags and short/extra payloads return `None`. Unused union bytes are constructor-zeroed.
Tags 0, 1 and 3 become typed `None`, `Equip` and `Mate`; the others remain
`Opaque {kind, payload:[u8;127]}`, preserving uninterpreted native bytes.

Equipment payload offsets (relative to the full record) are: look/template 25 (u32),
durability 29 (u8), explicit reserved bytes 30..32, rune 32 (u32), charge time 36 (i64),
dye 44 (u32), seven gem IDs 48..76 (u32 each), physical/magical temper 76/78 (u16 each).
The lane server's `models/item.rs::Item::write_details` writes the reserved i16 as zero;
aa-net skips it. The native decoder preserves those two bytes if another record supplies
different values. Mate payload has experience at 25 (i32), dead flag 29 (u8), level 30 (u8).
The application can adapt aa-net `None`/`Equip`/`Mate` without guessing data. Its
`ItemDetail::Other` discarded the raw payload, so it cannot construct an opaque link from
that variant until the network layer retains the bytes. Native slave payloads are fixed at
29 bytes; a server payload with an extra four-byte repair-time expansion must not be accepted
as if it matched that native layout.

Constructor `3901cb10` zeroes the complete detail union and common time fields. Its qword
at 160 also zeroes padding 164..168. Actual alignment gaps 14..16 and 169..176 are left
uninitialized and ignored by copy constructor `3901cba0` and network serialization.
The application packer explicitly chooses zero for those gaps; it does not claim the native
constructor initialized them. `unpack` is a semantic field view, while `decode_payload`
preserves the exact 192 input bytes if unused union/padding bytes must also survive.

`tests/item_link.rs` compares supported pack/unpack fields and encoded payloads byte-for-byte
against fixtures produced by executing the actual x86 constructor, item reader and encoder
offline. The reproducible generator `tests/fixtures/item_record_native.py` requires
`pefile`/`unicorn` only when regenerating fixtures; Cargo tests need neither. It intercepts
only the serializer's primitive reads, feeds explicit test-vector wire data, asserts complete
input consumption and proves the initialized/uninitialized padding distinction. Fixtures
cover no-detail, full equipment, mate and uninterpreted tag4 records; separate tests retain
nonzero equipment reserved bytes, opaque union contents and reject wrong wire lengths.
All four native-record tests pass, in addition to the three earlier codec unit tests.

Native mail money contract (read-only audit for application integration): incoming body
serializer `398a5e60` reads the three i32 amounts in order into offsets `0x860`, `0x864`,
`0x868` (`398a5fba`, `398a5fd2`, `398a5fea`). `GetCurrentMailMoneyStr` (`3948f100`)
returns offset `0x860` through signed decimal conversion `394248b0`, without scaling.
`GetCurrentMailAAPointStr` (`3948f4b0`) returns literal `"0"` when global feature bit
`0x1000` is off; when on, it returns offset `0x868` by the same decimal conversion.
Thus aa-net's `money_amount2` is the AA-point amount, while `billing_amount` is the
second value. The gold-only UI's AA-point accessor may return zero while raw data is
retained for attachment/lifecycle checks. The corresponding native currency accessor
`3944fc30` returns `CURRENCY_GOLD` (0) with the bit off, `CURRENCY_GOLD_WITH_AA_POINT` (4)
with it on; original `mailbox/mail/common.alb` creates an AA receipt only for non-gold
bag currency. These string methods expose raw integer units, with no coin split or division.

`GetCurrentMailBody`'s helper `3948e4c0` exposes `chargeMoney` as a Lua number from the
second amount (`3948eb86` / `3948eba7`). `mail_id` is a decimal string from the header's
i64 ID (`3948e501` / `3948e553`). `extra` is a decimal string from the header's i64
`extra` at `0x2b8` (`3948ec27` / `3948ec4d`), not the body receiver. `receiver` is a
separate string from body offset `0xc`; send/receive dates are structured date tables.

The two item-count methods have different sources. `GetCurrentMailAttachedItemCount`
(`3948d550` -> `393cce70`, load `393ccedd`) returns the cached header's signed attachment
byte at +0x2a3. It does not recompute body items/money. `GetCurrentMailItemCount`
(`3948d010`) checks ten body records at +0x888, stride192, and counts only records with
nonzero ID, valid template ID and successful item-definition lookup (`3948d0bc`..`3948d0ec`,
lookup `39695dc0`). Money/charge/AA points are not item slots in that count.

Native `InfoFromLink` (`394dfae0`) decodes the inner 192-byte item record. When a wrapper
is present it skips to the second comma; outer type/grade never overwrite payload fields.
The application's strict full-markup parser can therefore accept both raw encoded payload
and structurally valid `|iTYPE,GRADE,PAYLOAD;`, trusting the decoded record for tooltip data.

Empty digit edits now follow native `GetText` (`392930c0`, vtable `399c7788` + `0x1ac`):
when `SetDigit(true)` is active, password mode is off and the buffer is empty, the getter
formats `SetInitVal` (signed i32, default zero) and writes it back into the edit buffer.
The getter raises no `OnTextChanged`; literal programmatic `SetText('')` remains empty
until that read. Non-digit/password edits keep their empty text, and nonempty digit text
keeps its spelling. The write invalidates the toolkit's text layout/rendering normally.
Native `SetInitVal` is `39292400`; the default is initialized at `3929261c`.
Original `common_view.alb` enables digit mode on all three money edits. Consequently,
the original write-mail `Clear`/`GetAttachCount`/`OnUpdate` path now displays 50c standard /
100c express for one item and no coins, then 80c / 180c when coins add an attachment.
Eleven edit tests pass, including a full original write-mail fee regression and explicit
getter side-effect/event/password/init-value checks. No application normalization is needed.

The subsequent native event audit corrects programmatic edit `SetText`: changed text raises
`OnTextChanged` synchronously, while repeated identical text is a complete no-op (including
caret and visual dirtiness). Native setter `39292f70` compares at `39292fab`, skips unchanged
content at `39292fb6`, writes at `3929309a`, then dispatches vtable `+0x1b8` through
`39292460` -> event 6 in `3927b8b0`. The event-name table `39ac6ab0` entry 6 (`39ac6ac8`)
is `OnTextChanged`. The earlier no-event assumption was incorrect. Numeric getter fallback
remains event-free because it calls the internal buffer writer directly. Reentrant handlers
can clamp through `SetText` without recursing on identical values. Tests exercise changed /
unchanged calls, synchronous reentrant clamping, clean repeated setters and original
`CreateMoneyEdit` max-value and empty-zero normalization.

Native clipboard handling supports Ctrl+C / Ctrl+Insert, Ctrl+X, and Ctrl+V / Shift+Insert.
Ctrl+Shift+Insert runs paste followed by copy, matching both native dispatch branches.
Copy uses the selection, or the complete text when selection is empty. Cut requires a
nonempty selection and removes it before attempting clipboard output. Paste replaces the
selection, normalizes CRLF to LF and applies the editor's character limit. Empty clipboard
text deletes a selection. If Unicode text is unavailable, native reuses its transfer buffer
from the previous copy/cut/successful paste; that buffer is initially empty. This source-backed
fallback is kept per UI runtime. Native conversion buffers cap
copy at 1024 UTF-16 units and paste at 4096 UTF-8 bytes then 1024 Unicode characters.
Unlike typed input, paste writes the buffer directly, so explicitly enabling clipboard in
a digit edit also permits non-digit pasted text. Password mode does not change the clipboard
flag. `UseClipboard(false)`, read-only state and a disabled widget reject clipboard shortcuts;
`SetDigit(true)` disables clipboard, while `SetDigit(false)` does not reenable it.

Windows uses the Unicode OS clipboard only after an explicit shortcut. Optional
`AAUI.GetClipboard() -> string | nil` and `AAUI.SetClipboard(text)` replace OS access for
embedded runtimes and fixtures; they run outside Ui borrows. Other platforms require those
hooks. A paste hook that changes focus/text/selection cannot insert into an outdated owner.
The app needs no hook on Windows. `UiKey::{Copy,Cut,Paste}` also supports direct harness input.

Native references: shortcut dispatch `392297c1`; copy `39227b60`; cut `39227c30`;
Unicode write `39224a60`; paste `39228270`; clipboard setter `39292ab0` (wrapper+c29,
model+a4); digit-disable path `39228c90`; password setter `39292a90`; read-only setter
`39292840` and input gate `3922a0d1`. Paste marks model dirty at `392284a9`, including
replacement by identical text. Toolkit clipboard actions do no per-frame work.
`tests/clipboard.rs` exercises Unicode selection/copy-all/cut, key edges and both Insert
shortcuts, CRLF/empty/limits, independent flags, capture/reentrant hooks and original friend /
recipient / mail-body `.alb` widgets. All automated clipboard tests and the `native_edits`
capture use private hooks and leave the OS clipboard alone.

The full suite passed 141 runtime tests plus both doc tests (one font metrics test ignored).
The native transfer-buffer follow-up also passed all 22 clipboard/edit/keyboard tests.
`shots/aa-ui/native-clipboard.png` shows the original social edit's copied `Newfriend` in
the original mail recipient, plus selected/scrolled literal mail text. Shared panel fonts,
textures and HUD art were checked against `reference/ingame/bag/real.png`; the supplied
references contain no dedicated social/mail capture. The `aa_dev` run logged 372-445 FPS,
zero script errors, and exited after its screenshot.

IME events now reach focused, visible, enabled edit widgets through `UiPlugin` and
`UiRuntime::ime_event(&bevy::window::Ime)`. The plugin reads only the primary window's
events, enables its OS context for ordinary editable text, and disables it for password,
numeric, read-only, hidden or disabled edits and generic keyboard capture. Physical
`OnKeyDown` / `OnKeyUp` names and modifier state remain separate from IME text.

Preedit replaces the previous preview at the original insertion anchor. `GetText` exposes
that visible buffer and `OnTextChanged` runs for selection removal, preedit, cancellation
and nonempty result, matching the native widget/model boundary. An empty result commits
the current visible buffer without a new text event or caret move. Single-line `IsComposingText()`
reports true during preedit and false while a result is validated; the native multiline
class has no such Lua method. Starting composition deletes the selected committed text;
cancellation restores the remaining committed base. Results replace the preview, apply
the whole-buffer character limit, and preserve original synchronous validation handlers.
An overflow result places the caret at the maximum length, including any retained tail.
IME ingress is capped at 512 UTF-16 units; supplementary characters consume two units.
The adapter retains complete Unicode scalars at the boundary.

Focus release completes the visible composition, then detaches its owner. Changing text
programmatically clears composition and restarts the OS context; an identical `SetText`
keeps it. Late results cannot overwrite a newer buffer or enter a different edit before
its own context is enabled. OS focus loss, hiding and keyboard-capture transfer complete
the old preview. Input keys cannot duplicate text, move the edit caret, or submit chat
during preedit or its result frame, even when a result handler redirects focus. Bevy's
separate IME/key queues require this one-frame edit guard. Consecutive empty-preedit and
commit messages are combined without publishing a transient canceled buffer.

The OS candidate location follows the edit caret through scrolling, inherited widget
scale, UI scale and window DPI. An absent preedit cursor hides the drawn caret. Idle
composition checks and candidate positioning reuse cached text layout and do not dirty
the UI tree. The application needs no extra hook.

Native evidence: window dispatch `3922a0c0`; selection removal `392276f0`; preedit
`39227da0`; result `392277b0`; cancel-buffer branch `39229c15`; conversion helper
`39224c10` (1024-byte UTF-16 read, `XlUtf16To8`, then `XlMultiToWide`); visible getter
`392930c0`; single-line composition query `3923a0d0` reads model+88 at widget+358.
Single/multiline changed-buffer dispatch is `39292f13` / `39294a57`; focus accept/release
`39292230` / `39292290` uses `39225360` with `ImmNotifyIME(CPS_COMPLETE)` before detach.
Candidate placement reaches `ImmSetCandidateWindow` at `3922a8d7` / `3922a952`.

Remaining IME differences are explicit: Bevy does not expose native `GCS_COMPATTR` /
`GCS_COMPCLAUSE`, so preview glyphs retain the edit's normal color without the native
per-clause decoration. Bevy also has no portable `CPS_COMPLETE` request for a preview
overflow; the visible buffer is clipped immediately and the OS composition finishes on
its result or context release. The automated fixture injects IME messages without changing
the user's input language; a real OS candidate-popup capture remains pending.

`tests/ime.rs` covers Unicode byte/scalar cursor conversion, preview/cancel/result phases,
selection, native limits, original friend-name validation, original mail multiline text,
caret visibility, focus/OS transfer, stale results, readonly/password/digit eligibility,
chat-submit suppression and reentrant result handlers. The Bevy test checks primary-window
filtering, DPI position, context restart and commit-before-Enter ordering. The unattended
`native_ime` example shows the original social/mail assets in `shots/aa-ui/native-ime.png`;
the shared HUD fonts and textures were compared with `reference/ingame/bag/real.png` (no
dedicated IME reference is supplied). It logged 268–326 FPS, zero script errors and closed.
The full suite passed 161 runtime tests and both doc tests; one font-metrics test is ignored.

The chat window's input (`chatwindow:GetChatEdit()`, `lua/chat_input.rs`) takes slash commands
natively, like the client: a command followed by a space leaves the text and sets the input's
channel (`/s`, `/local` say, `/shout` `/y` zone, `/p` `/party`, `/raid`, `/guild` `/g`, `/n`
`/nation`, `/a` `/ally`, `/trade`, `/w Name` `/whisper Name` `/t` `/tell` whisper, and the
channel numbers `/1` shout, `/2` trade, `/3` party search, `/4` party, `/5` raid, `/6` nation,
`/7` guild, `/11` trial, `/14` alliance; the locale's own `locale.chatChannel.parse_chat_*`
first); the method selector (`GetChatMethodSelector()`) shows the channel's name
(`locale.chatChannel`: "General", "Shout", "Party", ...) and gets
`OnContentUpdated(self, "chat_method_changed")` (the scripts resize it), a whisper target is
the edit's prefix. Unknown commands stay in the text. Enter, when no script set
`OnEnterPressed` on the edit, sends the line through `AAUI.ChatSend(channel, text, target)`
(below), empties the edit, drops the focus and hides the edit; a bare `/p` + Enter only
switches. The channel stays for the next line.

`X2Chat:IsActivatedChatInput()` and `AddItemLinkToActiveChatInput(fullMarkup)` /
`AddQuestLinkToActiveChatInput('|qID;')` now use the native active chat window and edit's own
shown flag, independently of keyboard focus. Creation, chat pointer presses and focus choose
the active window. Native definitions come from the existing `AAUI.ChatLink` lookup. Missing
definitions or invalid markup leave the text alone; the native return value still reports
true when an active shown input exists, false when none exists. `chat_input::install(lua)` is
public for reinstalling these toolkit methods after application fallback scripts.

Insertion appends plain `[name]` to the end, without a trailing space, then focuses the edit.
The visible label uses the edit's normal color, not the message link's grade/quest color.
Caret movement, picking, selection, copy and deletion therefore operate on the visible label
characters. A separate per-input vector retains each complete original link, including its
opaque item instance payload. Immediately before `AAUI.ChatSend`, native sequential label
matching substitutes the raw markup for intact labels in insertion order and clears the vector.
Editing a label so it no longer matches leaves ordinary literal text when sent. Serialized
insertion length must remain below 1024 bytes; native label/raw buffers hold 255/511 bytes,
and the append buffer holds 511 visible bytes. Invalid oversized names/payloads are rejected
without creating partial metadata. `AAUI.ChatSend` retains its raw text contract. The app
calls `aa_ui::lua::chat::encode_for_transport(text)` exactly once before wire serialization:
native ordered replacements double every `|`, restore `||i` / `||q` to `|i` / `|q`, then
double every `@`, then replace CRLF and remaining CR/LF with spaces. This restores the
lowercase prefixes globally, including malformed input; it is not a structural link
validator. Whisper target and channel are not encoded. Native `393c5b06`..`393c5b59` also
round-trips through UTF-16, applying global limit `39ac7ef8` and a 1024-byte UTF-8 conversion
buffer. The global limit's runtime value and conversion-overflow policy are unconfirmed;
the utility does not invent a truncation policy. The app retains protocol length validation.

Incoming packet text reaches the original `CHAT_MESSAGE` event unchanged. Message insertion
(`AddMessage` / refresh methods and `DispatchChatMessage`) decodes nonoverlapping `@@` pairs
once before storing the line: `@@@@` becomes `@@`. Bars stay doubled until glyph parsing,
so literal `|c`, `|k`, etc. remain visible text while real item/quest links retain metadata.
The input and ordinary label/edit text do not run this message decoder. No incoming client
adapter decoding is needed. `tests/chat_transport.rs` exercises the original stage's input,
`AddLinkItem` and `CHAT_MESSAGE` handler, preserving the character/item/quest click payloads
alongside literal color and character codes, repeated bars, @ pairs and Unicode.

Native transport evidence: send `395115b8`..`395115ff` uses replacement helper `39294ca0`
(nonoverlapping matching), followed by network newline normalization
`393c5be2`..`393c5c0e`. The receive path `391d8b00` -> `393c7b90` passes text to event 10
at `393c82cc`; original `chat_msg_event.alb` adds character links and prefixes. Dispatch
`394584f0` -> `395085e0` -> `39501e80` -> `39511d70` reaches message insertion `392ba860`,
which calls formatter `39435050` at `392ba8fc`. Its escaped-@ branch at
`39435156`..`39435181` replaces `@@` with `@` and returns before localization-token parsing.

Native evidence: entry points `39459960` / `394599e0`, active query `39508700`, explicit
window selector `39502020` and edit own visibility `395128e0` -> `392778a0(false)`.
Append paths `39511100` / `39511300` format `%s%s`, keep label/raw records at edit+0xe74,
check serialized byte length with `3950f940`, and focus through `392a68a0`. Raw item/quest
parsers are `39511000` / `395111f0`; send substitution is `39510e80`, and send clears the
vector at `39511547`..`39511582`. No new per-frame text processing is introduced.
`tests/chat_input_links.rs` covers native activation, end append/caret, exact opaque payload
send, broken labels, duplicate names, invalid data and byte bounds, plus original
`chat/common.alb:AddLinkItem` and `SetTitleLabelEventHandler` Shift-click from
`quest_context_notifier.alb`.

The full suite passes 145 runtime tests and both doc tests (one font test ignored).
`examples/native_item_links.rs` now opens the original chat input and inserts its original
loot-debug item link plus a quest link. The inspected `shots/aa-ui/native-chat-input-links.png`
shows `Link: [Crushing Grip][The Prophet Terrien]` in normal edit color with no encoded data
visible; the message view retains distinct grade colors. Shared HUD/font/art comparison uses
`reference/ingame/bag/real.png`, since no dedicated linked-input reference was supplied.
The `aa_dev` capture logged 377-426 FPS, zero script errors and closed itself.

The transport fixture in the same example adds an original `CHAT_MESSAGE` event containing
literal `mail@host @@ |cFFFF0000 |kFake,0; |r` next to real item/quest links. Inspected capture
`shots/aa-ui/native-chat-transport.png` keeps those literals visible and displays the linked
item in its grade color. Shared HUD fonts/assets were compared against
`reference/ingame/bag/real.png`; no dedicated real-client escaped-chat capture is available.
The capture logged 308-373 FPS, zero script errors and closed itself.

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
  | `AAUI.SlotContent(slotType, slotIdx)` | `slot:EstablishSlot` (action bars: `ISLOT_ACTION`, 1..) | nil, or `{ type = "skill"\|"item"\|..., id, icon = "ui/icon/x.dds", stack, grade, canUse, tooltip, extraInfo, cooldown = { remain = ms, total = ms } }` (`cooldown` optional: the sweep and the time left, counted down natively, no time label when `total` is 1500 ms or less: the global cooldown only dims; `refresh_slots` restarts it from `remain`, a result without it clears it) |
  | `AAUI.SkillContent(skillType)`, `AAUI.ItemContent(itemType, grade)` | `slot:EstablishSkill` / `EstablishItem` (the skill window) | the same, plus for skills `learned` (default true), `canLearn`, `reqPoints`, `skillLevel` (sent as `OnContentUpdated` states) |
  | `AAUI.SlotHotKey(slotType, slotIdx)` | `slot:GetHotKey` (keybinding labels) | `"1"`, `"SHIFT-1"`, `"MINUS"`, ... or nil |
  | `AAUI.WorldMap()` | the `worldmap` window (every frame while shown) | nil, or `{ level = 2\|3, zone = <zone_groups id>, player = { u, v, angle }, maps = { { level = 1, id = 3, u, v }, { level = 0, id = 2, u, v } } }`: the place on the zone (or city) map image and, optional, on the maps above (`world_groups` ids: 3 west, 4 east, 5 origin continent, 2 world); `u = (x - X) / W`, `v = 1 - (y - Y) / H` from the global position and the map's `zone_groups` / `world_groups` rectangle, exact against `reference/ingame/worldmap_*`; without `maps` the upper maps' place is estimated from the masks, without the hook the `RoadMap` result is used; `markers = { { level, id, u, v, kind, icon, coords, tooltip }, ... }` are the icons on the map `(level, id)` (the fraction of that map; only the shown map's are drawn, 24 px at every zoom, the tooltip is raised as `SHOW_WORLDMAP_TOOLTIP` / `HIDE_WORLDMAP_TOOLTIP` through `GetTooltipController`), `markers_version` (an integer) makes an unchanged list cost nothing |
  | `AAUI.MapClimate(zoneGroupId)` | the world map's climate display (`GetClimateInfo`) | nil, or `{ climateId, ... }` (`climates.id`: 2 temperate, 3 tropical, 4 subarctic, 5 arid) |
  | `AAUI.ConflictZones(worldGroupId)`, `AAUI.ConflictState(zoneGroupId)` | the continent map's tension circles (`GetConflictZoneListByWorldId`, `GetConflictStateByZoneId`), the zone map's state panel | `{ zoneGroupId, ... }` (west 19, 20, 22, 26, 27); `state, remainMs` with `state` `HPWS_TROUBLE_0` .. `4` (0..4), `HPWS_BATTLE` 5, `HPWS_WAR` 6, `HPWS_PEACE` 7, nil or -1 none (both optional; `fake_worldmap.lua` has the capture's states) |
  | `AAUI.UseSlot(slotType, slotIdx, button)`, `AAUI.UseSkill(skillType, button)`, `AAUI.UseItem(itemType, grade, button)` | a click on a bound, enabled `slot` (`EstablishSlot` / `EstablishSkill` / `EstablishItem`), after the script's `OnClick` | nothing; `button` is `"LeftButton"` or `"RightButton"`. Run what the slot's hotkey runs (the action bar scripts' `OnClick` only hides the tooltip; the client uses the slot natively). The app decides by `slotType` and `button`. The bag window's buttons are not slots: a right click on an item runs the scripts' `X2Bag:HandleUse(virtualSlot)`, which `fake_bag.lua` turns into `AAUI.UseSlot(ISLOT_BAG, realSlot, "RightButton")` (the real slot from `X2Bag:SlotByIdx`, so the app's own `SlotByIdx` applies); a left click (`HandleClick`, picking the item up onto the cursor in the client) is the pointer's hand: the app defines `X2Bag.HandleClick` with `X2Cursor:SetHeld` (see Input, `crates/aa-client/src/hud/drag.lua`) |
  | `AAUI.PickupSlot(info)`, `AAUI.DropSlot(info)`, `AAUI.DropOnNothing(overUi)` | a drag that starts on a bound, enabled `slot` (no `OnDragStart` of its own); something held dropped on (or a left click on) a slot bound with `EstablishSlot`; a drag ended with the thing still held | nothing. `info = { widget, kind = "slot"\|"skill"\|"item", slotType, slotIdx \| skillType \| itemType, grade, content (the hook's table), empty, enabled }`; `PickupSlot` calls `X2Cursor:SetHeld(...)`, `DropSlot` reads `X2Cursor:GetHeld()`. Without `DropOnNothing` the hand is emptied; the toolkit does not decide what a drop means |
  | slot `tooltip` (in `SlotContent` / `SkillContent` / `ItemContent`), `X2Bag:GetBagItemInfo`, `X2Unit:UnitBuff`, `X2Skill:GetSkillTooltip(skill, item, flags)` | the game tooltips (`slot:GetTooltip`, the bag buttons, the buff icons, an item's use skill) | the client's tooltip info tables, which the tooltip scripts read: items `{ name, grade = "Basic", gradeColor = "FFBA976D" (item_grades), itemGrade, category (item_categories name), item_impl ("portal", "weapon", "armor", "accessory", ... or "etc"), soul_bound = 1 ("Untradeable"), skillType (the use skill: its description is the "Use" text), description, sellable, refund, stack, isStackable, level_requirement, level_limit, lifeSpan }`; skills `{ tipType = "skill", name, ability ("general" is "Basic"), skillLevel ("Rank 1"), path (icon), mana, casting, channeling (0: "Instant"), minRange, maxRange, cooldown, description }`; buffs `{ tipType = "buff", name, category = "Buff", path, description, timeLeft, stack }`; texts may carry `|c` colours (`fake_bag.lua`, `fake_ingame.lua` have the captured ones) |
  | `AAUI.ChatSend(channel, text, target)` | Enter in the chat input when no script set `OnEnterPressed` on it | nothing; `channel` `CHAT_SAY` 0, `CHAT_ZONE` 1, `CHAT_TRADE` 2, `CHAT_FIND_PARTY` 3, `CHAT_PARTY` 4, `CHAT_RAID` 5, `CHAT_FACTION` 6, `CHAT_EXPEDITION` 7, `CHAT_TRIAL` 11, `CHAT_RACE` 14, `CHAT_WHISPER` -3; `target` the whisper name or `""`; the slash command is already stripped |
  | `AAUI.Entity(name)` | `UIParent:GetEntityByName(name)` | nil (no such entity), or `{ pos = { x, y, z }, angles = { x, y, z } }` (or `{ 1, 2, 3 }` arrays); the scripts get an object with `GetName`, `GetPos` / `GetWorldPos`, `GetAngles` / `GetWorldAngles` (read from the table at each call, so the app may update it) and `IsValid` |
  | `AAUI.PlaySound(name, duplicable)`, `AAUI.StopSound(id, mode)`, `AAUI.IsSoundPlaying(id)` | `X2Sound:PlayUISound` (a sound pack item name such as `login_stage_try_login`), `StopSound`, `IsPlaying` | the sound id (any number or string) for `PlaySound`, `true` / `false` for `IsSoundPlaying`; nothing without the hook (as before) |
  | `AAUI.PlayMusic(name)`, `AAUI.StopMusic()` | `X2Sound:PlayMusic` (a pack item such as `login_stage_music_creator`), `X2Sound:StopMusic` | nothing |
  | `AAUI.WidgetSound(set, event)` | a widget with `SetSounds(set)`: `event` is `show` / `hide` (its `Show` changed its state), `left_up` / `right_up` (a click on an enabled button; the nearest button with a set), `over` (the pointer comes onto an enabled widget), `text_added` / `text_deleted` (typing in an edit box) | nothing. The game names the sound `<set>_<event>` in the `window`, `button` and `global` packs (`bag_show`, `default_left_up`, `edit_box_text_added`); the app resolves it. `SetSounds("")` clears the set |
  | `AAUI.OptionValue(id)`, `AAUI.OptionChanged(id, value)` | the sound sliders of the Options window in the toolkit fakes (`fake_ingame.lua`: `X2Option.GetOptionItemValue` / `SetItemFloatValue`; `fake_options.lua`: `X2Sound.PlayUISound` / `StopSound` use the toolkit natives when `AAUI.PlaySound` is bound) | `OptionValue`: the 0 to 1 value of `OPTION_ITEM_MASTER_VOLUME` 35, `BGM` 36, `SFX` 37, `USE_COMBAT_SOUND` 38 (0 / 1), `CINEMA` 39, nil for the other ids; `OptionChanged(id, value)` has no result. The fakes' own values apply without the hooks |
  | `AAUI.ChatTabs(windowId)` | `chatwindow:SetChatWindowId` | `{ { id, name = "All", filters = { CMF_PARTY, ... } }, ... }` (no `filters`: every line) |
  | `AAUI.ChatColor(filter)` | `X2Chat:DispatchChatMessage` | `{ r, g, b }` or nil (a default per filter) |
  | `AAUI.RoadMap()` | the `roadmap` (every frame) | nil, or `{ level = 2, zone = 5, player = { u, v, angle }, markers = { { u, v, kind, icon, coords, tooltip }, ... } }` (level as the scripts register maps: 2 zone, 3 city; `u, v` on the map image, 0..1; `angle` the heading in radians, 0 north, clockwise, `-yaw` of the engine; `kind` a lower-case `M_*` name of `map/common.lua` such as `"quest_zone_gives"` (yellow !) or `"quest_main_gives"`, icons from `ui/map/icon/npc_icon.dds` unless `icon` / `coords` are given; `npc_*` kinds show with the map's NPC option; `markers_version`, an integer, keeps an unchanged list from being read again) |
  | `AAUI.PlaceMapPing(level, id, u, v)` / `AAUI.RemoveMapPing()` | original world map ping placement / removal | Optional: return true to send an app request and retain the last confirmed marker until echo; nil/false uses local behavior. Coordinates are normalized on the displayed map image after zoom, pan, and widget scale. Callback failure logs and consumes the request. `WorldMap` / `RoadMap` results accept `ping = false` to clear or `ping = { level, id, u, v, maps = {{level,id,u,v}, ...}, version = sequence }` to place the echo on both maps; omitted `ping` preserves local state. Optional version restarts the pulse for a repeated point. `UpdatePingInfo` applies immediately; visible map ticking also reads the echo. |

  The scripts' `UIParent:Log` / `LogAlways` (`print`) output (state names, event names) is logged at debug level
  under the target `aa_ui::lua` (`RUST_LOG=aa_ui::lua=debug` shows it); `UIParent:Warning` / `Error` stay
  warnings and errors. `X2DialogManager:RequestTaskDialog(task, info, owner)` makes the task's
  dialog (`DLG_TASK_*`, the handlers of `dialog_handler.lua`) with `info` merged before default
  decoration. Omitted `buttonType` / `isModal` retain OK / Cancel and modal defaults; explicit
  `DBT_CUSTOM` (2), `DBT_OK` (1), or `isModal = false` reach the original decorator unchanged.
  Set `wnd.OkProc` on the result for what OK does. The native battlefield invitation task
  (`3936d620`, vtable `399d0d1c`) supplies task 5, custom buttons, and nonmodal state through
  virtuals `3936ced0`, `3936cee0`, and `3936cef0`. The manager writes these fields before
  decoration (`3902d400`). Its unchanged `DialogJoinBattleFieldHandler` adds the original
  three buttons without hiding Cancel, so decorating as OK / Cancel first left a fourth
  overlapping button. `popup_lifecycle::original_task_decoration_uses_explicit_buttons_and_modal_state`
  checks the actual original invitation handler, supplied notice label/content, and omitted
  field defaults. The unique inspected `shots/aa-ui/native-task-dialog-20261018.png` shows
  Enter / Next Room / Decline with no overlapping Cancel, using the existing `ingame_hud`
  aa-dev example (`--lua` requests the explicit custom, nonmodal task); 509–552 FPS and
  automatic screenshot exit. That generic fake HUD still logs its separate pre-existing
  `ENTERED_WORLD` battlefield `maxCount` nil error; the invitation and seven popup lifecycle
  tests complete successfully.

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
| `AddNewTab(text, selectedBtn, unselectedBtn, window)` uses the script's widgets (several tabs may share a window, which each tab shows when selected and hides otherwise, in tab order: the bag's shared `tabWindow` is hidden); `RemoveAllTabs` leaves script-made widgets alive | `lua/tab.rs` |
| `gametooltip` lines, spaces and heights (`AddLine` returns the new line's 0-based index), the tooltip sizes itself to them | `lua/tooltip.rs` |
| an anchor that would make an object depend on itself is refused | `tree.rs` |
| `OnEnter` / `OnLeave` for the hovered widget's ancestors under the cursor too | `input.rs` |
| `SetStartEffect(true)` starts timed colour/alpha, scale and rotation phases; finite repeats retain their final sample, `SetRepeatCount(0)` loops, stopping hides the effect | `lua/animation.rs` |
| `SetAnimFrameInfo` animates atlas rectangles and `DAT_MOVE` / `DAT_LINEAR_ALPHA` / `DAT_LINEAR_SCALE` frames; `Animation(play, repeat)` shows, restarts or stops it | `lua/animation.rs` |
| `SetCloseOnEscape(true)` windows are what `UiRuntime::escape` hides | `content.rs` |
| `EnableScroll(true)` clips the children and the picking to the widget; `ChangeChildAnchorByScrollValue(dir, v)` offsets the anchors that target it | `tree.rs`, `widget.rs` |
| folders place their title button by `SetTitleButtonWidget` / `SetInset` (the right value moves the right edge: negative is inward; the world map's filter folders are 130 px wide) | `lua/folder.rs` |
| two anchors setting the same edge: the point first in TOPLEFT .. BOTTOMRIGHT order wins, not the later one | `tree.rs` |
| slider thumbs size to the visible part (`SetMinThumbLength`, unless `SetFixedThumb(true)`); the page step defaults to 0 | `tree.rs`, `widget.rs` |
| fonts named in `ui/image_text.g` (`characterInfo`, `actionBar`, `combat`, `arrest`) are bitmap fonts | `text.rs` |
| `SetPrefixText` / `SetPrefixInset` draw a prefix before an edit's text; the chat input handles slash commands and sends through `AAUI.ChatSend` | `lua/edit.rs`, `lua/chat_input.rs` |
| a click on a bound slot uses it (`AAUI.UseSlot` / `UseSkill` / `UseItem`); a right click on a bag item too (`X2Bag:HandleUse` in `fake_bag.lua`) | `lua/slot.rs`, `input.rs` |
| cooldowns of 1.5 s or less (the global cooldown) draw the sweep without the time label | `lua/slot.rs` |
| `UIParent:GetEntityByName(name)` asks `AAUI.Entity(name)` | `globals.rs` |
| `worldmap:SetPlayerDrawable` leaves the drawable shown (the scripts copy its visibility to the cursor effect); `GetClimateInfo` asks `AAUI.MapClimate` | `lua/worldmap.rs` |
| `grid:SetItem(widget, row, col, makeIfNotExist, value, withoutExtent)` puts (and shows) a widget in a cell, sized to it unless `withoutExtent`; rows grow with their tallest widget; `SetRowHeight(height, row)`, `GetAdjustRowHeight()` the summed heights; `RemoveAllItems` hides the widgets (the scripts pool them) | `lua/grid.rs` |

mlua's `send` feature is not needed: the runtime is a non-send resource and errors are
converted with `.map_err(|e| anyhow::anyhow!("{e}"))`.

## Known differences

* Tone: the real screenshots have a global tone curve (highlights compressed, 244 -> 219,
  200 -> 193, 72 -> 55) and are darker in the middle of the screen than our render; probably a
  post effect of the login stage's 3D scene or the gamma setting, not a UI element (not found in
  the scripts). The login background sits 1 px higher than in the capture.
* 3D is not drawn: the character model view, the login stage scene behind the UI.
* `Show(visible, ms)` alpha fades, drawable atlas/move/alpha/scale frame lists, effect
  colour/scale/rotation and circle/edge movement phases, and widget alpha/scale animations
  run over elapsed time. `Animation(true, false)` plays once; `Animation(true, true)` repeats.
  Widget `SetStartAnimation(alpha, scale)` instead enables the two independent channels.
  Native velocity/acceleration timing and priority-controlled phase completion are
  implemented, including `SetEffectScaleAxis` pivots and per-phase `SetEffectClip` bounds.
  Persistent widget `SetScale` and `ApplyUIScale` work with rendering, picking, clips and
  effective geometry. The richer `damagedisplay:SetAnimFrameInfo` combat timeline remains a gap.
* In-game HUD: what the 3D engine draws is not here (name plates over units, the world, the
  player model between the equipment slot columns). Cooldown sweeps: the direction and the
  time label format (`8`, `5m`, `2h`, `6d`) are guesses; no label on cooldowns of 1.5 s or less
  (the global cooldown, which the client only dims; the client also tells the scripts
  `global_cooldown_start` instead of `skill_cooldown_start`, here every cooldown is a skill
  cooldown, which the action slot script treats the same).
  Roadmap markers come only from the `RoadMap` hook (the natives that feed quest / NPC data,
  `AddGivenQuestInfo`, `UpdateNpcInfo`, ..., are stubs), at 24 px at every map size. Chat
  scrolls by messages, not wrapped lines (`GetMessageLines`, `SetScrollPos` are stubs). The idle
  chat window fades its tab backgrounds out and its unselected tabs to 60 % (measured by eye on
  the capture).
* In-game windows: tooltips are drawn as the scripts lay them out (item, skill, buff and text
  tooltips match their captures); the comparison tooltips beside an equipped item and the
  `|n...;` colour codes of some skill texts are not checked against a capture. The world map
  draws the icons of the `WorldMap` hook's `markers` (no ship markers of its own) and no
  zone-name tooltips (the zone capture's route is missing), and without the `WorldMap` hook's `maps` the player cursor on the
  continent and world maps is estimated from the masks; the quest list's tree lines and zebra
  rows (`SetDrawLineType`, `SetLineBackGround`) are not drawn. Pixels inside the captured
  windows (`check_ingame.sh`): mean absolute error 1.8 (bag, character info) to 4.1 of 255
  (continent map), mostly the 3D world that shows through the windows' translucent
  backgrounds in the client.
* Many natives are stubs (table below): mostly in-game widgets (model view, the chat window's
  options, map markers fed by game data) and rarely used setters.

## API coverage

Per native class: methods implemented natively (including deliberate no-ops such as sounds and
animations), methods present as logging stubs, and the client's total. Generated with
`cargo run -p aa-ui --example native_listbox_tooltip -- --coverage` (2026-10-17).

| class | native | stub | listed |
|---|---|---|---|
| `button` | 90 | 38 | 128 |
| `chatwindow` | 98 | 27 | 125 |
| `checkbutton` | 94 | 38 | 132 |
| `circlediagram` | 77 | 25 | 102 |
| `cooldownactionbutton` | 93 | 39 | 132 |
| `cooldownbutton` | 93 | 38 | 131 |
| `cooldownconstantbutton` | 93 | 40 | 133 |
| `cooldowninventorybutton` | 93 | 41 | 134 |
| `damagedisplay` | 75 | 25 | 100 |
| `drawable:color` | 15 | 9 | 24 |
| `drawable:effect` | 39 | 9 | 48 |
| `drawable:iconimage` | 20 | 9 | 29 |
| `drawable:image` | 24 | 13 | 37 |
| `drawable:ninepart` | 18 | 10 | 28 |
| `drawable:text` | 16 | 13 | 29 |
| `drawable:threepart` | 18 | 9 | 27 |
| `editbox` | 92 | 27 | 119 |
| `editboxmultiline` | 80 | 27 | 107 |
| `emptywidget` | 69 | 24 | 93 |
| `folder` | 85 | 29 | 114 |
| `gametooltip` | 81 | 26 | 107 |
| `grid` | 94 | 54 | 148 |
| `label` | 74 | 24 | 98 |
| `listbox` | 111 | 27 | 138 |
| `listctrl` | 81 | 28 | 109 |
| `listctrlitem` | 102 | 1940 | 2042 |
| `message` | 92 | 27 | 119 |
| `modelview` | 69 | 90 | 159 |
| `paintcolorpicker` | 110 | 1932 | 2042 |
| `roadmap` | 79 | 61 | 140 |
| `slider` | 83 | 26 | 109 |
| `slot` | 81 | 25 | 106 |
| `statusbar` | 76 | 26 | 102 |
| `tab` | 82 | 24 | 106 |
| `textbox` | 77 | 27 | 104 |
| `uiparent` | 21 | 14 | 35 |
| `webbrowser` | 73 | 52 | 125 |
| `window` | 77 | 38 | 115 |
| `worldmap` | 158 | 1887 | 2045 |

`paintcolorpicker`, `worldmap` and `listctrlitem` are not in the capture's class list, so they
get our natives plus a stub for every method name any class has. Toolkit stubs the login
screens still call (with the fake game API): `emptywidget:EnableScroll` / `ChangeChildAnchorByScrollValue`,
`button:IgnoreInputOnFading`, and the
`gametooltip` section natives when a tooltip shows.

## Drawable animation verification (2026-10-17)

`lua/animation.rs` keeps configuration and samples outside the anchor tree. The tick visits
only running drawable IDs, skips layout invalidation, and requests a repaint only when a
visible sample changes. Atlas frames keep their mesh between frame boundaries; hidden loops
advance time without repainting. Finite effects leave the last phase visible (the action-bar
hover background needs this), finite image timelines hide on completion, and stop/clear/
`RemoveAllDrawables` remove active work. No application hook is needed.

The original `components/action_slot.alb` now pulses autoattack backgrounds, combo borders and
toggle highlights through their configured alpha and scale phases. The original
`baselib/common_texture.alb` attention arrow moves back and forth and fades out instead of
being hidden by its `Animation(true, false)` call. Character-select arrows and timer icons
cycle their configured atlas frames. The subsequent motion milestone below adds the
autoattack arrow's circular movement and the toggle highlight's edge movement.

Checked with `cargo test -p aa-ui --tests` (78 passed, one font metrics test ignored), including
the renderer regression in `tests/animation.rs`. Those tests include the original action-slot
and attention-arrow bytecode, phase boundaries, large time steps, repeats, stop/restart,
clear/removal, no layout invalidation, hidden-loop repaint suppression, and rendered UVs,
colour, scale and rotation.

Visual captures use the existing `aa_dev` examples, which close after the capture:

* `shots/aa-ui/animation/attention.png`: original HUD with autoattack and combo effects enabled
  on slots 2 and 3 and `DrawArrowMoveAnim` on slot 1; zero Lua errors. The HUD example logged
  167–236 FPS (4.3–6.0 ms) across the captured runs before/while the effects ran.
* `shots/aa-ui/animation/character-select.png`: selected character's animated arrow, compared
  with `reference/character_select/1_character/real.png`; UI geometry remains aligned (the
  2D-only example omits the reference's character model). Zero Lua errors; 307–432 FPS
  (2.4–3.7 ms).

## Native motion and widget animation milestone

Motion and curves were traced in the read-only
`D:/aa-re/client12/x2game_unpacked.dll`, rather than inferred from parameter names:

| native VA | verified behaviour |
|---|---|
| `3928a8d0`, `3928df00`, `3928ea80` | effect displacement `v*t + a*t*t/2`; `v=delta/velocityTime`, `a=2*(delta-v*accelerationTime)/accelerationTime²`; phase finishes when its priority component reaches its final value |
| `3928a950` | top/bottom move X along `width*(edge-.5)`; right/left move Y along `height*(edge-.5)`; radius values are fractions of the drawable width/height; art rotates 0/90/180/270 degrees |
| `3928e080` | circle rotates a ray from the top onto the ellipse defined by extent-relative radii; unequal radii use the ray/ellipse intersection, not independently scaled sine/cosine |
| `39291b30`, `39291d60`, `39291e20`, `3928f460` | independent move phases, start delay, phase hold, repeat count (0 means infinite), and optional art rotation |
| `39291c90`, `39290e20`, `39290e80` | scale-axis anchor; clear stops both channels and hides the drawable, clearing only the selected colour or movement phase list |
| `392771c0`, `39277230`, `3927db80`, `3927e017` | widget alpha/scale configure their own velocities and accelerations; the start booleans independently enable them; widget integration updates velocity before position, clamps at the final value, then emits `OnAlphaAnimeEnd` / `OnScaleAnimeEnd` |

The native movement channel is independent of the colour phase: an autoattack arrow still
orbits after its constant-alpha phase finishes. Finite movement holds its final transform.
`ClearEffect` / `ClearMoveEffect` stop both channels and hide the drawable, clear their
respective phase lists, and retain the other channel's configuration. Zero-delta priority phases
finish immediately. Effects and widgets only tick their active IDs and repaint visible
changes; neither invalidates anchor layout. Parent widget scaling includes child art/text and
their input rectangles; scroll clips follow animated ancestors. Explicit `SetAlpha` resumes
direct alpha control, and normal `Show(..., ms)` fades can take over an animation.

`cargo test -p aa-ui --tests`: **83 passed, one font metrics test ignored**. Ten animation
integration tests include original `action_slot.alb`, `common_texture.alb` and
`window_view.alb`; tests verify independent motion, ellipse/edge positions, rendered rotation,
parent scaling/picking, end callbacks once, repeats, clear, and native timing coefficients.

Reviewed captures (existing `ingame_hud` + `aa_dev`, all windows closed):

* `shots/aa-ui/animation/motion-quarter.png` and `motion-half.png`: autoattack arrow rotates
  around slot 2, combo pulse on slot 3, toggle line follows slot 4's edge. Zero script errors;
  174–188 FPS with movement active, 293–340 FPS before activation.
* `shots/aa-ui/animation/window-pop.png` and `window-settled.png`: original popup helper at
  mid-fade and after completion. Compared the settled bag with `reference/ingame/bag/real.png`:
  the panel, slots, tab icons and text return to the reference layout. The missing 3D scene is
  expected in this UI-only example. Zero script errors; 427–468 FPS in the popup capture.

## Keyboard capture and rebinding handoff

The original `option/key_binding_option.alb` already supplies the editing UI, ignored-key
list, modifier composition, primary/secondary rows, conflict tooltip and reset/release buttons.
The toolkit now implements `EnableKeyboard(true)` as one visible capture owner, independent
of edit/navigation focus (native `392789e0` / `392a6960`). A false call releases only its own
capture, so an old page cannot release a newer owner's capture. `EnableFocus(true)` enables
click focus on generic widgets; capture takes priority over that focus. The original page uses
`frame:RequestInput(button)`, enables keyboard on the page, and forwards real `OnKeyUp` to
`button:OnInput`. Wheel input goes to capture even outside its rectangle.

Physical OS events produce `OnKeyDown` on presses/repeats and `OnKeyUp` on release. Edit text
still uses the OS text, while binding names use the physical key table traced at native
`39435c10` through `39436d90`. These include `number0`..`number9`, `number+`, `number-`,
`apostrophe`, `backslash`, `minus`, `equals`, `{`, `}`, and `f1`..`f15`; Shift/CapsLock do not
rename physical letter/digit keys. `UiRuntime::key` remains a programmatic press+release.

Integration contracts:

* Run gameplay keyboard systems after the public `aa_ui::UiInput` Update system set. Read
  `UiRuntime::keyboard_captured()` or `UiStats::keyboard_focus`; both include capture/edit
  focus, a consumed edge for the rest of its frame, and UI-owned held keys until release.
  Releasing capture from its own `OnKeyUp` therefore cannot leak that release to gameplay.
  `UiRuntime::focus()` includes ordinary navigation focus, which need not block gameplay.
* `lua::keyboard::install_input(&rt.lua)` installs/restores `X2Input:IsShiftKeyDown`,
  `IsControlKeyDown`, and `IsAltKeyDown` after application fallback scripts. These read
  physical OS messages, independently of any `ButtonInput` clearing done by the app. Both
  left/right modifiers are tracked; OS focus loss clears held state without inventing a
  binding release. `UiRuntime::modifiers()` exposes the same state to Rust.
* Headless callers forwarding physical events use `key_event(code, pressed, text)` and call
  `lua::keyboard::begin_frame` once per simulated input frame. Bevy does this automatically.

Keyboard tests exercise capture precedence over an edit box, actual release timing and
repeats, wheel capture, transfer between owners, hidden captures, held-key/release suppression,
modifier reset/restoration, and the Bevy `UiInput` handoff. A test executes the original
`key_binding_option.alb` handlers: mouse click selects the primary row; F4 down does not bind;
release binds `SHIFT-CTRL-ALT-F4`, returns hotkeys to gameplay, and captures a modified wheel
binding on the secondary row. Its original `OnHide` ends capture. These are input changes;
the motion milestone above contains the associated UI rendering verification.
Final regression check: `cargo test -p aa-ui --tests` — **88 passed, one font metrics test
ignored**.

## Native hotkey application contract

The app owns hotkey definitions, draft/live state, persistence and gameplay dispatch. The
toolkit already provides the original binding page's keyboard/wheel capture, modifier queries
and release suppression described above. Install app `X2Hotkey` methods after both example
fallback scripts: `fake_options.lua` and `fake_ingame.lua` replace binding getters.

| Method | Native contract |
|---|---|
| `GetBinding(action, manager)` | Live lookup; manager 1/2 is primary/secondary. Returns a key string or `""`, never nil for an unbound entry. |
| `GetOptionBinding(action, manager, option, index)` | Boolean `option` selects draft when true, live when false; it is not a modifier-format flag. Indexed builtin actions use their original 1-based index; ordinary actions use 0. |
| `GetBindingButton(name, manager)` / `GetOptionBindingButton(name, manager)` | Named widget/button lookup in live/draft state respectively, returning `""` when unbound. |
| `SetOptionBindingWithIndex(action, key, manager, index)` | Draft only. Manager must be positive and is converted to manager-1 internally. Index > 0 stores builtin_multi kind 5 and preserves the index; otherwise builtin kind 0. |
| `SetOptionBindingButtonWithIndex(name, key, manager)` | Draft only, widget/button kind 2, manager-1. |
| `RemoveOptionBinding(action, manager, index)` | Remove the matching draft action/manager/index. Preserve an explicit deleted-default marker when saving so default merging cannot resurrect it. |
| `BindingToOption()` | Copy live to draft, emit `UPDATE_OPTION_BINDINGS` without arguments. |
| `InitOptionHotKey()` | Reload draft defaults; live bindings stay unchanged until save. |
| `SaveHotKey()` | In native world stage 6: copy draft to live, emit `UPDATE_BINDINGS`, then persist; other stages return without applying it. |
| `EnableHotkey(on)` | Register/unregister the application hotkey listener independently of widget capture. |

Mutators return no Lua value. Getters format modifier prefixes in `CTRL-ALT-SHIFT-` order,
then the native lowercase physical name. Original `MakeBindingKey` composes
`SHIFT-CTRL-ALT-`; native parsing finds each modifier substring independently and uses the
name after the final dash. Normalize both forms to the same physical binding.

The conflict identity is (mode, physical key, modifiers), shared by primary and secondary
managers. Rebinding first removes the previous key for the same action/manager/index, then
displaces a conflicting record in that mode. A successful changed option assignment emits
`UPDATE_OPTION_BINDINGS(overridden, displacedAction)`: exactly two arguments, and the displaced
name is the base action without its builtin_multi index. The original callback has three
formals, but its third is nil. Removing a binding emits the same event without arguments.
Assigning an unchanged binding returns early without an event. Other modes can retain the
same physical combination.

Gameplay matching checks active modes in priority order, then normal mode, using exact
modifiers. Native `393fc270` retries only after stripping Alt bits 0x44, preserving any
Ctrl/Shift bits: Alt+W may fall back to W; Ctrl+W and Shift+W do not. Ctrl+Alt+W may fall
back to Ctrl+W, not bare W. Press dispatcher `393ff880` latches record+0x108; release
`393ff9a0` visits all five modes for that physical key and releases latched records without
matching current Ctrl/Shift again. Modifier changes alone do not re-evaluate a held movement
binding inside this manager. Explicit input resets/disable and any OS replay are separate
boundaries; the audit does not infer their behavior.

`IsValidActionName` matches the exact 104 native spellings at array `39ac8e68` through
`393f8d20`; its import is `XlCompareString`, distinct from `XlCompareStringI`.
`IsOverridableAction` (`393f8db0`) finds the action in an 18-entry (enum, mode) table
`39ac9060` and checks mode > 0. Five names/order are directly readable at `39ac9020`:
normal 0, mode_action 1, builder_rotate 2, screen_shot 3, quest_directing 4. The 18-entry
table's raw PE values require runtime decoding, so its exact membership was not recovered.
The pak's `game/default_binding.g` has 18 distinct non-normal actions; equating those with
that native table remains an inference. Load actual defaults from this pak file, not the
example fake key maps.

Native evidence: getters `39476a30` / `39476b10` / `39476ce0` / `39476dc0` call `393fcfb0`,
whose argument +0x14 selects live (+0x28) or draft (+0x88). Setters `39476460` / `39476370`
call `393ffc20`; conflict metadata is assembled at `394000e8`..`394001ee` and the two-argument
event at `39400394`. Removal `39476250` -> `393fdf60` -> `393fcd90` marks deleted defaults at
`393fcf47`. Copy directions are `393ff140` / `393ff190`; save is `393ff1f0`, draft reset
`39400e50`, key parsing `393f81a0`, formatting `393fc400`.

Read-only target-menu integration audit: `X2:GetTargetFactionRelationship` (`39442960`)
returns hostile 1, neutral 2, friendly 3, invalid faction 0, or 2 when no target exists.
It reads target faction +0x3ae4 via `390b39c0`, then `392f8210` -> `398a4130`. Native string
conversion `394bb977`..`394bb99b` confirms the enum labels. Original popup bytecode excludes
result 1 before checking character/player and not-self; it does not require friendly 3.

## Native persistent scale and effect clipping

`SetEffectClip(phase, enabled)` now clips to the drawable's original screen bounds before
effect scale, rotation and movement. It is a phase property (native setter `39292070`, renderer
`3928f877` / `3928fab9`); ancestor scroll clips are independent. Rotated quads and gradients
are clipped as triangles with interpolated UVs and linear vertex colours, so rotating art
also stays inside scroll windows. Normal rectangles keep the existing fast clipping path;
no extra draw call or material is needed.

Native `SetScale` (`39265bd0` / `3927d120`) sets a persistent local scale about the centre and
ignores calls while a scale animation is running. Parent scale multiplies child scale
(`3927b2f0`). Top-level UI windows use the global UI scale as their default, while explicit
`SetScale` values are absolute; setting a tooltip to `UIParent:GetUIScale()` therefore avoids
double scaling. `ApplyUIScale(false)` (`39265b90`) freezes its current scale across later
global UI-scale changes. `GetEffectiveExtent` / `GetEffectiveOffset` report the rendered
geometry, including ancestor animations/scales. Logical anchor geometry stays unchanged.
Accepted `SetScale` calls and scale animation changes emit the native `OnScale` callback.

Tests cover persistent parent/child scale, frozen and resumed global scale, manual-versus-
animated scale ownership, original `tooltip_view.alb:SetInfo`, rotated effect clipping with
affine UVs, and rotated child art clipped after parent scaling. The full regression suite
passed 91 tests (one font metrics test ignored); the added scroll-clipping regression also
passes, bringing the total to 92. Captures use the existing
`ingame_hud` / `aa_dev` example; all windows closed and zero script errors:

* `shots/aa-ui/animation/effect-clip.png`: original combo border at 2x scale and 45 degrees;
  slot 3 clips to its base bounds while slot 5 shows the uncut diamond. 299 FPS active,
  387–534 FPS before activation.
* `shots/aa-ui/animation/bag-scale.png`: original bag at persistent 70% scale; panel art,
  icons, text and controls scale together about its centre. Compared with
  `reference/ingame/bag/real.png`. 393–473 FPS.
* `shots/aa-ui/animation/tooltip-native-scale.png`: original skill tooltip at default scale,
  compared with `reference/ingame/tooltip_skill/real.png`; panel, icon and text geometry
  remain aligned. 442–521 FPS.

`aa-client` needs to own these contracts (no client changes in this milestone):

* Active and draft bindings keyed by action/button name, primary/secondary manager (1/2),
  and indexed-action argument (0 for ordinary actions). `BindingToOption()` copies active to
  draft on open; `InitOptionHotKey()` resets the draft; `SaveHotKey()` commits and persists.
* `SetOptionBindingWithIndex(action,key,manager,arg)`,
  `SetOptionBindingButtonWithIndex(button,key,manager)` and
  `RemoveOptionBinding(action,manager,arg)` modify the draft and resolve key conflicts.
  Queue `UPDATE_OPTION_BINDINGS(overridden,oldAction,newAction)` for the original editor;
  queue `UPDATE_BINDINGS` after commit for HUD labels.
* `GetOptionBinding(action,manager,true,arg)` / `GetOptionBindingButton(button,manager)` read
  the draft; `GetBinding(action,manager)` / `GetBindingButton(button,manager)` read active.
  `X2Hotkey:EnableHotkey(false)` suspends gameplay binding execution while capturing input.
* Reinstall toolkit modifier queries after fallback scripts. The Lua editor composes
  `SHIFT-CTRL-ALT-KEY` in that order.
* Gameplay/window actions currently inspect fixed Bevy keys in `hud/input.rs` and the movement/
  combat systems; they need one active-binding resolver. Also remove the fixed quest-dialog
  `GetOptionBinding` wrapper in `hud/quest.lua` when bindings become authoritative.

The captured contract is in `docs/ui-api.md` under X2Hotkey; original defaults are in the pak's
`default_binding.g`. No application integration hook is needed for the animation milestone.

## Authoritative party map pings

`PlaceMapPing` and `RemoveMapPing` let the app send party requests while the original marker
retains its last confirmed point. Both maps consume the same optional `ping` echo; the world
map also accepts up to 256 exact named map projections in `ping.maps`, with the existing mask fallback
when absent. The original `SET_PING_MODE` and `SET_EFFECT_ICON_VISIBLE` handlers stop the
placement mode and animate the ping art. Unchanged echoes preserve the mesh content version.
Local examples keep placement/removal when callbacks are absent or decline the request.

`worldmap` tests cover waiting for echo/removal, callback failure, local fallback, normalized
coordinates after zoom/pan/widget scale, explicit continent projections, and a city projection
after eight other named views.
`map_ping_original` runs original `ping.alb`/`ping_view.alb` and verifies both icons, pulse
restart, unchanged echo, and removal. The full runtime suite passes 96 tests (one ignored).
`shots/aa-ui/animation/map-ping.png` shows both original blue ping icons and pulse art;
compared with `reference/ingame/worldmap_zone/real.png` for the map frame and art. The existing
`ingame_hud` / `aa_dev` harness logged 221 FPS with the map open, 413 FPS before opening,
zero script errors, and closed itself after capture.

The named-projection follow-up passes all 11 targeted ping/worldmap tests, including a ninth
city projection. Native placement (`392d79c0` -> `392ac150` -> `392ac030`) samples two engine
surface heights and sends their maximum plus 2; the two interfaces appear to be terrain/water
but have not been named conclusively. It uses the player's instance only when the displayed
map kind/id equals the player's current map kind/id; otherwise instance 0. Native removal
(`392d49c0`) sends false, a zero vector, and instance 0. The app owns surface sampling and
instance selection; the toolkit callback intentionally exposes map level/id and normalized XY.

## Native combat text display

`damagedisplay:SetAnimFrameInfo` now reads the original combat frame grammar; it is separate
from drawable atlas animation. Frames support hold/transition timing, alpha/scale interpolation,
accumulated movement, counting numbers, timed letter reveal (spaces are free), deterministic
shake, and authored afterimages. Native scale is clamped to at least 0.1. Exactly one phase
advances per render tick, with overshoot discarded. `Animation(true)` does not restart an active
display; stopping, hiding, or completing clears the active state and releases the original pool
slot. Empty frame tables remain shown without drawing, matching native behavior.

Optional `AAUI.CombatTextPosition(sourceId, targetId)` returns `{x, y, alpha = 1}`: physical
screen pixels at the projected target's authored height, with optional distance fade. A nil
result suppresses the display and attached art while its authored finite timeline continues,
releasing the pool slot normally. Without a hook the examples use screen center. `SetInitPos(x,y)` truncates to native
integers and offsets that position. The app supplies projection/height/distance; the toolkit
keeps the animation and text effects. Movement/shake affect text; attached critical atlas art
follows the projected widget independently. All samples live outside the layout tree, and
continuous scale transforms reuse rasterized glyphs instead of filling the font atlas with
intermediate sizes. Whole-text primary/extra font selection uses real glyph support.

Native references in the unpacked 1.2 client: frame parser `39288530`, animation start
`39286710`, rendering/timeline `39287500`, shake `39286860`, and afterimages `39286920`.
`tests/damage.rs` covers native timing, no layout/content churn during holds, projection,
count/reveal/afterimages, culling, independent art, active restart/stop, shake, and original
`combat_text.alb` events/pool reuse. The full runtime suite passed 101 tests plus both doc
tests (one font test ignored); the added shake and original combat-state regression also pass,
bringing runtime coverage to 102 tests.

The projection follow-up fixes offscreen pool exhaustion: native `39287500` proceeds from
projection (`392876e4`) into phase timing (`3928786f`); only its explicit distance cutoff
(`39287665` -> hide) exits before timing. A missing application projection therefore suppresses
painting without pausing finite phases. The original-pool regression fills all ordinary slots
under nil projection, verifies normal completion and no further hook calls, then reuses a slot
with fresh source/target IDs and makes its position available on the same frame.

`examples/combat_text.rs` uses `aa_dev` and the original stage scripts with fixture projections.
`shots/aa-ui/animation/combat-text.png` shows white melee numbers, the critical blood atlas,
and the red "Battle Start!" text. Compared with `reference/ingame/combat_melee/real_06.png`
for font shape, size, color and alignment; fixture positions replace unit projection.
Captures logged 435-515 FPS during repeated bursts, zero script errors, and exited.

## Native 3D overhead marker audit

This is a read-only audit for application integration; `aa-ui` does not render world markers.
`game/ui/hud/overhead_mark.dds` is a 256x256 atlas. Native `3940b340` builds 12 row-major
64x75 cells: `[64*(i%4),75*(i/4),64,75]`, zero-based i=0..11. The order is
1,2,3,4 / 5,6,7,8 / 9,heart,star,X. The tiny glyph row below y=225 is unused by this renderer.

`3940bcf0` rejects indices above 11 and calls camera-facing world billboard `3940b7c0` with
anchor 7 (bottom center). Runtime float cvars at globals `3a5a0128` and `3a5a012c` supply
world width/height, multiplied by the caller's scale; these are not 64x75 screen-pixel quads.
Global `3a5a0124` supplies a scaled offset along the camera direction. Caller `39411fed`
passes name-tag field +0x9c as the zero-based atlas index and uses its world anchor +0x28.
There is no marker-specific animation time in this draw path: it remains until cleared,
subject to the normal name-tag visibility path. Names/defaults of the three hashed cvars
remain unconfirmed; world sizes should not be guessed from the atlas dimensions.

Targeted read-only extracts: `data/extract/aa-ui/overhead-native.txt`,
`overhead-marker-native.txt`; decoded asset preview `data/extract/aa-ui/markers/overhead_mark.png`.

## Native flat listbox core

`lua/listbox.rs` implements the flat native row path used by original Options and shared
combo dropdowns. `AppendItem(text,value)` and its explicit RGBA overload store rows;
`ClearItem`, `ItemCount`, `GetOpenedItemCount`, selection getters, `Select`,
`InitializeSelect`, `ClearAllSelected`, row colors, text limits and top-row scrolling are
native operations. Row data lives on the widget, participates in ordinary teardown, and
repaints only when the resulting row/hover/style state changes. There is no listbox tick.

Indices are zero-based with initial selection -1. The original `baselib/list.alb` wrapper
adds one for its combo API. Programmatic `Select` notifies `OnSelChanged` only when the stored
index changes, which prevents the original slider-list handler's same-index `Select` from
recursing. A valid mouse click first runs authored `OnClick`, then updates selection and
always calls `OnSelChanged`, including a click on the selected row. Callbacks may clear the
row storage. `InitializeSelect` always notifies for a valid row and does not clear prior
flags. Native `ClearAllSelected` clears flags but preserves the selected index in normal
mode; consequently `GetSelectedValue` scans selected flags while `GetSelectedText` uses the
stored index. No selected value/text returns zero Lua values; value -1 is also the native
no-value sentinel. A high invalid `Select` clears flags while preserving the stored index;
negative indices are accepted without selecting a row.

Listbox `SetHeight` overrides the widget method: it sets nonnegative row padding, leaving
widget extent unchanged. `GetHeight` returns font line height plus padding. View count rounds
the inset-adjusted viewport height to the nearest row, `GetMaxTop` clamps the row count minus
view count to zero, `SetTop` clamps to that maximum, and `ScrollUp`/`ScrollDown` move one row.
Scrolling changes hover and painting without emitting a selection event. Row painting uses
authored insets plus the native extra 5-unit text inset, clips to the row viewport, and
supports default/hover/selected backgrounds and text colors. `UseConsistentTextColor(false)`
uses each row's stored append-time color. `SetTextLimit(n)` affects subsequent appends:
overlong text becomes its first n-1 scalars followed by literal `...`, within the native
511-scalar storage limit.

Source evidence in the unpacked 1.2 image: registration `39245ba0`, append overloads
`39243ec0` and row storage `392a3420`, selection `392a10a0`, mouse dispatch `392a22a0`,
selected-value scan `392a0020`, clear `392a2ae0`, row height `392a00c0` / `3929f5d0`, rounded
view count `392a0210`, clamped top `392a1d30`, and row renderer `392a0d78`. Native defaults are
white text; transparent gray normal backing and gray hover/selected backing at alpha 0.5.

The flat slice leaves icon append overloads, state textures/borders and row tooltips as
explicit gaps; the subsequent tree milestone below adds recursive rows. Flat
`GetOpenedItemCount` equals row count. The native selected-text getter also converts through
a 512-byte UTF-8 output buffer; its overflow policy is not reproduced by the current getter.
The selected-text output-buffer policy remains pending for both flat and tree records.

`tests/listbox.rs` covers native indexing and return counts, event order/reentrancy, repeated
selection without repaint, row metrics, scroll limits, clipping and colors. Original World
stage bytecode tests exercise shared dropdown open/select/reopen, repeated selected-row
clicks, cleanup and wheel scrolling, plus the actual Options resolution control's mouse
selection and original `Save` method.

`examples/native_listbox.rs` uses `aa_dev` and the original World/Options bytecode, opening
the resolution list through pointer input. `shots/aa-ui/native-listbox.png` was inspected
against `reference/ingame/options_keybinds/real.png` for the shared Options art, placement,
menu and type scale; there is no dedicated resolution-dropdown reference. The ten-row
viewport with four choices follows the original option combo's fixed `limitItemCount=10`.
The repeat capture logged 425–438 FPS and zero script errors and closed itself. Full
`cargo test -p aa-ui` passed 167 runtime tests and 2 doctests (1 existing ignored font-metrics
test); the final exact-i32 and unchanged-hover changes passed all 6 listbox tests again.

## Options font face audit

The supplied `fonts/fonts.g` maps `en_us` logical `ui/font/yd_ygo540.ttf` to physical
`ui/font/flareserif_821_roman.ttf`, whose font metadata identifies Flareserif 821 Regular
(`Flareserif821BT-Roman`). Its Korean/symbol ranges use YGO 540. Both heading aliases map to
Libre Baskerville Bold. Original `layout_variable.alb` and label/button helpers use those
logical paths. The real-asset unit test
`original_en_us_font_aliases_select_real_pak_faces_and_ranged_fallbacks` verifies the actual
face selected for the Options ASCII labels, Korean, authored symbols and heading text.
No alias/fallback defect was found in that path, and no family substitution was made.

Native startup `392ece20` reads `game/fonts/fonts.g`; `392ece92`–`392ecea9` selects the
section using imported `XlGetLocale` → `XlGetLocaleString` → `Data::ChildWithValue`.
Logical-font construction `392eca90` looks up the parsed alias table. Native
`SetFontSize` (`39275030`) preserves the logical path and font kind before calling the same
font setter (`39274f70`). This rules out a separate size-only raw-file fallback in that path.

The remaining visual difference is unresolved. The reference widget JSON does not contain
resolved font-face or engine-locale metadata. A concrete renderer boundary remains: the
native glyph path `392ef130`–`392ef137` calls the static FreeType loader with flags `0x8004`
(`FT_LOAD_RENDER | FT_LOAD_NO_AUTOHINT`), while the toolkit uses Swash hinted outline
rasterization and rounded scaled font advances. Those flag meanings follow the
[FreeType glyph-load definitions](https://freetype.org/freetype2/docs/reference/ft2-glyph_retrieval.html#ft_load_xxx).
Whether rasterization/metrics or the reference process's effective font configuration causes
the observed body/menu shape difference still needs a direct glyph comparison; the audit
does not establish pixel parity. Targeted font extracts are under
`data/extract/aa-ui/font-audit/`.

## Native listbox tree hierarchy

`lua/listbox/tree.rs` adds the recursive path used by original crafting categories.
`SetItemTrees` accepts a 1-based array of nodes with required string `text`, numeric `value`
(default 0), numeric `colorR`/`colorG`/`colorB`/`colorA` (each default 1), boolean `opened`
(default false), and recursive `child` arrays. An invalid node stops its sibling array after
the valid prefix, matching the native reader. A cyclic table or more than 64 nested arrays
is rejected by a toolkit guard; authored trees are acyclic. `iconPath` remains an unsupported
variant at this checkpoint; its bounded storage path is added below. `SetItemTrees` replaces storage and resets selection, hover and top, without a
selection event. Appending any valid tree node enables tree mode, even a leaf-only tree;
clearing storage returns to flat mode.

`ItemCount` counts roots. `GetOpenedItemCount` counts visible preorder rows, descending only
into opened nodes. `GetViewItemsInfo` returns only the current rounded viewport starting at
`GetTop`, as a fresh 1-based array of `{value=..., opened=...}` records. It does not return
the whole hierarchy or text fields. Top and maximum top are measured in visible rows.
The renderer now also uses the native rounded viewport row count, rather than painting an
extra partial row for every fractional viewport.

Tree `Select` is a native no-op. A pointer click records the visible preorder index; a parent
toggles across the whole row and emits `OnListboxToggled` without arguments before selection
notification. With `EnableSelectParent(false)`, the toggle handler sees index -1, then the
prior index is restored, selected flags remain intact and no `OnSelChanged` is emitted.
Consequently a hidden selected leaf can retain its value and old index after collapse.
With parent selection enabled, closing an open parent clears selection; opening it selects
the parent. Leaf clicks notify selection, including repeated clicks. Storage replacement from
a toggle callback is checked before continuing, so old row paths cannot select replacement
records. `ClearAllSelected` in tree mode clears flags, closes every branch and resets index -1.
`InitializeSelect` still addresses a root, sets its selected flag, toggles its branch, and
emits selection notification without a toggle event. The native text getter's root-index
lookup remains distinct from the recursive selected-value lookup; it is not reinterpreted as
a visible-row text getter.

`UseChildStyle(true)` copies the current item style, then original `childStyle` setters can
change descendant font/size/color; row height continues to use the parent item font plus
padding. `SetTreeTypeIndent` controls the extra text inset (row height by default, or the
authored amount). Native text starts at left inset + 5 + 10 per depth + that inset, with the
extra inset also subtracted at the right. Opened/closed image templates use authored texture
coordinates, extents and color. Their centers are left inset + 10 + 15 per depth, and top
inset + (viewport row + 0.5) times row height. They are painted for visible parents without
creating per-row widget objects or changing anchors. Independent template animation/visibility,
separator placement, icon-path rows, state textures/borders and tooltips remain pending.
There is no listbox tick; unchanged hover does not mutate widget state.

Native evidence: recursive node reader `392454c0`, tree append/flag `392a35e0`, opened count
`3929fad0`, visible viewport `392a1fb0`, view-info result `392a26e0`, tree pointer branch
`392a240c`–`392a269b`, toggle `392a0490`, recursive close/clear `3929fbd0`, root initialize
`392a1260`, child style `392a0370`, indent `3929f7b0`, and row/icon geometry
`392a07a0`–`392a0de8`.

`tests/listbox_tree.rs` covers recursive viewport/scroll contracts, event ordering, hidden
selection, callback replacement, child styles and authored image geometry. Its original
crafting fixture instantiates `CreateCraftingWindowRenewal` from World-stage bytecode, clicks
the original Alchemy group, expands its categories, selects recipes, scrolls and collapses.
The original `OnListboxToggled` / slider callbacks keep the separate count list aligned;
the original leaf-selection handler requests recipe 10101 and then 10104 after scrolling.
`examples/fixtures/crafting_tree.lua` supplies only small game-data responses; widget handlers
and row-table generation remain original bytecode.

`examples/native_crafting_tree.rs` uses `aa_dev` and the same original handlers. The inspected
capture `shots/aa-ui/native-crafting-tree.png` shows two opened categories, one closed category,
the authored arrow images, child indentation, selection and matching recipe counts. It logged
439–450 FPS, zero script errors and closed itself. Product/material details are deliberately
empty fixture data; this validates the tree and original selection requests, not crafting
transactions. There is no supplied Folio reference capture, so shared window art and scale
were checked against the existing Options reference without claiming pixel parity. Font
rendering is unchanged. Full `cargo test -p aa-ui` passed, including all 10 flat/tree listbox
tests and both doctests; the existing font-metrics test remains ignored.

## Native listbox state art, separators and bounded icons

`SetListItemStateTexture`, its four insets and `SetDefaultItemCoord` /
`SetOveredItemCoord` / `SetSelectedItemCoord` now render authored state images. Coordinate
setters act only when a texture is loaded and store normalized UVs, matching `3929f820`
and `392861d0`; replacing the texture preserves those UVs. A zero coordinate width or height
extends to that texture edge. The row starts at left inset + 5 + 10 per depth and ends at
right inset - 5; state insets then add left/top and subtract right/bottom. Selection takes
precedence over enabled hover, with the existing parent-selection gate. The whole widget
rectangle clips row artwork, preserving the expanded selection images authored with negative
insets by original `components/scrolllistbox.alb`. There is no per-frame listbox tick.

`CreateSeparatorImageDrawable` is now a list-owned template, painted for root-depth rows
whose viewport index is greater than zero. Native `392a0e20`–`392a0e9f` anchors its BOTTOMLEFT
at left inset + 10, top inset + viewport index times row height + 2. The anchor enum table
`39ac6a5c` establishes BOTTOMLEFT = 2. Separators use authored coordinates, extent, color and
alpha without generating per-row objects. `IsItemOpened(index)` is the native zero-based
**root** query (`392a0440`), returning false for invalid indices; original Achievement count
generation now uses it directly.

The native `AppendItem` dispatcher (`39243ec0`–`392440cb`) also accepts
`(text, value, iconPath)` and `(text, value, r, g, b, a, iconPath)`. Both overloads retain normal
text/value/selection behavior. `SetItemTrees` reads the same optional `iconPath` field into
each record. The native renderer (`392a0b42`–`392a0de8`) suppresses record icons in tree mode
and instead paints branch templates. Flat icons use row-height square extents and TOPLEFT
at left inset + 10, top inset + (viewport row + 0.5) times row height. The native icon-seen
flag becomes persistent after a visible icon is painted, including across `ClearItem`; later
rows and paints apply the extra row-height text inset (or authored tree indent).

Icon support is deliberately bounded to direct **64×64 DDS sources**. Native cache creation
`3921e161`–`3921e179` / `39223820` establishes 64×64 cells, and `3922439e` validates the
renderer-returned image dimensions before copying pixels to the cache. At this checkpoint,
ordinary 48×48 item DDS still needed a renderer audit; the following section resolves their
native rejection. The fixture's real 64×64 control
sheets (`ui/button/screenshot_mode.dds`, `ui/ranking/period.dds`) exercise the overloads and
full-image UVs; they do not establish ordinary item-icon parity. Missing or unsupported icon
sources leave the row text usable. Independent template animation/visibility, state-image
fallback texture shape, line borders and listbox tooltips remain pending.

`tests/listbox_art.rs` checks icon argument validation and placement, tree suppression,
persistent indentation, normalized coordinate lifecycle, expanded-state clipping and
selection/hover texture changes. `examples/fixtures/listbox_art.lua` constructs the original
Achievement window and original `CreateSliderList` icon-forwarding wrapper. No widget handler
is replaced. Its regression opens/collapses a real category, checks authored separators and
count alignment, and exercises viewport scrolling.

All 13 targeted tests passed with `cargo test -p aa-ui --test listbox --test listbox_tree`
and `cargo test -p aa-ui --test listbox_art`. The original-window test advances its authored
opening animation before comparing physical quads. `examples/native_listbox_art.rs` uses
`aa_dev`; inspected `shots/aa-ui/native-listbox-art.png` shows the original Achievement
summary, expanded General category, aligned counts/separators, textured selection and the
two bounded control-sheet icons. It logged 390–414 FPS, zero script errors and closed itself.
There is no supplied Achievement reference capture; shared window art/scale were compared
with the existing Options reference, without claiming pixel parity. Font rendering is
unchanged, and the fake Achievement data does not exercise server completion requests.

## Listbox 48-pixel DDS cache audit

The native listbox cache rejects ordinary **48×48 DDS icons**; no resize is missing from
this toolkit path. The renderer audit used the existing CryRenderD3D10 binary extract
(`data/extract/aa-cry/ida/render/cryrenderd3d10.dll`, SHA-256
`EBF051BBA5D349A6B3BF6D4EB676908115B73BB22D35D5A0CC917C325426589D`). Its renderer constructor
`38118689` installs vtable `382f2d88`, whose image-loader slot `+44c` is `381778d0` and
image-info slot `+440` is `38176150`, matching the UI cache calls.

The loader calls `381d52c0` → `381d5140` → `381d4ed0` → DDS constructor `381d6740`.
The constructor copies header width/height directly to image offsets `+4`/`+8` and keeps
them. Image-info `38176150` returns those fields unchanged. Flag `0x2000` selects the image
data allocator (`381d4ce0` / DDS allocation branch); it does not resize. Path normalizer
`38187610` changes `.tif` / `.srf` suffixes to `.dds` and leaves an existing DDS path intact.
The shared UI cache has one initialization call (`3921e161`–`3921e179` → `39223820`), with
64×64 cells in a 1024×1024 atlas. Cache `3922439e`–`392243b0` compares both decoded dimensions
for equality and follows its release/failure path for a 48-pixel source.

Consequently the existing 64-pixel source guard is preserved. This conclusion applies to
the shared icon cache used by these native listbox overloads; it is not a generic drawable
restriction. The original `CreateSliderList` regression appends the real 48×48 potion DDS
through its unmodified icon-forwarding Lua method, then selects Healing potion by mouse and
checks its text/value while confirming no rejected icon quad appears. Valid 64-pixel rows
continue to render. All three `listbox_art` tests passed.

`native_listbox_art --item-sources-48` produces the unique inspected capture
`shots/aa-ui/native-listbox-item-sources-48.png`: both potion labels remain usable, Healing
potion is selected and its rejected icon stays blank. It logged 411–456 FPS, zero script
errors and closed itself. The two valid control-sheet icons remain visible. No resize,
application hook, font substitution or broader image-loader change was introduced.

## Native listbox row tooltips

`ShowTooltip(bool)` enables the native `OnTooltip(text, x, y)` notification. It starts
disabled. Native `392a19c0` checks the absolute visible-preorder row against the last
notified row; it reports stored row text regardless of visual ellipsis and without a delay.
The callback's physical-screen coordinates are the row's right and bottom edges: widget
insets, the native 5-unit horizontal text margin, row height and viewport index, minus state
right/bottom insets, all scaled by the UI scale. Tree depth and icon indentation do not
change that corner. Native `392a1c90` refreshes this path when scrolling.

The toolkit emits a new notification on a changed valid row and refreshes it when the top
changes under the pointer. Unchanged hover returns before layout/row traversal and does not
mutate tree state. Replacing storage resets notification state; copied text is released from
the UI borrow before Lua runs, so callbacks can clear/repopulate rows safely. Disabling
tooltips only changes the gate, matching native `39243b80`. Native `3929f730` resets row
tracking and emits **zero-argument** `OnTooltip()` before base `OnLeave`, even when the gate
is disabled; the input handoff now preserves that order. There is no listbox timer or tick.

The notification buffer matches the source's 512-byte conversion buffer: up to 511 bytes
plus NUL. `xlcommon.dll!XlWideToMulti` (`3301a1d0`–`3301a287`) checks capacity after each
UTF-8 byte, so a boundary can retain a partial final sequence. The callback preserves those
raw Lua-string bytes rather than silently changing the native payload. A regression checks
this with repeated non-ASCII text.

The unmodified original `CreateSliderList` handler checks `IsMouseOver`, calls
`SetTooltipOnPos` for a row, and calls `HideTooltip` on leave. Original `SetTooltipOnPos`
converts the physical callback coordinates with `CalcDontApplyUIScale` and anchors the
tooltip's BOTTOMLEFT corner there. `tests/listbox_tooltip.rs` covers enable/disable,
physical coordinates at 1.25 UI scale, unchanged-hover cost, scrolling, recursive rows,
callback replacement, native leave ordering and the actual original Lua tooltip window.
All 16 targeted flat/tree/art/tooltip tests passed; the final UTF-8 boundary change passed
the three tooltip tests again.

`examples/native_listbox_tooltip.rs` uses `aa_dev` and the original list/tooltip scripts.
The inspected `shots/aa-ui/native-listbox-tooltip.png` shows a visually shortened Health row
and its complete text in the original dark tooltip skin. Tooltip art was compared with
`reference/ingame/tooltip_hud/real.png`; there is no dedicated supplied listbox-tooltip
reference. The capture logged 180–237 FPS, zero script errors and closed itself. Template
visibility/animation and line-border variants remain separate pending work.

## Independent listbox image templates and fallback states

Opened/closed branch images and separators now retain their real `SetVisible` state. They
start visible like ordinary image drawables; they are excluded only from the owner's
standalone drawable pass. Native listbox row rendering (`392a0cc3`, `392a0d67`, `392a0e90`)
calls the same image virtual renderer as ordinary drawables. `Image::Render` (`3928b420`)
checks its own shown flag at `+98` before rendering, and samples frame animation at `+11d`.
The toolkit therefore uses the existing image pipeline at each authored row rectangle,
including frame coordinates, movement, alpha, scale, rotation and clipping. It does not
rewrite the template's anchors or allocate objects per row. Existing active-only animation
timing, finite completion hiding and cleanup apply to these images as well.
Per-row rectangles retain their authored fractional coordinates; the normal standalone
drawable pass keeps its existing pixel snapping. Prior branch position/width assertions
are unchanged; their icon count now filters the authored grid sheet, since selected state
fallbacks are also image quads.

Native state fallback uses `Textures/Defaults/white.dds` for selected/default rows
(`392a0a2d`, `392a0aec`) and `Textures/Defaults/grey.dds` for hovered rows (`392a0a96`).
The original pak assets are a 4-by-4 opaque white image and a 1-by-1 opaque RGB 128 grey
image respectively. The toolkit now resolves those actual assets with full-image UVs and
the authored state color. An explicit state texture continues to replace both fallbacks.
This fixes the previous all-white hover fallback without changing fonts or row geometry.

Line-border methods remain pending. The native Lua wrapper exposes `SetBorder` and
`SetLineColor`, but a search of the complete extracted original `.alb` tree finds these names
only in that forwarding wrapper. Native `3929f540` writes line height at `+320` (initially zero), while
`3929f560` sets RGBA at `+324`. The constructor does not initialize that color. No guessed
default color or negative-height normalization has been added.

`tests/listbox_templates.rs` checks independent branch/separator visibility, authored
placement without anchor writes, shared UV/scale/alpha frame sampling, finite completion,
loop restart and removal cleanup. Its original Achievement fixture observes image handles
returned by the original constructor, then exercises ordinary image APIs and real mouse
category expansion. The original slider-list fixture checks selected white / hovered grey
fallbacks and explicit-texture replacement. All 19 listbox tests and 10 animation tests
passed, including existing dropdown/crafting/art regressions and the 511-byte tooltip limit.

`examples/native_listbox_templates.rs` uses `aa_dev`. The inspected final capture
`shots/aa-ui/native-listbox-templates.png` shows General expanded through the original mouse
handler, animated opened art, independently hidden closed art, authored separators and the
two fallback state tones in the Alchemy list. The fixture applies frame animation explicitly
to exercise the normal image API; the original Achievement Lua does not request that loop.
Shared window art/scale were compared with `reference/ingame/options_keybinds/real.png`;
there is no supplied Achievement reference. The final capture logged 342–357 FPS, zero script
errors and process exit 0 after closing itself. Reproduce with
`native_listbox_templates --lua 'closed:SetVisible(false)' 1 --hover 230 418 1 --shot shots/aa-ui/native-listbox-templates.png --shot-after 4.4`.
No application integration hook is required.

## Native equipment repair contracts

This read-only audit uses `D:/aa-re/client12/x2game_unpacked.dll` and original `.alb` scripts.
The application owns the repair bridge; the independent OS cursor hook below is toolkit-owned.

`X2Item:EnterRepairMode()` (`3947f1a0`) sets shared item mode 2 through `394df2c0`;
`LeaveRepairMode()` (`3947f1c0`) sets normal mode 1. `IsInRepairMode()` (`39480290`) returns
the boolean comparison with 2. Repeated mode assignments do nothing. Switching 1/2 emits no
dedicated repair Lua event. The native bag-click interceptor (`391f2a70`) calls single repair
with the physical packed location and shared auto-AA flag, then consumes the click even if
repair preflight rejects it. This happens before ordinary item handling. Original equipped
item Lua explicitly checks repair mode before either mouse-button action and calls
`X2Equipment:ItemRepair(slot)`.
The bag boundary is narrower: original `PreClickSlot` executes first. Only the subsequent
`X2Bag:HandleClick` wrapper (`3944c760`, view virtual `+50`, vtable `399baa30`) enters
`391ff2f0` and the repair interceptor at `391ff37f`. It does not preempt the Lua link/auction
pre-handler. `HandleUse` (`3944c7f0`, virtual `+5c` -> `391ff3e0`) follows its normal use path
through `391f48d0` / `394eb270`; it has no repair interception. The two other native
`391f2a70` callers (`39200585`, `39200853`) are distinct bag/bank-context paths, not HandleUse.

`X2Bag:ItemRepair` (`3944ccc0`, also `39452390`) converts its **physical one-based** Lua slot
to an unsigned byte minus one, with native container 2. `X2Equipment:ItemRepair`
(`3946ea70`) uses container 1 and the same conversion; Lua slot zero selects the native
last-location sentinel. Both use `GetAutoUseAAPoint()`, not an extra Lua argument, and return
no Lua values. Single repair (`394f14a0`) requires mode 2, a valid repair NPC interaction
predicate (`394ed3d0`), an existing item, repairable maximum greater than current durability,
and payment preflight. No independent distance constant or direct NPC blacksmith flag test
appears in this wrapper; those checks must not be inferred from it.

`RepairAllCost()` (`39481c70` → `394e82d0` → `394e7e80`) totals only the 28 **equipped**
records, not the bag. It includes existing repairable items whose maximum exceeds current
durability. `RepairAll()` (`3947f170` → `394f13e0` → `394e9a40`) gathers the same equipped
set and uses the shared auto-AA flag. It does not require mode 2. Sender `394f10e0` returns
without payment preflight or a packet for zero candidates; one candidate still uses the
repair-all packet because this call forces the all path. Single/all packets remain the
source-proved `0x03c` three bytes / `0x03d` one boolean documented by aa-net.

`X2Bag:ItemDurability(physicalSlot, checkRepairable)` (`3944e2f0`) and
`X2Equipment:ItemDurability(slot, checkRepairable)` (`3946f9b0`) return either nil or a table
with numeric `current` and `max`. Helper `3941a040` obtains the unsigned-byte maximum through
`397cf500` and the current unsigned byte from item `+1d`. Missing/unknown/non-durable items
and zero maximum return nil. The optional second argument is a strict boolean, default true;
false bypasses the repairable flags in the body/armor/mate definitions (`+28`/`+1c`/`+30`).
It does not bypass zero maximum. Bag slots are physical one-based, and equipment slot zero
uses the native last-location sentinel, as with repair itself.

The original misspelled `X2Equipment:DameagedItems(maxCount, checkRepairable)` (`39470750`)
requires its first numeric argument; signature `n` / wrapper `394709b0` returns nil when it
is absent or cannot be read as a number. The maximum count is truncated to signed integer.
With valid arguments it creates and returns three parallel one-based tables, even when empty:
item template IDs, literal raw item-grade bytes, and physical equipment slots plus one.
There is no grade-order conversion. `394e9ad0` scans the 28 equipment slots, retains nonzero
maximums with `f32(current)/f32(max) <= 0.25`, and applies the default-true repairability test.
`394efc90` / `394e9bc0` stable-sort by ascending float durability ratio before taking maxCount;
equal ratios preserve ascending physical slot order. Zero/negative counts return empty tables.

`UPDATE_DURABILITY_STATUS(added, removed)` is native event `0x1ac` (registration `39215c94`).
Its UID tracker (`3a5ba0cc`, vtable `399e6340`) initializes from equipped and occupied bag items
in `394f61f0`, using `397cf500(item, false)`: unrepairable durable items participate. It records
unsigned-byte current/maximum by UID, independently of the equipment-only notifier list.
Predicate `394df540` is nonzero maximum and float ratio <= 0.25. A successful insert
(`394f6190`) writes both values and sets added when the new item is low, including duplicate-UID
low inserts. Update (`394f5e30`) writes only current and uses the stored maximum: high-to-low
sets added, low-to-high sets removed, staying on the same side does neither. Remove
(`394f5ed0`) sets removed only for an existing low record. Missing-UID update/remove do nothing.
Flags accumulate with OR, so both may be true in one batch.

The tracker handles literal successful item action IDs 5/6 (insert), 7 (remove), and 9 (update)
at notification `0x1c` in `394f5310`; it ignores 8/12/13/14/15 and other IDs. It does not filter
for bag/equipment containers: successful bank-3 insertion snapshots can enter its map too,
though initial seeding covers only equipment and bag. Native `397cda20` skips containers 4/5
before copying an insertion snapshot. Batch-start notification `0x19` clears both flags;
batch-end `0x1a` emits the Lua event only if either flag is set (`394f5392`). Initial seeding
clears flags before listener registration. No separate threshold event for a trailing force-remove
list was established by this bounded trace; do not infer one from final inventory differences.

Native price function `397cea70` has this exact operation order; every assignment below is
rounded to `f32`, with current/maximum unsigned bytes and price/refund signed integers:

```text
factor_scaled = factor * 0.01f32
ratio = 1.0f32 - f32(current) / f32(maximum)
lost_price = ratio * f32(price)
base = factor_scaled * lost_price
grade_scaled = f32(refund_multiplier) * 0.01f32
raw_cost = base * grade_scaled
cost_float = f32(ceil(f64(raw_cost)))
```

The result is checked against zero and the float-converted integer limits, then converted
with x86 `CVTTSS2SI`. The exact upper-bound corner is unusual: price `i32::MAX`, refund 100,
factor 100, current 0 / maximum 1 produces `i32::MIN`, because `i32::MAX` rounds to
2147483648 as a float and that value passes the float upper-bound comparison. Normal
database prices do not establish that corner as useful gameplay behavior. The total uses a
checked signed-32-bit accumulator (`394e5fd0`); `RepairAllCost` then converts that total to a
float ScriptAnyValue/Lua number, not a decimal string. Its displayed value may lose precision
above 2^24 even though each underlying price was integral.

The ignored `data/extract/aa-ui/repair_cost_oracle.py` executes the original x86 function in
Unicorn, stubbing only database lookups and CRT `ceil`/integer-limit imports. Ten executions
match the native expression, including these differences from the lane's arithmetic order:

| Price | Refund | Factor | Current / max | Native copper | Lane copper |
|---:|---:|---:|---:|---:|---:|
| 1137792 | 150 | 75 | 97 / 151 | 457755 | 457754 |
| 1132972 | 120 | 75 | 4 / 105 | 980830 | 980831 |
| 1778503 | 300 | 33 | 125 / 184 | 564578 | 564579 |

The native UI quote and authoritative server money can therefore differ by one copper.
The CSV preserves all ten oracle inputs/results; no server charge was changed or synthesized.

Payment helper `394f0eb0` receives amount, currency selector, auto-AA, deferred task and error
code. The repair selector defaults to money (0); only player flag `0x1000` permits lookup of
currency-config kind 11 (`39744420`). Selector 0 checks money and emits error `0x1a` when
insufficient. Selector 3 checks AA funds and offers dialog 39 when insufficient. Selector 4
uses money directly when sufficient; otherwise auto-AA permits money plus AA if enough,
offers dialog 39 if still short, or opens confirmation dialog 38 when auto-AA is disabled.
Original handlers identify 38 as `DLG_TASK_ASK_USE_AA_POINT` (with auto-AA checkbox), and 39
as `DLG_TASK_ASK_CHARGE_AA_POINT`. The helper returns 0 to proceed, 1 to reject/offer charge,
or 2 for deferred confirmation. This does not prove that the current lane enables selectors
3/4 or entitlement flag `0x1000`.

The selector lookup is a static game-definition table, not a direct SCInitialConfig getter.
Loader `39750960` uses `SELECT kind_id, currency_id FROM currency_configs` (SQL at `39a0cdbc`),
stores the key/value arrays at game DB `+f7f4`/`+f7f8`, then builds their range map with
`3974e380`. Getter `39744420` searches that map at `+f798` by **kind_id**, not the table's
primary `id`. Repair requests kind 11 (`394f1127`) and reads its `currency_id`; a missing
record yields gold selector 0. The read-only lane `compact.sqlite3` currently has zero rows
in `currency_configs(id, kind_id, currency_id)`. No link from raw SCInitialConfig
`secondPriceType` (packet `+2bc`) to this record or the enabling player flag was established;
its runtime handler's protected body remains the source boundary. Raw server bytes must
not be substituted for this selector.

Item tooltip generation (`394237b0` / `394e4b60`) supplies numeric `repairCost` only for a
damaged repairable item, using that same native price function. Original tooltip Lua displays
the repair-cost money line while repair mode is active; it displays the cannot-repair line
for an explicitly unrepairable item with maximum durability.

The auto-AA getter (`397ccd50`) compares raw inventory field `+c340` to 1. The setter
(`3944c8a0` → `394e98d0`) requests packet `0x133` after same-value/pending checks and does not
optimistically write the field. Received action 18 (`397d1030` / `397cd940`) first records
packed key `0x0000f000` in a local sink, then writes the raw field. The sink vtable `399e6310`
calls `394e8600`, which only inserts a map key. After the action returns, `394f007f` calls
`394e7140`; action-18 branch `394e758c` reads the new getter and emits native event `0x24a`,
`CHANGED_AUTO_USE_AAPOINT(new_bool)`. `39424240` → `3920fcf0` dispatches listeners inline,
using BeginCall/argument push/EndCall before processing the next action. Coalesced actions
therefore expose their own new getter values during callbacks. Only after the full action
batch does `394f0225` → `397d22b0` clear pending keys, including `0xf000`, with
`397d2200(key, false)`. Sink insertion never sets an inventory request lock: unsolicited
action 18 does not acquire a pending lock before its callback. A separate unlock must not
fabricate a changed-value callback.

Repair mode selects native cursor enum 7 (`39434bb4` / `39434cca`). `3921e2bd`–`3921e2d2`
loads normal group-cursor resource 141 from `archeage.exe`; the large theme loads 142 at
`3921e3c5`–`3921e3e0`. Their original images are 32×32 and 64×64, both hotspot (0,0), with
RT_CURSOR entries 19/20. Targeted ignored extracts preserve those bytes and show the authored
hammer cursor. This is an OS cursor, independent of held-item state; reusing `SetCursorImage`
would replace the held thing and does not establish native repair-cursor behavior.

The toolkit's `cursor::set_repair_mode(&Lua, bool)` now selects that OS cursor through Bevy's
`CustomCursorImage`; `cursor::set_large(&Lua, bool)` explicitly selects the separate large
resource. Normal is the default. No DPI/UI-scale inference or enlargement of the normal
image is used. `cursor::CursorSource(PathBuf)` is configurable and defaults to the read-only
RE copy `D:/aa-re/client12/bin/archeage.exe`, whose bytes match the installed original.
Only resource groups 141/142 are decoded at runtime; no artwork is committed. Normal is
BGRA32 with authored alpha, large is BGR24 with its original AND mask, both bottom-up DIBs.
The large resource's AND mask is entirely zero, so its white matte remains opaque;
no guessed white-color transparency is applied.
The prior OS cursor component is restored on leaving repair mode or removing the UI runtime;
missing/unsupported resource files preserve it. Images are cached on first use, and ordinary
frames do not rewrite cursor components. Held-item state and generation are untouched.

Native NPC cancellation (`3947da90` → `393be180`) emits `NPC_INTERACTION_END` first, then
resets mode to 1 on its resolved interaction-object cleanup path (`393be23b`). The general
cancel/escape path `3921b1d5` also resets non-normal mode. Arbitrarily hiding the repair window
is distinct: its original `OnHide` does not itself call `LeaveRepairMode`.

Natural service routing remains unresolved. Original `npc_interaction.alb` proves
`"blacksmith"` → `BLACK_SMITH` 7 → `ToggleRepairWindow`; the sole `REPAIRMAN_NPC` reference
is the `"repairman"` branch, with no definition anywhere in the extracted original scripts.
Native `SelectNpcInteraction` (`3947da30`) is a no-op. Native NPC choices are built by
`393bd290` from authored world-interaction records; `3902ea31` / `397aa970` initializes their
type-to-action map, and `397a9510` / `393b7eb0` resolves candidates before `393bd240` inserts
them. This bounded trace does not prove skill 12086 maps to the gear-repair action or expose
a blacksmith option beside a shop through the lane's precedence. No combined menu or
repairman-to-blacksmith remapping is supported by this audit. The client must keep that
natural-entry limitation separate from original-window fixtures and working repair APIs.

`examples/native_repair_cursor.rs` loads the original World stage and repair window, supplies
only fixture game responses, and clicks the original Single Repair button through real mouse
dispatch. It asserts repair mode and an empty held cursor, then displays the decoded normal
and large resources at their actual pixel dimensions. GPU screenshots omit the OS cursor,
so these two previews explicitly expose its source pixels. The inspected final capture is
`shots/aa-ui/native-repair-cursor-final.png`: 354–364 FPS, zero script errors, native exit 0
and automatic window closure. Common parchment framing was compared with
`reference/ingame/options_keybinds/real.png`; no supplied repair-window reference exists.
Existing font differences remain unchanged. All 29 prior listbox/animation regressions passed.
Three new cursor tests also passed: exact source pixels/mask and truncated-resource rejection,
mode/size/leave transitions with unchanged held state, setters while Ui appdata is borrowed,
missing-source fallback, runtime removal, caching, and preservation of an external OS cursor.

## Popup and dialog input lifecycle (2026-10-18)

Original `components/popup_menu.alb` and `components/combobox.alb` subscribe to
`MOUSE_DOWN(widgetId)` and decide themselves whether an outside press hides their popup.
The toolkit now emits that event after `OnMouseDown`, and `MOUSE_UP` after `OnMouseUp`
before click activation. This follows native `39277da0`: event index 4 at `39277e39`,
index 5 at `39277eaa`. The payload uses the same numeric identity as our `GetId()`;
`IsDescendantWidget` and other widget arguments now resolve checked numeric identities.
Both right-button edges reach the same handlers/events. `UiVirtualPointer.right_press`
and the examples' `--rclick` deliver a real press followed by a release next frame.
The existing `input::update` API remains available; `input::update_buttons` accepts all
four edges together. Empty canvas presses use the toolkit's root identity, 0.

Popup removal is now implemented as requested by the original
`popup_menu_view.alb`'s `EnableHidingIsRemove(true)`. Native setter `392657e0` stores
the flag at `+0x2b5`; hide completion checks it at `392ab33e`. Removal waits until
the widget finishes hiding and callbacks finish, then retires its descendants,
drawables, Lua object registry entries, event subscriptions and frame handlers.
This prevents an old popup's event closure from running after its original
`OnHide` has cleared the shared popup upvalue. A flagged widget remains valid
through the current click and `OnHide`. Setting the flag while already hidden
does not remove it; a later shown-to-hidden transition is required, preserving
the original pet-frame initialization order. Showing it again or disabling the
flag cancels pending retirement. Removed focus/capture is released, while keys
already owned by UI remain consumed until their physical release.

Escape dispatches `OnCloseByEsc` before hiding, matching native `392d4500`'s handler
index `0x29`. The unchanged `components/dialog_handler.alb` therefore runs its
`OnCancel`: ordinary confirmations invoke `CancelProc` and manager cleanup;
notice dialogs invoke their `OkProc`. The dialog's `OnHide` follows that callback.
No new modal keyboard-routing policy is inferred from these mouse/Escape traces.

`tests/popup_lifecycle.rs` runs unchanged original popup, combo and dialog bytecode,
with mouse edges through the real dispatcher. Six regressions cover inside/owner/
outside/empty-space presses, both mouse buttons, dropdown selection, repeated
opening and listener cleanup, confirmation/notice Escape, initially-hidden flag
ordering, fade completion, cancellation of removal, descendant focus release and
held-key consumption. All 52 tests passed across this suite and the existing
keyboard, attachment, drag, animation, edit-selection and cursor/trade suites.

Visual check used the existing `character_create` example (`aa-dev` default plugins
and `DevPlugin`), selected Battlerage and the reference name `Refone`, opened the
original `ShowCreateConfirmPopup`, pressed Escape,
asserted exactly one cancellation, and reopened it before `--shot`.
`shots/aa-ui/native-popup-lifecycle-confirm-20261018.png` was inspected against
`reference/character_create/step3_nuian_male/real_confirm_dialog.png`: the dialog's
350x165 crop has mean absolute pixel error 3.59/255, and all 52 comparable named
screen objects match geometry. The 2D example omits the reference's character and
3D scene. It logged 356-463 FPS, zero script errors, exit 0 and automatic closure.

## UIParent event callback ownership (2026-10-18)

Original `unitframe/pet.alb` and `hud/pet_action_bar/pet.alb` both assign the global
`SpawnPet` and register their distinct closures using
`UIParent:SetEventHandler("SPAWN_PET", SpawnPet)`. The global name is intentionally
reused; registration retains each closure independently. Keeping only the last
function made one spawn invoke the action-bar callback twice and omit the frame.

Native bindings `392610d3` / `3926111f` lead to registration `3925cde0` and release
`3925ce30`. Registration manager `39212370` inserts every event/script/function
triple through multimap insertion `39211810`, without replacement or deduplication.
Release `39210f50` searches the event's equal range, compares script state and
function identity, and erases only the first matching registration. Its callback
argument is required; an event name alone does not release all callbacks.
Dispatch `3920fcf0` calls ordinary widget listeners first, then iterates the root
callback range in insertion order, passing only event arguments. The iterator is
live: additions can run in the current event and released future callbacks are
absent. The toolkit uses ordered Lua slots with released references replaced by
`false`, preserving iteration positions during callbacks and reentrant dispatch.
Widget subscriptions remain separate. Callback errors still allow later listeners.

`tests/event_handlers.rs` exercises distinct and duplicate functions, exact release,
widget-before-root order, unchanged event payloads, live addition/release, and error
isolation. Its original-bytecode regression creates both actual pet widgets from one
spawn, dismisses them, completes retirement, and creates both again from one spawn.

The same regression exposed a second constructor failure: original pet-bar
`UpdateBindingText` passes `slot:GetHotKey()` directly to
`ConvertAbbreviatedBindingText` / `string.upper`, so nil aborts construction.
Native `GetHotKey` (`39256500`) returns the slot string at `+0x340`; slot constructor
`392c7950` initializes it to empty string `39ac3044` and clears it with the empty
literal `3999c708`. Read-only loaded-client checks confirmed the shared string is
empty and vtable `399c97c8 + 0xa4` resolves to slot identity method `392c7aa0`.
The toolkit now returns an empty string for unbound/unestablished slots and absent
or nil `AAUI.SlotHotKey` responses. Bound key strings remain unchanged. The original
pet regression also checks the completed 250x105 bar, avoiding a partial-constructor
false positive where a visible but unfinished 0x0 widget exists.
All five event-handler/slot regressions and six popup-lifecycle regressions passed
after these changes (`cargo test -p aa-ui --test event_handlers --test popup_lifecycle`).
Integration visual check used the client's `owned_pet_hud` example with `aa-dev`
default plugins, `DevPlugin` and `--shot`:
`shots/hud/sol61-owned-pet/native-frame-and-bar-final.png` was inspected and shows
the named pet frame and full original state/command/action bar. It logged 304-399
FPS and closed automatically. The broader fixture logged one newly reached
`ENTERED_WORLD` callback error (`battlefield/entrance.alb:46`, nil `maxCount`);
the pet constructors completed. No supplied pet-specific native capture was available
for a pixel comparison; this fixture omits the 3D scene and changes no UI calibration.

## Original pet command gates: native evidence (2026-10-18)

This read-only UI audit supports client-owned API work; it does not add command
planning to the toolkit. Native `PassiveMode` (`39069ac0`) requires a current own
pet, requests mode 1 and returns true even for a repeated passive request.
`StandMode` (`39069bb0`) suppresses only an already-standing pet (`+0x34 == 0`)
whose active raw mount-skill command (`+0x58`) equals zero. `ProtectiveMode`
(`39069ca0`) and `AggressiveMode` (`39069d60`) also require attackability (`+0x24`),
and suppress the matching mode 2/3 only while that command field is zero. All four
report whether their branch requests a mode; deeper planner `3933c8f0` may still
reject it. Constructor `3933ab60` and spawn initialization `3933ac10` initialize
`+0x58` from zero global `3a16831c`; mode state is a separate field. Skill planner
`3933e630` stores the original raw mount-skill ID at `3933eb11`; its completion
paths clear the field at `3933ecf2`, `3933ed0d`, and `3933ed25`. It is not a target
object ID. Read-only process memory confirmed the relevant mode/skill sentinels
`39addc54`, `39addc5c`, and `3a16831c` are zero in the original loaded client.

`CanDismiss` (`3906a560`) additionally calls `39357830`: its result depends on the
local controller's virtual `+0x8c` result and casting/channel fields. The controller
result's semantic name remains unresolved, so the simpler existence check is not
claimed as parity. `CanMount` (`3906a440`) tries the current selected unit before
the current own pet through `3935aa10`; it includes kind/alive, unattached player,
controller/casting/channel, status, squared distance <=625 (25 units), two-second
cooldown, buff and model-seat gates. It is broader than `IsMountablePet`, and these
unresolved planner details must stay explicit when integrating native commands.

## Published radar rows in original Map widgets (2026-10-18)

`lua::radar::set_snapshot(&Lua, Snapshot)` supplies plain published data to the
native Map methods used by unchanged `world_map.alb` and `road_map.alb`. Snapshot
contains `Arc<Vec<MapBounds>>` and independent ship, transfer and fish contact
lists. Bounds are `{ level: u8, id: u32, rect: [x,y,width,height] }`, matching the
client's `GameData` rectangles; coordinates use game Y up. A contact supplies its
runtime object ID, world XY, classified `Symbol`, and optional tooltip metadata.
The toolkit has no aa-net dependency and adds no Lua row-getter API. Pending
network chunks, classification, localization and relationships remain app-owned.

Original `UpdateTelescopeInfo`, `UpdateTransferTelescopeInfo`, and
`UpdateFishSchoolInfo` copy only their own shared manager channel into each widget's
cache. `set_snapshot` advances that manager cache even before either map exists:
existing runtime IDs retain their first symbol and list position, duplicates update
the last received position, absent IDs retire, and new IDs append. The manager stores
only ID, position and symbol. Tooltip metadata reads the current published contact
separately. Injection alone does not replace displayed rows. `Remove*Info(id)` accepts
the numeric runtime identity without a lossy f32 conversion; `Clear*Info()` clears
only its channel. Drawable pools are reused. Markers reproject when map bounds,
current map or size changes. World-map and roadmap positions both anchor to the
Map widget; roadmap art cropping does not alter this native world projection.
They use `u=(x-originX)/worldWidth`, `v=1-(y-originY)/worldHeight` and original 24x24
`npc_icon.dds` cells, staying 24x24 through zoom. Native source is `392b0c90` /
`392b1190` / `392b1680` and shared creation `392afaa0`. Module-verified read-only
Map descriptor inspection confirmed all eight radar symbols enable continent,
zone and city, hide on world level, and remain enabled when the NPC checkbox is off
(descriptor visibility bytes `[0,1,1,1]`, category 3, NPC-off flag 1).

Source classification belongs in the app adapter: observer `3904bd90` explicitly
tests raw ship-row kind ==2 for ship, all other values for shipyard; result byte 1
from `39027f40` / `39838880` chooses enemy variants. This is a full polymorphic
relationship predicate, not proof that different factions always mean enemy.
Existing cached IDs update only position, retaining their first symbol; the cached
map row contains no health/name/owner strings. Transfer observer `3904c270` follows
`transfers.model_id` to `models.name`: `airship` selects airship, `carriage_main`
selects carriage, other names select landship. Missing template/model selects an
out-of-range symbol and should be omitted. Native SQL column order confirms the
model's name at `+0x1c` and transfer model ID at `+0x0c`.

Tooltips pass the original `SHOW_*MAP_TOOLTIP(rows,count)` schema and anchor its
original controller to the marker. Ship/shipyard metadata includes numeric ID,
name, enemy, owner, expedition, optional kind, buff, hp and maxHp. The unchanged Lua
builds its own title, owner/guild text, line and live health bar; its OnUpdate still
calls app-owned `X2Map:GetTelescopeUnitHealth`. Native builder `392ad655` sets buff
only for raw tail ==3554, clamps the initial tooltip HP to min(hp,maxHp), and reads
live published ship metadata. Its kind conversion `39571630` maps slave_kind_id
1..9 to `big_sailing_ship`, `small_sailing_ship`, `speedboat`, `boat`, `tank`,
`machine`, `siege_weapon`, `slave_equipment`, `fishboat`. Kind and buff may be
omitted while unresolved. Transfer tooltip `392ad940` uses the received name;
fish tooltip `392ad810` resolves the received template through DoodadAlmighty and
uses its name. Normal text tooltips use the same original handler.

Range circles remain undrawn: their separate native area methods need the exact
static `war_circle.dds` and pulsing `territory_circle.dds` styles and visibility
rules. Native explored-map and excluded-instance masks also remain outside this
first renderer milestone. The app must retain any unresolved enemy relation as a
documented limitation rather than claiming native relationship parity.

Validation: `cargo check -p aa-ui --lib` and all 13 tests across `map_radar`,
`map_markers`, `map_ping_original` and `event_handlers` passed. The new tests use
unchanged World-stage map event handlers, verify both maps' geometry, channel
independence, publication boundaries, large object IDs, world-level hiding,
continent projection, zoom and the actual original ship tooltip/removal lifecycle.
`examples/native_radar.rs` uses `aa-dev` default plugins and `DevPlugin`; its unique
`--shot` capture `shots/aa-ui/native-radar-published-20261018.png` was inspected.
It shows the eight original symbols and original Windfinder health tooltip,
logged 190 FPS during startup and 491-544 FPS thereafter, zero script errors and
exit 0 with automatic closure. Common framing was compared with
`reference/ingame/worldmap_zone/real.png`; the fixture deliberately uses full-map
zoom 1, while that reference uses zoom 3 and contains no radar rows. It omits the
3D scene, and no font/layout calibration was changed.

### Shared radar cache and native enemy relation follow-up

The cache distinction is backed by `3904bd90`: the Map manager's ship list at
`+0x4e0` contains no tooltip strings. The existing-ID branch `3904bed1`–`3904beed`
overwrites only position, while `3904bef7`–`3904bf59` resolves relation/kind only for
a new ID. Transfer/fish observers use the same identity pattern. The shared aa-ui
cache now follows this distinction rather than freezing tooltip metadata or
reclassifying an existing icon. A tooltip's current kind/enemy can consequently
differ from the retained icon when the same received ID changes classification.
Clearing or retiring the ID allows its next appearance to use a new symbol.

Radar's native enemy test is **relation byte 0 ==1**, not the attackability bit.
The source `39027f40` -> `39838880` accepts two polymorphic facades. All combat
permission branches after `398389d6` modify byte 1/byte 2 only and therefore cannot
change the radar icon's enemy classification. For a valid player/ship pair the
relevant precedence is:

1. Either facade's virtual `+0x30` forces relation 3. ShipRow always returns false;
   the player's `39356c90` tests raw kind 0 and byte `unit+0x3afe`. That byte's
   gameplay name remains unresolved.
2. Try owner-character-ID versus other effective-faction override (`39838740`),
   then the reverse direction, then sorted effective-faction pair override
   (`398387e0`). Relation 0 means continue. The two override maps are manager
   `+4` and `+0x14`; packet-facing setters `39027e30` / `39027e70` remain separate
   network integration work.
3. Try regional callback `390280a0`, then base faction callback `392f81f0`.

Normal effective-faction policy `398379b0` returns nonzero expedition ID unless
virtual `+0x44` is true, otherwise raw faction ID. ShipRow's flag is false; the
player flag is byte `unit+0x3bec`, whose gameplay name remains unresolved. Policy
registration `390281c0` instead chooses faction-only `39837750` when global mode
`3a16efd8` is 4..8. ShipRow validity `39196920` requires nonzero faction or
expedition; its direct character ID is zero, its owner-character ID is received
summoner ID `+8`, faction is `+0x30`, expedition `+0x4b8`, raw kind `+0x0c` and
packed position `+0x18`. Module-verified read-only runtime inspection confirmed
all corresponding invalid-ID sentinels are zero. The later forced-hostility
helper `39358ab0` cannot apply to ShipRow: its virtual `+0x2c` is always zero,
but equality with that value is reachable only after rejecting zero on the player.

The regional callback resolves both positions to zones and requires the same
`zones.group_id` (`397a6c60` -> zone template `+0x10`; native SQL column order
confirms this field). It finds that group's controller through `392f3510`, requires
controller state `+0xd8 ==4`, and requires both positions inside its primary or
secondary circle (`39839b40`). `39839820` resolves a side from direct character ID,
then owner-character ID for raw kinds 2/5, then expedition ID, then faction ID.
Unresolved side returns relation 0; matching sides return 3, opposite sides 1
(`39839900`). Circle coordinates/radii and side-membership records must come from
the actual regional state; they are not derivable from a ship's faction alone.

Base faction `398a4130` returns 0 for either zero effective ID and 3 for equal IDs,
otherwise follows each faction's mother chain repeatedly (`398a37a0`). The final
root comparison `398a3660` returns 3 for equal roots, then checks either root 3
(hostile), either root 2 (neutral), either root 1 (friendly), then the stored
unordered pair or default neutral. Read-only runtime inspection confirmed these
special root constants are exactly 1/2/3. This is stronger evidence than an
ordered, single-parent server approximation, but dynamic expedition/faction maps,
character/faction override messages and regional controller inputs still belong
to the app. `Contact.symbol` already accepts its resolved result; no extra Lua or
network-facing aa-ui API is needed. Source excerpts and module verification are
in ignored `data/extract/aa-ui/native-radar-relation-runtime-20261018.txt` and
`native-radar-relation-source-20261018.txt`.

All 14 tests across `map_radar`, `map_markers`, `map_ping_original` and
`event_handlers` pass, including first classification before map construction,
shared world/road classification, duplicate ID/last position, retirement and
reappearance, and fresh tooltip metadata without an icon update. The unique
aa-dev capture `shots/aa-ui/native-radar-shared-cache-20261018.png` was inspected:
same original map/tooltip rendering, zero script errors, startup 251 FPS and
387–508 FPS afterward, automatic closure and exit 0.

## Original screen overflow recovery (2026-10-18)

`CheckOutOfScreen()` and `CorrectOffsetByScreen()` now implement the native
queries instead of returning no values. Unchanged pet-bar Lua uses them when a
bar or equipment panel changes size/scale, and original tooltip/window positioning
uses the same methods. A nil check previously bypassed recovery altogether.

Native registration `39262b02` / `39262b6b` binds `39262170` / `392622e0`.
They read effective position (`39275470`, object `+0x6c`) and extent (`39275490`,
`+0x74`), then compare against renderer pixel dimensions `39ac67fc` / `39ac6800`.
The boolean query uses strict overflow: touching an edge is inside. Correction
returns two physical pixel coordinates without moving the widget. Per axis it
first corrects a negative near edge to zero; otherwise a far-edge overflow returns
`screen-size`. An oversized positive-position widget can therefore return a
negative origin. The original Lua divides these results with `CalcDontApplyUIScale`
and applies its own anchors. No screen margin or forced global positioning was added.

Pet `CorrectWidgetScreenPos` removes anchors **before** requesting the corrected
position. Native `RemoveAllAnchors` (`392616c0` -> `39276a70` -> `39276940`)
frees the nine anchor/dependency records without resetting stored offset or extent.
The toolkit now retains the resolved logical origin and size on removal, while
freshly created unanchored objects retain their existing default placement. New
anchors replace that retained origin. Repeating a remove/re-add of identical
anchors still leaves settled rendering unchanged. The source excerpts are in
ignored `data/extract/aa-ui/native-screen-bounds-source-20261018.txt`.

Three new regressions cover edge equality, every overflow direction, non-mutating
physical returns, oversized-widget precedence, effective persistent scale,
anchorless origin/extent/dependency removal, and the unchanged pet `OnScale` and
tooltip placement helpers at UI scales 1 and 0.8. Across the library and focused
native, animation, grid, scale/clip, popup, event, world-map, radar, marker and ping
suites, 91 tests passed (one existing font-metrics test ignored). This includes
the original pet frame/bar constructors and unchanged-state repaint regression.

`examples/native_screen_bounds.rs` uses aa-dev default plugins and `DevPlugin`.
The final unique `--shot` capture
`shots/aa-ui/native-screen-bounds-final-20261018.png` was inspected: the original
Stormmane bar recovered to the bottom-right, the original brown tooltip recovered
to the top-right, zero script errors, 233–272 FPS, automatic closure and exit 0.
Shared HUD/tooltip framing was compared with `reference/ingame/tooltip_hud/real.png`;
that reference has no pet/overflow scenario, so it is not a pixel comparison of
the corrected placement. No font, asset layout, or scene calibration changed.

## Native battlefield entrance data contracts (2026-10-18)

These are source contracts for the app-owned `X2BattleField` adapter, traced from
the unchanged entrance Lua through the original native bindings. They do not add
battlefield state or database access to aa-ui.

`GetEntranceBasicInfo(npcType)` (`39456130`) requires a local player, a valid NPC
template, and its nonzero `npcs.recruiting_battle_field_id`. It returns a table
with `type=battle_fields.id`, `name=battle_fields.name`, and
`desc=battle_fields.desc`. `type` is not `field_kind_id`. Native SQL `39a14988`
loads `id,desc,field_kind_id,name,zone_key`; loader `3982d396` / `3982d3cc`
localizes the description/name through the `battle_fields` table and row ID.

Rule selection `39456970` scans that battlefield's rules in descending
`level_min` order and selects the first threshold at or below the local player's
level. Native SQL `39a16a28` explicitly orders
`game_rule_sets BY battle_field_id,level_min DESC`. The remaining getters use:

| Original method | Binding | Native result |
|---|---|---|
| `GetEntranceMaximumPlayers(npcType)` | `39456a40` | Selected rule's `corps_size`, the size of one team |
| `GetEntranceLevelSection(npcType)` | `39456af0` | Two numbers: selected minimum and next higher threshold minus one; the highest band ends at the game's level cap |
| `GetEntranceFactionInfo(npcType,corps)` | `39456e00` | Corps 0/1 resolves selected rule's `corps1_id`/`corps2_id` through the faction manager; corps 2 returns literal `ANY_CORPS`; other corps returns an empty string |
| `GetEntranceVictoryConditionInfo(npcType)` | `39456c50` | Table with `victoryScore=victory_score`, `victoryKillCount=victory_kill_count`, and `victoryTarget=victory_kill_corps1_head` |

The victory target is the first corps-head field's numeric value; the binding
does not convert it to a boolean or fall back to the second corps-head field.
Faction resolution follows `3936ce60` -> `397f21d0` -> `392f8100` -> `398a4090`,
rather than using the battlefield's display name. Missing local player, NPC or
battlefield returns **zero Lua values**. Maximum/faction/victory also return zero
values when no threshold matches. Level-section lookup itself can return
`0,levelCap` for an empty rule list and uses minimum zero below every threshold;
it does not invent a matching lowest-level rule for the other methods. The
read-only, module-verified original runtime's level-cap byte was 55.

`GetInteractedNpcType()` (`394568d0`) reads the battlefield manager returned by
`3936ce50`, specifically manager `+0x3cd8` at `3a172cb0`. This manager is static
`3a16efd8`, not the current actor or selected target. Constructor/reset
`3936d0d0` initializes it to the invalid-NPC sentinel (runtime-verified zero).
The recruiting transition `3936f780` stores a nonzero request NPC type at
`3936f845`; a request with sentinel zero preserves the existing value. The Lua
request path `39455e00` supplies the actual NPC type, while server-driven caller
`391e0812` supplies zero. The getter initially returns numeric zero even without
a player. Original entrance Lua separately prefers a nearby target NPC only when
its type differs from this stored value and its distance is below 2.5.

The adjacent `X2Unit:GetNpcTypeIndex(unit)` binding `394c0f40` is deliberately
narrower than unit display-name grouping: resolver `3945fcd0` must return actor
category 1 and the raw unit kind at `+0x410` must equal NPC kind 1. It returns
that unit's numeric template ID at `+0x41c`. Unresolved units, characters, Mates,
Slaves, housing and shipyards return zero Lua values. A Mate's NPC-backed
template does not make it eligible for this interaction getter.

Source and bounded read-only runtime excerpts are in ignored
`data/extract/aa-ui/native-battlefield-entrance-source-20261018.txt` and
`native-battlefield-entrance-runtime-20261018.txt`. This audit changes no UI
rendering; the app owns getter integration and its original-panel validation.

## Native pet information-panel stat contracts (2026-10-18)

Original `hud/pet_action_bar/pet_view.lua` requests
`X2Unit:UnitInfo("playerpet")` for displayed values and
`UnitModifierInfo("playerpet")` for text colors. They use the pet's actual actor
and equipment, not the player's attribute snapshot. `UnitInfo` binding
`394bdc10` resolves the raw unit, advances to its actor at `unit+8`, then calls
`398aa290`; its attribute component is `unit+0x1aa8`.

Let `A(i)` be that actor's final signed integer attribute from `398a0bc0`.
The presentation helper uses f32 operations in the listed order; `f(i)` below
means conversion of `A(i)` to f32 and `inv=1.0f/f(13)`.

| `UnitInfo` key | Native calculation |
|---|---|
| `melee_dps` | `f(96)*0.001f + f(33)*0.001f` (separate scaled terms) |
| `melee_critical_rate` | `f(16)*inv*100.0f` |
| `dodge_rate` | `f(178)*inv*100.0f` |
| `armor` | `f(8)` |
| `armor_percentage` | `f(8)/(f(8)+5300.0f)*100.0f` |
| `magic_resist` | `f(64)` |
| `magic_resist_percentage` | `f(64)/(f(64)+5300.0f)*100.0f` |
| `persistent_health_regen` | `f(67)*0.2f` |
| `health_regen` | `f(11)+persistent_health_regen` |
| `persistent_mana_regen` | `f(68)*0.2f` |
| `mana_regen` | `f(12)+persistent_mana_regen` |

The armor/resistance denominator is the fixed constant 5300 (`399e3e20`), not a
formula involving player or pet level. Critical/dodge percentages have no extra
presentation clamp. The unchanged Lua formats armor/resistance as
`%d (%.2f%%)`, DPS as `%.2f`, critical/dodge as `%.2f%%`, and regeneration through
the normal value-to-string path.

### Attribute bases, aggregation and equipment

Mate raw kind 5 uses generic base dispatch `398b4e80` -> `398a0d60` and
`unit_formulas` owner type 5. Relevant base routes are:

| Attribute | Base source / formula kind |
|---|---|
| 11 / 12 | Kinds 16 / 17 (health / mana regeneration) |
| 13 | Kind 20 (rating denominator) |
| 33 | Kind 21 (additional melee DPS, stored in thousandths) |
| 67 / 68 | Kinds 31 / 32 (persistent regeneration) |
| 16 | Helper `398a7220`, kind 1 and multiplier attribute 77 |
| 178 | Same helper, kind 46 and multiplier attribute 180 |
| 8 | Equipped armor sum `398b4b90`; no kind-18 formula in this Mate route |
| 64 | Truncated kind-19 formula plus equipped resistance `398b4ae0` |
| 96 | Mainhand slot 15 weapon DPS `398a7ae0`, multiplied by 1000 then truncated |

Helper `398a7220` subtracts
`trunc(f32(A(multiplier)) * (f32(A(13)) * -0.001f))` from the truncated formula
result. The multiplier constant at `39a204c8` is **negative 0.001**, so a positive
multiplier increases the rating. Attributes 54 (`melee_attack_speed_mul`) and
180 (`dodge_mul`) have zero base in this route. Normal UnitFormula evaluation
`39567110` explicitly spills/reloads its evaluated result as f32 at
`395672b3`/`395672b6`, before integer conversion `398b56d0`. Its f64 conversion
store therefore does not justify retaining an unnarrowed formula result.

For reference, compact owner-5 formulas include kind 1 `(dex*10)*100`, kind 16
`spi*0.1+7`, kind 17 `spi*0.07+7`, kind 19 `spi*2`, kind 20
`((level^1.3+level*3+17)*100)*100`, kind 21
`(((level*2)/3+19)+(level^2/15)+(str*0.2))*1000`, kinds 31/32
`(spi*0.1)*2`, and kind 46 `((dex+int)/2*8)*100`. These are database formulas,
not hardcoded replacement formulas in aa-ui.

Final `A` combines all applicable flat and additive percentage deltas into one
pair initialized to `(0,100)`: buff/provider/temp modifiers (`398a0060`), static
NPC-template/level modifiers (`3989ffb0`), and for actor kinds 0/5 all three gear
buckets from `398a0a50`. Percentages from independent sources are added as
deltas, not multiplied sequentially. Percent 100 uses integer `base+flat`;
otherwise `f32(base+flat)*f32(percent)*0.01f` is truncated toward zero. Native
`395729f0` then clamps against `unit_attribute_limits`. Primary-derived formula
inputs consequently use the pet's final attributes, including its own gear.

Armor base scans armor slots 0..14 and holdable slots 15..18. Armor implementation
2 and accessory implementation 24 use their armor/slot types to select
`wearables.armor_bp`; the latter's dedicated native lookup is `396a6d00`
(`item_accessories`, native SQL `39a01890`).
Resistance base scans eligible slots 0..14 and 27, using
`wearables.magic_resistance_bp`. Both require usable durability. Wearable
helpers `397cc6d0` / `397cc750` evaluate kind 0/1 `wearable_formulas` with
`item_level` and the matching `item_grades` coefficient, narrow to f32, apply a
positive received override as a percent, multiply by f32 basis points and
`0.0001f`, then truncate. Their compact formula is
`floor(item_level^1.1*45+100)*item_grade`. Armor on holdables uses
`holdables.formula_armor` through `397cc4c0`.

Mainhand DPS evaluates `holdables.formula_dps` using the template's signed-byte
item level and `item_grades.var_holdable_dps`. This is a separate evaluation path
(`397cc440` -> `39568560`): without a positive DPS override, `397cc45b` stores f32
with **FST**, retaining the evaluated x87 result for the caller's multiply by
1000 and truncation. A positive override instead uses the narrowed f32 result
times the received percent times `0.01f`. Do not extend UnitFormula's mandatory
final f32 spill to this no-override holdable path. Empty mainhand uses
`398a7320` with the pet's level, default holdable ID 0 and grade 0 (bounded,
module-verified read-only runtime constants). Compact holdable 0 is `fist` with
DPS/armor formulas `0`, so its weapon DPS contribution is zero.

### Modifier values and original label colors

`UnitModifierInfo` (`394bec40`) calls `394bb740` for raw modifier deltas. It does
not subtract an unmodified value from clamped final `A`. That helper first
obtains raw base `B` through `3989f860`, gathers static template pair `S`, and
stores an f32 baseline
`b=(f32(B)+f32(S.flat))*f32(S.percent)*0.01f`. A new pair `T=(0,100)` gathers
buff/provider/temp modifiers and eligible main-item, rune and gem buckets.
`skipMainGear=true` omits only the main-item bucket; runes/gems still count.
Neutral `T` returns zero; otherwise it computes
`(b+T.flat)*T.percent*0.01f-b` with the native x87 arithmetic and narrows the
returned Lua value to f32. No integer truncation or attribute-limit clamp is
applied. This separate static baseline is deliberate and differs from `A`'s
single aggregate operation.

| Original pet modifier key(s) | Attribute | `skipMainGear` |
|---|---|---|
| `melee_dps`, `melee_attack_speed_mul` | 33, 54 | false |
| `melee_critical_rate`, `dodge_rate`, `dodge_mul` | 16, 178, 180 | false |
| `armor`, `magic_resist` | 8, 64 | false |
| `health_regen`, `persistent_health_regen` | 11, 67 | false |
| `mana_regen`, `persistent_mana_regen` | 12, 68 | false |
| `dex`, `int`, `spi`, `max_health`, `max_mana`, `move_speed` | 1, 3, 4, 6, 7, 10 | true |

These modifier numbers retain raw attribute units: no DPS `0.001`, critical/dodge
percentage conversion, or persistent-regeneration `0.2` scaling. Original labels
sum their declared affect keys; for example dodge sums `dodge_rate,int,dex,dodge_mul`
and regeneration sums its direct, spirit and persistent fields. The private
`GetPetModifierColor` treats only `-0.009 < sum < 0.001` as zero; positive becomes
green, negative red, and zero the default color. Missing modifier keys would
therefore break the original loop rather than merely lose an optional tooltip.

Disassembly is retained in ignored `data/extract/aa-ui/` artifacts
`native-pet-unitinfo-source-20261018.txt`, `native-pet-stats-source-20261018.txt`,
`native-pet-unitmodifier-source-20261018.txt`, and
`native-pet-attribute-components-20261018.txt`. The app owns real actor, formula,
equipment and buff integration. This source-only audit changes no Lua toolkit
code or visual output, so it adds no screenshot or performance claim.

## Original widget edge resizing (2026-10-18)

`UseResizing`, `SetResizingBorderSize`, `SetMinResizingExtent`,
`SetMaxResizingExtent`, and `StartSizing` now implement the native resize path.
The unchanged quest tracker enables its authored bottom-only 10-pixel border,
sets limits from its content/screen size, and can be resized with the pointer.
Original chat also calls these setters; no application hook is required.

Bindings `39265450`, `392659b0`, `39265a70`, `39265b00` and `39265130` lead to
widget properties `+0x244..0x264`, motion capture `39278a10`, and incremental
motion `3927a110`. Generic defaults are zero borders/min/max and resizing off.
The setters clamp negative values to zero. Setting limits itself does not resize
the widget. `StartSizing(point)` explicitly captures independently of the
automatic enable flag; `StopMovingOrSizing` and button release end capture.

Native `39277580` uses strict interior left/top/right/bottom strips in **physical
pixels**, with the mask table `399c533c` selecting eight edges/corners. Border
thickness does not grow with widget/UI scale. Automatic left-button border hits
consume before that widget's `OnMouseDown` and suppress the release click. Motion
uses current pointer deltas and `39277650`'s direction/re-entry gates; dragging
beyond a limit and reversing outside the edge does not immediately shrink it.
Left/top retain the opposite edge when clamped; right/bottom retain the origin.
Min/max extents use widget units, transformed by its effective scale. The toolkit
inverts the current widget/ancestor transform when applying the new logical
rectangle, preserving scaled placement and updating ordinary anchored children.

`UseResizing(true)` creates the original `ui/hud/chat_guide.dds` nine-part guide:
coords `(0,0,16,16)`, insets `(7,7,8,8)`, top-left `(-3,-1)`, bottom-right `(2,5)`.
It starts hidden and appears on a resize edge. Disabling removes the guide.
Settled hover does not rewrite the anchor tree or allocate drawable objects.

Resize cursors use the existing independent OS-cursor hook. `39277fc0` requests
native enums 8/9/10/11 for the two diagonals, horizontal and vertical; handle
selection `3921aa80` indexes `39ac694c`, adding 14 for the large theme. Loader
`3921e2cc..3921e31b` / `3921e3da..3921e429` supplies normal resource groups
161/163/165/167 and large groups 162/164/166/168. All eight original resources
are 32x32 BGRA, hotspot `(16,16)`; large resize artwork is not stretched to 64.
The cache reads these authored resources only when requested. Resize overrides
the repair request, which resumes afterward, and the prior OS component is
restored when neither is active. Held item/image state is unchanged.

Source excerpts are in ignored
`data/extract/aa-ui/native-widget-resizing-source-20261018.txt`. This milestone
does not broaden generic mouse-event bubbling or invent additional movement
notifications; existing `OnBoundChanged` layout delivery updates dependants.

Validation: the library plus resizing, drag, popup lifecycle, screen bounds,
scale/clip and grid/bounds suites passed 54 tests (one existing font-metrics
test ignored). Four new interaction regressions cover all eight edges/corners,
click consumption, limit re-entry, explicit capture, UI/widget scale and the
unchanged original quest tracker. Two cursor regressions verify all eight
authored resources and resize -> repair -> prior OS cursor restoration.

`examples/native_resizing.rs` uses aa-dev default plugins and `DevPlugin`.
The unique `--shot` capture `shots/aa-ui/native-resizing-final-20261018.png` was
inspected: original three-quest tracker, physical bottom drag from 300 to 430
units, original guide, zero script errors, 360-408 FPS, automatic closure and
exit 0. Shared HUD/map framing was compared with `reference/ingame/spawn/real.png`;
that reference has an empty tracker and no resizing gesture. This fixture omits
the 3D world and changes no font, authored content layout or UI calibration.

## Original battlefield progress getters and clock (2026-10-18)

These are source contracts for the app's received-state bridge, not new aa-ui
game-state defaults. The native bindings for `GetProgressEntireInfo`
(`39456450`), `GetProgressTimeInfo` (`39456540`), `GetProgressScoreInfo`
(`39456630`), and `GetProgressVictoryConditionInfo` (`39456720`) all call
`3936cfd0`: states exactly 5/6/7/8 are admitted, and every other state returns
**zero Lua values**. The returned state names are `JOINED`, `COUNTDOWN`,
`STARTED`, and `ENDED` (native string pointer table `39ac74ec`).

Time builder `3936da30` always adds `state` and `nowtime`. Its phase-dependent
keys are:

| State | `startTime` | `endTime` | `nowtime` |
|---|---|---|---|
| JOINED (5) | omitted | omitted | fresh mapped timer value |
| COUNTDOWN (6) | manager `+3ce8` | manager `+3cec` | mapped timer + 6000 |
| STARTED (7) | manager `+3cec` | `startTime + manager[3ce0]` | mapped timer |
| ENDED (8) | manager `+3cf0` | `startTime + manager[3ce4]` | mapped timer |

All additions above use wrapping 32-bit arithmetic. Time values are unsigned
u32 converted to f32 before becoming Lua numbers (`3901c950` and the inline
`nowtime` conversion); they are neither decimal strings nor exact f64 integers.
The countdown adjustment applies to `nowtime`, not every phase's clock.

Score builder `3936dbd0` returns the following keys. The selected team starts at
`manager + 58 + manager[8] * 1df8`; its index is the accepted/received corps,
not the player's current faction or the invitation's separately stored choice.

| Key | Native source |
|---|---|
| `myKillStreakPoint` | signed i32 manager `+3cd0` |
| `myCorpsVictoryState` | signed i32 selected team `+0` |
| `myCorpsKill`, `myCorpsDeath` | unsigned u16 selected team `+c`, `+e` |
| `score1`, `score2` | signed i32 manager `+3c48`, `+3c4c` |
| `killScore1`, `killScore2` | signed i32 manager `+3c50`, `+3c54` |

Each score is narrowed to f32. Victory builder `3936de90` adds
`victoryScore` from signed i32 `+3c`, `victoryKillCount` from signed i32 `+40`,
and `victoryTarget` from signed i32 `+44`, all via f32. The received kill count
is originally u16, promoted on receipt. Target equal to the native invalid-NPC
sentinel becomes numeric zero; it is a template ID, not a boolean objective flag.

Entire builder `3936e7a0` adds `corpsName`, `corpsName1`, `corpsName2`, and
signed-i32-to-f32 `endReason` (`+38`), then merges the complete Time, Score and
Victory fields into the same table. `corpsName` resolves the stored rule ID
(`+c`) and invitation choice (`+18`): 0/1 uses that rule's faction name,
2 returns literal `ANY_CORPS`, and unresolved/other cases return `""`.
`corpsName1/2` are separately cached faction names (`+50/+54`) resolved from the
two received faction IDs (`+48/+4c`), with an empty string for failed lookup.
Reply acceptance `3936f646..3936f654` replaces the selected-team index `+8`
with its correlated accepted corps; it does not rewrite invitation choice `+18`.

`GetMyKillstreakSkillsInfos` (`39456000` -> `3936e5e0`) is **not state-gated**.
It always returns 24 numbered rows, indexed 1..24, each with unsigned-u32-to-f32
`skill_type` from manager `+3c58 + (row-1)*4` and boolean `enable` from
`+3cb8 + (row-1)`. Its top-level `skill_count` counts IDs different from the native
invalid-skill sentinel. That sentinel (`3a172cdc`) is zero, verified by a bounded
read-only read in the existing original process after module verification.
Zero-ID and disabled rows remain present; neither filtering nor compaction is
part of this getter.

Reset `3936d0d0` zeros `endReason`, victory score/count, both full team blocks,
killstreak points, all skill enables, three phase durations and four time
anchors. Skill IDs and victory target use their zero invalid sentinels. The
JOINED receiver `3936d890` requires state 4 and matching zone, instance and
accepted corps. It stores received score/count/target and the three phase
durations at `+3cdc/+3ce0/+3ce4`, sets state 5, clears time anchors
`+3ce8/+3cec/+3cf0/+3cf4`, resolves the two faction names, copies all 24 skill
IDs, disables all skills, and clears the four aggregate score fields. Wrapper
`3936e0c0` raises `ENTERED_INSTANT_GAME_ZONE` only after that receiver succeeds.
No start/end key is invented merely because the reset anchors contain zeros.

The original waiting alarm's Joined-event branch shows its people-waiting
message without reading start/end times. The action bar reads the killstreak
table's `skill_count`. The mini scoreboard opens on `INSTANT_GAME_START` or
`ENTERED_WORLD`, not the Joined event; its EnteredWorld path can attempt timing
while state is JOINED. Preserve the native key omission and event sequence
instead of supplying fictional start/end values to bypass that timing edge.

### Native mapped clock producer

`3936da9c..3936dabe` queries the environment's timer (`[3a5bb1d8] + 14`), calls
virtual `+10` with timer index 0, then calls virtual `+20` with that result.
There is no battlefield-manager `nowtime` field. Verified read-only runtime
source identifies those virtuals as CrySystem `3656f740` and `3656ea30`:
timer-0's cached i64 microseconds become milliseconds through integer
`*1000/1000000`, then the timer's embedded `xlcommon::TimeKeeper` maps them.
Its named `GetRemoteTime` export (`33025600`; CrySystem import `36608788`)
returns `localMilliseconds + remoteAnchor - localMean`, wrapping u32.
Constructor `330321d0` zeros both anchors and five sample pairs; `Init`
(`33032010`) resets sample index and remote anchor. The first sample replaces
the retained local mean after Init.

The actual sample producer is **proxy Pong 0x13**. CryNetwork registration
`1626bbff` binds callback `1626b6e6` through `16270d7e`, whose packet constructor
`1626c606` writes opcode `13` and vtable `1629b190`. Serializer `16267a47`
reads four i64 fields in order (`tm`, `when`, `elapsed`, `remote`) followed by
two u32 fields (`local`, `world`), a 40-byte payload. Visible callback body
`1626b0d8..1626b119` calls timer virtual `+1c` with
`AddSample(packet.local, packet.world, freshTimer0Milliseconds)`. It does not
use the packet's four i64 values for this mapping. The call requires a nonnull
connection-owned listener pointer at `+2c`; that pointer starts null and also
receives ordinary virtual listener calls (`16269eaa..16269ebb`). Its protected
attachment path has not been interpreted as an invented enable flag.

`TimeKeeper::AddSample` (`33032040`) computes wrapping RTT `receive-send`,
interprets it as signed i32, and multiplies its f32 conversion by `0.5f`.
For the first sample, `remoteAnchor=packet.world`, and all five pairs become
`(RTT, trunc(f32(receive as i32)-halfRTT))`. Later samples use
`trunc(f32(remoteAnchor-packet.world+receive as i32)-halfRTT)` at the ring index,
advancing modulo 5. The five pair pointers are sorted by signed RTT comparison,
with sorted index 2 as the median. The native threshold is signed truncation of
`(square(r0-r2)+square(r1-r2)+square(r3-r2))/5`, using wrapping i32 arithmetic:
the highest sample's squared difference is computed but is not included in
that threshold sum (`3303211e..3303214b`). It averages the signed local samples
whose squared RTT difference is at most this threshold; no selected samples
sets the mean to zero. Preserve that actual instruction sequence rather than
substituting a conventional statistical filter.

The imported MSVCR100 `qsort` (`63194a80`) routes five elements through its
short sort (`63194993`): for last index 4 down to 1, scan indices 1..last,
replace the selected maximum only for a positive wrapping comparator result,
and swap that maximum with the last element. This preserves behavior even
when extreme RTT differences make the comparator nontransitive. A local
Unicorn oracle executes the original AddSample, comparator and CRT short-sort
instructions without mocked sorting. Its seven vector groups cover odd RTTs,
outliers, f32 boundaries, u32 wrap, zero remote anchors, threshold overflow with
zero selected samples, and nontransitive RTT comparisons. Results are in
`data/extract/aa-ui/native-timekeeper-oracle-20261018.json`; only the local
emulator's synthetic memory is modified.

The separate `SCInstantGameStartPacket.now` value is unused by the native start
receiver `3936d9b0`; that receiver queries this mapped timer and adds 6000.
The read-only server lane currently gives proxy Pong and battlefield scheduling
independent `OnceLock` tick-counter origins. Those producers do not establish
a common native remote-clock domain. The app/network owners must account for
that integration gap; swapping in the unused received `now` field would change
the proven client contract.

Source artifacts are ignored `data/extract/aa-ui/` files
`native-battlefield-progress-bindings-20261018.txt`,
`native-battlefield-progress-source-20261018.txt`,
`native-battlefield-clock-source-20261018.txt`, and
`native-progress-time-runtime-20261018.txt`. Runtime inspection used only
`PROCESS_VM_READ` in the already-running original client, with `StandMode` at
`399a08dc` verified before each read; no input, writes or injection. This is a
source audit, so it adds no new rendered scenario or performance claim.

### Scoreboard member details, leave, and killstreak slots

`GetMembersDetailInfo` binds `39456810` -> `3936ebb0`, admitting the same native
states 5..8 as the other progress getters and returning zero Lua values outside
them. It returns **two** values: a table with two numbered team tables, and a
signed-i32-to-f32 own-member index. Each team retains received member order,
including an empty table when its count is zero. Native received team storage
holds at most 50 members. The second result starts at zero and is assigned the
**zero-based** row index when a received character ID equals the local player's
character ID; it is not a team number or an `isMe` row field.

| Member key | Native value |
|---|---|
| `name` | received member name |
| `memberScores`, `memberBonus` | `f32(received signed i32) * factor` |
| `memberKillstreaks` | received u16 -> f32 |
| `memberNPcKills`, `memberNPcKillAssists`, `memberNDeaths` | received u16 -> f32 |
| `isWinner`, `noDeath`, `topPcKill`, `topNonPcKill`, `isLoser` | booleans from received bonus-set bits 0..4, respectively |

There are no character-ID, player-stat or team-victory-derived additions to those
12 keys. The factor is `f32(trunc(manager[3cd4]/100))` when the signed field is
positive, otherwise `1f`; the integer division precedes conversion, so a
positive value below 100 produces zero. Reset `3936d172` clears this field.
Its later producer is `OnScheduledEventStarted` (`391e1e10`): received packet
field `+4c` goes through setter `3936cec0`. It is separate from the battlefield
result packet. Constructor/reset `endReason` (`+38`) is also zero, and Joined
does not replace it with -1 or another fabricated reason.

`AskLeaveInstantGame` (`39455de0`) returns zero Lua values regardless of whether
the request is permitted. Native sender `3936d210` permits only state 8 (ENDED),
then sends empty opcode `e5`. It leaves the displayed result/state intact until
the actual load-instance path `3936f1c0` resets states 5..8 and raises
`LEAVED_INSTANT_GAME_ZONE`. The app owns this already-typed network operation.

The unchanged battlefield action bar constructs ten `ISLOT_INSTANT_KILL_STREAK`
(244) slots. Native `EstablishSlot` (`392c6b90`, case `392c6c6c`) subtracts one
from the Lua index and calls `392c64b0` -> `3936d080`: indices 0..23 select the
24 received raw skill IDs; every other index returns invalid ID zero. This
lookup itself has no battlefield-state gate.

Binding resolves the ordinary skill descriptor through `392c5790`. For type
244, that skill must carry native tag `39ac6f08` = 204, verified by a bounded
module-checked read; compact `tags[204]` is `kill streak`, with membership in
`tagged_skills`. The tag creates raw SkillAccessor kind 15. Type 244 has no
fallback kind when the tag is absent, so that binding fails/clears. Kind 15
reports `GetBindedType() == "skill"` (`3925689a`), uses its raw skill ID for
ordinary tooltip content, and obtains its icon from the descriptor's icon ID
(`39208f10`). It is not substituted with a learned-player skill or compacted
learned-skill list.

Can-use update `3920c663..3920c69c` combines the ordinary skill predicate with
`3936d590`: state must be at least 5, then the **first matching raw skill ID**
among all 24 supplies its received enable byte. Duplicate IDs consequently use
that first row's enable, even when the displayed slot is a later duplicate.
The complete ordinary predicate `39209a70` is protected and remains open.
Clicks use raw SkillAccessor `3920a220` / `3920a170` -> `39209d80` -> ordinary
actor skill planner `393989e0`, which includes target selection, forced-self
modifiers and target-type-specific preparation. This audit does not justify a
new direct killstreak packet or replacing that planner with a plain skill cast.

Source excerpts are in
`data/extract/aa-ui/native-battlefield-members-slots-source-20261018.txt` and
`native-battlefield-slots-runtime-20261018.txt`. This is source-only work; the
app owns original scoreboard/action-bar integration and its rendered checks.

### Status-bar child anchors and filled image geometry (2026-10-18)

The unchanged `components/castingbar_view.alb` attaches its 21-pixel light
decoration with `statusBar:AddAnchorChildToBar(lightDeco, "TOPLEFT",
"TOPRIGHT", -15, -2)`. That method was a stub, and the bar image's layout
rectangle still covered its full owner while only the renderer cropped it.
The casting decoration consequently could not follow the displayed fill.

Native registration `392583d3` selects wrapper `39257b30`, which calls
`392c7f10`. It anchors the supplied widget/drawable to the existing bar image
at status-bar `+2cc` through ordinary `39276dd0` anchors; absent child or bar
is a no-op. aa-ui now implements that method with the actual internal image
ID. Its filled rectangle participates in normal anchor resolution, and the
renderer crops the texture once against that already filled geometry.

Update `392c7f80` uses horizontal top-left/bottom-left anchors and a
`width * fraction` extent. Vertical uses top-left/top-right anchors and
`height * fraction`, filling downward from the top. The native point table
`39ac6a5c` and parser `3922c0a0` establish those point identities. The
`min == max` and `value == min` branches only hide the image (`392c8250`),
retaining its last extent and anchors. The filled axis therefore retains
its last length while those cross-axis anchors still follow the owner.
aa-ui retains the last positive extent before entering either hidden state.

The original vertical UV path has a distinct quirk, which is preserved:
`392c81bd..392c81c7` writes `original height * fraction` into the UV
**width** slot and leaves UV height unchanged. This is not inferred from
the geometry. Wrapper `392574c0` passes x/y/width/height in that order,
`392c8410` stores them unchanged, and image conversion `3928b260` confirms
their use. Horizontal crops its UV width normally. Value interpolation is
covered by the following milestone.

`tests/statusbar.rs` checks missing-bar no-op, ordinary child target IDs,
moving fill endpoints, range/owner changes, cached hidden lengths in both
orientations, the native vertical UV quirk, and the unchanged original
casting-bar view at 25/50/75 percent. `native_castingbar` is the shared
original-component example using the aa-dev harness. Source excerpts:
`data/extract/aa-ui/native-statusbar-fill-source-20261018.txt` and
`native-statusbar-coords-source-20261018.txt`.

Validation: `cargo check -p aa-ui --lib` passed. The library and selected
statusbar/native/HUD/animation/resize/scale/clip/popup/bounds tests passed
**81 tests**, with one existing font-metric test ignored. Built and ran the
copied `native_castingbar` example with
`--shot shots/aa-ui/native-castingbar-20261018.png --shot-after 8`; inspected
that unique image, with the authored flare at all three fill endpoints.
The aa-dev run logged **316.1–498.9 FPS**, zero Lua errors and no WARN/ERROR,
and process 34436 closed automatically after the capture. The spawn reference
was inspected for HUD context; it contains no active cast, so this is an
original-component/native-geometry check rather than a claimed active-cast
pixel comparison.

### Native HP gauge and after-image interpolation (2026-10-18)

The unchanged `unitframe/unitframe.lua` `SetHp` animates player/target HP
gains for 0.2 seconds, and their HP after-image losses for 0.3 seconds.
The toolkit previously jumped both immediately. Native wrapper `39257a80`
accepts an optional boolean and numeric duration in **seconds**, not
milliseconds; untyped substitutes do not enable interpolation. `GetValue`
continues returning the new target immediately while a separate f32 offset
controls the displayed image.

Native `392c82e0` clamps the new i32 target to the i32 range. A same-target
call leaves the existing animation settings intact, even when it supplies
`false, 0`; this matters because the original unit frame refreshes its HP
every frame. For a changed target with animation enabled and positive
duration, the normal speed is `abs((displayed - newOffset) / seconds)`.
An interruption with nonzero old speed and remaining delta instead uses
`remaining = abs((displayed - oldOffset) / oldSpeed)`, then
`abs((displayed - newOffset) / remaining)`, preserving the previous remaining
duration. The newly supplied duration is not used in that branch.

`392c7f80` steps by `(speed * dt_ms) * 0.001` in f32, with the original
strict overshoot comparison and operation order. Tick `392c8280` requires
both animation and running flags. Widget update `3927e286..3927e29f` invokes
that virtual update after its Lua `OnUpdate`; child update traversal checks
the child's `+ba` visible flag (`3927e1f2`). aa-ui advances visible gauges
after Lua updates and preserves their state while hidden. Minimum/zero and
degenerate-range updates hide immediately and retain the displayed offset,
extent and running state, as the native early return does.

The related scalar prerequisites are now native too: constructor
`392c7cb9..392c7cd0` initializes the range and target to zero; reversed
`SetMinMaxValues` ranges are ignored (`392c82b0`); accepted range changes
preserve the existing target. Internal minimum/maximum/target fields are
i32, while displayed offset and speed are f32. The native update compares
the adjusted target with the absolute maximum (`392c7fec`), including its
nonzero-minimum range-change behavior.

Six groups of original arithmetic were executed locally in Unicorn:
gain, loss/same-target, interruption, zero while settled/running, and range
changes. The oracle executes unmodified `392c82e0`, `392c82b0`, `392c8280`
and `392c7f80`; only anchor/extent/UV helpers are stubbed. It reads the
offline DLL and performs no live-client execution or writes. Exact recorded
f32 checkpoints, including interrupted speed 299.9999694824219 and displayed
20.000001907348633 before the final snap, are checked by regression tests.
Artifacts: `data/extract/aa-ui/native-statusbar-animation-source-20261018.txt`,
`native-statusbar-animation-oracle-20261018.json`, and `statusbar_oracle.py`.

The shared `fake/healthbar.rs` fixture obtains the registered original
player frame through `ADDON:GetContent(UIC_PLAYER_UNITFRAME)` and updates
the fake received HP together with its unchanged `SetHp` method. Its
original per-frame refresh remains active. Tests verify HP gain and damage
after-images in that original path; `native_healthbar_animation` captures
the original damage fade near its midpoint using aa-dev.

Validation: `cargo check -p aa-ui --lib` and **87 selected tests** passed,
with one existing font-metric test ignored. The library, statusbar, original
widgets/HUD, animation, resize, scale/clip, popup and screen-bounds suites
were included. Built and ran the copied `native_healthbar_animation`
example with `--shot shots/aa-ui/native-healthbar-animation-20261018.png
--shot-after 8`; inspected the unique PNG showing the main fill at 40%
and the original brighter after-image still near 70%. The aa-dev run logged
**250.1–314.1 FPS**, zero Lua errors and no WARN/ERROR. Process 25604 closed
automatically. The spawn reference provides original HUD/texture context;
it has no matching active damage sample, so no timed pixel match is claimed.

### Direct text-drawable style methods (2026-10-18)

The original action-slot view creates required-point and skill-level text
drawables and calls their direct `SetAlign(ALIGN_CENTER)`. Direct alignment
was a stub (426 calls in the full HUD), although `text.style:SetAlign` was
implemented. Direct `SetColor` also wrote only the generic drawable tint,
which the text renderer did not read. These calls now update the existing
`TextBlock.style` fields, so the original direct methods affect the rendered
glyphs without introducing a second style or changing fonts.

The native bindings establish the shared state: direct alignment wrapper
`3925b110` and widget-style alignment wrapper `39235000` both call
`39274940`, which writes TextStyle `+30`. Direct color wrapper `3925b1d0`
calls `39274950`, copying RGBA to TextStyle `+20`. Shadow wrapper
`3925b0d0` calls `392748a0` (flag bit 1), and outline wrapper `3925b150`
calls `392748e0` (flag bit 4). Direct shadow/outline now reach the same
existing rendering fields as their style methods. Generic image/color
drawable tint behavior is preserved. Snap behavior remains open; its
existing flag is not consumed by the current text renderer.

`tests/drawable_text.rs` verifies all nine existing alignment values,
independent image tint, identical direct/style glyph quads including RGBA
and decoration, and the unchanged original action-slot view's centered
required-point text and authored font selection. The shared fixture uses
the original ability-view slot constructor and supplies three unlearned
passive-buff entries with required-point counts 1/2/3 through `SlotContent`.
Their ordinary content refresh controls text visibility; explicit
white/blue/red test colors and shadow/outline enable visual inspection.
Source: `data/extract/aa-ui/native-text-drawable-style-source-20261018.txt`.

Validation: `cargo check -p aa-ui --lib` and **52 selected tests** passed
(drawable text, animation, original natives, tooltip, scale/clip and statusbar).
The three drawable tests were rerun after making the visual fixture use the
ordinary passive-slot content path. Built and ran the copied example with
`--shot shots/aa-ui/native-drawable-text-final-20261018.png --shot-after 8`;
inspected that unique PNG with all three original centered required-point
digits still visible after normal refresh. The run logged **439.3–508.7 FPS**,
zero Lua errors and no WARN/ERROR, and its process 25604 closed automatically.
The spawn reference was used for HUD context; it has no matching passive-slot
sample. The fixture colors intentionally multiply the authored Snail image
font, rather than replacing it with a guessed font or recolored asset.

### Constant equipment-button skill cooldowns (2026-10-18)

Original `chracterinfo/equipped_item.lua` sets an equipped item's icon and
then calls its constant button's `SetSkill(tooltip.skillType or 0)`. That
native was a stub. It now records a cooldown identity without creating a
slot accessor or changing the existing item icon and original click handler.
The app supplies `AAUI.SkillCooldown(rawId)` independently of learned-skill
metadata: two u32 millisecond values `(remaining, total)` for an existing
descriptor, or nil/no results for an unknown descriptor. Known descriptors
without an actor or timer return `(0, 0)`.

Native setter `392325b0` -> `39280c20` writes skill `+5cc`, invalid item
identity `+5c8`, and clears ready flag `+5d0`. Initializer `39284865..86`
sets invalid identities and ready=false; a read-only, module-verified read
of original process 48924 confirmed both invalid globals `39d2e568` and
`39d2e57c` are zero. Virtual update `39281df0` resolves the descriptor
through `396f6390`, then calls `393919f0` -> current actor's cooldown
manager -> `3989a3e0`. Active timers emit `OnContentUpdated(self,
"skill_cooldown", remaining, total)` each visible update, clearing the
ready flag. Zero remaining emits once and sets ready=true. Missing
descriptors emit zeros while ready=false without setting it; rebinding,
including the same ID, resets it. Counts reach Lua through native f32
conversion, including the original rounding of large unsigned values.
The unchanged `AddOnUpdateConstantCooldown` callback owns `SetCoolDown`
and its original completion animation.

The timer-selection contract is also source-closed. `3989a3e0` passes
descriptor byte `+dc` to `3989a290`. If false, global remaining/total at
manager `+8/+c` is the first candidate; otherwise the initial pair is zero.
The getter then scans every ordered descriptor tag (`+238..23c`) against
received tag rows and finally direct skill rows. A candidate replaces the
winner only when its unsigned remaining time is strictly greater, so ties
retain the global/earlier-tag/earlier-row total. `LoadSkillDescs` SQL at
`39a06a90` reads column 61, `ignore_global_cooldown`, into `+dc` at
`39731f04..24`. `LoadSkillTagsRelation` (`395d15a0`) executes exactly
`SELECT skill_id, tag_id FROM tagged_skills` and appends each matching tag
to `+238` in SQL result order (`395d166a..1720`). This vector is all generic
skill-tag memberships, not only `skills.cooldown_tag_id`.

Source artifacts: `data/extract/aa-ui/native-cooldown-constant-source-20261018.txt`,
`native-cooldown-manager-source-20261018.txt`,
`native-cooldown-loader-full-20261018.txt`, and
`native-cooldown-tag-loader-20261018.txt`. Native `SetItem` descriptor
selection and the global update-suppression byte `3a5a4db8` remain outside
this milestone. Local-cast timer production is being audited separately;
the new hook does not invent a cast or derive timers from this widget.

Validation: `cargo check -p aa-ui` and **48 selected tests** passed
(constant cooldown, animation, original natives/slot state and statusbar).
The five constant-button tests cover constructor/unbound behavior,
active/ready/missing-descriptor transitions, rebinding, hidden widgets,
u32 identity/f32 callback conversion, and the actual original equipment
button constructor and cooldown handler with preserved item texture/click.
Built and ran the copied `native_constant_cooldown` aa-dev example with
`--shot shots/aa-ui/native-constant-cooldown-20261018.png --shot-after 8`.
Inspected that unique capture: original item 4045 icons show 8 seconds,
3 seconds, and ready. The run logged **188.8-274.7 FPS** during shared
builds, zero Lua errors and no WARN/ERROR; process 46448 self-closed.
The spawn reference supplies HUD context but has no matching equipped-item
cooldown sample, so a timed pixel match is not claimed.

### Native cooldown production audit (2026-10-18, app/state work pending)

The new constant-button observer above consumes timers; the following audit
does not make the existing cast/DB estimates exact native state. Normal
`OnSkillStarted` (`391d57f0` -> `3939a810`) starts a global timer at
`3939aa97..aadd` only on its successful path, with an existing source actor,
the final internal suppression flag false, and descriptor `timing_id == 1`
(`+1bc`). `default_gcd` (`+9c`) selects `39899f30`; otherwise positive
`custom_gcd` (`+94`) selects `39899df0`. Both query actual actor attribute
74 (`global_cooldown_mul`) through `398a0bc0`, clamp it below at -999, and
set global remaining and total together. Default duration is unsigned
`1000000 / (1000 + attribute)`; custom duration is unsigned
`wrapping(custom_ms * 1000) / (1000 + attribute)`. The addition also wraps.
An actor `+3af1` flag suppresses these writes; its gameplay setter is not
closed here. These offsets are relative to the cooldown manager's stored
unit interface pointer, which initialization `3936621f..37` sets to the
raw actor plus 8; the flag is therefore raw actor `+3af9`, not part of the
raw actor's mounted-object ID at `+3af0`. SQL field mappings are explicit
in `LoadSkillDescs`, including
column 135 `timing_id`, 45 `default_gcd`, and 43 `custom_gcd`.

The ordinary direct/tag cooldown begins in `OnSkillFired` (`391d58a0` ->
`3939b010`), at `3939b3d1..3eb`, rather than unconditionally at Started.
It requires the resolved source actor, decoded boolean `packet+160`
(native argument `+34`) false, `toggle_buff_id == 0` (`+1c0`) and nonzero
`timing_id`. It calls `39368f90` with descriptor `cooldown_time` (`+88`),
then `39368440` -> `3989a360` -> `3989f900` -> `39899f90`. If Fired has no
matching timeline, `3939b0b6..e9` first runs the Started setup, so a missing
Started packet does not imply the global timer is absent.
Serializer `397ca390` derives `packet+160` from **bit 1** of the final raw
wire flags byte (`raw_final_flag & 2`); bit 0 instead populates `+14c`.
Thus raw flags value 1 does not suppress this cooldown path. The aa-net
executed serializer oracle covers all 256 flag values in
`data/extract/aa-net/native-cooldown-fired-result-oracle-20261018.json`.

`3989f900` evaluates a separate skill-modifier query with category 1 and
an initial flat/percentage pair 0/100. It invokes the native provider at
manager `+4`, actor collection `+3a8c` through `3989e5c0` (selector 1),
and the base collection selected by actor `+4a4/+4a8` through `3989e6a0`.
Callbacks `398a05b0/05f0` -> `398a0480` select matching skill-modifier rows
by attribute category, universal selector, exact skill ID or generic tag
membership. Modifier type 0 adds flat; type 1 adds percentage; each value
is multiplied by its callback's stack factor. The base and flat sum wraps
as i32. Percentage 100 returns that sum directly; otherwise native f32
conversions/multiplications by percentage and 0.01 precede truncation to
i32. The final duration has a signed floor at zero. Substituting a generic
player attribute for these skill-specific sources would lose this contract.

Insertion `39899f90` chooses a tag row when descriptor `cooldown_tag_id`
(`+84`) is nonzero, otherwise a direct raw-skill row. It first searches
existing entries, stopping at the first matching ID. Only a strictly larger
unsigned new duration replaces that entry's remaining and total; duplicate
later entries are untouched. Zero duration and actor `+3af1` are no-ops.
On a miss at capacity 150 it compacts only positive-remaining rows in
original order, appends if space exists, or rejects if all 150 remain active.
This differs from the received `SCCooldowns` append-only handler.

Received stop durations are another authoritative producer. Native
`OnCastingStopped` (`391d5cf0` -> `39396000`) resolves the timeline's
actor/skill, inserts its nonzero received u32 duration directly through
`39899f90`, then ends the timeline. `OnPlotCastingStopped` (`391d5af0` ->
`3938c410`) and `OnPlotChannelingStopped` (`391d5b70` -> `3938d970`)
likewise apply the received duration to the tracked skill before ending it.
These paths do not derive a new duration from the database. The current
state fold discards these durations; this remains an integration gap.
Ordinary Ended/Stopped is not evidence for clearing an existing cooldown.
The separate local-player Fired call to `3939ce90` concerns a skill ID
extracted by `3939c950` from SpecialEffect 48; it is not a general reset
of every Fired skill. That replacement/combo subsystem remains open.

Artifacts: `data/extract/aa-ui/native-cooldown-producer-contracts-20261018.txt`,
`native-cooldown-packet-handlers-20261018.txt`,
`native-cooldown-fired-body-20261018.txt`, and
`native-cooldown-local-producers-20261018.txt`. The local, offline-only
`cooldown_producer_oracle.py` executes the original insertion and global
setter instructions, stubbing only memory-copy/log/cookie helpers and the
actor attribute query. Its ten recorded groups in
`native-cooldown-producer-oracle-20261018.json` verify duplicate-first
extension, preserved totals, expiry compaction, full-list rejection,
zero/flag no-ops, denominator clamping and custom multiplication wrap.
For example default attribute 333 yields 750 ms; -1001 clamps to a
1,000,000 ms timer; custom `0xffffffff` at attribute zero yields
4,294,966 ms. No live client execution, input, injection or source writes
were performed.

### Family identity, getters and original UI events (2026-10-18 audit)

Family ownership is not inferred from the first received roster. Created
(`39300250`), Described (`39300220`) and MemberAdded (`39300340`) copy the
keyed roster through `398a8df0` and call `392ff2e0` for every received row
with its family ID. That callback resolves a loaded character by persistent
character ID (`3935e370`, character field `+418`) and calls unit virtual
`+28`, whose setter `390b3a20` writes family `+3aec`. This includes the
actual local character when present. Replacing a roster does not clear IDs
of members omitted from the replacement. The subsequent `39300020` updates
the relationship/contact cache, not the family ID.

Removed (`392ff310` -> `398a8c90`) calls the same callback with zero for
every cached member before deleting the roster. MemberRemoved (`392ff910`
-> `398a8d40` -> `398a8160`) clears the first matching member's ID, removes
that first row, and preserves survivor order. If fewer than two remain,
it also clears the remaining member and deletes the keyed family. Unknown
family/member is a no-op. These callbacks do not require the actor's current
family ID to equal the removed packet ID; actual cached membership drives
them. Role/name/title/online updates do not change family identity.

Native registered getters are `IsOwner`, `IsMyFamily(name)` and `IsFamily`.
`IsOwner` (`39474b10` -> `392fff40` -> `398a81f0`) checks the actual local
persistent character ID in its own cached family and accepts any nonzero
role, independently of online status. `IsMyFamily` (`39474b90` -> `392fff70`)
uses case-sensitive `strcmp` against the own roster's names. Both return
false without an actor/cache. `IsFamily` (`39474c10` -> `39300000`) tests
actor family ID nonzero, even if no roster is cached. No `IsMember` or
`GetFamilyID` registration was found in this original X2Family binding;
those names must not be invented as native aliases.

Event IDs are registered at `39215d84..f7`. Events occur after cache and
loaded-character ID updates, without an additional own-family filter:

| Receipt/native entry | Original UI event and arguments |
| --- | --- |
| Created `39300250` | `FAMILY_MEMBER_ADDED(row0.name, row1.name, row1.title)`; native assumes its initial two-member roster |
| MemberAdded `39300340` | `FAMILY_MEMBER_ADDED(ownerName, addedRow.name, addedRow.title)`; received row index is zero-based |
| TitleChanged `392ffcc0` | `FAMILY_MEMBER_ADDED(ownerName, changedRow.name, newTitle)`, when found |
| MemberRemoved `392ff910` | `FAMILY_MEMBER_KICKED(name)` or `FAMILY_MEMBER_LEFT(name)`, after a successful removal |
| OwnerChanged `392ffa10` | `FAMILY_OWNER_CHANGED(newOwner.name)`, when found |
| Removed `392ff310` | `FAMILY_REMOVED()`, even when no cached roster existed |
| MemberOnline `392ffdc0` | `FAMILY_MEMBER_ONLINE(name, receivedBool)`, when found, including repeated status |
| NameChanged `392ff340` | `FAMILY_REFRESH()`, when found |
| Described `39300220` | No immediate UI event |

The owner name helper `398a8130` returns the last nonzero-role row's name,
or literal `"X"` when none exists. MemberAdded indexes the received roster
directly without a native bounds check; an app bounds guard is separate.
OwnerChanged clears every cached row's role even when the new owner is
missing; in that case there is no owner-change UI event. Dissolution from
MemberRemoved does not add an extra `FAMILY_REMOVED` event. Original
`relationship/family_tab.lua` refreshes its member data on these authored
events. Its member popup offers transfer/title/kick only for an owner;
the local row offers title for an owner and leave for a non-owner.

Native command guards are separate from that popup. Invite (`392ff770`)
allows an actor with no family or an owner of its existing family. Leave
(`392ff8b0`) only requires a nonzero family ID. Kick (`392ffa90`) and
ChangeTitle (`392ffb60`) require ownership and an exact cached target name.
ChangeOwner (`392ffc30` -> `398a8b00`) also rejects the local character as
target. These paths do not require the target online, mutate the cache or
optimistically assign ownership. Their protocol builders select CS1a/1c/
1d/1e/1f respectively. Native wrappers commonly check game flags `+14`
bit 2 before forwarding mutations. Transport/app integration remains with
the owning crates.

Sources: `data/extract/aa-ui/native-family-receive-dispatch-20261018.txt`,
`native-family-lifecycle-20261018.txt`,
`native-family-cache-callback-source-20261018.txt`,
`native-family-unit-id-callbacks-20261018.txt`,
`native-family-lua-controls-20261018.txt` and
`native-family-controls-final-20261018.txt`. This is a read-only audit;
no Family UI or outgoing request is fabricated by aa-ui.

### Pet equipment empty tooltip and refresh contracts (2026-10-18 audit)

`GetEquippedItemTooltipText` (`3946f1b0`) resolves the supplied unit with
`3945fcd0`, requiring a valid actor result of resolver kind 1. Invalid
unit/kind returns zero Lua results. Reading equipment does not require the
owned-Mate predicate. It formats the actual physical item at actor
`+3f8 + LuaSlot * c0`, equivalent to `+4b8 + (LuaSlot-1)*c0`; the native
wrapper has no explicit slot bounds check. A valid empty item still returns
a table: `39423560` calls `39421e20`, which creates the Lua table before
descriptor lookup and writes raw `itemType` and `lookType` under the full
mask. Thus an empty type zero yields `itemType=0, lookType=0`, not nil.
Only the local-player branch creates player-equipment container 1; the
nonlocal actor's formatter location stays invalid. No pet/player ownership
or player set-count substitution follows from this read API.

`UNIT_NPC_EQUIPMENT_CHANGED` is native event d5 and has exactly one numeric,
one-based physical slot argument, with no unit selector or object ID.
`3936b850` and `394efae0` assign each received physical item and emit this
event when `3933b1f0(actualActor)` identifies an owned Mate; they do not
compare old/new item values. The flag-location path `394e1d20` checks
container fc, current mate identity and raw slot at most 1b before writing
the received flags; dispatcher `394e111d` extracts the slot and emits
`slot+1`. The event lets the original pet equipment view refresh its icons
and details from received state. The read getter's permissive actor rule
does not replace `IsMateEquipableSlot`'s separate owned-Mate/slot-pack guard.

Sources: `data/extract/aa-ui/native-pet-equipment-tooltip-20261018.txt`,
`native-pet-equipment-empty-base-20261018.txt`, and
`native-pet-equipment-event-producers-20261018.txt`; the existing aa-net
mate-equipment notes cover the corresponding receipt and physical slot pack.

### Family invitation task and rejection option (2026-10-18 audit)

`OnFamilyInvitation` (`391d29e0` -> `392ff820`) captures received inviter
character ID, inviter name and title in task `392ff4c0` (name buffer 129,
title buffer 105 bytes). It returns silently if the game/local actor is
absent, with no reply. It does not test existing family membership or an
invented stale-invitation condition. Task vtable `399cd330` reports type 4,
button type 0 (OK/Cancel), and `isModal=false`. Common builder `3902d400`
explicitly maps virtual methods `+4/+8/+c` to `type/buttonType/isModal`;
`39300130` adds exact string keys `invitorName` and `familyTitle`.

Admission `392ff560` rejects an already queued type-4 task or option index
3 equal to 1. CVar registration `390fa82c..85c` binds that option at store
`+330` (`+324 + 3*4`) to `g_ignore_family_invite`, default 0, help text
`0(accept family invite), 1(ignore family invite)`. The Lua constant
`OPTION_ITEM_IGNORE_FAMILY_INVITATION` is 74 (`394950cf..d8`). Failed task
admission calls its result callback with false (`3902d5c0..d9`), whereas a
missing actor never constructs a task and never reaches that callback.
The callback `392ff590` constructs CS1b using the captured inviter ID/title
and dialog result. Its serializer `397c7340` emits ID, bool, string, as
independently recorded in the aa-net Family command audit. No reply changes
Family membership before a received roster/identity update.

Source: `data/extract/aa-ui/native-family-invitation-closure-20261018.txt`,
plus the earlier invitation task and common-behavior artifacts.

### Native cooldown suppression flag producer (2026-10-18 audit)

The cooldown manager's owner is the unit interface at raw actor `+8`.
Its checked byte `owner+3af1` is therefore raw actor `+3af9`, index 1 of a
four-byte flag array at raw `+3af8`. Handler `39186420` resolves received
runtime object ID at packet `+c`, then writes packet byte `+14` to
`actor[3af8 + packet.i32(+10)]`. Registration `391cd2d0` selects factory
vtable `399b6144`, whose `391b7b50` allocates opcode **0x203**. This is the
lane's `SCUnitGmModeChangedPacket`; serializer `397c7270` names its BC
`unitId`, i32 `mode`, and u8 `value`. Thus received mode 1 controls the
actual guard on local global/direct cooldown insertion. Other flag names
are not inferred from their adjacent positions.

Reset helper `398a1aa0` copies default word `3a5e136c` to raw actor `+3af8`;
that default is zero in the verified original module. Hydration
`398a218d..93` copies UnitState `+2020` to the same flags. These are not
four extra snapshot bytes: serializer `397c9870..9890` decodes a four-integer
PISH pack immediately after `raceGender`, storing its first decoded integer
at `+2020`. The reverse path `397c9a81..ad8` emits those four integers as
`pish`. Preserve the first integer's little-endian four bytes and apply
explicit SC203 mode updates; do not reinterpret mounting identity or add
an unobserved wire field.

Sources: `data/extract/aa-ui/native-cooldown-suppression-producer-20261018.txt`
and `native-cooldown-suppression-default-readonly-20261018.txt`. This audit
performs no live writes, injection or input and does not introduce an aa-ui
network-state dependency.

### FeatureSet level-limit field types (2026-10-18 audit)

Original `X2Player:GetFeatureSet` (`39496b20`) reads feature storage at
game `+14`. `39497025..58` exposes unsigned byte 1 as numeric `levelLimit`;
`39497071..a4` exposes unsigned byte 8 as numeric `mateLevelLimit`. Both
convert to the native f32 script number. These fields are not booleans or
derived from a unit's current level/XP. The received `InitialConfig` already
retains the feature byte vector and exposes both byte positions through
the aa-net config getters. Lane `FeatureSet::initialized` writes its actual
configured maximums to those positions (default player 55, Mate 50).

Source: `data/extract/aa-ui/native-feature-level-caps-20261018.txt`.

### Cached character clipping on original chat tabs (2026-10-18)

The unchanged original `chat/chat_window.lua` calls
`chatTabButton:SetAutoClipChar(true)` in `OnCreateChatTabButton`. This
method now draws the native cached fitting prefix while preserving the
complete `GetText`/text measurement source and original hover tooltip.
It does not set ellipsis or change font, color, shadow or outline.

Wrapper `3922d490` only stores the boolean at button `+592`; constructor
`392837fb..3801` initializes the flag and cached count to zero. Enabling
alone therefore preserves the prior count. Text setter `39281150`,
auto-resize setter `39281210`, and extent/font callbacks
`39280850/392808c0/39280930` refresh it for nonempty text. Native inset
setter `39280730` only copies four floats; changing insets alone retains
the cached count. Empty text likewise leaves that count intact. aa-ui
tracks resolved extent changes, including anchor-derived widths, and
uses the same existing font metrics for the fitting prefix.

Callbacks subtract left and right insets in f32 order; fitter `39274a70`
-> `392e3760` then accumulates kerning and glyph advances separately in f32.
Only a width strictly greater than the available width stops fitting, so
an exact fit is included. It counts Unicode codepoints, not UTF-8 bytes,
stops at NUL, and preserves the native newline quirk: newlines neither
count nor reset the pen, while the renderer `39280510` takes the first
cached-count UTF-32 characters. No alternate wrapping policy is added.

Sources: `data/extract/aa-ui/native-auto-clip-char-source-20261018.txt`,
`native-auto-clip-font-source-20261018.txt`, and
`native-auto-clip-lifecycle-closure-20261018.txt`. The offline Unicorn
oracle `auto_clip_oracle.py` executes the actual native fitting function,
supplying only glyph/kerning lookup results. Its seven groups in
`native-auto-clip-oracle-20261018.json` close exact Unicode fit, one-f32-ULP
exclusion, kerning order, newline behavior, missing-glyph counting, NUL and
negative width, without live client execution or writes.

Validation: `cargo check -p aa-ui` and **69 selected tests** passed, with
one existing font-metric test ignored. The final six clipping tests were
rerun after the callback-order/NUL refinement and all passed. Coverage
includes original chat construction/full-name hover, literal prefix render
equality, Unicode and f32 width boundaries, copper coin preservation,
cached enable/inset/empty-text behavior, anchor-derived extent changes,
original text decoration, chat transport, scale/clip and pet/tooltip overflow.
The copied `native_auto_clip` aa-dev example ran with `--shot
shots/aa-ui/native-auto-clip-hover-20261018.png --shot-after 8`; its existing
`UiVirtualPointer` holds the original first-tab hover. Inspected the unique
PNG showing three clipped labels and the complete first-tab tooltip.
Measured **259.7–294.1 FPS**, zero script errors and no WARN/ERROR;
process 27004 self-closed. `reference/ingame/spawn/real.png` provides the
original HUD/tab style context, without a matching long-name clipping
sample, so no exact pixel-match claim is made.

### Native fixed and auto chat-tab widths (2026-10-18)

`UseAutoResizingTabButtonMode` now selects fixed or automatic tab widths;
the original chat constructor's `true` call was previously ignored and
every chat window used a fractional auto width. Native constructor
`392cb6d0` initializes fixed mode, width 50, minimum 0, tab-area height 20,
zero tab offset/gap/insets, and a hidden 20-by-20 add button. Original
`chat/chat_window.lua` then applies its locale width, minimum 42, height
30 and automatic mode. Empty tab containers return without changing child
anchors or visibility, matching the early guard in `392c97e0`.

Automatic layout uses the local extent getter `39275480`, independent of
framebuffer/UI scale. The native f32 order is: tab offset + available add
button width, then `(tab count - 1) * gap`, left inset and right inset;
subtract that total from local window width, divide by tab count, and
floor. Clamp helper `3922aed0` checks the minimum first, then maximum.
Reversed limits are legal: minimum 90/maximum 20 yields 90 for a proposed
width below 90, and 20 for a proposed width above 90. Fixed mode uses the
configured width directly. Width/minimum/area-height setters clamp
negative inputs to zero. Tab height subtracts top/bottom area insets.

The actual chat binding is the TabController registration `3925a5c0`:
`SetOffset` at `3925a7b1` calls `39258990` -> `392c9f00`, storing tab
offset `+2e8`. `SetGap` uses `392589d0` -> `392c9f20` (`+2ec`). A different
native class's same-named `39258790` wrapper is not the chat binding.
`SetContentOffset` is a separate four-float method, `39258a50` ->
`392cad90`, writing message-area offsets `+324..330`; it is not mapped to
the single tab offset. Tabs now anchor to the preceding tab's right edge
with the native gap; the add button anchors bottom-left to the final
tab's bottom-right. Native callbacks `392ca020/30/40` relayout on geometry
changes. aa-ui handles explicit size/anchor changes and settles inherited
anchor-size changes on the next UI frame, preserving cached character
clipping and full text/tooltip getters.

Sources: `data/extract/aa-ui/native-chat-tab-layout-contracts-20261018.txt`,
`native-chat-tab-binding-closure-20261018.txt`, and
`native-chat-tab-width-next-20261018.txt`. Offline
`chat_tab_width_oracle.py` executes the actual `392c97e0` width calculation,
SSE floor, extent getter and clamp; only add-button availability is
supplied. Its eleven cases in `native-chat-tab-width-oracle-20261018.json`
cover fixed/auto, add reservation, zero tabs, reversed bounds, insets,
offset/gap, negative space, signed zero and a one-f32-ULP width boundary.

Remaining chat layout gaps are explicit: native add availability
`39512b60` -> `39500190` reads eight global configuration occupancy bytes
`+60..67`, keyed by native tab IDs 0..7. Allocation/load at
`39506ce8/39506e1f` marks the ID, deletion `39506b6d` clears it, and
`3951421e..2b` passes that same ID to `OnSetChatTabId`. These configuration
slots outlive a constructed chat widget. The current per-window
`AAUI.ChatTabs` bridge has no complete global configuration lifetime, so
aa-ui retains its existing requested add visibility rather than inventing
a visible-widget capacity rule. Native tab clipping and four-float message
content offsets are covered below; native sliding remains open work.

Validation: `cargo check -p aa-ui` and the final selected library/native,
chat-layout, character-clipping, chat-link/transport, screen-bounds and
scale/clip suites passed **72 tests**, with one existing font-metric test
ignored. Six layout regressions cover native defaults, fixed/auto toggles,
reversed limits, negative setters, exact f32 floor boundaries, zero tabs,
offset/gap/insets, bottom-aligned add controls, explicit/inherited resizing,
and physical pick/press/release selection in both original chat windows.
The original Unicode labels retain cached prefixes and complete tooltips.
Built and copied the aa-dev `native_chat_tab_layout` example, then ran
`--shot shots/aa-ui/native-chat-tab-layout-20261018.png --shot-after 8`.
Inspected the unique capture: original auto tabs are 104 px in the 345 px
window, and fixed tabs remain 140 px in the 430 px window. Logs report
**476.6–604.2 FPS**, zero script errors and no WARN/ERROR; process 48680
self-closed. `reference/ingame/spawn/real.png` supplies the original HUD/tab
style comparison; no matching resized-tab reference or pixel-parity claim.

### Physical font sizing and scaled character clipping (2026-10-18)

Original chat tabs now fit and draw ordinary button text using the same
native physical font size. Before this change, fitting used the local
width/base font while rendering rasterized at global UI scale and then
stretched the finished glyphs through parent/widget transforms. That
disagreed with the original at non-unit scales, including the original
1280-by-720 default UI scale of 0.85.

Native `3927b2f0` stores inherited scale `+68 = own +64 * parent +68`
and physical extents `+74/+78 = local +8c/+90 * inherited scale`.
Button font callback `39280780` selects the ordinary face through
`39275150`: truncate the f32 result `f32(base integer size * scale) +
0.3f32`. This is not nearest rounding: size 13 at scale 1.5 selects size
19, while scales 0.8 and 1.25 select 10 and 16. Font kinds 0/1 receive
that physical integer; ImageText kind 2 receives the base integer instead.
This callback is shared by the seven source-verified button-family
vtables, including check buttons and action/inventory buttons.

Geometry callbacks `39280930/39280850/392808c0` subtract **raw, unscaled**
left and right insets from the physical width, in f32 order, before the
existing native-prefix fitting operation. Drawing `39280e6c` scales those
insets separately. Draw bridge `392218a0` branches at `392219b4`: ordinary
faces draw with scale 1; ImageText draws with widget effective scale.
aa-ui now emits the ordinary button-text range directly at the physical
face size and transformed origin, bypassing only that range in the later
widget/ancestor transform pass. Artwork, other widget text, overlays,
clipping, shadows and font aliases retain their existing paths. Both
AutoClip flag states use this ordinary face selection, so toggling the
flag changes only the displayed prefix.

The fitting cache tracks resolved position/extent and inherited scale.
Accepted `SetScale`, including an identical value, refreshes the native
cache; native `3927d120` sets the geometry-dirty byte before calling
`3927b2f0` and `39276350(true)`. Active scale-animation frames use the
same path (`3927e18f..1d3`). Explicit `SetWidth/Height/Extent`
(`392763c0/39276410/39276460`) dispatch their callback even for unchanged
extents, propagating to anchor dependents. `AddAnchor` also dispatches;
`RemoveAllAnchors` (`39276a70` -> `39276940`) only detaches anchors.
Inset-only writes still retain the cached count until a real callback.
A small derived index of enabled caches propagates scale/geometry changes
to hidden tabs without scanning the full HUD. Opted-out UI-scale roots
retain their effective scale and cached prefix.

Style `GetTextWidth` binding `3925b320` passes `true` to `39275210`,
measuring the complete source with the selected physical font and
dividing by effective scale. Original `SetAutoResize(true)` likewise
uses that full local width, adds raw horizontal insets in the FPU, then
narrows the resulting local extent. Full text getters, the unchanged
original chat-tab name lookup, and its tooltip keep the complete string.
Original `baselib.lua:935..941` supplies the 0.85 default; the original
option conversion maps step 90 to 0.93.

Source artifacts under `data/extract/aa-ui/`:
`native-auto-clip-scaled-contracts-next-20261018.txt`,
`native-auto-clip-scaled-units-next-20261018.txt`,
`native-auto-clip-scaled-font-next-20261018.txt`,
`native-auto-clip-scale-lifecycle-next-20261018.txt`,
`native-auto-clip-render-and-forced-callbacks-next-20261018.txt`,
`native-auto-clip-anchor-callbacks-next-20261018.txt`, and
`native-auto-clip-fulltext-metrics-next-20261018.txt`.
Offline `auto_clip_scale_oracle.py` executes actual `39275150` in nine
groups, supplying only the font factory; `auto_clip_geometry_scale_oracle.py`
executes actual `39280930` -> `39274a70` -> `392e3760` in five groups,
supplying only glyph/kerning accessors. All fourteen groups pass, without
live-client execution or writes. Supplied glyph advances prove width
units, without claiming measured FreeType font metrics.

The ImageText fitting branch uses the supported base-size bitmap metrics
and retains the existing bitmap drawing backend. Native ImageText's
zero-height/baseline and alignment behavior (`392ef420/392ef430`) remains
open; this change does not claim full ImageText raster parity. General
FreeType-versus-Swash glyph rasterization and invalid/nonpositive native
font-size fallback remain separate boundaries.

Validation: `cargo check -p aa-ui` and **94 selected tests** passed, with
one existing font-metric test ignored. Eight new regressions cover native
physical-face rounding, parent/widget/global scale, hidden and unchanged
callbacks, animation ownership, unscaled fitting insets, scaled Unicode
exact-fit/one-f32-ULP exclusion, full-width auto-resize, ImageText base-size
fitting, and original 1280-by-720 chat hover/physical pointer presses at
0.85 and 0.93. Existing prefix/copper-glyph, chat layout/link/transport,
animation, drawable-text, native widgets and pet/tooltip overflow suites
also pass. The aa-dev `native_auto_clip_scale` example supports
`--size 1280x720` and optional `--option-step 90`; it applies the original
Lua conversion after three seconds and holds the first tab's full-name
tooltip using `UiVirtualPointer`.
Inspected both unique captures,
`shots/aa-ui/native-auto-clip-scale085-20261018.png` and
`shots/aa-ui/native-auto-clip-scale093-20261018.png`: original labels
retain clipped prefixes while the complete first-tab name remains in the
tooltip after the option change. Measured **164.9–222.0 FPS** at 0.85 and
**216.6–249.5 FPS** in the scale-change run; both logs report zero script
errors and no WARN/ERROR. Owned processes 44000 and 19844 self-closed.
`reference/ingame/spawn/real.png` supplies the original HUD/tab style
context; no matching scaled long-name reference or pixel-parity claim.

### Independent chat content anchors (2026-10-18)

Original message frames now use the complete chat-window width. The
previous layout reused the tab area's right inset, subtracting another
10 local units even though original `OnCreateChatMessage` already sets
its own 10-unit text inset. This unnecessarily wrapped messages and
their clickable links earlier than the native client.

Constructor `392cb932..992` initializes four content offsets to zero.
Binding `39258a50` -> `392cad90` stores the four f32 values at `+324..330`
without clamping, negation or conversion to tab offsets, then immediately
calls content layout `392ca610`. Each non-null message frame receives
these anchors relative to its chat window:

```
TOPLEFT     = (offset[0], f32(tab_area_height + offset[1]))
BOTTOMRIGHT = (offset[2], offset[3])
```

Positive right/bottom offsets extend the frame; negative values pull
those corners inward. This layout visits hidden frames too. It also runs
after tab insertion (`392cc240/392cc349`). Ordinary tab layout
`392c97e0`, including selection, tab-area-height and geometry callbacks,
does not call it. aa-ui now preserves that independent anchor lifetime:
tab-area-height changes retain the prior message top offset until a
content setter or tab insertion refreshes it. Window resize still moves
the anchored right/bottom corners normally.

Sources: `data/extract/aa-ui/native-chat-content-source-20261018.txt`.
Offline `chat_content_oracle.py` executes actual `392cad90` -> `392ca610`,
capturing only anchor removal/attachment. The five passing groups in
`native-chat-content-oracle-20261018.json` cover zero defaults, signed
offsets, positive corner extension, null entries/empty vectors and f32
top-offset addition. No live-client execution or writes are involved.

Five focused regressions cover default/full-width layout, independent
tab insets, hidden frames, same-offset refresh, selection and height
callback lifetime, zero-tab insertion, f32 addition, and the actual
original message formatter/link picker at the previously lost right-edge
width boundary. The original link remains on one line and pickable at
both 430- and 345-unit window widths.
Validation: `cargo check -p aa-ui` and **64 selected tests** passed,
including character clipping/scaling, tab layout, chat links/transport,
native widgets, scale/clip and pet/tooltip overflow. Built and copied the
aa-dev `native_chat_content` example; inspected the unique
`shots/aa-ui/native-chat-content-20261018.png` capture at 1280-by-720.
It shows the unchanged original constructors/conversation at default
offsets (430-by-170 message frame) beside signed offsets (402-by-150).
Measured **356.9–488.0 FPS**, zero script errors and no WARN/ERROR; owned
process 21236 self-closed. `reference/ingame/spawn/real.png` supplies the
original HUD/chat style context; no matching offset demonstration or
pixel-parity claim.

### Native selected-tab divider anchors (2026-10-18)

`SetLeftLineOffset` and `SetRightLineOffset`, both called by the original
chat constructor, now retain their signed f32 values and update the
divider anchors. Previously both methods were ignored, and tab layout
rewrote divider width/height directly, including overwriting any
explicit drawable height with one pixel.
Constructor `392cbb07/392cbb48` initializes both dividers to height one
and hidden; aa-ui keeps that native default even without the original
Lua constructor.

Registration `3925ab7f..ab8c` binds left offset to `39258b70` ->
`392cadd0` (`+33c`); `3925abde..abeb` binds right offset to `39258bb0`
-> `392cadf0` (`+340`). Both immediately call `392ca420`. This updater
hides both dividers and every message frame before checking selection.
A valid selected tab restores the two dividers and selected message;
invalid/empty selection leaves them hidden. It changes neither content
anchors nor tab-button layout and does not re-fire the original Lua
`OnSelectedChatTab` callback.

The native tab area spans the chat-window width and has the configured
tab-area height (`392c9966..99e1`). Relative to that area and selected
button, `392ca420` installs:

```
left:  BOTTOMLEFT  -> area.BOTTOMLEFT  (left_offset, 0)
       BOTTOMRIGHT -> tab.BOTTOMLEFT   (0, 1)
right: BOTTOMRIGHT -> area.BOTTOMRIGHT (right_offset, 1)
       BOTTOMLEFT  -> tab.BOTTOMRIGHT  (0, 0)
```

These are horizontal bottom-edge constraints: drawable height remains
authored. With original one-pixel height and zero bottom inset, existing
bottom-anchor precedence resolves the normal vertical position to
`tab_area_height - 1`; no default one-pixel shift is introduced. The
anchors now follow direct changes to selected-tab extent/position and
preserve custom drawable heights, signed outer offsets and the native
asymmetry when the tab has a bottom inset. aa-ui expresses the internal
tab area's two outer points as equivalent window-top corner anchors;
it does not add a new public widget.
The internal tab area's clipping is covered by the following milestone;
native sliding remains a separate open boundary.

Sources: `data/extract/aa-ui/native-chat-lines-source-20261018.txt`.
Offline `chat_lines_oracle.py` executes actual `392ca420`, capturing only
Show/anchor operations. Its five passing groups in
`native-chat-lines-oracle-20261018.json` prove default/middle selection,
signed offsets and invalid/empty selection hiding, without live-client
execution or writes.

Validation: `cargo check -p aa-ui` and **45 selected tests** passed,
including five new divider regressions plus content anchors, tab layout,
scaled character clipping and native widgets. Tests exercise physical
original tab selection/rendered line quads, signed offsets, authored
heights, direct tab movement/resize, invalid/empty selection, f32 values,
message visibility restoration, unchanged content anchors and absence of
duplicate Lua selection callbacks. The original one-pixel divider height
is preserved.
Built and copied the aa-dev `native_chat_lines` example; inspected the
unique `shots/aa-ui/native-chat-lines-20261018.png` capture at
1280-by-720 after physical selection of the second original tab.
It retains the original textures and one-pixel divider height with
signed outer offsets. Measured **364.7–371.6 FPS**, zero script errors
and no WARN/ERROR; owned process 20548 self-closed.
After the constructor-default refinement, rebuilt and inspected fresh
`shots/aa-ui/native-chat-lines-final-20261018.png`; it is byte-identical
to the prior PNG (SHA-256 `f6332bb3ab0626af5f4732497dcea94b60259899f9ba7806eef27ab16645a9d4`).
That run measured **85.4–198.2 FPS**, again zero script errors and no
WARN/ERROR; owned process 52988 self-closed.
`reference/ingame/spawn/real.png` supplies the original HUD/chat style
context; no matching signed-offset reference or pixel-parity claim.

### Native internal chat tab clipping (2026-10-18)

Fixed-width tabs, their descendants, the add control and the two selected-tab
dividers now crop at the native internal tab area. Previously overflow could
draw and accept pointer input outside the chat window. The message frames,
edit control, caret and lock notifier retain their separate native membership;
clipping the whole chat window would incorrectly crop these controls.
Public Lua `GetParent` identities remain unchanged.

Constructor `392cba29..bacb` creates `tab_cliping_area` at `+370` and sets
its clipping marker `+a4`. Its initial top-left anchor is `(0,3)` and bottom-right
anchor is the window top-right plus `(0,20)`; `3999edbc` contains f32 `3`.
Nonempty tab layout `392c9966..99e1` changes those anchors to `(0,0)` and
`(0,tab_area_height)`. The zero-tab guard at `392c97fe` leaves existing
area anchors intact. aa-ui caches those two vertical anchor offsets separately
from the current configured height, preserving that update lifetime.

Tab construction `392cc18c` -> `392c8520` -> `392c8574..8589` passes the internal
area as native parent. Add-control construction `392cbc7d..cbcd3` and divider
construction `392cbae1..cbb58` use the same area. Caret construction
`392cbe01..be46` instead uses the chat window, as do message frames and the
lock notifier. aa-ui models this paint/input membership without introducing
another public widget or changing existing content anchors.

Nearest clipping-ancestor lookup `39277a10` walks `+c0` parent links until
the `+a4` marker, then physical rectangle getter `392754a0` returns
`[x,y,x+w,y+h]` from physical position/extent, without pixel rounding.
Renderer `3927b010` uses this lookup before painter push `3921f480` and pop
`3921f1e0`. Nested pushes intersect max left/top and min right/bottom;
disjoint bounds remain disjoint rather than clamping to an empty point.
The new tab bounds retain physical f32 edges through widget/parent/UI scale.
Divider quads crop after transforms within their own paint ranges, including
rotated artwork, while other drawables belonging to the chat window stay
outside that range. Existing ancestor clipping also intersects the tab area.

Pointer traversal `3927c743..c790` applies the corresponding clipping ancestor.
Comparisons `3927c795..7bc` reject only coordinates strictly outside the
rectangle, so its edges are inclusive. aa-ui uses those comparisons for tab
members and descendants. Ordinary widget hit bounds retain their existing
contract. This milestone covers the original ordinary member path; native
sliding, escape/skip flags `+26c/+26d` and the painter's behavior beyond its
16 cached clip rectangles remain open.

Source: `data/extract/aa-ui/native-chat-clip-source-20261018.txt`.
Offline `chat_clip_oracle.py` executes actual nearest-ancestor, physical-rect,
painter-push/pop and pointer-comparison instructions without mocked calls.
Seven passing groups in `native-chat-clip-oracle-20261018.json` cover self,
parent and root markers, no marker, nested/disjoint intersections and exact
inclusive corners versus coordinates outside each edge. No live-client
execution or writes are involved.

Validation: `cargo check -p aa-ui` and **56 selected tests** passed: seven new
clipping tests plus divider/content anchors, tab layout, scaled text clipping,
native widgets and existing scale/clip coverage. Regressions include fixed-tab
overflow, one-f32-ULP pointer boundaries, member descendants, untouched message
and sibling paint, rotated divider ranges, nested areas, widget/parent/UI scale,
zero-tab anchor lifetime and actual original physical selection at UI scales
0.85 and 0.93. Full tab text and original tooltip values remain intact.

Built and copied the aa-dev `native_chat_clip` example and inspected unique
`shots/aa-ui/native-chat-clip-20261018.png` at 1280-by-720/default scale 0.85.
It shows original fixed-width tabs cropped at the chat edge after physical
selection, a complete selected-tab tooltip and unchanged original conversation
formatting. Measured **166.6–185.1 FPS**, zero script errors and no WARN/ERROR;
owned process 12052 self-closed. `reference/ingame/spawn/real.png` supplies the
original HUD/chat style context; no matching overflow fixture or pixel-parity
claim.

### Active tab counts and original guild sponsors (2026-10-18)

Original `create_expedition_view.lua` allocates seven sponsor tabs, then its
Lua `FillAllianceInfo` -> `InitSponsorsTab` calls `SetActivateTabCount(#info)`:
six sponsors for Nuia alliance 148, five for Haranya alliance 149. The method
was previously a no-op in aa-ui, so blank inactive sponsor buttons stayed
visible. `FillAllianceInfo` itself is original Lua, not a missing game getter.

Registration `3925a38a..a3a0` -> `39258680` -> `392c8760` stores a separate
signed count at `+2f0`, upper-bounded only by the allocated vector length
`(+2d4 - +2d0) / 16`. It does not change visibility, layout, selection or Lua
callbacks. `GetActivateTabCount` (`39259290` -> `392c84d0`) returns that field
through f32 rather than recomputing allocated rows. Native signed-number
adapter `3925995a` converts f32 with CVTTSS2SI. aa-ui now preserves the
independent count and delayed visibility update.

`SelectTab` binding `392586c0` subtracts one and calls `392c8790`. This stores
the requested index without a range guard. Active rows show either selected
or unselected buttons; only the selected active pane shows. Inactive rows
hide both buttons and their pane. Operations visit selected button, unselected
button, then pane, in row order; shared `AddNewTab` panes retain that native
sequential behavior. Afterward event `0x26` delivers `OnTabChanged` with the
requested one-based index as f32 on every call, including repeats, zero and
past-end requests. Selecting zero clears all panes, as the original sponsor
initializer requires. The previous keep-selection-on-invalid-index regression
encoded the wrong contract and has been corrected.

Native button dispatcher `392c8940` selects only a matching unselected button
(`row+8`). Clicking a selected button (`row+4`) does not repeat selection or
that callback; the separate `392c89c0` path accepts both for the tab sound.
This remains distinct from explicit repeated Lua `SelectTab` calls.
Layout `392c8a80` still lays out every allocated row, so inactive rows retain
their anchors and do not collapse remaining button/pane geometry.

Constructor `392c8450..84a8` initializes active count zero and native selected
index zero, exposed by `GetSelectedTab` as one even before allocation.
`AddSimpleTab` (`392cd2a4..2c9`) and `AddNewTab` (`392cd0d5..0fa`) restore active
count to allocated count, lay out and select the first tab on every successful
insertion. `RemoveAllTabs` (`392cd338`) resets count zero while retaining the
last selection field. aa-ui now matches those lifetimes. The native negative
count setter is retained by its getter, but aa-ui refuses selection while the
count is negative: native selection would index before the allocated vector.
No invalid native memory access is reproduced. `RemoveTab` remains outside
this bounded implementation.

Source: `data/extract/aa-ui/native-tab-active-source-20261018.txt`.
Offline `tab_active_oracle.py` executes actual count setter/getter, selection
and button-ID dispatch instructions. Only Show and Lua event delivery are
captured. Eleven passing groups in `native-tab-active-oracle-20261018.json`
cover six/five active rows, upper/zero/negative setter values, repeated/zero/
past-end selection, selected-outside-active and selected/unselected clicks.
Negative-count selection is not executed. No live-client execution or writes
are involved.

The adjacent application-owned guild name-policy audit is recorded in
`data/extract/aa-ui/native-name-policy-table-20261018.txt`: `394c8ca0` returns
zero results for unsigned kind >6 or a missing Xl policy record. Fields `+0`,
`+4`, `+8`, `+c` map to signed-number `local_min`, `local_max`, `eng_min`,
`eng_max`; the low byte at `+10` maps to numeric `limit_space_cnt`. Flag bits
0/1/2/4 at `+14` map to boolean `is_allow_space`, `is_allow_mix_lang`,
`is_allow_mix_case`, `is_allow_special_letter`. This is source evidence for
aa-client's utility adapter; aa-ui does not replace game policy data.

Validation: `cargo check -p aa-ui`, the example build and **70 selected tests**
passed: six new active-tab tests, 21 native-widget and two borrowed-tab tests,
plus 41 original friend/mail, clipboard, selection, IME, keyboard, HUD and
tooltip checks. The synthetic pointer fixture explicitly shows its parent tab.
Tests verify delayed count effects, allocated identity/geometry, numeric type
and f32 truncation, safe negative-count refusal, callback values/repetition,
zero/out-of-range selection, insertion/reset lifetime, selected-button clicks
and actual original sponsor controls through physical pointer input.

Built and copied the aa-dev `native_guild_sponsors` example and inspected both
unique original creation-window captures at 1280-by-720/UI scale 0.85:
`shots/aa-ui/native-guild-sponsors-nuia-20261018.png` shows six sponsor icons
after physical selection of Noryette Family (**243.7–285.6 FPS**), and
`shots/aa-ui/native-guild-sponsors-harihara-20261018.png` shows five after
physical selection of Wandering Winds (**264.7–287.1 FPS**). Both log zero
script errors and no WARN/ERROR; owned processes 8800 and 6824 self-closed.
The examples retain existing game-data fixtures, including their name guide;
these captures verify active controls and original artwork/layout, not the
application-owned name-policy adapter. `reference/ingame/spawn/real.png`
supplies HUD style context; no matching sponsor reference or pixel-parity claim.

### English native name-policy source and application handoff (2026-10-18)

The original character, summons/pet, guild, family-title, chat-tab and portal
name controls share application-owned `X2Util` methods. Their complete English
contracts are now recorded for aa-client integration; aa-ui does not replace
these game utilities. Existing type-2 guild oracle artifacts were preserved.

`XlInitStringUtil` (`xlcommon3301af40`) matches `en_us`, selects locale index 2
at `3307d5e4` and calls MSVCR100 `setlocale(LC_CTYPE=2,"english")`.
`XlGetNamePolicyInfo33009650` uses six 24-byte records per locale. Native kind
6 aliases kind 0; its separate public constant/name is not invented.
The English records, before X2Util's table conversion, are:

| Kind / original constant | local min/max | English min/max | space count | flags |
|---|---|---|---|---|
| 0 `VNT_CHAR` | 2/26 | 2/26 | 0 | 0 |
| 1 `VNT_SUMMONS` | 2/26 | 2/26 | 0 | 0 |
| 2 `VNT_FACTION` | 2/32 | 3/32 | 0 | 5 |
| 3 `VNT_FAMILY_TITLE` | 2/26 | 2/26 | 0 | 4 |
| 4 `VNT_CHAT_TAB` | 2/10 | 2/10 | 0 | 5 |
| 5 `VNT_PORTAL` | 3/32 | 3/32 | 0 | 5 |
| 6 (native alias of 0) | 2/26 | 2/26 | 0 | 0 |

The nine emitted fields and flag bits are the mapping in the preceding
milestone. A zero space-count limit is unlimited when spaces are allowed.
Metadata flags alone do not describe the validator: the English branch does
not apply the mixed-language bit, and its character range includes punctuation
and digits. `A1b`, `1ab`, `A_b`, `A-b` and `A.b` pass character-name validation;
`A1B` fails because uppercase after the first decoded character is forbidden
for kinds 0/1/6. A lowercase initial letter is accepted by the validator.

`XlIsValidName330095c0` rejects an original first byte `.` before dispatching
to English validator `330091a0`. Original byte-string `XlStringLen3301b000`
counts UTF-8 characters including a leading BOM. The original first byte's
ASCII alphabetic classification selects English versus local length bounds.
`XlMultiToWide3301a100` then strips exactly one leading UTF-8 BOM. The first
decoded character cannot be native space (ASCII 9..13 or 32). The trailing
check at `33009257` indexes the **original UTF-8 bytes** with converted
UTF-32 count minus one; it does not inspect the final decoded character.
Remaining decoded characters pass native space rules or ranges U+0020..007F
and U+00A0..017F, then the optional uppercase-after-first check.

Executed kind-2 boundary results include BOM+`Ab` and BOM+`A ` accepted,
BOM+` .ab` rejected, BOM+`.Abc` accepted, and two BOMs+`Ab` rejected.
For BOM+`Ab`, original scalar count is 3, converted count is 2 and the trailing
check reads original byte `BB`. For `é ` it reads byte `A9`, so this trailing
space is accepted. Leading/trailing NBSP U+00A0 are accepted; U+0085, U+1680,
U+2000 and U+2028 are rejected. Unicode trimming or general Unicode character
classification would change these source-backed results.

`CheckAvailableName394c8940` takes text and a mix-case boolean. It converts
at most 32 decoded codepoints into a 33-DWORD buffer, strips one leading BOM,
and returns through a 129-byte UTF-8 buffer. When mix-case is false, leading
native space produces the empty string. If the first decoded character is
native alphabetic, it uppercases that character and lowercases the rest;
otherwise case remains unchanged. With mix-case true it preserves case and
leading space. Punctuation is preserved. C-string NUL termination applies to
all three utilities. This method performs no server name-availability request.

Native `XlIsAlpha`/`XlIsUpper` and case functions cap their domain at U+07FF;
above that, classification is false and case conversion is identity. ASCII
uses original byte tables, with remaining values delegated to the initialized
MSVCR100 wide-character functions. `native-name-classes-20261018.json` records
all **8,192 actual native helper calls** for 0..07FF: 39 alphabetic ranges,
265 uppercase ranges, 456 changed lowercase pairs and 459 changed uppercase
pairs. The reference Rust artifact retains 65 uppercase ranges needed within
the validator's Latin range and losslessly compresses the complete case pairs
into 82 lowercase and 85 uppercase arithmetic runs. No Rust Unicode case or
classification approximation is used. The host x64 MSVCR100 used by the
oracle and the installed x86 counterpart both report version `10.00.40219.325`.

Argument conversion is explicit. CryScriptSystem `3550A460` -> `3550BF90`
requires Lua string for text and Lua number for numeric kind; numeric strings
and numeric text are rejected. Numbers narrow to f32 and CVTTSS2SI truncates
the kind. A requested boolean also accepts Lua numbers through double `!=0`;
missing/nil/string values fail. Failed parameter parsing yields zero results.
Successfully parsed invalid kinds (negative, >6, or CVTT indefinite) return
zero results from `GetNamePolicyInfo`, but one `false` from `IsValidName`.
Valid metadata returns one nine-field table; normalization returns one string,
including the empty string when appropriate.

Read-only artifacts under `data/extract/aa-ui/`:

- `native-name-source-20261018.txt`: PE hashes, disassembly and CryScriptSystem
  conversion source; `name_source.py` reproduces the extraction.
- `name_oracle.py` / `native-name-oracle-20261018.json`: **448 validation**,
  **58 normalization**, **31 wrapper** cases, direct BOM/length probes and
  eight separately labeled malformed/NUL byte groups. Original X2 trampolines,
  typed adapters, bodies and returns execute; Script GetParamAny/table/return
  interfaces and memset are captured, with conversion grounded in `3550BF90`.
  Windows conversion and CRT functions call the real host APIs after actual
  native English initialization, including buffer writes on conversion failure.
- `name_classes.py` / `native-name-classes-20261018.json`: complete helper
  classification/case mappings for the bounded native domain.
- `name_contract.py` / `native-name-contract-20261018.json` and
  `native_name_contract.rs`: implementation-ready well-formed UTF-8 contract.
  Its equivalent Python rules match every fixed case plus **609 differential
  groups / 5,481 actual native calls**. Generated Rust stays outside Cargo and
  was intentionally not compiled; aa-client owns implementation and tests.

Malformed UTF-8 observations use offline NUL-terminated, zero-padded buffers.
They are separate evidence of the native decoder boundary, not a claim that
safe Rust strings reproduce arbitrary invalid-buffer reads. No live-client
execution/writes, Cargo runs, production edits or additional visual deliverable
were needed for this source-only audit.

### Native English edit input and model clearing (2026-10-18)

Original `loginstage/login/login_view.lua` calls `SetEnglish(true)` on both
`nameEdit` and `passwdEdit` (lines 108 and 83). The previous toolkit method was
a no-op, so the username accepted typed spaces and Unicode and remained an
IME owner. `EditData::english` now implements the native typed-input flag.

Registration `3923a89a` binds `392398c0`; the bool adapter `3923a230` accepts
Lua booleans or numbers (`double != 0`, including NaN), rejects missing/nil/
string/table arguments, and returns no values. Widget setter `392923e0` writes
`+c28` then calls model setter `39228c70`. The latter always invokes clear
`39228510`, even for repeated or false calls: all four text buffers and caret/
selection positions become empty/zero. It preserves the input-dirty byte and
composing byte `+88` and dispatches no `OnTextChanged`. The toolkit clears the
visible buffer and composition base without notifying; context completion of
that cleared preview also produces no synthetic text-change callback.

Typed path `392298d0` checks English `model+98` and accepts only signed
character values **0x21..0x7e inclusive** (`!` through `~`). Rejected characters
leave the text and selection alone. Native focus acceptance `39292230` detaches
the IME context for password, English or digit edits. Result `392277b0` clears
composing before refusing an English result; preedit `39227da0` cancels a
nonempty English composition. English edits are therefore excluded from the
portable IME owner; enabling the flag during a preview cannot resurrect stale
text on context release.

`SetText` remains unrestricted. Paste `39228270` converts the OS clipboard,
then directly inserts its UTF32 buffer at `3922836f..392284c4` without the typed
English/digit filters. It still uses the native maximum-length rule and text
callback. `UseClipboard` is independent of English input.

Ignored offline evidence is `data/extract/aa-ui/english_edit_oracle.py`,
`native-english-edit-oracle-20261018.json` and
`native-english-edit-source-20261018.txt`. **51 groups** execute the original
setter/clear, typed guard, focus branches, paste insertion/copy, and typed bool
adapter. Setter and typed guard use no mocked calls. Focus captures context/
base-callback endpoints; paste starts after OS clipboard conversion with
supplied UTF32 data and executes native insertion/copy instructions unchanged.
The bool adapter captures `GetParamAny` at the documented Cry `3550BF90`
conversion boundary. No live process or OS clipboard is accessed.

QA: `cargo check -p aa-ui`, the example build, and **61 selected tests** pass
(one pre-existing font-metrics test ignored). Coverage includes seven English
cases, original login pointer picking and character-by-character callbacks,
IME, clipboard, selection, physical keyboard and library regressions. The
original password helper hides and the real callback enables Login; no login
submission is made. The example uses the original login constructors/handlers,
development-mode visibility and the existing fake option values required by
`SetReadyToLogin`'s hidden option window.

Both final **1280x720 / 0.85** captures were inspected:
`shots/aa-ui/native-english-typed-final-20261018.png` shows typed `Account!`
and a masked password (**240.8–350.7 FPS**);
`shots/aa-ui/native-english-paste-final-20261018.png` shows pasted `René Test`
with its space/accent preserved (**263.0–300.9 FPS**). Both report zero Lua
errors and self-close (owned PIDs 6748 and 30012). The standalone 2D login
still has a partially shown logo and no `login_camera` scene entity; each run
logs that one missing-camera warning. Full login-scene rendering remains open.

### Native local chat configuration and live tab lifecycle (2026-10-18)

The application-facing Rust API is `aa_ui::lua::chat::{ChatTabInfo,
ChatWindowInfo, hydrate_window, append_tab, update_tab, remove_tab}`. These
helpers take `&Lua`, a widget or global raw tab ID, and plain configuration
data; they add no native Lua method or event. The application owns native
global ID allocation, name validation, persistent configuration and API result
semantics. `ChatTabInfo` carries the ID/name, optional enabled-filter list,
optional font size/raw background RGBA, and per-filter RGBA pairs. Omitted
font/background fields keep the legacy defaults on creation and existing
values on update. `filters=None` accepts every filter; an empty enabled list
accepts none. A window carries its ID, ordered rows, signed stored selected
index and input method.

Native `39514120` skips same-window-ID assignments before reading config.
The constructor initializes window ID `-1` at `3951448a`; base constructor
`392cb708` and RemoveAll `392cb303` initialize/reset selection to `-1`.
`ChatData.selected == usize::MAX` represents that native no-selection state.
Hydration constructs each message, then its button, refreshes the raw-ID
metadata/history and calls `OnSetChatTabId`; only after all rows does it
force the stored selection and restore the input method. Invalid indices
cause no selection callback. Empty hydration returns before method restore
and creates no fallback row. A same-ID call preserves the rows and does not
call the app's hook.

Live append `39513680 -> 392cc270 -> 392cc160` calls
`OnCreateChatMessage`, then `OnCreateChatTabButton`, selects the appended row,
and finally calls `OnSetChatTabId`. Existing button/message objects, Lua
handlers, scrollback and selection callbacks survive. Native selection
`392cad00` suppresses an unchanged selection unless its force argument is set.
Rename/update `39507ae0`/`39507d40 -> 39503fd0 -> 395125a0` refresh the existing
objects and emit no create/ID/selection callbacks. Rename uses the ordinary
button text/auto-size/prefix-cache path, so a same-extent rename refreshes its
visible prefix while full text and the original tooltip getter remain intact.

Widget removal `39501e00 -> 39512fb0 -> 392cac10` preserves the numeric
selected index, clamping only when it is beyond the new end; removing an
earlier row does not preserve the selected raw ID. No selection callback is
emitted. Descendant widgets/drawables, focus, input registrations, handlers,
attachments and event listeners retire through the same subtree cleanup used
by completed popup removal. An empty configured window survives until the
next toolkit update, matching `395130e0`.

Direct native Delete has a deliberate observable edge: `39507f90` removes
the live widget row first, then model deletion `39506a70` refuses raw ID0.
Thus Delete(0) can return false after the UI changed, without saving or global
relayout. The Rust widget helper accepts ID0; configuration policy remains
separate. History remains available for later hydration when model deletion
fails. A newly allocated append, including reuse of a raw ID, starts empty.
Unknown live raw IDs return false without widget mutation.

`GetChatTabInfoTable` (`394593a0 -> 395080f0 -> 39503d20`) returns no values
for an unknown ID, or one six-field table: `id`, `name`, `fontSize`,
`bgColor={r,g,b,a}`, `filters={[2..59]=bool}` and
`filterColors={[2..59]={r,g,b,a}}`. Numeric metadata crosses native f32.
The internal `0xdb8`-byte record has ID/window/name at `+0/+4/+8`, 200 filter
bytes at `+59`, 200 RGBA entries at `+124`, font at `+da4`, background at
`+da8`. Font/background tint application is explicit in `395125a0`:
`39275030` applies the font and `3927b860` tints background-layer drawables.
`395124a0` relayouts retained lines with the current per-filter palette.
The toolkit retains each dispatched line's source filter and fallback colour;
composition replaces its base colour while preserving stored text, times,
scroll position, link payloads and inline colour markup. As with existing
chat markup, rendered colour codes use 8-bit ARGB. Native filter eligibility
`39511860` accepts IDs2..199; filter56 follows filter5's enabled flag while
retaining colour56. Updating filters affects subsequent dispatch, not old
history.

The existing `AAUI.ChatTabs(windowId)` hook remains compatible with its old
ID/name/enabled-ID-list rows. It also accepts native keyed-boolean filters,
`fontSize`, `bgColor`, and `filterColors`; the returned row list may carry
app-only `selected` and `method` fields. Only an absent/non-table hook falls
back to the legacy All row. `AAUI.ChatColor` remains the fallback palette.

Configuration evidence for the application adapter: manager `3a5bb0a0` has
eight global occupancy bytes at `+60..67`, shared across windows. Native
base/new tab `39500c50` has no enabled filters, en_us font15 (explicit original
`SetOptionDefaultFontSize(15)`), background `[0,0,0,1]` and a 200-entry palette.
Default constructors `39500cd0/d40/db0` produce raw IDs0/1/2 with en_us names
All/Party/Trade. General enables 41 filters including60, beyond the Lua
getter's range; Party enables `[4,5,32,35,36,37,38,39,47,56]`, Trade enables
`[9,35,36,38,40,49]`. `GetDefault` requires an existing record and preserves
its current ID/window/name after resetting the other fields. Native saves
raw records in ascending raw-ID order through LevelDB, independently of
window geometry and the separate 12-byte window extra records (window ID,
selected index, input method). Save-key `39286690` uses
`world_u8:character_field_u32:suffix`: original EnterWorld `394cbb10` passes
the requested world to `393cf830`, which resolves the login world list and
stores that ID's low byte at session `+7b` (`393cf893`). Current-world lookup
`393ce880` and the item formatter's `isMyWorld` comparison corroborate it.
The second component is `[[client+40]+18]`; its precise character identity
producer remains a separate source audit. No network or disk format is
invented here.

Ignored evidence under `data/extract/aa-ui/`:
`chat_lifecycle_source.py` / `native-chat-lifecycle-source-20261018.txt` record
the PE hash and targeted original instructions.
`chat_metadata_oracle.py` / `native-chat-metadata-oracle-20261018.json` execute
19 metadata/default groups plus 10 filter-eligibility cases, including all
200 palette entries and both native tree lookup levels.
`chat_lifecycle_oracle.py` / `native-chat-lifecycle-oracle-20261018.json`
execute 18 assignment/delete/selection/append groups. Widget allocation, layout/visibility,
destruction, scripting interfaces and nonzero model erase/save endpoints are
captured; the actual ID0 model guard, widget lookup/removal, selection
bounds/force logic and append ordering run unchanged. This is offline PE
execution, with no live-client or original game-data writes.

General dynamic drawable alpha/fade, drag reparent/tear-off, and app-owned
chat configuration persistence remain separate work. This change applies
the supplied raw tint and introduces no generalized alpha workaround.

QA: `cargo check -p aa-ui`, example build, and **79 focused tests** pass
(chat lifecycle/layout/clip/dividers/content/links/input/transport, native
widgets, HUD scrolling and popup lifecycle). The final seven lifecycle tests
were rerun after the final changes. They cover exact callback order, same-ID
no-op, invalid/empty selections, raw ID reuse/history restoration, metadata
recolouring without changing stored lines, filter56, focused descendant/event
retirement, and actual original chat handlers with physical tab picking,
same-extent rename/prefix refresh and the full original tooltip. Three older
tests that manually cleared rows now assign a different window ID to rebuild;
their previous same-ID reconstruction assumption contradicted native source.

`native_chat_lifecycle` uses the original scripts through aa-dev's standard
plugins and screenshot/exit harness. Both inspected **1280x720 / 0.85**
captures report zero Lua errors and self-close:
`shots/aa-ui/native-chat-before-085-20261018.png` shows initial All/Trade
with retained history and the original tooltip (**180.0–201.3 FPS**);
`shots/aa-ui/native-chat-after-085-20261018.png` shows appended/renamed Harbor,
the updated font/raw background/palette on old lines, inline colour markup,
and removal of Trade (**317.9–411.5 FPS**). An initial inspected 1920x1080
capture, `native-chat-before-20261018.png`, logged **154.3–166.9 FPS** and
also self-closed. The differing host load prevents treating these ranges as
a before/after performance comparison.

### Chat selected-index persistence before original callbacks (2026-10-18)

`AAUI.ChatSelection(windowId, zeroBasedIndex)` is an optional application hook,
not a native Lua method/event. Native selected-frame callback `395138b0`
reads the existing 12-byte window-extra row through `39503cd0`, replaces its
selected index at `+4`, writes it through `39505030`, and calls
`SaveWindowExtra` (`395044d0`) at `395139bb`. Only afterward does the original
loop invoke `OnSelectedChatTab` for each button in index order. The toolkit
now calls the app hook at that same point, after storing selection/visibility
and before the original button callbacks.

Valid forced hydration and live append both persist selection. Unchanged
unforced clicks, invalid stored indices, same-window-ID assignment, metadata
refresh, and row removal/index clamping do not. A hook error logs a warning
and continues the original callbacks with the new selection intact, matching
the native ignored save-result behavior. The hook intentionally carries only
the selected index: hydration restores the input method after selection, so
passing the widget's constructor-time method would overwrite the saved method.
The app preserves its other window-extra values and chooses its storage.

`data/extract/aa-ui/chat_selection_oracle.py` and
`native-chat-selection-oracle-20261018.json` execute **seven groups** through
actual `392cad00 -> 395138b0`, native existing-row RB lookup/update, raw
12-byte copies and the complete per-button callback loop. Only visibility,
drawable/input endpoints, LevelDB save entry and Lua callback delivery are
captured. Both save success/failure leave the new index in the row before all
three callbacks, preserving input methods `4` and `-6`. Same, forced, negative
and out-of-range selection cases retain their native gating.

All **nine focused lifecycle tests** pass, including physical selection with
the hook, persistence before skin callbacks, method preservation, no callback
on metadata/remove/clamp, and warning/continuation on a failed save. The
existing original-script capture remains applicable because an absent hook
changes no rendering. Save-key scope source also identifies session `+28`
as the account ID: `393cfbb0` stores its first argument there at `393cfc0f`
and writes it to `cl_account_id` at `393cfc34..3f`; that account field is used
by the separate generic Data manager and is not part of chat's two-number key.

### Native ListCtrl automatic row geometry (2026-10-18)

The original achievement list allocates six rows inside a 440-unit control,
then sets its column header height to zero. It never supplies an item height.
The toolkit previously assigned every row 25 units, leaving most of the list
unused and crowding its nested achievement controls. Rows now receive the
native automatic height: **73 units**, with the final two units unused.

Native constructor `392a37e0` initializes header height to `30.0` and line gap
to zero. Layout `392a4410` returns without changing row geometry when either
the column or row vector is empty. Otherwise its exact arithmetic is:

```text
occupied = f32(rowCount - 1) * gap + headerHeight
available = controlHeight - occupied
rowHeight = f32(CVTTSS2SI(available) / signed(rowCount))
```

The operations before conversion are f32; division is signed integer
division toward zero. Invalid/nonfinite/out-of-i32-range conversion produces
`i32::MIN`, as the native instruction does. Each row receives the full
resolved control width and this same height. Its first anchor is below the
first header; subsequent rows anchor below the preceding row with the line
gap. Sub-items use their recorded column widths, share the row height and
anchor horizontally to the preceding sub-item. No remainder is distributed.

`SetColumnHeight` (`392a49f0`) updates the header widgets before layout.
`SetItemLineGap` (`392a4a50`, Lua wrapper `39246e90`) now has its real binding
instead of the existing class stub. Native geometry callback `392a4a70`
runs the ordinary widget geometry callback before layout. The toolkit
handles direct list extent/anchor changes and settles resolved extent changes
after bounds updates, including changes inherited from anchored parents.
Unchanged extents do not rebuild rows every frame; objects, data, selection
and handlers remain intact. Row/column/header/gap mutations lay out immediately.
The old toolkit-only `SetItemHeight` implementation retains an explicit-height
override, but remains absent from the captured native Lua class method list;
this change adds no invented public native method.

`data/extract/aa-ui/native-listctrl-geometry-source-20261018.txt` records the
targeted original instructions. `listctrl_geometry_oracle.py` and
`native-listctrl-geometry-oracle-20261018.json` execute **ten native groups**
with the original PE hash: constructor defaults, complete row/sub-item loops,
fractional arithmetic, negative/nonfinite conversion, zero guards, both
setters and the base-then-layout geometry callback. Only widget anchor/extent
endpoints and constructor external registration/base/name calls are captured;
extent writes feed the actual native getter. The oracle also records native
optional event-window gap geometry; creating that separate event-window
branch of `InsertRows(..., true)` remains outside this layout correction.

`cargo check -p aa-ui`, the example build, and **63 selected tests** pass,
including four new geometry tests, actual original achievement checkbox
picking/add/remove, original crafting category controls, achievement category
templates/art, listboxes, native/HUD widgets, active tabs and popup retirement.
Resize tests preserve row/sub-item identity and selection, verify anchored
parent changes, and ensure an unchanged tick does not overwrite row geometry.

`native_achievement_list` opens the actual original window through its content
trigger and provides six deterministic achievement records. It changes no
script/fixture geometry. The inspected aa-dev capture
`shots/aa-ui/native-achievement-list-73px-20261018.png` shows all six properly
spaced rows at **1280x720, UI scale 0.85**, reports **421.2–567.4 FPS**, zero
Lua errors and no runtime warnings/errors, and self-closes after its screenshot.
The repository has no matching real-client achievement screenshot; its exact
row size is checked against executed native code and the original 73-unit
achievement background asset coordinates.

Chat save-key character identity is also now source-closed by the aa-net
audit: `OnCharacterState` (`391d3420 -> 391b9640`) constructs `3930fa80`,
stores that object at client `+40` (`391b988f`), and passes object `+10` plus
the received character state to `39089d50`. That helper reads the state's
first u32 persistent character ID (`39089d58`) and stores it at header `+8`
(`39089d69`), namely object `+18`. Thus chat's `[[client+40]+18]` component
is the selected persistent character ID, alongside the already traced world
byte and the separate account-scoped generic Data storage.

### Native chat alpha and owner drawable layers (2026-10-18)

Global chat opacity now uses the native owner drawable-layer state rather than
rewriting message tint or multiplying every ancestor's opacity. The application
calls `lua::chat::set_alpha_options(&Lua, over_alpha, leave_alpha)` after saving
its configuration. The raw f32 pair is retained for future windows and applied
to existing windows, including an unchanged-pair refresh. The setter adds no
native Lua methods, events or persistence policy.

Native defaults `395072c0` are over `0.7`, leave `0.5`. Get/UpdateChatOption
(`39459bd0`, `39458230`, `39459db0`) use those two numeric fields without
clamping; missing or nonnumeric update fields retain the old value. Setter
`39508b90` saves the eight bytes through `395019d0`, then refreshes live windows
even when saving fails. This refresh snaps enabled layers to idle immediately;
the next ordinary frame reapplies the current pointer state.

Configuration `39512a40` assigns leave/over pairs recursively, then enables
background, artwork and overlay on the chat window. Input/edit/selector/IME
drawables use `1/1`; the four message scroll controls use `0/1` through the
message override `39511cc0 -> 395118d0`. These controls are configured before
the original message/window creation callbacks (`39512447..7b`,
`39514714..29`). Content opacity remains one, preserving text, links and inline
markup. Message metadata application `395125a0` retains the complete raw
background RGBA but explicitly supplies alpha **one** to the background tint
at `39512607`; global opacity is independent of that metadata alpha. The raw
alpha remains available for the existing tab/divider decoration paths
(`395118c0`, `395138ea`, `3950408f`).

`UseDynamicDrawableState(layer, bool)` now implements the real binding.
Native `3927d4b0` holds the current value when disabled and snaps/propagates
the owner's present hover state when enabled. Original enter/leave handlers
run before propagation (`3927ef10`); the per-frame chat update also reapplies
state (`395130e0 -> 3927e500`), so later children settle even without a new
pointer transition. Ordinary child initialization leaves its own enable flags
false. Attachment `3927e5d0` copies enabled ancestors' pairs and initial idle
values, with farther ancestors applied last. It does not clone enable flags.
This preserves an original popup's ability to freeze a whole chat branch.

The renderer applies only the drawable owner's layer factor. Native
`3927af50 -> 3921a870/3921a8e0` pushes and restores that factor before child
rendering, so parent and child factors are not multiplied together. Authored
widget/drawable alpha and animation remain separate. The native fourth
`overoverlay` state can be stored, but this backend still renders only its
existing three drawable layers. Generic dynamic content state, native
input-activation exceptions, blocked-layer flag producers, and the older
toolkit tab-decoration visibility/alpha estimates remain separate gaps. This
slice does not infer their behavior from hover or focus.

`data/extract/aa-ui/chat_alpha_oracle.py` and
`native-chat-alpha-oracle-20261018.json` execute **16 native groups** with the
original PE hash: defaults, raw options and failed save, recursive pairs,
hover/idle, disabled/reenabled state, callback order, late-child attachment,
metadata alpha, and renderer stack restoration. The real child loops and
numeric/state operations execute unchanged; external manager/save/render
delivery and metadata/font/layout endpoints are captured. In particular,
four raw metadata alphas (`0`, `0.8`, `-1`, `2`) survive storage while all four
background draw tints use alpha one. Targeted instructions are recorded in
`native-chat-alpha-source-20261018.txt`.

`cargo check -p aa-ui`, the example build and **101 selected tests** pass,
including six new alpha tests and the chat lifecycle, clipping, transport,
text/link, input, tab layout, animation, native/HUD widget, list geometry and
popup retirement suites. Tests exercise original physical pointer hover,
scroll overrides, raw options, unchanged refresh, original callback order,
disabled state, late children and preserved history. Two fresh original World
runtimes restore four tabs while refreshing options inside `AAUI.ChatTabs`.

The aa-dev example `native_chat_alpha` uses the original World/chat scripts at
1280x720 and UI scale 0.85. Inspected captures
`shots/aa-ui/native-chat-alpha-idle-20261018.png` (**255.5-265.2 FPS**) and
`native-chat-alpha-hover-20261018.png` (**465.7-548.9 FPS**) show the background
changing while text and links remain fully opaque. Both report zero Lua errors,
no runtime warnings/errors, and self-close after the screenshot. These are
deterministic source-contract captures; no matching real-client opacity
reference is claimed.

### Original chat tab context dispatch (2026-10-18)

Right-clicking a chat tab now reaches the original context menu. The native
tab event receiver `392ccc80` routes click kind `2` to `392caf30`. That handler
recognizes mouse code `3` as `RightButton` (native table `39ac6a30`), finds the
exact registered tab button, and invokes virtual `+1bc`, namely `39513750`.
It does not select the tab or save a selected index. The separate left-button
branch retains its ordinary selection path. This context path does not test
the `AllowTabSwitch` flag used by the separate press/drag handler.

`39513750` returns for an invalid index, missing message or failed chat-message
cast. Otherwise it first emits **`SHOW_CHAT_TAB_CONTEXT(button, rawTabId)`**
(event index `0x1f4`, name initialized at `392160cc`), then invokes the original
global **`OnContextChatTab(message, button, rawTabId)`**. The raw signed ID is
narrowed to f32 for each Lua number, and is read again after event delivery.
An absent global callback does not suppress the UI event. The toolkit follows
this order at its existing native click dispatch point and introduces no new
game methods or context-menu policy.

The unchanged `x2ui/chat/chat_window.alb` callback disables the chat window's
three dynamic drawable layers and opens `chat/tab_popup.alb`. Its existing
hide procedure reenables those layers. The original popup owns its protected
tab-zero actions, rename/settings/clear/remove controls, outside-click closing
and retirement; the toolkit does not reconstruct this menu.

`data/extract/aa-ui/chat_context_oracle.py` and
`native-chat-context-oracle-20261018.json` execute **13 native groups** through
the actual event receiver, button lookup, message cast, native context handler
and Lua argument marshaler. Only supplied script-interface endpoints, reference
counting and event/script delivery are captured. Cases cover unselected-tab
right-click without selection, other buttons/events, non-tab targets, invalid
indices/message/type, missing global, f32 ID narrowing and an event listener
changing the ID before the callback's second read. Instructions and vtable/name
tables are recorded in `native-chat-context-source-20261018.txt`.

All **39 selected tests** pass across context dispatch, alpha, clipping,
input links, transport, configuration lifecycle and original popup retirement.
The three new tests verify exact physical picking, press/release timing,
right-click without selection/persistence, event/callback arguments and order,
the post-event ID read, missing callback, original popup alpha freeze,
outside-click retirement/reopen, and the original Clear control's raw tab ID.
The example builds successfully. Its inspected aa-dev capture
`shots/aa-ui/native-chat-context-20261018.png` shows the unchanged original
tab-zero context menu at 1280x720/UI scale 0.85; zero Lua errors and no runtime
warnings/errors are reported at **260.1-290.6 FPS**, and the process self-closes
after its shot.

### Drawable GetColor named RGBA and original chat option save (2026-10-18)

The original chat option dialog passes `colorBG:GetColor()` directly into each
filter's color record and reads `.r/.g/.b` for the background. The toolkit's
sequence-only return omitted those names, causing the source-faithful metadata
parser to reject a real SaveOption operation. `Drawable:GetColor()` now returns
one fresh table with exactly **`r`, `g`, `b`, `a`** and no numeric aliases.
Changing the returned table does not change the drawable or another returned
table. Raw f32 channels are preserved without clamping. Calling the method on
an object that cannot provide a drawable returns zero values.

Native registration `39237e7d..d3` binds `39235760`. It obtains the drawable
through virtual `+b0`, reads the four floats through virtual `+f8`, calls
`CreateTable(0)`, writes those four named keys through `SetValue(+14)`, and
returns the single table at `39235901..4f`. There are no indexed `SetAt` calls.
The earlier string reference to GetColor at `39231ec3` belongs to a separate
cursor RGB API and does not describe this drawable method. Native SetColor's
float constructor `3922b330` also copies its four inputs without clamping.

`data/extract/aa-ui/drawable_color_oracle.py` and
`native-drawable-color-oracle-20261018.json` execute **four groups** through
the unchanged binding, with its PE hash: ordinary channels, values outside
zero-to-one, missing drawable, and consecutive calls with changed channels.
Only drawable/script-interface endpoints and table delivery are captured;
the actual body performs all f32 copies, named writes, fresh table requests
and return-arity branches. Exact instructions are recorded in
`native-drawable-color-source-20261018.txt`.

All **12 focused tests** pass, including exact key count, absent numeric keys,
return arity, table independence, raw f32 values, the original slot-state script,
chat alpha/context lifetimes, and physical OK on the unchanged original chat
option dialog. That original save retains filter RGBA `0.25/0.5/0.75/0.8`, saves
background RGB `0.1/0.2/0.3`, preserves the prior background alpha as its Lua code
does, and returns font size 16. The application metadata parser remains strict.

The `native_chat_options` aa-dev example builds and performs the original
physical save before reopening the saved dialog with `--saved`. The inspected
capture `shots/aa-ui/native-chat-options-named-color-20261018.png` shows its
retained blue Local palette swatch at 1280x720/UI scale 0.85. It reports zero
Lua errors, no runtime warnings/errors, **318.9-408.4 FPS**, and self-closes
after the screenshot. No matching chat-options reference capture is present.

### Native chat method persistence before display (2026-10-18)

The optional application hook **`AAUI.ChatMethod(raw_tab_id, method)`** now
runs at the native method-save position, before changing the selector label
and before its original `OnContentUpdated` callback. The input's method and
whisper-prefix state are already updated. The application resolves the raw
tab ID through its global configuration, preserves that owner's selected
index, and saves only when the raw ID exists. It must not substitute the
current widget's window ID. Unknown IDs still change the visible input method;
hook failures warn and continue without rolling back the input state.

Native `39510390` resolves input `+e70` through `39503c00`, copies the owner's
window-extra record, replaces only its method, calls `39505030/395044d0`, then
updates the display through `39510120` and emits content event `0x20`.
Missing raw IDs skip saving; a failed save does not skip display. Restore
`395107d0` calls this same path through virtual `+1c4` (`399e781c`), even for an
unchanged method. Hydration `3951429f..a6` therefore persists restored methods
after selection and before display. Empty hydration returns before restoration.

The input identity is distinct from the visible numeric selected index.
Initialization `3950ffa1..ffb5` sets method zero and raw ID `-1`.
Selection `3951394b..67 -> 39510e60` binds the selected message's raw ID before
selected-index persistence. Native row deletion `392cac10` clamps only the row
index, and clear-for-rehydration `392cb240` clears it to `-1`; both retain input
`+e70`. A later method change can consequently save through a removed/moved
raw ID's current model owner, or skip saving when that ID is absent. Invalid
stored selection during rehydration also retains the previous input raw ID.
The toolkit now keeps this field explicitly and follows those lifetimes.

`data/extract/aa-ui/chat_method_oracle.py` and
`native-chat-method-oracle-20261018.json` execute **12 native groups**, with
the PE hash: actual owner lookup, existing-extra lookup/update, save/display
order, repeats, failed save, signed fields, restoration, unknown IDs, the
initializer's exact store block, selected-raw-ID store, deletion/clamping and
clear-for-rehydration. Prefix/font/content/method-list delivery, widget
destruction/layout, primitive memmove and LevelDB save endpoints are captured;
the remaining native control flow and numeric operations execute unchanged.
Instructions are in `native-chat-method-source-20261018.txt`.

`cargo check -p aa-ui`, the example build and **30 selected tests** pass across
method persistence, lifecycle, input links, context, alpha and transport.
Five new tests verify save-before-display/callback order, selection preservation,
repeat and restored saves, current model ownership after movement, no save for
unknown IDs, raw-ID retention after removal/invalid rehydration, empty/same-ID
guards, failure continuation, and original chat restoration plus typed channel
changes. No additional native Lua game methods or events are introduced; native
method-list population and the existing command-parser approximation remain
separate boundaries.

The aa-dev example `native_chat_method` restores an original chat window with
method 4, opens/focuses its actual edit, and types through the normal input path.
The inspected `shots/aa-ui/native-chat-method-restored-20261018.png` shows the
restored party selector and editable text at 1280x720/UI scale 0.85. It reports
zero Lua errors, no runtime warnings/errors and **517.4-566.5 FPS**, and its
process self-closes after the screenshot.

### Native chat drag locking and notification (2026-10-18)

`AllowTabSwitch(bool)` now controls the native tab-drag policy. Ordinary left
selection and right-click context remain available while locked. The optional
application API `lua::chat::set_allow_tab_switch(&Lua, widget_id, allowed) -> bool`
returns false for a non-chat widget; the application separately applies native
resizing policy and saves its lock configuration. `ChatData` exposes the policy,
notification drawable ID, remaining milliseconds, maximum duration and blink
interval for inspection. No native Lua getter or application event is invented.

The trigger is **drag start**, not mouse-down. Native tab construction
`392cc19e..af` registers LeftButton dragging and subscribes the chat receiver.
`ButtonWidget::OnDragStart` at `392820e0` invokes Lua event `0x14`, then constructs
the source/button packet and sends internal event 4 at `39282211..27`.
Receiver `392ccc80 -> 392cc600` reads the current lock flag after that callback.
The toolkit reuses its existing drag threshold and source lifecycle in that
order. A locked drag copies maximum duration to the remaining timer; it does not
move, split or select the tab. A simple click does not arm the notification.

Unlocked first-tab dragging calls ordinary `StartMoving` (`3927a510 ->
39278a10(9)`) **before** checking whether raw tab zero permits splitting.
The toolkit now captures this first-tab window movement. Enabling/disabling the
policy does not cancel an existing generic move. Native secondary-tab splitting,
cross-window drop/reparenting and the manager's transient split-drag state remain
open; secondary tabs are not substituted with whole-window movement. The
existing toolkit drag threshold and its LeftButton-only input boundary remain
explicit rather than claiming new native threshold or right-drag coverage.

Fresh constructor store blocks `392cb708..40` and `392cb8b4..392cba1c` establish
locked, remaining zero, 500 ms interval and 2,000 ms maximum. Original
`x2ui/chat/chat_window.alb` overrides duration/interval to 3,000/400 ms and supplies
the HUD lock image. The constructor anchors that drawable's BOTTOMLEFT to the
window TOPLEFT with zero offsets (`392cbe81..b1`; native point table `39ac6a5c`).
`AllowTabSwitch(true)` (`392c9680`) clears the timer and hides the drawable;
false retains both. Number adapters `39259c90` and bool adapter `39259d50` require
their exact types: missing values, numeric strings and wrong types do nothing.
Timer setters `392c96c0/392c96f0` clamp negative values to zero while preserving
negative zero, NaN and positive infinity; they retain any running timer.

Native frame block `392cb310..80` subtracts the actual f32 frame milliseconds,
computes `(maximum - remaining) / interval` in f32, converts with CVTTSS2SI, and
shows the image only for signed remainder 1 modulo 2 while remaining is positive.
Expiry retains a possibly negative remainder. A nonpositive or NaN remaining
value leaves visibility untouched; zero/NaN interval converts to integer-indefinite
and hides the image. The toolkit preserves those operations and passes frame
milliseconds directly into chat update.

`data/extract/aa-ui/chat_lock_oracle.py` and
`native-chat-lock-oracle-20261018.json` execute **40 native groups** with the PE
hash: constructor stores/anchor, strict argument adapters, raw setters and f32
edges, actual ButtonWidget drag producer/callback order, real tab/message lookup,
raw-zero protection, secondary split boundary and frame blinking/expiry.
Lua/audio/profile delivery, observer forwarding, generic move capture, secondary
split and manager preparation are explicit endpoints; remaining instructions run
unchanged. The oracle neither performs nor emulates splitting. Instructions are
in `native-chat-lock-source-20261018.txt`.

`cargo check -p aa-ui`, the example build and **37 selected tests** pass across
locking, alpha, context, method persistence, ordinary dragging and resizing.
Seven new tests exercise exact setter types/defaults, signed zero/NaN/infinity,
simple clicks/context while locked, callback-before-policy ordering, physical
movement and capture release, disabling during capture, retained timer state,
expiry/zero interval, removal during the drag callback and the unchanged original
chat script's artwork/timers. The aa-dev example `native_chat_lock` makes a real
pointer drag attempt and retains the original script's notification geometry.

The inspected `shots/aa-ui/native-chat-lock-20261018.png` shows the original lock
above the chat tabs at 1280x720/UI scale 0.85. It reports zero Lua errors, no
runtime warnings/errors and **224.1-283.8 FPS**; its copied process self-closes
after the screenshot. No corresponding locked-chat reference capture is present.
