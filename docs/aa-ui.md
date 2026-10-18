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
  dialog (`DLG_TASK_*`, the handlers of `dialog_handler.lua`) with `info` merged in, OK / Cancel;
  set `wnd.OkProc` on the result for what OK does.

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
`cargo run -p aa-ui --example native_listbox -- --coverage` (2026-10-17).

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
| `listbox` | 99 | 39 | 138 |
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

The bounded slice leaves tree rows (`SetItemTrees` / `GetViewItemsInfo`), icon append
overloads, state textures/borders and row tooltips as explicit gaps. Flat
`GetOpenedItemCount` equals row count. The native selected-text getter also converts through
a 512-byte UTF-8 output buffer; its overflow policy is not reproduced by the current getter.
Crafting category hierarchy therefore still needs a separate recovered tree contract.

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
